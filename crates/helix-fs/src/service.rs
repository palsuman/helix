//! The file system service (Task 1.7, REQ-FS-004, REQ-ED-006, REQ-NFR-002).
//!
//! Like `Logger` in `helix-log` and `ConfigService` in `helix-config`, this is
//! the subsystem itself and knows nothing about Tauri or the service container.
//! `helix-kernel` wraps it as a managed service, registers its commands, and
//! bridges its change notifications onto the streaming channel.
//!
//! ```text
//!  read  ──► detect encoding ──► detect EOL ──► normalise to LF ──► hash
//!  write ──► apply EOL ──► encode ──► atomic write (temp, fsync, rename)
//!  list  ──► one walk, the watcher's exclusions
//!  watch ──► notify ──► 50ms debounce ──► listeners ──► stream channel
//! ```
//!
//! ## Decisions worth naming
//!
//! **Text reaches the editor as LF, always.** The file's real style is reported
//! alongside it and reapplied on save. Any other arrangement means every buffer
//! operation — cursor movement, column arithmetic, diffing, search offsets — has
//! to know whether a line break is one byte or two, and that knowledge would
//! have to be correct in dozens of places instead of two.
//!
//! **A conflicting write is refused, not merged.** `write` accepts the hash the
//! caller believed was on disk; if disk disagrees, the write fails with
//! `FS_WRITE_CONFLICT` and the caller decides. The kernel cannot resolve a
//! conflict correctly — only the user can — and overwriting silently is how an
//! external `git checkout` gets destroyed by a stale buffer (REQ-FS-004.3).
//!
//! **Reads are size-capped.** Loading an arbitrary file fully into memory is
//! fine right up until someone opens a 4GB log and the kernel, which owns every
//! window's state, is killed by the OOM killer. Over the cap, the read fails
//! with the size in the error so the frontend can offer a large-file mode
//! rather than showing a crash.
//!
//! **Binary files are described, not decoded.** The metadata comes back with
//! `binary: true` and no text, so the caller can show an appropriate viewer
//! instead of 4MB of replacement characters.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use helix_core::error::AppError;
use helix_log::{Logger, log_debug, log_error, log_warn};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::atomic;
use crate::change::FileChange;
use crate::encoding::{self, Encoding, SNIFF_BYTES};
use crate::eol::{self, EolInfo, LineEnding};
use crate::exclude::Exclusions;
use crate::hash;
use crate::listing::{self, FileEntry, Listing};
use crate::watch::{ChangeListener, FsWatcher, LOG_SOURCE, RootReport, WatchConfig, WatchStats};

/// Largest file read fully into memory. Above this the caller is told the size
/// and asked to choose, rather than the kernel being killed for it.
pub const DEFAULT_MAX_READ_BYTES: u64 = 50 * 1024 * 1024;

/// Service configuration, assembled by the kernel from `files.*` settings.
#[derive(Debug, Clone)]
pub struct FsConfig {
    pub watch: WatchConfig,
    /// Encoding for new files, from `files.encoding`. Existing files keep their
    /// detected encoding.
    pub default_encoding: Encoding,
    /// Line ending for new files, from `files.eol`. `None` means `auto`:
    /// existing files keep their style and new files get the platform default.
    pub default_eol: Option<LineEnding>,
    pub max_read_bytes: u64,
}

impl Default for FsConfig {
    fn default() -> Self {
        Self {
            watch: WatchConfig::default(),
            default_encoding: Encoding::Utf8,
            default_eol: None,
            max_read_bytes: DEFAULT_MAX_READ_BYTES,
        }
    }
}

/// A file's contents and everything the editor needs to save it back
/// faithfully.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct FileContent {
    pub path: String,
    /// LF-normalised text. `None` for a binary file.
    pub text: Option<String>,
    pub encoding: Encoding,
    /// True when a byte-order mark declared the encoding rather than a
    /// heuristic guessing it, so the UI knows not to offer to change it.
    pub encoding_from_bom: bool,
    pub eol: EolInfo,
    pub binary: bool,
    /// xxHash of the bytes on disk. The value to pass back as
    /// `expected_hash` on save, and the value the index compares against.
    pub hash: String,
    /// Size on disk in bytes, before decoding.
    pub size: u64,
    pub readonly: bool,
    pub modified_ms: Option<u64>,
}

/// What to write, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteOptions {
    /// LF-normalised text, as the editor holds it.
    pub text: String,
    /// `None` uses `files.encoding`.
    pub encoding: Option<Encoding>,
    /// `None` uses the file's existing style, falling back to `files.eol` and
    /// then to the platform default.
    pub eol: Option<LineEnding>,
    /// The hash the caller believed was on disk. `None` skips the check, which
    /// is correct for a brand new file and for a deliberate overwrite the user
    /// has already confirmed.
    pub expected_hash: Option<String>,
}

impl WriteOptions {
    /// Write text with every default: `files.encoding`, the existing line
    /// ending style, and no conflict check.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            encoding: None,
            eol: None,
            expected_hash: None,
        }
    }

    pub fn with_encoding(mut self, encoding: Encoding) -> Self {
        self.encoding = Some(encoding);
        self
    }

    pub fn with_eol(mut self, eol: LineEnding) -> Self {
        self.eol = Some(eol);
        self
    }

    /// Refuse the write if the bytes on disk no longer hash to this.
    pub fn expecting(mut self, hash: impl Into<String>) -> Self {
        self.expected_hash = Some(hash.into());
        self
    }
}

/// Result of a successful write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct WriteOutcome {
    pub path: String,
    pub bytes_written: u64,
    /// Hash of what is now on disk. Pass it as the next `expected_hash`.
    pub hash: String,
    pub encoding: Encoding,
    pub eol: LineEnding,
    /// True when the chosen encoding could not represent every character and
    /// substitutions were made. The caller should say so.
    pub lossy: bool,
}

#[derive(Debug, Default)]
struct Counters {
    reads: AtomicU64,
    read_errors: AtomicU64,
    writes: AtomicU64,
    write_errors: AtomicU64,
    conflicts: AtomicU64,
    bytes_read: AtomicU64,
    bytes_written: AtomicU64,
    listings: AtomicU64,
}

/// Point-in-time counters, surfaced through the kernel's health model.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct FsMetrics {
    pub reads: u64,
    pub read_errors: u64,
    pub writes: u64,
    pub write_errors: u64,
    /// Writes refused because disk had changed under the caller.
    pub conflicts: u64,
    pub bytes_read: u64,
    pub bytes_written: u64,
    pub listings: u64,
    /// Watcher metrics, published to health monitoring together with these
    /// (REQ-FS-004.8).
    pub watch: WatchStats,
}

/// The file system service.
pub struct FileSystemService {
    config: RwLock<FsConfig>,
    logger: Arc<Logger>,
    watcher: FsWatcher,
    /// Shared with the watcher's own listener, which is how the watcher fans
    /// changes out to this service's subscribers without either owning the
    /// other.
    listeners: Arc<RwLock<Vec<ChangeListener>>>,
    counters: Counters,
}

impl FileSystemService {
    /// Build the service and start its watcher thread. No roots are watched
    /// until [`watch`](Self::watch) is called.
    pub fn new(config: FsConfig, logger: Arc<Logger>) -> Self {
        let listeners: Arc<RwLock<Vec<ChangeListener>>> = Arc::new(RwLock::new(Vec::new()));
        let fanout_listeners = listeners.clone();
        let fanout: ChangeListener = Arc::new(move |changes: &[FileChange]| {
            for listener in fanout_listeners.read().unwrap().iter() {
                listener(changes);
            }
        });
        let watcher = FsWatcher::new(config.watch.clone(), logger.clone(), fanout);
        Self {
            config: RwLock::new(config),
            logger,
            watcher,
            listeners,
            counters: Counters::default(),
        }
    }

    /// A service with the requirement's defaults.
    pub fn with_defaults(logger: Arc<Logger>) -> Self {
        Self::new(FsConfig::default(), logger)
    }

    pub fn config(&self) -> FsConfig {
        self.config.read().unwrap().clone()
    }

    /// Replace operation defaults and rebuild active roots under the new watch
    /// rules. The old configuration remains active if any root cannot be
    /// re-registered.
    pub fn reconfigure(&self, config: FsConfig) -> Result<Vec<RootReport>, AppError> {
        let reports = self.watcher.reconfigure(config.watch.clone())?;
        *self.config.write().unwrap() = config;
        Ok(reports)
    }

    /// Register a listener called with every debounced batch of changes. The
    /// kernel uses this to publish onto the streaming channel.
    pub fn add_listener(&self, listener: ChangeListener) {
        self.listeners.write().unwrap().push(listener);
    }

    // ---- reading --------------------------------------------------------

    /// Read a file, detecting encoding and line endings.
    ///
    /// Returns LF-normalised text for a text file and no text for a binary one.
    pub fn read(&self, path: impl AsRef<Path>) -> Result<FileContent, AppError> {
        let path = path.as_ref();
        let entry = self.stat(path)?;
        let max_read_bytes = self.config.read().unwrap().max_read_bytes;
        if entry.is_dir {
            self.counters.read_errors.fetch_add(1, Ordering::Relaxed);
            return Err(AppError::permanent(
                "FS_NOT_A_FILE",
                format!("{} is a directory, not a file", path.display()),
            ));
        }
        if entry.size > max_read_bytes {
            self.counters.read_errors.fetch_add(1, Ordering::Relaxed);
            return Err(AppError::permanent(
                "FS_FILE_TOO_LARGE",
                format!(
                    "{} is {} bytes, above the {} byte read limit",
                    path.display(),
                    entry.size,
                    max_read_bytes
                ),
            )
            .with_details(serde_json::json!({
                "path": path.to_string_lossy(),
                "size": entry.size,
                "limit": max_read_bytes,
            })));
        }

        let bytes = fs::read(path).map_err(|error| {
            self.counters.read_errors.fetch_add(1, Ordering::Relaxed);
            self.log_io_error("file could not be read", path, &error);
            io_error(path, &error, "FS_READ_FAILED")
        })?;

        self.counters.reads.fetch_add(1, Ordering::Relaxed);
        self.counters
            .bytes_read
            .fetch_add(bytes.len() as u64, Ordering::Relaxed);

        let content_hash = hash::hash_bytes(&bytes).to_string();
        if encoding::looks_binary(&bytes) {
            return Ok(FileContent {
                path: path.to_string_lossy().into_owned(),
                text: None,
                // Reported rather than guessed: a binary file has no encoding,
                // and claiming UTF-8 would invite a caller to decode it.
                encoding: Encoding::Utf8,
                encoding_from_bom: false,
                eol: EolInfo {
                    style: LineEnding::None,
                    lf_count: 0,
                    crlf_count: 0,
                },
                binary: true,
                hash: content_hash,
                size: entry.size,
                readonly: entry.readonly,
                modified_ms: entry.modified_ms,
            });
        }

        let detection = encoding::detect(&bytes);
        let decoded = detection.encoding.decode(&bytes);
        let eol_info = eol::detect(&decoded);
        Ok(FileContent {
            path: path.to_string_lossy().into_owned(),
            text: Some(eol::to_lf(&decoded)),
            encoding: detection.encoding,
            encoding_from_bom: detection.from_bom,
            eol: eol_info,
            binary: false,
            hash: content_hash,
            size: entry.size,
            readonly: entry.readonly,
            modified_ms: entry.modified_ms,
        })
    }

    /// Whether a file is binary, without reading all of it.
    ///
    /// Only the first [`SNIFF_BYTES`] are read, so this is safe to call on a
    /// path of unknown size and is what the search index and the explorer use.
    pub fn is_binary(&self, path: impl AsRef<Path>) -> Result<bool, AppError> {
        let path = path.as_ref();
        let mut file = File::open(path).map_err(|error| {
            self.counters.read_errors.fetch_add(1, Ordering::Relaxed);
            io_error(path, &error, "FS_READ_FAILED")
        })?;
        let mut head = vec![0u8; SNIFF_BYTES];
        let read = read_up_to(&mut file, &mut head)
            .map_err(|error| io_error(path, &error, "FS_READ_FAILED"))?;
        head.truncate(read);
        Ok(encoding::looks_binary(&head))
    }

    /// Hash a file's contents without holding them in memory.
    pub fn hash(&self, path: impl AsRef<Path>) -> Result<String, AppError> {
        let path = path.as_ref();
        hash::hash_file(path)
            .map(|hash| hash.to_string())
            .map_err(|error| {
                self.counters.read_errors.fetch_add(1, Ordering::Relaxed);
                io_error(path, &error, "FS_READ_FAILED")
            })
    }

    // ---- writing --------------------------------------------------------

    /// Write a file atomically (REQ-NFR-002): temp file, fsync, rename.
    ///
    /// A crash at any point leaves the previous contents intact. See
    /// [`crate::atomic`] for the sequence and why each step is ordered as it is.
    pub fn write(
        &self,
        path: impl AsRef<Path>,
        options: WriteOptions,
    ) -> Result<WriteOutcome, AppError> {
        let path = path.as_ref();
        let existing = self.peek_existing(path);

        if let Some(expected) = &options.expected_hash {
            let actual = existing.as_ref().map(|(hash, _)| hash.as_str());
            if actual != Some(expected.as_str()) {
                self.counters.conflicts.fetch_add(1, Ordering::Relaxed);
                log_warn!(
                    self.logger,
                    LOG_SOURCE,
                    "refused a write because the file changed on disk since it was read",
                    "path" => path.display().to_string(),
                    "expected_hash" => expected.clone(),
                    "actual_hash" => actual.unwrap_or("<absent>").to_string(),
                );
                return Err(AppError::permanent(
                    "FS_WRITE_CONFLICT",
                    format!(
                        "{} changed on disk since it was read; reload or overwrite explicitly",
                        path.display()
                    ),
                )
                .with_details(serde_json::json!({
                    "path": path.to_string_lossy(),
                    "expected_hash": expected,
                    "actual_hash": actual,
                })));
            }
        }

        // Precedence: what the caller asked for, then what the file already
        // was, then configuration, then the platform. Each step is only
        // consulted because the one before it had nothing to say.
        let config = self.config.read().unwrap().clone();
        let encoding = options
            .encoding
            .or_else(|| existing.as_ref().map(|(_, existing)| existing.encoding))
            .unwrap_or(config.default_encoding);
        let line_ending = options
            .eol
            .or_else(|| {
                existing
                    .as_ref()
                    .map(|(_, existing)| existing.eol.dominant())
            })
            .or(config.default_eol)
            .unwrap_or_else(LineEnding::platform_default);

        let styled = eol::from_lf(&options.text, line_ending);
        let encoded = encoding.encode(&styled);
        if encoded.lossy {
            log_warn!(
                self.logger,
                LOG_SOURCE,
                "some characters could not be represented in the file's encoding and were substituted",
                "path" => path.display().to_string(),
                "encoding" => encoding.as_str(),
            );
        }

        atomic::write_atomic(path, &encoded.bytes).map_err(|error| {
            self.counters.write_errors.fetch_add(1, Ordering::Relaxed);
            self.log_io_error("file could not be written", path, &error);
            io_error(path, &error, "FS_WRITE_FAILED")
        })?;

        let bytes_written = encoded.bytes.len() as u64;
        self.counters.writes.fetch_add(1, Ordering::Relaxed);
        self.counters
            .bytes_written
            .fetch_add(bytes_written, Ordering::Relaxed);
        log_debug!(
            self.logger,
            LOG_SOURCE,
            "file written",
            "path" => path.display().to_string(),
            "bytes" => bytes_written,
            "encoding" => encoding.as_str(),
            "eol" => line_ending.as_str(),
        );

        Ok(WriteOutcome {
            path: path.to_string_lossy().into_owned(),
            bytes_written,
            hash: hash::hash_bytes(&encoded.bytes).to_string(),
            encoding,
            eol: line_ending,
            lossy: encoded.lossy,
        })
    }

    /// The hash and the encoding/EOL shape of what is currently on disk.
    ///
    /// Only the sniff window is decoded, because this runs on the save path and
    /// re-reading a large file in full just to learn its line ending style would
    /// double the cost of every save.
    fn peek_existing(&self, path: &Path) -> Option<(String, ExistingShape)> {
        let hash = hash::hash_file(path).ok()?;
        let mut file = File::open(path).ok()?;
        let mut head = vec![0u8; SNIFF_BYTES];
        let read = read_up_to(&mut file, &mut head).ok()?;
        head.truncate(read);
        let detection = encoding::detect(&head);
        let decoded = detection.encoding.decode(&head);
        Some((
            hash.to_string(),
            ExistingShape {
                encoding: detection.encoding,
                eol: eol::detect(&decoded),
            },
        ))
    }

    // ---- listing --------------------------------------------------------

    /// List a directory, honouring exclusions (REQ-FS-004.4, .5).
    ///
    /// A directory inside a watched root reuses that root's compiled
    /// exclusions, so the explorer and the watcher cannot disagree about what
    /// exists. Outside any watched root, exclusions are compiled on the spot.
    pub fn list(&self, path: impl AsRef<Path>, recursive: bool) -> Result<Listing, AppError> {
        let path = path.as_ref();
        if !path.is_dir() {
            return Err(AppError::permanent(
                "FS_NOT_A_DIRECTORY",
                format!("{} is not a directory", path.display()),
            ));
        }
        self.counters.listings.fetch_add(1, Ordering::Relaxed);

        match self.exclusions_covering(path) {
            Some(exclusions) => Ok(listing::list(path, &exclusions, recursive)),
            None => {
                let config = self.config.read().unwrap().clone();
                let exclusions = Exclusions::build(path, &config.watch.exclusions);
                Ok(listing::list(path, &exclusions, recursive))
            }
        }
    }

    /// Stat one path.
    pub fn stat(&self, path: impl AsRef<Path>) -> Result<FileEntry, AppError> {
        let path = path.as_ref();
        listing::stat(path).map_err(|error| io_error(path, &error, "FS_STAT_FAILED"))
    }

    /// The exclusions of the watched root containing `path`, if any.
    fn exclusions_covering(&self, path: &Path) -> Option<Arc<Exclusions>> {
        self.watcher
            .roots()
            .iter()
            .map(|report| PathBuf::from(&report.root))
            .filter(|root| path.starts_with(root))
            // Deepest matching root wins: a nested root's exclusions are the
            // more specific statement about that subtree.
            .max_by_key(|root| root.components().count())
            .and_then(|root| self.watcher.exclusions_for(root))
    }

    // ---- watching -------------------------------------------------------

    /// Start watching a root recursively (REQ-FS-004.1).
    pub fn watch(&self, root: impl AsRef<Path>) -> Result<RootReport, AppError> {
        self.watcher.watch(root)
    }

    /// Stop watching a root and release its OS registrations.
    pub fn unwatch(&self, root: impl AsRef<Path>) -> Result<(), AppError> {
        self.watcher.unwatch(root)
    }

    pub fn watched_roots(&self) -> Vec<RootReport> {
        self.watcher.roots()
    }

    pub fn watch_stats(&self) -> WatchStats {
        self.watcher.stats()
    }

    pub fn metrics(&self) -> FsMetrics {
        FsMetrics {
            reads: self.counters.reads.load(Ordering::Relaxed),
            read_errors: self.counters.read_errors.load(Ordering::Relaxed),
            writes: self.counters.writes.load(Ordering::Relaxed),
            write_errors: self.counters.write_errors.load(Ordering::Relaxed),
            conflicts: self.counters.conflicts.load(Ordering::Relaxed),
            bytes_read: self.counters.bytes_read.load(Ordering::Relaxed),
            bytes_written: self.counters.bytes_written.load(Ordering::Relaxed),
            listings: self.counters.listings.load(Ordering::Relaxed),
            watch: self.watcher.stats(),
        }
    }

    fn log_io_error(&self, message: &'static str, path: &Path, error: &std::io::Error) {
        log_error!(
            self.logger,
            LOG_SOURCE,
            message,
            "path" => path.display().to_string(),
            "kind" => format!("{:?}", error.kind()),
            "error" => error.to_string(),
        );
    }
}

/// What a file on disk already looks like, for the save path's defaults.
struct ExistingShape {
    encoding: Encoding,
    eol: EolInfo,
}

/// Map an OS error to a typed one, preserving the specific reason.
///
/// The distinction matters to the caller: a missing file is permanent and a
/// busy one is worth retrying, and REQ-FS-003's failure modes require the
/// specific OS error to reach the user rather than a generic "could not open".
fn io_error(path: &Path, error: &std::io::Error, fallback_code: &str) -> AppError {
    use std::io::ErrorKind;
    let (code, category_permanent) = match error.kind() {
        ErrorKind::NotFound => ("FS_NOT_FOUND", true),
        ErrorKind::PermissionDenied => ("FS_PERMISSION_DENIED", true),
        ErrorKind::IsADirectory => ("FS_NOT_A_FILE", true),
        ErrorKind::NotADirectory => ("FS_NOT_A_DIRECTORY", true),
        ErrorKind::StorageFull => ("FS_DISK_FULL", false),
        // Retryable: a file locked by another process usually is not for long.
        ErrorKind::Interrupted | ErrorKind::WouldBlock => (fallback_code, false),
        _ => (fallback_code, false),
    };
    let message = format!("{}: {error}", path.display());
    let error = if category_permanent {
        AppError::permanent(code, message)
    } else {
        AppError::transient(code, message)
    };
    error.with_details(serde_json::json!({ "path": path.to_string_lossy() }))
}

/// Fill `buffer` as far as the reader allows, tolerating short reads.
///
/// `Read::read` is permitted to return fewer bytes than asked for even when
/// more are available, which is easy to forget and produces a detector that
/// works on local disks and misclassifies files on slower ones.
fn read_up_to<R: Read>(reader: &mut R, buffer: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use helix_log::LogLevel;

    fn service() -> FileSystemService {
        FileSystemService::with_defaults(Arc::new(Logger::in_memory(LogLevel::Trace)))
    }

    #[test]
    fn a_utf8_file_reads_back_with_its_encoding_and_eol() {
        let dir = TempDir::new("service-read");
        let path = dir.write("main.rs", "fn main() {}\nlet x = 1;\n");
        let content = service().read(&path).unwrap();

        assert_eq!(content.encoding, Encoding::Utf8);
        assert!(!content.encoding_from_bom);
        assert_eq!(content.eol.style, LineEnding::Lf);
        assert!(!content.binary);
        assert_eq!(content.text.as_deref(), Some("fn main() {}\nlet x = 1;\n"));
        assert_eq!(content.size, 24);
    }

    #[test]
    fn a_crlf_file_reads_as_lf_text_with_crlf_reported() {
        let dir = TempDir::new("service-crlf");
        let path = dir.write("win.txt", "one\r\ntwo\r\n");
        let content = service().read(&path).unwrap();

        assert_eq!(content.eol.style, LineEnding::Crlf);
        assert_eq!(
            content.text.as_deref(),
            Some("one\ntwo\n"),
            "the editor always sees LF"
        );
    }

    #[test]
    fn a_utf16_file_reads_as_text_not_as_binary() {
        let dir = TempDir::new("service-utf16");
        let bytes = Encoding::Utf16Le.encode("hello\nworld\n").bytes;
        let path = dir.write("wide.txt", &bytes);
        let content = service().read(&path).unwrap();

        assert_eq!(content.encoding, Encoding::Utf16Le);
        assert!(content.encoding_from_bom);
        assert!(!content.binary);
        assert_eq!(content.text.as_deref(), Some("hello\nworld\n"));
    }

    #[test]
    fn a_png_is_reported_as_binary_with_no_text() {
        // The Task 1.7 demo criterion.
        let dir = TempDir::new("service-png");
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&[0x00, 0x00, 0x00, 0x0D]);
        png.extend_from_slice(b"IHDR");
        let path = dir.write("icon.png", &png);

        let svc = service();
        let content = svc.read(&path).unwrap();
        assert!(content.binary);
        assert!(content.text.is_none());
        assert!(svc.is_binary(&path).unwrap());
    }

    #[test]
    fn a_source_file_is_not_binary() {
        let dir = TempDir::new("service-not-binary");
        let path = dir.write("main.rs", "fn main() {}\n");
        assert!(!service().is_binary(&path).unwrap());
    }

    #[test]
    fn a_write_round_trips_through_a_read() {
        let dir = TempDir::new("service-write");
        let path = dir.path().join("new.rs");
        let svc = service();

        let outcome = svc
            .write(
                &path,
                WriteOptions::new("fn main() {}\n").with_eol(LineEnding::Lf),
            )
            .unwrap();
        assert_eq!(outcome.bytes_written, 13);
        assert_eq!(outcome.encoding, Encoding::Utf8);

        let content = svc.read(&path).unwrap();
        assert_eq!(content.text.as_deref(), Some("fn main() {}\n"));
        assert_eq!(content.hash, outcome.hash);
    }

    #[test]
    fn a_brand_new_file_with_no_configured_eol_gets_the_platform_style() {
        // `files.eol` defaults to `auto`, which for a file with no existing
        // style can only mean the platform's convention.
        let dir = TempDir::new("service-new-eol");
        let path = dir.path().join("fresh.txt");
        let outcome = service().write(&path, WriteOptions::new("a\nb\n")).unwrap();
        assert_eq!(outcome.eol, LineEnding::platform_default());
    }

    #[test]
    fn a_save_preserves_the_files_existing_encoding_and_eol() {
        // Opening a CRLF UTF-16 file, editing one line, and saving must not
        // rewrite every line ending and drop the encoding.
        let dir = TempDir::new("service-preserve");
        let original = Encoding::Utf16Le.encode("one\r\ntwo\r\n").bytes;
        let path = dir.write("legacy.txt", &original);
        let svc = service();

        let outcome = svc
            .write(&path, WriteOptions::new("one\ntwo edited\n"))
            .unwrap();
        assert_eq!(outcome.encoding, Encoding::Utf16Le);
        assert_eq!(outcome.eol, LineEnding::Crlf);

        let reread = svc.read(&path).unwrap();
        assert_eq!(reread.encoding, Encoding::Utf16Le);
        assert_eq!(reread.eol.style, LineEnding::Crlf);
        assert_eq!(reread.text.as_deref(), Some("one\ntwo edited\n"));
    }

    #[test]
    fn an_explicit_eol_conversion_is_honoured() {
        // The Task 1.7 demo: switch a file from CRLF to LF and save.
        let dir = TempDir::new("service-convert");
        let path = dir.write("win.txt", "one\r\ntwo\r\n");
        let svc = service();

        svc.write(
            &path,
            WriteOptions::new("one\ntwo\n").with_eol(LineEnding::Lf),
        )
        .unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"one\ntwo\n");
        assert_eq!(svc.read(&path).unwrap().eol.style, LineEnding::Lf);
    }

    #[test]
    fn a_new_file_uses_the_configured_encoding_and_eol() {
        let dir = TempDir::new("service-defaults");
        let svc = FileSystemService::new(
            FsConfig {
                default_encoding: Encoding::Utf8Bom,
                default_eol: Some(LineEnding::Crlf),
                ..FsConfig::default()
            },
            Arc::new(Logger::in_memory(LogLevel::Trace)),
        );
        let path = dir.path().join("configured.txt");
        let outcome = svc.write(&path, WriteOptions::new("a\nb\n")).unwrap();

        assert_eq!(outcome.encoding, Encoding::Utf8Bom);
        assert_eq!(outcome.eol, LineEnding::Crlf);
        assert_eq!(fs::read(&path).unwrap(), b"\xEF\xBB\xBFa\r\nb\r\n");
    }

    #[test]
    fn reconfiguring_changes_the_defaults_for_the_next_new_file() {
        let dir = TempDir::new("service-reconfigure");
        let svc = service();
        let mut config = svc.config();
        config.default_eol = Some(LineEnding::Crlf);
        svc.reconfigure(config).unwrap();

        let path = dir.path().join("configured-after-start.txt");
        let outcome = svc.write(&path, WriteOptions::new("a\nb\n")).unwrap();

        assert_eq!(outcome.eol, LineEnding::Crlf);
        assert_eq!(fs::read(&path).unwrap(), b"a\r\nb\r\n");
    }

    #[test]
    fn a_write_whose_expected_hash_matches_succeeds() {
        let dir = TempDir::new("service-hash-ok");
        let path = dir.write("main.rs", "before\n");
        let svc = service();
        let content = svc.read(&path).unwrap();

        let outcome = svc
            .write(&path, WriteOptions::new("after\n").expecting(&content.hash))
            .unwrap();
        assert_ne!(outcome.hash, content.hash);
    }

    #[test]
    fn a_write_is_refused_when_the_file_changed_on_disk() {
        // REQ-FS-004.3: the kernel detects the conflict; the user decides.
        let dir = TempDir::new("service-conflict");
        let path = dir.write("main.rs", "before\n");
        let svc = service();
        let content = svc.read(&path).unwrap();

        fs::write(&path, "changed by git\n").unwrap();

        let error = svc
            .write(&path, WriteOptions::new("after\n").expecting(&content.hash))
            .expect_err("a stale write must be refused");
        assert_eq!(error.code, "FS_WRITE_CONFLICT");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "changed by git\n",
            "the external change must survive the refused write"
        );
        assert_eq!(svc.metrics().conflicts, 1);
    }

    #[test]
    fn a_write_without_an_expected_hash_overwrites_deliberately() {
        let dir = TempDir::new("service-force");
        let path = dir.write("main.rs", "before\n");
        let svc = service();
        svc.write(&path, WriteOptions::new("after\n")).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "after\n");
    }

    #[test]
    fn a_lossy_encoding_choice_is_reported_rather_than_hidden() {
        let dir = TempDir::new("service-lossy");
        let path = dir.path().join("latin.txt");
        let outcome = service()
            .write(
                &path,
                WriteOptions::new("world 世界\n").with_encoding(Encoding::Latin1),
            )
            .unwrap();
        assert!(outcome.lossy);
    }

    #[test]
    fn reading_a_missing_file_is_a_typed_permanent_error() {
        let dir = TempDir::new("service-missing");
        let error = service()
            .read(dir.path().join("nope.rs"))
            .expect_err("a missing file must error");
        assert_eq!(error.code, "FS_NOT_FOUND");
        assert_eq!(error.category, helix_core::error::ErrorCategory::Permanent);
    }

    #[test]
    fn reading_a_directory_is_an_error_rather_than_a_confusing_success() {
        let dir = TempDir::new("service-dir");
        let error = service()
            .read(dir.path())
            .expect_err("a directory is not a file");
        assert_eq!(error.code, "FS_NOT_A_FILE");
    }

    #[test]
    fn a_file_above_the_read_limit_reports_its_size_instead_of_being_loaded() {
        let dir = TempDir::new("service-large");
        let path = dir.write("big.log", vec![b'x'; 4096]);
        let svc = FileSystemService::new(
            FsConfig {
                max_read_bytes: 1024,
                ..FsConfig::default()
            },
            Arc::new(Logger::in_memory(LogLevel::Trace)),
        );

        let error = svc
            .read(&path)
            .expect_err("an oversized read must be refused");
        assert_eq!(error.code, "FS_FILE_TOO_LARGE");
        assert_eq!(error.details.unwrap()["size"], 4096);
    }

    #[test]
    fn listing_honours_exclusions_and_reports_stat_information() {
        let dir = TempDir::new("service-list");
        dir.write("src/main.rs", "fn main() {}");
        dir.write("node_modules/pkg/index.js", "noise");
        dir.write(".gitignore", "*.log\n");
        dir.write("app.log", "noise");

        let listing = service().list(dir.path(), true).unwrap();
        let paths: Vec<&str> = listing
            .entries
            .iter()
            .map(|e| e.relative_path.as_str())
            .collect();
        assert!(paths.contains(&"src/main.rs"));
        assert!(!paths.iter().any(|p| p.contains("node_modules")));
        assert!(!paths.contains(&"app.log"), "{paths:?}");
    }

    #[test]
    fn listing_a_file_is_a_typed_error() {
        let dir = TempDir::new("service-list-file");
        let path = dir.write("main.rs", "fn main() {}");
        let error = service()
            .list(&path, false)
            .expect_err("a file is not a directory");
        assert_eq!(error.code, "FS_NOT_A_DIRECTORY");
    }

    #[test]
    fn hashing_a_file_matches_hashing_its_bytes() {
        let dir = TempDir::new("service-hash");
        let path = dir.write("main.rs", "fn main() {}\n");
        assert_eq!(
            service().hash(&path).unwrap(),
            crate::hash::hash_bytes(b"fn main() {}\n").to_string()
        );
    }

    #[test]
    fn metrics_count_reads_writes_and_watcher_state() {
        let dir = TempDir::new("service-metrics");
        let path = dir.write("main.rs", "fn main() {}\n");
        let svc = service();
        svc.read(&path).unwrap();
        svc.write(&path, WriteOptions::new("edited\n")).unwrap();
        svc.watch(dir.path()).unwrap();

        let metrics = svc.metrics();
        assert_eq!(metrics.reads, 1);
        assert_eq!(metrics.writes, 1);
        assert!(metrics.bytes_read > 0);
        assert!(metrics.bytes_written > 0);
        assert_eq!(metrics.watch.roots, 1);
        assert!(metrics.watch.watched_paths >= 1);
    }

    #[test]
    fn a_listener_receives_debounced_changes_from_a_watched_root() {
        use std::sync::mpsc::sync_channel;
        let dir = TempDir::new("service-listener");
        let svc = service();
        let (tx, rx) = sync_channel::<Vec<FileChange>>(64);
        svc.add_listener(Arc::new(move |changes: &[FileChange]| {
            let _ = tx.try_send(changes.to_vec());
        }));
        svc.watch(dir.path()).unwrap();

        dir.write("watched.rs", "fn main() {}");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let remaining = deadline
                .checked_duration_since(std::time::Instant::now())
                .expect("a change must reach the listener");
            if let Ok(batch) = rx.recv_timeout(remaining)
                && batch.iter().any(|c| c.path.ends_with("watched.rs"))
            {
                break;
            }
        }
    }
}
