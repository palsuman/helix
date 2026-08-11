//! Exclusion rules for listing and watching (Task 1.7, REQ-FS-004.4, .5).
//!
//! Two independent sources decide whether a path is interesting:
//!
//! - **Configured globs**, from `files.exclude` and `files.watcherExclude`.
//!   These are the user's and the workspace's explicit statement.
//! - **`.gitignore`**, at the root and nested anywhere below it. This is the
//!   project's existing statement about what is generated rather than authored,
//!   and honouring it means a developer does not have to write their exclusions
//!   twice.
//!
//! One matcher serves both the directory lister and the watcher on purpose. If
//! they disagreed, a file could appear in the explorer and never receive change
//! events, or vice versa, which is the kind of inconsistency that is nearly
//! impossible to diagnose from a bug report.
//!
//! ## Why ancestor prefixes are tested
//!
//! A pattern like `**/node_modules` is written by a human to mean "and
//! everything in it". Glob semantics do not agree: the pattern matches the
//! directory and nothing below it. Rather than requiring every user to write
//! both `**/node_modules` and `**/node_modules/**`, each path is tested along
//! with each of its ancestor prefixes, so excluding a directory excludes its
//! subtree. This also makes the walk cheap: an excluded directory is never
//! descended into, so `node_modules` costs one match, not 40,000.
//!
//! ## `.gitignore` precedence
//!
//! Git's rules are precedence-ordered, not set-based: a deeper `.gitignore` can
//! re-include what a shallower one excluded (`!important.log`). Matchers are
//! therefore consulted shallowest-first and the last decisive answer wins,
//! which is git's own resolution order. Configured globs are checked before
//! `.gitignore` and are not overridable by it: an explicit exclusion in
//! settings is a direct instruction, and a project's ignore file should not be
//! able to override the user's own configuration.

use std::path::{Component, Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::gitignore::{Gitignore, GitignoreBuilder};

use crate::atomic::is_temp_path;

/// Excluded unless a workspace says otherwise (REQ-FS-004.5).
///
/// `.git` is here rather than only in `.gitignore` because git does not ignore
/// its own directory, and watching `.git/objects` on a repository under active
/// use is the single most effective way to generate an event storm.
pub const DEFAULT_EXCLUDE_GLOBS: &[&str] = &[
    "**/.git",
    "**/node_modules",
    "**/target",
    "**/dist",
    "**/build",
    "**/out",
    "**/.venv",
    "**/__pycache__",
    "**/.DS_Store",
];

/// How exclusions should be assembled for one root.
#[derive(Debug, Clone)]
pub struct ExclusionConfig {
    /// Glob patterns, from settings. Replaces the defaults rather than adding
    /// to them, so a workspace that needs to see inside `target` can say so.
    pub globs: Vec<String>,
    /// Honour `.gitignore` files (REQ-FS-004.4).
    pub respect_gitignore: bool,
    /// How deep to descend. `None` is unlimited; the watcher and lister both
    /// take it from `files.watchDepth`-style configuration.
    pub max_depth: Option<usize>,
}

impl Default for ExclusionConfig {
    fn default() -> Self {
        Self {
            globs: DEFAULT_EXCLUDE_GLOBS
                .iter()
                .map(|glob| (*glob).to_string())
                .collect(),
            respect_gitignore: true,
            max_depth: None,
        }
    }
}

impl ExclusionConfig {
    /// No exclusions at all. For a caller that genuinely wants everything, and
    /// for tests that would otherwise be at the mercy of the defaults.
    pub fn permissive() -> Self {
        Self {
            globs: Vec::new(),
            respect_gitignore: false,
            max_depth: None,
        }
    }

    pub fn with_globs<I, S>(mut self, globs: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.globs = globs.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_gitignore(mut self, respect: bool) -> Self {
        self.respect_gitignore = respect;
        self
    }

    pub fn with_max_depth(mut self, depth: Option<usize>) -> Self {
        self.max_depth = depth;
        self
    }
}

/// A compiled exclusion matcher for one root.
pub struct Exclusions {
    root: PathBuf,
    globs: GlobSet,
    /// `(directory, matcher)`, shallowest first. See the module docs on
    /// precedence.
    gitignores: Vec<(PathBuf, Gitignore)>,
    respect_gitignore: bool,
    max_depth: Option<usize>,
    /// Patterns that would not compile. Surfaced rather than swallowed: a typo
    /// in an exclusion glob silently doing nothing is how a user ends up
    /// convinced watching is broken.
    invalid_globs: Vec<String>,
}

impl Exclusions {
    /// Compile the rules for `root`, discovering `.gitignore` files under it.
    ///
    /// Discovery is bounded by the rules being built: the scan for nested
    /// ignore files does not descend into a directory the globs already
    /// exclude, so a repository with a `node_modules` full of packages that
    /// each ship a `.gitignore` does not pay for them.
    pub fn build(root: impl AsRef<Path>, config: &ExclusionConfig) -> Self {
        let root = root.as_ref().to_path_buf();
        let mut builder = GlobSetBuilder::new();
        let mut invalid_globs = Vec::new();
        for pattern in &config.globs {
            match Glob::new(pattern) {
                Ok(glob) => {
                    builder.add(glob);
                }
                Err(_) => invalid_globs.push(pattern.clone()),
            }
        }
        let globs = builder.build().unwrap_or_else(|_| GlobSet::empty());

        let mut exclusions = Self {
            root,
            globs,
            gitignores: Vec::new(),
            respect_gitignore: config.respect_gitignore,
            max_depth: config.max_depth,
            invalid_globs,
        };
        if config.respect_gitignore {
            exclusions.gitignores = exclusions.discover_gitignores();
        }
        exclusions
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn max_depth(&self) -> Option<usize> {
        self.max_depth
    }

    pub fn invalid_globs(&self) -> &[String] {
        &self.invalid_globs
    }

    /// Whether a path should be hidden from listing and from watch events.
    pub fn is_excluded(&self, path: &Path, is_dir: bool) -> bool {
        // Our own in-flight writes are never anyone's business: a save must
        // surface as one change to the target file, not as a create and a
        // delete of a temporary the user never made.
        if is_temp_path(path) {
            return true;
        }

        let Some(relative) = self.relative(path) else {
            // Outside the root entirely. Not this matcher's business, and
            // reporting it as excluded would silently drop events for a
            // symlinked file the user really does have open.
            return false;
        };

        if self.glob_excluded(&relative) {
            return true;
        }
        self.respect_gitignore && self.gitignore_excluded(path, is_dir)
    }

    /// Whether the walk should descend into this directory.
    ///
    /// Separate from [`is_excluded`] so the depth limit lives with the walk
    /// rather than with the exclusion question: a directory at the depth limit
    /// is not excluded, its children simply are not visited.
    pub fn should_descend(&self, dir: &Path, depth: usize) -> bool {
        if let Some(max) = self.max_depth
            && depth >= max
        {
            return false;
        }
        !self.is_excluded(dir, true)
    }

    fn relative(&self, path: &Path) -> Option<String> {
        let relative = path.strip_prefix(&self.root).ok()?;
        let mut parts = Vec::new();
        for component in relative.components() {
            if let Component::Normal(part) = component {
                parts.push(part.to_string_lossy().into_owned());
            }
        }
        Some(parts.join("/"))
    }

    /// Test the path and each ancestor prefix. See the module docs.
    fn glob_excluded(&self, relative: &str) -> bool {
        if relative.is_empty() {
            return false;
        }
        let mut prefix = String::new();
        for segment in relative.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(segment);
            if self.globs.is_match(&prefix) {
                return true;
            }
        }
        false
    }

    /// Resolve `.gitignore` shallowest-first, last decisive answer winning.
    fn gitignore_excluded(&self, path: &Path, is_dir: bool) -> bool {
        let mut excluded = false;
        for (dir, matcher) in &self.gitignores {
            if !path.starts_with(dir) {
                continue;
            }
            // `matched_path_or_any_parents` is what makes `build/` exclude
            // `build/lib/x.js`: the file itself matches no pattern, one of its
            // parents does.
            let verdict = matcher.matched_path_or_any_parents(path, is_dir);
            if verdict.is_ignore() {
                excluded = true;
            } else if verdict.is_whitelist() {
                excluded = false;
            }
        }
        excluded
    }

    /// Find `.gitignore` files under the root, skipping glob-excluded subtrees.
    fn discover_gitignores(&self) -> Vec<(PathBuf, Gitignore)> {
        let mut found = Vec::new();
        let mut queue = vec![(self.root.clone(), 0usize)];
        while let Some((dir, depth)) = queue.pop() {
            let candidate = dir.join(".gitignore");
            if candidate.is_file() {
                let mut builder = GitignoreBuilder::new(&dir);
                // A malformed line is skipped by the builder itself; an
                // unreadable file yields an empty matcher rather than aborting
                // the whole root.
                let _ = builder.add(&candidate);
                if let Ok(matcher) = builder.build() {
                    found.push((dir.clone(), matcher));
                }
            }

            if self.max_depth.is_some_and(|max| depth >= max) {
                continue;
            }
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.filter_map(Result::ok) {
                let path = entry.path();
                // `file_type` rather than `is_dir` so a symlinked directory is
                // not followed: a link back up the tree would make discovery
                // non-terminating.
                let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                if !is_dir {
                    continue;
                }
                if self
                    .relative(&path)
                    .is_some_and(|relative| self.glob_excluded(&relative))
                {
                    continue;
                }
                queue.push((path, depth + 1));
            }
        }
        // Shallowest first, so deeper files get the last word.
        found.sort_by_key(|(dir, _)| dir.components().count());
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    fn build(dir: &TempDir, config: ExclusionConfig) -> Exclusions {
        Exclusions::build(dir.path(), &config)
    }

    #[test]
    fn the_defaults_exclude_the_usual_generated_directories() {
        let dir = TempDir::new("exclude-defaults");
        let exclusions = build(&dir, ExclusionConfig::default());
        for name in [".git", "node_modules", "target", "dist"] {
            let path = dir.path().join(name);
            assert!(exclusions.is_excluded(&path, true), "{name}");
        }
        assert!(!exclusions.is_excluded(&dir.path().join("src"), true));
    }

    #[test]
    fn excluding_a_directory_excludes_everything_under_it() {
        let dir = TempDir::new("exclude-subtree");
        let exclusions = build(&dir, ExclusionConfig::default());
        let deep = dir.path().join("node_modules/react/lib/index.js");
        assert!(exclusions.is_excluded(&deep, false));
        assert!(!exclusions.should_descend(&dir.path().join("node_modules"), 0));
    }

    #[test]
    fn configured_globs_replace_the_defaults() {
        let dir = TempDir::new("exclude-configured");
        let exclusions = build(
            &dir,
            ExclusionConfig::default().with_globs(["**/*.log", "coverage"]),
        );
        assert!(exclusions.is_excluded(&dir.path().join("logs/app.log"), false));
        assert!(exclusions.is_excluded(&dir.path().join("coverage"), true));
        assert!(
            !exclusions.is_excluded(&dir.path().join("target"), true),
            "an explicit glob list is the whole list"
        );
    }

    #[test]
    fn a_gitignore_at_the_root_is_respected() {
        // REQ-FS-004.4, and the Task 1.7 gitignore test.
        let dir = TempDir::new("exclude-gitignore");
        dir.write(".gitignore", "*.log\nsecrets/\n");
        dir.write("app.log", "noise");
        dir.write("src/main.rs", "fn main() {}");
        dir.mkdir("secrets");

        let exclusions = build(&dir, ExclusionConfig::default());
        assert!(exclusions.is_excluded(&dir.path().join("app.log"), false));
        assert!(exclusions.is_excluded(&dir.path().join("secrets"), true));
        assert!(exclusions.is_excluded(&dir.path().join("secrets/key.pem"), false));
        assert!(!exclusions.is_excluded(&dir.path().join("src/main.rs"), false));
    }

    #[test]
    fn a_nested_gitignore_can_re_include_what_the_root_ignored() {
        // Git's actual precedence rule. Getting this backwards would hide a
        // file the project deliberately un-ignored.
        let dir = TempDir::new("exclude-nested");
        dir.write(".gitignore", "*.log\n");
        dir.write("logs/.gitignore", "!keep.log\n");
        dir.write("logs/keep.log", "kept");
        dir.write("logs/drop.log", "dropped");

        let exclusions = build(&dir, ExclusionConfig::default());
        assert!(exclusions.is_excluded(&dir.path().join("logs/drop.log"), false));
        assert!(!exclusions.is_excluded(&dir.path().join("logs/keep.log"), false));
    }

    #[test]
    fn gitignore_can_be_turned_off_without_losing_the_globs() {
        let dir = TempDir::new("exclude-no-gitignore");
        dir.write(".gitignore", "*.log\n");
        let exclusions = build(&dir, ExclusionConfig::default().with_gitignore(false));
        assert!(!exclusions.is_excluded(&dir.path().join("app.log"), false));
        assert!(exclusions.is_excluded(&dir.path().join("target"), true));
    }

    #[test]
    fn a_configured_glob_wins_over_a_gitignore_whitelist() {
        let dir = TempDir::new("exclude-precedence");
        dir.write(".gitignore", "!*.log\n");
        let exclusions = build(&dir, ExclusionConfig::default().with_globs(["**/*.log"]));
        assert!(
            exclusions.is_excluded(&dir.path().join("app.log"), false),
            "settings must not be overridable by a file inside the workspace"
        );
    }

    #[test]
    fn our_own_write_temporaries_are_never_reported() {
        let dir = TempDir::new("exclude-temp");
        let exclusions = build(&dir, ExclusionConfig::permissive());
        let temp = dir.path().join(".main.rs.1234-0.helixtmp");
        assert!(exclusions.is_excluded(&temp, false));
    }

    #[test]
    fn a_path_outside_the_root_is_not_this_matchers_business() {
        let dir = TempDir::new("exclude-outside");
        let exclusions = build(&dir, ExclusionConfig::default());
        assert!(!exclusions.is_excluded(Path::new("/elsewhere/node_modules"), true));
    }

    #[test]
    fn the_depth_limit_stops_descent_without_excluding_the_directory() {
        let dir = TempDir::new("exclude-depth");
        let exclusions = build(&dir, ExclusionConfig::default().with_max_depth(Some(1)));
        let level_one = dir.path().join("src");
        assert!(exclusions.should_descend(&level_one, 0));
        assert!(!exclusions.should_descend(&level_one, 1));
        assert!(!exclusions.is_excluded(&level_one, true));
    }

    #[test]
    fn an_unparseable_glob_is_reported_rather_than_silently_ignored() {
        let dir = TempDir::new("exclude-invalid");
        let exclusions = build(&dir, ExclusionConfig::default().with_globs(["["]));
        assert_eq!(exclusions.invalid_globs(), &["[".to_string()]);
    }

    #[test]
    fn the_permissive_config_excludes_nothing_but_temporaries() {
        let dir = TempDir::new("exclude-permissive");
        dir.write(".gitignore", "*.log\n");
        let exclusions = build(&dir, ExclusionConfig::permissive());
        assert!(!exclusions.is_excluded(&dir.path().join("app.log"), false));
        assert!(!exclusions.is_excluded(&dir.path().join("node_modules"), true));
    }
}
