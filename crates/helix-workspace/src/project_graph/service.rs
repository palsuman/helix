use std::path::{Path, PathBuf};
use std::sync::Arc;

use helix_core::error::AppError;

use crate::identity::canonical_path;
use crate::{WorkspaceRegistry, WorkspaceSnapshot};

use super::{
    MonorepoTool, Project, ProjectGraph, ProjectGraphCache, ProjectGraphStatus, SourceFingerprint,
    detect_tools,
};

/// Query and publication surface shared by every window on a workspace.
pub struct ProjectGraphService {
    registry: Arc<WorkspaceRegistry>,
    cache: ProjectGraphCache,
}

impl ProjectGraphService {
    pub fn new(registry: Arc<WorkspaceRegistry>) -> Arc<Self> {
        Arc::new(Self {
            registry,
            cache: ProjectGraphCache::new(),
        })
    }

    pub fn with_cache(registry: Arc<WorkspaceRegistry>, cache: ProjectGraphCache) -> Arc<Self> {
        Arc::new(Self { registry, cache })
    }

    pub fn publish(&self, graph: ProjectGraph) -> Result<Arc<ProjectGraph>, AppError> {
        graph.validate().map_err(|message| {
            AppError::permanent(
                "PROJECT_GRAPH_INVALID",
                format!("project graph cannot be published: {message}"),
            )
        })?;
        let key = graph.workspace_key.clone();
        let graph = Arc::new(graph);
        if !self.registry.publish_if_active(&key, graph.clone()) {
            return Err(AppError::cancelled(format!(
                "workspace '{key}' closed before its project graph was published"
            )));
        }
        Ok(graph)
    }

    pub fn current(&self, workspace_key: &str) -> Option<Arc<ProjectGraph>> {
        self.registry.resolve(workspace_key)
    }

    pub fn require(&self, workspace_key: &str) -> Result<Arc<ProjectGraph>, AppError> {
        self.current(workspace_key).ok_or_else(|| {
            AppError::transient(
                "PROJECT_GRAPH_PENDING",
                format!("project graph for workspace '{workspace_key}' is still loading"),
            )
        })
    }

    /// Load cache when available, otherwise build a cheap per-root graph. The
    /// caller performs the generation check before publishing the result.
    pub fn cached_or_fallback(
        &self,
        workspace: &WorkspaceSnapshot,
    ) -> Result<ProjectGraph, AppError> {
        let available_roots: Vec<PathBuf> = workspace
            .available_roots()
            .into_iter()
            .map(|root| root.as_path().to_path_buf())
            .collect();
        let detection = detect_tools(&available_roots);
        if let Some(cached) = self.cache.load(&workspace.key, &detection.source_files)? {
            return Ok(cached.graph);
        }
        Ok(fallback_graph(workspace))
    }

    pub fn fallback(&self, workspace: &WorkspaceSnapshot) -> ProjectGraph {
        fallback_graph(workspace)
    }

    pub fn cache(
        &self,
        graph: &ProjectGraph,
        fingerprints: &[SourceFingerprint],
    ) -> Result<(), AppError> {
        self.cache.store(graph, fingerprints)
    }

    pub fn owner_of(&self, workspace_key: &str, path: &Path) -> Result<Option<Project>, AppError> {
        Ok(self.require(workspace_key)?.owner_of(path).cloned())
    }

    pub fn dependencies_of(
        &self,
        workspace_key: &str,
        project_id: &str,
    ) -> Result<Vec<Project>, AppError> {
        Ok(self
            .require(workspace_key)?
            .dependencies_of(project_id)
            .into_iter()
            .cloned()
            .collect())
    }

    pub fn dependents_of(
        &self,
        workspace_key: &str,
        project_id: &str,
    ) -> Result<Vec<Project>, AppError> {
        Ok(self
            .require(workspace_key)?
            .dependents_of(project_id)
            .into_iter()
            .cloned()
            .collect())
    }

    pub fn affected(
        &self,
        workspace_key: &str,
        changed_files: &[PathBuf],
    ) -> Result<Vec<Project>, AppError> {
        Ok(self
            .require(workspace_key)?
            .affected(changed_files)
            .into_iter()
            .cloned()
            .collect())
    }
}

fn fallback_graph(workspace: &WorkspaceSnapshot) -> ProjectGraph {
    let mut projects: Vec<Project> = workspace
        .roots
        .iter()
        .map(|root| {
            let path = canonical_path(root.as_path());
            let name = if root.name.trim().is_empty() {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("workspace")
                    .to_string()
            } else {
                root.name.clone()
            };
            Project {
                id: name.clone(),
                name,
                root: path.to_string_lossy().to_string(),
                dependencies: Vec::new(),
                tool: MonorepoTool::Fallback,
            }
        })
        .collect();
    projects.sort_by(|left, right| left.root.cmp(&right.root));
    for index in 0..projects.len() {
        if projects
            .iter()
            .enumerate()
            .any(|(other, project)| other != index && project.id == projects[index].id)
        {
            projects[index].id = format!("{}@{}", projects[index].id, index + 1);
        }
    }
    ProjectGraph {
        workspace_key: workspace.key.clone(),
        projects,
        tools: vec![MonorepoTool::Fallback],
        source_files: Vec::new(),
        generated_ms: workspace.opened_ms,
        status: ProjectGraphStatus::Fallback,
    }
}
