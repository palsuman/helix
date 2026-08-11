//! The rotating file sink (REQ-OBS-001.6, .7).
//!
//! 50MB per file and 5 files by default, both configurable. The file sink is
//! the only sink that is always present: the developer panel exists only
//! while it is open and stdout exists only under a CLI launch, so the file is
//! what a bug report can be built from after the fact.
//!
//! Rotation happens *before* the write that would exceed the limit, not
//! after, so no file ever exceeds `max_bytes`. The alternative (rotate once
//! over the limit) means the size cap is advisory, and a single very large
//! record could push a file arbitrarily past it.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

/// Default size cap per file (REQ-OBS-001.6): 50MB.
pub const DEFAULT_MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;

/// Default number of files retained (REQ-OBS-001.6): the active file plus
/// four archives.
pub const DEFAULT_MAX_FILES: usize = 5;

/// Base name of the active log file.
pub const DEFAULT_FILE_NAME: &str = "helix.log";

/// An append-only JSON-lines file that rotates on size.
///
/// Archives are numbered from the newest: `helix.log` is active, `helix.log.1`
/// is the previous file, up to `helix.log.<max_files - 1>`. Numbering by
/// recency rather than by creation order means the newest archive is always
/// `.1`, which is what someone reaching for the file after a crash expects.
#[derive(Debug)]
pub struct RotatingFileSink {
    directory: PathBuf,
    file_name: String,
    max_bytes: u64,
    max_files: usize,
    writer: BufWriter<File>,
    current_bytes: u64,
    rotations: u64,
}

impl RotatingFileSink {
    /// Open (or create) the active log file, appending to it if it already
    /// exists so a restart does not discard the previous session's tail.
    pub fn open(
        directory: impl AsRef<Path>,
        file_name: impl Into<String>,
        max_bytes: u64,
        max_files: usize,
    ) -> io::Result<Self> {
        let directory = directory.as_ref().to_path_buf();
        fs::create_dir_all(&directory)?;
        let file_name = file_name.into();
        let path = directory.join(&file_name);
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let current_bytes = file.metadata().map(|m| m.len()).unwrap_or(0);

        Ok(Self {
            directory,
            file_name,
            // A cap of zero would rotate on every record; one byte is the
            // smallest cap with defined behaviour.
            max_bytes: max_bytes.max(1),
            max_files: max_files.max(1),
            writer: BufWriter::new(file),
            current_bytes,
            rotations: 0,
        })
    }

    pub fn path(&self) -> PathBuf {
        self.directory.join(&self.file_name)
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    pub fn max_files(&self) -> usize {
        self.max_files
    }

    pub fn current_bytes(&self) -> u64 {
        self.current_bytes
    }

    pub fn rotations(&self) -> u64 {
        self.rotations
    }

    /// Append one line, rotating first if the line would not fit.
    pub fn write_line(&mut self, line: &str) -> io::Result<()> {
        let needed = line.len() as u64 + 1; // + newline
        // An empty file is never rotated, even if the record alone exceeds
        // the cap: rotating would produce an empty archive and still have to
        // write the record somewhere.
        if self.current_bytes > 0 && self.current_bytes + needed > self.max_bytes {
            self.rotate()?;
        }
        self.writer.write_all(line.as_bytes())?;
        self.writer.write_all(b"\n")?;
        // Flushed per record rather than per buffer: a log's value is
        // highest immediately after a crash, and a buffered tail is exactly
        // the part that explains one.
        self.writer.flush()?;
        self.current_bytes += needed;
        Ok(())
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }

    /// Shift archives up by one, discarding the oldest, and start a new
    /// active file.
    fn rotate(&mut self) -> io::Result<()> {
        self.writer.flush()?;

        let active = self.path();
        if self.max_files > 1 {
            // Oldest first, so nothing is overwritten before it has been
            // moved.
            let oldest = self.archive_path(self.max_files - 1);
            if oldest.exists() {
                fs::remove_file(&oldest)?;
            }
            for index in (1..self.max_files - 1).rev() {
                let from = self.archive_path(index);
                if from.exists() {
                    fs::rename(&from, self.archive_path(index + 1))?;
                }
            }
            fs::rename(&active, self.archive_path(1))?;
        } else {
            // Retaining a single file means the active one is truncated;
            // there is nowhere to move it to.
            fs::remove_file(&active)?;
        }

        let file = OpenOptions::new().create(true).append(true).open(&active)?;
        self.writer = BufWriter::new(file);
        self.current_bytes = 0;
        self.rotations += 1;
        Ok(())
    }

    fn archive_path(&self, index: usize) -> PathBuf {
        self.directory.join(format!("{}.{index}", self.file_name))
    }

    /// Existing log files, newest first: the active file followed by its
    /// archives. Used by diagnostics export and by tests.
    pub fn files(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let active = self.path();
        if active.exists() {
            files.push(active);
        }
        for index in 1..self.max_files {
            let path = self.archive_path(index);
            if path.exists() {
                files.push(path);
            }
        }
        files
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A unique temporary directory per test, without pulling in a
    /// temp-file crate for four call sites.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let unique = format!(
                "helix-log-{label}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::SeqCst)
            );
            let path = std::env::temp_dir().join(unique);
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

    fn line_count(path: &Path) -> usize {
        fs::read_to_string(path).unwrap().lines().count()
    }

    #[test]
    fn the_defaults_match_the_requirement() {
        assert_eq!(DEFAULT_MAX_FILE_BYTES, 50 * 1024 * 1024);
        assert_eq!(DEFAULT_MAX_FILES, 5);
    }

    #[test]
    fn lines_are_appended_and_readable_immediately() {
        let dir = TempDir::new("append");
        let mut sink = RotatingFileSink::open(dir.path(), DEFAULT_FILE_NAME, 1_000_000, 5).unwrap();
        sink.write_line("{\"a\":1}").unwrap();
        sink.write_line("{\"a\":2}").unwrap();

        let contents = fs::read_to_string(sink.path()).unwrap();
        assert_eq!(contents, "{\"a\":1}\n{\"a\":2}\n");
        assert_eq!(sink.rotations(), 0);
    }

    #[test]
    fn reopening_appends_rather_than_truncating() {
        let dir = TempDir::new("reopen");
        {
            let mut sink =
                RotatingFileSink::open(dir.path(), DEFAULT_FILE_NAME, 1_000_000, 5).unwrap();
            sink.write_line("first").unwrap();
        }
        let mut sink = RotatingFileSink::open(dir.path(), DEFAULT_FILE_NAME, 1_000_000, 5).unwrap();
        assert_eq!(sink.current_bytes(), 6);
        sink.write_line("second").unwrap();
        assert_eq!(line_count(&sink.path()), 2);
    }

    #[test]
    fn no_file_ever_exceeds_the_size_cap() {
        let dir = TempDir::new("cap");
        // Each line is 9 bytes + newline = 10; a 25-byte cap fits two.
        let mut sink = RotatingFileSink::open(dir.path(), DEFAULT_FILE_NAME, 25, 5).unwrap();
        for i in 0..20 {
            sink.write_line(&format!("line-{i:03}")).unwrap();
        }

        for path in sink.files() {
            let size = fs::metadata(&path).unwrap().len();
            assert!(
                size <= 25,
                "{} grew to {size} bytes, past the 25 byte cap",
                path.display()
            );
        }
        assert!(sink.rotations() > 0);
    }

    #[test]
    fn rotation_retains_exactly_max_files_and_discards_the_oldest() {
        let dir = TempDir::new("retain");
        let mut sink = RotatingFileSink::open(dir.path(), DEFAULT_FILE_NAME, 25, 5).unwrap();
        for i in 0..100 {
            sink.write_line(&format!("line-{i:03}")).unwrap();
        }

        let files = sink.files();
        assert_eq!(
            files.len(),
            5,
            "5 files retained (active plus four archives), found {files:?}"
        );

        // The newest archive is .1 and holds newer records than .4.
        let newest_archive = fs::read_to_string(dir.path().join("helix.log.1")).unwrap();
        let oldest_archive = fs::read_to_string(dir.path().join("helix.log.4")).unwrap();
        assert!(
            newest_archive > oldest_archive,
            "archives are ordered by recency: .1 must hold later lines than .4"
        );
        assert!(
            !dir.path().join("helix.log.5").exists(),
            "an archive beyond the retention limit must be discarded"
        );
    }

    #[test]
    fn the_last_record_written_is_always_in_the_active_file() {
        let dir = TempDir::new("tail");
        let mut sink = RotatingFileSink::open(dir.path(), DEFAULT_FILE_NAME, 25, 3).unwrap();
        for i in 0..50 {
            sink.write_line(&format!("line-{i:03}")).unwrap();
        }
        let active = fs::read_to_string(sink.path()).unwrap();
        assert!(
            active.contains("line-049"),
            "the newest record must be in the active file, found: {active:?}"
        );
    }

    #[test]
    fn a_single_file_retention_truncates_instead_of_archiving() {
        let dir = TempDir::new("single");
        let mut sink = RotatingFileSink::open(dir.path(), DEFAULT_FILE_NAME, 25, 1).unwrap();
        for i in 0..20 {
            sink.write_line(&format!("line-{i:03}")).unwrap();
        }
        assert_eq!(sink.files().len(), 1);
        assert!(!dir.path().join("helix.log.1").exists());
    }

    #[test]
    fn a_record_larger_than_the_cap_is_still_written() {
        let dir = TempDir::new("oversize");
        let mut sink = RotatingFileSink::open(dir.path(), DEFAULT_FILE_NAME, 10, 3).unwrap();
        let long = "x".repeat(100);
        sink.write_line(&long).unwrap();
        assert!(fs::read_to_string(sink.path()).unwrap().contains(&long));
    }

    #[test]
    fn the_directory_is_created_if_it_does_not_exist() {
        let dir = TempDir::new("mkdir");
        let nested = dir.path().join("a").join("b");
        let sink = RotatingFileSink::open(&nested, DEFAULT_FILE_NAME, 1_000, 3).unwrap();
        assert!(sink.directory().exists());
    }
}
