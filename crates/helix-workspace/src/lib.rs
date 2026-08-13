//! `helix-workspace` — the workspace manager (Task 1.8, REQ-FS-001).
//!
//! A workspace is a set of root folders opened together, the document that
//! describes them, and the scope every per-project service belongs to. One
//! window is bound to one workspace; one workspace may be shared by several
//! windows.
//!
//! ```text
//!  .helix/workspace.json ──► roots ──► availability probe ──► hooks bind
//!         │                    │                              (watchers,
//!         │                    │                               servers,
//!         id ──────────────────┴──► workspace key ──► lease   terminals)
//!                                     (refcounted across windows)
//!
//!  defaults + user ─► workspace.json settings ─► <root>/.helix/settings.json
//!                                              ─► <folder>/.helix/settings.json
//! ```
//!
//! Module map:
//!
//! - [`model`] — the multi-root model, the `.helix/workspace.json` document,
//!   its validation, and root availability.
//! - [`identity`] — id generation and the state and cache key Task 1.10 files
//!   session state under.
//! - [`settings`] — per-folder settings layered onto workspace settings, using
//!   the configuration service's own merge primitives.
//! - [`registry`] — workspace-scoped resources, reference-counted by holder.
//! - [`recent`] — the last 20 workspaces, in user data.
//! - [`service`] — the manager itself: open, close, add and remove roots,
//!   availability retry, and the cleanup hooks.
//! - [`commands`] — the `workspace.*` IPC payloads and the streaming channel
//!   name.
//!
//! Like `helix-config` and `helix-fs`, this crate has no Tauri dependency and no
//! dependency on the service container. `helix-kernel` wraps it as a managed
//! service, registers its commands, bridges its events onto the stream, and
//! drives the availability retry, so the tests here exercise the real code path
//! with no process around them.

pub mod commands;
pub mod identity;
pub mod model;
pub mod project_graph;
pub mod recent;
pub mod registry;
pub mod service;
pub mod settings;

pub use commands::{
    AffectedProjectsRequest, AffectedProjectsResponse, AffectedProjectsSource, CHANNEL,
    PROJECT_GRAPH_CHANNEL, ProjectGraphEvent, ProjectGraphEventKind, ProjectGraphRequest,
    ProjectGraphResponse, ProjectOwnerRequest, ProjectOwnerResponse, ProjectRelationsRequest,
    ProjectRelationsResponse, RefreshProjectGraphRequest, RefreshProjectGraphResponse,
    WorkspaceCloseRequest, WorkspaceCloseResponse, WorkspaceForgetRecentRequest,
    WorkspaceForgetRecentResponse, WorkspaceListRequest, WorkspaceListResponse,
    WorkspaceOpenRequest, WorkspaceRecentRequest, WorkspaceRecentResponse, WorkspaceResponse,
    WorkspaceRootRequest, WorkspaceSchemaRequest, WorkspaceSchemaResponse,
    WorkspaceSettingsRequest, WorkspaceSettingsResponse,
};
pub use identity::{
    generate_id, key_from_roots, path_contains, relative_path, same_path,
    workspace_cache_directory, workspace_key,
};
pub use model::{
    FolderEntry, MAX_ROOTS, RootAvailability, WORKSPACE_FILE_NAME, WorkspaceFile, WorkspaceIssue,
    WorkspaceIssueKind, WorkspaceRoot, workspace_file_path_in, workspace_json_schema,
};
pub use project_graph::{
    CachedProjectGraph, DetectedTool, ExtractionWarning, MonorepoTool, Project, ProjectGraph,
    ProjectGraphCache, ProjectGraphExtraction, ProjectGraphService, ProjectGraphStatus,
    SourceFingerprint, ToolDetection, detect_tools, extract_project_graph, fingerprint_sources,
    is_graph_source_file,
};
pub use recent::{MAX_RECENT, RecentWorkspace, RecentWorkspaces, recent_path};
pub use registry::{TeardownHook, WorkspaceLease, WorkspaceRegistry};
pub use service::{
    DEFAULT_RETRY_INTERVAL, LOG_SOURCE, MAX_ROOTS_SETTING, RootEvent, WorkspaceEvent,
    WorkspaceEventKind, WorkspaceHooks, WorkspaceListener, WorkspaceMetrics, WorkspaceService,
    WorkspaceSnapshot,
};
pub use settings::WorkspaceSettings;
