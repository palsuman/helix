use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use helix_fs::testutil::TempDir;
use helix_ipc::{
    IpcDispatcher, KERNEL_EPOCH_ENV, KERNEL_LAUNCH_TOKEN_ENV, KERNEL_READY_PREFIX, KernelReady,
    serve_internal_rpc_request_with_shutdown,
};
use helix_state::{BufferState, StateStore, StateStoreConfig};
use helix_supervisor_lib::{KernelSupervisor, SupervisorStatus};
use tokio::net::TcpListener;

fn helper_binary() -> PathBuf {
    std::env::current_exe().unwrap()
}

fn helper_args() -> Vec<String> {
    vec![
        "--exact".into(),
        "process_fixture_entrypoint".into(),
        "--nocapture".into(),
    ]
}

fn fixture_env(mode: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("HELIX_TEST_KERNEL_FIXTURE".into(), "1".into()),
        ("HELIX_TEST_KERNEL_MODE".into(), mode.into()),
    ])
}

/// The integration-test executable doubles as its child process fixture. This
/// avoids adding a test-only binary to the application package.
#[test]
fn process_fixture_entrypoint() {
    if std::env::var_os("HELIX_TEST_KERNEL_FIXTURE").is_none() {
        return;
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(run_process_fixture())
        .unwrap();
}

async fn run_process_fixture() -> Result<(), Box<dyn std::error::Error>> {
    let token = std::env::var(KERNEL_LAUNCH_TOKEN_ENV)?;
    let epoch = std::env::var(KERNEL_EPOCH_ENV)?;
    let mode = std::env::var("HELIX_TEST_KERNEL_MODE").unwrap_or_default();
    if mode == "fail_until_safe" && std::env::var_os("HELIX_SAFE_MODE").is_none() {
        std::process::exit(19);
    }
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    println!(
        "{KERNEL_READY_PREFIX}{}",
        serde_json::to_string(&KernelReady {
            port: listener.local_addr()?.port(),
            process_id: std::process::id(),
            epoch: epoch.clone(),
        })?
    );
    std::io::stdout().flush()?;
    if mode == "always_crash" {
        eprintln!("fixture kernel crash");
        tokio::time::sleep(Duration::from_millis(50)).await;
        std::process::exit(17);
    }
    if mode == "crash_once" {
        let marker = std::env::var("HELIX_TEST_CRASH_MARKER")?;
        if !std::path::Path::new(&marker).exists() {
            std::fs::write(marker, b"crashed")?;
            eprintln!("fixture kernel crash once");
            tokio::time::sleep(Duration::from_millis(50)).await;
            std::process::exit(17);
        }
    }
    let dispatcher = Arc::new(IpcDispatcher::new());
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel(1);
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let stream = accepted?.0;
                let token = token.clone();
                let epoch = epoch.clone();
                let dispatcher = dispatcher.clone();
                let shutdown_tx = shutdown_tx.clone();
                tokio::spawn(async move {
                    let _ = serve_internal_rpc_request_with_shutdown(
                        stream, &token, &epoch, dispatcher, Some(shutdown_tx),
                    ).await;
                });
            }
            _ = shutdown_rx.recv() => return Ok(()),
        }
    }
}

async fn wait_for<F>(
    supervisor: &KernelSupervisor,
    timeout: Duration,
    predicate: F,
) -> SupervisorStatus
where
    F: Fn(&SupervisorStatus) -> bool,
{
    tokio::time::timeout(timeout, async {
        loop {
            let status = supervisor.status().await;
            if predicate(&status) {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("supervisor state transition timed out")
}

#[tokio::test]
async fn an_abnormal_exit_restarts_within_two_seconds_and_wal_state_survives() {
    let dir = TempDir::new("supervisor-restart");
    let state_dir = dir.path().join("workspace-state");
    let store = StateStore::new(&state_dir, "workspace", vec![], StateStoreConfig::default());
    store.queue_buffer(
        BufferState {
            id: "editor-1".into(),
            content: "unsaved edit".into(),
            language: "text".into(),
            target: None,
            dirty: true,
            cursor_line: 0,
            cursor_column: 12,
        },
        0,
    );
    store.flush_all(1).unwrap();
    let mut env = fixture_env("crash_once");
    env.insert(
        "HELIX_TEST_CRASH_MARKER".into(),
        dir.path().join("crashed").to_string_lossy().into_owned(),
    );
    let supervisor = KernelSupervisor::launch_with_options(
        helper_binary(),
        dir.path().join("host-state"),
        env,
        helper_args(),
    )
    .await;
    supervisor
        .wait_until_ready(Duration::from_secs(2))
        .await
        .unwrap();
    let first_epoch = match supervisor.status().await {
        SupervisorStatus::Running { epoch, .. } => epoch,
        other => panic!("expected running, got {other:?}"),
    };
    let started = Instant::now();
    wait_for(
        &supervisor,
        Duration::from_secs(2),
        |status| matches!(status, SupervisorStatus::Running { epoch, .. } if epoch != &first_epoch),
    )
    .await;
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(
        StateStore::new(state_dir, "workspace", vec![], StateStoreConfig::default())
            .recover()
            .unwrap()
            .session
            .buffers[0]
            .content,
        "unsaved edit"
    );
    supervisor.shutdown().await;
}

#[tokio::test]
async fn a_clean_acknowledged_exit_is_not_restarted() {
    let dir = TempDir::new("supervisor-clean-exit");
    let supervisor = KernelSupervisor::launch_with_options(
        helper_binary(),
        dir.path().join("host-state"),
        fixture_env("clean"),
        helper_args(),
    )
    .await;
    supervisor
        .wait_until_ready(Duration::from_secs(2))
        .await
        .unwrap();
    supervisor.shutdown().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(supervisor.status().await, SupervisorStatus::Stopped);
}

#[tokio::test]
async fn restart_storm_stops_and_failed_starts_enter_safe_mode() {
    let dir = TempDir::new("supervisor-storm");
    let supervisor = KernelSupervisor::launch_with_options(
        helper_binary(),
        dir.path().join("storm-state"),
        fixture_env("always_crash"),
        helper_args(),
    )
    .await;
    wait_for(&supervisor, Duration::from_secs(3), |status| {
        matches!(status, SupervisorStatus::RecoveryRequired { .. })
    })
    .await;
    let crash = std::fs::read_to_string(dir.path().join("storm-state/last-crash.json")).unwrap();
    assert!(crash.contains("fixture kernel crash"));
    assert!(crash.contains("\"exit_code\": 17"));
    supervisor.shutdown().await;

    let safe = KernelSupervisor::launch_with_options(
        helper_binary(),
        dir.path().join("safe-state"),
        fixture_env("fail_until_safe"),
        helper_args(),
    )
    .await;
    wait_for(&safe, Duration::from_secs(3), |status| {
        matches!(
            status,
            SupervisorStatus::Running {
                safe_mode: true,
                ..
            }
        )
    })
    .await;
    safe.shutdown().await;
}
