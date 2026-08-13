use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use helix_core::error::AppError;
use helix_fs::{hash_file, write_atomic_str};
use serde::{Deserialize, Serialize};

use crate::identity::{comparison_key, workspace_cache_directory};

use super::{ProjectGraph, ProjectGraphStatus};

const CACHE_VERSION: u32 = 1;
const CACHE_FILE_NAME: &str = "project-graph.json";

/// Content identity of one graph configuration file or lockfile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFingerprint {
    pub path: String,
    /// `None` records a source that was deleted after the graph was built.
    pub hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    version: u32,
    fingerprints: Vec<SourceFingerprint>,
    graph: ProjectGraph,
}

/// A usable cached graph and whether every known source still matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedProjectGraph {
    pub graph: ProjectGraph,
    pub fresh: bool,
}

/// Atomic, per-workspace graph cache.
#[derive(Debug, Clone, Default)]
pub struct ProjectGraphCache {
    /// Test override for the per-workspace parent directory.
    base: Option<PathBuf>,
}

impl ProjectGraphCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Store every workspace below `base`, used by tests and embedders that
    /// provide their own OS-directory policy.
    pub fn at(base: impl Into<PathBuf>) -> Self {
        Self {
            base: Some(base.into()),
        }
    }

    pub fn path(&self, workspace_key: &str) -> Option<PathBuf> {
        self.base
            .as_ref()
            .map(|base| base.join(safe_component(workspace_key)))
            .or_else(|| workspace_cache_directory(workspace_key))
            .map(|directory| directory.join(CACHE_FILE_NAME))
    }

    /// Load the last good graph. `current_sources` are the just-detected
    /// top-level tool files; a newly added config must make an older cache
    /// stale even though it could not be in that cache's fingerprint list.
    pub fn load(
        &self,
        workspace_key: &str,
        current_sources: &[PathBuf],
    ) -> Result<Option<CachedProjectGraph>, AppError> {
        let Some(path) = self.path(workspace_key) else {
            return Ok(None);
        };
        let body = match fs::read_to_string(&path) {
            Ok(body) => body,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(cache_error("PROJECT_GRAPH_CACHE_READ", &path, error)),
        };
        let mut entry: CacheEntry = serde_json::from_str(&body).map_err(|error| {
            AppError::transient(
                "PROJECT_GRAPH_CACHE_INVALID",
                format!(
                    "could not parse project graph cache {}: {error}",
                    path.display()
                ),
            )
        })?;
        if entry.version != CACHE_VERSION || entry.graph.workspace_key != workspace_key {
            return Ok(None);
        }
        entry.graph.validate().map_err(|message| {
            AppError::transient(
                "PROJECT_GRAPH_CACHE_INVALID",
                format!(
                    "project graph cache {} is invalid: {message}",
                    path.display()
                ),
            )
        })?;

        let known: BTreeSet<String> = entry
            .fingerprints
            .iter()
            .map(|source| comparison_key(Path::new(&source.path)))
            .collect();
        let no_new_sources = current_sources
            .iter()
            .all(|source| known.contains(&comparison_key(source)));
        let fresh = no_new_sources
            && entry
                .fingerprints
                .iter()
                .all(|source| source.hash == fingerprint_one(Path::new(&source.path)).hash);
        entry.graph.status = ProjectGraphStatus::Cached;
        Ok(Some(CachedProjectGraph {
            graph: entry.graph,
            fresh,
        }))
    }

    pub fn store(
        &self,
        graph: &ProjectGraph,
        fingerprints: &[SourceFingerprint],
    ) -> Result<(), AppError> {
        graph.validate().map_err(|message| {
            AppError::permanent(
                "PROJECT_GRAPH_INVALID",
                format!("refusing to cache an invalid project graph: {message}"),
            )
        })?;
        let Some(path) = self.path(&graph.workspace_key) else {
            return Ok(());
        };
        let entry = CacheEntry {
            version: CACHE_VERSION,
            fingerprints: fingerprints.to_vec(),
            graph: graph.clone(),
        };
        let body = serde_json::to_string_pretty(&entry).map_err(|error| {
            AppError::transient(
                "PROJECT_GRAPH_CACHE_SERIALIZE",
                format!("could not serialize the project graph cache: {error}"),
            )
        })?;
        write_atomic_str(&path, &body)
            .map_err(|error| cache_error("PROJECT_GRAPH_CACHE_WRITE", &path, error))
    }
}

pub fn fingerprint_sources(paths: &[PathBuf]) -> Vec<SourceFingerprint> {
    let mut fingerprints: Vec<SourceFingerprint> =
        paths.iter().map(|path| fingerprint_one(path)).collect();
    fingerprints.sort_by(|left, right| left.path.cmp(&right.path));
    fingerprints.dedup_by(|left, right| left.path == right.path);
    fingerprints
}

fn fingerprint_one(path: &Path) -> SourceFingerprint {
    SourceFingerprint {
        path: path.to_string_lossy().to_string(),
        hash: hash_file(path).ok().map(|hash| hash.to_string()),
    }
}

fn safe_component(key: &str) -> String {
    let value: String = key
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if value.is_empty() {
        "workspace".to_string()
    } else {
        value
    }
}

fn cache_error(code: &str, path: &Path, error: std::io::Error) -> AppError {
    AppError::transient(
        code,
        format!("project graph cache {} failed: {error}", path.display()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MonorepoTool, Project};
    use helix_fs::testutil::TempDir;

    fn graph(source: &Path) -> ProjectGraph {
        ProjectGraph {
            workspace_key: "workspace".into(),
            projects: vec![Project {
                id: "app".into(),
                name: "app".into(),
                root: source
                    .parent()
                    .unwrap()
                    .join("app")
                    .to_string_lossy()
                    .to_string(),
                dependencies: Vec::new(),
                tool: MonorepoTool::Nx,
            }],
            tools: vec![MonorepoTool::Nx],
            source_files: vec![source.to_string_lossy().to_string()],
            generated_ms: 1,
            status: ProjectGraphStatus::Fresh,
        }
    }

    #[test]
    fn cache_round_trip_marks_the_graph_cached_and_fresh() {
        let dir = TempDir::new("project-graph-cache");
        let source = dir.write("repo/nx.json", "{}");
        let cache = ProjectGraphCache::at(dir.path().join("cache"));
        cache
            .store(
                &graph(&source),
                &fingerprint_sources(std::slice::from_ref(&source)),
            )
            .unwrap();

        let loaded = cache
            .load("workspace", std::slice::from_ref(&source))
            .unwrap()
            .unwrap();
        assert!(loaded.fresh);
        assert_eq!(loaded.graph.status, ProjectGraphStatus::Cached);
        assert_eq!(loaded.graph.projects[0].id, "app");
    }

    #[test]
    fn changed_deleted_and_new_sources_invalidate_the_cache() {
        let dir = TempDir::new("project-graph-cache-invalid");
        let source = dir.write("repo/nx.json", "{}");
        let cache = ProjectGraphCache::at(dir.path().join("cache"));
        cache
            .store(
                &graph(&source),
                &fingerprint_sources(std::slice::from_ref(&source)),
            )
            .unwrap();

        fs::write(&source, "{\"changed\":true}").unwrap();
        assert!(
            !cache
                .load("workspace", std::slice::from_ref(&source))
                .unwrap()
                .unwrap()
                .fresh
        );

        fs::remove_file(&source).unwrap();
        assert!(!cache.load("workspace", &[]).unwrap().unwrap().fresh);

        let source = dir.write("repo/nx.json", "{}");
        cache
            .store(
                &graph(&source),
                &fingerprint_sources(std::slice::from_ref(&source)),
            )
            .unwrap();
        let new_source = dir.write("repo/turbo.json", "{}");
        assert!(
            !cache
                .load("workspace", &[source, new_source])
                .unwrap()
                .unwrap()
                .fresh
        );
    }
}
