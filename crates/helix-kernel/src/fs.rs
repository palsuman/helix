//! Kernel-side wiring for the file system service (Task 1.7).
//!
//! The service itself lives in `helix-fs` and knows nothing about Tauri or the
//! service container. This module mirrors [`crate::config`], [`crate::log`],
//! [`crate::stream`], and [`crate::ipc`]:
//!
//! 1. Builds the service from the live `files.*` settings ([`build_service`]).
//! 2. Registers the `fs.*` commands the frontend calls
//!    ([`register_commands`]).
//! 3. Registers [`FsKernelService`] as a container-managed singleton, which
//!    publishes the service for other services to resolve, bridges change
//!    batches onto the streaming channel, and reports watcher health
//!    ([`register`]).
//!
//! It depends on `config`, `stream`, and `log`: exclusions and encoding defaults
//! come from settings, change notifications have to reach the hub to be
//! delivered, and watcher problems have to be logged with the same redaction and
//! correlation as everything else. All three are declared, so the container
//! starts them first (Task 1.2).
//!
//! ## Why the exclusion settings are read once per watch, not per event
//!
//! `files.exclude` and `files.watcherExclude` are compiled into a glob set when
//! a root starts being watched. A settings change after that does not retune a
//! live watch, because retuning means re-registering every OS handle for the
//! root — the watch has to be restarted, which the workspace manager (Task 1.8)
//! does when it reopens a root. Recompiling per event, the alternative, would
//! put a settings lookup in the hot path of an event storm.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use helix_config::ConfigService;
use helix_core::container::{
    HealthCheck, Lifetime, ManagedService, Service, ServiceContainer, ServiceContext, ServiceError,
};
use helix_core::error::AppError;
use helix_core::health::{ServiceHealth, ServiceMetrics};
use helix_fs::commands::{
    FsListRequest, FsListResponse, FsReadRequest, FsReadResponse, FsStatRequest, FsStatResponse,
    FsUnwatchRequest, FsUnwatchResponse, FsWatchRequest, FsWatchResponse, FsWriteRequest,
    FsWriteResponse, LIST, READ, STAT, UNWATCH, WATCH, WRITE,
};
use helix_fs::{
    CHANNEL, ChangeListener, DEFAULT_PATH_BUDGET, Encoding, ExclusionConfig, FileChange,
    FileSystemService, FsChangeNotification, FsConfig, LOG_SOURCE, LineEnding, WatchConfig,
    WriteOptions,
};
use helix_ipc::IpcDispatcher;
use helix_log::{Logger, log_info, log_warn};
use helix_stream::StreamHub;

/// Container service name for the file system layer.
pub const SERVICE_NAME: &str = "fs";

/// Assemble [`FsConfig`] from the resolved `files.*` settings.
///
/// Read through the configuration service rather than from the files directly,
/// so the same layer precedence applies here as everywhere else: a workspace can
/// widen its own exclusions without the user's settings being edited.
pub fn fs_config_from(config: &ConfigService) -> FsConfig {
    let mut globs = enabled_globs(config, "files.exclude");
    globs.extend(enabled_globs(config, "files.watcherExclude"));
    if globs.is_empty() {
        // An empty object in settings means "exclude nothing", which is a
        // legitimate request. Only a *missing* setting falls back to the
        // built-in list, and the schema always supplies one, so this is the
        // defaults-only path.
        globs = ExclusionConfig::default().globs;
    }

    let max_depth = match config.integer_value("files.watchDepth").unwrap_or(0) {
        depth if depth > 0 => Some(depth as usize),
        // 0 is the documented "no limit", not a limit of zero.
        _ => None,
    };

    FsConfig {
        watch: WatchConfig {
            path_budget: DEFAULT_PATH_BUDGET,
            exclusions: ExclusionConfig {
                globs,
                respect_gitignore: config.bool_value("files.respectGitignore").unwrap_or(true),
                max_depth,
            },
            ..WatchConfig::default()
        },
        default_encoding: config
            .string_value("files.encoding")
            .as_deref()
            .and_then(Encoding::parse)
            .unwrap_or(Encoding::Utf8),
        // `auto` parses to `None`, which means "keep what the file has".
        default_eol: config
            .string_value("files.eol")
            .as_deref()
            .and_then(LineEnding::parse),
        ..FsConfig::default()
    }
}

/// Glob keys of an exclusion object whose value is `true`.
///
/// The object form (`{ "**/target": true }`) rather than an array is what makes
/// layered exclusions work: a workspace can set `"**/target": false` to
/// re-include something the user excluded, which an array could only do by
/// replacing the whole list (REQ-CONFIG-001.1 merge semantics).
fn enabled_globs(config: &ConfigService, key: &str) -> Vec<String> {
    config
        .value(key, None)
        .as_object()
        .map(|map| {
            map.iter()
                .filter(|(_, enabled)| enabled.as_bool().unwrap_or(false))
                .map(|(glob, _)| glob.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// Build the file system service from the live configuration.
pub fn build_service(config: &ConfigService, logger: Arc<Logger>) -> Arc<FileSystemService> {
    Arc::new(FileSystemService::new(fs_config_from(config), logger))
}

/// Register the `fs.*` commands on the kernel's dispatcher.
///
/// Each handler closes over the shared service rather than being a method on the
/// managed wrapper, so the command surface is testable without starting the
/// container.
///
/// The file work runs on `spawn_blocking`. Reads, writes, and directory walks
/// are blocking syscalls that can take arbitrarily long on a network share, and
/// running them on an async worker would stall every other IPC command sharing
/// that worker — which is exactly the 5ms p95 budget in REQ-NFR-001 being missed
/// for reasons unrelated to the request that missed it.
pub fn register_commands(dispatcher: &mut IpcDispatcher, fs: Arc<FileSystemService>) {
    let read_fs = fs.clone();
    dispatcher.register(READ, move |req: FsReadRequest, _ctx| {
        let fs = read_fs.clone();
        async move {
            let content = blocking(move || fs.read(&req.path)).await?;
            Ok::<FsReadResponse, AppError>(FsReadResponse { content })
        }
    });

    let write_fs = fs.clone();
    dispatcher.register(WRITE, move |req: FsWriteRequest, _ctx| {
        let fs = write_fs.clone();
        async move {
            let outcome = blocking(move || {
                let mut options = WriteOptions::new(req.text);
                options.encoding = req.encoding;
                options.eol = req.eol;
                options.expected_hash = req.expected_hash;
                fs.write(&req.path, options)
            })
            .await?;
            Ok::<FsWriteResponse, AppError>(FsWriteResponse { outcome })
        }
    });

    let list_fs = fs.clone();
    dispatcher.register(LIST, move |req: FsListRequest, _ctx| {
        let fs = list_fs.clone();
        async move {
            let listing = blocking(move || fs.list(&req.path, req.recursive)).await?;
            Ok::<FsListResponse, AppError>(FsListResponse { listing })
        }
    });

    let stat_fs = fs.clone();
    dispatcher.register(STAT, move |req: FsStatRequest, _ctx| {
        let fs = stat_fs.clone();
        async move {
            let entry = blocking(move || fs.stat(&req.path)).await?;
            Ok::<FsStatResponse, AppError>(FsStatResponse { entry })
        }
    });

    let watch_fs = fs.clone();
    dispatcher.register(WATCH, move |req: FsWatchRequest, _ctx| {
        let fs = watch_fs.clone();
        async move {
            // Starting a watch scans the tree to count watched paths, so this
            // is blocking work too, and on a large monorepo it is the slowest
            // thing in this module.
            let report = blocking(move || fs.watch(&req.root)).await?;
            Ok::<FsWatchResponse, AppError>(FsWatchResponse { report })
        }
    });

    dispatcher.register(UNWATCH, move |req: FsUnwatchRequest, _ctx| {
        let fs = fs.clone();
        async move {
            fs.unwatch(&req.root)?;
            Ok::<FsUnwatchResponse, AppError>(FsUnwatchResponse { stopped: true })
        }
    });
}

/// Run blocking file work off the async worker pool.
///
/// A panic inside the closure surfaces as a typed error rather than taking the
/// runtime worker with it; the container's supervision covers a service that
/// panics, but a single malformed request should not need supervision to
/// recover from.
async fn blocking<T, F>(work: F) -> Result<T, AppError>
where
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(result) => result,
        Err(join_error) => Err(AppError::transient(
            "FS_TASK_FAILED",
            format!("the file system task did not complete: {join_error}"),
        )),
    }
}

/// Container-managed wrapper around the file system service.
pub struct FsKernelService {
    fs: Arc<FileSystemService>,
    logger: Arc<Logger>,
}

impl FsKernelService {
    pub fn new(fs: Arc<FileSystemService>, logger: Arc<Logger>) -> Self {
        Self { fs, logger }
    }

    pub fn fs(&self) -> &Arc<FileSystemService> {
        &self.fs
    }
}

#[async_trait]
impl Service for FsKernelService {
    fn name(&self) -> &'static str {
        SERVICE_NAME
    }

    fn dependencies(&self) -> &'static [&'static str] {
        // Exclusions and encoding defaults come from settings, change batches
        // publish onto the stream hub, and watcher problems are logged.
        &[
            crate::config::SERVICE_NAME,
            crate::stream::SERVICE_NAME,
            crate::log::SERVICE_NAME,
        ]
    }

    async fn start(&mut self, ctx: &ServiceContext) -> Result<(), ServiceError> {
        // Published so every later service (workspace, search index, git, LSP
        // host) reads and writes through this one rather than touching disk
        // directly.
        ctx.publish(self.fs.clone());

        if let Some(hub) = ctx.resolve::<StreamHub>() {
            let bridge_hub = hub.clone();
            let listener: ChangeListener = Arc::new(move |changes: &[FileChange]| {
                // One frame per debounced batch, not per change: a
                // `git checkout` is thousands of changes and a frame each would
                // flood the channel it is trying to inform.
                let payload = serde_json::to_value(FsChangeNotification {
                    changes: changes.to_vec(),
                })
                .unwrap_or(serde_json::Value::Null);
                bridge_hub.publish(CHANNEL, payload);
            });
            self.fs.add_listener(listener);
        }

        let config = self.fs.config();
        log_info!(
            self.logger,
            LOG_SOURCE,
            "file system service started",
            "channel" => CHANNEL,
            "path_budget" => config.watch.path_budget,
            "debounce_ms" => config.watch.debounce.as_millis() as u64,
            "respect_gitignore" => config.watch.exclusions.respect_gitignore,
            "exclusions" => config.watch.exclusions.globs.len(),
            "default_encoding" => config.default_encoding.as_str(),
        );
        Ok(())
    }

    /// Steady state: publish watcher metrics to the log at a low frequency.
    ///
    /// Health and metrics are pulled on demand by [`HealthCheck`], so this loop
    /// exists only to leave a periodic trace of watcher load in the log, which
    /// is what makes a "the IDE got slow an hour ago" report diagnosable after
    /// the fact. It logs only when there is something to say.
    async fn run(&mut self) -> Result<(), ServiceError> {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let stats = self.fs.watch_stats();
            if stats.roots == 0 {
                continue;
            }
            log_info!(
                self.logger,
                LOG_SOURCE,
                "watcher status",
                "roots" => stats.roots,
                "watched_paths" => stats.watched_paths,
                "events_per_second" => stats.events_per_second,
                "changes_emitted" => stats.changes_emitted,
                "dropped_as_noise" => stats.dropped_as_noise,
                "polling_roots" => stats.polling_roots,
            );
            for report in self.fs.watched_roots() {
                if report.over_budget {
                    log_warn!(
                        self.logger,
                        LOG_SOURCE,
                        "a watched root remains over its path budget",
                        "root" => report.root.clone(),
                        "watched_paths" => report.watched_paths,
                        "suggested_exclusions" => report.suggested_exclusions.clone(),
                    );
                }
            }
        }
    }

    async fn stop(&mut self) -> Result<(), ServiceError> {
        // Releasing the OS registrations on shutdown rather than leaving them to
        // process teardown (REQ-ARCH-002.4).
        for report in self.fs.watched_roots() {
            let _ = self.fs.unwatch(&report.root);
        }
        Ok(())
    }
}

impl HealthCheck for FsKernelService {
    fn health(&self) -> ServiceHealth {
        let metrics = self.fs.metrics();

        // A root that fell back to polling is working, but slower and with a
        // 5s worst-case delay the user will notice. That is degraded service,
        // not healthy service with a warning nobody reads (REQ-OBS-004.3).
        if metrics.watch.polling_roots > 0 {
            return ServiceHealth::Degraded {
                reason: format!(
                    "{} watched root(s) are being polled rather than watched natively; \
                     external changes may take up to {}s to appear",
                    metrics.watch.polling_roots,
                    helix_fs::POLL_INTERVAL.as_secs()
                ),
                since_ms: 0,
            };
        }
        if metrics.watch.over_budget_roots > 0 {
            return ServiceHealth::Degraded {
                reason: format!(
                    "{} watched root(s) exceed the {} path budget; add exclusions to restore \
                     full performance",
                    metrics.watch.over_budget_roots, DEFAULT_PATH_BUDGET
                ),
                since_ms: 0,
            };
        }
        if metrics.write_errors > 0 {
            return ServiceHealth::Degraded {
                reason: format!("{} file write(s) failed", metrics.write_errors),
                since_ms: 0,
            };
        }
        ServiceHealth::Healthy
    }

    fn metrics(&self) -> ServiceMetrics {
        let metrics = self.fs.metrics();
        ServiceMetrics {
            memory_bytes: 0,
            uptime_ms: 0,
            // "Requests" for the file system is work done on behalf of a
            // caller, plus the watcher events it delivered, so the watched-path
            // count and event rate reach health monitoring through the same
            // channel as everything else (REQ-FS-004.8, REQ-OBS-004.1).
            request_count: metrics.reads
                + metrics.writes
                + metrics.listings
                + metrics.watch.changes_emitted,
            error_count: metrics.read_errors + metrics.write_errors + metrics.conflicts,
        }
    }
}

/// Register [`FsKernelService`] on a container as a supervised singleton.
pub fn register(
    container: &mut ServiceContainer,
    fs: Arc<FileSystemService>,
    logger: Arc<Logger>,
) -> Result<(), ServiceError> {
    container.register(
        SERVICE_NAME,
        &[
            crate::config::SERVICE_NAME,
            crate::stream::SERVICE_NAME,
            crate::log::SERVICE_NAME,
        ],
        Lifetime::Singleton,
        move |_ctx| {
            Ok(Box::new(FsKernelService::new(fs.clone(), logger.clone()))
                as Box<dyn ManagedService>)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_config::{ConfigPaths, ConfigScope, SchemaRegistry};
    use helix_fs::testutil::TempDir;
    use helix_ipc::IpcRequest;
    use helix_log::LogLevel;
    use helix_stream::{ChannelSubscription, StreamFrame};
    use std::fs;

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

    fn config_with(user_settings: &str, dir: &TempDir, logger: Arc<Logger>) -> Arc<ConfigService> {
        let path = dir.write("settings.json", user_settings);
        Arc::new(ConfigService::load(
            ConfigPaths {
                user: Some(path),
                ..ConfigPaths::default()
            },
            Arc::new(SchemaRegistry::builtin()),
            logger,
        ))
    }

    fn service(logger: Arc<Logger>) -> Arc<FileSystemService> {
        build_service(&defaults_only_config(logger.clone()), logger)
    }

    fn dispatcher(fs: Arc<FileSystemService>) -> IpcDispatcher {
        let mut dispatcher = IpcDispatcher::new();
        register_commands(&mut dispatcher, fs);
        dispatcher
    }

    #[tokio::test]
    async fn read_returns_the_content_with_its_encoding_and_eol() {
        let dir = TempDir::new("kernel-fs-read");
        let path = dir.write("main.rs", "fn main() {}\r\n");

        let response = dispatcher(service(logger()))
            .dispatch(IpcRequest::new(
                READ,
                "r1",
                serde_json::json!({ "path": path.to_string_lossy() }),
            ))
            .await;

        let content = &response.result.expect("fs.read must succeed")["content"];
        assert_eq!(content["encoding"], "utf8");
        assert_eq!(content["eol"]["style"], "crlf");
        assert_eq!(content["binary"], false);
        assert_eq!(content["text"], "fn main() {}\n");
    }

    #[tokio::test]
    async fn read_of_a_binary_file_reports_it_without_text() {
        let dir = TempDir::new("kernel-fs-binary");
        let path = dir.write("icon.png", b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR");

        let response = dispatcher(service(logger()))
            .dispatch(IpcRequest::new(
                READ,
                "r2",
                serde_json::json!({ "path": path.to_string_lossy() }),
            ))
            .await;

        let content = &response.result.unwrap()["content"];
        assert_eq!(content["binary"], true);
        assert!(content["text"].is_null());
    }

    #[tokio::test]
    async fn read_of_a_missing_file_is_a_typed_error() {
        let dir = TempDir::new("kernel-fs-missing");
        let response = dispatcher(service(logger()))
            .dispatch(IpcRequest::new(
                READ,
                "r3",
                serde_json::json!({ "path": dir.path().join("nope.rs").to_string_lossy() }),
            ))
            .await;

        let error = response.error.expect("a missing file must error");
        assert_eq!(error.code, "FS_NOT_FOUND");
        assert_eq!(error.category, helix_core::error::ErrorCategory::Permanent);
    }

    #[tokio::test]
    async fn write_lands_atomically_and_returns_the_new_hash() {
        let dir = TempDir::new("kernel-fs-write");
        let path = dir.path().join("new.rs");

        let response = dispatcher(service(logger()))
            .dispatch(IpcRequest::new(
                WRITE,
                "w1",
                serde_json::json!({
                    "path": path.to_string_lossy(),
                    "text": "fn main() {}\n",
                    "eol": "lf",
                }),
            ))
            .await;

        let outcome = &response.result.expect("fs.write must succeed")["outcome"];
        assert_eq!(outcome["bytes_written"], 13);
        assert_eq!(outcome["eol"], "lf");
        assert_eq!(fs::read_to_string(&path).unwrap(), "fn main() {}\n");
        assert_eq!(
            outcome["hash"],
            helix_fs::hash_bytes(b"fn main() {}\n").to_string()
        );
    }

    #[tokio::test]
    async fn write_with_a_stale_expected_hash_is_refused() {
        let dir = TempDir::new("kernel-fs-conflict");
        let path = dir.write("main.rs", "on disk\n");

        let response = dispatcher(service(logger()))
            .dispatch(IpcRequest::new(
                WRITE,
                "w2",
                serde_json::json!({
                    "path": path.to_string_lossy(),
                    "text": "from the buffer\n",
                    "expected_hash": "0000000000000000",
                }),
            ))
            .await;

        let error = response.error.expect("a stale write must be refused");
        assert_eq!(error.code, "FS_WRITE_CONFLICT");
        assert_eq!(fs::read_to_string(&path).unwrap(), "on disk\n");
    }

    #[tokio::test]
    async fn list_and_stat_answer_over_ipc() {
        let dir = TempDir::new("kernel-fs-list");
        dir.write("src/main.rs", "fn main() {}\n");
        dir.write("node_modules/pkg/index.js", "noise\n");
        let dispatcher = dispatcher(service(logger()));

        let listing = dispatcher
            .dispatch(IpcRequest::new(
                LIST,
                "l1",
                serde_json::json!({ "path": dir.path().to_string_lossy(), "recursive": true }),
            ))
            .await
            .result
            .expect("fs.list must succeed");
        let paths: Vec<String> = listing["listing"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["relative_path"].as_str().unwrap().to_string())
            .collect();
        assert!(paths.contains(&"src/main.rs".to_string()), "{paths:?}");
        assert!(!paths.iter().any(|path| path.contains("node_modules")));

        let stat = dispatcher
            .dispatch(IpcRequest::new(
                STAT,
                "s1",
                serde_json::json!({ "path": dir.path().join("src/main.rs").to_string_lossy() }),
            ))
            .await
            .result
            .expect("fs.stat must succeed");
        assert_eq!(stat["entry"]["name"], "main.rs");
        assert_eq!(stat["entry"]["is_dir"], false);
    }

    #[tokio::test]
    async fn watch_reports_the_budget_verdict_and_unwatch_stops_it() {
        let dir = TempDir::new("kernel-fs-watch");
        dir.write("src/main.rs", "fn main() {}\n");
        let dispatcher = dispatcher(service(logger()));

        let watched = dispatcher
            .dispatch(IpcRequest::new(
                WATCH,
                "wa1",
                serde_json::json!({ "root": dir.path().to_string_lossy() }),
            ))
            .await
            .result
            .expect("fs.watch must succeed");
        assert_eq!(watched["report"]["over_budget"], false);
        assert_eq!(watched["report"]["mode"], "native");

        let stopped = dispatcher
            .dispatch(IpcRequest::new(
                UNWATCH,
                "wa2",
                serde_json::json!({ "root": dir.path().to_string_lossy() }),
            ))
            .await
            .result
            .expect("fs.unwatch must succeed");
        assert_eq!(stopped["stopped"], true);
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
        let config = defaults_only_config(logger.clone());
        let fs = build_service(&config, logger.clone());

        let mut container = ServiceContainer::new();
        crate::stream::register(&mut container, streaming.clone()).unwrap();
        crate::log::register(&mut container, logger.clone()).unwrap();
        crate::config::register(&mut container, config, logger.clone()).unwrap();
        register(&mut container, fs.clone(), logger).unwrap();
        container.start_all().await.unwrap();

        assert_eq!(
            container.health_summary().get(SERVICE_NAME),
            Some(&ServiceHealth::Healthy)
        );
        let resolved = container
            .context()
            .resolve::<FileSystemService>()
            .expect("dependents must resolve the file system service");
        assert!(Arc::ptr_eq(&resolved, &fs));

        container.stop_all().await.unwrap();
    }

    #[tokio::test]
    async fn an_external_change_reaches_the_frontend_over_the_streaming_channel() {
        // The Task 1.7 demo criterion, end to end through the kernel's wiring:
        // a change made outside the application reaches the frontend.
        let dir = TempDir::new("kernel-fs-bridge");
        let logger = logger();
        let streaming = crate::stream::bind(
            helix_stream::HubConfig::default(),
            helix_stream::ServerConfig::default(),
        )
        .await
        .unwrap();
        let config = defaults_only_config(logger.clone());
        let fs = build_service(&config, logger.clone());

        let mut container = ServiceContainer::new();
        crate::stream::register(&mut container, streaming.clone()).unwrap();
        crate::log::register(&mut container, logger.clone()).unwrap();
        crate::config::register(&mut container, config, logger.clone()).unwrap();
        register(&mut container, fs.clone(), logger).unwrap();
        container.start_all().await.unwrap();

        let session = streaming.hub().open_session();
        session.subscribe(&[ChannelSubscription::new(CHANNEL)]);
        let _ = session.drain();
        fs.watch(dir.path()).unwrap();

        fs::write(dir.path().join("external.rs"), "fn main() {}\n").unwrap();

        let envelope = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let Some(frames) = session.next_frames().await else {
                    panic!("session closed unexpectedly");
                };
                for frame in frames {
                    if let StreamFrame::Data(envelope) = frame
                        && envelope.payload["changes"]
                            .as_array()
                            .is_some_and(|changes| {
                                changes.iter().any(|change| {
                                    change["path"]
                                        .as_str()
                                        .is_some_and(|path| path.ends_with("external.rs"))
                                })
                            })
                    {
                        return envelope;
                    }
                }
            }
        })
        .await
        .expect("an external change must reach the channel");

        let change = &envelope.payload["changes"][0];
        assert_ne!(change["kind"], "deleted");
        assert_eq!(change["is_dir"], false);

        container.stop_all().await.unwrap();
    }

    #[test]
    fn the_configuration_maps_the_files_settings_onto_the_service() {
        let dir = TempDir::new("kernel-fs-config");
        let logger = logger();
        let config = config_with(
            r#"{
                "files.exclude": { "**/vendor": true, "**/target": false },
                "files.watcherExclude": { "**/.cache/**": true },
                "files.watchDepth": 4,
                "files.respectGitignore": false,
                "files.encoding": "utf16_le",
                "files.eol": "crlf"
            }"#,
            &dir,
            logger,
        );

        let fs_config = fs_config_from(&config);
        let globs = &fs_config.watch.exclusions.globs;
        assert!(globs.contains(&"**/vendor".to_string()));
        assert!(globs.contains(&"**/.cache/**".to_string()));
        assert!(
            !globs.contains(&"**/target".to_string()),
            "a pattern set to false must be re-included, which is what the object form is for"
        );
        assert_eq!(fs_config.watch.exclusions.max_depth, Some(4));
        assert!(!fs_config.watch.exclusions.respect_gitignore);
        assert_eq!(fs_config.default_encoding, Encoding::Utf16Le);
        assert_eq!(fs_config.default_eol, Some(LineEnding::Crlf));
    }

    #[test]
    fn the_default_configuration_excludes_the_usual_directories_and_keeps_eol_auto() {
        let config = defaults_only_config(logger());
        let fs_config = fs_config_from(&config);

        assert!(
            fs_config
                .watch
                .exclusions
                .globs
                .iter()
                .any(|glob| glob.contains("node_modules"))
        );
        assert!(fs_config.watch.exclusions.respect_gitignore);
        assert_eq!(
            fs_config.watch.exclusions.max_depth, None,
            "files.watchDepth 0 means unlimited, not a depth of zero"
        );
        assert_eq!(
            fs_config.default_eol, None,
            "`auto` must mean 'keep the file's own style'"
        );
    }

    #[test]
    fn health_degrades_while_a_root_is_polled_rather_than_watched() {
        let dir = TempDir::new("kernel-fs-health");
        let logger = logger();
        let mut fs_config = fs_config_from(&defaults_only_config(logger.clone()));
        // Forces the polling decision without needing a real network share.
        fs_config.watch.latency_threshold = Duration::from_nanos(1);
        let fs = Arc::new(FileSystemService::new(fs_config, logger.clone()));
        fs.watch(dir.path()).unwrap();

        let service = FsKernelService::new(fs, logger);
        match service.health() {
            ServiceHealth::Degraded { reason, .. } => {
                assert!(reason.contains("polled"), "{reason}");
            }
            other => panic!("expected degraded health, got {other:?}"),
        }
    }

    #[test]
    fn metrics_carry_the_watcher_counts_into_the_container_shape() {
        let dir = TempDir::new("kernel-fs-metrics");
        let path = dir.write("main.rs", "fn main() {}\n");
        let logger = logger();
        let fs = service(logger.clone());
        fs.read(&path).unwrap();
        fs.list(dir.path(), false).unwrap();

        let metrics = FsKernelService::new(fs, logger).metrics();
        assert_eq!(metrics.request_count, 2);
        assert_eq!(metrics.error_count, 0);
    }

    #[test]
    fn the_settings_layer_is_read_through_the_configuration_service_not_the_files() {
        // Guards against a regression to reading ~/.helix/settings.json
        // directly, which would bypass workspace and folder precedence.
        let dir = TempDir::new("kernel-fs-scope");
        let logger = logger();
        let config = config_with(r#"{ "files.watchDepth": 2 }"#, &dir, logger.clone());
        assert_eq!(
            config.get("files.watchDepth", None).unwrap().scope,
            ConfigScope::User
        );
        assert_eq!(fs_config_from(&config).watch.exclusions.max_depth, Some(2));
    }
}
