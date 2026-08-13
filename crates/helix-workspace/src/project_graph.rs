//! Monorepo project graph model and queries (Task 1.9, REQ-FS-002).
//!
//! Extraction and process orchestration build immutable [`ProjectGraph`]
//! values. Consumers only query a snapshot, so replacing a graph in the
//! workspace registry cannot expose a half-updated dependency graph.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::identity::comparison_key;

mod cache;
mod detect;
mod extract;
mod service;

pub use cache::{CachedProjectGraph, ProjectGraphCache, SourceFingerprint, fingerprint_sources};
pub use detect::{DetectedTool, ToolDetection, detect_tools, is_graph_source_file};
pub use extract::{ExtractionWarning, ProjectGraphExtraction, extract_project_graph};
pub use service::ProjectGraphService;

/// Monorepo tool or manifest family that contributed projects to a graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum MonorepoTool {
    Nx,
    Turborepo,
    Lerna,
    PnpmWorkspaces,
    NpmWorkspaces,
    YarnWorkspaces,
    Cargo,
    Go,
    Maven,
    Gradle,
    DotNet,
    /// No supported monorepo tool was usable; each workspace root is one
    /// independent project.
    Fallback,
}

/// Whether the current snapshot came from extraction, disk, or degradation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum ProjectGraphStatus {
    Fresh,
    Cached,
    Fallback,
}

/// One buildable/testable project in a workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct Project {
    /// Stable within this graph and used by every dependency edge.
    pub id: String,
    pub name: String,
    /// Absolute, normalized project root.
    pub root: String,
    /// Direct dependencies, by project id.
    pub dependencies: Vec<String>,
    pub tool: MonorepoTool,
}

/// Immutable project graph published in a workspace scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct ProjectGraph {
    pub workspace_key: String,
    pub projects: Vec<Project>,
    pub tools: Vec<MonorepoTool>,
    /// Tool configuration files and lockfiles that invalidate this graph.
    pub source_files: Vec<String>,
    #[ts(type = "number")]
    pub generated_ms: u64,
    pub status: ProjectGraphStatus,
}

impl ProjectGraph {
    /// Reject malformed tool output and corrupt cache entries before either is
    /// published to consumers.
    pub fn validate(&self) -> Result<(), String> {
        if self.workspace_key.trim().is_empty() {
            return Err("the graph has no workspace key".to_string());
        }
        let mut ids = BTreeSet::new();
        for project in &self.projects {
            if project.id.trim().is_empty() {
                return Err("a project has an empty id".to_string());
            }
            if !ids.insert(project.id.as_str()) {
                return Err(format!("duplicate project id '{}'", project.id));
            }
            if project.root.trim().is_empty() {
                return Err(format!("project '{}' has no root", project.id));
            }
        }
        for project in &self.projects {
            for dependency in &project.dependencies {
                if !ids.contains(dependency.as_str()) {
                    return Err(format!(
                        "project '{}' depends on unknown project '{}'",
                        project.id, dependency
                    ));
                }
            }
        }
        Ok(())
    }

    /// The deepest project root containing `path` owns it. This makes a nested
    /// package win over the repository-level project.
    pub fn owner_of(&self, path: &Path) -> Option<&Project> {
        let path = comparison_key(path);
        self.projects
            .iter()
            .filter(|project| path_contains(&comparison_key(Path::new(&project.root)), &path))
            .max_by_key(|project| comparison_key(Path::new(&project.root)).len())
    }

    pub fn project(&self, id: &str) -> Option<&Project> {
        self.projects.iter().find(|project| project.id == id)
    }

    /// Direct dependencies of `project_id`, in stable id order.
    pub fn dependencies_of(&self, project_id: &str) -> Vec<&Project> {
        let Some(project) = self.project(project_id) else {
            return Vec::new();
        };
        let dependencies: BTreeSet<&str> =
            project.dependencies.iter().map(String::as_str).collect();
        self.projects
            .iter()
            .filter(|candidate| dependencies.contains(candidate.id.as_str()))
            .collect()
    }

    /// Projects with a direct edge to `project_id`, in stable id order.
    pub fn dependents_of(&self, project_id: &str) -> Vec<&Project> {
        self.projects
            .iter()
            .filter(|project| {
                project
                    .dependencies
                    .iter()
                    .any(|dependency| dependency == project_id)
            })
            .collect()
    }

    /// Fallback affected computation: changed-file owners plus their complete
    /// transitive reverse-dependency closure.
    pub fn affected<'a>(&'a self, changed_files: &[impl AsRef<Path>]) -> Vec<&'a Project> {
        let reverse = self.reverse_edges();
        let mut affected = BTreeSet::new();
        let mut pending = VecDeque::new();

        for file in changed_files {
            let file_key = comparison_key(file.as_ref());
            if self
                .source_files
                .iter()
                .any(|source| comparison_key(Path::new(source)) == file_key)
            {
                return self.projects.iter().collect();
            }
            if let Some(project) = self.owner_of(file.as_ref())
                && affected.insert(project.id.as_str())
            {
                pending.push_back(project.id.as_str());
            }
        }

        while let Some(project_id) = pending.pop_front() {
            for dependent in reverse.get(project_id).into_iter().flatten() {
                if affected.insert(dependent) {
                    pending.push_back(dependent);
                }
            }
        }

        self.projects
            .iter()
            .filter(|project| affected.contains(project.id.as_str()))
            .collect()
    }

    fn reverse_edges(&self) -> HashMap<&str, Vec<&str>> {
        let mut reverse: HashMap<&str, Vec<&str>> = HashMap::new();
        for project in &self.projects {
            for dependency in &project.dependencies {
                reverse
                    .entry(dependency.as_str())
                    .or_default()
                    .push(project.id.as_str());
            }
        }
        reverse
    }
}

fn path_contains(root: &str, path: &str) -> bool {
    path == root
        || (root == "/" && path.starts_with('/'))
        || path
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph() -> ProjectGraph {
        ProjectGraph {
            workspace_key: "workspace".into(),
            projects: vec![
                Project {
                    id: "app".into(),
                    name: "app".into(),
                    root: "/repo/apps/app".into(),
                    dependencies: vec!["ui".into()],
                    tool: MonorepoTool::Nx,
                },
                Project {
                    id: "repo".into(),
                    name: "repo".into(),
                    root: "/repo".into(),
                    dependencies: Vec::new(),
                    tool: MonorepoTool::Nx,
                },
                Project {
                    id: "ui".into(),
                    name: "ui".into(),
                    root: "/repo/libs/ui".into(),
                    dependencies: vec!["tokens".into()],
                    tool: MonorepoTool::Nx,
                },
                Project {
                    id: "tokens".into(),
                    name: "tokens".into(),
                    root: "/repo/libs/tokens".into(),
                    dependencies: Vec::new(),
                    tool: MonorepoTool::Nx,
                },
            ],
            tools: vec![MonorepoTool::Nx],
            source_files: vec!["/repo/nx.json".into()],
            generated_ms: 1,
            status: ProjectGraphStatus::Fresh,
        }
    }

    #[test]
    fn the_deepest_project_owns_a_path() {
        let graph = graph();
        assert_eq!(
            graph.owner_of(Path::new("/repo/apps/app/src/main.ts")),
            graph.project("app")
        );
        assert_eq!(
            graph.owner_of(Path::new("/repo/README.md")),
            graph.project("repo")
        );
        assert!(
            graph
                .owner_of(Path::new("/repository/not-inside"))
                .is_none()
        );
    }

    #[test]
    fn dependency_queries_preserve_edge_direction() {
        let graph = graph();
        assert_eq!(
            graph
                .dependencies_of("app")
                .iter()
                .map(|project| project.id.as_str())
                .collect::<Vec<_>>(),
            vec!["ui"]
        );
        assert_eq!(
            graph
                .dependents_of("ui")
                .iter()
                .map(|project| project.id.as_str())
                .collect::<Vec<_>>(),
            vec!["app"]
        );
    }

    #[test]
    fn affected_projects_include_transitive_dependents() {
        let graph = graph();
        assert_eq!(
            graph
                .affected(&["/repo/libs/tokens/src/color.ts"])
                .iter()
                .map(|project| project.id.as_str())
                .collect::<Vec<_>>(),
            vec!["app", "ui", "tokens"]
        );
    }
}
