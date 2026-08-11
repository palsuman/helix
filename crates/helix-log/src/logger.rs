//! The log aggregator: one entry point, several sinks (REQ-OBS-001.3, .7,
//! .8, .11).
//!
//! Every record from every source — kernel services, the frontend (shipped
//! over IPC), LSP servers, the agent, plugins — goes through
//! [`Logger::log`]. That is what makes a single unified stream possible
//! (REQ-OBS-001.3) and what makes redaction unconditional
//! (REQ-OBS-001.11): there is no second path to a sink.
//!
//! ## Order of operations
//!
//! 1. **Level check.** One relaxed atomic load rejects a disabled level
//!    before anything is allocated (REQ-OBS-001.8). The logging macros put
//!    the check *around* the record construction, so a disabled
//!    `log_trace!` does not even evaluate its field expressions.
//! 2. **Correlation.** A record with no explicit correlation ID inherits the
//!    one in scope, which is how a kernel service's log line ends up
//!    attributable to the IPC command that caused it (REQ-OBS-001.9).
//! 3. **Redaction.** Applied once, centrally, before any sink sees the
//!    record.
//! 4. **Fan-out.** Ring buffer (always), file (always, when a directory is
//!    configured), stdout (CLI launches), and any registered sink — the
//!    streaming bridge that feeds the viewer live, and later the crash
//!    reporter.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use crate::filter::{LevelConfig, LogQuery};
use crate::record::{LogLevel, LogRecord};
use crate::redact::Redactor;
use crate::ring::{DEFAULT_RING_CAPACITY, RecordRing};
use crate::rotate::{
    DEFAULT_FILE_NAME, DEFAULT_MAX_FILE_BYTES, DEFAULT_MAX_FILES, RotatingFileSink,
};

/// An additional destination for records, beyond the built-in ring, file, and
/// stdout sinks.
///
/// Receives records that are already redacted, so an implementor cannot
/// accidentally observe a secret.
pub trait LogSink: Send + Sync {
    fn write(&self, record: &LogRecord);
}

impl<F> LogSink for F
where
    F: Fn(&LogRecord) + Send + Sync,
{
    fn write(&self, record: &LogRecord) {
        self(record)
    }
}

/// Configuration for a [`Logger`].
#[derive(Debug, Clone)]
pub struct LoggerConfig {
    pub levels: LevelConfig,
    /// Viewer ring depth (REQ-OBS-001.4).
    pub ring_capacity: usize,
    /// Directory for the log file. `None` disables the file sink, which is
    /// what unit tests and short-lived tooling want.
    pub directory: Option<PathBuf>,
    pub file_name: String,
    pub max_file_bytes: u64,
    pub max_files: usize,
    /// Mirror records to stdout, for a CLI launch (REQ-OBS-001.7).
    pub stdout: bool,
}

impl Default for LoggerConfig {
    fn default() -> Self {
        Self {
            levels: LevelConfig::default(),
            ring_capacity: DEFAULT_RING_CAPACITY,
            directory: None,
            file_name: DEFAULT_FILE_NAME.to_string(),
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            max_files: DEFAULT_MAX_FILES,
            stdout: false,
        }
    }
}

impl LoggerConfig {
    pub fn with_default_level(mut self, level: LogLevel) -> Self {
        self.levels.default_level = level;
        self
    }

    pub fn with_module_level(mut self, module: impl Into<String>, level: LogLevel) -> Self {
        self.levels.set_module(module, level);
        self
    }

    pub fn with_ring_capacity(mut self, capacity: usize) -> Self {
        self.ring_capacity = capacity;
        self
    }

    pub fn with_directory(mut self, directory: impl Into<PathBuf>) -> Self {
        self.directory = Some(directory.into());
        self
    }

    pub fn with_rotation(mut self, max_file_bytes: u64, max_files: usize) -> Self {
        self.max_file_bytes = max_file_bytes;
        self.max_files = max_files;
        self
    }

    pub fn with_stdout(mut self, stdout: bool) -> Self {
        self.stdout = stdout;
        self
    }
}

/// Counters describing the logger's own behaviour, surfaced through the log
/// service's health report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoggerMetrics {
    /// Records that passed the level check and reached the sinks.
    pub emitted: u64,
    /// Records rejected by the level check. A large number here is normal
    /// and cheap; it is the evidence that the fast path is doing its job.
    pub suppressed: u64,
    /// File writes that failed (disk full, permissions).
    pub write_errors: u64,
    /// Records evicted from the viewer ring.
    pub evicted: u64,
    pub rotations: u64,
    pub ring_len: usize,
    pub ring_capacity: usize,
}

/// The result of a viewer query.
#[derive(Debug, Clone, PartialEq)]
pub struct QueryResult {
    pub entries: Vec<LogRecord>,
    /// Records matching the filter before `limit` was applied, so the viewer
    /// can say "showing 500 of 4,213".
    pub matched: usize,
    pub ring_len: usize,
    pub ring_capacity: usize,
    pub evicted: u64,
}

/// The log aggregator.
pub struct Logger {
    levels: RwLock<LevelConfig>,
    /// Cached most-verbose enabled level, so the hot path costs one atomic
    /// load rather than a lock acquisition (REQ-OBS-001.8).
    min_level: AtomicU8,
    ring: Mutex<RecordRing>,
    file: Mutex<Option<RotatingFileSink>>,
    stdout: AtomicBool,
    redactor: Redactor,
    sinks: RwLock<Vec<Arc<dyn LogSink>>>,
    emitted: AtomicU64,
    suppressed: AtomicU64,
    write_errors: AtomicU64,
}

impl Default for Logger {
    fn default() -> Self {
        Self::new(LoggerConfig::default())
    }
}

impl Logger {
    /// Build a logger. A file sink that cannot be opened is reported through
    /// [`Logger::write_error_count`] rather than failing construction: losing
    /// the log file must not prevent the application from starting, and the
    /// ring and panel sinks still work.
    pub fn new(config: LoggerConfig) -> Self {
        let min_level = config.levels.min_enabled_level().rank();
        let write_errors = AtomicU64::new(0);
        let file = match &config.directory {
            Some(directory) => match RotatingFileSink::open(
                directory,
                config.file_name.clone(),
                config.max_file_bytes,
                config.max_files,
            ) {
                Ok(sink) => Some(sink),
                Err(_) => {
                    write_errors.store(1, Ordering::Relaxed);
                    None
                }
            },
            None => None,
        };

        Self {
            levels: RwLock::new(config.levels),
            min_level: AtomicU8::new(min_level),
            ring: Mutex::new(RecordRing::new(config.ring_capacity)),
            file: Mutex::new(file),
            stdout: AtomicBool::new(config.stdout),
            redactor: Redactor::new(),
            sinks: RwLock::new(Vec::new()),
            emitted: AtomicU64::new(0),
            suppressed: AtomicU64::new(0),
            write_errors,
        }
    }

    /// A logger with no file sink, for tests and tooling.
    pub fn in_memory(default_level: LogLevel) -> Self {
        Self::new(LoggerConfig::default().with_default_level(default_level))
    }

    /// Whether a record at this level from this source would be recorded.
    ///
    /// The first check is a single relaxed atomic load against the most
    /// verbose level any module is configured at. Because no module can be
    /// more verbose than that, a level below it cannot be enabled anywhere,
    /// so the answer is `false` without touching the lock or the map. This is
    /// the property REQ-OBS-001.8 asks for, and the macros build on it by
    /// keeping argument evaluation inside the `if`.
    pub fn enabled(&self, level: LogLevel, source: &str) -> bool {
        if level.rank() < self.min_level.load(Ordering::Relaxed) {
            return false;
        }
        self.levels.read().unwrap().enabled(level, source)
    }

    /// Record an entry. Applies the level check again, so a caller that
    /// bypassed [`Logger::enabled`] cannot smuggle a suppressed level in.
    pub fn log(&self, mut record: LogRecord) {
        if !self.enabled(record.level, &record.source) {
            self.suppressed.fetch_add(1, Ordering::Relaxed);
            return;
        }

        if record.correlation_id.is_none() {
            record.correlation_id = crate::correlation::current();
        }
        // Every sink downstream of this point sees a redacted record; there
        // is no path around it (REQ-OBS-001.11, REQ-SEC-002.5).
        self.redactor.redact_record(&mut record);

        let line = record.to_json_line();

        if let Ok(mut file) = self.file.lock()
            && let Some(sink) = file.as_mut()
            && sink.write_line(&line).is_err()
        {
            self.write_errors.fetch_add(1, Ordering::Relaxed);
        }

        if self.stdout.load(Ordering::Relaxed) {
            println!("{line}");
        }

        // Registered sinks run before the ring lock is taken, so a slow sink
        // does not serialize with viewer queries.
        for sink in self.sinks.read().unwrap().iter() {
            sink.write(&record);
        }

        self.ring.lock().unwrap().push(record);
        self.emitted.fetch_add(1, Ordering::Relaxed);
    }

    /// Convenience emitter for call sites that have no structured fields.
    /// Prefer the macros, which skip the argument work when the level is
    /// disabled.
    pub fn event(&self, level: LogLevel, source: &str, message: impl Into<String>) {
        if !self.enabled(level, source) {
            self.suppressed.fetch_add(1, Ordering::Relaxed);
            return;
        }
        self.log(LogRecord::new(level, source, message));
    }

    // ---- configuration --------------------------------------------------

    pub fn levels(&self) -> LevelConfig {
        self.levels.read().unwrap().clone()
    }

    /// The most verbose level enabled anywhere, which is the threshold the
    /// hot path compares against.
    pub fn min_enabled_level(&self) -> LogLevel {
        LogLevel::from_rank(self.min_level.load(Ordering::Relaxed))
    }

    pub fn set_levels(&self, levels: LevelConfig) {
        self.min_level
            .store(levels.min_enabled_level().rank(), Ordering::Relaxed);
        *self.levels.write().unwrap() = levels;
    }

    pub fn set_default_level(&self, level: LogLevel) {
        let mut levels = self.levels.write().unwrap();
        levels.default_level = level;
        self.min_level
            .store(levels.min_enabled_level().rank(), Ordering::Relaxed);
    }

    /// Set (or with `None`, clear) the level for a module prefix.
    pub fn set_module_level(&self, module: &str, level: Option<LogLevel>) {
        let mut levels = self.levels.write().unwrap();
        match level {
            Some(level) => levels.set_module(module, level),
            None => {
                levels.clear_module(module);
            }
        }
        self.min_level
            .store(levels.min_enabled_level().rank(), Ordering::Relaxed);
    }

    pub fn set_stdout(&self, enabled: bool) {
        self.stdout.store(enabled, Ordering::Relaxed);
    }

    /// Register an exact secret value for redaction. Called by the secret
    /// service (Task 1.12) whenever a credential is loaded, so a value that
    /// no heuristic would recognize is still never written.
    pub fn register_secret(&self, value: impl Into<String>) {
        self.redactor.register_secret(value);
    }

    pub fn forget_secret(&self, value: &str) {
        self.redactor.forget_secret(value);
    }

    pub fn redactor(&self) -> &Redactor {
        &self.redactor
    }

    /// Add a sink. Returns the sink's index, which is not currently used to
    /// remove one: every sink registered so far lives as long as the logger.
    pub fn add_sink(&self, sink: Arc<dyn LogSink>) -> usize {
        let mut sinks = self.sinks.write().unwrap();
        sinks.push(sink);
        sinks.len() - 1
    }

    pub fn sink_count(&self) -> usize {
        self.sinks.read().unwrap().len()
    }

    /// Path of the active log file, if a file sink is configured.
    pub fn file_path(&self) -> Option<PathBuf> {
        self.file.lock().unwrap().as_ref().map(|sink| sink.path())
    }

    /// Every existing log file, newest first.
    pub fn files(&self) -> Vec<PathBuf> {
        self.file
            .lock()
            .unwrap()
            .as_ref()
            .map(|sink| sink.files())
            .unwrap_or_default()
    }

    // ---- queries ---------------------------------------------------------

    /// Records matching a query, oldest first, limited to the newest `limit`
    /// matches.
    ///
    /// Ordering is chronological in the returned vector even though the
    /// limit keeps the newest: a viewer that shows the newest 500 entries
    /// still wants to read them top to bottom.
    pub fn query(&self, query: &LogQuery) -> QueryResult {
        let ring = self.ring.lock().unwrap();
        let limit = query.limit.map(|l| l as usize);

        let mut matched = 0usize;
        let mut entries: Vec<LogRecord> = Vec::new();
        for record in ring.iter_rev() {
            if !query.matches(record) {
                continue;
            }
            matched += 1;
            if limit.map(|l| entries.len() < l).unwrap_or(true) {
                entries.push(record.clone());
            }
        }
        entries.reverse();

        QueryResult {
            entries,
            matched,
            ring_len: ring.len(),
            ring_capacity: ring.capacity(),
            evicted: ring.evicted(),
        }
    }

    /// The filtered set as JSON lines, ready to attach to a bug report
    /// (REQ-OBS-001.5). Identical in format to the log file, because it is
    /// produced by the same serializer.
    pub fn export(&self, query: &LogQuery) -> (String, usize) {
        let result = self.query(query);
        let mut out = String::new();
        for record in &result.entries {
            out.push_str(&record.to_json_line());
            out.push('\n');
        }
        (out, result.entries.len())
    }

    /// Distinct sources present in the ring, for the viewer's source filter.
    pub fn sources(&self) -> Vec<String> {
        self.ring.lock().unwrap().sources()
    }

    pub fn clear(&self) {
        self.ring.lock().unwrap().clear();
    }

    pub fn metrics(&self) -> LoggerMetrics {
        let ring = self.ring.lock().unwrap();
        let rotations = self
            .file
            .lock()
            .unwrap()
            .as_ref()
            .map(|sink| sink.rotations())
            .unwrap_or(0);
        LoggerMetrics {
            emitted: self.emitted.load(Ordering::Relaxed),
            suppressed: self.suppressed.load(Ordering::Relaxed),
            write_errors: self.write_errors.load(Ordering::Relaxed),
            evicted: ring.evicted(),
            rotations,
            ring_len: ring.len(),
            ring_capacity: ring.capacity(),
        }
    }

    pub fn write_error_count(&self) -> u64 {
        self.write_errors.load(Ordering::Relaxed)
    }

    pub fn flush(&self) {
        if let Ok(mut file) = self.file.lock()
            && let Some(sink) = file.as_mut()
        {
            let _ = sink.flush();
        }
    }
}

impl std::fmt::Debug for Logger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Logger")
            .field("levels", &self.levels())
            .field("sinks", &self.sink_count())
            .field("metrics", &self.metrics())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::to_field;
    use crate::{log_debug, log_error, log_info, log_trace, log_warn};
    use std::sync::atomic::AtomicU32;

    fn logger() -> Logger {
        Logger::in_memory(LogLevel::Trace)
    }

    #[test]
    fn a_record_reaches_the_ring_and_is_queryable() {
        let logger = logger();
        logger.event(LogLevel::Info, "kernel.fs", "file saved");

        let result = logger.query(&LogQuery::new());
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].message, "file saved");
        assert_eq!(logger.metrics().emitted, 1);
    }

    #[test]
    fn a_record_below_the_configured_level_is_suppressed() {
        let logger = Logger::in_memory(LogLevel::Warn);
        logger.event(LogLevel::Debug, "kernel.fs", "chatty");
        logger.event(LogLevel::Error, "kernel.fs", "broken");

        let result = logger.query(&LogQuery::new());
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].message, "broken");
        assert_eq!(logger.metrics().suppressed, 1);
    }

    #[test]
    fn per_module_levels_are_honoured_at_emit_time() {
        let logger = Logger::new(
            LoggerConfig::default()
                .with_default_level(LogLevel::Warn)
                .with_module_level("kernel.fs", LogLevel::Trace),
        );
        logger.event(LogLevel::Trace, "kernel.fs.watcher", "descendant included");
        logger.event(LogLevel::Trace, "kernel.ipc", "excluded");

        let messages: Vec<String> = logger
            .query(&LogQuery::new())
            .entries
            .into_iter()
            .map(|r| r.message)
            .collect();
        assert_eq!(messages, vec!["descendant included"]);
    }

    #[test]
    fn changing_a_level_at_runtime_takes_effect_immediately() {
        let logger = Logger::in_memory(LogLevel::Info);
        assert!(!logger.enabled(LogLevel::Debug, "kernel.fs"));

        logger.set_module_level("kernel.fs", Some(LogLevel::Debug));
        assert!(logger.enabled(LogLevel::Debug, "kernel.fs"));

        logger.set_module_level("kernel.fs", None);
        assert!(!logger.enabled(LogLevel::Debug, "kernel.fs"));
    }

    #[test]
    fn a_disabled_level_does_not_evaluate_its_arguments() {
        // REQ-OBS-001.8: zero cost on a performance-critical path when the
        // level is disabled. "Zero cost" is only meaningful if the *call
        // site's* work is skipped too, not merely the record's storage.
        let logger = Logger::in_memory(LogLevel::Warn);
        let evaluations = AtomicU32::new(0);
        let expensive = || {
            evaluations.fetch_add(1, Ordering::SeqCst);
            "expensive"
        };

        log_trace!(logger, "hot.path", "tracing", "detail" => expensive());
        log_debug!(logger, "hot.path", "debugging", "detail" => expensive());
        assert_eq!(
            evaluations.load(Ordering::SeqCst),
            0,
            "a suppressed record must not evaluate its field expressions"
        );

        log_error!(logger, "hot.path", "failing", "detail" => expensive());
        assert_eq!(evaluations.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn the_macros_populate_level_source_message_and_fields() {
        let logger = logger();
        log_info!(logger, "lsp_host", "Server started", "language" => "typescript", "startup_ms" => 1200);

        let record = &logger.query(&LogQuery::new()).entries[0];
        assert_eq!(record.level, LogLevel::Info);
        assert_eq!(record.source, "lsp_host");
        assert_eq!(record.message, "Server started");
        assert_eq!(record.fields["language"], "typescript");
        assert_eq!(record.fields["startup_ms"], 1200);
    }

    #[test]
    fn every_macro_level_maps_to_its_level() {
        let logger = logger();
        log_trace!(logger, "s", "t");
        log_debug!(logger, "s", "d");
        log_info!(logger, "s", "i");
        log_warn!(logger, "s", "w");
        log_error!(logger, "s", "e");

        let levels: Vec<LogLevel> = logger
            .query(&LogQuery::new())
            .entries
            .into_iter()
            .map(|r| r.level)
            .collect();
        assert_eq!(levels, LogLevel::ALL.to_vec());
    }

    #[test]
    fn secrets_are_redacted_before_any_sink_sees_the_record() {
        let logger = logger();
        let seen: Arc<Mutex<Vec<LogRecord>>> = Arc::new(Mutex::new(Vec::new()));
        let captured = seen.clone();
        logger.add_sink(Arc::new(move |record: &LogRecord| {
            captured.lock().unwrap().push(record.clone());
        }));
        logger.register_secret("super-secret-token-value");

        log_info!(
            logger,
            "ai.provider",
            "calling provider with super-secret-token-value",
            "api_key" => "super-secret-token-value",
        );

        let sink_records = seen.lock().unwrap();
        assert_eq!(sink_records.len(), 1);
        assert!(!sink_records[0].message.contains("super-secret-token-value"));
        assert_eq!(sink_records[0].fields["api_key"], crate::redact::REDACTED);

        let ring_record = &logger.query(&LogQuery::new()).entries[0];
        assert!(!ring_record.message.contains("super-secret-token-value"));
    }

    #[test]
    fn queries_filter_by_level_source_search_and_time_range() {
        let logger = logger();
        logger.log(
            LogRecord::at(
                "2026-01-01T10:00:00.000Z",
                LogLevel::Info,
                "kernel.fs",
                "read",
            )
            .with_field("path", to_field("/a")),
        );
        logger.log(LogRecord::at(
            "2026-01-01T11:00:00.000Z",
            LogLevel::Error,
            "kernel.ipc",
            "dispatch failed",
        ));

        assert_eq!(
            logger
                .query(&LogQuery::new().with_min_level(LogLevel::Warn))
                .entries
                .len(),
            1
        );
        assert_eq!(
            logger
                .query(&LogQuery::new().with_sources(["kernel.fs"]))
                .entries
                .len(),
            1
        );
        assert_eq!(
            logger
                .query(&LogQuery::new().with_search("/a"))
                .entries
                .len(),
            1
        );
        assert_eq!(
            logger
                .query(
                    &LogQuery::new()
                        .with_time_range(Some("2026-01-01T10:30:00.000Z"), None::<String>)
                )
                .entries
                .len(),
            1
        );
    }

    #[test]
    fn a_limit_keeps_the_newest_matches_but_returns_them_chronologically() {
        let logger = logger();
        for i in 0..10 {
            logger.log(LogRecord::at(
                format!("2026-01-01T10:00:{i:02}.000Z"),
                LogLevel::Info,
                "s",
                format!("m{i}"),
            ));
        }

        let result = logger.query(&LogQuery::new().with_limit(3));
        assert_eq!(result.matched, 10, "matched counts before the limit");
        let messages: Vec<String> = result.entries.into_iter().map(|r| r.message).collect();
        assert_eq!(messages, vec!["m7", "m8", "m9"]);
    }

    #[test]
    fn export_produces_the_filtered_set_as_json_lines() {
        let logger = logger();
        logger.event(LogLevel::Info, "a", "kept");
        logger.event(LogLevel::Info, "b", "dropped");

        let (content, count) = logger.export(&LogQuery::new().with_sources(["a"]));
        assert_eq!(count, 1);
        assert_eq!(content.lines().count(), 1);
        let parsed: LogRecord = serde_json::from_str(content.lines().next().unwrap()).unwrap();
        assert_eq!(parsed.message, "kept");
    }

    #[test]
    fn sources_lists_what_actually_logged() {
        let logger = logger();
        logger.event(LogLevel::Info, "kernel.fs", "x");
        logger.event(LogLevel::Info, "frontend.app", "y");
        logger.event(LogLevel::Info, "kernel.fs", "z");
        assert_eq!(logger.sources(), vec!["frontend.app", "kernel.fs"]);
    }

    #[test]
    fn the_ring_evicts_the_oldest_and_reports_it() {
        let logger = Logger::new(
            LoggerConfig::default()
                .with_default_level(LogLevel::Trace)
                .with_ring_capacity(3),
        );
        for i in 0..5 {
            logger.event(LogLevel::Info, "s", format!("m{i}"));
        }
        let result = logger.query(&LogQuery::new());
        assert_eq!(result.entries.len(), 3);
        assert_eq!(result.evicted, 2);
    }

    #[test]
    fn a_file_sink_receives_the_same_json_line_as_the_ring() {
        let dir = std::env::temp_dir().join(format!("helix-log-sink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let logger = Logger::new(
            LoggerConfig::default()
                .with_default_level(LogLevel::Trace)
                .with_directory(&dir),
        );
        log_info!(logger, "kernel.fs", "saved", "path" => "/tmp/x");
        logger.flush();

        let path = logger.file_path().expect("a directory was configured");
        let contents = std::fs::read_to_string(&path).unwrap();
        let from_file: LogRecord = serde_json::from_str(contents.trim()).unwrap();
        assert_eq!(from_file, logger.query(&LogQuery::new()).entries[0]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unwritable_directory_does_not_prevent_logging() {
        // A path whose parent is a file cannot be created as a directory.
        let file = std::env::temp_dir().join(format!("helix-log-not-a-dir-{}", std::process::id()));
        std::fs::write(&file, b"x").unwrap();
        let logger = Logger::new(
            LoggerConfig::default()
                .with_default_level(LogLevel::Trace)
                .with_directory(file.join("logs")),
        );

        logger.event(LogLevel::Error, "s", "still recorded");
        assert_eq!(logger.query(&LogQuery::new()).entries.len(), 1);
        assert!(logger.write_error_count() > 0);
        assert!(logger.file_path().is_none());

        let _ = std::fs::remove_file(&file);
    }

    #[tokio::test]
    async fn a_record_inherits_the_correlation_id_in_scope() {
        let logger = logger();
        crate::correlation::scope("cmd-abc123", async {
            logger.event(LogLevel::Info, "kernel.fs", "writing inside a command");
        })
        .await;
        logger.event(LogLevel::Info, "kernel.fs", "writing outside a command");

        let entries = logger.query(&LogQuery::new()).entries;
        assert_eq!(entries[0].correlation_id.as_deref(), Some("cmd-abc123"));
        assert_eq!(entries[1].correlation_id, None);
    }

    #[tokio::test]
    async fn an_explicit_correlation_id_is_not_overwritten_by_the_scope() {
        let logger = logger();
        crate::correlation::scope("outer", async {
            logger
                .log(LogRecord::new(LogLevel::Info, "s", "explicit").with_correlation_id("inner"));
        })
        .await;
        assert_eq!(
            logger.query(&LogQuery::new()).entries[0]
                .correlation_id
                .as_deref(),
            Some("inner")
        );
    }
}
