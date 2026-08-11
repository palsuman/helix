//! `helix-fs` — the file system service (Task 1.7, REQ-FS-004, REQ-ED-006,
//! REQ-NFR-002).
//!
//! Every byte the application reads from or writes to disk goes through here.
//! That is the point: encoding detection, line-ending fidelity, crash-safe
//! writes, exclusions, and change notification are each easy to get subtly
//! wrong, and there is exactly one place to get them right.
//!
//! ```text
//!  fs.read  ──► encoding detect ─► EOL detect ─► LF-normalised text + hash
//!  fs.write ──► EOL apply ─► encode ─► temp file ─► fsync ─► rename
//!  fs.list  ──► one walk, gitignore + configured globs, stat per entry
//!  fs.watch ──► notify (native or polling) ─► 50ms debounce ─► fs:changed
//! ```
//!
//! Module map:
//!
//! - [`encoding`] — UTF-8, UTF-16 LE/BE and Latin-1 detection by mark plus
//!   heuristics, and binary detection.
//! - [`eol`] — LF/CRLF/mixed detection and the normalisation boundary.
//! - [`hash`] — xxHash content hashing for dirty detection and index
//!   invalidation.
//! - [`atomic`] — the temp-fsync-rename write, and the crash points its tests
//!   exercise.
//! - [`exclude`] — `.gitignore` plus configured globs, one matcher for the
//!   lister and the watcher.
//! - [`listing`] — the directory walk and per-entry stat information.
//! - [`change`] — the change model and the 50ms debouncer.
//! - [`probe`] — network filesystem detection by latency.
//! - [`watch`] — the `notify`-backed watcher, its path budget, and its metrics.
//! - [`service`] — the service itself: read, write, list, stat, watch.
//! - [`commands`] — the `fs.*` IPC payloads and the streaming channel name.
//!
//! Like `helix-log`, `helix-stream`, and `helix-config`, this crate has no Tauri
//! dependency and no dependency on the service container. `helix-kernel` wraps
//! it as a managed service, registers its commands, and bridges its changes onto
//! the stream, so the tests here drive the real code path with no process
//! around it.

pub mod atomic;
pub mod change;
pub mod commands;
pub mod encoding;
pub mod eol;
pub mod exclude;
pub mod hash;
pub mod listing;
pub mod probe;
pub mod service;
pub mod watch;

#[cfg(any(test, feature = "testutil"))]
pub mod testutil;

pub use atomic::{CrashPoint, TEMP_SUFFIX, is_temp_path, write_atomic, write_atomic_str};
pub use change::{ChangeKind, DEFAULT_DEBOUNCE, DEFAULT_MAX_HOLD, Debouncer, FileChange};
pub use commands::{
    CHANNEL, FsChangeNotification, FsListRequest, FsListResponse, FsReadRequest, FsReadResponse,
    FsStatRequest, FsStatResponse, FsUnwatchRequest, FsUnwatchResponse, FsWatchRequest,
    FsWatchResponse, FsWriteRequest, FsWriteResponse,
};
pub use encoding::{Detection, EncodeOutcome, Encoding, SNIFF_BYTES, looks_binary};
pub use eol::{EolInfo, LineEnding};
pub use exclude::{DEFAULT_EXCLUDE_GLOBS, ExclusionConfig, Exclusions};
pub use hash::{ContentHash, hash_bytes, hash_file};
pub use listing::{FileEntry, Listing};
pub use probe::{NETWORK_LATENCY_THRESHOLD, POLL_INTERVAL, ProbeOutcome};
pub use service::{
    DEFAULT_MAX_READ_BYTES, FileContent, FileSystemService, FsConfig, FsMetrics, WriteOptions,
    WriteOutcome,
};
pub use watch::{
    ChangeListener, DEFAULT_PATH_BUDGET, FsWatcher, LOG_SOURCE, RootReport, WatchConfig, WatchMode,
    WatchStats,
};
