//! Directory listing with stat information (Task 1.7).
//!
//! One walk serves the explorer, the watcher's path budget, and the search
//! index's initial scan, and it uses the same [`Exclusions`] as the watcher so
//! the three can never disagree about what is in the workspace.
//!
//! ## Symlinks are reported, never followed
//!
//! A followed symlink can point at a parent directory, and a walk that follows
//! it does not terminate. It can also point outside the workspace entirely,
//! which would put files the user never opened into the explorer and the index.
//! Links are therefore listed as entries with [`FileEntry::is_symlink`] set, and
//! the walk stops there. Opening one still works: the read path resolves it,
//! because reading through a link is what the user meant.
//!
//! ## Errors are per-entry, not fatal
//!
//! A permission-denied subdirectory in the middle of a tree must not fail the
//! listing of everything around it — that turns one inaccessible folder into an
//! empty explorer. Unreadable entries are counted in
//! [`Listing::unreadable_paths`] and the walk continues, so the caller can
//! surface "3 folders could not be read" alongside the results it does have.

use std::fs::{self, Metadata};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::exclude::Exclusions;

/// One filesystem entry with the stat information the explorer needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct FileEntry {
    /// Absolute path.
    pub path: String,
    /// Path relative to the listed root, with forward slashes on every
    /// platform so the frontend can treat it as an opaque, comparable key.
    pub relative_path: String,
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    /// Size in bytes. Zero for directories, whose on-disk size is a property
    /// of the filesystem rather than of the project.
    pub size: u64,
    /// Last modification time, milliseconds since the Unix epoch. `None` when
    /// the filesystem does not report one.
    pub modified_ms: Option<u64>,
    pub readonly: bool,
}

impl FileEntry {
    fn from_metadata(
        root: &Path,
        path: &Path,
        metadata: &Metadata,
        is_symlink: bool,
    ) -> Option<Self> {
        let is_dir = metadata.is_dir();
        Some(Self {
            path: path.to_string_lossy().into_owned(),
            relative_path: relative_slashed(root, path),
            name: path.file_name()?.to_string_lossy().into_owned(),
            is_dir,
            is_symlink,
            size: if is_dir { 0 } else { metadata.len() },
            modified_ms: metadata.modified().ok().and_then(to_epoch_ms),
            readonly: metadata.permissions().readonly(),
        })
    }
}

/// The result of a walk.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct Listing {
    pub entries: Vec<FileEntry>,
    /// Directories visited, including the root. This is the number the watcher
    /// budget is measured against (REQ-FS-004.6), because a recursive watch
    /// registers per directory, not per file.
    pub directory_count: u32,
    pub file_count: u32,
    /// Entries that could not be stat'ed or directories that could not be
    /// read. Reported so the caller can say so rather than showing a
    /// mysteriously short list.
    pub unreadable_paths: Vec<String>,
    /// True when the walk stopped at the configured depth limit, so the caller
    /// knows the listing is a window rather than the whole tree.
    pub truncated_by_depth: bool,
}

/// Walk `root`, applying `exclusions`.
///
/// `recursive` false lists only the immediate children, which is what the
/// explorer wants when a user expands one folder. `recursive` true walks the
/// whole subtree subject to the exclusions' depth limit.
pub fn list(root: impl AsRef<Path>, exclusions: &Exclusions, recursive: bool) -> Listing {
    let root = root.as_ref();
    let mut listing = Listing::default();
    // Depth-first with an explicit stack rather than recursion: a deeply nested
    // node_modules can be hundreds of levels down, and a stack overflow in the
    // kernel is not a recoverable error.
    let mut stack = vec![(root.to_path_buf(), 0usize)];

    while let Some((dir, depth)) = stack.pop() {
        listing.directory_count += 1;
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => {
                listing
                    .unreadable_paths
                    .push(dir.to_string_lossy().into_owned());
                continue;
            }
        };

        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            // `symlink_metadata` describes the link itself. Following it here
            // would report a link to a 4GB file as a 4GB file and, worse, a
            // link to a directory as a directory the walk should descend into.
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                listing
                    .unreadable_paths
                    .push(path.to_string_lossy().into_owned());
                continue;
            };
            let is_symlink = metadata.file_type().is_symlink();
            let is_dir = metadata.is_dir();

            if exclusions.is_excluded(&path, is_dir) {
                continue;
            }

            match FileEntry::from_metadata(root, &path, &metadata, is_symlink) {
                Some(file_entry) => {
                    if is_dir {
                        // Counted as a directory only once it is visited, so
                        // `directory_count` matches what the watcher registers.
                    } else {
                        listing.file_count += 1;
                    }
                    listing.entries.push(file_entry);
                }
                None => continue,
            }

            if recursive && is_dir && !is_symlink {
                if exclusions.should_descend(&path, depth + 1) {
                    stack.push((path, depth + 1));
                } else if exclusions.max_depth().is_some() {
                    listing.truncated_by_depth = true;
                }
            }
        }

        if !recursive {
            break;
        }
    }

    // Directories before files, then by name. Deterministic output matters:
    // the explorer renders it directly and the index diffs it between runs.
    listing.entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.relative_path.cmp(&b.relative_path))
    });
    listing
}

/// Stat one path, without walking anything.
pub fn stat(path: impl AsRef<Path>) -> std::io::Result<FileEntry> {
    let path = path.as_ref();
    let metadata = fs::symlink_metadata(path)?;
    let is_symlink = metadata.file_type().is_symlink();
    let root = path.parent().unwrap_or(path);
    FileEntry::from_metadata(root, path, &metadata, is_symlink).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{} has no file name component", path.display()),
        )
    })
}

fn relative_slashed(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn to_epoch_ms(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|since| since.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exclude::ExclusionConfig;
    use crate::testutil::TempDir;

    fn tree() -> TempDir {
        let dir = TempDir::new("listing");
        dir.write("README.md", "# project\n");
        dir.write("src/main.rs", "fn main() {}\n");
        dir.write("src/lib/util.rs", "pub fn u() {}\n");
        dir.write("node_modules/react/index.js", "module.exports = {};\n");
        dir.write("target/debug/app", "binary");
        dir
    }

    fn exclusions(dir: &TempDir, config: ExclusionConfig) -> Exclusions {
        Exclusions::build(dir.path(), &config)
    }

    #[test]
    fn a_shallow_listing_returns_only_immediate_children() {
        let dir = tree();
        let listing = list(
            dir.path(),
            &exclusions(&dir, ExclusionConfig::default()),
            false,
        );
        let names: Vec<&str> = listing.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["src", "README.md"]);
    }

    #[test]
    fn a_recursive_listing_walks_the_whole_tree_with_stat_information() {
        let dir = tree();
        let listing = list(
            dir.path(),
            &exclusions(&dir, ExclusionConfig::default()),
            true,
        );
        let paths: Vec<&str> = listing
            .entries
            .iter()
            .map(|e| e.relative_path.as_str())
            .collect();
        assert!(paths.contains(&"src/lib/util.rs"), "{paths:?}");

        let main = listing
            .entries
            .iter()
            .find(|e| e.relative_path == "src/main.rs")
            .expect("src/main.rs must be listed");
        assert!(!main.is_dir);
        assert_eq!(main.size, "fn main() {}\n".len() as u64);
        assert!(main.modified_ms.is_some());
        assert!(!main.readonly);
    }

    #[test]
    fn excluded_directories_are_neither_listed_nor_descended_into() {
        let dir = tree();
        let listing = list(
            dir.path(),
            &exclusions(&dir, ExclusionConfig::default()),
            true,
        );
        assert!(
            listing
                .entries
                .iter()
                .all(|e| !e.relative_path.starts_with("node_modules")
                    && !e.relative_path.starts_with("target")),
            "{:?}",
            listing.entries
        );
    }

    #[test]
    fn the_directory_count_is_what_the_watcher_budget_measures() {
        let dir = tree();
        let listing = list(
            dir.path(),
            &exclusions(&dir, ExclusionConfig::default()),
            true,
        );
        // root, src, src/lib. node_modules and target are excluded.
        assert_eq!(listing.directory_count, 3);
        assert_eq!(listing.file_count, 3);
    }

    #[test]
    fn a_depth_limit_truncates_and_says_so() {
        let dir = tree();
        let listing = list(
            dir.path(),
            &exclusions(&dir, ExclusionConfig::default().with_max_depth(Some(1))),
            true,
        );
        assert!(listing.truncated_by_depth);
        assert!(
            listing
                .entries
                .iter()
                .all(|e| e.relative_path.matches('/').count() <= 1)
        );
    }

    #[test]
    fn a_relative_path_uses_forward_slashes_on_every_platform() {
        let dir = tree();
        let listing = list(
            dir.path(),
            &exclusions(&dir, ExclusionConfig::default()),
            true,
        );
        assert!(
            listing
                .entries
                .iter()
                .all(|e| !e.relative_path.contains('\\'))
        );
    }

    #[test]
    fn directories_sort_before_files() {
        let dir = tree();
        let listing = list(
            dir.path(),
            &exclusions(&dir, ExclusionConfig::default()),
            true,
        );
        let first_file = listing.entries.iter().position(|e| !e.is_dir).unwrap();
        let last_dir = listing.entries.iter().rposition(|e| e.is_dir).unwrap();
        assert!(last_dir < first_file);
    }

    #[test]
    fn an_unreadable_root_is_reported_rather_than_panicking() {
        let dir = TempDir::new("listing-missing");
        let missing = dir.path().join("not-there");
        let listing = list(
            &missing,
            &Exclusions::build(&missing, &ExclusionConfig::default()),
            true,
        );
        assert!(listing.entries.is_empty());
        assert_eq!(listing.unreadable_paths.len(), 1);
    }

    #[test]
    fn stat_reports_a_single_path() {
        let dir = tree();
        let entry = stat(dir.path().join("README.md")).unwrap();
        assert_eq!(entry.name, "README.md");
        assert!(!entry.is_dir);
        assert_eq!(entry.size, "# project\n".len() as u64);
    }

    #[test]
    fn stat_of_a_missing_path_is_an_error_not_an_empty_entry() {
        let dir = TempDir::new("listing-stat");
        assert!(stat(dir.path().join("nope")).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn a_symlink_is_reported_but_not_followed() {
        use std::os::unix::fs::symlink;
        let dir = TempDir::new("listing-symlink");
        dir.write("real/file.txt", "content");
        symlink(dir.path().join("real"), dir.path().join("link")).unwrap();

        let listing = list(
            dir.path(),
            &Exclusions::build(dir.path(), &ExclusionConfig::default()),
            true,
        );
        let link = listing
            .entries
            .iter()
            .find(|e| e.name == "link")
            .expect("the link itself must be listed");
        assert!(link.is_symlink);
        // Not followed: the file appears once, under `real/`, not twice.
        assert_eq!(
            listing
                .entries
                .iter()
                .filter(|e| e.name == "file.txt")
                .count(),
            1
        );
    }
}
