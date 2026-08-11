//! A scratch directory for tests, and for the integration tests next door.
//!
//! Hand-rolled rather than pulled from `tempfile`, matching what `helix-config`
//! and `helix-kernel` already do: one small type is cheaper than a dependency
//! that has to be pinned, audited, and updated for the rest of the project's
//! life.
//!
//! Compiled into the library under `cfg(test)` for the unit tests, and behind
//! the `testutil` feature so `tests/` can use the same type instead of copying
//! it.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// A directory under the system temp dir, removed on drop.
pub struct TempDir(PathBuf);

impl TempDir {
    /// Create a fresh directory. The label appears in the path, so a test that
    /// leaves one behind after a hard failure is identifiable.
    pub fn new(label: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "helix-fs-{label}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("scratch directory must be creatable");
        // Canonicalized because macOS reports `/var/folders/...` from
        // `temp_dir()` but `/private/var/folders/...` in watcher events, and a
        // test comparing the two would fail for a reason that has nothing to
        // do with what it is testing.
        let path = fs::canonicalize(&path).unwrap_or(path);
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Create a file, including any missing parent directories.
    pub fn write(&self, relative: &str, contents: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, contents).unwrap();
        path
    }

    /// Create a directory, including any missing parents.
    pub fn mkdir(&self, relative: &str) -> PathBuf {
        let path = self.0.join(relative);
        fs::create_dir_all(&path).unwrap();
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
