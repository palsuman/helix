//! File watching (Task 1.7, REQ-FS-004).
//!
//! ```text
//!  OS events ──► per-root notify watcher ──► channel ──► watcher thread
//!   (native or polling)      │                              │
//!                            └─ excluded paths dropped here  ├─ Debouncer (50ms)
//!                               (cheapest possible place)    ├─ rate meter
//!                                                            └─► listener
//! ```
//!
//! One thread serves every root. Debouncing is inherently cross-root (a
//! `git checkout` in a monorepo touches several at once) and a thread per root
//! would multiply the cost of a multi-root workspace for no benefit.
//!
//! ## Decisions worth naming
//!
//! **Exclusions are applied in the notify callback**, before the event reaches
//! the channel. That is the only place where an excluded event costs nothing:
//! filtering later means a `cargo build` still pushes tens of thousands of
//! `target/` events through a bounded channel, and the channel becomes the
//! bottleneck instead of the thing that absorbs the burst.
//!
//! **Native or polling is decided per root, at watch time.** A workspace can
//! have a local root and a root on a network share, and forcing both into one
//! mode would either break the share or make the local root needlessly slow.
//! See [`crate::probe`] for why latency rather than the mount table decides.
//!
//! **A watcher error degrades the root to polling rather than failing it.** The
//! failure that actually happens is inotify's per-user watch limit being
//! exhausted, and the useful response is to keep working more slowly while
//! telling the user what to raise (REQ-FS-004 failure modes). Dropping the root
//! would mean the explorer silently stops updating, which is the worst of the
//! available outcomes.
//!
//! **The budget warns, it does not refuse.** 10,000 paths per root is the
//! documented limit (REQ-FS-004.6), and a monorepo that exceeds it still needs
//! to be usable. The warning carries concrete suggestions derived from the
//! actual tree, because "add some exclusions" without naming the directories
//! responsible is not advice.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use helix_core::error::AppError;
use helix_log::{Logger, log_debug, log_info, log_warn};
use notify::event::{EventKind, ModifyKind, RenameMode};
use notify::{Config as NotifyConfig, PollWatcher, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::change::{ChangeKind, DEFAULT_DEBOUNCE, DEFAULT_MAX_HOLD, Debouncer, FileChange};
use crate::exclude::{ExclusionConfig, Exclusions};
use crate::listing::{self, Listing};
use crate::probe::{self, NETWORK_LATENCY_THRESHOLD, POLL_INTERVAL};

/// Watched paths per root before the warning fires (REQ-FS-004.6).
pub const DEFAULT_PATH_BUDGET: u32 = 10_000;

/// Log source for file system records.
pub const LOG_SOURCE: &str = "kernel.fs";

/// Called with each debounced batch of changes.
pub type ChangeListener = Arc<dyn Fn(&[FileChange]) + Send + Sync>;

/// How a root is being watched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum WatchMode {
    /// OS change notification. inotify, FSEvents, or ReadDirectoryChangesW.
    Native,
    /// Periodic directory scanning, for network filesystems and for roots the
    /// OS refused to watch natively.
    Polling,
}

impl WatchMode {
    pub fn as_str(self) -> &'static str {
        match self {
            WatchMode::Native => "native",
            WatchMode::Polling => "polling",
        }
    }
}

/// Watcher tuning. Defaults come from the requirement, not from taste.
#[derive(Debug, Clone)]
pub struct WatchConfig {
    pub debounce: Duration,
    pub max_hold: Duration,
    pub path_budget: u32,
    pub latency_threshold: Duration,
    pub poll_interval: Duration,
    pub exclusions: ExclusionConfig,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            debounce: DEFAULT_DEBOUNCE,
            max_hold: DEFAULT_MAX_HOLD,
            path_budget: DEFAULT_PATH_BUDGET,
            latency_threshold: NETWORK_LATENCY_THRESHOLD,
            poll_interval: POLL_INTERVAL,
            exclusions: ExclusionConfig::default(),
        }
    }
}

/// What watching one root turned out to involve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct RootReport {
    pub root: String,
    pub mode: WatchMode,
    /// Directories registered with the OS. This, not the file count, is what
    /// the budget measures, because a recursive watch costs one handle per
    /// directory.
    pub watched_paths: u32,
    pub files_seen: u32,
    pub over_budget: bool,
    /// Glob patterns that would bring the root under budget, derived from the
    /// directories actually responsible (REQ-FS-004.6).
    pub suggested_exclusions: Vec<String>,
    /// Measured filesystem latency, when the probe could run.
    pub probe_latency_ms: Option<u64>,
    /// Set when the root fell back to polling after the OS refused a native
    /// watch, carrying the reason so the user can act on it.
    pub degraded_reason: Option<String>,
}

/// Point-in-time watcher metrics, published to health monitoring
/// (REQ-FS-004.8, REQ-OBS-004.1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct WatchStats {
    pub roots: u32,
    pub watched_paths: u32,
    pub polling_roots: u32,
    pub over_budget_roots: u32,
    /// Raw OS events received.
    pub events_seen: u64,
    /// Debounced changes delivered.
    pub changes_emitted: u64,
    /// Raw events that cancelled out and were never reported.
    pub dropped_as_noise: u64,
    /// Raw events over the most recently completed one-second window.
    pub events_per_second: u32,
    /// Changes waiting out their debounce window right now.
    pub pending: u32,
    /// Roots moved to polling after a native watcher error.
    pub degraded_roots: u32,
}

/// A raw event on its way to the debouncer.
enum RawEvent {
    Change {
        root: PathBuf,
        path: PathBuf,
        kind: ChangeKind,
        is_dir: bool,
    },
    /// The OS watcher for a root reported a problem.
    WatcherError { root: PathBuf, message: String },
}

/// Events per second over the last completed window.
///
/// Bucketed rather than a timestamp list: an event storm is exactly when this
/// metric matters, and a metric that allocates per event during a storm makes
/// the storm worse.
struct RateMeter {
    bucket_start: Instant,
    current: u32,
    previous: u32,
}

impl RateMeter {
    fn new(now: Instant) -> Self {
        Self {
            bucket_start: now,
            current: 0,
            previous: 0,
        }
    }

    fn record(&mut self, count: u32, now: Instant) {
        self.roll(now);
        self.current = self.current.saturating_add(count);
    }

    fn rate(&mut self, now: Instant) -> u32 {
        self.roll(now);
        self.previous
    }

    fn roll(&mut self, now: Instant) {
        let elapsed = now.duration_since(self.bucket_start);
        if elapsed < Duration::from_secs(1) {
            return;
        }
        // More than one second of silence means the last completed window was
        // empty, not stale.
        self.previous = if elapsed < Duration::from_secs(2) {
            self.current
        } else {
            0
        };
        self.current = 0;
        self.bucket_start = now;
    }
}

struct RootEntry {
    /// Kept alive because dropping a `notify` watcher unregisters it.
    watcher: Box<dyn Watcher + Send>,
    exclusions: Arc<Exclusions>,
    report: RootReport,
}

/// Shared between the public handle and the watcher thread.
struct WatcherCore {
    config: WatchConfig,
    logger: Arc<Logger>,
    listener: ChangeListener,
    events_tx: Sender<RawEvent>,
    debouncer: Mutex<Debouncer>,
    rate: Mutex<RateMeter>,
    roots: Mutex<HashMap<PathBuf, RootEntry>>,
    degraded: AtomicU32,
    shutdown: AtomicBool,
}

/// The file watcher.
///
/// Watching stops when this is dropped: the OS registrations are released and
/// the thread is joined, so a closed workspace does not leave handles behind
/// (REQ-FS-004, and the cleanup the workspace manager in Task 1.8 relies on).
pub struct FsWatcher {
    core: Arc<WatcherCore>,
    thread: Option<JoinHandle<()>>,
}

impl FsWatcher {
    /// Start the watcher thread. No roots are watched until [`watch`] is
    /// called.
    pub fn new(config: WatchConfig, logger: Arc<Logger>, listener: ChangeListener) -> Self {
        let (events_tx, events_rx) = channel();
        let core = Arc::new(WatcherCore {
            debouncer: Mutex::new(Debouncer::new(config.debounce, config.max_hold)),
            rate: Mutex::new(RateMeter::new(Instant::now())),
            roots: Mutex::new(HashMap::new()),
            degraded: AtomicU32::new(0),
            shutdown: AtomicBool::new(false),
            config,
            logger,
            listener,
            events_tx,
        });

        let thread_core = core.clone();
        // A plain OS thread, not a tokio task: `notify` delivers on its own
        // threads and the loop is a blocking `recv_timeout`, which would
        // occupy a runtime worker for the process's whole life.
        let thread = std::thread::Builder::new()
            .name("helix-fs-watch".to_string())
            .spawn(move || thread_core.run(events_rx))
            .expect("the watcher thread must be spawnable");

        Self {
            core,
            thread: Some(thread),
        }
    }

    /// Begin watching `root` recursively.
    ///
    /// Watching an already-watched root re-reports it without re-registering,
    /// so a second window opening the same workspace is idempotent.
    pub fn watch(&self, root: impl AsRef<Path>) -> Result<RootReport, AppError> {
        self.core.watch(root.as_ref())
    }

    /// Stop watching `root` and discard its pending changes.
    pub fn unwatch(&self, root: impl AsRef<Path>) -> Result<(), AppError> {
        self.core.unwatch(root.as_ref())
    }

    pub fn stats(&self) -> WatchStats {
        self.core.stats()
    }

    /// Reports for every watched root, ordered by path.
    pub fn roots(&self) -> Vec<RootReport> {
        let roots = self.core.roots.lock().unwrap();
        let mut reports: Vec<RootReport> =
            roots.values().map(|entry| entry.report.clone()).collect();
        reports.sort_by(|a, b| a.root.cmp(&b.root));
        reports
    }

    /// The exclusion matcher for a watched root, so the read and list paths
    /// answer the same question the watcher does.
    pub fn exclusions_for(&self, root: impl AsRef<Path>) -> Option<Arc<Exclusions>> {
        self.core
            .roots
            .lock()
            .unwrap()
            .get(root.as_ref())
            .map(|entry| entry.exclusions.clone())
    }
}

impl Drop for FsWatcher {
    fn drop(&mut self) {
        self.core.shutdown.store(true, Ordering::SeqCst);
        // Dropping the registrations makes `notify` stop sending, and dropping
        // every sender is what ends the thread's `recv_timeout`.
        self.core.roots.lock().unwrap().clear();
        if let Some(thread) = self.thread.take() {
            // A panicked watcher thread must not panic the dropping thread in
            // turn, which would abort during unwind.
            let _ = thread.join();
        }
    }
}

impl WatcherCore {
    fn watch(&self, root: &Path) -> Result<RootReport, AppError> {
        let root = canonical(root);
        if let Some(existing) = self.roots.lock().unwrap().get(&root) {
            return Ok(existing.report.clone());
        }
        if !root.is_dir() {
            return Err(AppError::permanent(
                "FS_WATCH_ROOT_MISSING",
                format!("{} is not a directory that can be watched", root.display()),
            ));
        }

        let exclusions = Arc::new(Exclusions::build(&root, &self.config.exclusions));
        for glob in exclusions.invalid_globs() {
            log_warn!(
                self.logger,
                LOG_SOURCE,
                "exclusion pattern is not a valid glob and was skipped",
                "pattern" => glob.clone(),
                "root" => root.display().to_string(),
            );
        }

        // The scan is needed anyway to know the budget, and it is the cheapest
        // moment to also learn what to suggest excluding.
        let listing = listing::list(&root, &exclusions, true);
        let outcome = probe::probe_with_threshold(&root, self.config.latency_threshold);
        let mut mode = if outcome.remote {
            WatchMode::Polling
        } else {
            WatchMode::Native
        };
        let mut degraded_reason = None;

        let mut watcher = match self.spawn_watcher(&root, exclusions.clone(), mode) {
            Ok(watcher) => watcher,
            Err(error) if mode == WatchMode::Native => {
                // The inotify-limit case. Keep the root, more slowly.
                mode = WatchMode::Polling;
                degraded_reason = Some(error.clone());
                self.degraded.fetch_add(1, Ordering::Relaxed);
                log_warn!(
                    self.logger,
                    LOG_SOURCE,
                    "native watching was refused; falling back to polling for this root",
                    "root" => root.display().to_string(),
                    "error" => error.clone(),
                    "poll_interval_ms" => self.config.poll_interval.as_millis() as u64,
                    "suggestion" => "raise the OS watch limit or add exclusions",
                );
                self.spawn_watcher(&root, exclusions.clone(), mode)
                    .map_err(|error| {
                        AppError::transient(
                            "FS_WATCH_FAILED",
                            format!("could not watch {}: {error}", root.display()),
                        )
                    })?
            }
            Err(error) => {
                return Err(AppError::transient(
                    "FS_WATCH_FAILED",
                    format!("could not watch {}: {error}", root.display()),
                ));
            }
        };

        let recursive_mode = RecursiveMode::Recursive;
        watcher.watch(&root, recursive_mode).map_err(|error| {
            AppError::transient(
                "FS_WATCH_FAILED",
                format!("could not watch {}: {error}", root.display()),
            )
        })?;

        let over_budget = listing.directory_count > self.config.path_budget;
        let suggested_exclusions = if over_budget {
            suggest_exclusions(&listing)
        } else {
            Vec::new()
        };
        let report = RootReport {
            root: root.to_string_lossy().into_owned(),
            mode,
            watched_paths: listing.directory_count,
            files_seen: listing.file_count,
            over_budget,
            suggested_exclusions: suggested_exclusions.clone(),
            probe_latency_ms: outcome.latency.map(|l| l.as_millis() as u64),
            degraded_reason,
        };

        if over_budget {
            log_warn!(
                self.logger,
                LOG_SOURCE,
                "watched path budget exceeded; watching continues but consider excluding the directories listed",
                "root" => root.display().to_string(),
                "watched_paths" => listing.directory_count,
                "budget" => self.config.path_budget,
                "suggested_exclusions" => suggested_exclusions,
            );
        }
        if outcome.remote {
            log_info!(
                self.logger,
                LOG_SOURCE,
                "filesystem latency exceeds the native-watching threshold; polling this root",
                "root" => root.display().to_string(),
                "latency_ms" => outcome.latency.map(|l| l.as_millis() as u64).unwrap_or(0),
                "threshold_ms" => self.config.latency_threshold.as_millis() as u64,
            );
        }
        log_info!(
            self.logger,
            LOG_SOURCE,
            "watching root",
            "root" => root.display().to_string(),
            "mode" => mode.as_str(),
            "watched_paths" => listing.directory_count,
            "files" => listing.file_count,
        );

        self.roots.lock().unwrap().insert(
            root,
            RootEntry {
                watcher,
                exclusions,
                report: report.clone(),
            },
        );
        Ok(report)
    }

    fn unwatch(&self, root: &Path) -> Result<(), AppError> {
        let root = canonical(root);
        let removed = self.roots.lock().unwrap().remove(&root);
        match removed {
            Some(mut entry) => {
                // Explicit, rather than relying on the drop: an error here is
                // worth logging, and a dropped watcher swallows it.
                if let Err(error) = entry.watcher.unwatch(&root) {
                    log_debug!(
                        self.logger,
                        LOG_SOURCE,
                        "unregistering a watch reported an error; the registration is dropped regardless",
                        "root" => root.display().to_string(),
                        "error" => error.to_string(),
                    );
                }
                self.debouncer.lock().unwrap().forget_root(&root);
                log_info!(
                    self.logger,
                    LOG_SOURCE,
                    "stopped watching root",
                    "root" => root.display().to_string(),
                );
                Ok(())
            }
            None => Err(AppError::permanent(
                "FS_WATCH_NOT_WATCHED",
                format!("{} is not being watched", root.display()),
            )),
        }
    }

    /// Build a `notify` watcher whose callback filters and forwards events.
    fn spawn_watcher(
        &self,
        root: &Path,
        exclusions: Arc<Exclusions>,
        mode: WatchMode,
    ) -> Result<Box<dyn Watcher + Send>, String> {
        let tx = self.events_tx.clone();
        let owned_root = root.to_path_buf();
        let handler = move |result: notify::Result<notify::Event>| match result {
            Ok(event) => {
                for (path, kind) in translate(&event) {
                    // `is_dir` is a live check because the event does not carry
                    // it. A deleted path answers false, which is the best that
                    // can be known and is why the field is documented as
                    // best-effort.
                    let is_dir = path.is_dir();
                    if exclusions.is_excluded(&path, is_dir) {
                        continue;
                    }
                    let _ = tx.send(RawEvent::Change {
                        root: owned_root.clone(),
                        path,
                        kind,
                        is_dir,
                    });
                }
            }
            Err(error) => {
                let _ = tx.send(RawEvent::WatcherError {
                    root: owned_root.clone(),
                    message: error.to_string(),
                });
            }
        };

        match mode {
            WatchMode::Native => RecommendedWatcher::new(handler, NotifyConfig::default())
                .map(|watcher| Box::new(watcher) as Box<dyn Watcher + Send>)
                .map_err(|error| error.to_string()),
            WatchMode::Polling => PollWatcher::new(
                handler,
                NotifyConfig::default().with_poll_interval(self.config.poll_interval),
            )
            .map(|watcher| Box::new(watcher) as Box<dyn Watcher + Send>)
            .map_err(|error| error.to_string()),
        }
    }

    /// The watcher thread: drain, debounce, deliver.
    fn run(&self, events_rx: Receiver<RawEvent>) {
        loop {
            if self.shutdown.load(Ordering::SeqCst) {
                break;
            }

            // Sleep exactly as long as the earliest pending deadline allows,
            // and no longer than one window when nothing is pending. Neither
            // spinning nor oversleeping past a change's due time.
            let timeout = self
                .debouncer
                .lock()
                .unwrap()
                .next_deadline(Instant::now())
                .unwrap_or(self.config.debounce)
                .max(Duration::from_millis(1));

            match events_rx.recv_timeout(timeout) {
                Ok(event) => {
                    self.absorb(event);
                    // Drain whatever else is queued before flushing, so a
                    // burst is coalesced in one pass rather than one lock
                    // acquisition per event.
                    while let Ok(event) = events_rx.try_recv() {
                        self.absorb(event);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }

            self.deliver(self.debouncer.lock().unwrap().flush(Instant::now()));
        }

        // Anything already observed is delivered rather than lost to shutdown.
        let remaining = self.debouncer.lock().unwrap().flush_all();
        self.deliver(remaining);
    }

    fn absorb(&self, event: RawEvent) {
        match event {
            RawEvent::Change {
                root,
                path,
                kind,
                is_dir,
            } => {
                let now = Instant::now();
                self.debouncer
                    .lock()
                    .unwrap()
                    .record(&root, &path, kind, is_dir, now);
                self.rate.lock().unwrap().record(1, now);
            }
            RawEvent::WatcherError { root, message } => {
                self.degraded.fetch_add(1, Ordering::Relaxed);
                if let Some(entry) = self.roots.lock().unwrap().get_mut(&root) {
                    entry.report.mode = WatchMode::Polling;
                    entry.report.degraded_reason = Some(message.clone());
                }
                log_warn!(
                    self.logger,
                    LOG_SOURCE,
                    "the OS watcher reported an error; this root is degraded",
                    "root" => root.display().to_string(),
                    "error" => message,
                    "suggestion" => "raise the OS watch limit or add exclusions",
                );
            }
        }
    }

    fn deliver(&self, changes: Vec<FileChange>) {
        if changes.is_empty() {
            return;
        }
        (self.listener)(&changes);
    }

    fn stats(&self) -> WatchStats {
        let roots = self.roots.lock().unwrap();
        let debouncer = self.debouncer.lock().unwrap();
        WatchStats {
            roots: roots.len() as u32,
            watched_paths: roots.values().map(|entry| entry.report.watched_paths).sum(),
            polling_roots: roots
                .values()
                .filter(|entry| entry.report.mode == WatchMode::Polling)
                .count() as u32,
            over_budget_roots: roots
                .values()
                .filter(|entry| entry.report.over_budget)
                .count() as u32,
            events_seen: debouncer.events_seen(),
            changes_emitted: debouncer.changes_emitted(),
            dropped_as_noise: debouncer.dropped_as_noise(),
            events_per_second: self.rate.lock().unwrap().rate(Instant::now()),
            pending: debouncer.pending_count() as u32,
            degraded_roots: self.degraded.load(Ordering::Relaxed),
        }
    }
}

/// Turn one `notify` event into zero or more `(path, kind)` pairs.
///
/// Renames are the interesting case. A rename reported with both endpoints
/// becomes a delete of the old path and a create of the new one, which is what
/// it means to every consumer: the editor retargets the tab, the index moves
/// the entry. Reporting it as a single opaque "modify" would leave the old path
/// in the index forever.
fn translate(event: &notify::Event) -> Vec<(PathBuf, ChangeKind)> {
    match event.kind {
        EventKind::Access(_) => Vec::new(),
        EventKind::Create(_) => pair_all(event, ChangeKind::Created),
        EventKind::Remove(_) => pair_all(event, ChangeKind::Deleted),
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if event.paths.len() >= 2 => {
            vec![
                (event.paths[0].clone(), ChangeKind::Deleted),
                (event.paths[1].clone(), ChangeKind::Created),
            ]
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
            pair_all(event, ChangeKind::Deleted)
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::To)) => pair_all(event, ChangeKind::Created),
        // A bare rename with one endpoint (`RenameMode::Any`, which is what
        // several platforms report) cannot be resolved into a direction here.
        // "Modified" is the safe reading: the consumer re-reads the path, finds
        // it present or absent, and is right either way.
        _ => pair_all(event, ChangeKind::Modified),
    }
}

fn pair_all(event: &notify::Event, kind: ChangeKind) -> Vec<(PathBuf, ChangeKind)> {
    event
        .paths
        .iter()
        .map(|path| (path.clone(), kind))
        .collect()
}

/// Derive exclusion suggestions from the tree that blew the budget
/// (REQ-FS-004.6).
///
/// Directories are attributed to their top-level ancestor and the heaviest
/// ones are named. A suggestion is only worth making if acting on it helps, so
/// anything contributing under a fifth of the tree is left out: a list of ten
/// directories each saving 3% is not advice, it is a shrug.
fn suggest_exclusions(listing: &Listing) -> Vec<String> {
    let mut per_top_level: HashMap<&str, u32> = HashMap::new();
    for entry in listing.entries.iter().filter(|entry| entry.is_dir) {
        let top = entry
            .relative_path
            .split('/')
            .next()
            .unwrap_or(&entry.relative_path);
        *per_top_level.entry(top).or_default() += 1;
    }

    let total = listing.directory_count.max(1);
    let mut candidates: Vec<(&str, u32)> = per_top_level
        .into_iter()
        .filter(|(_, count)| count.saturating_mul(5) >= total)
        .collect();
    // Heaviest first; name ties broken alphabetically so the advice is stable
    // between runs.
    candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    candidates
        .into_iter()
        .take(5)
        .map(|(name, _)| format!("**/{name}"))
        .collect()
}

/// Resolve a root to a stable, comparable form.
///
/// Watch events arrive with resolved paths (macOS reports `/private/var/...`
/// where the caller passed `/var/...`), so a root stored unresolved would never
/// match its own events.
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listing::FileEntry;
    use crate::testutil::TempDir;
    use helix_log::LogLevel;
    use std::sync::mpsc::sync_channel;

    fn logger() -> Arc<Logger> {
        Arc::new(Logger::in_memory(LogLevel::Trace))
    }

    /// A listener plus a receiver the test can block on, so watcher tests wait
    /// for the event they expect instead of sleeping for a guessed duration.
    fn collecting_listener() -> (ChangeListener, Receiver<Vec<FileChange>>) {
        let (tx, rx) = sync_channel::<Vec<FileChange>>(256);
        let listener: ChangeListener = Arc::new(move |changes: &[FileChange]| {
            let _ = tx.try_send(changes.to_vec());
        });
        (listener, rx)
    }

    /// Wait for a change matching `predicate`, up to `timeout`.
    fn await_change(
        rx: &Receiver<Vec<FileChange>>,
        timeout: Duration,
        predicate: impl Fn(&FileChange) -> bool,
    ) -> Option<FileChange> {
        let deadline = Instant::now() + timeout;
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            match rx.recv_timeout(remaining) {
                Ok(batch) => {
                    if let Some(found) = batch.into_iter().find(&predicate) {
                        return Some(found);
                    }
                }
                Err(_) => return None,
            }
        }
        None
    }

    /// Generous relative to the 100ms surfacing budget in the task's demo
    /// criterion. The budget is asserted separately, in the integration test
    /// that measures it; this is a liveness bound for CI machines under load.
    const WAIT: Duration = Duration::from_secs(10);

    fn watcher(dir: &TempDir) -> (FsWatcher, Receiver<Vec<FileChange>>) {
        let (listener, rx) = collecting_listener();
        let watcher = FsWatcher::new(WatchConfig::default(), logger(), listener);
        watcher.watch(dir.path()).expect("the root must be watched");
        (watcher, rx)
    }

    #[test]
    fn an_external_create_surfaces() {
        let dir = TempDir::new("watch-create");
        let (_watcher, rx) = watcher(&dir);

        dir.write("new.rs", "fn main() {}\n");

        let change =
            await_change(&rx, WAIT, |c| c.path.ends_with("new.rs")).expect("a create must surface");
        // Some platforms report the first write to a new file as create, others
        // as create-then-modify; both coalesce to Created, and either way the
        // path must not be reported as deleted.
        assert_ne!(change.kind, ChangeKind::Deleted);
    }

    #[test]
    fn an_external_modify_surfaces() {
        let dir = TempDir::new("watch-modify");
        let path = dir.write("existing.rs", "before\n");
        let (_watcher, rx) = watcher(&dir);

        std::fs::write(&path, "after\n").unwrap();

        let change = await_change(&rx, WAIT, |c| c.path.ends_with("existing.rs"))
            .expect("a modification must surface");
        assert_ne!(change.kind, ChangeKind::Deleted);
    }

    #[test]
    fn an_external_delete_surfaces() {
        let dir = TempDir::new("watch-delete");
        let path = dir.write("doomed.rs", "temporary\n");
        let (_watcher, rx) = watcher(&dir);

        std::fs::remove_file(&path).unwrap();

        let change = await_change(&rx, WAIT, |c| c.path.ends_with("doomed.rs"))
            .expect("a delete must surface");
        assert_eq!(change.kind, ChangeKind::Deleted);
    }

    #[test]
    fn changes_inside_an_excluded_directory_never_surface() {
        // REQ-FS-004.4 on the live path, not just in the matcher's unit tests.
        let dir = TempDir::new("watch-excluded");
        dir.write(".gitignore", "*.log\n");
        dir.mkdir("node_modules/pkg");
        let (_watcher, rx) = watcher(&dir);

        dir.write("node_modules/pkg/index.js", "noise");
        dir.write("app.log", "noise");
        // A file that must surface, published after the noise: if it arrives
        // and the noise did not, the exclusions held.
        dir.write("real.rs", "fn main() {}");

        let change = await_change(&rx, WAIT, |c| c.path.ends_with("real.rs"));
        assert!(change.is_some(), "the unexcluded file must surface");

        let mut leaked = Vec::new();
        while let Ok(batch) = rx.recv_timeout(Duration::from_millis(200)) {
            leaked.extend(
                batch
                    .into_iter()
                    .filter(|c| c.path.contains("node_modules") || c.path.ends_with("app.log")),
            );
        }
        assert!(leaked.is_empty(), "excluded paths leaked: {leaked:?}");
    }

    #[test]
    fn an_atomic_write_surfaces_as_one_change_to_the_target() {
        // The temp file must not appear as its own create and delete, and the
        // target must not appear as deleted. This is the interaction between
        // the atomic writer and the watcher, and it is the one that would close
        // the user's editor tab if it were wrong.
        let dir = TempDir::new("watch-atomic");
        let path = dir.write("main.rs", "before\n");
        let (_watcher, rx) = watcher(&dir);

        crate::atomic::write_atomic_str(&path, "after\n").unwrap();

        let change = await_change(&rx, WAIT, |c| c.path.ends_with("main.rs"))
            .expect("the save must surface");
        assert_ne!(change.kind, ChangeKind::Deleted);

        let mut temp_changes = Vec::new();
        while let Ok(batch) = rx.recv_timeout(Duration::from_millis(200)) {
            temp_changes.extend(
                batch
                    .into_iter()
                    .filter(|c| c.path.contains(crate::atomic::TEMP_SUFFIX)),
            );
        }
        assert!(
            temp_changes.is_empty(),
            "write temporaries leaked: {temp_changes:?}"
        );
    }

    #[test]
    fn watching_reports_the_mode_and_the_path_count() {
        let dir = TempDir::new("watch-report");
        dir.write("src/main.rs", "fn main() {}");
        dir.write("src/lib/util.rs", "pub fn u() {}");
        let (watcher, _rx) = watcher(&dir);

        let reports = watcher.roots();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].mode, WatchMode::Native);
        assert_eq!(reports[0].watched_paths, 3, "root, src, src/lib");
        assert!(!reports[0].over_budget);
        assert!(reports[0].probe_latency_ms.is_some());
    }

    #[test]
    fn the_budget_warning_fires_with_concrete_suggestions() {
        // REQ-FS-004.6. The budget is lowered rather than building a 10,000
        // directory tree, which would make the suite minutes slower to test
        // the same comparison.
        let dir = TempDir::new("watch-budget");
        for index in 0..12 {
            dir.mkdir(&format!("generated/pkg{index}"));
        }
        dir.mkdir("src");

        let config = WatchConfig {
            path_budget: 5,
            exclusions: ExclusionConfig::permissive(),
            ..WatchConfig::default()
        };
        let (listener, _rx) = collecting_listener();
        let watcher = FsWatcher::new(config, logger(), listener);
        let report = watcher.watch(dir.path()).unwrap();

        assert!(report.over_budget);
        assert!(report.watched_paths > 5);
        assert_eq!(
            report.suggested_exclusions,
            vec!["**/generated".to_string()],
            "the directory actually responsible must be named"
        );
        assert_eq!(watcher.stats().over_budget_roots, 1);
    }

    #[test]
    fn a_slow_filesystem_is_watched_by_polling() {
        // Forced via the threshold rather than a real network share, which CI
        // does not have. The decision path is identical.
        let dir = TempDir::new("watch-polling");
        let config = WatchConfig {
            latency_threshold: Duration::from_nanos(1),
            poll_interval: Duration::from_millis(100),
            ..WatchConfig::default()
        };
        let (listener, rx) = collecting_listener();
        let watcher = FsWatcher::new(config, logger(), listener);
        let report = watcher.watch(dir.path()).unwrap();
        assert_eq!(report.mode, WatchMode::Polling);
        assert_eq!(watcher.stats().polling_roots, 1);

        // Polling must actually deliver, not merely be selected.
        dir.write("polled.rs", "fn main() {}");
        assert!(
            await_change(&rx, WAIT, |c| c.path.ends_with("polled.rs")).is_some(),
            "a polling watcher must still report changes"
        );
    }

    #[test]
    fn unwatching_stops_delivery() {
        let dir = TempDir::new("watch-unwatch");
        let (watcher, rx) = watcher(&dir);
        watcher.unwatch(dir.path()).unwrap();
        // Drain anything already queued from the setup.
        while rx.recv_timeout(Duration::from_millis(100)).is_ok() {}

        dir.write("after-unwatch.rs", "fn main() {}");

        assert!(
            await_change(&rx, Duration::from_millis(500), |c| c
                .path
                .ends_with("after-unwatch.rs"))
            .is_none()
        );
        assert_eq!(watcher.stats().roots, 0);
    }

    #[test]
    fn watching_the_same_root_twice_is_idempotent() {
        let dir = TempDir::new("watch-twice");
        let (watcher, _rx) = watcher(&dir);
        watcher.watch(dir.path()).unwrap();
        assert_eq!(watcher.stats().roots, 1);
    }

    #[test]
    fn watching_a_path_that_is_not_a_directory_is_a_typed_error() {
        let dir = TempDir::new("watch-not-a-dir");
        let file = dir.write("file.txt", "content");
        let (listener, _rx) = collecting_listener();
        let watcher = FsWatcher::new(WatchConfig::default(), logger(), listener);

        let error = watcher.watch(&file).expect_err("a file is not a root");
        assert_eq!(error.code, "FS_WATCH_ROOT_MISSING");
    }

    #[test]
    fn unwatching_a_root_that_was_never_watched_is_a_typed_error() {
        let dir = TempDir::new("watch-unknown");
        let (listener, _rx) = collecting_listener();
        let watcher = FsWatcher::new(WatchConfig::default(), logger(), listener);
        let error = watcher.unwatch(dir.path()).expect_err("nothing is watched");
        assert_eq!(error.code, "FS_WATCH_NOT_WATCHED");
    }

    #[test]
    fn stats_report_the_event_rate_for_health_monitoring() {
        // REQ-FS-004.8: watched-path count and event rate are both published.
        let dir = TempDir::new("watch-stats");
        let (watcher, rx) = watcher(&dir);
        dir.write("counted.rs", "fn main() {}");
        await_change(&rx, WAIT, |c| c.path.ends_with("counted.rs"));

        let stats = watcher.stats();
        assert_eq!(stats.roots, 1);
        assert!(stats.watched_paths >= 1);
        assert!(stats.events_seen >= 1);
        assert!(stats.changes_emitted >= 1);
    }

    #[test]
    fn the_rate_meter_reports_the_last_completed_window() {
        let start = Instant::now();
        let mut meter = RateMeter::new(start);
        meter.record(7, start + Duration::from_millis(500));
        assert_eq!(meter.rate(start + Duration::from_millis(600)), 0);
        assert_eq!(meter.rate(start + Duration::from_millis(1_100)), 7);
        // Two idle seconds means the last window was genuinely empty.
        assert_eq!(meter.rate(start + Duration::from_millis(4_000)), 0);
    }

    #[test]
    fn suggestions_name_the_heaviest_directories_and_ignore_the_trivial_ones() {
        let listing = Listing {
            entries: (0..40)
                .map(|index| dir_entry(&format!("generated/pkg{index}")))
                .chain(std::iter::once(dir_entry("src")))
                .collect(),
            directory_count: 42,
            ..Listing::default()
        };
        assert_eq!(
            suggest_exclusions(&listing),
            vec!["**/generated".to_string()]
        );
    }

    #[test]
    fn suggestions_are_empty_when_no_directory_dominates() {
        let listing = Listing {
            entries: (0..40).map(|i| dir_entry(&format!("d{i}"))).collect(),
            directory_count: 41,
            ..Listing::default()
        };
        assert!(suggest_exclusions(&listing).is_empty());
    }

    fn dir_entry(relative: &str) -> FileEntry {
        FileEntry {
            path: format!("/root/{relative}"),
            relative_path: relative.to_string(),
            name: relative.rsplit('/').next().unwrap().to_string(),
            is_dir: true,
            is_symlink: false,
            size: 0,
            modified_ms: None,
            readonly: false,
        }
    }
}
