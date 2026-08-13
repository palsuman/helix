//! `helix-kernel` — the Rust backend process that owns all application
//! state (REQ-ARCH-001).
//!
//! This crate contains no Tauri or windowing code. The separate
//! `helix-supervisor` process owns Tauri Core and forwards authenticated typed
//! requests to this authoritative domain process.

pub mod config;
pub mod fs;
pub mod ipc;
pub mod log;
pub mod project_graph;
pub mod state;
pub mod stream;
pub mod workspace;

use std::sync::Arc;

use helix_config::ConfigService;
use helix_core::container::ServiceContainer;
use helix_fs::FileSystemService;
use helix_ipc::IpcDispatcher;
use helix_log::{LogLevel, Logger, log_info};
use helix_state::StatePersistence;
use helix_workspace::{ProjectGraphService, WorkspaceService};

const KERNEL_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Everything [`bootstrap`] hands back: the started container plus the
/// long-lived pieces the kernel transport layer needs.
pub struct Kernel {
    pub container: ServiceContainer,
    pub dispatcher: Arc<IpcDispatcher>,
    pub streaming: stream::StreamRuntime,
    pub logger: Arc<Logger>,
    pub config: Arc<ConfigService>,
    pub fs: Arc<FileSystemService>,
    pub workspace: Arc<WorkspaceService>,
    pub project_graph: Arc<ProjectGraphService>,
    pub state: Arc<StatePersistence>,
}

/// Build and start the kernel's service container, returning it alongside
/// the shared dispatcher the transport layer needs, the streaming runtime,
/// and the process logger.
///
/// Two things are constructed before the container starts. The streaming
/// server, because its port is assigned by the OS and the `stream.endpoint`
/// command has to be able to answer the moment the frontend asks. And the
/// logger, because startup itself is worth logging: a logger created by the
/// container could not record anything that happened before it.
pub async fn bootstrap() -> Result<Kernel, helix_core::ServiceError> {
    let logger = log::build_logger(default_log_level());
    log_info!(
        logger,
        log::KERNEL_SOURCE,
        "kernel starting",
        "version" => KERNEL_VERSION,
    );

    let streaming = stream::bind(
        helix_stream::HubConfig::default(),
        helix_stream::ServerConfig::default(),
    )
    .await?;

    // Built before the container for the same reason the logger is: the layer
    // files are read from disk, and startup itself is worth configuring. The
    // service applies its own log settings once the container starts it.
    let config = config::build_service(logger.clone());

    // After the configuration service, because its exclusions, encoding, and
    // watch depth come from `files.*`. No roots are watched yet: the workspace
    // manager decides what to watch, and until a workspace is opened the
    // watcher thread idles.
    let fs = fs::build_service(&config, logger.clone());

    // After both, because a workspace resolves its settings over the
    // configuration layers and reads its document and roots through the file
    // system service. No workspace is open until a window asks for one.
    let workspace = workspace::build_service(config.clone(), fs.clone(), logger.clone());

    // Graph work is scheduled only after a workspace event, and extraction is
    // always background. Constructing the service here performs no repository
    // I/O and therefore cannot delay kernel or workspace startup.
    let (project_graph, project_graph_runtime) = project_graph::build_service(&workspace);

    // State is outside every workspace and follows workspace lifecycle. Its
    // flush interval is the configured editor recovery-point objective.
    let state = state::build_service(&config);

    let mut dispatcher = ipc::build_dispatcher(KERNEL_VERSION);
    stream::register_commands(&mut dispatcher, &streaming);
    log::register_commands(&mut dispatcher, logger.clone());
    config::register_commands(&mut dispatcher, config.clone(), Some(workspace.clone()));
    fs::register_commands(&mut dispatcher, fs.clone());
    workspace::register_commands(&mut dispatcher, workspace.clone());
    project_graph::register_commands(
        &mut dispatcher,
        project_graph.clone(),
        workspace.clone(),
        project_graph_runtime.scheduler.clone(),
    );
    let dispatcher = Arc::new(dispatcher);

    let mut container = ServiceContainer::new();
    ipc::register(&mut container, dispatcher.clone())?;
    stream::register(&mut container, streaming.clone())?;
    log::register(&mut container, logger.clone())?;
    config::register(&mut container, config.clone(), logger.clone())?;
    fs::register(&mut container, fs.clone(), logger.clone())?;
    workspace::register(
        &mut container,
        workspace.clone(),
        fs.clone(),
        logger.clone(),
    )?;
    project_graph::register(
        &mut container,
        project_graph.clone(),
        workspace.clone(),
        fs.clone(),
        logger.clone(),
        project_graph_runtime,
    )?;
    state::register(
        &mut container,
        state.clone(),
        workspace.clone(),
        logger.clone(),
        config.clone(),
    )?;
    container.start_all().await?;

    Ok(Kernel {
        container,
        dispatcher,
        streaming,
        logger,
        config,
        fs,
        workspace,
        project_graph,
        state,
    })
}

/// Default log level, overridable with `HELIX_LOG_LEVEL`.
///
/// `log.level` in settings is the durable control and is applied by the
/// configuration service once the container starts it. The environment
/// variable still matters for the window before that: it is the only way to
/// get verbose records out of the logger's own construction and the layer load
/// that decides `log.level` in the first place.
fn default_log_level() -> LogLevel {
    std::env::var("HELIX_LOG_LEVEL")
        .ok()
        .as_deref()
        .and_then(LogLevel::parse)
        .unwrap_or(if cfg!(debug_assertions) {
            LogLevel::Debug
        } else {
            LogLevel::Info
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_ipc::{IpcRequest, PING};
    use helix_log::LogQuery;

    #[tokio::test]
    async fn bootstrap_starts_the_ipc_service_and_serves_a_typed_command() {
        let Kernel {
            mut container,
            dispatcher,
            ..
        } = bootstrap().await.unwrap();

        let response = dispatcher
            .dispatch(IpcRequest::new(
                PING,
                "boot-1",
                serde_json::json!({ "message": "hello" }),
            ))
            .await;

        let result = response.result.expect("ping must succeed after bootstrap");
        assert_eq!(result["echo"], "hello");
        assert_eq!(result["kernel_version"], KERNEL_VERSION);

        container.stop_all().await.unwrap();
    }

    #[tokio::test]
    async fn bootstrap_binds_the_stream_server_and_advertises_it_over_ipc() {
        let Kernel {
            mut container,
            dispatcher,
            streaming,
            ..
        } = bootstrap().await.unwrap();

        let response = dispatcher
            .dispatch(IpcRequest::new(
                stream::ENDPOINT,
                "boot-2",
                serde_json::json!({}),
            ))
            .await;

        let result = response
            .result
            .expect("stream.endpoint must answer after bootstrap");
        assert_eq!(result["port"], streaming.port());
        assert_ne!(streaming.port(), 0);

        container.stop_all().await.unwrap();
    }

    #[tokio::test]
    async fn bootstrap_resolves_configuration_and_serves_the_config_commands() {
        let Kernel {
            mut container,
            dispatcher,
            config,
            ..
        } = bootstrap().await.unwrap();

        // Asserted against the schema default rather than a literal, so a
        // developer machine with its own ~/.helix/settings.json does not fail
        // the suite.
        let expected = config.get("editor.fontSize", None).unwrap().value;
        let response = dispatcher
            .dispatch(IpcRequest::new(
                helix_config::commands::GET,
                "boot-4",
                serde_json::json!({ "key": "editor.fontSize" }),
            ))
            .await;
        assert_eq!(
            response
                .result
                .expect("config.get must answer after bootstrap")["setting"]["value"],
            expected
        );

        let resolved = container
            .context()
            .resolve::<helix_config::ConfigService>()
            .expect("dependents must resolve the configuration service");
        assert!(std::sync::Arc::ptr_eq(&resolved, &config));

        container.stop_all().await.unwrap();
    }

    #[tokio::test]
    async fn bootstrap_resolves_the_file_system_service_and_serves_the_fs_commands() {
        let Kernel {
            mut container,
            dispatcher,
            fs,
            ..
        } = bootstrap().await.unwrap();

        let dir = helix_fs::testutil::TempDir::new("bootstrap-fs");
        let path = dir.write("main.rs", "fn main() {}\n");
        let response = dispatcher
            .dispatch(IpcRequest::new(
                helix_fs::commands::READ,
                "boot-5",
                serde_json::json!({ "path": path.to_string_lossy() }),
            ))
            .await;
        assert_eq!(
            response
                .result
                .expect("fs.read must answer after bootstrap")["content"]["text"],
            "fn main() {}\n"
        );

        let resolved = container
            .context()
            .resolve::<helix_fs::FileSystemService>()
            .expect("dependents must resolve the file system service");
        assert!(std::sync::Arc::ptr_eq(&resolved, &fs));
        assert_eq!(
            fs.watch_stats().roots,
            0,
            "nothing is watched until a workspace is opened"
        );

        container.stop_all().await.unwrap();
    }

    #[tokio::test]
    async fn bootstrap_resolves_the_workspace_manager_and_serves_the_workspace_commands() {
        let Kernel {
            mut container,
            dispatcher,
            workspace,
            ..
        } = bootstrap().await.unwrap();

        // Read-only commands only: opening a workspace here would write to the
        // developer's own recent list, which a test has no business doing.
        let response = dispatcher
            .dispatch(IpcRequest::new(
                helix_workspace::commands::LIST,
                "boot-6",
                serde_json::json!({}),
            ))
            .await;
        assert_eq!(
            response
                .result
                .expect("workspace.list must answer after bootstrap")["workspaces"]
                .as_array()
                .unwrap()
                .len(),
            0,
            "no workspace is open until a window asks for one"
        );

        let resolved = container
            .context()
            .resolve::<helix_workspace::WorkspaceService>()
            .expect("dependents must resolve the workspace manager");
        assert!(std::sync::Arc::ptr_eq(&resolved, &workspace));
        assert_eq!(workspace.max_roots(), helix_workspace::MAX_ROOTS);

        container.stop_all().await.unwrap();
    }

    #[tokio::test]
    async fn bootstrap_records_its_own_startup_and_serves_the_log_query_command() {
        let Kernel {
            mut container,
            dispatcher,
            logger,
            ..
        } = bootstrap().await.unwrap();

        let startup = logger.query(&LogQuery::new().with_sources([log::KERNEL_SOURCE]));
        assert!(
            startup
                .entries
                .iter()
                .any(|record| record.message == "kernel starting"),
            "startup must be recorded by the logger that outlives the container"
        );

        let response = dispatcher
            .dispatch(IpcRequest::new(
                helix_log::commands::QUERY,
                "boot-3",
                serde_json::json!({ "query": { "sources": ["kernel.log"] } }),
            ))
            .await;
        assert!(
            !response
                .result
                .expect("log.query must answer after bootstrap")["entries"]
                .as_array()
                .unwrap()
                .is_empty()
        );

        container.stop_all().await.unwrap();
    }
}
