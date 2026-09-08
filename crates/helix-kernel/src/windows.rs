//! Kernel window registry (Task 2.4, REQ-ARCH-006).
//!
//! The Helix Host owns native windows. This service is the authoritative
//! record of those windows, the workspace each is bound to, and the
//! window-scoped layout/geometry that must not leak across windows.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use helix_config::ConfigService;
use helix_core::container::{
    HealthCheck, Lifetime, ManagedService, Service, ServiceContainer, ServiceContext, ServiceError,
    ServiceProbe,
};
use helix_core::error::AppError;
use helix_core::health::{ServiceHealth, ServiceMetrics};
use helix_ipc::IpcDispatcher;
use helix_log::{Logger, log_info, log_warn};
use helix_state::state_root_directory;
use helix_stream::StreamHub;
use helix_workspace::WorkspaceService;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

pub const SERVICE_NAME: &str = "windows";
pub const LOG_SOURCE: &str = "kernel.windows";
pub const CHANNEL: &str = "window:changed";
pub const RESTORE_SETTING: &str = "window.restoreWindows";

pub const LIST: &str = "window.list";
pub const OPEN: &str = "window.open";
pub const CLOSE: &str = "window.close";
pub const FOCUS: &str = "window.focus";
pub const SET_GEOMETRY: &str = "window.setGeometry";
pub const FIND: &str = "window.findByWorkspace";
pub const SESSION: &str = "window.session";
pub const GET_LAYOUT: &str = "window.layout.get";
pub const SET_LAYOUT: &str = "window.layout.set";
pub const ROUTE: &str = "window.routeNotification";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct WindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
    pub fullscreen: bool,
    pub monitor: Option<String>,
}

impl Default for WindowGeometry {
    fn default() -> Self {
        Self {
            x: 80,
            y: 80,
            width: 1200,
            height: 800,
            maximized: false,
            fullscreen: false,
            monitor: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct WindowRecord {
    pub id: String,
    pub workspace_key: Option<String>,
    pub roots: Vec<String>,
    pub geometry: WindowGeometry,
    pub focused: bool,
    #[ts(type = "unknown")]
    #[serde(default)]
    pub layout: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct WindowSession {
    pub windows: Vec<WindowRecord>,
    pub focused_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct WindowOpenRequest {
    pub id: String,
    pub roots: Vec<String>,
    pub geometry: Option<WindowGeometry>,
    #[ts(type = "unknown")]
    pub layout: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct WindowOpenResponse {
    pub window: WindowRecord,
    pub focused_existing: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct WindowIdRequest {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct WindowCloseResponse {
    pub closed: bool,
    pub workspace_torn_down: bool,
    pub remaining: u32,
    pub shutdown_kernel: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct WindowFindRequest {
    pub roots: Vec<String>,
    pub force_new: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct WindowFindResponse {
    pub window_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct WindowListResponse {
    pub windows: Vec<WindowRecord>,
    pub focused_id: Option<String>,
    pub restore_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct WindowSessionResponse {
    pub session: WindowSession,
    pub restore_enabled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct WindowGeometryRequest {
    pub id: String,
    pub geometry: WindowGeometry,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct WindowLayoutGetRequest {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct WindowLayoutGetResponse {
    #[ts(type = "unknown")]
    pub layout: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct WindowLayoutSetRequest {
    pub id: String,
    #[ts(type = "unknown")]
    pub layout: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default, rename_all = "snake_case")]
pub struct WindowRouteRequest {
    pub scope: NotificationScope,
    pub origin_window_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum NotificationScope {
    #[default]
    Global,
    Window,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct WindowRouteResponse {
    pub window_id: Option<String>,
}

#[derive(Clone)]
pub struct WindowManager {
    inner: Arc<Mutex<Inner>>,
    store_path: PathBuf,
    config: Arc<ConfigService>,
    workspace: Arc<WorkspaceService>,
    logger: Arc<Logger>,
}

struct Inner {
    windows: BTreeMap<String, WindowRecord>,
    focused_id: Option<String>,
}

impl WindowManager {
    pub fn new(
        store_path: PathBuf,
        config: Arc<ConfigService>,
        workspace: Arc<WorkspaceService>,
        logger: Arc<Logger>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                windows: BTreeMap::new(),
                focused_id: None,
            })),
            store_path,
            config,
            workspace,
            logger,
        }
    }

    pub fn restore_enabled(&self) -> bool {
        if std::env::var_os(helix_ipc::KERNEL_SKIP_SESSION_RESTORE_ENV).is_some() {
            return false;
        }
        self.config.bool_value(RESTORE_SETTING).unwrap_or(true)
    }

    pub fn load_session(&self) -> WindowSession {
        if !self.restore_enabled() {
            return WindowSession::default();
        }
        match std::fs::read_to_string(&self.store_path) {
            Ok(text) => match serde_json::from_str::<WindowSession>(&text) {
                Ok(session) => session,
                Err(_) => {
                    log_warn!(
                        self.logger,
                        LOG_SOURCE,
                        "window session was unreadable; starting with an empty window set"
                    );
                    WindowSession::default()
                }
            },
            Err(_) => WindowSession::default(),
        }
    }

    pub fn open(&self, request: WindowOpenRequest) -> Result<WindowOpenResponse, AppError> {
        if request.id.trim().is_empty() {
            return Err(AppError::permanent(
                "INVALID_WINDOW_ID",
                "a window id is required",
            ));
        }
        let mut inner = self.inner.lock().unwrap();
        if let Some(mut existing) = inner.windows.get(&request.id).cloned() {
            self.mark_focused(&mut inner, &request.id);
            existing.focused = true;
            inner.windows.insert(request.id.clone(), existing.clone());
            inner.focused_id = Some(request.id.clone());
            drop(inner);
            self.persist();
            return Ok(WindowOpenResponse {
                window: existing,
                focused_existing: true,
            });
        }
        let mut record = WindowRecord {
            id: request.id.clone(),
            workspace_key: None,
            roots: Vec::new(),
            geometry: request.geometry.unwrap_or_default(),
            focused: true,
            layout: request.layout.unwrap_or(Value::Null),
        };
        if !request.roots.is_empty() {
            let snapshot = self.bind_locked(&request.roots)?;
            record.workspace_key = Some(snapshot.key);
            record.roots = snapshot
                .roots
                .iter()
                .map(|root| root.path.clone())
                .collect();
        }
        self.mark_focused(&mut inner, &request.id);
        inner.windows.insert(request.id.clone(), record.clone());
        inner.focused_id = Some(request.id);
        drop(inner);
        self.persist();
        log_info!(self.logger, LOG_SOURCE, "window opened", "id" => record.id.clone());
        Ok(WindowOpenResponse {
            window: record,
            focused_existing: false,
        })
    }

    pub fn close(&self, id: &str) -> Result<WindowCloseResponse, AppError> {
        let mut inner = self.inner.lock().unwrap();
        let Some(record) = inner.windows.remove(id) else {
            let remaining = inner.windows.len() as u32;
            return Ok(WindowCloseResponse {
                closed: false,
                workspace_torn_down: false,
                remaining,
                shutdown_kernel: remaining == 0,
            });
        };
        if inner.focused_id.as_deref() == Some(id) {
            inner.focused_id = inner.windows.keys().next().cloned();
            if let Some(next) = inner.focused_id.clone() {
                self.mark_focused(&mut inner, &next);
            }
        }
        let remaining = inner.windows.len() as u32;
        drop(inner);
        let workspace_torn_down = if let Some(key) = record.workspace_key {
            self.workspace.close(&key)?
        } else {
            false
        };
        self.persist();
        log_info!(
            self.logger,
            LOG_SOURCE,
            "window closed",
            "id" => id.to_string(),
            "remaining" => remaining,
            "workspace_torn_down" => workspace_torn_down,
        );
        Ok(WindowCloseResponse {
            closed: true,
            workspace_torn_down,
            remaining,
            shutdown_kernel: remaining == 0,
        })
    }

    pub fn focus(&self, id: &str) -> Result<WindowRecord, AppError> {
        let mut inner = self.inner.lock().unwrap();
        if !inner.windows.contains_key(id) {
            return Err(AppError::permanent(
                "WINDOW_NOT_FOUND",
                format!("window '{id}' is not open"),
            ));
        }
        self.mark_focused(&mut inner, id);
        inner.focused_id = Some(id.to_string());
        let record = inner.windows.get(id).cloned().unwrap();
        drop(inner);
        self.persist();
        Ok(record)
    }

    pub fn set_geometry(
        &self,
        id: &str,
        geometry: WindowGeometry,
    ) -> Result<WindowRecord, AppError> {
        let mut inner = self.inner.lock().unwrap();
        let record = inner.windows.get_mut(id).ok_or_else(|| {
            AppError::permanent("WINDOW_NOT_FOUND", format!("window '{id}' is not open"))
        })?;
        record.geometry = geometry;
        let cloned = record.clone();
        drop(inner);
        self.persist();
        Ok(cloned)
    }

    pub fn set_layout(&self, id: &str, layout: Value) -> Result<(), AppError> {
        let mut inner = self.inner.lock().unwrap();
        let record = inner.windows.get_mut(id).ok_or_else(|| {
            AppError::permanent("WINDOW_NOT_FOUND", format!("window '{id}' is not open"))
        })?;
        record.layout = layout;
        drop(inner);
        self.persist();
        Ok(())
    }

    pub fn layout(&self, id: &str) -> Result<Value, AppError> {
        let inner = self.inner.lock().unwrap();
        inner
            .windows
            .get(id)
            .map(|record| record.layout.clone())
            .ok_or_else(|| {
                AppError::permanent("WINDOW_NOT_FOUND", format!("window '{id}' is not open"))
            })
    }

    pub fn list(&self) -> WindowListResponse {
        let inner = self.inner.lock().unwrap();
        WindowListResponse {
            windows: inner.windows.values().cloned().collect(),
            focused_id: inner.focused_id.clone(),
            restore_enabled: self.restore_enabled(),
        }
    }

    pub fn session(&self) -> WindowSession {
        let inner = self.inner.lock().unwrap();
        WindowSession {
            windows: inner.windows.values().cloned().collect(),
            focused_id: inner.focused_id.clone(),
        }
    }

    pub fn find_by_workspace(&self, roots: &[String], force_new: bool) -> Option<String> {
        if force_new || roots.is_empty() {
            return None;
        }
        let wanted: Vec<String> = roots.iter().map(|root| normalize_root(root)).collect();
        let inner = self.inner.lock().unwrap();
        inner.windows.values().find_map(|window| {
            let have: Vec<String> = window
                .roots
                .iter()
                .map(|root| normalize_root(root))
                .collect();
            if same_roots(&have, &wanted) {
                Some(window.id.clone())
            } else {
                None
            }
        })
    }

    pub fn route_notification(
        &self,
        scope: NotificationScope,
        origin: Option<&str>,
    ) -> Option<String> {
        let inner = self.inner.lock().unwrap();
        match scope {
            NotificationScope::Window => origin
                .filter(|id| inner.windows.contains_key(*id))
                .map(str::to_string),
            NotificationScope::Global => inner.focused_id.clone(),
        }
    }

    fn bind_locked(
        &self,
        roots: &[String],
    ) -> Result<helix_workspace::WorkspaceSnapshot, AppError> {
        let paths: Vec<PathBuf> = roots.iter().map(PathBuf::from).collect();
        self.workspace.open(&paths, None)
    }

    fn mark_focused(&self, inner: &mut Inner, id: &str) {
        for window in inner.windows.values_mut() {
            window.focused = window.id == id;
        }
    }

    fn persist(&self) {
        let session = self.session();
        if let Some(parent) = self.store_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(&session) {
            let _ = std::fs::write(&self.store_path, text);
        }
    }
}

fn normalize_root(path: &str) -> String {
    helix_workspace::identity::canonical_path(Path::new(path))
        .to_string_lossy()
        .to_string()
}

fn same_roots(left: &[String], right: &[String]) -> bool {
    let mut a = left.to_vec();
    let mut b = right.to_vec();
    a.sort();
    b.sort();
    a == b
}

pub fn default_session_path() -> PathBuf {
    state_root_directory()
        .unwrap_or_else(|| std::env::temp_dir().join("helix-state"))
        .join("windows.json")
}

pub fn build_service(
    config: Arc<ConfigService>,
    workspace: Arc<WorkspaceService>,
    logger: Arc<Logger>,
) -> Arc<WindowManager> {
    Arc::new(WindowManager::new(
        default_session_path(),
        config,
        workspace,
        logger,
    ))
}

pub fn register_commands(dispatcher: &mut IpcDispatcher, windows: Arc<WindowManager>) {
    let open_windows = windows.clone();
    dispatcher.register(OPEN, move |req: WindowOpenRequest, _ctx| {
        let windows = open_windows.clone();
        async move { windows.open(req) }
    });

    let close_windows = windows.clone();
    dispatcher.register(CLOSE, move |req: WindowIdRequest, _ctx| {
        let windows = close_windows.clone();
        async move { windows.close(&req.id) }
    });

    let focus_windows = windows.clone();
    dispatcher.register(FOCUS, move |req: WindowIdRequest, _ctx| {
        let windows = focus_windows.clone();
        async move { windows.focus(&req.id) }
    });

    let list_windows = windows.clone();
    dispatcher.register(LIST, move |_req: Value, _ctx| {
        let windows = list_windows.clone();
        async move { Ok::<WindowListResponse, AppError>(windows.list()) }
    });

    let session_windows = windows.clone();
    dispatcher.register(SESSION, move |_req: Value, _ctx| {
        let windows = session_windows.clone();
        async move {
            Ok::<WindowSessionResponse, AppError>(WindowSessionResponse {
                restore_enabled: windows.restore_enabled(),
                session: if windows.restore_enabled() {
                    windows.load_session()
                } else {
                    WindowSession::default()
                },
            })
        }
    });

    let find_windows = windows.clone();
    dispatcher.register(FIND, move |req: WindowFindRequest, _ctx| {
        let windows = find_windows.clone();
        async move {
            Ok::<WindowFindResponse, AppError>(WindowFindResponse {
                window_id: windows.find_by_workspace(&req.roots, req.force_new),
            })
        }
    });

    let geometry_windows = windows.clone();
    dispatcher.register(SET_GEOMETRY, move |req: WindowGeometryRequest, _ctx| {
        let windows = geometry_windows.clone();
        async move { windows.set_geometry(&req.id, req.geometry) }
    });

    let get_layout = windows.clone();
    dispatcher.register(GET_LAYOUT, move |req: WindowLayoutGetRequest, ctx| {
        let windows = get_layout.clone();
        async move {
            let id = if req.id.is_empty() {
                ctx.window_id().unwrap_or_default().to_string()
            } else {
                req.id
            };
            Ok::<WindowLayoutGetResponse, AppError>(WindowLayoutGetResponse {
                layout: windows.layout(&id)?,
            })
        }
    });

    let set_layout = windows.clone();
    dispatcher.register(SET_LAYOUT, move |req: WindowLayoutSetRequest, ctx| {
        let windows = set_layout.clone();
        async move {
            let id = if req.id.is_empty() {
                ctx.window_id().unwrap_or_default().to_string()
            } else {
                req.id
            };
            windows.set_layout(&id, req.layout)?;
            Ok::<WindowLayoutGetResponse, AppError>(WindowLayoutGetResponse {
                layout: windows.layout(&id)?,
            })
        }
    });

    let route_windows = windows.clone();
    dispatcher.register(ROUTE, move |req: WindowRouteRequest, ctx| {
        let windows = route_windows.clone();
        async move {
            let origin = req.origin_window_id.as_deref().or_else(|| ctx.window_id());
            Ok::<WindowRouteResponse, AppError>(WindowRouteResponse {
                window_id: windows.route_notification(req.scope, origin),
            })
        }
    });
}

pub fn register(
    container: &mut ServiceContainer,
    windows: Arc<WindowManager>,
    logger: Arc<Logger>,
) -> Result<(), ServiceError> {
    container.register(
        SERVICE_NAME,
        &[crate::workspace::SERVICE_NAME, crate::config::SERVICE_NAME],
        Lifetime::Singleton,
        move |_ctx| {
            Ok(
                Box::new(WindowKernelService::new(windows.clone(), logger.clone()))
                    as Box<dyn ManagedService>,
            )
        },
    )
}

struct WindowKernelService {
    windows: Arc<WindowManager>,
    logger: Arc<Logger>,
}

impl WindowKernelService {
    fn new(windows: Arc<WindowManager>, logger: Arc<Logger>) -> Self {
        Self { windows, logger }
    }
}

#[async_trait]
impl Service for WindowKernelService {
    fn name(&self) -> &'static str {
        SERVICE_NAME
    }

    fn dependencies(&self) -> &'static [&'static str] {
        &[crate::workspace::SERVICE_NAME, crate::config::SERVICE_NAME]
    }

    async fn start(&mut self, ctx: &ServiceContext) -> Result<(), ServiceError> {
        ctx.publish(self.windows.clone());
        if let Some(hub) = ctx.resolve::<StreamHub>() {
            let hub = hub.clone();
            // Session load is host-driven; publish the restored set for subscribers.
            hub.publish(
                CHANNEL,
                serde_json::to_value(self.windows.list()).unwrap_or(Value::Null),
            );
        }
        log_info!(
            self.logger,
            LOG_SOURCE,
            "window manager started",
            "restore_enabled" => self.windows.restore_enabled(),
        );
        Ok(())
    }

    async fn stop(&mut self) -> Result<(), ServiceError> {
        log_info!(self.logger, LOG_SOURCE, "window manager stopped");
        Ok(())
    }
}

impl HealthCheck for WindowKernelService {
    fn health(&self) -> ServiceHealth {
        ServiceHealth::Healthy
    }

    fn metrics(&self) -> ServiceMetrics {
        ServiceMetrics::default()
    }

    fn live_probe(&self) -> Option<ServiceProbe> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_config::{ConfigPaths, ConfigScope, SchemaRegistry};
    use helix_fs::testutil::TempDir;
    use helix_ipc::IpcRequest;
    use helix_log::LogLevel;
    use helix_workspace::WorkspaceService;

    fn logger() -> Arc<Logger> {
        Arc::new(Logger::in_memory(LogLevel::Trace))
    }

    fn harness(
        dir: &TempDir,
    ) -> (
        Arc<WindowManager>,
        Arc<ConfigService>,
        Arc<WorkspaceService>,
    ) {
        let config = Arc::new(ConfigService::load(
            ConfigPaths {
                user: Some(dir.path().join("settings.json")),
                ..ConfigPaths::default()
            },
            Arc::new(SchemaRegistry::builtin()),
            logger(),
        ));
        let fs = crate::fs::build_service(&config, logger());
        let workspace = Arc::new(WorkspaceService::with_recent_path(
            config.clone(),
            fs,
            logger(),
            Some(dir.path().join("recent.json")),
        ));
        let windows = Arc::new(WindowManager::new(
            dir.path().join("windows.json"),
            config.clone(),
            workspace.clone(),
            logger(),
        ));
        (windows, config, workspace)
    }

    #[test]
    fn per_window_layout_does_not_leak_across_windows() {
        let dir = TempDir::new("windows-layout");
        let (windows, _, _) = harness(&dir);
        windows
            .open(WindowOpenRequest {
                id: "a".into(),
                ..WindowOpenRequest::default()
            })
            .unwrap();
        windows
            .open(WindowOpenRequest {
                id: "b".into(),
                ..WindowOpenRequest::default()
            })
            .unwrap();
        windows
            .set_layout("a", serde_json::json!({ "panelSize": 300 }))
            .unwrap();
        windows
            .set_layout("b", serde_json::json!({ "panelSize": 180 }))
            .unwrap();
        assert_eq!(windows.layout("a").unwrap()["panelSize"], 300);
        assert_eq!(windows.layout("b").unwrap()["panelSize"], 180);
    }

    #[test]
    fn opening_a_restored_window_preserves_its_layout_in_the_session() {
        let dir = TempDir::new("windows-restored-layout");
        let (windows, _, _) = harness(&dir);
        let layout = serde_json::json!({
            "primarySidebarSize": 340,
            "profiles": [{ "name": "Debug" }]
        });

        let opened = windows
            .open(WindowOpenRequest {
                id: "restored".into(),
                layout: Some(layout.clone()),
                ..WindowOpenRequest::default()
            })
            .unwrap();

        assert_eq!(opened.window.layout, layout);
        assert_eq!(windows.load_session().windows[0].layout, layout);
    }

    #[test]
    fn reopening_a_window_focuses_it_and_unfocuses_the_rest() {
        let dir = TempDir::new("windows-refocus");
        let (windows, _, _) = harness(&dir);
        windows
            .open(WindowOpenRequest {
                id: "a".into(),
                ..WindowOpenRequest::default()
            })
            .unwrap();
        windows
            .open(WindowOpenRequest {
                id: "b".into(),
                ..WindowOpenRequest::default()
            })
            .unwrap();

        let reopened = windows
            .open(WindowOpenRequest {
                id: "a".into(),
                ..WindowOpenRequest::default()
            })
            .unwrap();

        assert!(reopened.focused_existing);
        assert!(reopened.window.focused);
        let listed = windows.list();
        assert_eq!(listed.focused_id.as_deref(), Some("a"));
        let by_id = |id: &str| {
            listed
                .windows
                .iter()
                .find(|window| window.id == id)
                .cloned()
                .unwrap()
        };
        assert!(by_id("a").focused);
        assert!(!by_id("b").focused);
    }

    #[test]
    fn geometry_is_scoped_per_window_and_persisted_across_a_session_reload() {
        let dir = TempDir::new("windows-geometry");
        let store = dir.path().join("windows.json");
        let project = dir.mkdir("repo");
        let roots = vec![project.to_string_lossy().into_owned()];

        // A fresh manager per "launch" against the same session file.
        let build = || {
            let config = Arc::new(ConfigService::load(
                ConfigPaths {
                    user: Some(dir.path().join("settings.json")),
                    ..ConfigPaths::default()
                },
                Arc::new(SchemaRegistry::builtin()),
                logger(),
            ));
            let fs = crate::fs::build_service(&config, logger());
            let workspace = Arc::new(WorkspaceService::with_recent_path(
                config.clone(),
                fs,
                logger(),
                Some(dir.path().join("recent.json")),
            ));
            Arc::new(WindowManager::new(
                store.clone(),
                config,
                workspace,
                logger(),
            ))
        };

        let first = build();
        first
            .open(WindowOpenRequest {
                id: "left".into(),
                roots: roots.clone(),
                geometry: None,
                layout: None,
            })
            .unwrap();
        first
            .open(WindowOpenRequest {
                id: "right".into(),
                roots: Vec::new(),
                geometry: None,
                layout: None,
            })
            .unwrap();
        first
            .set_geometry(
                "left",
                WindowGeometry {
                    x: 0,
                    y: 0,
                    width: 1600,
                    height: 900,
                    maximized: true,
                    fullscreen: false,
                    monitor: Some("Built-in Retina Display".into()),
                },
            )
            .unwrap();
        first
            .set_geometry(
                "right",
                WindowGeometry {
                    x: 100,
                    y: 100,
                    width: 800,
                    height: 600,
                    maximized: false,
                    fullscreen: true,
                    monitor: None,
                },
            )
            .unwrap();

        // Relaunch: the restored session carries each window's own geometry.
        let second = build();
        let restored = second.load_session();
        assert_eq!(restored.windows.len(), 2);
        let left = restored.windows.iter().find(|w| w.id == "left").unwrap();
        let right = restored.windows.iter().find(|w| w.id == "right").unwrap();
        assert_eq!(left.geometry.width, 1600);
        assert!(left.geometry.maximized);
        assert_eq!(
            left.geometry.monitor.as_deref(),
            Some("Built-in Retina Display")
        );
        assert_eq!(right.geometry.width, 800);
        assert!(right.geometry.fullscreen);
        assert!(!right.geometry.maximized);
    }

    #[test]
    fn closing_the_focused_window_moves_focus_to_a_survivor() {
        let dir = TempDir::new("windows-focus-transfer");
        let (windows, _, _) = harness(&dir);
        windows
            .open(WindowOpenRequest {
                id: "a".into(),
                ..WindowOpenRequest::default()
            })
            .unwrap();
        windows
            .open(WindowOpenRequest {
                id: "b".into(),
                ..WindowOpenRequest::default()
            })
            .unwrap();
        windows.focus("b").unwrap();

        let closed = windows.close("b").unwrap();

        assert!(closed.closed);
        assert_eq!(closed.remaining, 1);
        assert_eq!(windows.list().focused_id.as_deref(), Some("a"));
        assert!(windows.list().windows[0].focused);
    }

    #[test]
    fn closing_an_unknown_window_reports_without_touching_the_session() {
        let dir = TempDir::new("windows-close-unknown");
        let (windows, _, _) = harness(&dir);
        windows
            .open(WindowOpenRequest {
                id: "only".into(),
                ..WindowOpenRequest::default()
            })
            .unwrap();

        let closed = windows.close("ghost").unwrap();

        assert!(!closed.closed);
        assert!(!closed.shutdown_kernel);
        assert_eq!(closed.remaining, 1);
        assert_eq!(windows.list().windows.len(), 1);
    }

    #[test]
    fn already_open_workspace_is_found_unless_a_duplicate_is_requested() {
        let dir = TempDir::new("windows-find");
        let project = dir.mkdir("repo");
        let (windows, _, _) = harness(&dir);
        windows
            .open(WindowOpenRequest {
                id: "first".into(),
                roots: vec![project.to_string_lossy().into_owned()],
                geometry: None,
                layout: None,
            })
            .unwrap();
        assert_eq!(
            windows.find_by_workspace(&[project.to_string_lossy().into_owned()], false),
            Some("first".into())
        );
        assert_eq!(
            windows.find_by_workspace(&[project.to_string_lossy().into_owned()], true),
            None
        );
    }

    #[test]
    fn closing_one_of_two_windows_on_a_shared_workspace_keeps_the_workspace() {
        let dir = TempDir::new("windows-refcount");
        let project = dir.mkdir("repo");
        let (windows, _, workspace) = harness(&dir);
        let roots = vec![project.to_string_lossy().into_owned()];
        windows
            .open(WindowOpenRequest {
                id: "a".into(),
                roots: roots.clone(),
                geometry: None,
                layout: None,
            })
            .unwrap();
        windows
            .open(WindowOpenRequest {
                id: "b".into(),
                roots: roots.clone(),
                geometry: None,
                layout: None,
            })
            .unwrap();
        let key = windows.list().windows[0]
            .workspace_key
            .clone()
            .expect("bound");
        assert_eq!(workspace.snapshot(&key).unwrap().holders, 2);
        let closed = windows.close("a").unwrap();
        assert!(!closed.workspace_torn_down);
        assert!(!closed.shutdown_kernel);
        assert_eq!(workspace.snapshot(&key).unwrap().holders, 1);
        let last = windows.close("b").unwrap();
        assert!(last.workspace_torn_down);
        assert!(last.shutdown_kernel);
        assert!(workspace.snapshot(&key).is_none());
    }

    #[test]
    fn settings_changes_are_global_across_windows() {
        let dir = TempDir::new("windows-settings");
        let (windows, config, _) = harness(&dir);
        windows
            .open(WindowOpenRequest {
                id: "a".into(),
                ..WindowOpenRequest::default()
            })
            .unwrap();
        windows
            .open(WindowOpenRequest {
                id: "b".into(),
                ..WindowOpenRequest::default()
            })
            .unwrap();
        config
            .set(
                ConfigScope::User,
                "editor.fontSize",
                serde_json::json!(18),
                None,
            )
            .unwrap();
        let from_a = config.integer_value("editor.fontSize");
        let from_b = config.integer_value("editor.fontSize");
        assert_eq!(from_a, from_b);
        assert_eq!(from_a, Some(18));
    }

    #[test]
    fn global_notifications_go_to_the_focused_window() {
        let dir = TempDir::new("windows-notify");
        let (windows, _, _) = harness(&dir);
        windows
            .open(WindowOpenRequest {
                id: "a".into(),
                ..WindowOpenRequest::default()
            })
            .unwrap();
        windows
            .open(WindowOpenRequest {
                id: "b".into(),
                ..WindowOpenRequest::default()
            })
            .unwrap();
        windows.focus("a").unwrap();
        assert_eq!(
            windows.route_notification(NotificationScope::Global, Some("b")),
            Some("a".into())
        );
        assert_eq!(
            windows.route_notification(NotificationScope::Window, Some("b")),
            Some("b".into())
        );
    }

    #[test]
    fn corrupt_session_file_restores_an_empty_set() {
        let dir = TempDir::new("windows-corrupt");
        std::fs::write(dir.path().join("windows.json"), "{ not json").unwrap();
        let (windows, _, _) = harness(&dir);
        assert!(windows.load_session().windows.is_empty());
    }

    #[tokio::test]
    async fn ipc_open_and_close_round_trip() {
        let dir = TempDir::new("windows-ipc");
        let (windows, _, _) = harness(&dir);
        let mut dispatcher = IpcDispatcher::new();
        register_commands(&mut dispatcher, windows);
        let opened = dispatcher
            .dispatch(
                IpcRequest::new(OPEN, "w1", serde_json::json!({ "id": "main" }))
                    .with_window_id("main"),
            )
            .await;
        assert_eq!(opened.result.unwrap()["window"]["id"], "main");
        let closed = dispatcher
            .dispatch(IpcRequest::new(
                CLOSE,
                "w2",
                serde_json::json!({ "id": "main" }),
            ))
            .await;
        assert_eq!(closed.result.unwrap()["shutdown_kernel"], true);
    }
}
