//! The change model and the debouncer (Task 1.7, REQ-FS-004.1, .7).
//!
//! Raw OS events are not what a consumer wants. Saving one file produces
//! anywhere from one to six events depending on the platform and the editor's
//! write strategy; `git checkout` of a branch produces thousands in a burst.
//! The debouncer turns that into one change per path, which is what the editor,
//! the search index, and the git decorator all actually need.
//!
//! ## Coalescing is per path, and order-dependent
//!
//! Two events on one path within the window collapse to a single verdict, and
//! which verdict is not always the later one:
//!
//! | first    | then     | result   | why |
//! |----------|----------|----------|-----|
//! | Created  | Modified | Created  | it is still a new file to the consumer |
//! | Created  | Deleted  | *dropped* | net zero; a build's scratch file is not news |
//! | Modified | Deleted  | Deleted  | the file is gone, that is the fact |
//! | Deleted  | Created  | Modified | this is what an atomic replace looks like |
//!
//! The Deleted-then-Created row is the one that matters most in practice.
//! Every editor that saves safely, including this one, writes a temp file and
//! renames it over the target, and several platforms report that as a delete
//! followed by a create. Reporting it as such would make the editor close the
//! user's tab on every save.
//!
//! ## Two timers, not one
//!
//! Quiescence alone is not enough. A running build or a `cargo watch` can touch
//! the same file every few milliseconds indefinitely, and a purely
//! quiescence-based debouncer would never emit anything at all — the events
//! stop arriving only when the user has stopped caring. So an entry is emitted
//! when it has been quiet for [`Debouncer::window`] *or* when it has been held
//! for [`Debouncer::max_hold`], whichever comes first.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Debounce window (REQ-FS-004.7).
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(50);

/// Longest an entry is held before being emitted regardless of ongoing
/// activity. Ten windows: long enough that a normal save still coalesces,
/// short enough to stay inside the 100ms surfacing budget in the task's demo
/// criterion for the first event of a burst.
pub const DEFAULT_MAX_HOLD: Duration = Duration::from_millis(500);

/// What happened to a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Created,
    Modified,
    Deleted,
}

impl ChangeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeKind::Created => "created",
            ChangeKind::Modified => "modified",
            ChangeKind::Deleted => "deleted",
        }
    }

    /// Combine an earlier verdict with a later one. `None` means the two
    /// cancel out and nothing should be reported. See the table in the module
    /// docs.
    fn coalesce(self, later: ChangeKind) -> Option<ChangeKind> {
        use ChangeKind::{Created, Deleted, Modified};
        match (self, later) {
            (Created, Modified) => Some(Created),
            (Created, Deleted) => None,
            (Deleted, Created) => Some(Modified),
            (_, later) => Some(later),
        }
    }
}

impl std::fmt::Display for ChangeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One debounced change, as delivered to listeners and published on the
/// streaming channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct FileChange {
    /// The watched root this change belongs to, so a multi-root workspace can
    /// route it without re-deriving the owner from the path.
    pub root: String,
    pub path: String,
    pub kind: ChangeKind,
    /// Best-effort. A deleted path cannot be stat'ed, so this is what was known
    /// when the event arrived, not a fresh check.
    pub is_dir: bool,
    /// How many raw OS events collapsed into this one. Useful in the log when
    /// diagnosing an event storm, and it is the number that shows debouncing is
    /// doing something.
    pub coalesced: u32,
}

#[derive(Debug, Clone)]
struct Pending {
    root: PathBuf,
    kind: ChangeKind,
    is_dir: bool,
    first_seen: Instant,
    last_seen: Instant,
    coalesced: u32,
}

/// Collapses raw events into one change per path per window.
///
/// Deliberately free of I/O, threads, and wall-clock reads: every method takes
/// `now`. That is what makes the coalescing table above testable exactly rather
/// than approximately, with no sleeps in the test suite.
pub struct Debouncer {
    window: Duration,
    max_hold: Duration,
    pending: HashMap<PathBuf, Pending>,
    /// Raw events seen, for the event-rate metric (REQ-FS-004.8).
    events_seen: u64,
    changes_emitted: u64,
    dropped_as_noise: u64,
}

impl Debouncer {
    pub fn new(window: Duration, max_hold: Duration) -> Self {
        Self {
            window,
            // A max hold below the window would emit before coalescing could
            // happen, defeating the purpose. Clamped rather than rejected: a
            // misconfiguration should degrade, not fail to start.
            max_hold: max_hold.max(window),
            pending: HashMap::new(),
            events_seen: 0,
            changes_emitted: 0,
            dropped_as_noise: 0,
        }
    }

    /// A debouncer with the requirement's 50ms window.
    pub fn with_defaults() -> Self {
        Self::new(DEFAULT_DEBOUNCE, DEFAULT_MAX_HOLD)
    }

    pub fn window(&self) -> Duration {
        self.window
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn events_seen(&self) -> u64 {
        self.events_seen
    }

    pub fn changes_emitted(&self) -> u64 {
        self.changes_emitted
    }

    /// Raw events that cancelled out and were never reported.
    pub fn dropped_as_noise(&self) -> u64 {
        self.dropped_as_noise
    }

    /// Record one raw event.
    pub fn record(
        &mut self,
        root: &Path,
        path: &Path,
        kind: ChangeKind,
        is_dir: bool,
        now: Instant,
    ) {
        self.events_seen += 1;
        match self.pending.get_mut(path) {
            Some(existing) => match existing.kind.coalesce(kind) {
                Some(combined) => {
                    existing.kind = combined;
                    existing.last_seen = now;
                    existing.is_dir |= is_dir;
                    existing.coalesced += 1;
                }
                None => {
                    self.dropped_as_noise += existing.coalesced as u64 + 1;
                    self.pending.remove(path);
                }
            },
            None => {
                self.pending.insert(
                    path.to_path_buf(),
                    Pending {
                        root: root.to_path_buf(),
                        kind,
                        is_dir,
                        first_seen: now,
                        last_seen: now,
                        coalesced: 1,
                    },
                );
            }
        }
    }

    /// Emit every entry that has gone quiet or has been held too long.
    pub fn flush(&mut self, now: Instant) -> Vec<FileChange> {
        let window = self.window;
        let max_hold = self.max_hold;
        let ready: Vec<PathBuf> = self
            .pending
            .iter()
            .filter(|(_, pending)| {
                now.duration_since(pending.last_seen) >= window
                    || now.duration_since(pending.first_seen) >= max_hold
            })
            .map(|(path, _)| path.clone())
            .collect();
        self.take(ready)
    }

    /// Emit everything immediately, regardless of timing. Used when a watch is
    /// being torn down, so a change already observed is not lost to shutdown.
    pub fn flush_all(&mut self) -> Vec<FileChange> {
        let all: Vec<PathBuf> = self.pending.keys().cloned().collect();
        self.take(all)
    }

    /// Drop every pending entry for a root, without emitting. Used when a root
    /// is unwatched: the consumer has stopped caring about that subtree.
    pub fn forget_root(&mut self, root: &Path) {
        self.pending.retain(|_, pending| pending.root != root);
    }

    /// When the caller should next wake up to flush, or `None` when there is
    /// nothing pending. Lets the watcher thread sleep instead of spinning.
    pub fn next_deadline(&self, now: Instant) -> Option<Duration> {
        self.pending
            .values()
            .map(|pending| {
                let by_quiet = (pending.last_seen + self.window).saturating_duration_since(now);
                let by_hold = (pending.first_seen + self.max_hold).saturating_duration_since(now);
                by_quiet.min(by_hold)
            })
            .min()
    }

    fn take(&mut self, paths: Vec<PathBuf>) -> Vec<FileChange> {
        let mut changes = Vec::with_capacity(paths.len());
        for path in paths {
            if let Some(pending) = self.pending.remove(&path) {
                changes.push(FileChange {
                    root: pending.root.to_string_lossy().into_owned(),
                    path: path.to_string_lossy().into_owned(),
                    kind: pending.kind,
                    is_dir: pending.is_dir,
                    coalesced: pending.coalesced,
                });
            }
        }
        self.changes_emitted += changes.len() as u64;
        // Deterministic order, so a consumer applying changes in sequence gets
        // the same result every run and tests do not depend on hash iteration
        // order.
        changes.sort_by(|a, b| a.path.cmp(&b.path));
        changes
    }
}

impl Default for Debouncer {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from("/workspace")
    }

    fn path(name: &str) -> PathBuf {
        root().join(name)
    }

    #[test]
    fn a_single_event_is_emitted_once_the_window_has_passed() {
        let mut debouncer = Debouncer::with_defaults();
        let start = Instant::now();
        debouncer.record(&root(), &path("a.rs"), ChangeKind::Modified, false, start);

        assert!(
            debouncer
                .flush(start + Duration::from_millis(40))
                .is_empty(),
            "nothing may be emitted before the window elapses"
        );

        let changes = debouncer.flush(start + Duration::from_millis(60));
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].kind, ChangeKind::Modified);
        assert_eq!(changes[0].path, path("a.rs").to_string_lossy());
        assert_eq!(changes[0].coalesced, 1);
    }

    #[test]
    fn a_burst_on_one_path_collapses_to_one_change() {
        // REQ-FS-004.7 in one test: twenty events, one notification.
        let mut debouncer = Debouncer::with_defaults();
        let start = Instant::now();
        for tick in 0..20 {
            debouncer.record(
                &root(),
                &path("a.rs"),
                ChangeKind::Modified,
                false,
                start + Duration::from_millis(tick),
            );
        }
        let changes = debouncer.flush(start + Duration::from_millis(200));
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].coalesced, 20);
        assert_eq!(debouncer.events_seen(), 20);
        assert_eq!(debouncer.changes_emitted(), 1);
    }

    #[test]
    fn separate_paths_are_debounced_independently() {
        let mut debouncer = Debouncer::with_defaults();
        let start = Instant::now();
        debouncer.record(&root(), &path("a.rs"), ChangeKind::Created, false, start);
        debouncer.record(
            &root(),
            &path("b.rs"),
            ChangeKind::Modified,
            false,
            start + Duration::from_millis(40),
        );

        let first = debouncer.flush(start + Duration::from_millis(60));
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].kind, ChangeKind::Created);

        let second = debouncer.flush(start + Duration::from_millis(100));
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].kind, ChangeKind::Modified);
    }

    #[test]
    fn a_create_followed_by_a_modify_stays_a_create() {
        let mut debouncer = Debouncer::with_defaults();
        let start = Instant::now();
        debouncer.record(&root(), &path("new.rs"), ChangeKind::Created, false, start);
        debouncer.record(
            &root(),
            &path("new.rs"),
            ChangeKind::Modified,
            false,
            start + Duration::from_millis(5),
        );
        let changes = debouncer.flush(start + Duration::from_millis(100));
        assert_eq!(changes[0].kind, ChangeKind::Created);
    }

    #[test]
    fn a_create_followed_by_a_delete_is_dropped_entirely() {
        let mut debouncer = Debouncer::with_defaults();
        let start = Instant::now();
        debouncer.record(&root(), &path("tmp.o"), ChangeKind::Created, false, start);
        debouncer.record(
            &root(),
            &path("tmp.o"),
            ChangeKind::Deleted,
            false,
            start + Duration::from_millis(5),
        );
        assert!(
            debouncer
                .flush(start + Duration::from_millis(100))
                .is_empty(),
            "a file that appeared and vanished inside the window is not news"
        );
        assert_eq!(debouncer.dropped_as_noise(), 2);
    }

    #[test]
    fn a_delete_followed_by_a_create_is_reported_as_a_modification() {
        // This is what an atomic save looks like on several platforms.
        // Reporting the delete would close the user's editor tab.
        let mut debouncer = Debouncer::with_defaults();
        let start = Instant::now();
        debouncer.record(&root(), &path("main.rs"), ChangeKind::Deleted, false, start);
        debouncer.record(
            &root(),
            &path("main.rs"),
            ChangeKind::Created,
            false,
            start + Duration::from_millis(2),
        );
        let changes = debouncer.flush(start + Duration::from_millis(100));
        assert_eq!(changes[0].kind, ChangeKind::Modified);
    }

    #[test]
    fn a_modify_followed_by_a_delete_reports_the_delete() {
        let mut debouncer = Debouncer::with_defaults();
        let start = Instant::now();
        debouncer.record(
            &root(),
            &path("gone.rs"),
            ChangeKind::Modified,
            false,
            start,
        );
        debouncer.record(
            &root(),
            &path("gone.rs"),
            ChangeKind::Deleted,
            false,
            start + Duration::from_millis(2),
        );
        let changes = debouncer.flush(start + Duration::from_millis(100));
        assert_eq!(changes[0].kind, ChangeKind::Deleted);
    }

    #[test]
    fn continuous_activity_still_emits_at_the_max_hold() {
        // Without the second timer this test would hang forever in production:
        // the window never elapses while a build keeps touching the file.
        let mut debouncer = Debouncer::new(Duration::from_millis(50), Duration::from_millis(500));
        let start = Instant::now();
        let mut now = start;
        for _ in 0..100 {
            debouncer.record(&root(), &path("busy.log"), ChangeKind::Modified, false, now);
            now += Duration::from_millis(10);
            if !debouncer.flush(now).is_empty() {
                let held = now.duration_since(start);
                assert!(held >= Duration::from_millis(500), "emitted too early");
                assert!(
                    held <= Duration::from_millis(600),
                    "held too long: {held:?}"
                );
                return;
            }
        }
        panic!("a continuously touched path was never emitted");
    }

    #[test]
    fn flush_all_emits_pending_changes_regardless_of_timing() {
        let mut debouncer = Debouncer::with_defaults();
        let now = Instant::now();
        debouncer.record(&root(), &path("a.rs"), ChangeKind::Modified, false, now);
        assert_eq!(debouncer.flush_all().len(), 1);
        assert_eq!(debouncer.pending_count(), 0);
    }

    #[test]
    fn unwatching_a_root_discards_its_pending_changes_only() {
        let mut debouncer = Debouncer::with_defaults();
        let now = Instant::now();
        let other = PathBuf::from("/other");
        debouncer.record(&root(), &path("a.rs"), ChangeKind::Modified, false, now);
        debouncer.record(
            &other,
            &other.join("b.rs"),
            ChangeKind::Modified,
            false,
            now,
        );

        debouncer.forget_root(&root());
        let remaining = debouncer.flush_all();
        assert_eq!(remaining.len(), 1);
        assert!(remaining[0].path.contains("b.rs"));
    }

    #[test]
    fn the_next_deadline_lets_the_caller_sleep_instead_of_spinning() {
        let mut debouncer = Debouncer::with_defaults();
        let start = Instant::now();
        assert_eq!(debouncer.next_deadline(start), None);

        debouncer.record(&root(), &path("a.rs"), ChangeKind::Modified, false, start);
        let deadline = debouncer.next_deadline(start).unwrap();
        assert!(deadline <= DEFAULT_DEBOUNCE && deadline > Duration::ZERO);
    }

    #[test]
    fn emitted_changes_are_ordered_by_path() {
        let mut debouncer = Debouncer::with_defaults();
        let start = Instant::now();
        for name in ["c.rs", "a.rs", "b.rs"] {
            debouncer.record(&root(), &path(name), ChangeKind::Modified, false, start);
        }
        let changes = debouncer.flush(start + Duration::from_millis(100));
        let paths: Vec<&String> = changes.iter().map(|c| &c.path).collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted);
    }

    #[test]
    fn a_max_hold_below_the_window_is_clamped_rather_than_breaking_coalescing() {
        let debouncer = Debouncer::new(Duration::from_millis(50), Duration::from_millis(1));
        assert_eq!(debouncer.max_hold, Duration::from_millis(50));
    }
}
