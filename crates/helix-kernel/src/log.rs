//! Kernel-side wiring for structured logging (Task 1.5).
//!
//! The aggregator itself lives in `helix-log` and knows nothing about Tauri or
//! the service container. This module mirrors [`crate::ipc`] and
//! [`crate::stream`]:
//!
//! 1. Builds the process logger, including the platform log directory
//!    ([`build_logger`]).
//! 2. Registers the `log.*` commands the viewer calls
//!    ([`register_commands`]).
//! 3. Registers [`LogService`] as a container-managed singleton, which
//!    publishes the logger for other services to resolve and bridges every
//!    record onto the streaming channel the viewer tails
//!    ([`register`]).
//!
//! The bridge is why this service depends on `stream`: a record has to reach
//! the hub to be tailed, and the hub belongs to the streaming service. The
//! dependency is declared rather than assumed, so the container starts them in
//! the right order (Task 1.2).

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use helix_core::container::{
    HealthCheck, Lifetime, ManagedService, Service, ServiceContainer, ServiceContext, ServiceError,
};
use helix_core::error::AppError;
use helix_core::health::{ServiceHealth, ServiceMetrics};
use helix_ipc::IpcDispatcher;
use helix_log::commands::{
    APPEND, CHANNEL, EXPORT, LEVELS, LogAppendRequest, LogAppendResponse, LogExportRequest,
    LogExportResponse, LogLevelsRequest, LogLevelsResponse, LogQueryRequest, LogQueryResponse,
    LogSetLevelRequest, QUERY, SET_LEVEL,
};
use helix_log::{LogLevel, LogRecord, Logger, LoggerConfig, log_info};
use helix_stream::StreamHub;

/// Container service name for the logging layer.
pub const SERVICE_NAME: &str = "log";

/// Source name used for the kernel's own lifecycle records.
pub const KERNEL_SOURCE: &str = "kernel.log";

/// Namespace every frontend-supplied record is filed under, so a renderer
/// cannot post records as a kernel service.
pub const FRONTEND_NAMESPACE: &str = "frontend";

/// Build the process logger.
///
/// The log directory follows the design document's Storage Locations table:
/// the OS state directory under a `Helix` (or `helix`) application folder.
/// When the platform directory cannot be determined the logger still runs
/// with its in-memory and panel sinks, because losing the file must not stop
/// the application from starting.
pub fn build_logger(default_level: LogLevel) -> Arc<Logger> {
    let mut config = LoggerConfig::default().with_default_level(default_level);
    if let Some(directory) = default_log_directory() {
        config = config.with_directory(directory);
    }
    // Mirrored to stdout only for a CLI/dev launch (REQ-OBS-001.7); a
    // packaged windowed build has nowhere for it to go.
    config = config.with_stdout(cfg!(debug_assertions));
    Arc::new(Logger::new(config))
}

/// `<state dir>/logs`, per the design document's Storage Locations.
///
/// Resolved from environment variables rather than a directories crate: three
/// lookups against documented variables is a smaller surface than another
/// pinned dependency, and the fallback path is the same one the supervisor
/// (Task 1.11) will need.
pub fn default_log_directory() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|p| p.join("Helix").join("state"))
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(PathBuf::from).map(|p| {
            p.join("Library")
                .join("Application Support")
                .join("Helix")
                .join("state")
        })
    } else {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|p| p.join(".local").join("state"))
            })
            .map(|p| p.join("helix").join("state"))
    };
    base.map(|p| p.join("logs"))
}

/// Register the `log.*` commands on the kernel's dispatcher.
///
/// Each handler is a closure over the shared logger rather than a method on
/// the service, so the command surface is testable without starting the
/// container.
pub fn register_commands(dispatcher: &mut IpcDispatcher, logger: Arc<Logger>) {
    let query_logger = logger.clone();
    dispatcher.register(QUERY, move |req: LogQueryRequest, _ctx| {
        let logger = query_logger.clone();
        async move {
            let result = logger.query(&req.query);
            Ok::<LogQueryResponse, AppError>(LogQueryResponse {
                entries: result.entries,
                matched: result.matched as u32,
                ring_len: result.ring_len as u32,
                ring_capacity: result.ring_capacity as u32,
                evicted: result.evicted,
                sources: logger.sources(),
            })
        }
    });

    let export_logger = logger.clone();
    dispatcher.register(EXPORT, move |req: LogExportRequest, _ctx| {
        let logger = export_logger.clone();
        async move {
            let (content, entry_count) = logger.export(&req.query);
            Ok::<LogExportResponse, AppError>(LogExportResponse {
                format: "jsonl".to_string(),
                content,
                entry_count: entry_count as u32,
                suggested_file_name: format!(
                    "helix-log-{}.jsonl",
                    helix_log::time::now_rfc3339_millis().replace([':', '.'], "-")
                ),
            })
        }
    });

    let append_logger = logger.clone();
    dispatcher.register(APPEND, move |req: LogAppendRequest, _ctx| {
        let logger = append_logger.clone();
        async move {
            let source = namespaced_frontend_source(&req.source);
            let recorded = logger.enabled(req.level, &source);
            if recorded {
                let mut record = match req.ts.as_deref().filter(|ts| is_kernel_timestamp(ts)) {
                    Some(ts) => LogRecord::at(ts, req.level, source.clone(), req.message),
                    // A missing or unusable client timestamp is replaced with
                    // the kernel's, so the viewer cannot be reordered by a
                    // skewed clock in the renderer.
                    None => LogRecord::new(req.level, source.clone(), req.message),
                };
                record.fields = req.fields;
                record.correlation_id = req.correlation_id;
                logger.log(record);
            }
            Ok::<LogAppendResponse, AppError>(LogAppendResponse { recorded, source })
        }
    });

    let levels_logger = logger.clone();
    dispatcher.register(LEVELS, move |_req: LogLevelsRequest, _ctx| {
        let logger = levels_logger.clone();
        async move {
            Ok::<LogLevelsResponse, AppError>(LogLevelsResponse {
                levels: logger.levels(),
            })
        }
    });

    dispatcher.register(SET_LEVEL, move |req: LogSetLevelRequest, _ctx| {
        let logger = logger.clone();
        async move {
            match (req.module, req.level) {
                (Some(module), level) => logger.set_module_level(&module, level),
                (None, Some(level)) => logger.set_default_level(level),
                (None, None) => {
                    return Err(AppError::permanent(
                        "INVALID_PAYLOAD",
                        "log.set_level needs a module to clear or a level to apply",
                    ));
                }
            }
            Ok(LogLevelsResponse {
                levels: logger.levels(),
            })
        }
    });
}

/// Container-managed wrapper around the process logger.
pub struct LogService {
    logger: Arc<Logger>,
    bridged: bool,
}

impl LogService {
    pub fn new(logger: Arc<Logger>) -> Self {
        Self {
            logger,
            bridged: false,
        }
    }

    pub fn logger(&self) -> &Arc<Logger> {
        &self.logger
    }
}

#[async_trait]
impl Service for LogService {
    fn name(&self) -> &'static str {
        SERVICE_NAME
    }

    fn dependencies(&self) -> &'static [&'static str] {
        // The live tail publishes onto the stream hub, which the streaming
        // service owns and publishes into the context.
        &[crate::stream::SERVICE_NAME]
    }

    async fn start(&mut self, ctx: &ServiceContext) -> Result<(), ServiceError> {
        // Published so every later service resolves the same logger rather
        // than building its own and fragmenting the stream.
        ctx.publish(self.logger.clone());

        if let Some(hub) = ctx.resolve::<StreamHub>() {
            let logger = self.logger.clone();
            let bridge_hub = hub.clone();
            self.logger.add_sink(Arc::new(move |record: &LogRecord| {
                // Nothing is subscribed until the viewer is open, so the
                // common case costs one atomic-ish counter read rather
                // than a serialization (REQ-OBS-001.7: the panel is a
                // sink only while it exists).
                if bridge_hub.subscriber_count(CHANNEL) == 0 {
                    return;
                }
                let payload = serde_json::to_value(record).unwrap_or(serde_json::Value::Null);
                match &record.correlation_id {
                    Some(correlation_id) => {
                        bridge_hub.publish_correlated(CHANNEL, payload, correlation_id.clone());
                    }
                    None => {
                        bridge_hub.publish(CHANNEL, payload);
                    }
                }
            }));
            self.bridged = true;
            log_info!(
                logger,
                KERNEL_SOURCE,
                "logging started",
                "channel" => CHANNEL,
                "ring_capacity" => logger.metrics().ring_capacity,
                "file" => logger
                    .file_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "none".to_string()),
            );
        }

        Ok(())
    }

    async fn stop(&mut self) -> Result<(), ServiceError> {
        log_info!(self.logger, KERNEL_SOURCE, "logging stopped");
        // The file sink flushes per record, so this only covers a partial
        // buffered write; cheap insurance at shutdown either way.
        self.logger.flush();
        Ok(())
    }
}

impl HealthCheck for LogService {
    fn health(&self) -> ServiceHealth {
        let metrics = self.logger.metrics();
        // A log that cannot be written is a degraded service, not a healthy
        // one: the next bug report loses its evidence, silently, unless this
        // is surfaced.
        if metrics.write_errors > 0 {
            return ServiceHealth::Degraded {
                reason: format!(
                    "{} log write error(s); records are still buffered in memory",
                    metrics.write_errors
                ),
                since_ms: 0,
            };
        }
        ServiceHealth::Healthy
    }

    fn metrics(&self) -> ServiceMetrics {
        let metrics = self.logger.metrics();
        ServiceMetrics {
            memory_bytes: 0,
            uptime_ms: 0,
            request_count: metrics.emitted,
            error_count: metrics.write_errors,
        }
    }
}

/// Register [`LogService`] on a container as a supervised singleton.
pub fn register(container: &mut ServiceContainer, logger: Arc<Logger>) -> Result<(), ServiceError> {
    container.register(
        SERVICE_NAME,
        &[crate::stream::SERVICE_NAME],
        Lifetime::Singleton,
        move |_ctx| Ok(Box::new(LogService::new(logger.clone())) as Box<dyn ManagedService>),
    )
}

/// File a frontend-supplied source under the `frontend.` namespace.
fn namespaced_frontend_source(source: &str) -> String {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return FRONTEND_NAMESPACE.to_string();
    }
    if trimmed == FRONTEND_NAMESPACE || trimmed.starts_with(&format!("{FRONTEND_NAMESPACE}.")) {
        return trimmed.to_string();
    }
    format!("{FRONTEND_NAMESPACE}.{trimmed}")
}

/// Whether a client-supplied timestamp is in the kernel's fixed-width RFC
/// 3339 form. Checked structurally rather than parsed: the only property the
/// viewer depends on is that comparing two timestamps as strings compares
/// them as instants.
fn is_kernel_timestamp(ts: &str) -> bool {
    let bytes = ts.as_bytes();
    bytes.len() == 24
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && bytes[19] == b'.'
        && bytes[23] == b'Z'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| matches!(i, 4 | 7 | 10 | 13 | 16 | 19 | 23) || b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_ipc::{IpcRequest, PING};
    use helix_log::{LogQuery, log_warn};
    use helix_stream::{ChannelSubscription, StreamFrame};

    fn logger() -> Arc<Logger> {
        Arc::new(Logger::in_memory(LogLevel::Trace))
    }

    fn dispatcher(logger: Arc<Logger>) -> IpcDispatcher {
        let mut dispatcher = IpcDispatcher::new();
        register_commands(&mut dispatcher, logger);
        dispatcher
    }

    #[tokio::test]
    async fn query_returns_matching_entries_with_the_sources_that_produced_them() {
        let logger = logger();
        log_info!(logger, "kernel.fs", "read a file", "path" => "/tmp/a");
        log_warn!(logger, "kernel.ipc", "slow command");
        let dispatcher = dispatcher(logger);

        let response = dispatcher
            .dispatch(IpcRequest::new(
                QUERY,
                "q1",
                serde_json::json!({ "query": { "min_level": "warn" } }),
            ))
            .await;

        let result = response.result.expect("log.query must succeed");
        assert_eq!(result["entries"].as_array().unwrap().len(), 1);
        assert_eq!(result["entries"][0]["message"], "slow command");
        assert_eq!(result["matched"], 1);
        assert_eq!(
            result["sources"],
            serde_json::json!(["kernel.fs", "kernel.ipc"])
        );
    }

    #[tokio::test]
    async fn query_with_an_empty_payload_returns_everything() {
        let logger = logger();
        log_info!(logger, "kernel.fs", "one");
        let dispatcher = dispatcher(logger);

        let response = dispatcher
            .dispatch(IpcRequest::new(QUERY, "q2", serde_json::json!({})))
            .await;
        assert_eq!(
            response.result.unwrap()["entries"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn export_returns_the_filtered_set_as_json_lines() {
        let logger = logger();
        log_info!(logger, "kernel.fs", "kept");
        log_info!(logger, "kernel.ipc", "dropped");
        let dispatcher = dispatcher(logger);

        let response = dispatcher
            .dispatch(IpcRequest::new(
                EXPORT,
                "e1",
                serde_json::json!({ "query": { "sources": ["kernel.fs"] } }),
            ))
            .await;

        let result = response.result.expect("log.export must succeed");
        assert_eq!(result["format"], "jsonl");
        assert_eq!(result["entry_count"], 1);
        let content = result["content"].as_str().unwrap();
        assert_eq!(content.lines().count(), 1);
        assert!(content.contains("kept"));
        assert!(
            result["suggested_file_name"]
                .as_str()
                .unwrap()
                .ends_with(".jsonl")
        );
    }

    #[tokio::test]
    async fn a_frontend_record_joins_the_unified_stream_under_the_frontend_namespace() {
        let logger = logger();
        let dispatcher = dispatcher(logger.clone());

        let response = dispatcher
            .dispatch(IpcRequest::new(
                APPEND,
                "a1",
                serde_json::json!({
                    "level": "warn",
                    "source": "app.editor",
                    "message": "save failed",
                    "fields": { "path": "/tmp/x" },
                    "correlation_id": "cmd-7"
                }),
            ))
            .await;

        let result = response.result.expect("log.append must succeed");
        assert_eq!(result["recorded"], true);
        assert_eq!(result["source"], "frontend.app.editor");

        let entries = logger.query(&LogQuery::new()).entries;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].source, "frontend.app.editor");
        assert_eq!(entries[0].level, LogLevel::Warn);
        assert_eq!(entries[0].correlation_id.as_deref(), Some("cmd-7"));
        assert_eq!(entries[0].fields["path"], "/tmp/x");
    }

    #[tokio::test]
    async fn a_frontend_record_cannot_impersonate_a_kernel_source() {
        let logger = logger();
        let dispatcher = dispatcher(logger.clone());
        let _ = dispatcher
            .dispatch(IpcRequest::new(
                APPEND,
                "a2",
                serde_json::json!({ "source": "kernel.fs", "message": "not really the kernel" }),
            ))
            .await;

        assert_eq!(
            logger.query(&LogQuery::new()).entries[0].source,
            "frontend.kernel.fs"
        );
    }

    #[tokio::test]
    async fn a_frontend_record_with_a_bogus_timestamp_is_stamped_by_the_kernel() {
        let logger = logger();
        let dispatcher = dispatcher(logger.clone());
        let _ = dispatcher
            .dispatch(IpcRequest::new(
                APPEND,
                "a3",
                serde_json::json!({ "message": "skewed", "ts": "yesterday afternoon" }),
            ))
            .await;

        let ts = &logger.query(&LogQuery::new()).entries[0].ts;
        assert!(is_kernel_timestamp(ts), "{ts}");
    }

    #[tokio::test]
    async fn a_frontend_record_keeps_a_well_formed_client_timestamp() {
        let logger = logger();
        let dispatcher = dispatcher(logger.clone());
        let _ = dispatcher
            .dispatch(IpcRequest::new(
                APPEND,
                "a4",
                serde_json::json!({ "message": "captured earlier", "ts": "2026-01-01T10:00:00.000Z" }),
            ))
            .await;

        assert_eq!(
            logger.query(&LogQuery::new()).entries[0].ts,
            "2026-01-01T10:00:00.000Z"
        );
    }

    #[tokio::test]
    async fn a_frontend_record_is_redacted_like_any_other() {
        let logger = logger();
        let dispatcher = dispatcher(logger.clone());
        let _ = dispatcher
            .dispatch(IpcRequest::new(
                APPEND,
                "a5",
                serde_json::json!({
                    "message": "configuring provider",
                    "fields": { "api_key": "sk-not-in-the-log-please" }
                }),
            ))
            .await;

        assert_eq!(
            logger.query(&LogQuery::new()).entries[0].fields["api_key"],
            helix_log::REDACTED
        );
    }

    #[tokio::test]
    async fn levels_can_be_read_and_changed_per_module_over_ipc() {
        let logger = Arc::new(Logger::in_memory(LogLevel::Info));
        let dispatcher = dispatcher(logger.clone());

        let response = dispatcher
            .dispatch(IpcRequest::new(
                SET_LEVEL,
                "l1",
                serde_json::json!({ "module": "kernel.fs", "level": "trace" }),
            ))
            .await;
        assert_eq!(
            response.result.unwrap()["levels"]["modules"]["kernel.fs"],
            "trace"
        );
        assert!(logger.enabled(LogLevel::Trace, "kernel.fs"));
        assert!(!logger.enabled(LogLevel::Trace, "kernel.ipc"));

        let cleared = dispatcher
            .dispatch(IpcRequest::new(
                SET_LEVEL,
                "l2",
                serde_json::json!({ "module": "kernel.fs" }),
            ))
            .await;
        assert!(
            cleared.result.unwrap()["levels"]["modules"]
                .as_object()
                .unwrap()
                .is_empty()
        );

        let read = dispatcher
            .dispatch(IpcRequest::new(LEVELS, "l3", serde_json::json!({})))
            .await;
        assert_eq!(read.result.unwrap()["levels"]["default_level"], "info");
    }

    #[tokio::test]
    async fn setting_neither_a_module_nor_a_level_is_rejected() {
        let dispatcher = dispatcher(logger());
        let response = dispatcher
            .dispatch(IpcRequest::new(SET_LEVEL, "l4", serde_json::json!({})))
            .await;
        assert_eq!(response.error.unwrap().code, "INVALID_PAYLOAD");
    }

    #[tokio::test]
    async fn the_service_publishes_the_logger_and_reports_health() {
        let logger = logger();
        let streaming = crate::stream::bind(
            helix_stream::HubConfig::default(),
            helix_stream::ServerConfig::default(),
        )
        .await
        .unwrap();

        let mut container = ServiceContainer::new();
        crate::stream::register(&mut container, streaming.clone()).unwrap();
        register(&mut container, logger.clone()).unwrap();
        container.start_all().await.unwrap();

        assert_eq!(
            container.health_summary().get(SERVICE_NAME),
            Some(&ServiceHealth::Healthy)
        );
        let resolved = container
            .context()
            .resolve::<Logger>()
            .expect("dependents must resolve the process logger");
        assert!(Arc::ptr_eq(&resolved, &logger));

        container.stop_all().await.unwrap();
    }

    #[tokio::test]
    async fn every_record_reaches_the_streaming_channel_while_the_viewer_is_subscribed() {
        let logger = logger();
        let streaming = crate::stream::bind(
            helix_stream::HubConfig::default(),
            helix_stream::ServerConfig::default(),
        )
        .await
        .unwrap();
        let mut container = ServiceContainer::new();
        crate::stream::register(&mut container, streaming.clone()).unwrap();
        register(&mut container, logger.clone()).unwrap();
        container.start_all().await.unwrap();

        let session = streaming.hub().open_session();
        session.subscribe(&[ChannelSubscription::new(CHANNEL)]);
        let _ = session.drain();

        log_warn!(logger, "kernel.fs", "disk nearly full", "free_mb" => 12);

        let frames = session.drain().expect("session is open");
        let record = frames
            .iter()
            .find_map(|frame| match frame {
                StreamFrame::Data(envelope) => Some(envelope),
                _ => None,
            })
            .expect("the record must be published to the log channel");
        assert_eq!(record.payload["message"], "disk nearly full");
        assert_eq!(record.payload["fields"]["free_mb"], 12);

        container.stop_all().await.unwrap();
    }

    #[tokio::test]
    async fn nothing_is_published_when_no_viewer_is_listening() {
        let logger = logger();
        let streaming = crate::stream::bind(
            helix_stream::HubConfig::default(),
            helix_stream::ServerConfig::default(),
        )
        .await
        .unwrap();
        let mut container = ServiceContainer::new();
        crate::stream::register(&mut container, streaming.clone()).unwrap();
        register(&mut container, logger.clone()).unwrap();
        container.start_all().await.unwrap();

        log_warn!(logger, "kernel.fs", "nobody is watching");
        assert_eq!(
            streaming.hub().next_sequence(CHANNEL),
            1,
            "a closed viewer must cost nothing per record"
        );

        container.stop_all().await.unwrap();
    }

    #[tokio::test]
    async fn a_commands_correlation_id_links_its_kernel_side_records() {
        // The Task 1.5 demo criterion: a frontend command's correlation ID
        // links the kernel-side entries it caused, with no explicit
        // threading through the service that logged them.
        let logger = logger();
        let mut dispatcher = IpcDispatcher::new();
        helix_ipc::register_builtins(&mut dispatcher, "0.0.0-test");
        register_commands(&mut dispatcher, logger.clone());
        let service_logger = logger.clone();
        dispatcher.register("demo.work", move |_req: serde_json::Value, _ctx| {
            let logger = service_logger.clone();
            async move {
                // A kernel service, unaware of correlation, logging normally.
                log_info!(logger, "kernel.fs", "opened a file", "path" => "/tmp/demo");
                Ok::<serde_json::Value, AppError>(serde_json::Value::Null)
            }
        });

        let _ = dispatcher
            .dispatch(IpcRequest::new(
                PING,
                "unrelated",
                serde_json::json!({ "message": "x" }),
            ))
            .await;
        let _ = dispatcher
            .dispatch(IpcRequest::new(
                "demo.work",
                "cmd-link-me",
                serde_json::json!({}),
            ))
            .await;

        let response = dispatcher
            .dispatch(IpcRequest::new(
                QUERY,
                "q-corr",
                serde_json::json!({ "query": { "correlation_id": "cmd-link-me" } }),
            ))
            .await;

        let entries = response.result.unwrap()["entries"].clone();
        let entries = entries.as_array().unwrap();
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0]["source"], "kernel.fs");
        assert_eq!(entries[0]["message"], "opened a file");
        assert_eq!(entries[0]["correlation_id"], "cmd-link-me");
    }

    #[test]
    fn a_frontend_source_is_namespaced_once_and_only_once() {
        assert_eq!(namespaced_frontend_source("app"), "frontend.app");
        assert_eq!(namespaced_frontend_source("frontend.app"), "frontend.app");
        assert_eq!(namespaced_frontend_source("frontend"), "frontend");
        assert_eq!(namespaced_frontend_source("  "), "frontend");
    }

    #[test]
    fn timestamp_validation_accepts_the_kernel_format_and_rejects_anything_else() {
        assert!(is_kernel_timestamp("2026-01-01T10:00:00.000Z"));
        assert!(!is_kernel_timestamp("2026-01-01T10:00:00Z"));
        assert!(!is_kernel_timestamp("2026-01-01T10:00:00.000+01:00"));
        assert!(!is_kernel_timestamp("not a timestamp at all!!"));
    }

    #[test]
    fn the_log_directory_sits_under_the_platform_state_directory() {
        // Only asserted when the platform variable exists, so the test is
        // meaningful on a developer machine and inert in a bare container.
        if let Some(directory) = default_log_directory() {
            assert!(directory.ends_with("logs"), "{}", directory.display());
            let as_text = directory.to_string_lossy().to_lowercase();
            assert!(as_text.contains("helix"), "{as_text}");
        }
    }
}
