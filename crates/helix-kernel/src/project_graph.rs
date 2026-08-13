//! Kernel orchestration for the monorepo project graph (Task 1.9).
//!
//! The workspace crate owns detection, parsing, caching, and graph semantics.
//! This layer owns only scheduling, the hard timeout, stream/IPC wiring, and
//! optional delegation to a tool process.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use helix_core::container::{
    HealthCheck, Lifetime, ManagedService, Service, ServiceContainer, ServiceContext, ServiceError,
    ServiceProbe,
};
use helix_core::error::AppError;
use helix_core::health::{ServiceHealth, ServiceMetrics};
use helix_fs::FileSystemService;
use helix_ipc::{CommandContext, IpcDispatcher};
use helix_log::{Logger, log_info, log_warn};
use helix_stream::StreamHub;
use helix_workspace::commands::{
    AFFECTED_PROJECTS, AffectedProjectsRequest, AffectedProjectsResponse, AffectedProjectsSource,
    PROJECT_GRAPH, PROJECT_GRAPH_CHANNEL, PROJECT_OWNER, PROJECT_RELATIONS, ProjectGraphEvent,
    ProjectGraphEventKind, ProjectGraphRequest, ProjectGraphResponse, ProjectOwnerRequest,
    ProjectOwnerResponse, ProjectRelationsRequest, ProjectRelationsResponse, REFRESH_PROJECT_GRAPH,
    RefreshProjectGraphRequest, RefreshProjectGraphResponse,
};
use helix_workspace::{
    MonorepoTool, ProjectGraph, ProjectGraphExtraction, ProjectGraphService, ProjectGraphStatus,
    WorkspaceEvent, WorkspaceEventKind, WorkspaceListener, WorkspaceService, WorkspaceSnapshot,
    detect_tools, extract_project_graph, fingerprint_sources, is_graph_source_file, path_contains,
    relative_path,
};
use serde_json::Value;
use tokio::process::Command;
use tokio::sync::Notify;
use tokio::task::JoinSet;

pub const SERVICE_NAME: &str = "project_graph";
pub const LOG_SOURCE: &str = "kernel.project_graph";
pub const EXTRACTION_TIMEOUT: Duration = Duration::from_secs(10);

/// Synchronous extraction boundary, injectable so timeout and malformed-output
/// behavior can be tested without invoking developer tools.
pub trait GraphExtractor: Send + Sync {
    fn extract(&self, workspace_key: &str, roots: &[PathBuf]) -> ProjectGraphExtraction;
}

struct ManifestGraphExtractor;

impl GraphExtractor for ManifestGraphExtractor {
    fn extract(&self, workspace_key: &str, roots: &[PathBuf]) -> ProjectGraphExtraction {
        extract_project_graph(workspace_key, roots)
    }
}

#[derive(Clone)]
pub struct ProjectGraphScheduler {
    generations: Arc<Mutex<HashMap<String, u64>>>,
    pending: Arc<Mutex<HashMap<String, ScheduledExtraction>>>,
    notify: Arc<Notify>,
}

impl ProjectGraphScheduler {
    pub fn schedule(&self, workspace: WorkspaceSnapshot, reason: &'static str) -> bool {
        let generation = {
            let mut generations = self.generations.lock().unwrap();
            let generation = generations.entry(workspace.key.clone()).or_default();
            *generation = generation.saturating_add(1);
            *generation
        };
        self.pending.lock().unwrap().insert(
            workspace.key.clone(),
            ScheduledExtraction {
                workspace,
                generation,
                reason,
            },
        );
        self.notify.notify_one();
        true
    }

    fn is_current(&self, key: &str, generation: u64) -> bool {
        self.generations
            .lock()
            .unwrap()
            .get(key)
            .is_some_and(|current| *current == generation)
    }

    fn publish_if_current(
        &self,
        graph_service: &ProjectGraphService,
        graph: ProjectGraph,
        generation: u64,
    ) -> Option<Result<Arc<ProjectGraph>, AppError>> {
        let generations = self.generations.lock().unwrap();
        if !generations
            .get(&graph.workspace_key)
            .is_some_and(|current| *current == generation)
        {
            return None;
        }
        Some(graph_service.publish(graph))
    }

    fn invalidate(&self, key: &str) {
        let mut generations = self.generations.lock().unwrap();
        let generation = generations.entry(key.to_string()).or_default();
        *generation = generation.saturating_add(1);
        self.pending.lock().unwrap().remove(key);
    }

    fn take_ready(&self, active: &HashSet<String>) -> Vec<ScheduledExtraction> {
        let mut pending = self.pending.lock().unwrap();
        let ready: Vec<String> = pending
            .keys()
            .filter(|key| !active.contains(*key))
            .cloned()
            .collect();
        ready
            .into_iter()
            .filter_map(|key| pending.remove(&key))
            .collect()
    }
}

#[derive(Clone)]
pub struct ProjectGraphRuntime {
    pub scheduler: ProjectGraphScheduler,
}

struct ScheduledExtraction {
    workspace: WorkspaceSnapshot,
    generation: u64,
    reason: &'static str,
}

pub fn build_service(
    workspace: &Arc<WorkspaceService>,
) -> (Arc<ProjectGraphService>, ProjectGraphRuntime) {
    let graph = ProjectGraphService::new(workspace.registry().clone());
    let runtime = ProjectGraphRuntime {
        scheduler: ProjectGraphScheduler {
            generations: Arc::new(Mutex::new(HashMap::new())),
            pending: Arc::new(Mutex::new(HashMap::new())),
            notify: Arc::new(Notify::new()),
        },
    };
    (graph, runtime)
}

pub fn register_commands(
    dispatcher: &mut IpcDispatcher,
    graph: Arc<ProjectGraphService>,
    workspace: Arc<WorkspaceService>,
    scheduler: ProjectGraphScheduler,
) {
    let get_graph = graph.clone();
    dispatcher.register(PROJECT_GRAPH, move |request: ProjectGraphRequest, _ctx| {
        let graph = get_graph.clone();
        async move {
            Ok::<ProjectGraphResponse, AppError>(ProjectGraphResponse {
                graph: graph.require(&request.key)?.as_ref().clone(),
            })
        }
    });

    let owner_graph = graph.clone();
    dispatcher.register(PROJECT_OWNER, move |request: ProjectOwnerRequest, _ctx| {
        let graph = owner_graph.clone();
        async move {
            Ok::<ProjectOwnerResponse, AppError>(ProjectOwnerResponse {
                project: graph.owner_of(&request.key, Path::new(&request.path))?,
            })
        }
    });

    let relations_graph = graph.clone();
    dispatcher.register(
        PROJECT_RELATIONS,
        move |request: ProjectRelationsRequest, _ctx| {
            let graph = relations_graph.clone();
            async move {
                Ok::<ProjectRelationsResponse, AppError>(ProjectRelationsResponse {
                    dependencies: graph.dependencies_of(&request.key, &request.project_id)?,
                    dependents: graph.dependents_of(&request.key, &request.project_id)?,
                })
            }
        },
    );

    let affected_graph = graph.clone();
    let affected_workspace = workspace.clone();
    dispatcher.register(
        AFFECTED_PROJECTS,
        move |request: AffectedProjectsRequest, ctx| {
            let graph = affected_graph.clone();
            let workspace = affected_workspace.clone();
            async move { affected_projects(graph, workspace, request, ctx).await }
        },
    );

    dispatcher.register(
        REFRESH_PROJECT_GRAPH,
        move |request: RefreshProjectGraphRequest, _ctx| {
            let workspace = workspace.clone();
            let scheduler = scheduler.clone();
            async move {
                let snapshot = workspace.snapshot(&request.key).ok_or_else(|| {
                    AppError::permanent(
                        "WORKSPACE_NOT_OPEN",
                        format!("workspace '{}' is not open", request.key),
                    )
                })?;
                Ok::<RefreshProjectGraphResponse, AppError>(RefreshProjectGraphResponse {
                    accepted: scheduler.schedule(snapshot, "explicit refresh"),
                })
            }
        },
    );
}

async fn affected_projects(
    graph_service: Arc<ProjectGraphService>,
    workspace: Arc<WorkspaceService>,
    request: AffectedProjectsRequest,
    ctx: CommandContext,
) -> Result<AffectedProjectsResponse, AppError> {
    let graph = graph_service.require(&request.key)?;
    let mut fallback_files: Vec<PathBuf> =
        request.changed_files.iter().map(PathBuf::from).collect();
    let mut affected = BTreeSet::new();
    let mut source = AffectedProjectsSource::Graph;

    if let Some(snapshot) = workspace.snapshot(&request.key) {
        let roots: Vec<PathBuf> = snapshot
            .available_roots()
            .into_iter()
            .map(|root| root.as_path().to_path_buf())
            .collect();
        let detection = tokio::task::spawn_blocking(move || detect_tools(&roots))
            .await
            .map_err(|error| {
                AppError::transient(
                    "PROJECT_GRAPH_DETECTION_FAILED",
                    format!("tool detection task failed: {error}"),
                )
            })?;
        for detected in detection
            .tools
            .iter()
            .filter(|detected| matches!(detected.tool, MonorepoTool::Nx | MonorepoTool::Turborepo))
        {
            let files: Vec<PathBuf> = fallback_files
                .iter()
                .filter_map(|path| relative_path(&detected.root, path))
                .collect();
            if files.is_empty() {
                continue;
            }
            let names = match detected.tool {
                MonorepoTool::Nx => native_nx_affected(&detected.root, &files, &ctx).await,
                MonorepoTool::Turborepo => {
                    native_turbo_affected(&detected.root, &files, &graph, &ctx).await
                }
                _ => None,
            };
            if let Some(names) = names {
                source = AffectedProjectsSource::Tool;
                apply_native_affected(
                    &graph,
                    &detected.root,
                    &names,
                    &mut fallback_files,
                    &mut affected,
                );
            }
        }
    }

    affected.extend(
        graph
            .affected(&fallback_files)
            .into_iter()
            .map(|project| project.id.clone()),
    );

    let projects = graph
        .projects
        .iter()
        .filter(|project| affected.contains(&project.id))
        .cloned()
        .collect();
    Ok(AffectedProjectsResponse { projects, source })
}

fn apply_native_affected(
    graph: &ProjectGraph,
    root: &Path,
    names: &HashSet<String>,
    fallback_files: &mut Vec<PathBuf>,
    affected: &mut BTreeSet<String>,
) {
    fallback_files.retain(|path| !path_contains(root, path));
    affected.extend(
        graph
            .projects
            .iter()
            .filter(|project| {
                path_contains(root, Path::new(&project.root))
                    && (names.contains(&project.name) || names.contains(&project.id))
            })
            .map(|project| project.id.clone()),
    );
}

async fn native_nx_affected(
    root: &Path,
    changed_files: &[PathBuf],
    ctx: &CommandContext,
) -> Option<HashSet<String>> {
    let executable = local_tool(root, "nx");
    let files = changed_files
        .iter()
        .map(|path| path.to_string_lossy())
        .collect::<Vec<_>>()
        .join(",");
    let mut command = Command::new(executable);
    command
        .current_dir(root)
        .args([
            "show",
            "projects",
            "--affected",
            "--files",
            files.as_str(),
            "--json",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let timeout = ctx.timeout().min(EXTRACTION_TIMEOUT);
    let output = tokio::select! {
        _ = ctx.cancelled() => return None,
        result = tokio::time::timeout(timeout, command.output()) => result.ok()?.ok()?,
    };
    if !output.status.success() {
        return None;
    }
    let value: Value = serde_json::from_slice(&output.stdout).ok()?;
    let projects = value.as_array()?;
    Some(
        projects
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
    )
}

async fn native_turbo_affected(
    root: &Path,
    changed_files: &[PathBuf],
    graph: &ProjectGraph,
    ctx: &CommandContext,
) -> Option<HashSet<String>> {
    let mut package_names = BTreeSet::new();
    let mut select_all = false;
    for path in changed_files {
        let absolute = root.join(path);
        match graph.owner_of(&absolute) {
            Some(project) if path_contains(root, Path::new(&project.root)) => {
                package_names.insert(project.name.clone());
            }
            _ => select_all = true,
        }
    }

    let mut command = Command::new(local_tool(root, "turbo"));
    command
        .current_dir(root)
        .args(["ls", "--output=json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if !select_all {
        for name in package_names {
            command.arg(format!("--filter=...{name}"));
        }
    }
    let timeout = ctx.timeout().min(EXTRACTION_TIMEOUT);
    let output = tokio::select! {
        _ = ctx.cancelled() => return None,
        result = tokio::time::timeout(timeout, command.output()) => result.ok()?.ok()?,
    };
    if !output.status.success() {
        return None;
    }
    parse_turbo_packages(&output.stdout)
}

fn parse_turbo_packages(output: &[u8]) -> Option<HashSet<String>> {
    let value: Value = serde_json::from_slice(output).ok()?;
    Some(
        value
            .get("packages")?
            .get("items")?
            .as_array()?
            .iter()
            .filter_map(|project| project.get("name").and_then(Value::as_str))
            .map(str::to_string)
            .collect(),
    )
}

fn local_tool(root: &Path, name: &str) -> PathBuf {
    let executable = if cfg!(windows) {
        format!("{name}.cmd")
    } else {
        name.to_string()
    };
    let local = root.join("node_modules").join(".bin").join(&executable);
    if local.is_file() {
        local
    } else {
        PathBuf::from(executable)
    }
}

#[derive(Default)]
struct Counters {
    scheduled: AtomicU64,
    completed: AtomicU64,
    failures: AtomicU64,
    timeouts: AtomicU64,
}

pub struct ProjectGraphKernelService {
    graph: Arc<ProjectGraphService>,
    workspace: Arc<WorkspaceService>,
    fs: Arc<FileSystemService>,
    logger: Arc<Logger>,
    runtime: ProjectGraphRuntime,
    extractor: Arc<dyn GraphExtractor>,
    extraction_timeout: Duration,
    hub: Arc<Mutex<Option<Arc<StreamHub>>>>,
    workspace_listener_registered: Arc<AtomicBool>,
    fs_listener_registered: Arc<AtomicBool>,
    counters: Arc<Counters>,
}

#[derive(Clone)]
struct SharedState {
    graph: Arc<ProjectGraphService>,
    workspace: Arc<WorkspaceService>,
    fs: Arc<FileSystemService>,
    logger: Arc<Logger>,
    runtime: ProjectGraphRuntime,
    extractor: Arc<dyn GraphExtractor>,
    extraction_timeout: Duration,
    hub: Arc<Mutex<Option<Arc<StreamHub>>>>,
    workspace_listener_registered: Arc<AtomicBool>,
    fs_listener_registered: Arc<AtomicBool>,
    counters: Arc<Counters>,
}

impl ProjectGraphKernelService {
    fn with_shared_state(state: SharedState) -> Self {
        Self {
            graph: state.graph,
            workspace: state.workspace,
            fs: state.fs,
            logger: state.logger,
            runtime: state.runtime,
            extractor: state.extractor,
            extraction_timeout: state.extraction_timeout,
            hub: state.hub,
            workspace_listener_registered: state.workspace_listener_registered,
            fs_listener_registered: state.fs_listener_registered,
            counters: state.counters,
        }
    }
}

#[async_trait]
impl Service for ProjectGraphKernelService {
    fn name(&self) -> &'static str {
        SERVICE_NAME
    }

    fn dependencies(&self) -> &'static [&'static str] {
        &[
            crate::workspace::SERVICE_NAME,
            crate::fs::SERVICE_NAME,
            crate::stream::SERVICE_NAME,
            crate::log::SERVICE_NAME,
        ]
    }

    async fn start(&mut self, ctx: &ServiceContext) -> Result<(), ServiceError> {
        ctx.publish(self.graph.clone());
        *self.hub.lock().unwrap() = ctx.resolve::<StreamHub>();

        if !self
            .workspace_listener_registered
            .swap(true, Ordering::SeqCst)
        {
            let scheduler = self.runtime.scheduler.clone();
            let graph = self.graph.clone();
            let listener: WorkspaceListener = Arc::new(move |event: &WorkspaceEvent| {
                if event.kind == WorkspaceEventKind::Closed && event.torn_down {
                    scheduler.invalidate(&event.key);
                    return;
                }
                let relevant = matches!(
                    event.kind,
                    WorkspaceEventKind::Opened
                        | WorkspaceEventKind::RootsChanged
                        | WorkspaceEventKind::AvailabilityChanged
                        | WorkspaceEventKind::DocumentChanged
                );
                if relevant
                    && let Some(snapshot) = event.workspace.clone()
                    && (event.kind != WorkspaceEventKind::Opened
                        || snapshot.holders == 1
                        || graph.current(&snapshot.key).is_none())
                {
                    scheduler.schedule(snapshot, "workspace changed");
                }
            });
            self.workspace.add_listener(listener);
        }

        if !self.fs_listener_registered.swap(true, Ordering::SeqCst) {
            let scheduler = self.runtime.scheduler.clone();
            let workspace = self.workspace.clone();
            self.fs.add_listener(Arc::new(move |changes| {
                let changed: Vec<PathBuf> = changes
                    .iter()
                    .map(|change| PathBuf::from(&change.path))
                    .filter(|path| is_graph_source_file(path))
                    .collect();
                if changed.is_empty() {
                    return;
                }
                for snapshot in workspace.snapshots() {
                    if changed.iter().any(|path| {
                        snapshot
                            .roots
                            .iter()
                            .any(|root| path_contains(root.as_path(), path))
                    }) {
                        scheduler.schedule(snapshot, "graph source changed");
                    }
                }
            }));
        }

        for snapshot in self.workspace.snapshots() {
            self.runtime.scheduler.schedule(snapshot, "service started");
        }
        log_info!(
            self.logger,
            LOG_SOURCE,
            "project graph service started",
            "channel" => PROJECT_GRAPH_CHANNEL,
            "timeout_ms" => self.extraction_timeout.as_millis() as u64,
        );
        Ok(())
    }

    async fn run(&mut self) -> Result<(), ServiceError> {
        let mut tasks = JoinSet::new();
        let mut active = HashSet::new();
        loop {
            for request in self.runtime.scheduler.take_ready(&active) {
                let key = request.workspace.key.clone();
                active.insert(key.clone());
                self.counters.scheduled.fetch_add(1, Ordering::Relaxed);
                let graph = self.graph.clone();
                let scheduler = self.runtime.scheduler.clone();
                let extractor = self.extractor.clone();
                let timeout = self.extraction_timeout;
                let hub = self.hub.lock().unwrap().clone();
                let logger = self.logger.clone();
                let counters = self.counters.clone();
                tasks.spawn(async move {
                    refresh_snapshot(
                        graph, scheduler, request, extractor, timeout, hub, logger, counters,
                    )
                    .await;
                    key
                });
            }

            if tasks.is_empty() {
                self.runtime.scheduler.notify.notified().await;
                continue;
            }

            tokio::select! {
                _ = self.runtime.scheduler.notify.notified() => {}
                result = tasks.join_next(), if !tasks.is_empty() => {
                    match result {
                        Some(Ok(key)) => {
                            active.remove(&key);
                        }
                        Some(Err(error)) => {
                            self.counters.failures.fetch_add(1, Ordering::Relaxed);
                            log_warn!(
                                self.logger,
                                LOG_SOURCE,
                                "project graph refresh task ended abnormally",
                                "error" => error.to_string(),
                            );
                            tasks.abort_all();
                            while tasks.join_next().await.is_some() {}
                            active.clear();
                            for snapshot in self.workspace.snapshots() {
                                self.runtime.scheduler.schedule(snapshot, "refresh task recovery");
                            }
                        }
                        None => {}
                    }
                }
            }
        }
    }

    async fn stop(&mut self) -> Result<(), ServiceError> {
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
async fn refresh_snapshot(
    graph: Arc<ProjectGraphService>,
    scheduler: ProjectGraphScheduler,
    request: ScheduledExtraction,
    extractor: Arc<dyn GraphExtractor>,
    timeout: Duration,
    hub: Option<Arc<StreamHub>>,
    logger: Arc<Logger>,
    counters: Arc<Counters>,
) {
    if !scheduler.is_current(&request.workspace.key, request.generation) {
        return;
    }
    emit(
        hub.as_ref(),
        ProjectGraphEvent {
            kind: ProjectGraphEventKind::Loading,
            key: request.workspace.key.clone(),
            graph: graph
                .current(&request.workspace.key)
                .map(|graph| graph.as_ref().clone()),
            message: Some(request.reason.to_string()),
        },
    );

    if graph.current(&request.workspace.key).is_none() {
        let initial_graph = graph.clone();
        let initial_workspace = request.workspace.clone();
        let initial = match tokio::task::spawn_blocking(move || {
            initial_graph.cached_or_fallback(&initial_workspace)
        })
        .await
        {
            Ok(Ok(initial)) => initial,
            Ok(Err(error)) => {
                log_warn!(
                    logger,
                    LOG_SOURCE,
                    "project graph cache could not be loaded; using per-root graph",
                    "key" => request.workspace.key.clone(),
                    "error" => error.message.clone(),
                );
                graph.fallback(&request.workspace)
            }
            Err(error) => {
                log_warn!(
                    logger,
                    LOG_SOURCE,
                    "project graph cache task failed; using per-root graph",
                    "key" => request.workspace.key.clone(),
                    "error" => error.to_string(),
                );
                graph.fallback(&request.workspace)
            }
        };
        if scheduler.is_current(&request.workspace.key, request.generation)
            && graph.current(&request.workspace.key).is_none()
        {
            let _ = graph.publish(initial);
        }
    }

    if !scheduler.is_current(&request.workspace.key, request.generation) {
        return;
    }
    let roots: Vec<PathBuf> = request
        .workspace
        .available_roots()
        .into_iter()
        .map(|root| root.as_path().to_path_buf())
        .collect();
    let key = request.workspace.key.clone();
    let mut extraction = tokio::task::spawn_blocking(move || extractor.extract(&key, &roots));
    let extracted = match tokio::time::timeout(timeout, &mut extraction).await {
        Ok(Ok(extracted)) => extracted,
        Ok(Err(error)) => {
            counters.failures.fetch_add(1, Ordering::Relaxed);
            degraded(
                hub.as_ref(),
                &request.workspace.key,
                graph.current(&request.workspace.key),
                format!("project graph extraction task failed: {error}"),
            );
            return;
        }
        Err(_) => {
            counters.timeouts.fetch_add(1, Ordering::Relaxed);
            degraded(
                hub.as_ref(),
                &request.workspace.key,
                graph.current(&request.workspace.key),
                format!(
                    "project graph extraction exceeded {}ms; using the last cached or per-root graph",
                    timeout.as_millis()
                ),
            );
            // Blocking filesystem calls cannot be force-cancelled safely. Keep
            // this workspace's extraction slot occupied until the worker exits,
            // so repeated invalidations cannot accumulate blocking workers.
            let _ = extraction.await;
            return;
        }
    };

    if !scheduler.is_current(&request.workspace.key, request.generation) {
        return;
    }
    let source_paths: Vec<PathBuf> = extracted
        .graph
        .source_files
        .iter()
        .map(PathBuf::from)
        .collect();
    let current_fingerprints =
        tokio::task::spawn_blocking(move || fingerprint_sources(&source_paths)).await;
    if !matches!(current_fingerprints, Ok(ref current) if current == &extracted.fingerprints) {
        counters.failures.fetch_add(1, Ordering::Relaxed);
        scheduler.schedule(
            request.workspace.clone(),
            "sources changed during extraction",
        );
        degraded(
            hub.as_ref(),
            &request.workspace.key,
            graph.current(&request.workspace.key),
            "project graph sources changed during extraction; the result was discarded".to_string(),
        );
        return;
    }
    for warning in &extracted.warnings {
        log_warn!(
            logger,
            LOG_SOURCE,
            "a monorepo manifest could not be extracted",
            "key" => request.workspace.key.clone(),
            "path" => warning.path.to_string_lossy().to_string(),
            "error" => warning.message.clone(),
        );
    }

    if extracted.graph.status == ProjectGraphStatus::Fallback
        && !extracted.warnings.is_empty()
        && graph
            .current(&request.workspace.key)
            .is_some_and(|current| current.status != ProjectGraphStatus::Fallback)
    {
        counters.failures.fetch_add(1, Ordering::Relaxed);
        degraded(
            hub.as_ref(),
            &request.workspace.key,
            graph.current(&request.workspace.key),
            "graph extraction was malformed; keeping the last good graph".to_string(),
        );
        return;
    }

    if extracted.graph.status == ProjectGraphStatus::Fresh {
        if !scheduler.is_current(&request.workspace.key, request.generation) {
            return;
        }
        let cache_graph = extracted.graph.clone();
        let cache_fingerprints = extracted.fingerprints.clone();
        let cache_service = graph.clone();
        if let Ok(Err(error)) = tokio::task::spawn_blocking(move || {
            cache_service.cache(&cache_graph, &cache_fingerprints)
        })
        .await
        {
            log_warn!(
                logger,
                LOG_SOURCE,
                "project graph cache write failed; the in-memory graph remains usable",
                "key" => request.workspace.key.clone(),
                "error" => error.message.clone(),
            );
        }
    }
    let Some(published) = scheduler.publish_if_current(&graph, extracted.graph, request.generation)
    else {
        return;
    };
    match published {
        Ok(published) => {
            counters.completed.fetch_add(1, Ordering::Relaxed);
            emit(
                hub.as_ref(),
                ProjectGraphEvent {
                    kind: ProjectGraphEventKind::Updated,
                    key: request.workspace.key,
                    graph: Some(published.as_ref().clone()),
                    message: None,
                },
            );
        }
        Err(error) => {
            counters.failures.fetch_add(1, Ordering::Relaxed);
            degraded(
                hub.as_ref(),
                &request.workspace.key,
                graph.current(&request.workspace.key),
                error.message,
            );
        }
    }
}

fn degraded(
    hub: Option<&Arc<StreamHub>>,
    key: &str,
    graph: Option<Arc<ProjectGraph>>,
    message: String,
) {
    emit(
        hub,
        ProjectGraphEvent {
            kind: ProjectGraphEventKind::Degraded,
            key: key.to_string(),
            graph: graph.map(|graph| graph.as_ref().clone()),
            message: Some(message),
        },
    );
}

fn emit(hub: Option<&Arc<StreamHub>>, event: ProjectGraphEvent) {
    if let Some(hub) = hub {
        hub.publish(
            PROJECT_GRAPH_CHANNEL,
            serde_json::to_value(event).unwrap_or(Value::Null),
        );
    }
}

impl HealthCheck for ProjectGraphKernelService {
    fn health(&self) -> ServiceHealth {
        let timeouts = self.counters.timeouts.load(Ordering::Relaxed);
        let failures = self.counters.failures.load(Ordering::Relaxed);
        if timeouts + failures > 0 {
            ServiceHealth::Degraded {
                reason: format!(
                    "{timeouts} project graph extraction timeout(s), {failures} failure(s); cached or per-root graphs remain active"
                ),
                since_ms: 0,
            }
        } else {
            ServiceHealth::Healthy
        }
    }

    fn metrics(&self) -> ServiceMetrics {
        ServiceMetrics {
            memory_bytes: 0,
            uptime_ms: 0,
            request_count: self.counters.scheduled.load(Ordering::Relaxed),
            error_count: self.counters.failures.load(Ordering::Relaxed)
                + self.counters.timeouts.load(Ordering::Relaxed),
        }
    }

    fn live_probe(&self) -> Option<ServiceProbe> {
        let health_counters = self.counters.clone();
        let metrics_counters = self.counters.clone();
        Some(ServiceProbe::new(
            move || health_from(&health_counters),
            move || metrics_from(&metrics_counters),
        ))
    }
}

fn health_from(counters: &Counters) -> ServiceHealth {
    let timeouts = counters.timeouts.load(Ordering::Relaxed);
    let failures = counters.failures.load(Ordering::Relaxed);
    if timeouts + failures > 0 {
        ServiceHealth::Degraded {
            reason: format!("{timeouts} extraction timeout(s), {failures} failure(s)"),
            since_ms: 0,
        }
    } else {
        ServiceHealth::Healthy
    }
}

fn metrics_from(counters: &Counters) -> ServiceMetrics {
    ServiceMetrics {
        memory_bytes: 0,
        uptime_ms: 0,
        request_count: counters.scheduled.load(Ordering::Relaxed),
        error_count: counters.failures.load(Ordering::Relaxed)
            + counters.timeouts.load(Ordering::Relaxed),
    }
}

pub fn register(
    container: &mut ServiceContainer,
    graph: Arc<ProjectGraphService>,
    workspace: Arc<WorkspaceService>,
    fs: Arc<FileSystemService>,
    logger: Arc<Logger>,
    runtime: ProjectGraphRuntime,
) -> Result<(), ServiceError> {
    let extractor: Arc<dyn GraphExtractor> = Arc::new(ManifestGraphExtractor);
    let hub = Arc::new(Mutex::new(None));
    let workspace_listener_registered = Arc::new(AtomicBool::new(false));
    let fs_listener_registered = Arc::new(AtomicBool::new(false));
    let counters = Arc::new(Counters::default());
    let shared = SharedState {
        graph,
        workspace,
        fs,
        logger,
        runtime,
        extractor,
        extraction_timeout: EXTRACTION_TIMEOUT,
        hub,
        workspace_listener_registered,
        fs_listener_registered,
        counters,
    };
    container.register(
        SERVICE_NAME,
        &[
            crate::workspace::SERVICE_NAME,
            crate::fs::SERVICE_NAME,
            crate::stream::SERVICE_NAME,
            crate::log::SERVICE_NAME,
        ],
        Lifetime::Singleton,
        move |_ctx| {
            Ok(
                Box::new(ProjectGraphKernelService::with_shared_state(shared.clone()))
                    as Box<dyn ManagedService>,
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_fs::testutil::TempDir;
    use helix_workspace::{
        Project, ProjectGraphCache, RootAvailability, WorkspaceRegistry, WorkspaceRoot,
    };

    fn snapshot(key: &str, root: &Path) -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            key: key.to_string(),
            id: Some(key.to_string()),
            name: "Test".to_string(),
            roots: vec![WorkspaceRoot {
                path: root.to_string_lossy().to_string(),
                name: "repo".to_string(),
                availability: RootAvailability::Available,
                primary: true,
            }],
            file_path: root
                .join(".helix/workspace.json")
                .to_string_lossy()
                .to_string(),
            has_file: false,
            issues: Vec::new(),
            parse_error: None,
            settings_parse_errors: Vec::new(),
            settings_issues: Vec::new(),
            persist_error: None,
            max_roots: 20,
            at_root_limit: false,
            holders: 1,
            opened_ms: 1,
        }
    }

    fn runtime() -> ProjectGraphRuntime {
        ProjectGraphRuntime {
            scheduler: ProjectGraphScheduler {
                generations: Arc::new(Mutex::new(HashMap::new())),
                pending: Arc::new(Mutex::new(HashMap::new())),
                notify: Arc::new(Notify::new()),
            },
        }
    }

    struct SlowExtractor;

    impl GraphExtractor for SlowExtractor {
        fn extract(&self, workspace_key: &str, roots: &[PathBuf]) -> ProjectGraphExtraction {
            std::thread::sleep(Duration::from_millis(50));
            extract_project_graph(workspace_key, roots)
        }
    }

    #[test]
    fn scheduler_coalesces_repeated_refreshes_for_one_workspace() {
        let runtime = runtime();
        let root = Path::new("/repo");
        runtime
            .scheduler
            .schedule(snapshot("workspace", root), "first");
        runtime
            .scheduler
            .schedule(snapshot("workspace", root), "second");
        runtime
            .scheduler
            .schedule(snapshot("workspace", root), "latest");

        let ready = runtime.scheduler.take_ready(&HashSet::new());
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].generation, 3);
        assert_eq!(ready[0].reason, "latest");
    }

    #[test]
    fn stale_generation_cannot_publish() {
        let registry = WorkspaceRegistry::new();
        let _lease = registry.acquire("workspace");
        let graph = ProjectGraphService::with_cache(
            registry,
            ProjectGraphCache::at(std::env::temp_dir().join("helix-unused-cache")),
        );
        let runtime = runtime();
        runtime
            .scheduler
            .schedule(snapshot("workspace", Path::new("/repo")), "first");
        let first = runtime.scheduler.take_ready(&HashSet::new()).pop().unwrap();
        runtime
            .scheduler
            .schedule(snapshot("workspace", Path::new("/repo")), "newer");

        let result = runtime.scheduler.publish_if_current(
            &graph,
            ProjectGraph {
                workspace_key: "workspace".into(),
                projects: Vec::new(),
                tools: Vec::new(),
                source_files: Vec::new(),
                generated_ms: 1,
                status: ProjectGraphStatus::Fresh,
            },
            first.generation,
        );

        assert!(result.is_none());
        assert!(graph.current("workspace").is_none());
    }

    #[test]
    fn successful_native_results_replace_graph_fallback_for_covered_files() {
        let graph = ProjectGraph {
            workspace_key: "workspace".into(),
            projects: vec![
                Project {
                    id: "app".into(),
                    name: "app".into(),
                    root: "/repo/app".into(),
                    dependencies: vec!["lib".into()],
                    tool: MonorepoTool::Nx,
                },
                Project {
                    id: "lib".into(),
                    name: "lib".into(),
                    root: "/repo/lib".into(),
                    dependencies: Vec::new(),
                    tool: MonorepoTool::Nx,
                },
            ],
            tools: vec![MonorepoTool::Nx],
            source_files: Vec::new(),
            generated_ms: 1,
            status: ProjectGraphStatus::Fresh,
        };
        let mut fallback_files = vec![PathBuf::from("/repo/lib/src/lib.ts")];
        let mut affected = BTreeSet::new();
        apply_native_affected(
            &graph,
            Path::new("/repo"),
            &HashSet::from(["lib".to_string()]),
            &mut fallback_files,
            &mut affected,
        );

        assert!(fallback_files.is_empty());
        assert_eq!(affected, BTreeSet::from(["lib".to_string()]));
    }

    #[test]
    fn parses_turbo_package_listing() {
        let packages = parse_turbo_packages(
            br#"{"packageManager":"pnpm","packages":{"count":2,"items":[{"name":"web","path":"apps/web"},{"name":"ui","path":"packages/ui"}]}}"#,
        )
        .unwrap();
        assert_eq!(
            packages,
            HashSet::from(["web".to_string(), "ui".to_string()])
        );
    }

    #[tokio::test]
    async fn graph_commands_answer_ownership_and_relations() {
        let registry = WorkspaceRegistry::new();
        let _lease = registry.acquire("workspace");
        let graph = ProjectGraphService::with_cache(
            registry,
            ProjectGraphCache::at(std::env::temp_dir().join("helix-unused-cache")),
        );
        graph
            .publish(ProjectGraph {
                workspace_key: "workspace".into(),
                projects: vec![
                    Project {
                        id: "app".into(),
                        name: "app".into(),
                        root: "/repo/app".into(),
                        dependencies: vec!["lib".into()],
                        tool: MonorepoTool::Nx,
                    },
                    Project {
                        id: "lib".into(),
                        name: "lib".into(),
                        root: "/repo/lib".into(),
                        dependencies: Vec::new(),
                        tool: MonorepoTool::Nx,
                    },
                ],
                tools: vec![MonorepoTool::Nx],
                source_files: Vec::new(),
                generated_ms: 1,
                status: ProjectGraphStatus::Fresh,
            })
            .unwrap();

        let logger = Arc::new(Logger::in_memory(helix_log::LogLevel::Trace));
        let config = Arc::new(helix_config::ConfigService::load(
            helix_config::ConfigPaths::default(),
            Arc::new(helix_config::SchemaRegistry::builtin()),
            logger.clone(),
        ));
        let fs = crate::fs::build_service(&config, logger.clone());
        let workspace = Arc::new(WorkspaceService::with_recent_path(config, fs, logger, None));
        let scheduler = runtime().scheduler;
        let mut dispatcher = IpcDispatcher::new();
        register_commands(&mut dispatcher, graph, workspace, scheduler);

        let owner = dispatcher
            .dispatch(helix_ipc::IpcRequest::new(
                PROJECT_OWNER,
                "graph-owner",
                serde_json::json!({"key":"workspace","path":"/repo/lib/src/lib.ts"}),
            ))
            .await
            .result
            .unwrap();
        assert_eq!(owner["project"]["id"], "lib");

        let relations = dispatcher
            .dispatch(helix_ipc::IpcRequest::new(
                PROJECT_RELATIONS,
                "graph-relations",
                serde_json::json!({"key":"workspace","project_id":"lib"}),
            ))
            .await
            .result
            .unwrap();
        assert_eq!(relations["dependents"][0]["id"], "app");
    }

    #[tokio::test]
    async fn extraction_timeout_keeps_the_cached_graph() {
        let dir = TempDir::new("kernel-project-graph-timeout");
        let root = dir.mkdir("repo");
        let source = dir.write("repo/nx.json", "{}");
        let registry = WorkspaceRegistry::new();
        let _lease = registry.acquire("workspace");
        let graph = ProjectGraphService::with_cache(
            registry,
            ProjectGraphCache::at(dir.path().join("cache")),
        );
        graph
            .cache(
                &ProjectGraph {
                    workspace_key: "workspace".into(),
                    projects: vec![Project {
                        id: "cached-app".into(),
                        name: "cached-app".into(),
                        root: root.join("app").to_string_lossy().to_string(),
                        dependencies: Vec::new(),
                        tool: MonorepoTool::Nx,
                    }],
                    tools: vec![MonorepoTool::Nx],
                    source_files: vec![source.to_string_lossy().to_string()],
                    generated_ms: 1,
                    status: ProjectGraphStatus::Fresh,
                },
                &helix_workspace::fingerprint_sources(std::slice::from_ref(&source)),
            )
            .unwrap();

        let runtime = runtime();
        assert!(
            runtime
                .scheduler
                .schedule(snapshot("workspace", &root), "test")
        );
        let request = runtime.scheduler.take_ready(&HashSet::new()).pop().unwrap();
        let counters = Arc::new(Counters::default());
        refresh_snapshot(
            graph.clone(),
            runtime.scheduler,
            request,
            Arc::new(SlowExtractor),
            Duration::from_millis(5),
            None,
            Arc::new(Logger::in_memory(helix_log::LogLevel::Trace)),
            counters.clone(),
        )
        .await;

        let current = graph.current("workspace").expect("cached fallback");
        assert_eq!(current.status, ProjectGraphStatus::Cached);
        assert_eq!(current.projects[0].id, "cached-app");
        assert_eq!(counters.timeouts.load(Ordering::Relaxed), 1);
    }
}
