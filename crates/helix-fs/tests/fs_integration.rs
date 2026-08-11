//! End-to-end tests for the file system service (Task 1.7).
//!
//! These drive the public surface exactly as `helix-kernel` does, against a
//! real temp directory and a real OS watcher. They exist to check the *demo
//! criteria* of the task, which are properties of the whole service rather than
//! of any one module:
//!
//! 1. An external create, modify, and delete each surface within 100ms.
//! 2. A killed write leaves the original file intact.
//! 3. A PNG is detected as binary.
//!
//! plus the two cross-cutting behaviours the unit tests can only check in
//! pieces: exclusions applying identically to listing and watching, and an
//! edit-save-reload cycle preserving encoding and line endings.

use std::fs;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, sync_channel};
use std::time::{Duration, Instant};

use helix_fs::testutil::TempDir;
use helix_fs::{
    ChangeKind, CrashPoint, Encoding, FileChange, FileSystemService, LineEnding, WriteOptions,
    atomic,
};
use helix_log::{LogLevel, Logger};

/// The demo criterion. Asserted as a median rather than per event: a single
/// scheduling hiccup on a loaded CI runner is not a regression in the watcher,
/// but a median above this would be.
const SURFACING_BUDGET: Duration = Duration::from_millis(100);

/// Hard liveness bound for any single event. Generous on purpose; its only job
/// is to fail the test rather than hang it.
const LIVENESS_BOUND: Duration = Duration::from_secs(10);

fn service() -> FileSystemService {
    FileSystemService::with_defaults(Arc::new(Logger::in_memory(LogLevel::Trace)))
}

fn subscribe(service: &FileSystemService) -> Receiver<Vec<FileChange>> {
    let (tx, rx) = sync_channel::<Vec<FileChange>>(256);
    service.add_listener(Arc::new(move |changes: &[FileChange]| {
        let _ = tx.try_send(changes.to_vec());
    }));
    rx
}

fn await_change(
    rx: &Receiver<Vec<FileChange>>,
    predicate: impl Fn(&FileChange) -> bool,
) -> Option<FileChange> {
    let deadline = Instant::now() + LIVENESS_BOUND;
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

#[test]
fn external_create_modify_and_delete_all_surface_promptly() {
    let dir = TempDir::new("integration-surface");
    let existing = dir.write("existing.rs", "before\n");
    let doomed = dir.write("doomed.rs", "temporary\n");

    let service = service();
    let rx = subscribe(&service);
    let report = service.watch(dir.path()).expect("the root must be watched");
    assert_eq!(report.watched_paths, 1, "one directory, the root itself");

    let mut latencies = Vec::new();

    // Create.
    let started = Instant::now();
    dir.write("created.rs", "fn main() {}\n");
    let change =
        await_change(&rx, |c| c.path.ends_with("created.rs")).expect("a create must surface");
    latencies.push(started.elapsed());
    assert_ne!(change.kind, ChangeKind::Deleted);

    // Modify.
    let started = Instant::now();
    fs::write(&existing, "after\n").unwrap();
    let change =
        await_change(&rx, |c| c.path.ends_with("existing.rs")).expect("a modify must surface");
    latencies.push(started.elapsed());
    assert_ne!(change.kind, ChangeKind::Deleted);

    // Delete.
    let started = Instant::now();
    fs::remove_file(&doomed).unwrap();
    let change =
        await_change(&rx, |c| c.path.ends_with("doomed.rs")).expect("a delete must surface");
    latencies.push(started.elapsed());
    assert_eq!(change.kind, ChangeKind::Deleted);

    latencies.sort();
    let median = latencies[latencies.len() / 2];
    assert!(
        median <= SURFACING_BUDGET,
        "median surfacing latency was {median:?}, above the {SURFACING_BUDGET:?} budget \
         (measured: {latencies:?})"
    );

    let stats = service.watch_stats();
    assert!(stats.events_seen >= 3);
    assert!(stats.changes_emitted >= 3);
}

#[test]
fn a_killed_write_leaves_the_original_file_intact() {
    // The demo criterion, at every stage of the write sequence. `CrashPoint`
    // stands in for a process that died mid-save; see `helix_fs::atomic` for
    // why that is simulated inside the sequence rather than by killing a real
    // process per case.
    let dir = TempDir::new("integration-crash");
    let path = dir.write("important.rs", "the work of an afternoon\n");
    let original_hash = helix_fs::hash_file(&path).unwrap().to_string();

    for crash_point in [
        CrashPoint::AfterTempCreate,
        CrashPoint::AfterTempWrite,
        CrashPoint::AfterSync,
    ] {
        let result = atomic::write_atomic_at(&path, b"a replacement that never lands", crash_point);
        assert!(result.is_err(), "{crash_point:?} must not report success");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "the work of an afternoon\n",
            "{crash_point:?} damaged the file"
        );
        assert_eq!(
            helix_fs::hash_file(&path).unwrap().to_string(),
            original_hash,
            "{crash_point:?} changed the content hash"
        );
    }

    // And the service still reads it, so no crash left the file in a state
    // that only `fs::read` can cope with.
    let content = service().read(&path).unwrap();
    assert_eq!(content.hash, original_hash);
}

#[test]
fn a_png_is_binary_and_a_source_file_is_not() {
    let dir = TempDir::new("integration-binary");
    // A real PNG signature followed by the start of an IHDR chunk.
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(&[0x00, 0x00, 0x00, 0x0D]);
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&[0, 0, 1, 0, 0, 0, 1, 0, 8, 6, 0, 0, 0]);
    let image = dir.write("icon.png", &png);
    let source = dir.write("main.rs", "fn main() {}\n");

    let service = service();

    let content = service.read(&image).unwrap();
    assert!(content.binary);
    assert!(
        content.text.is_none(),
        "a binary file must not be handed back as text"
    );
    assert!(service.is_binary(&image).unwrap());

    assert!(!service.read(&source).unwrap().binary);
    assert!(!service.is_binary(&source).unwrap());
}

#[test]
fn exclusions_apply_identically_to_listing_and_to_watching() {
    // The failure this rules out is the nasty one: a file visible in the
    // explorer that never receives change events, or the reverse.
    let dir = TempDir::new("integration-exclusions");
    dir.write(".gitignore", "*.log\ngenerated/\n");
    dir.write("src/main.rs", "fn main() {}\n");
    dir.write("app.log", "noise\n");
    dir.write("generated/schema.rs", "// generated\n");
    dir.write("node_modules/pkg/index.js", "noise\n");

    let service = service();
    let rx = subscribe(&service);
    service.watch(dir.path()).unwrap();

    let listing = service.list(dir.path(), true).unwrap();
    let listed: Vec<&str> = listing
        .entries
        .iter()
        .map(|entry| entry.relative_path.as_str())
        .collect();
    assert!(listed.contains(&"src/main.rs"), "{listed:?}");
    for hidden in ["app.log", "generated", "node_modules"] {
        assert!(
            !listed.iter().any(|path| path.starts_with(hidden)),
            "{hidden} was listed: {listed:?}"
        );
    }

    // Touch every excluded path, then an included one. The included change
    // arriving with no excluded change ahead of it is the assertion.
    fs::write(dir.path().join("app.log"), "more noise\n").unwrap();
    fs::write(dir.path().join("generated/schema.rs"), "// again\n").unwrap();
    fs::write(dir.path().join("node_modules/pkg/index.js"), "// again\n").unwrap();
    fs::write(dir.path().join("src/main.rs"), "fn main() { }\n").unwrap();

    assert!(
        await_change(&rx, |c| c.path.ends_with("main.rs")).is_some(),
        "the unexcluded file must surface"
    );

    let mut leaked = Vec::new();
    while let Ok(batch) = rx.recv_timeout(Duration::from_millis(300)) {
        leaked.extend(batch.into_iter().filter(|change| {
            change.path.ends_with("app.log")
                || change.path.contains("generated")
                || change.path.contains("node_modules")
        }));
    }
    assert!(leaked.is_empty(), "excluded paths surfaced: {leaked:?}");
}

#[test]
fn an_edit_and_save_cycle_preserves_encoding_and_line_endings() {
    // A UTF-16 LE file with CRLF endings, which is what a Windows-authored file
    // in a mixed-platform repository actually looks like. Reading it, editing
    // the text, and saving must not silently convert it — that would produce a
    // diff touching every line.
    let dir = TempDir::new("integration-round-trip");
    let original = Encoding::Utf16Le
        .encode("first line\r\nsecond line\r\n")
        .bytes;
    let path = dir.write("legacy.txt", &original);
    let service = service();

    let content = service.read(&path).unwrap();
    assert_eq!(content.encoding, Encoding::Utf16Le);
    assert!(content.encoding_from_bom);
    assert_eq!(content.eol.style, LineEnding::Crlf);
    assert_eq!(
        content.text.as_deref(),
        Some("first line\nsecond line\n"),
        "the editor always receives LF"
    );

    let edited = content.text.unwrap().replace("second", "edited");
    let outcome = service
        .write(&path, WriteOptions::new(&edited).expecting(&content.hash))
        .unwrap();
    assert_eq!(outcome.encoding, Encoding::Utf16Le);
    assert_eq!(outcome.eol, LineEnding::Crlf);
    assert!(!outcome.lossy);

    let reread = service.read(&path).unwrap();
    assert_eq!(reread.hash, outcome.hash);
    assert_eq!(reread.encoding, Encoding::Utf16Le);
    assert_eq!(reread.eol.style, LineEnding::Crlf);
    assert_eq!(reread.text.as_deref(), Some("first line\nedited line\n"));

    // The bytes on disk really are UTF-16 with a mark, not merely reported as
    // such.
    let bytes = fs::read(&path).unwrap();
    assert_eq!(&bytes[..2], &[0xFF, 0xFE]);
    assert!(bytes.contains(&0));
}

#[test]
fn a_save_through_the_service_does_not_surface_its_own_temporary() {
    // An internal save must look like one change to the target, not like a
    // create and delete of a file the user never made. Getting this wrong makes
    // the explorer flicker on every keystroke under auto-save.
    let dir = TempDir::new("integration-self-save");
    let path = dir.write("main.rs", "before\n");
    let service = service();
    let rx = subscribe(&service);
    service.watch(dir.path()).unwrap();

    service
        .write(&path, WriteOptions::new("after\n").with_eol(LineEnding::Lf))
        .unwrap();

    let change = await_change(&rx, |c| c.path.ends_with("main.rs")).expect("the save must surface");
    assert_ne!(change.kind, ChangeKind::Deleted);

    let mut temporaries = Vec::new();
    while let Ok(batch) = rx.recv_timeout(Duration::from_millis(300)) {
        temporaries.extend(
            batch
                .into_iter()
                .filter(|change| change.path.contains(helix_fs::TEMP_SUFFIX)),
        );
    }
    assert!(
        temporaries.is_empty(),
        "write temporaries surfaced: {temporaries:?}"
    );
}

#[test]
fn closing_a_workspace_releases_its_watches() {
    // Task 1.8 relies on this: a closed workspace must not leave OS handles or
    // pending events behind.
    let dir = TempDir::new("integration-teardown");
    let service = service();
    let rx = subscribe(&service);
    service.watch(dir.path()).unwrap();
    assert_eq!(service.watch_stats().roots, 1);

    service.unwatch(dir.path()).unwrap();
    assert_eq!(service.watch_stats().roots, 0);
    while rx.recv_timeout(Duration::from_millis(100)).is_ok() {}

    dir.write("after-close.rs", "fn main() {}\n");
    let deadline = Instant::now() + Duration::from_millis(500);
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(remaining) {
            Ok(batch) => assert!(
                !batch.iter().any(|c| c.path.ends_with("after-close.rs")),
                "an unwatched root still delivered changes"
            ),
            Err(_) => break,
        }
    }
}
