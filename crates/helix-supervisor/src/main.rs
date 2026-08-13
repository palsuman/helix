//! Thin Tauri Host: windows, typed forwarding, and kernel supervision only.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use helix_ipc::{
    CancelRequest, CancelResponse, DEFAULT_TIMEOUT_MS, InternalRpcRequest, InternalRpcResponse,
    IpcRequest, IpcResponse,
};
use helix_supervisor_lib::{
    KernelSupervisor, RecoveryAction, SupervisorStatus, default_host_state_directory,
};
use tauri::State;

fn kernel_binary_path() -> Result<PathBuf, Box<dyn Error + Send + Sync>> {
    if let Some(path) = std::env::var_os("HELIX_KERNEL_BIN") {
        return Ok(PathBuf::from(path));
    }
    let mut path = std::env::current_exe()?;
    path.set_file_name(format!("helix-kernel{}", std::env::consts::EXE_SUFFIX));
    Ok(path)
}

#[tauri::command]
async fn ipc_dispatch(
    supervisor: State<'_, Arc<KernelSupervisor>>,
    request: IpcRequest<serde_json::Value>,
) -> Result<IpcResponse<serde_json::Value>, String> {
    let correlation_id = request.correlation_id.clone();
    let timeout = Duration::from_millis(
        u64::from(
            request
                .timeout_ms
                .filter(|value| *value > 0)
                .unwrap_or(DEFAULT_TIMEOUT_MS),
        ) + 1_000,
    );
    let response = match supervisor
        .call(InternalRpcRequest::Dispatch(request), timeout)
        .await
    {
        Ok(response) => response,
        Err(error) => {
            let _ = supervisor
                .call(
                    InternalRpcRequest::Cancel(CancelRequest { correlation_id }),
                    Duration::from_secs(5),
                )
                .await;
            return Err(error);
        }
    };
    match response {
        InternalRpcResponse::Dispatch(response) => Ok(response),
        InternalRpcResponse::ProtocolError { message } => Err(message),
        _ => Err("kernel returned an unexpected response".into()),
    }
}

#[tauri::command]
async fn ipc_cancel(
    supervisor: State<'_, Arc<KernelSupervisor>>,
    request: CancelRequest,
) -> Result<CancelResponse, String> {
    match supervisor
        .call(InternalRpcRequest::Cancel(request), Duration::from_secs(5))
        .await?
    {
        InternalRpcResponse::Cancel(response) => Ok(response),
        InternalRpcResponse::ProtocolError { message } => Err(message),
        _ => Err("kernel returned an unexpected response".into()),
    }
}

#[tauri::command]
async fn supervisor_status(
    supervisor: State<'_, Arc<KernelSupervisor>>,
) -> Result<SupervisorStatus, String> {
    Ok(supervisor.status().await)
}

#[tauri::command]
async fn supervisor_recovery_action(
    supervisor: State<'_, Arc<KernelSupervisor>>,
    action: RecoveryAction,
) -> Result<(), String> {
    supervisor.recovery_action(action).await;
    Ok(())
}

#[cfg(feature = "ipc-e2e")]
#[tauri::command]
async fn ipc_e2e_restart(supervisor: State<'_, Arc<KernelSupervisor>>) -> Result<bool, String> {
    supervisor.restart_and_probe_stale_peer().await
}

#[cfg(feature = "ipc-e2e")]
#[tauri::command]
async fn ipc_e2e_report(app: tauri::AppHandle, report: serde_json::Value) -> Result<(), String> {
    let path =
        std::env::var_os("HELIX_IPC_E2E_REPORT").ok_or("HELIX_IPC_E2E_REPORT is required")?;
    let encoded = serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?;
    std::fs::write(path, encoded).map_err(|error| error.to_string())?;
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(25)).await;
        app.exit(0);
    });
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let host_state = default_host_state_directory()
        .unwrap_or_else(|| std::env::temp_dir().join("helix-state").join("host"));
    let supervisor = tauri::async_runtime::block_on(async {
        let supervisor = KernelSupervisor::launch(
            kernel_binary_path().expect("failed to resolve helix-kernel"),
            host_state,
        )
        .await;
        supervisor
            .wait_until_ready(Duration::from_secs(10))
            .await
            .expect("failed to launch helix-kernel");
        supervisor
    });
    let shutdown_started = Arc::new(AtomicBool::new(false));
    let builder = tauri::Builder::default().manage(supervisor.clone());
    #[cfg(not(feature = "ipc-e2e"))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        ipc_dispatch,
        ipc_cancel,
        supervisor_status,
        supervisor_recovery_action,
    ]);
    #[cfg(feature = "ipc-e2e")]
    let builder = builder.invoke_handler(tauri::generate_handler![
        ipc_dispatch,
        ipc_cancel,
        supervisor_status,
        supervisor_recovery_action,
        ipc_e2e_restart,
        ipc_e2e_report,
    ]);
    let app = builder
        .build(tauri::generate_context!())
        .expect("failed to build Helix Host");
    app.run(move |handle, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event
            && !shutdown_started.swap(true, Ordering::SeqCst)
        {
            api.prevent_exit();
            let supervisor = supervisor.clone();
            let handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                supervisor.shutdown().await;
                handle.exit(0);
            });
        }
    });
}

fn main() {
    run();
}
