use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crc32fast::Hasher;
use helix_fs::write_atomic;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    AgentState, BufferState, RecoveryReport, SessionSnapshot, StateMutation, TerminalState,
};

pub const DEFAULT_WAL_INTERVAL: Duration = Duration::from_millis(1_000);
pub const AUXILIARY_WAL_INTERVAL: Duration = Duration::from_secs(5);
pub const DEFAULT_SNAPSHOT_INTERVAL: Duration = Duration::from_secs(5 * 60);
pub const DEFAULT_RETENTION: Duration = Duration::from_secs(30 * 24 * 60 * 60);

const WAL_DIR: &str = "wal";
const WAL_FILE: &str = "state.jsonl";
const SNAPSHOT_FILE: &str = "snapshot.json";
const PREVIOUS_SNAPSHOT_FILE: &str = "snapshot.previous.json";
const METADATA_FILE: &str = "workspace.json";

#[derive(Debug, Error)]
pub enum StateError {
    #[error("state persistence I/O failed at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("state serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("state data is invalid: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PersistenceStatus {
    pub degraded: bool,
    pub last_error: Option<String>,
    pub wal_entries_written: u64,
    pub corrupt_entries_discarded: u64,
    pub pending_entries: usize,
}

#[derive(Debug, Clone)]
pub struct StateStoreConfig {
    pub wal_interval: Duration,
    pub auxiliary_wal_interval: Duration,
    pub snapshot_interval: Duration,
    pub retention: Duration,
}

impl Default for StateStoreConfig {
    fn default() -> Self {
        Self {
            wal_interval: DEFAULT_WAL_INTERVAL,
            auxiliary_wal_interval: AUXILIARY_WAL_INTERVAL,
            snapshot_interval: DEFAULT_SNAPSHOT_INTERVAL,
            retention: DEFAULT_RETENTION,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Checked<T> {
    timestamp_ms: u64,
    payload: T,
    crc32: u32,
}

impl<T: Serialize> Checked<T> {
    fn new(timestamp_ms: u64, payload: T) -> Result<Self, StateError> {
        let crc32 = checksum(timestamp_ms, &payload)?;
        Ok(Self {
            timestamp_ms,
            payload,
            crc32,
        })
    }

    fn valid(&self) -> bool {
        checksum(self.timestamp_ms, &self.payload)
            .map(|actual| actual == self.crc32)
            .unwrap_or(false)
    }
}

#[derive(Debug)]
struct Pending {
    mutation: StateMutation,
    queued_ms: u64,
}

#[derive(Debug, Default)]
struct Inner {
    pending: BTreeMap<String, Pending>,
    status: PersistenceStatus,
    last_snapshot_ms: u64,
}

/// Durable state for one workspace. Callers queue high-frequency changes;
/// [`flush_due`](Self::flush_due) coalesces them by entity and fsyncs only at
/// the configured RPO. [`flush_all`](Self::flush_all) is the clean-shutdown path.
pub struct StateStore {
    root: PathBuf,
    workspace_key: String,
    roots: Vec<PathBuf>,
    config: StateStoreConfig,
    inner: Mutex<Inner>,
}

impl StateStore {
    pub fn new(
        root: impl Into<PathBuf>,
        workspace_key: impl Into<String>,
        roots: Vec<PathBuf>,
        config: StateStoreConfig,
    ) -> Self {
        Self {
            root: root.into(),
            workspace_key: workspace_key.into(),
            roots,
            config,
            inner: Mutex::new(Inner::default()),
        }
    }

    pub fn for_workspace(
        workspace_id: Option<&str>,
        roots: Vec<PathBuf>,
        config: StateStoreConfig,
    ) -> Result<Self, StateError> {
        let key = helix_workspace::workspace_key(workspace_id, &roots);
        let root = workspace_state_directory(&key).ok_or_else(|| {
            StateError::Invalid("the operating-system state directory could not be resolved".into())
        })?;
        Ok(Self::new(root, key, roots, config))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn workspace_key(&self) -> &str {
        &self.workspace_key
    }

    pub fn queue_buffer(&self, state: BufferState, now_ms: u64) {
        self.queue(
            format!("buffer:{}", state.id),
            StateMutation::Buffer(state),
            now_ms,
        );
    }

    pub fn queue_terminal(&self, state: TerminalState, now_ms: u64) {
        self.queue(
            format!("terminal:{}", state.id),
            StateMutation::Terminal(state),
            now_ms,
        );
    }

    pub fn queue_agent(&self, state: AgentState, now_ms: u64) {
        self.queue(
            format!("agent:{}", state.id),
            StateMutation::Agent(state),
            now_ms,
        );
    }

    fn queue(&self, key: String, mutation: StateMutation, now_ms: u64) {
        let mut inner = self.lock();
        inner.pending.insert(
            key,
            Pending {
                mutation,
                queued_ms: now_ms,
            },
        );
        inner.status.pending_entries = inner.pending.len();
    }

    /// Flush entries whose coalescing interval elapsed. WAL is prioritized
    /// before the optional snapshot, including while degraded by disk errors.
    pub fn flush_due(&self, now_ms: u64, session: &SessionSnapshot) -> Result<(), StateError> {
        let due = {
            let mut inner = self.lock();
            let keys: Vec<String> = inner
                .pending
                .iter()
                .filter_map(|(key, pending)| {
                    let interval = match pending.mutation {
                        StateMutation::Buffer(_) => self.config.wal_interval,
                        _ => self.config.auxiliary_wal_interval,
                    };
                    (now_ms.saturating_sub(pending.queued_ms) >= interval.as_millis() as u64)
                        .then(|| key.clone())
                })
                .collect();
            keys.into_iter()
                .filter_map(|key| inner.pending.remove(&key).map(|p| (key, p)))
                .collect::<Vec<_>>()
        };

        if let Err(error) = self.append_pending(now_ms, &due) {
            self.restore_pending(due);
            self.degrade(&error);
            return Err(error);
        }
        self.mark_healthy();

        let snapshot_due = now_ms.saturating_sub(self.lock().last_snapshot_ms)
            >= self.config.snapshot_interval.as_millis() as u64;
        if snapshot_due && let Err(error) = self.write_snapshot(session, now_ms) {
            self.degrade(&error);
            return Err(error);
        }
        Ok(())
    }

    /// Flush every queued mutation regardless of age. Used during graceful shutdown.
    pub fn flush_all(&self, now_ms: u64) -> Result<(), StateError> {
        let pending = {
            let mut inner = self.lock();
            std::mem::take(&mut inner.pending)
                .into_iter()
                .collect::<Vec<_>>()
        };
        if let Err(error) = self.append_pending(now_ms, &pending) {
            self.restore_pending(pending);
            self.degrade(&error);
            return Err(error);
        }
        self.mark_healthy();
        Ok(())
    }

    pub fn write_snapshot(&self, session: &SessionSnapshot, now_ms: u64) -> Result<(), StateError> {
        self.ensure_root()?;
        let mut session = session.clone();
        session.timestamp_ms = now_ms;
        session.workspace_key.clone_from(&self.workspace_key);
        session.roots = self
            .roots
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let checked = Checked::new(now_ms, session)?;
        let bytes = serde_json::to_vec(&checked)?;
        let current = self.root.join(SNAPSHOT_FILE);
        if current.exists() {
            let old = fs::read(&current).map_err(|source| io_error(&current, source))?;
            write_atomic(self.root.join(PREVIOUS_SNAPSHOT_FILE), &old)
                .map_err(|source| io_error(self.root.join(PREVIOUS_SNAPSHOT_FILE), source))?;
        }
        write_atomic(&current, &bytes).map_err(|source| io_error(&current, source))?;
        self.write_metadata(now_ms)?;
        self.lock().last_snapshot_ms = now_ms;
        self.mark_healthy();
        Ok(())
    }

    /// Load the newest valid snapshot, then replay valid WAL entries newer than it.
    pub fn recover(&self) -> Result<RecoveryReport, StateError> {
        let (mut session, snapshot_corrupt) = self.read_snapshot_chain()?;
        if session.workspace_key.is_empty() {
            session.workspace_key.clone_from(&self.workspace_key);
        }
        if session.roots.is_empty() {
            session.roots = self
                .roots
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
        }
        let mut discarded = 0;
        let path = self.wal_path();
        if path.exists() {
            let file = fs::File::open(&path).map_err(|source| io_error(&path, source))?;
            for line in BufReader::new(file).lines() {
                let Ok(line) = line else {
                    discarded += 1;
                    continue;
                };
                let Ok(entry) = serde_json::from_str::<Checked<StateMutation>>(&line) else {
                    discarded += 1;
                    continue;
                };
                if !entry.valid() {
                    discarded += 1;
                    continue;
                }
                if entry.timestamp_ms > session.timestamp_ms {
                    apply(&mut session, entry.payload);
                }
            }
        }
        // The buffer remains useful even when its network volume or checkout
        // is gone. It must be dirty so a later close cannot discard the only
        // recovered copy without asking, and the target stays attached for a
        // deferred save when that location becomes reachable again.
        for buffer in &mut session.buffers {
            if buffer
                .target
                .as_deref()
                .is_some_and(|target| !Path::new(target).exists())
            {
                buffer.dirty = true;
            }
        }
        let mut inner = self.lock();
        inner.last_snapshot_ms = session.timestamp_ms;
        inner.status.corrupt_entries_discarded += discarded;
        Ok(RecoveryReport {
            session,
            discarded_entries: discarded,
            snapshot_corrupt,
        })
    }

    pub fn status(&self) -> PersistenceStatus {
        let mut status = self.lock().status.clone();
        status.pending_entries = self.lock().pending.len();
        status
    }

    fn append_pending(&self, now_ms: u64, pending: &[(String, Pending)]) -> Result<(), StateError> {
        if pending.is_empty() {
            return Ok(());
        }
        self.ensure_root()?;
        let path = self.wal_path();
        fs::create_dir_all(path.parent().unwrap())
            .map_err(|source| io_error(path.parent().unwrap(), source))?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|source| io_error(&path, source))?;
        for (_, item) in pending {
            let checked = Checked::new(now_ms, item.mutation.clone())?;
            serde_json::to_writer(&mut file, &checked)?;
            file.write_all(b"\n")
                .map_err(|source| io_error(&path, source))?;
        }
        file.flush()
            .and_then(|_| file.sync_data())
            .map_err(|source| io_error(&path, source))?;
        self.lock().status.wal_entries_written += pending.len() as u64;
        Ok(())
    }

    fn restore_pending(&self, pending: Vec<(String, Pending)>) {
        let mut inner = self.lock();
        for (key, value) in pending {
            inner.pending.entry(key).or_insert(value);
        }
        inner.status.pending_entries = inner.pending.len();
    }

    fn ensure_root(&self) -> Result<(), StateError> {
        fs::create_dir_all(&self.root).map_err(|source| io_error(&self.root, source))
    }

    fn wal_path(&self) -> PathBuf {
        self.root.join(WAL_DIR).join(WAL_FILE)
    }

    fn read_snapshot_chain(&self) -> Result<(SessionSnapshot, bool), StateError> {
        let mut saw_corrupt = false;
        for name in [SNAPSHOT_FILE, PREVIOUS_SNAPSHOT_FILE] {
            let path = self.root.join(name);
            if !path.exists() {
                continue;
            }
            match fs::read(&path)
                .map_err(|source| io_error(&path, source))
                .and_then(|bytes| {
                    serde_json::from_slice::<Checked<SessionSnapshot>>(&bytes)
                        .map_err(StateError::from)
                }) {
                Ok(checked) if checked.valid() => return Ok((checked.payload, saw_corrupt)),
                Ok(_) | Err(StateError::Serialization(_)) => saw_corrupt = true,
                Err(error) => return Err(error),
            }
        }
        Ok((SessionSnapshot::default(), saw_corrupt))
    }

    fn write_metadata(&self, now_ms: u64) -> Result<(), StateError> {
        #[derive(Serialize)]
        struct Metadata<'a> {
            workspace_key: &'a str,
            roots: Vec<String>,
            touched_ms: u64,
        }
        let metadata = Metadata {
            workspace_key: &self.workspace_key,
            roots: self
                .roots
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            touched_ms: now_ms,
        };
        let bytes = serde_json::to_vec(&metadata)?;
        let path = self.root.join(METADATA_FILE);
        write_atomic(&path, &bytes).map_err(|source| io_error(&path, source))
    }

    fn degrade(&self, error: &StateError) {
        let mut inner = self.lock();
        inner.status.degraded = true;
        inner.status.last_error = Some(error.to_string());
    }

    fn mark_healthy(&self) {
        let mut inner = self.lock();
        inner.status.degraded = false;
        inner.status.last_error = None;
        inner.status.pending_entries = inner.pending.len();
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// Delete expired state directories only when every recorded root is unavailable.
pub fn prune_stale_state(
    state_root: &Path,
    now_ms: u64,
    retention: Duration,
) -> Result<Vec<PathBuf>, StateError> {
    #[derive(Deserialize)]
    struct Metadata {
        roots: Vec<String>,
        touched_ms: u64,
    }
    let mut removed = Vec::new();
    if !state_root.exists() {
        return Ok(removed);
    }
    for entry in fs::read_dir(state_root).map_err(|source| io_error(state_root, source))? {
        let entry = entry.map_err(|source| io_error(state_root, source))?;
        if !entry
            .file_type()
            .map_err(|source| io_error(entry.path(), source))?
            .is_dir()
        {
            continue;
        }
        let metadata_path = entry.path().join(METADATA_FILE);
        let Ok(bytes) = fs::read(&metadata_path) else {
            continue;
        };
        let Ok(metadata) = serde_json::from_slice::<Metadata>(&bytes) else {
            continue;
        };
        let expired = now_ms.saturating_sub(metadata.touched_ms) >= retention.as_millis() as u64;
        if expired && metadata.roots.iter().all(|root| !Path::new(root).exists()) {
            fs::remove_dir_all(entry.path()).map_err(|source| io_error(entry.path(), source))?;
            removed.push(entry.path());
        }
    }
    Ok(removed)
}

pub fn workspace_state_directory(key: &str) -> Option<PathBuf> {
    let component: String = key
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let component = if component.is_empty() {
        "workspace"
    } else {
        &component
    };
    if cfg!(target_os = "windows") {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|p| p.join("Helix").join("state").join(component))
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(PathBuf::from).map(|p| {
            p.join("Library")
                .join("Application Support")
                .join("Helix")
                .join("state")
                .join(component)
        })
    } else {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|p| p.join(".local").join("state"))
            })
            .map(|p| p.join("helix").join("state").join(component))
    }
}

/// Platform directory containing every keyed workspace state directory.
pub fn state_root_directory() -> Option<PathBuf> {
    workspace_state_directory("__probe__").and_then(|path| path.parent().map(Path::to_path_buf))
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

fn checksum<T: Serialize>(timestamp_ms: u64, payload: &T) -> Result<u32, StateError> {
    let mut hasher = Hasher::new();
    hasher.update(&timestamp_ms.to_le_bytes());
    hasher.update(&serde_json::to_vec(payload)?);
    Ok(hasher.finalize())
}

fn apply(session: &mut SessionSnapshot, mutation: StateMutation) {
    match mutation {
        StateMutation::Buffer(value) => upsert(&mut session.buffers, value, |v| &v.id),
        StateMutation::Terminal(value) => upsert(&mut session.terminals, value, |v| &v.id),
        StateMutation::Agent(value) => upsert(&mut session.agents, value, |v| &v.id),
    }
}

fn upsert<T, F>(values: &mut Vec<T>, value: T, id: F)
where
    F: Fn(&T) -> &str,
{
    if let Some(index) = values.iter().position(|current| id(current) == id(&value)) {
        values[index] = value;
    } else {
        values.push(value);
    }
}

fn io_error(path: impl Into<PathBuf>, source: std::io::Error) -> StateError {
    StateError::Io {
        path: path.into(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_fs::testutil::TempDir;

    fn buffer(content: &str, target: Option<&Path>) -> BufferState {
        BufferState {
            id: "editor-1".into(),
            content: content.into(),
            language: "rust".into(),
            target: target.map(|p| p.to_string_lossy().into_owned()),
            dirty: true,
            cursor_line: 3,
            cursor_column: 7,
        }
    }

    fn test_store(dir: &TempDir) -> StateStore {
        StateStore::new(
            dir.path().join("state/workspace"),
            "workspace",
            vec![dir.path().join("readonly-root")],
            StateStoreConfig::default(),
        )
    }

    #[test]
    fn buffer_updates_are_coalesced_and_recovered_after_a_hard_kill() {
        let dir = TempDir::new("state-hard-kill");
        let store = test_store(&dir);
        store.queue_buffer(buffer("one", None), 0);
        store.queue_buffer(buffer("two", None), 500);
        store.flush_due(1_499, &SessionSnapshot::default()).unwrap();
        assert_eq!(store.status().wal_entries_written, 0);
        store.flush_due(1_500, &SessionSnapshot::default()).unwrap();
        drop(store);

        let recovered = test_store(&dir).recover().unwrap();
        assert_eq!(recovered.session.buffers[0].content, "two");
        assert_eq!(recovered.discarded_entries, 0);
    }

    #[test]
    fn graceful_shutdown_flushes_an_entry_that_is_not_due() {
        let dir = TempDir::new("state-clean-stop");
        let store = test_store(&dir);
        store.queue_buffer(buffer("last keystroke", None), 900);
        store.flush_all(901).unwrap();
        assert_eq!(
            test_store(&dir).recover().unwrap().session.buffers[0].content,
            "last keystroke"
        );
    }

    #[test]
    fn terminal_state_flushes_every_five_seconds() {
        let dir = TempDir::new("state-terminal-interval");
        let store = test_store(&dir);
        store.queue_terminal(
            TerminalState {
                id: "terminal-1".into(),
                shell: "/bin/sh".into(),
                cwd: "/work".into(),
                scrollback: "output".into(),
            },
            0,
        );
        store.flush_due(4_999, &SessionSnapshot::default()).unwrap();
        assert_eq!(store.status().wal_entries_written, 0);
        store.flush_due(5_000, &SessionSnapshot::default()).unwrap();
        assert_eq!(store.status().wal_entries_written, 1);
        assert_eq!(store.recover().unwrap().session.terminals.len(), 1);
    }

    #[test]
    fn session_snapshot_is_written_on_the_five_minute_cadence() {
        let dir = TempDir::new("state-snapshot-interval");
        let store = test_store(&dir);
        let mut session = SessionSnapshot::default();
        session.buffers.push(buffer("snapshot", None));
        store.flush_due(299_999, &session).unwrap();
        assert!(!store.root.join(SNAPSHOT_FILE).exists());
        store.flush_due(300_000, &session).unwrap();
        assert_eq!(
            store.recover().unwrap().session.buffers[0].content,
            "snapshot"
        );
    }

    #[test]
    fn corrupt_wal_entries_are_discarded_without_losing_later_valid_entries() {
        let dir = TempDir::new("state-corrupt-wal");
        let store = test_store(&dir);
        store.queue_buffer(buffer("first", None), 0);
        store.flush_all(1).unwrap();
        let path = store.wal_path();
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{{ definitely not json }}").unwrap();
        store.queue_buffer(buffer("last valid", None), 2);
        store.flush_all(3).unwrap();

        let recovered = store.recover().unwrap();
        assert_eq!(recovered.session.buffers[0].content, "last valid");
        assert_eq!(recovered.discarded_entries, 1);
    }

    #[test]
    fn a_corrupt_current_snapshot_falls_back_to_the_previous_valid_one() {
        let dir = TempDir::new("state-corrupt-snapshot");
        let store = test_store(&dir);
        let mut first = SessionSnapshot::default();
        first.buffers.push(buffer("safe", None));
        store.write_snapshot(&first, 10).unwrap();
        let mut second = first.clone();
        second.buffers[0].content = "new".into();
        store.write_snapshot(&second, 20).unwrap();
        fs::write(store.root.join(SNAPSHOT_FILE), b"corrupt").unwrap();

        let recovered = store.recover().unwrap();
        assert!(recovered.snapshot_corrupt);
        assert_eq!(recovered.session.buffers[0].content, "safe");
    }

    #[test]
    fn unavailable_and_read_only_targets_do_not_block_recovery() {
        let dir = TempDir::new("state-unavailable-root");
        let missing = dir.path().join("unmounted/file.rs");
        let store = test_store(&dir);
        let mut state = buffer("recover me", Some(&missing));
        state.dirty = false;
        store.queue_buffer(state, 0);
        store.flush_all(1).unwrap();
        let recovered = store.recover().unwrap();
        assert_eq!(
            recovered.session.buffers[0].target.as_deref(),
            Some(missing.to_string_lossy().as_ref())
        );
        assert!(recovered.session.buffers[0].dirty);
        assert!(!missing.exists());
    }

    #[test]
    fn a_write_failure_degrades_and_retains_pending_work_for_retry() {
        let dir = TempDir::new("state-disk-full");
        let blocker = dir.write("not-a-directory", b"x");
        let store = StateStore::new(
            blocker.join("workspace"),
            "workspace",
            vec![],
            StateStoreConfig::default(),
        );
        store.queue_buffer(buffer("keep", None), 0);
        assert!(store.flush_all(1).is_err());
        let status = store.status();
        assert!(status.degraded);
        assert_eq!(status.pending_entries, 1);
        assert!(status.last_error.is_some());
    }

    #[test]
    fn workspace_keys_are_stable_for_reordered_and_symlinked_roots() {
        let dir = TempDir::new("state-key");
        let a = dir.mkdir("a");
        let b = dir.mkdir("b");
        assert_eq!(
            helix_workspace::workspace_key(None, &[a.clone(), b.clone()]),
            helix_workspace::workspace_key(None, &[b, a.clone()])
        );
        #[cfg(unix)]
        {
            let link = dir.path().join("link-a");
            std::os::unix::fs::symlink(&a, &link).unwrap();
            assert_eq!(
                helix_workspace::workspace_key(None, std::slice::from_ref(&a)),
                helix_workspace::workspace_key(None, &[link])
            );
        }
    }

    #[test]
    fn retention_only_prunes_expired_workspaces_with_no_existing_root() {
        let dir = TempDir::new("state-retention");
        let state_root = dir.mkdir("states");
        let existing_root = dir.mkdir("live-root");
        let stale = StateStore::new(
            state_root.join("stale"),
            "stale",
            vec![dir.path().join("gone")],
            StateStoreConfig::default(),
        );
        stale
            .write_snapshot(&SessionSnapshot::default(), 1)
            .unwrap();
        let live = StateStore::new(
            state_root.join("live"),
            "live",
            vec![existing_root],
            StateStoreConfig::default(),
        );
        live.write_snapshot(&SessionSnapshot::default(), 1).unwrap();
        let removed = prune_stale_state(
            &state_root,
            DEFAULT_RETENTION.as_millis() as u64 + 2,
            DEFAULT_RETENTION,
        )
        .unwrap();
        assert_eq!(removed, vec![state_root.join("stale")]);
        assert!(state_root.join("live").exists());
    }
}
