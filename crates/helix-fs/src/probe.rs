//! Network filesystem detection by latency probe (Task 1.7, REQ-FS-004
//! failure modes).
//!
//! The reason this matters: native change notification does not work reliably
//! over SMB or NFS. The protocols either do not carry change events at all or
//! carry them with delays and gaps, so a watcher that trusts them on a network
//! share reports nothing and the user concludes the IDE cannot see their files.
//! Polling is slower and costlier, but it is *correct* there, so the watcher
//! needs to know which kind of filesystem it is looking at.
//!
//! ## Why latency and not the mount table
//!
//! Asking the OS whether a path is on a network filesystem means three
//! platform-specific implementations (`statfs` type codes on Linux,
//! `f_fstypename` on macOS, `GetDriveType` plus UNC parsing on Windows), each
//! with its own list of filesystem type constants that grows every time a new
//! one appears. And it still gets the answer wrong for the cases that matter:
//! an overlay filesystem in a container, a FUSE mount over SSH, a virtualised
//! folder shared into a VM. All of those are local by type and slow by
//! behaviour.
//!
//! Latency is the property the decision actually depends on. A local disk
//! answers a create-fsync-stat-delete cycle in well under a millisecond; a
//! network share takes tens of milliseconds at best. The 500ms threshold from
//! the requirement is far above the noise floor of any local disk, including a
//! loaded spinning one, so a false positive needs a genuinely pathological
//! local filesystem — at which point polling is the right choice anyway.
//!
//! ## The probe writes
//!
//! Read-only probing would measure the page cache, not the filesystem. The
//! probe therefore creates a small file, fsyncs it, stats it, and removes it,
//! which is the same sequence a save performs and so measures the thing that
//! will actually be slow. A read-only directory cannot be probed; that is
//! reported as [`ProbeOutcome::unavailable`] and the caller keeps the native
//! watcher, because being unable to write is not evidence of being remote.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

/// Above this, treat the filesystem as remote and poll instead of watching
/// natively (REQ-FS-004 failure modes).
pub const NETWORK_LATENCY_THRESHOLD: Duration = Duration::from_millis(500);

/// Polling interval used for remote roots and for overflow paths.
pub const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Result of probing one directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeOutcome {
    /// Measured round trip, or `None` when the probe could not run.
    pub latency: Option<Duration>,
    /// True when the measurement exceeded the threshold.
    pub remote: bool,
}

impl ProbeOutcome {
    fn unavailable() -> Self {
        Self {
            latency: None,
            remote: false,
        }
    }
}

/// Measure write-and-sync latency in `dir` and classify it.
///
/// The probe runs three times and takes the *median*, because a single
/// measurement on a busy machine can be an outlier in either direction and this
/// decision is sticky for the lifetime of the watch.
pub fn probe(dir: impl AsRef<Path>) -> ProbeOutcome {
    probe_with_threshold(dir, NETWORK_LATENCY_THRESHOLD)
}

/// [`probe`] with an explicit threshold, so the classification boundary can be
/// tested without needing a genuinely slow filesystem to hand.
pub fn probe_with_threshold(dir: impl AsRef<Path>, threshold: Duration) -> ProbeOutcome {
    let dir = dir.as_ref();
    let mut samples = Vec::with_capacity(3);
    for attempt in 0..3 {
        match measure_once(dir, attempt) {
            Some(latency) => samples.push(latency),
            // A failure on the first attempt means the directory is not
            // writable or not there; retrying would just be slow about
            // reaching the same conclusion.
            None => return ProbeOutcome::unavailable(),
        }
    }
    samples.sort_unstable();
    let latency = samples[samples.len() / 2];
    ProbeOutcome {
        latency: Some(latency),
        remote: latency > threshold,
    }
}

fn measure_once(dir: &Path, attempt: u32) -> Option<Duration> {
    let path = dir.join(format!(
        ".helix-probe-{}-{attempt}{}",
        std::process::id(),
        crate::atomic::TEMP_SUFFIX
    ));
    let started = Instant::now();
    let result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.write_all(b"helix latency probe")?;
        file.sync_all()?;
        drop(file);
        // Stat and remove are included: on a network filesystem the metadata
        // round trips are frequently slower than the write itself.
        fs::metadata(&path)?;
        fs::remove_file(&path)
    })();
    let elapsed = started.elapsed();

    if result.is_err() {
        // Best effort, in case the failure came after creation.
        let _ = fs::remove_file(&path);
        return None;
    }
    Some(elapsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    #[test]
    fn a_local_temp_directory_is_not_classified_as_remote() {
        let dir = TempDir::new("probe-local");
        let outcome = probe(dir.path());
        assert!(!outcome.remote);
        let latency = outcome.latency.expect("a writable directory must measure");
        assert!(
            latency < NETWORK_LATENCY_THRESHOLD,
            "a local disk measured {latency:?}, which would force polling"
        );
    }

    #[test]
    fn an_impossibly_low_threshold_classifies_even_a_local_disk_as_remote() {
        // Exercises the comparison and the sticky decision it feeds, without
        // needing a real network share in CI.
        let dir = TempDir::new("probe-threshold");
        let outcome = probe_with_threshold(dir.path(), Duration::from_nanos(1));
        assert!(outcome.remote);
    }

    #[test]
    fn a_missing_directory_is_unavailable_rather_than_remote() {
        // Not writable is not evidence of being remote, and misreading it as
        // such would silently degrade a perfectly good local root to polling.
        let dir = TempDir::new("probe-missing");
        let outcome = probe(dir.path().join("not-there"));
        assert_eq!(outcome, ProbeOutcome::unavailable());
        assert!(!outcome.remote);
    }

    #[test]
    fn the_probe_leaves_nothing_behind() {
        let dir = TempDir::new("probe-clean");
        probe(dir.path());
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(leftovers.is_empty(), "left behind {leftovers:?}");
    }
}
