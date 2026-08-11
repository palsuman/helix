//! Atomic file writes (Task 1.7, REQ-NFR-002).
//!
//! The guarantee: at no point does the target path hold a partially written
//! file. Either the previous contents are there, or the new contents are. A
//! process killed mid-save cannot produce a truncated source file.
//!
//! The sequence, and why each step is load-bearing:
//!
//! ```text
//! 1. create  <dir>/.<name>.<pid>-<n>.helixtmp   ── same directory, so the
//!                                                  rename is within one
//!                                                  filesystem and therefore
//!                                                  atomic
//! 2. write + flush                              ── all bytes handed to the OS
//! 3. fsync the temp file                        ── bytes are durable *before*
//!                                                  anything points at them
//! 4. rename temp -> target                      ── the atomic step
//! 5. fsync the directory (unix)                 ── the rename itself is
//!                                                  durable, not just queued
//! ```
//!
//! Step 3 before step 4 is the part that is easy to get wrong. Renaming first
//! and syncing after leaves a window in which the directory entry points at a
//! file whose contents are still in the page cache; a power loss there yields a
//! zero-length file where the old one used to be, which is worse than not
//! having saved at all.
//!
//! **The temp file is in the same directory, not in `/tmp`.** `/tmp` is very
//! often a different filesystem, and `rename` across filesystems is not
//! atomic; it degrades to copy-then-delete, which is exactly the partial write
//! this module exists to prevent.
//!
//! **Permissions are carried over.** Replacing a file loses its mode
//! otherwise, so an executable script would silently stop being executable
//! after one save.
//!
//! Windows: `fs::rename` fails when the destination exists, so the platform's
//! `ReplaceFile`-equivalent path is used via a remove-then-rename fallback
//! guarded to the smallest possible window. The temp file is already durable at
//! that point, so the worst case is a leftover temp file with the full new
//! contents, never a truncated target.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Suffix marking a file as one of our in-flight writes.
///
/// Public because the watcher and the directory lister both have to ignore
/// these: a save should surface as one change to the target, not as a create
/// and delete of a temp file the user never asked about.
pub const TEMP_SUFFIX: &str = ".helixtmp";

/// Whether a path is one of this process family's in-flight write temporaries.
pub fn is_temp_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(TEMP_SUFFIX))
}

/// A point in the write sequence to stop at, standing in for a process that
/// died there.
///
/// This exists so the crash-safety guarantee is *tested* rather than asserted
/// in a comment. Simulating the kill inside the sequence is the only way to
/// check the invariant at each step without spawning and killing a real
/// process per case, which no test suite can do reliably on three platforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashPoint {
    /// Run to completion. What production uses.
    Never,
    /// Die after creating the temp file, before writing anything.
    AfterTempCreate,
    /// Die after writing bytes, before the fsync.
    AfterTempWrite,
    /// Die after the fsync, before the rename. The most dangerous-looking
    /// point, and the one where the target must still be untouched.
    AfterSync,
}

/// Write `bytes` to `path`, atomically.
///
/// Creates the parent directory if it does not exist, because "save this file"
/// meaning "…but only if you already made the folder" is not a useful contract.
pub fn write_atomic(path: impl AsRef<Path>, bytes: &[u8]) -> io::Result<()> {
    write_atomic_at(path, bytes, CrashPoint::Never)
}

/// [`write_atomic`], stopping early at `crash_point`. See [`CrashPoint`].
pub fn write_atomic_at(
    path: impl AsRef<Path>,
    bytes: &[u8],
    crash_point: CrashPoint,
) -> io::Result<()> {
    let path = path.as_ref();
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    if !parent.as_os_str().is_empty() && !parent.exists() {
        fs::create_dir_all(parent)?;
    }

    let temp = temp_path_for(path);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;

    if crash_point == CrashPoint::AfterTempCreate {
        return simulated_crash(crash_point);
    }

    // Every failure past this point removes the temp file before returning, so
    // a recoverable error (disk full, permissions) does not litter the user's
    // directory with debris.
    let result = (|| -> io::Result<()> {
        file.write_all(bytes)?;
        file.flush()?;
        if crash_point == CrashPoint::AfterTempWrite {
            return simulated_crash(crash_point);
        }

        // Durability before visibility. See the module docs.
        file.sync_all()?;
        drop(file);
        if crash_point == CrashPoint::AfterSync {
            return simulated_crash(crash_point);
        }

        copy_permissions(path, &temp)?;
        replace(&temp, path)?;
        sync_parent_dir(parent);
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Write text to `path` atomically. Convenience for the common caller.
pub fn write_atomic_str(path: impl AsRef<Path>, text: &str) -> io::Result<()> {
    write_atomic(path, text.as_bytes())
}

/// A unique sibling path for the in-flight copy.
///
/// Process id plus a per-process counter, so two windows of the same
/// installation saving the same file at the same moment cannot pick the same
/// temp name and corrupt each other's write.
fn temp_path_for(path: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let stem = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unnamed");
    let name = format!(
        ".{stem}.{}-{}{TEMP_SUFFIX}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    path.parent().unwrap_or_else(|| Path::new(".")).join(name)
}

fn simulated_crash(crash_point: CrashPoint) -> io::Result<()> {
    Err(io::Error::other(format!(
        "simulated crash at {crash_point:?}"
    )))
}

/// Carry the target's permissions onto the replacement, when there is a
/// target. A brand new file keeps the OS default.
fn copy_permissions(target: &Path, temp: &Path) -> io::Result<()> {
    match fs::metadata(target) {
        Ok(metadata) => fs::set_permissions(temp, metadata.permissions()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Move the temp file over the target.
#[cfg(not(windows))]
fn replace(temp: &Path, target: &Path) -> io::Result<()> {
    // POSIX rename over an existing path is atomic and needs nothing else.
    fs::rename(temp, target)
}

/// Move the temp file over the target.
///
/// `fs::rename` on Windows refuses an existing destination, so the target is
/// removed first. That opens a window in which the target does not exist,
/// which is a weaker guarantee than POSIX gives — but the temp file is already
/// fully written and fsynced, so a crash inside the window leaves the complete
/// new contents on disk under the temp name rather than a truncated target.
/// The read path's recovery for a missing file is to report it missing, which
/// is recoverable; a silently truncated source file is not.
#[cfg(windows)]
fn replace(temp: &Path, target: &Path) -> io::Result<()> {
    match fs::rename(temp, target) {
        Ok(()) => Ok(()),
        Err(_) if target.exists() => {
            fs::remove_file(target)?;
            fs::rename(temp, target)
        }
        Err(error) => Err(error),
    }
}

/// fsync the directory so the rename itself survives a power loss.
///
/// Best effort: some filesystems and all of Windows refuse to open a directory
/// for this, and a failure here does not make the write incorrect (the data is
/// already durable), only slightly less durable in its placement. Failing the
/// user's save over it would be the wrong trade.
#[cfg(not(windows))]
fn sync_parent_dir(parent: &Path) {
    if let Ok(dir) = std::fs::File::open(parent) {
        let _ = dir.sync_all();
    }
}

/// Windows cannot open a directory as a file, so there is nothing to sync.
#[cfg(windows)]
fn sync_parent_dir(_parent: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    #[test]
    fn a_completed_write_lands_the_new_contents() {
        let dir = TempDir::new("atomic-write");
        let path = dir.path().join("file.txt");
        write_atomic_str(&path, "new contents").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new contents");
    }

    #[test]
    fn a_write_creates_missing_parent_directories() {
        let dir = TempDir::new("atomic-parents");
        let path = dir.path().join("a/b/c/file.txt");
        write_atomic_str(&path, "deep").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "deep");
    }

    #[test]
    fn a_crash_at_any_point_leaves_the_original_file_intact() {
        // The Task 1.7 demo criterion, one case per stage of the sequence.
        for crash_point in [
            CrashPoint::AfterTempCreate,
            CrashPoint::AfterTempWrite,
            CrashPoint::AfterSync,
        ] {
            let dir = TempDir::new("atomic-crash");
            let path = dir.path().join("important.rs");
            fs::write(&path, "the original contents").unwrap();

            let result = write_atomic_at(&path, b"a replacement that never lands", crash_point);
            assert!(result.is_err(), "{crash_point:?} must not report success");
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                "the original contents",
                "{crash_point:?} damaged the target"
            );
        }
    }

    #[test]
    fn a_crash_before_the_rename_never_creates_the_target() {
        // Saving a new file that crashes must not leave an empty file behind
        // that the editor would then reopen as "saved, and empty".
        let dir = TempDir::new("atomic-crash-new");
        let path = dir.path().join("brand-new.txt");
        assert!(write_atomic_at(&path, b"content", CrashPoint::AfterSync).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn a_recoverable_failure_removes_its_temp_file() {
        let dir = TempDir::new("atomic-cleanup");
        let path = dir.path().join("file.txt");
        // AfterTempWrite fails inside the guarded closure, which is the path
        // that cleans up. (AfterTempCreate deliberately does not, standing in
        // for a process that is simply gone.)
        assert!(write_atomic_at(&path, b"x", CrashPoint::AfterTempWrite).is_err());
        let leftovers: Vec<PathBuf> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|p| is_temp_path(p))
            .collect();
        assert!(leftovers.is_empty(), "left behind {leftovers:?}");
    }

    #[test]
    fn repeated_writes_replace_rather_than_append() {
        let dir = TempDir::new("atomic-replace");
        let path = dir.path().join("file.txt");
        write_atomic_str(&path, "first and longer").unwrap();
        write_atomic_str(&path, "second").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "second");
    }

    #[test]
    fn a_temp_path_is_recognisable_and_a_normal_path_is_not() {
        let temp = temp_path_for(Path::new("/tmp/project/main.rs"));
        assert!(is_temp_path(&temp));
        assert!(!is_temp_path(Path::new("/tmp/project/main.rs")));
    }

    #[test]
    fn concurrent_writes_to_one_path_do_not_share_a_temp_name() {
        let target = Path::new("/tmp/project/main.rs");
        let first = temp_path_for(target);
        let second = temp_path_for(target);
        assert_ne!(first, second);
    }

    #[test]
    #[cfg(unix)]
    fn an_executable_file_stays_executable_after_a_save() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("atomic-mode");
        let path = dir.path().join("script.sh");
        fs::write(&path, "#!/bin/sh\necho old\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();

        write_atomic_str(&path, "#!/bin/sh\necho new\n").unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755, "the executable bit was dropped");
    }
}
