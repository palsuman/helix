//! Kernel-side wiring for the configuration service (Task 1.6).
//!
//! The service itself lives in `helix-config` and knows nothing about Tauri or
//! the service container. This module mirrors [`crate::ipc`], [`crate::stream`],
//! and [`crate::log`]:
//!
//! 1. Builds the service against the platform layer paths ([`build_service`]).
//! 2. Registers the `config.*` commands the frontend calls
//!    ([`register_commands`]).
//! 3. Registers [`ConfigKernelService`] as a container-managed singleton, which
//!    publishes the service for other services to resolve, applies the logging
//!    settings, and watches the layer files for external edits, publishing each
//!    changed key set onto the streaming channel ([`register`]).
//!
//! It depends on `log` and `stream` for the same reason [`crate::log`] depends
//! on `stream`: a change notification has to reach the hub to be delivered, and
//! settings problems have to be logged with the same redaction and correlation
//! as everything else. Both dependencies are declared, so the container starts
//! them first (Task 1.2).
//!
//! The watch loop is the service's steady state. It polls rather than using an
//! OS watcher because the file system service (Task 1.7) owns watching and is
//! scheduled after this task; three kilobyte-sized files on a sub-second timer
//! costs nothing, and the propagation budget in the task's demo criterion is a
//! full second.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use helix_config::commands::{
    ConfigGetRequest, ConfigGetResponse, ConfigListRequest, ConfigListResponse, ConfigResetRequest,
    ConfigSchemaRequest, ConfigSchemaResponse, ConfigScopeInfo, ConfigSetRequest,
    ConfigWriteResponse, GET, LIST, RESET, SCHEMA, SET,
};
use helix_config::{
    CHANNEL, ConfigChange, ConfigPaths, ConfigScope, ConfigService, SchemaRegistry,
};
use helix_core::container::{
    HealthCheck, Lifetime, ManagedService, Service, ServiceContainer, ServiceContext, ServiceError,
};
use helix_core::error::AppError;
use helix_core::health::{ServiceHealth, ServiceMetrics};
use helix_ipc::IpcDispatcher;
use helix_log::{LogLevel, Logger, log_info, log_warn};
use helix_stream::StreamHub;

/// Container service name for the configuration layer.
pub const SERVICE_NAME: &str = "config";

/// Build the configuration service against the real layer locations.
///
/// Only the user layer exists at this point in the plan: the workspace and
/// folder layers arrive with the workspace manager (Task 1.8), which resolves
/// roots and will supply their paths.
pub fn build_service(logger: Arc<Logger>) -> Arc<ConfigService> {
    Arc::new(ConfigService::load(
        ConfigPaths::for_user(),
        Arc::new(SchemaRegistry::builtin()),
        logger,
    ))
}

/// Register the `config.*` commands on the kernel's dispatcher.
///
/// Each handler closes over the shared service rather than being a method on
/// the managed wrapper, so the command surface is testable without starting the
/// container.
pub fn register_commands(dispatcher: &mut IpcDispatcher, config: Arc<ConfigService>) {
    let get_config = config.clone();
    dispatcher.register(GET, move |req: ConfigGetRequest, _ctx| {
        let config = get_config.clone();
        async move {
            Ok::<ConfigGetResponse, AppError>(ConfigGetResponse {
                setting: config.get(&req.key, req.language.as_deref()),
            })
        }
    });

    let set_config = config.clone();
    dispatcher.register(SET, move |req: ConfigSetRequest, _ctx| {
        let config = set_config.clone();
        async move {
            let change = config.set(req.scope, &req.key, req.value, req.language.as_deref())?;
            Ok::<ConfigWriteResponse, AppError>(write_response(
                &config,
                change,
                &req.key,
                req.language.as_deref(),
            ))
        }
    });

    let reset_config = config.clone();
    dispatcher.register(RESET, move |req: ConfigResetRequest, _ctx| {
        let config = reset_config.clone();
        async move {
            let change = config.reset(req.scope, &req.key, req.language.as_deref())?;
            Ok::<ConfigWriteResponse, AppError>(write_response(
                &config,
                change,
                &req.key,
                req.language.as_deref(),
            ))
        }
    });

    let list_config = config.clone();
    dispatcher.register(LIST, move |req: ConfigListRequest, _ctx| {
        let config = list_config.clone();
        async move {
            Ok::<ConfigListResponse, AppError>(ConfigListResponse {
                settings: config.list(req.prefix.as_deref(), req.language.as_deref()),
                parse_errors: config.parse_errors(),
                issues: config.issues(),
                scopes: scope_info(&config),
            })
        }
    });

    dispatcher.register(SCHEMA, move |_req: ConfigSchemaRequest, _ctx| {
        let config = config.clone();
        async move {
            Ok::<ConfigSchemaResponse, AppError>(ConfigSchemaResponse {
                schema: config.schema().json_schema(),
            })
        }
    });
}

fn write_response(
    config: &ConfigService,
    change: ConfigChange,
    key: &str,
    language: Option<&str>,
) -> ConfigWriteResponse {
    ConfigWriteResponse {
        scope: change.scope,
        changed_keys: change.changed_keys,
        requires_restart: change.requires_restart,
        setting: config.get(key, language),
    }
}

fn scope_info(config: &ConfigService) -> Vec<ConfigScopeInfo> {
    ConfigScope::ASCENDING
        .into_iter()
        .filter(|scope| !scope.is_writable() || config.paths().path(*scope).is_some())
        .map(|scope| ConfigScopeInfo {
            scope,
            path: config
                .paths()
                .path(scope)
                .map(|path| path.display().to_string()),
            writable: scope.is_writable(),
        })
        .collect()
}

/// Container-managed wrapper around the configuration service.
pub struct ConfigKernelService {
    config: Arc<ConfigService>,
    logger: Arc<Logger>,
    bridged: bool,
}

impl ConfigKernelService {
    pub fn new(config: Arc<ConfigService>, logger: Arc<Logger>) -> Self {
        Self {
            config,
            logger,
            bridged: false,
        }
    }

    pub fn config(&self) -> &Arc<ConfigService> {
        &self.config
    }

    /// Apply the logging settings to the running logger.
    ///
    /// This is the first consumer of the configuration service, and it is a
    /// useful one to have first: it proves a setting reaches a live subsystem
    /// without a restart, which is REQ-CONFIG-001.8 in one function.
    fn apply_log_settings(&self) {
        if let Some(level) = self
            .config
            .string_value("log.level")
            .as_deref()
            .and_then(LogLevel::parse)
        {
            self.logger.set_default_level(level);
        }
        if let Some(modules) = self.config.value("log.moduleLevels", None).as_object() {
            for (module, value) in modules {
                match value.as_str().and_then(LogLevel::parse) {
                    Some(level) => self.logger.set_module_level(module, Some(level)),
                    None => log_warn!(
                        self.logger,
                        helix_config::LOG_SOURCE,
                        "log.moduleLevels entry ignored: not a level name",
                        "module" => module.clone(),
                    ),
                }
            }
        }
    }
}

#[async_trait]
impl Service for ConfigKernelService {
    fn name(&self) -> &'static str {
        SERVICE_NAME
    }

    fn dependencies(&self) -> &'static [&'static str] {
        // Changes publish onto the stream hub, and load-time problems are
        // reported through the logger.
        &[crate::stream::SERVICE_NAME, crate::log::SERVICE_NAME]
    }

    async fn start(&mut self, ctx: &ServiceContext) -> Result<(), ServiceError> {
        // Published so every later service (workspace, file system, LSP host,
        // AI providers) resolves the same settings rather than reading the
        // files a second time.
        ctx.publish(self.config.clone());

        if let Some(hub) = ctx.resolve::<StreamHub>() {
            let bridge_hub = hub.clone();
            self.config
                .add_listener(Arc::new(move |change: &ConfigChange| {
                    // Published unconditionally rather than only while something is
                    // subscribed: a window that reconnects mid-change would
                    // otherwise miss it, and settings changes are rare enough that
                    // the ring buffer is the right place to hold them.
                    let payload = serde_json::to_value(change).unwrap_or(serde_json::Value::Null);
                    bridge_hub.publish(CHANNEL, payload);
                }));
            self.bridged = true;
        }

        self.apply_log_settings();

        log_info!(
            self.logger,
            helix_config::LOG_SOURCE,
            "configuration service started",
            "channel" => CHANNEL,
            "watch_interval_ms" => self.config.watch_interval_ms(),
            "user_settings" => self
                .config
                .paths()
                .path(ConfigScope::User)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "none".to_string()),
        );
        Ok(())
    }

    /// Steady state: detect external edits and republish them.
    ///
    /// The interval is itself a setting, so lowering it takes effect on the
    /// next tick rather than at the next launch.
    async fn run(&mut self) -> Result<(), ServiceError> {
        loop {
            let interval = Duration::from_millis(self.config.watch_interval_ms());
            tokio::time::sleep(interval).await;

            let config = self.config.clone();
            // Blocking file reads belong off the async worker: three small
            // reads are quick, but a settings file on a slow network share is
            // not something the runtime should stall on.
            let changes = tokio::task::spawn_blocking(move || config.poll_external_changes())
                .await
                .unwrap_or_default();

            for change in changes {
                if !change.requires_restart.is_empty() {
                    log_warn!(
                        self.logger,
                        helix_config::LOG_SOURCE,
                        "some changed settings take effect only after a restart",
                        "keys" => change.requires_restart.clone(),
                    );
                }
            }
            // Reapplied after an external edit so the same code path serves
            // both a `config.set` and a hand edit of the file.
            self.apply_log_settings();
        }
    }
}

impl HealthCheck for ConfigKernelService {
    fn health(&self) -> ServiceHealth {
        let parse_errors = self.config.parse_errors();
        // A settings file that will not parse means the user's most recent
        // intent is not in effect. That is degraded service, not healthy
        // service with a warning nobody reads (REQ-OBS-004.3).
        if let Some(first) = parse_errors.first() {
            return ServiceHealth::Degraded {
                reason: format!(
                    "{} settings file(s) could not be parsed; last known good values are in effect ({first})",
                    parse_errors.len()
                ),
                since_ms: 0,
            };
        }
        let write_errors = self.config.metrics().write_errors;
        if write_errors > 0 {
            return ServiceHealth::Degraded {
                reason: format!("{write_errors} settings write(s) failed"),
                since_ms: 0,
            };
        }
        ServiceHealth::Healthy
    }

    fn metrics(&self) -> ServiceMetrics {
        let metrics = self.config.metrics();
        ServiceMetrics {
            memory_bytes: 0,
            uptime_ms: 0,
            // "Requests" for configuration is layer loads; "errors" is loads
            // and writes that did not take.
            request_count: metrics.reloads + metrics.writes,
            error_count: metrics.parse_errors + metrics.write_errors + metrics.secrets_rejected,
        }
    }
}

/// Register [`ConfigKernelService`] on a container as a supervised singleton.
pub fn register(
    container: &mut ServiceContainer,
    config: Arc<ConfigService>,
    logger: Arc<Logger>,
) -> Result<(), ServiceError> {
    container.register(
        SERVICE_NAME,
        &[crate::stream::SERVICE_NAME, crate::log::SERVICE_NAME],
        Lifetime::Singleton,
        move |_ctx| {
            Ok(
                Box::new(ConfigKernelService::new(config.clone(), logger.clone()))
                    as Box<dyn ManagedService>,
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_config::{ChangeOrigin, IssueKind};
    use helix_ipc::IpcRequest;
    use helix_stream::{ChannelSubscription, StreamFrame};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let path = std::env::temp_dir().join(format!(
                "helix-kernel-config-{label}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::SeqCst)
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn logger() -> Arc<Logger> {
        Arc::new(Logger::in_memory(LogLevel::Trace))
    }

    fn service_with(paths: ConfigPaths, logger: Arc<Logger>) -> Arc<ConfigService> {
        Arc::new(ConfigService::load(
            paths,
            Arc::new(SchemaRegistry::builtin()),
            logger,
        ))
    }

    fn dispatcher(config: Arc<ConfigService>) -> IpcDispatcher {
        let mut dispatcher = IpcDispatcher::new();
        register_commands(&mut dispatcher, config);
        dispatcher
    }

    #[tokio::test]
    async fn get_returns_the_effective_value_and_its_layer() {
        let dir = TempDir::new("get");
        let user = dir.path().join("settings.json");
        fs::write(&user, r#"{ "editor.fontSize": 19 }"#).unwrap();
        let config = service_with(
            ConfigPaths {
                user: Some(user),
                ..ConfigPaths::default()
            },
            logger(),
        );

        let response = dispatcher(config)
            .dispatch(IpcRequest::new(
                GET,
                "g1",
                serde_json::json!({ "key": "editor.fontSize" }),
            ))
            .await;

        let setting = &response.result.expect("config.get must succeed")["setting"];
        assert_eq!(setting["value"], 19);
        assert_eq!(setting["scope"], "user");
        assert_eq!(setting["is_default"], false);
        assert_eq!(setting["requires_restart"], false);
    }

    #[tokio::test]
    async fn get_of_a_language_override_returns_the_language_value() {
        let dir = TempDir::new("get-language");
        let user = dir.path().join("settings.json");
        fs::write(&user, r#"{ "[typescript].editor.tabSize": 2 }"#).unwrap();
        let config = service_with(
            ConfigPaths {
                user: Some(user),
                ..ConfigPaths::default()
            },
            logger(),
        );
        let dispatcher = dispatcher(config);

        let typescript = dispatcher
            .dispatch(IpcRequest::new(
                GET,
                "g2",
                serde_json::json!({ "key": "editor.tabSize", "language": "typescript" }),
            ))
            .await;
        assert_eq!(typescript.result.unwrap()["setting"]["value"], 2);

        let rust = dispatcher
            .dispatch(IpcRequest::new(
                GET,
                "g3",
                serde_json::json!({ "key": "editor.tabSize", "language": "rust" }),
            ))
            .await;
        assert_eq!(rust.result.unwrap()["setting"]["value"], 4);
    }

    #[tokio::test]
    async fn get_of_an_unknown_key_reports_no_setting_rather_than_failing() {
        let config = service_with(ConfigPaths::default(), logger());
        let response = dispatcher(config)
            .dispatch(IpcRequest::new(
                GET,
                "g4",
                serde_json::json!({ "key": "nothing.here" }),
            ))
            .await;
        assert!(response.result.unwrap()["setting"].is_null());
    }

    #[tokio::test]
    async fn set_writes_the_value_and_returns_the_changed_keys_and_new_state() {
        let dir = TempDir::new("set");
        let user = dir.path().join("settings.json");
        let config = service_with(
            ConfigPaths {
                user: Some(user.clone()),
                ..ConfigPaths::default()
            },
            logger(),
        );

        let response = dispatcher(config)
            .dispatch(IpcRequest::new(
                SET,
                "s1",
                serde_json::json!({ "scope": "user", "key": "editor.fontSize", "value": 21 }),
            ))
            .await;

        let result = response.result.expect("config.set must succeed");
        assert_eq!(
            result["changed_keys"],
            serde_json::json!(["editor.fontSize"])
        );
        assert_eq!(result["setting"]["value"], 21);
        assert!(fs::read_to_string(&user).unwrap().contains("21"));
    }

    #[tokio::test]
    async fn set_of_a_restart_only_setting_reports_it_in_the_response() {
        let dir = TempDir::new("set-restart");
        let config = service_with(
            ConfigPaths {
                user: Some(dir.path().join("settings.json")),
                ..ConfigPaths::default()
            },
            logger(),
        );

        let response = dispatcher(config)
            .dispatch(IpcRequest::new(
                SET,
                "s2",
                serde_json::json!({ "scope": "user", "key": "helix.locale", "value": "de" }),
            ))
            .await;

        let result = response.result.unwrap();
        assert_eq!(
            result["requires_restart"],
            serde_json::json!(["helix.locale"])
        );
        assert_eq!(result["setting"]["requires_restart"], true);
    }

    #[tokio::test]
    async fn set_of_a_credential_is_refused_with_a_typed_error() {
        let dir = TempDir::new("set-secret");
        let config = service_with(
            ConfigPaths {
                user: Some(dir.path().join("settings.json")),
                ..ConfigPaths::default()
            },
            logger(),
        );

        let response = dispatcher(config)
            .dispatch(IpcRequest::new(
                SET,
                "s3",
                serde_json::json!({
                    "scope": "user",
                    "key": "ai.password",
                    "value": "correct-horse-battery"
                }),
            ))
            .await;

        let error = response.error.expect("a credential must be refused");
        assert_eq!(error.code, "CONFIG_SECRET_REJECTED");
        assert_eq!(error.category, helix_core::error::ErrorCategory::Permanent);
        assert!(!error.message.contains("correct-horse-battery"));
    }

    #[tokio::test]
    async fn reset_returns_the_value_to_the_layer_below() {
        let dir = TempDir::new("reset");
        let user = dir.path().join("settings.json");
        fs::write(&user, r#"{ "editor.fontSize": 25 }"#).unwrap();
        let config = service_with(
            ConfigPaths {
                user: Some(user),
                ..ConfigPaths::default()
            },
            logger(),
        );

        let response = dispatcher(config)
            .dispatch(IpcRequest::new(
                RESET,
                "r1",
                serde_json::json!({ "scope": "user", "key": "editor.fontSize" }),
            ))
            .await;

        let result = response.result.unwrap();
        assert_eq!(result["setting"]["value"], 14);
        assert_eq!(result["setting"]["scope"], "default");
    }

    #[tokio::test]
    async fn list_reports_settings_parse_errors_and_the_available_scopes() {
        let dir = TempDir::new("list");
        let user = dir.path().join("settings.json");
        fs::write(
            &user,
            r#"{ "editor.fontSize": 15, "somePlugin.enabled": true }"#,
        )
        .unwrap();
        let config = service_with(
            ConfigPaths {
                user: Some(user),
                ..ConfigPaths::default()
            },
            logger(),
        );

        let response = dispatcher(config)
            .dispatch(IpcRequest::new(
                LIST,
                "l1",
                serde_json::json!({ "prefix": "editor." }),
            ))
            .await;

        let result = response.result.expect("config.list must succeed");
        let settings = result["settings"].as_array().unwrap();
        assert!(
            settings
                .iter()
                .all(|s| s["key"].as_str().unwrap().starts_with("editor."))
        );
        assert!(result["parse_errors"].as_array().unwrap().is_empty());
        let scopes = result["scopes"].as_array().unwrap();
        assert_eq!(scopes.len(), 2, "defaults and user, no workspace open");
        assert_eq!(scopes[0]["scope"], "default");
        assert_eq!(scopes[0]["writable"], false);
        assert_eq!(scopes[1]["scope"], "user");
    }

    #[tokio::test]
    async fn schema_serves_a_document_the_json_editor_can_validate_against() {
        let config = service_with(ConfigPaths::default(), logger());
        let response = dispatcher(config)
            .dispatch(IpcRequest::new(SCHEMA, "sc1", serde_json::json!({})))
            .await;

        let schema = &response.result.expect("config.schema must succeed")["schema"];
        assert_eq!(schema["properties"]["editor.fontSize"]["type"], "integer");
        assert!(schema["patternProperties"].is_object());
    }

    #[tokio::test]
    async fn the_service_publishes_itself_and_reports_health() {
        let logger = logger();
        let streaming = crate::stream::bind(
            helix_stream::HubConfig::default(),
            helix_stream::ServerConfig::default(),
        )
        .await
        .unwrap();
        let config = service_with(ConfigPaths::default(), logger.clone());

        let mut container = ServiceContainer::new();
        crate::stream::register(&mut container, streaming.clone()).unwrap();
        crate::log::register(&mut container, logger.clone()).unwrap();
        register(&mut container, config.clone(), logger).unwrap();
        container.start_all().await.unwrap();

        assert_eq!(
            container.health_summary().get(SERVICE_NAME),
            Some(&ServiceHealth::Healthy)
        );
        let resolved = container
            .context()
            .resolve::<ConfigService>()
            .expect("dependents must resolve the configuration service");
        assert!(Arc::ptr_eq(&resolved, &config));

        container.stop_all().await.unwrap();
    }

    #[tokio::test]
    async fn an_external_edit_reaches_the_frontend_over_the_streaming_channel() {
        // The Task 1.6 demo criterion: an edit to the user settings file
        // reaches the frontend, with the changed key named.
        let dir = TempDir::new("bridge");
        let user = dir.path().join("settings.json");
        fs::write(
            &user,
            r#"{ "editor.fontSize": 14, "config.watchIntervalMs": 50 }"#,
        )
        .unwrap();

        let logger = logger();
        let streaming = crate::stream::bind(
            helix_stream::HubConfig::default(),
            helix_stream::ServerConfig::default(),
        )
        .await
        .unwrap();
        let config = service_with(
            ConfigPaths {
                user: Some(user.clone()),
                ..ConfigPaths::default()
            },
            logger.clone(),
        );

        let mut container = ServiceContainer::new();
        crate::stream::register(&mut container, streaming.clone()).unwrap();
        crate::log::register(&mut container, logger.clone()).unwrap();
        register(&mut container, config.clone(), logger).unwrap();
        container.start_all().await.unwrap();

        let session = streaming.hub().open_session();
        session.subscribe(&[ChannelSubscription::new(CHANNEL)]);
        let _ = session.drain();

        fs::write(
            &user,
            r#"{ "editor.fontSize": 22, "config.watchIntervalMs": 50 }"#,
        )
        .unwrap();

        let envelope = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(frames) = session.next_frames().await {
                    if let Some(envelope) = frames.iter().find_map(|frame| match frame {
                        StreamFrame::Data(envelope) => Some(envelope.clone()),
                        _ => None,
                    }) {
                        return envelope;
                    }
                } else {
                    panic!("session closed unexpectedly");
                }
            }
        })
        .await
        .expect("a settings change must reach the channel within a second");

        assert_eq!(envelope.payload["origin"], "external");
        assert_eq!(
            envelope.payload["changed_keys"],
            serde_json::json!(["editor.fontSize"])
        );
        assert_eq!(config.get("editor.fontSize", None).unwrap().value, 22);

        container.stop_all().await.unwrap();
    }

    #[tokio::test]
    async fn the_log_level_setting_is_applied_to_the_running_logger() {
        let dir = TempDir::new("log-level");
        let user = dir.path().join("settings.json");
        fs::write(
            &user,
            r#"{ "log.level": "error", "log.moduleLevels": { "kernel.fs": "trace" } }"#,
        )
        .unwrap();

        let logger = Arc::new(Logger::in_memory(LogLevel::Info));
        let config = service_with(
            ConfigPaths {
                user: Some(user),
                ..ConfigPaths::default()
            },
            logger.clone(),
        );
        let mut service = ConfigKernelService::new(config, logger.clone());
        let ctx = ServiceContext::new();
        service.start(&ctx).await.unwrap();

        assert!(!logger.enabled(LogLevel::Info, "kernel.ipc"));
        assert!(logger.enabled(LogLevel::Error, "kernel.ipc"));
        assert!(
            logger.enabled(LogLevel::Trace, "kernel.fs"),
            "a per-module override must survive the default level change"
        );
    }

    #[tokio::test]
    async fn health_degrades_while_a_settings_file_cannot_be_parsed() {
        let dir = TempDir::new("health");
        let user = dir.path().join("settings.json");
        fs::write(&user, "{ this is not json").unwrap();
        let logger = logger();
        let config = service_with(
            ConfigPaths {
                user: Some(user),
                ..ConfigPaths::default()
            },
            logger.clone(),
        );
        let service = ConfigKernelService::new(config.clone(), logger);

        match service.health() {
            ServiceHealth::Degraded { reason, .. } => {
                assert!(reason.contains("could not be parsed"), "{reason}");
            }
            other => panic!("expected degraded health, got {other:?}"),
        }
        assert!(config.issues().iter().all(|i| i.kind != IssueKind::Secret));
        assert_eq!(config.metrics().parse_errors, 1);
    }

    #[tokio::test]
    async fn a_change_carries_its_origin_so_the_frontend_can_tell_who_moved_it() {
        let dir = TempDir::new("origin");
        let config = service_with(
            ConfigPaths {
                user: Some(dir.path().join("settings.json")),
                ..ConfigPaths::default()
            },
            logger(),
        );
        let change = config
            .set(
                ConfigScope::User,
                "editor.fontSize",
                serde_json::json!(23),
                None,
            )
            .unwrap();
        assert_eq!(change.origin, ChangeOrigin::Internal);
    }
}
