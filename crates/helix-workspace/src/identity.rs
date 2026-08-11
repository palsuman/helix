//! Workspace identity: the `id` in the document and the key state and caches
//! are filed under (REQ-FS-001.2, REQ-NFR-002.11).
//!
//! ```text
//!  .helix/workspace.json has an id?  ──yes──►  key = id
//!                 │
//!                 no
//!                 ▼
//!  key = hash(sorted, canonicalized root paths)
//! ```
//!
//! Two properties matter, and both are the reason the fallback is a hash over a
//! *set* rather than over the first root:
//!
//! - **One home per workspace.** A three-root workspace has one key, so its
//!   session state is one directory, not three that each know a third of the
//!   story.
//! - **Reordering and symlinks do not move it.** The roots are canonicalized
//!   and sorted before hashing, so opening the same folders in a different
//!   order, or reaching one through a symlink, resolves to the same key. Task
//!   1.10 asserts exactly this, and it is the difference between recovery
//!   finding unsaved work and silently starting fresh.
//!
//! The id itself is generated once, on the first write of the document, and
//! never derived from paths — a workspace that is moved or renamed keeps its
//! history precisely because its id does not depend on where it lives.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use xxhash_rust::xxh3::xxh3_128;

/// Length of a generated id and of a derived key, in hex characters.
const KEY_HEX_LEN: usize = 32;

/// Generate a fresh workspace id.
///
/// Not a UUID, and deliberately not a dependency: a 128-bit hash over the
/// clock, the process id, and a per-process counter is unique enough for a
/// value whose only job is to name a directory, and it keeps the id format
/// identical to the derived key so Task 1.10 has one shape to handle.
pub fn generate_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let seed = format!(
        "{nanos}:{}:{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    format!("{:032x}", xxh3_128(seed.as_bytes()))
}

/// Whether a string is shaped like an id this module produces.
pub fn is_generated_id(candidate: &str) -> bool {
    candidate.len() == KEY_HEX_LEN && candidate.chars().all(|c| c.is_ascii_hexdigit())
}

/// Canonicalize one path as far as the file system allows.
///
/// An unavailable root cannot be canonicalized — the drive is not there — so
/// this falls back to a textual normalization rather than failing. Without the
/// fallback, a workspace with an unmounted root would compute a different key
/// than the same workspace with the drive attached, and its state would move
/// out from under it exactly when a user least wants surprises.
pub fn canonical_path(path: &Path) -> PathBuf {
    if let Ok(resolved) = std::fs::canonicalize(path) {
        return strip_verbatim(&resolved);
    }

    let mut ancestor = path;
    let mut missing = Vec::new();
    while let Some(name) = ancestor.file_name() {
        missing.push(name.to_os_string());
        let Some(parent) = ancestor.parent() else {
            break;
        };
        ancestor = parent;
        if let Ok(mut resolved) = std::fs::canonicalize(ancestor) {
            for component in missing.iter().rev() {
                resolved.push(component);
            }
            return strip_verbatim(&resolved);
        }
    }

    strip_verbatim(&crate::model::normalize(path))
}

/// Windows canonicalization returns the `\\?\` verbatim form, which is correct
/// but leaks into anything that compares or displays a path. Removing it keeps
/// a key derived on Windows readable and stable across the two spellings.
pub fn strip_verbatim(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(stripped) => PathBuf::from(stripped),
        None => path.to_path_buf(),
    }
}

/// The canonicalized, de-duplicated, sorted root set a key is derived from.
pub fn canonical_root_set(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut set: Vec<PathBuf> = roots.iter().map(|root| canonical_path(root)).collect();
    set.sort_by_key(|path| comparison_key(path));
    set.dedup_by_key(|path| comparison_key(path));
    set
}

/// The form two paths are compared as. Case-insensitive on Windows and macOS,
/// where the file system is, so `C:\Work\Api` and `c:\work\api` are one root
/// rather than two that would each get their own session state.
///
/// The Windows verbatim prefix is stripped here as well as in
/// [`canonical_path`], because a path can arrive already canonicalized — from a
/// watcher event, from a caller that canonicalized it first — and
/// `\\?\C:\work\api` and `C:\work\api` naming different roots would be a bug
/// that only shows up on one platform.
pub fn comparison_key(path: &Path) -> String {
    let text = strip_verbatim(path).to_string_lossy().replace('\\', "/");
    let trimmed = text.trim_end_matches('/');
    let text = if trimmed.is_empty() { &text } else { trimmed };
    if cfg!(any(windows, target_os = "macos")) {
        text.to_lowercase()
    } else {
        text.to_string()
    }
}

/// Whether two paths name the same location, by the platform's rules.
pub fn same_path(left: &Path, right: &Path) -> bool {
    comparison_key(left) == comparison_key(right)
}

/// Key derived from a root set, used when the document has no `id`.
pub fn key_from_roots(roots: &[PathBuf]) -> String {
    let joined = canonical_root_set(roots)
        .iter()
        .map(|path| comparison_key(path))
        .collect::<Vec<_>>()
        .join("\u{0}");
    format!("{:032x}", xxh3_128(joined.as_bytes()))
}

/// The state and cache key for a workspace: its `id` when the document has
/// one, the derived root hash otherwise.
pub fn workspace_key(id: Option<&str>, roots: &[PathBuf]) -> String {
    match id {
        Some(id) if !id.trim().is_empty() => id.trim().to_string(),
        _ => key_from_roots(roots),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_fs::testutil::TempDir;

    #[test]
    fn a_generated_id_is_unique_and_well_shaped() {
        let a = generate_id();
        let b = generate_id();
        assert_ne!(a, b);
        assert!(is_generated_id(&a), "{a}");
        assert_eq!(a.len(), KEY_HEX_LEN);
        assert!(!is_generated_id("not-a-key"));
    }

    #[test]
    fn the_id_wins_over_the_derived_key() {
        let roots = vec![PathBuf::from("/work/api")];
        assert_eq!(workspace_key(Some("chosen"), &roots), "chosen");
        assert_eq!(
            workspace_key(None, &roots),
            key_from_roots(&roots),
            "no id falls back to the root hash"
        );
        assert_eq!(
            workspace_key(Some("   "), &roots),
            key_from_roots(&roots),
            "a blank id is not an id"
        );
    }

    #[test]
    fn reordering_the_roots_does_not_move_the_key() {
        let dir = TempDir::new("workspace-key-order");
        let api = dir.mkdir("api");
        let web = dir.mkdir("web");

        let one = key_from_roots(&[api.clone(), web.clone()]);
        let other = key_from_roots(&[web, api]);
        assert_eq!(one, other);
    }

    #[test]
    fn a_multi_root_workspace_has_exactly_one_key() {
        let dir = TempDir::new("workspace-key-multi");
        let api = dir.mkdir("api");
        let web = dir.mkdir("web");

        let together = key_from_roots(&[api.clone(), web.clone()]);
        assert_ne!(together, key_from_roots(&[api]));
        assert_ne!(together, key_from_roots(&[web]));
    }

    #[test]
    fn a_duplicated_root_does_not_change_the_key() {
        let dir = TempDir::new("workspace-key-dupe");
        let api = dir.mkdir("api");
        assert_eq!(
            key_from_roots(std::slice::from_ref(&api)),
            key_from_roots(&[api.clone(), api])
        );
    }

    #[test]
    fn a_relative_and_an_absolute_spelling_of_one_root_agree() {
        let dir = TempDir::new("workspace-key-spelling");
        let api = dir.mkdir("api");
        let indirect = api.join("..").join("api");
        assert_eq!(key_from_roots(&[api]), key_from_roots(&[indirect]));
    }

    #[test]
    fn the_windows_verbatim_spelling_of_a_path_is_the_same_path() {
        let plain = PathBuf::from(r"C:\work\api");
        let verbatim = PathBuf::from(r"\\?\C:\work\api");
        assert_eq!(comparison_key(&plain), comparison_key(&verbatim));
        assert!(same_path(&plain, &verbatim));
        assert_eq!(
            key_from_roots(&[plain]),
            key_from_roots(&[verbatim]),
            "and it must not get its own session state"
        );
    }

    #[test]
    fn canonicalization_survives_a_path_that_is_not_there() {
        let missing = PathBuf::from("/definitely/not/mounted/./project");
        assert_eq!(
            canonical_path(&missing),
            PathBuf::from("/definitely/not/mounted/project"),
            "an unavailable root still has to produce a stable key"
        );
    }
}
