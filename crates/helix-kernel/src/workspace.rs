//! Kernel-side wiring for the workspace manager (Task 1.8).
//!
//! The manager itself lives in `helix-workspace` and knows nothing about Tauri
//! or the service container. This module mirrors [`crate::fs`],
//! [`crate::config`], [`crate::log`], [`crate::stream`], and [`crate::ipc`]:
//!
//! 1. Builds the manager over the configuration and file system services
//!    ([`build_service`]).
//! 2. Registers the `workspace.*` commands the frontend calls
//!    ([`register_commands`]).
//! 3. Registers [`WorkspaceKernelService`] as a container-managed singleton,
//!    which publishes the manager for other services to resolve, binds the file
//!    watcher to each root, bridges events onto the streaming channel, drives
//!    the availability retry, and closes every workspace on shutdown
//!    ([`register`]).
//!
//! ## Where cleanup comes from
//!
//! Opening a root has to start watching it, and removing a root has to stop.
//! That is [`WatcherHook`], registered here because the manager must not depend
//! on the file watcher's API and the watcher must not know what a workspace is.
//! Language servers (Task 5.1), terminals (Task 6.1), and the search index
//! (Task 3.5) register their own hooks the same way, and each is torn down by
//! the same close path — which is the point of making cleanup a registration
//! rather than a list this module maintains.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use helix_config::ConfigService;
use helix_core::container::{
    HealthCheck, Lifetime, ManagedService, Service, ServiceContainer, ServiceContext, ServiceError,
};
use helix_core::error::AppError;
use helix_core::health::{ServiceHealth, ServiceMetrics};
use helix_fs::FileSystemService;
use helix_ipc::IpcDispatcher;
use helix_log::{Logger, log_info, log_warn};
use helix_stream::StreamHub;
use helix_workspace::commands::{
    ADD_ROOT, CLOSE, FORGET_RECENT, LIST, OPEN, RECENT, REFRESH, REMOVE_ROOT, SCHEMA, SETTINGS,
    WorkspaceCloseRequest, WorkspaceCloseResponse, WorkspaceForgetRecentRequest,
    WorkspaceForgetRecentResponse, WorkspaceListRequest, WorkspaceListResponse,
    WorkspaceOpenRequest, WorkspaceRecentRequest, WorkspaceRecentResponse, WorkspaceResponse,
    WorkspaceRootRequest, WorkspaceSchemaRequest, WorkspaceSchemaResponse,
    WorkspaceSettingsRequest, WorkspaceSettingsResponse,
};
use helix_workspace::{
    CHANNEL, DEFAULT_RETRY_INTERVAL, LOG_SOURCE, RootEvent, WorkspaceEvent, WorkspaceHooks,
    WorkspaceListener, WorkspaceService,
};

/// Container service name for the workspace layer.
pub const SERVICE_NAME: &str = "workspace";

/// Build the workspace manager over the configuration and file system services.
pub fn build_service(
    config: Arc<ConfigService>,
    fs: Arc<FileSystemService>,
    logger: Arc<Logger>,
) -> Arc<WorkspaceService> {
    Arc::new(WorkspaceService::new(config, fs, logger))
}

/// Starts and stops watching a root as it joins and leaves a workspace
/// (REQ-FS-004.1, and the cleanup half of REQ-FS-001.4).
pub struct WatcherHook {
    fs: Arc<FileSystemService>,
    logger: Arc<Logger>,
}

impl WatcherHook {
    pub fn new(fs: Arc<FileSystemService>, logger: Arc<Logger>) -> Self {
        Self { fs, logger }
    }
}

impl WorkspaceHooks for WatcherHook {
    fn name(&self) -> &'static str {
        "fs.watcher"
    }

    fn root_opened(&self, event: &RootEvent<'_>) -> Result<(), AppError> {
        let report = self.fs.watch(event.root)?;
        if report.over_budget {
            // Not an error: the root is watched, just expensively. The watcher
            // already suggests exclusions, and refusing to open a large
            // repository would be the wrong trade.
            log_warn!(
                self.logger,
                LOG_SOURCE,
                "a workspace root exceeds the watcher path budget",
                "root" => report.root.clone(),
                "watched_paths" => report.watched_paths,
                "suggested_exclusions" => report.suggested_exclusions.clone(),
            );
        }
        Ok(())
    }

    fn root_closed(&self, event: &RootEvent<'_>) {
        // A root shared with another open workspace is unwatched here and
        // re-watched by that workspace's own binding on the next availability
        // pass. Watching is idempotent, so the cost is one interval of missed
        // events in a rare configuration, not a leaked registration.
        if let Err(error) = self.fs.unwatch(event.root) {
            log_warn!(
                self.logger,
                LOG_SOURCE,
                "a workspace root could not be unwatched",
                "root" => event.root.to_string_lossy().to_string(),
                "error" => error.message.clone(),
            );
        }
    }
}

/// Register the `workspace.*` commands on the kernel's dispatcher.
///
/// Each handler closes over the shared manager rather than being a method on the
/// managed wrapper, so the command surface is testable without starting the
/// container.
///
/// The work runs on `spawn_blocking` for the same reason the `fs.*` handlers do:
/// opening a workspace stats every root, reads the workspace document and the
/// settings files, and starts watchers, all of which are blocking syscalls that
/// can take arbitrarily long on a network share.
pub fn register_commands(dispatcher: &mut IpcDispatcher, workspace: Arc<WorkspaceService>) {
    let open_workspace = workspace.clone();
    dispatcher.register(OPEN, move |req: WorkspaceOpenRequest, _ctx| {
        let workspace = open_workspace.clone();
        async move {
            let roots: Vec<PathBuf> = req.roots.iter().map(PathBuf::from).collect();
            let name = req.name.clone();
            let snapshot = blocking(move || workspace.open(&roots, name.as_deref())).await?;
            Ok::<WorkspaceResponse, AppError>(WorkspaceResponse {
                workspace: snapshot,
            })
        }
    });

    let close_workspace = workspace.clone();
    dispatcher.register(CLOSE, move |req: WorkspaceCloseRequest, _ctx| {
        let workspace = close_workspace.clone();
        async move {
            let closing = workspace.clone();
            let key = req.key.clone();
            let torn_down = blocking(move || closing.close(&key)).await?;
            // Read back rather than inferred, so the answer is the registry's
            // count and not this handler's arithmetic.
            let remaining = workspace
                .snapshot(&req.key)
                .map(|snapshot| snapshot.holders)
                .unwrap_or(0);
            Ok::<WorkspaceCloseResponse, AppError>(WorkspaceCloseResponse {
                closed: true,
                torn_down,
                remaining_holders: remaining,
            })
        }
    });

    let list_workspace = workspace.clone();
    dispatcher.register(LIST, move |_req: WorkspaceListRequest, _ctx| {
        let workspace = list_workspace.clone();
        async move {
            Ok::<WorkspaceListResponse, AppError>(WorkspaceListResponse {
                workspaces: workspace.snapshots(),
            })
        }
    });

    let add_workspace = workspace.clone();
    dispatcher.register(ADD_ROOT, move |req: WorkspaceRootRequest, _ctx| {
        let workspace = add_workspace.clone();
        async move {
            let snapshot = blocking(move || {
                workspace.add_root(&req.key, &PathBuf::from(&req.path), req.name.as_deref())
            })
            .await?;
            Ok::<WorkspaceResponse, AppError>(WorkspaceResponse {
                workspace: snapshot,
            })
        }
    });

    let remove_workspace = workspace.clone();
    dispatcher.register(REMOVE_ROOT, move |req: WorkspaceRootRequest, _ctx| {
        let workspace = remove_workspace.clone();
        async move {
            let snapshot =
                blocking(move || workspace.remove_root(&req.key, &PathBuf::from(&req.path)))
                    .await?;
            Ok::<WorkspaceResponse, AppError>(WorkspaceResponse {
                workspace: snapshot,
            })
        }
    });

    let settings_workspace = workspace.clone();
    dispatcher.register(SETTINGS, move |req: WorkspaceSettingsRequest, _ctx| {
        let workspace = settings_workspace.clone();
        async move {
            let path = req.path.as_ref().map(PathBuf::from);
            let root = path
                .as_deref()
                .and_then(|path| workspace.owning_root(&req.key, path))
                .map(|root| root.to_string_lossy().to_string());
            let settings = workspace.settings_tree(&req.key, path.as_deref())?;
            let value = match &req.setting {
                Some(setting) => workspace.setting_value(&req.key, path.as_deref(), setting)?,
                None => None,
            };
            Ok::<WorkspaceSettingsResponse, AppError>(WorkspaceSettingsResponse {
                root,
                settings,
                value,
            })
        }
    });

    let recent_workspace = workspace.clone();
    dispatcher.register(RECENT, move |_req: WorkspaceRecentRequest, _ctx| {
        let workspace = recent_workspace.clone();
        async move {
            Ok::<WorkspaceRecentResponse, AppError>(WorkspaceRecentResponse {
                entries: workspace.recent(),
            })
        }
    });

    let forget_workspace = workspace.clone();
    dispatcher.register(
        FORGET_RECENT,
        move |req: WorkspaceForgetRecentRequest, _ctx| {
            let workspace = forget_workspace.clone();
            async move {
                let forgotten = blocking(move || Ok(workspace.forget_recent(&req.key))).await?;
                Ok::<WorkspaceForgetRecentResponse, AppError>(WorkspaceForgetRecentResponse {
                    forgotten,
                })
            }
        },
    );

    let refresh_workspace = workspace.clone();
    dispatcher.register(REFRESH, move |_req: WorkspaceListRequest, _ctx| {
        let workspace = refresh_workspace.clone();
        async move {
            let workspaces = blocking(move || {
                workspace.refresh_availability();
                Ok(workspace.snapshots())
            })
            .await?;
            Ok::<WorkspaceListResponse, AppError>(WorkspaceListResponse { workspaces })
        }
    });

    dispatcher.register(SCHEMA, move |_req: WorkspaceSchemaRequest, _ctx| {
        let workspace = workspace.clone();
        async move {
            Ok::<WorkspaceSchemaResponse, AppError>(WorkspaceSchemaResponse {
                schema: workspace.document_schema(),
            })
        }
    });
}

/// Run blocking workspace work off the async worker pool.
async fn blocking<T, F>(work: F) -> Result<T, AppError>
where
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(result) => result,
        Err(join_error) => Err(AppError::transient(
            "WORKSPACE_TASK_FAILED",
            format!("the workspace task did not complete: {join_error}"),
        )),
    }
}

/// Container-managed wrapper around the workspace manager.
pub struct WorkspaceKernelService {
    workspace: Arc<WorkspaceService>,
    fs: Arc<FileSystemService>,
    logger: Arc<Logger>,
    retry_interval: Duration,
}

impl WorkspaceKernelService {
    pub fn new(
        workspace: Arc<WorkspaceService>,
        fs: Arc<FileSystemService>,
        logger: Arc<Logger>,
    ) -> Self {
        Self {
            workspace,
            fs,
            logger,
            retry_interval: DEFAULT_RETRY_INTERVAL,
        }
    }

    /// Retry interval for unavailable roots. Shortened by tests.
    pub fn with_retry_interval(mut self, interval: Duration) -> Self {
        self.retry_interval = interval;
        self
    }

    pub fn workspace(&self) -> &Arc<WorkspaceService> {
        &self.workspace
    }
}

#[async_trait]
impl Service for WorkspaceKernelService {
    fn name(&self) -> &'static str {
        SERVICE_NAME
    }

    fn dependencies(&self) -> &'static [&'static str] {
        // Settings supply the root cap and the layers a workspace resolves over,
        // roots are read and watched through the file system service, events
        // publish onto the stream hub, and workspace problems are logged.
        &[
            crate::config::SERVICE_NAME,
            crate::fs::SERVICE_NAME,
            crate::stream::SERVICE_NAME,
            crate::log::SERVICE_NAME,
        ]
    }

    async fn start(&mut self, ctx: &ServiceContext) -> Result<(), ServiceError> {
        // Published so every later workspace-scoped service (project graph,
        // search index, git, LSP host, terminals) resolves roots and settings
        // through this one.
        ctx.publish(self.workspace.clone());

        self.workspace.add_hook(Arc::new(WatcherHook::new(
            self.fs.clone(),
            self.logger.clone(),
        )));

        if let Some(hub) = ctx.resolve::<StreamHub>() {
            let bridge_hub = hub.clone();
            let listener: WorkspaceListener = Arc::new(move |event: &WorkspaceEvent| {
                let payload = serde_json::to_value(event).unwrap_or(serde_json::Value::Null);
                bridge_hub.publish(CHANNEL, payload);
            });
            self.workspace.add_listener(listener);
        }

        log_info!(
            self.logger,
            LOG_SOURCE,
            "workspace manager started",
            "channel" => CHANNEL,
            "max_roots" => self.workspace.max_roots(),
            "recent_workspaces" => self.workspace.recent().len(),
            "retry_interval_ms" => self.retry_interval.as_millis() as u64,
        );
        Ok(())
    }

    /// Steady state: retry unavailable roots.
    ///
    /// This is the periodic retry REQ-FS-001's failure modes require. It is
    /// cheap — one `stat` per root — and it is the only thing that gets a
    /// workspace back after a drive is remounted without the user reopening it.
    async fn run(&mut self) -> Result<(), ServiceError> {
        loop {
            tokio::time::sleep(self.retry_interval).await;
            let workspace = self.workspace.clone();
            // On a dropped network share a `stat` can block for seconds, so the
            // probe runs off the async worker rather than stalling every other
            // task on the runtime.
            let _ = tokio::task::spawn_blocking(move || workspace.refresh_availability()).await;
        }
    }

    async fn stop(&mut self) -> Result<(), ServiceError> {
        // Closing every workspace on shutdown rather than leaving watchers,
        // servers, and terminals to process teardown (REQ-ARCH-002.4). Each
        // close releases one holder, so a workspace shared by two windows is
        // closed twice.
        for snapshot in self.workspace.snapshots() {
            for _ in 0..snapshot.holders {
                if let Err(error) = self.workspace.close(&snapshot.key) {
                    log_warn!(
                        self.logger,
                        LOG_SOURCE,
                        "a workspace did not close cleanly",
                        "key" => snapshot.key.clone(),
                        "error" => error.message.clone(),
                    );
                    break;
                }
            }
        }
        Ok(())
    }
}

impl HealthCheck for WorkspaceKernelService {
    fn health(&self) -> ServiceHealth {
        let metrics = self.workspace.metrics();

        // A root the user can see but cannot use is degraded service, whatever
        // the rest of the workspace is doing (REQ-OBS-004.3).
        if metrics.unavailable_roots > 0 {
            return ServiceHealth::Degraded {
                reason: format!(
                    "{} workspace root(s) are unavailable; they are retried every {}s and the \
                     other roots are unaffected",
                    metrics.unavailable_roots,
                    DEFAULT_RETRY_INTERVAL.as_secs()
                ),
                since_ms: 0,
            };
        }
        if metrics.document_write_errors > 0 {
            return ServiceHealth::Degraded {
                reason: format!(
                    "{} workspace file write(s) failed; root changes apply to this session only",
                    metrics.document_write_errors
                ),
                since_ms: 0,
            };
        }
        if metrics.parse_errors > 0 {
            return ServiceHealth::Degraded {
                reason: format!(
                    "{} workspace file(s) could not be parsed; those workspaces opened on the \
                     folders that were requested",
                    metrics.parse_errors
                ),
                since_ms: 0,
            };
        }
        ServiceHealth::Healthy
    }

    fn metrics(&self) -> ServiceMetrics {
        let metrics = self.workspace.metrics();
        ServiceMetrics {
            memory_bytes: 0,
            uptime_ms: 0,
            request_count: metrics.opens
                + metrics.closes
                + metrics.roots_added
                + metrics.roots_removed
                + metrics.document_writes,
            error_count: metrics.document_write_errors + metrics.parse_errors + metrics.hook_errors,
        }
    }
}

/// Register [`WorkspaceKernelService`] on a container as a supervised singleton.
pub fn register(
    container: &mut ServiceContainer,
    workspace: Arc<WorkspaceService>,
    fs: Arc<FileSystemService>,
    logger: Arc<Logger>,
) -> Result<(), ServiceError> {
    container.register(
        SERVICE_NAME,
        &[
            crate::config::SERVICE_NAME,
            crate::fs::SERVICE_NAME,
            crate::stream::SERVICE_NAME,
            crate::log::SERVICE_NAME,
        ],
        Lifetime::Singleton,
        move |_ctx| {
            Ok(Box::new(WorkspaceKernelService::new(
                workspace.clone(),
                fs.clone(),
                logger.clone(),
            )) as Box<dyn ManagedService>)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_config::{ConfigPaths, SchemaRegistry};
    use helix_fs::testutil::TempDir;
    use helix_ipc::IpcRequest;
    use helix_log::LogLevel;
    use helix_stream::{ChannelSubscription, StreamFrame};
    use helix_workspace::{RootAvailability, same_path};
    use std::path::Path;

    fn logger() -> Arc<Logger> {
        Arc::new(Logger::in_memory(LogLevel::Trace))
    }

    fn defaults_only_config(logger: Arc<Logger>) -> Arc<ConfigService> {
        Arc::new(ConfigService::load(
            // Deliberately not `for_user()`: a developer machine's own
            // ~/.helix/settings.json must not be able to change the outcome of
            // the suite.
            ConfigPaths::default(),
            Arc::new(SchemaRegistry::builtin()),
            logger,
        ))
    }

    /// The manager, with its recent list inside the scratch directory rather
    /// than in the developer's home.
    fn service(
        dir: &TempDir,
        logger: Arc<Logger>,
    ) -> (Arc<WorkspaceService>, Arc<FileSystemService>) {
        let config = defaults_only_config(logger.clone());
        let fs = crate::fs::build_service(&config, logger.clone());
        let workspace = Arc::new(WorkspaceService::with_recent_path(
            config,
            fs.clone(),
            logger,
            Some(dir.path().join("recent.json")),
        ));
        (workspace, fs)
    }

    fn dispatcher(workspace: Arc<WorkspaceService>) -> IpcDispatcher {
        let mut dispatcher = IpcDispatcher::new();
        register_commands(&mut dispatcher, workspace);
        dispatcher
    }

    fn watched(fs: &FileSystemService, root: &Path) -> bool {
        fs.watched_roots()
            .iter()
            .any(|report| same_path(Path::new(&report.root), root))
    }

    /// The Task 1.8 demo criterion, end to end over the dispatcher: open a
    /// two-root workspace, add a third over IPC, verify merged settings, remove
    /// one and verify cleanup.
    #[tokio::test]
    async fn the_demo_opens_two_roots_adds_a_third_and_cleans_up_the_one_removed() {
        let dir = TempDir::new("kernel-workspace-demo");
        let api = dir.mkdir("api");
        let web = dir.mkdir("web");
        let tools = dir.mkdir("tools");
        dir.write(
            "api/.helix/workspace.json",
            r#"{
                "id": "demo-workspace",
                "name": "Payments",
                "folders": [".", "../web"],
                "settings": { "editor.tabSize": 2 }
            }"#,
        );
        // The `web` root has its own opinion, for its own files.
        dir.write("web/.helix/settings.json", r#"{ "editor.tabSize": 8 }"#);

        let logger = logger();
        let (workspace, fs) = service(&dir, logger.clone());
        workspace.add_hook(Arc::new(WatcherHook::new(fs.clone(), logger)));
        let dispatcher = dispatcher(workspace.clone());

        // 1. Open the two-root workspace.
        let opened = dispatcher
            .dispatch(IpcRequest::new(
                OPEN,
                "ws-1",
                serde_json::json!({ "roots": [api.to_string_lossy()] }),
            ))
            .await
            .result
            .expect("workspace.open must succeed");
        assert_eq!(opened["workspace"]["key"], "demo-workspace");
        assert_eq!(opened["workspace"]["roots"].as_array().unwrap().len(), 2);
        assert!(
            watched(&fs, &api) && watched(&fs, &web),
            "both roots watched"
        );

        // 2. Add a third root over IPC.
        let added = dispatcher
            .dispatch(IpcRequest::new(
                ADD_ROOT,
                "ws-2",
                serde_json::json!({
                    "key": "demo-workspace",
                    "path": tools.to_string_lossy(),
                }),
            ))
            .await
            .result
            .expect("workspace.addRoot must succeed");
        assert_eq!(added["workspace"]["roots"].as_array().unwrap().len(), 3);
        assert!(watched(&fs, &tools), "the new root is watched too");

        // 3. Merged settings: the folder layer wins for its own root only.
        let for_web = dispatcher
            .dispatch(IpcRequest::new(
                SETTINGS,
                "ws-3",
                serde_json::json!({
                    "key": "demo-workspace",
                    "path": web.join("app.ts").to_string_lossy(),
                    "setting": "editor.tabSize",
                }),
            ))
            .await
            .result
            .expect("workspace.settings must succeed");
        assert_eq!(for_web["value"], 8);
        assert!(same_path(
            Path::new(for_web["root"].as_str().unwrap()),
            &web
        ));

        let for_api = dispatcher
            .dispatch(IpcRequest::new(
                SETTINGS,
                "ws-4",
                serde_json::json!({
                    "key": "demo-workspace",
                    "path": api.join("main.rs").to_string_lossy(),
                    "setting": "editor.tabSize",
                }),
            ))
            .await
            .result
            .unwrap();
        assert_eq!(for_api["value"], 2, "the workspace value applies elsewhere");

        // 4. Remove one root and confirm its watcher is released.
        let removed = dispatcher
            .dispatch(IpcRequest::new(
                REMOVE_ROOT,
                "ws-5",
                serde_json::json!({
                    "key": "demo-workspace",
                    "path": web.to_string_lossy(),
                }),
            ))
            .await
            .result
            .expect("workspace.removeRoot must succeed");
        assert_eq!(removed["workspace"]["roots"].as_array().unwrap().len(), 2);
        assert!(!watched(&fs, &web), "the removed root is no longer watched");
        assert!(
            watched(&fs, &api) && watched(&fs, &tools),
            "and the roots that remain are untouched"
        );

        // The document on disk reflects the change, so the next open agrees.
        let body = std::fs::read_to_string(api.join(".helix/workspace.json")).unwrap();
        let document: serde_json::Value = serde_json::from_str(&body).unwrap();
        let folders: Vec<String> = document["folders"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["path"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(folders, vec![".".to_string(), "../tools".to_string()]);
    }

    #[tokio::test]
    async fn closing_over_ipc_reports_whether_the_workspace_was_torn_down() {
        let dir = TempDir::new("kernel-workspace-close");
        let api = dir.mkdir("api");
        let (workspace, _fs) = service(&dir, logger());
        let dispatcher = dispatcher(workspace.clone());

        let mut key = String::new();
        for id in ["w1", "w2"] {
            let opened = dispatcher
                .dispatch(IpcRequest::new(
                    OPEN,
                    id,
                    serde_json::json!({ "roots": [api.to_string_lossy()] }),
                ))
                .await
                .result
                .unwrap();
            key = opened["workspace"]["key"].as_str().unwrap().to_string();
        }

        let first = dispatcher
            .dispatch(IpcRequest::new(
                CLOSE,
                "c1",
                serde_json::json!({ "key": key }),
            ))
            .await
            .result
            .unwrap();
        assert_eq!(first["torn_down"], false, "one of two windows closing");
        assert_eq!(first["remaining_holders"], 1);

        let second = dispatcher
            .dispatch(IpcRequest::new(
                CLOSE,
                "c2",
                serde_json::json!({ "key": key }),
            ))
            .await
            .result
            .unwrap();
        assert_eq!(second["torn_down"], true);
        assert_eq!(second["remaining_holders"], 0);
    }

    #[tokio::test]
    async fn list_recent_and_schema_answer_over_ipc() {
        let dir = TempDir::new("kernel-workspace-reads");
        let api = dir.mkdir("api");
        let (workspace, _fs) = service(&dir, logger());
        let dispatcher = dispatcher(workspace.clone());

        dispatcher
            .dispatch(IpcRequest::new(
                OPEN,
                "r0",
                serde_json::json!({ "roots": [api.to_string_lossy()], "name": "Api" }),
            ))
            .await
            .result
            .unwrap();

        let listed = dispatcher
            .dispatch(IpcRequest::new(LIST, "r1", serde_json::json!({})))
            .await
            .result
            .unwrap();
        assert_eq!(listed["workspaces"].as_array().unwrap().len(), 1);

        let recent = dispatcher
            .dispatch(IpcRequest::new(RECENT, "r2", serde_json::json!({})))
            .await
            .result
            .unwrap();
        assert_eq!(recent["entries"][0]["name"], "Api");

        let schema = dispatcher
            .dispatch(IpcRequest::new(SCHEMA, "r3", serde_json::json!({})))
            .await
            .result
            .unwrap();
        assert_eq!(schema["schema"]["properties"]["folders"]["maxItems"], 20);

        let key = listed["workspaces"][0]["key"].as_str().unwrap().to_string();
        let forgotten = dispatcher
            .dispatch(IpcRequest::new(
                FORGET_RECENT,
                "r4",
                serde_json::json!({ "key": key }),
            ))
            .await
            .result
            .unwrap();
        assert_eq!(forgotten["forgotten"], true);
    }

    #[tokio::test]
    async fn refresh_over_ipc_picks_up_a_root_that_came_back() {
        let dir = TempDir::new("kernel-workspace-refresh");
        let api = dir.mkdir("api");
        let late = dir.path().join("late");
        let (workspace, fs) = service(&dir, logger());
        workspace.add_hook(Arc::new(WatcherHook::new(fs.clone(), logger())));
        let dispatcher = dispatcher(workspace.clone());

        let opened = dispatcher
            .dispatch(IpcRequest::new(
                OPEN,
                "f1",
                serde_json::json!({
                    "roots": [api.to_string_lossy(), late.to_string_lossy()],
                }),
            ))
            .await
            .result
            .unwrap();
        let key = opened["workspace"]["key"].as_str().unwrap().to_string();
        assert!(!watched(&fs, &late));

        std::fs::create_dir_all(&late).unwrap();
        let refreshed = dispatcher
            .dispatch(IpcRequest::new(REFRESH, "f2", serde_json::json!({})))
            .await
            .result
            .unwrap();

        let roots = refreshed["workspaces"][0]["roots"].as_array().unwrap();
        assert!(
            roots
                .iter()
                .all(|root| root["availability"] == RootAvailability::Available.as_str())
        );
        assert!(watched(&fs, &late), "the returning root is watched now");
        assert!(workspace.is_open(&key));
    }

    #[tokio::test]
    async fn the_service_publishes_itself_and_reports_health() {
        let dir = TempDir::new("kernel-workspace-container");
        let logger = logger();
        let streaming = crate::stream::bind(
            helix_stream::HubConfig::default(),
            helix_stream::ServerConfig::default(),
        )
        .await
        .unwrap();
        let config = defaults_only_config(logger.clone());
        let fs = crate::fs::build_service(&config, logger.clone());
        let workspace = Arc::new(WorkspaceService::with_recent_path(
            config.clone(),
            fs.clone(),
            logger.clone(),
            Some(dir.path().join("recent.json")),
        ));

        let mut container = ServiceContainer::new();
        crate::stream::register(&mut container, streaming.clone()).unwrap();
        crate::log::register(&mut container, logger.clone()).unwrap();
        crate::config::register(&mut container, config, logger.clone()).unwrap();
        crate::fs::register(&mut container, fs.clone(), logger.clone()).unwrap();
        register(&mut container, workspace.clone(), fs, logger).unwrap();
        container.start_all().await.unwrap();

        assert_eq!(
            container.health_summary().get(SERVICE_NAME),
            Some(&ServiceHealth::Healthy)
        );
        let resolved = container
            .context()
            .resolve::<WorkspaceService>()
            .expect("dependents must resolve the workspace manager");
        assert!(Arc::ptr_eq(&resolved, &workspace));

        container.stop_all().await.unwrap();
    }

    #[tokio::test]
    async fn a_workspace_change_reaches_the_frontend_over_the_streaming_channel() {
        let dir = TempDir::new("kernel-workspace-bridge");
        let api = dir.mkdir("api");
        let logger = logger();
        let streaming = crate::stream::bind(
            helix_stream::HubConfig::default(),
            helix_stream::ServerConfig::default(),
        )
        .await
        .unwrap();
        let config = defaults_only_config(logger.clone());
        let fs = crate::fs::build_service(&config, logger.clone());
        let workspace = Arc::new(WorkspaceService::with_recent_path(
            config.clone(),
            fs.clone(),
            logger.clone(),
            Some(dir.path().join("recent.json")),
        ));

        let mut container = ServiceContainer::new();
        crate::stream::register(&mut container, streaming.clone()).unwrap();
        crate::log::register(&mut container, logger.clone()).unwrap();
        crate::config::register(&mut container, config, logger.clone()).unwrap();
        crate::fs::register(&mut container, fs.clone(), logger.clone()).unwrap();
        register(&mut container, workspace.clone(), fs, logger).unwrap();
        container.start_all().await.unwrap();

        let session = streaming.hub().open_session();
        session.subscribe(&[ChannelSubscription::new(CHANNEL)]);
        let _ = session.drain();

        workspace.open(&[api], None).unwrap();

        let envelope = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let Some(frames) = session.next_frames().await else {
                    panic!("session closed unexpectedly");
                };
                for frame in frames {
                    if let StreamFrame::Data(envelope) = frame
                        && envelope.payload["kind"] == "opened"
                    {
                        return envelope;
                    }
                }
            }
        })
        .await
        .expect("a workspace change must reach the channel");

        assert_eq!(
            envelope.payload["workspace"]["roots"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        container.stop_all().await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_closes_every_open_workspace() {
        let dir = TempDir::new("kernel-workspace-shutdown");
        let api = dir.mkdir("api");
        let logger = logger();
        let (workspace, fs) = service(&dir, logger.clone());
        workspace.add_hook(Arc::new(WatcherHook::new(fs.clone(), logger.clone())));

        let mut service = WorkspaceKernelService::new(workspace.clone(), fs.clone(), logger);
        let snapshot = workspace.open(std::slice::from_ref(&api), None).unwrap();
        assert!(watched(&fs, &api));

        service.stop().await.unwrap();

        assert!(!workspace.is_open(&snapshot.key));
        assert!(
            !watched(&fs, &api),
            "shutdown releases watchers rather than leaving them to process teardown"
        );
    }

    #[test]
    fn health_degrades_while_a_root_is_unavailable() {
        let dir = TempDir::new("kernel-workspace-health");
        let api = dir.mkdir("api");
        let missing = dir.path().join("missing");
        let logger = logger();
        let (workspace, fs) = service(&dir, logger.clone());
        workspace.open(&[api, missing], None).unwrap();

        let service = WorkspaceKernelService::new(workspace, fs, logger);
        match service.health() {
            ServiceHealth::Degraded { reason, .. } => {
                assert!(reason.contains("unavailable"), "{reason}");
            }
            other => panic!("expected degraded health, got {other:?}"),
        }
    }

    #[test]
    fn metrics_carry_the_workspace_counts_into_the_container_shape() {
        let dir = TempDir::new("kernel-workspace-metrics");
        let api = dir.mkdir("api");
        let web = dir.mkdir("web");
        let logger = logger();
        let (workspace, fs) = service(&dir, logger.clone());
        let snapshot = workspace.open(&[api], None).unwrap();
        workspace.add_root(&snapshot.key, &web, None).unwrap();

        let metrics = WorkspaceKernelService::new(workspace, fs, logger).metrics();
        // One open, one root added, one document write.
        assert_eq!(metrics.request_count, 3);
        assert_eq!(metrics.error_count, 0);
    }
}
