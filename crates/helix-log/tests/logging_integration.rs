//! End-to-end behaviour of the logging pipeline (Task 1.5, REQ-OBS-001).
//!
//! The unit tests in `src/` cover each piece in isolation. These drive the
//! whole pipeline the way the kernel does: several sources logging at several
//! levels, a rotating file on disk, a live sink standing in for the viewer's
//! stream bridge, a correlation scope around the work, and a query and export
//! afterwards.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use helix_log::{
    LogLevel, LogQuery, LogRecord, Logger, LoggerConfig, log_debug, log_error, log_info, log_trace,
    log_warn,
};

/// A unique temporary directory per test, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "helix-log-it-{label}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn read_lines(path: &Path) -> Vec<LogRecord> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<LogRecord>(line).expect("every line is a valid record"))
        .collect()
}

#[test]
fn several_services_log_at_several_levels_into_one_stream() {
    let dir = TempDir::new("levels");
    let logger = Logger::new(
        LoggerConfig::default()
            .with_default_level(LogLevel::Trace)
            .with_directory(dir.path()),
    );

    log_trace!(logger, "kernel.fs", "stat", "path" => "/tmp/a");
    log_debug!(logger, "kernel.ipc", "dispatching", "command" => "file.save");
    log_info!(logger, "kernel.stream", "counter subscribed", "channel" => "demo:counter");
    log_warn!(logger, "lsp_host", "server slow to start", "startup_ms" => 4200);
    log_error!(logger, "frontend.app", "render failed", "component" => "Editor");

    // The ring holds all five, in arrival order, from five different sources.
    let entries = logger.query(&LogQuery::new()).entries;
    assert_eq!(entries.len(), 5);
    assert_eq!(
        entries.iter().map(|r| r.level).collect::<Vec<_>>(),
        LogLevel::ALL.to_vec()
    );
    assert_eq!(
        logger.sources(),
        vec![
            "frontend.app",
            "kernel.fs",
            "kernel.ipc",
            "kernel.stream",
            "lsp_host"
        ]
    );

    // The file holds exactly the same records, as JSON lines.
    let from_file = read_lines(&logger.file_path().unwrap());
    assert_eq!(from_file, entries);
}

#[test]
fn a_quiet_module_can_be_turned_up_without_turning_up_everything() {
    let logger = Logger::new(LoggerConfig::default().with_default_level(LogLevel::Info));

    log_debug!(logger, "kernel.fs.watcher", "suppressed");
    logger.set_module_level("kernel.fs", Some(LogLevel::Trace));
    log_debug!(logger, "kernel.fs.watcher", "now recorded");
    log_debug!(logger, "kernel.ipc", "still suppressed");

    let messages: Vec<String> = logger
        .query(&LogQuery::new())
        .entries
        .into_iter()
        .map(|r| r.message)
        .collect();
    assert_eq!(messages, vec!["now recorded"]);
}

#[test]
fn the_file_rotates_at_its_configured_size_and_retains_five_files() {
    // The requirement's default is 50MB (asserted separately, below); this
    // exercises the same code path at 256KB so the test writes kilobytes
    // rather than a quarter of a gigabyte.
    let dir = TempDir::new("rotation");
    let max_bytes = 256 * 1024;
    let logger = Logger::new(
        LoggerConfig::default()
            .with_default_level(LogLevel::Trace)
            .with_directory(dir.path())
            .with_rotation(max_bytes, 5),
    );

    for i in 0..8_000 {
        log_info!(
            logger,
            "kernel.fs",
            "wrote a file",
            "path" => format!("/workspace/src/module-{i:05}.rs"),
            "bytes" => i,
        );
    }

    let files = logger.files();
    assert_eq!(
        files.len(),
        5,
        "the active file plus four archives, found {files:?}"
    );
    for path in &files {
        let size = fs::metadata(path).unwrap().len();
        assert!(
            size <= max_bytes,
            "{} grew to {size} bytes past the {max_bytes} byte cap",
            path.display()
        );
    }
    assert!(logger.metrics().rotations >= 4);

    // Rotation must not corrupt the format: every line in every retained file
    // is still a parseable record, and the newest record is in the active file.
    for path in &files {
        assert!(!read_lines(path).is_empty(), "{}", path.display());
    }
    let active = read_lines(&logger.file_path().unwrap());
    assert_eq!(
        active.last().unwrap().fields["bytes"],
        7_999,
        "the newest record must be in the active file"
    );
}

#[test]
fn the_documented_rotation_defaults_are_fifty_megabytes_and_five_files() {
    assert_eq!(helix_log::DEFAULT_MAX_FILE_BYTES, 50 * 1024 * 1024);
    assert_eq!(helix_log::DEFAULT_MAX_FILES, 5);
    assert_eq!(helix_log::DEFAULT_RING_CAPACITY, 10_000);

    let config = LoggerConfig::default();
    assert_eq!(config.max_file_bytes, 50 * 1024 * 1024);
    assert_eq!(config.max_files, 5);
    assert_eq!(config.ring_capacity, 10_000);
}

#[tokio::test]
async fn a_correlation_scope_links_every_record_emitted_while_serving_a_command() {
    let dir = TempDir::new("correlation");
    let logger = Arc::new(Logger::new(
        LoggerConfig::default()
            .with_default_level(LogLevel::Trace)
            .with_directory(dir.path()),
    ));

    // Ambient work, outside any command.
    log_info!(logger, "kernel.stream", "heartbeat");

    // Two "services" logging inside one command's scope, neither of which
    // knows a correlation ID exists.
    let inner = logger.clone();
    helix_log::correlation::scope("cmd-abc123", async move {
        log_debug!(inner, "kernel.ipc", "handling file.save");
        log_info!(inner, "kernel.fs", "wrote atomically", "path" => "/workspace/src/main.rs");
    })
    .await;

    let linked = logger.query(&LogQuery::new().with_correlation_id("cmd-abc123"));
    assert_eq!(linked.entries.len(), 2);
    assert_eq!(
        linked
            .entries
            .iter()
            .map(|r| r.source.as_str())
            .collect::<Vec<_>>(),
        vec!["kernel.ipc", "kernel.fs"]
    );

    // The correlation survives the trip to disk, which is what makes it
    // useful in a bug report rather than only in a live session.
    let from_file = read_lines(&logger.file_path().unwrap());
    assert_eq!(
        from_file
            .iter()
            .filter(|r| r.correlation_id.as_deref() == Some("cmd-abc123"))
            .count(),
        2
    );
}

#[test]
fn a_secret_never_reaches_any_sink_including_the_file() {
    let dir = TempDir::new("redaction");
    let logger = Logger::new(
        LoggerConfig::default()
            .with_default_level(LogLevel::Trace)
            .with_directory(dir.path()),
    );

    // A live sink standing in for the viewer's stream bridge.
    let observed: Arc<Mutex<Vec<LogRecord>>> = Arc::new(Mutex::new(Vec::new()));
    let sink_records = observed.clone();
    logger.add_sink(Arc::new(move |record: &LogRecord| {
        sink_records.lock().unwrap().push(record.clone());
    }));

    logger.register_secret("xyzzy-the-actual-api-key");
    log_info!(
        logger,
        "ai.provider",
        "calling https://alice:hunter2pass@api.example.com with xyzzy-the-actual-api-key",
        "api_key" => "xyzzy-the-actual-api-key",
        "content" => "fn main() { /* user source code */ }",
        "path" => "/workspace/src/main.rs",
    );

    let secrets = ["xyzzy-the-actual-api-key", "hunter2pass"];
    let file_text = fs::read_to_string(logger.file_path().unwrap()).unwrap();
    let sink_text = serde_json::to_string(&*observed.lock().unwrap()).unwrap();
    let ring_text = serde_json::to_string(&logger.query(&LogQuery::new()).entries).unwrap();

    for secret in secrets {
        for (name, text) in [
            ("file", &file_text),
            ("live sink", &sink_text),
            ("ring", &ring_text),
        ] {
            assert!(
                !text.contains(secret),
                "{secret} reached the {name} sink: {text}"
            );
        }
    }

    // File contents are omitted, the path is kept: REQ-OBS-001.10.
    let record = &logger.query(&LogQuery::new()).entries[0];
    assert_eq!(record.fields["content"], helix_log::OMITTED_CONTENT);
    assert_eq!(record.fields["path"], "/workspace/src/main.rs");
}

#[test]
fn a_filtered_export_reproduces_the_on_disk_format() {
    let dir = TempDir::new("export");
    let logger = Logger::new(
        LoggerConfig::default()
            .with_default_level(LogLevel::Trace)
            .with_directory(dir.path()),
    );

    log_info!(logger, "kernel.fs", "ordinary work");
    log_error!(logger, "kernel.ipc", "the failure being reported", "code" => "TIMEOUT");

    let (content, count) = logger.export(&LogQuery::new().with_min_level(LogLevel::Error));
    assert_eq!(count, 1);

    let exported: Vec<LogRecord> = content
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let on_disk = read_lines(&logger.file_path().unwrap());
    assert_eq!(
        exported,
        on_disk
            .into_iter()
            .filter(|r| r.level == LogLevel::Error)
            .collect::<Vec<_>>(),
        "an exported set must be byte-comparable with the log file it came from"
    );
}

#[test]
fn the_ring_keeps_the_newest_ten_thousand_entries_while_the_file_keeps_the_rest() {
    let dir = TempDir::new("ring-vs-file");
    let logger = Logger::new(
        LoggerConfig::default()
            .with_default_level(LogLevel::Trace)
            .with_ring_capacity(100)
            .with_directory(dir.path()),
    );

    for i in 0..250 {
        log_info!(logger, "kernel.fs", "work", "n" => i);
    }

    let result = logger.query(&LogQuery::new());
    assert_eq!(result.entries.len(), 100);
    assert_eq!(result.evicted, 150);
    assert_eq!(result.entries.first().unwrap().fields["n"], 150);

    // Nothing was lost from the durable sink, which is the point of having
    // both: the viewer is a window, the file is the record.
    assert_eq!(read_lines(&logger.file_path().unwrap()).len(), 250);
}
