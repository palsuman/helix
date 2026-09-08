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
    FolderOpenPlan, KernelSupervisor, RecoveryAction, RestorePlan, SupervisorStatus,
    closing_last_window, default_host_state_directory, folder_open_plan, restore_plan,
    restored_window_payload,
};
use serde::Deserialize;
use tauri::{
    LogicalPosition, LogicalSize, Manager, Position, Size, State, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};
use uuid::Uuid;

fn kernel_binary_path() -> Result<PathBuf, Box<dyn Error + Send + Sync>> {
    if let Some(path) = std::env::var_os("HELIX_KERNEL_BIN") {
        return Ok(PathBuf::from(path));
    }
    let mut path = std::env::current_exe()?;
    path.set_file_name(format!("helix-kernel{}", std::env::consts::EXE_SUFFIX));
    Ok(path)
}

async fn kernel_dispatch(
    supervisor: &KernelSupervisor,
    mut request: IpcRequest<serde_json::Value>,
    window_id: Option<&str>,
) -> Result<serde_json::Value, String> {
    if request.window_id.is_none() {
        request.window_id = window_id.map(str::to_string);
    }
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
        InternalRpcResponse::Dispatch(response) => {
            if let Some(error) = response.error {
                return Err(error.message);
            }
            response
                .result
                .ok_or_else(|| "kernel returned no result".into())
        }
        InternalRpcResponse::ProtocolError { message } => Err(message),
        _ => Err("kernel returned an unexpected response".into()),
    }
}

#[tauri::command]
async fn ipc_dispatch(
    window: WebviewWindow,
    supervisor: State<'_, Arc<KernelSupervisor>>,
    mut request: IpcRequest<serde_json::Value>,
) -> Result<IpcResponse<serde_json::Value>, String> {
    if request.window_id.is_none() {
        request.window_id = Some(window.label().to_string());
    }
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

fn next_window_id() -> String {
    format!("w-{}", Uuid::new_v4())
}

fn spawn_native_window(
    app: &tauri::AppHandle,
    id: &str,
    geometry: Option<&serde_json::Value>,
) -> Result<(), String> {
    let width = geometry
        .and_then(|value| value.get("width"))
        .and_then(|value| value.as_u64())
        .unwrap_or(1200) as f64;
    let height = geometry
        .and_then(|value| value.get("height"))
        .and_then(|value| value.as_u64())
        .unwrap_or(800) as f64;
    let x = geometry
        .and_then(|value| value.get("x"))
        .and_then(|value| value.as_i64())
        .unwrap_or(80) as f64;
    let y = geometry
        .and_then(|value| value.get("y"))
        .and_then(|value| value.as_i64())
        .unwrap_or(80) as f64;
    let maximized = geometry
        .and_then(|value| value.get("maximized"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let fullscreen = geometry
        .and_then(|value| value.get("fullscreen"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    WebviewWindowBuilder::new(app, id, WebviewUrl::App("index.html".into()))
        .title("Helix")
        .inner_size(width, height)
        .position(x, y)
        .min_inner_size(1024.0, 600.0)
        .maximized(maximized)
        .fullscreen(fullscreen)
        .build()
        .map_err(|error| error.to_string())?;
    Ok(())
}

async fn spawn_and_register_window(
    app: &tauri::AppHandle,
    supervisor: &KernelSupervisor,
    id: &str,
    payload: serde_json::Value,
) -> Result<String, String> {
    spawn_native_window(app, id, payload.get("geometry"))?;
    match kernel_dispatch(
        supervisor,
        IpcRequest::new("window.open", Uuid::new_v4().to_string(), payload),
        Some(id),
    )
    .await
    {
        Ok(_) => Ok(id.to_string()),
        Err(error) => {
            if let Some(window) = app.get_webview_window(id) {
                let _ = window.close();
            }
            Err(error)
        }
    }
}

fn apply_native_geometry(window: &WebviewWindow, geometry: &serde_json::Value) {
    let width = geometry
        .get("width")
        .and_then(|value| value.as_f64())
        .unwrap_or(1200.0);
    let height = geometry
        .get("height")
        .and_then(|value| value.as_f64())
        .unwrap_or(800.0);
    let x = geometry
        .get("x")
        .and_then(|value| value.as_f64())
        .unwrap_or(80.0);
    let y = geometry
        .get("y")
        .and_then(|value| value.as_f64())
        .unwrap_or(80.0);
    let _ = window.set_size(Size::Logical(LogicalSize::new(width, height)));
    let _ = window.set_position(Position::Logical(LogicalPosition::new(x, y)));
    if let Some(maximized) = geometry.get("maximized").and_then(|value| value.as_bool()) {
        if maximized {
            let _ = window.maximize();
        } else {
            let _ = window.unmaximize();
        }
    }
    if let Some(fullscreen) = geometry.get("fullscreen").and_then(|value| value.as_bool()) {
        let _ = window.set_fullscreen(fullscreen);
    }
}

fn persist_native_geometry(window: &WebviewWindow, supervisor: Arc<KernelSupervisor>) {
    let Ok(scale) = window.scale_factor() else {
        return;
    };
    let Ok(size) = window.inner_size() else {
        return;
    };
    let Ok(position) = window.outer_position() else {
        return;
    };
    let maximized = window.is_maximized().unwrap_or(false);
    let fullscreen = window.is_fullscreen().unwrap_or(false);
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .and_then(|monitor| monitor.name().cloned());
    let id = window.label().to_string();
    let geometry = serde_json::json!({
        "x": (f64::from(position.x) / scale).round() as i32,
        "y": (f64::from(position.y) / scale).round() as i32,
        "width": (f64::from(size.width) / scale).round() as u32,
        "height": (f64::from(size.height) / scale).round() as u32,
        "maximized": maximized,
        "fullscreen": fullscreen,
        "monitor": monitor,
    });
    tauri::async_runtime::spawn(async move {
        let _ = kernel_dispatch(
            &supervisor,
            IpcRequest::new(
                "window.setGeometry",
                Uuid::new_v4().to_string(),
                serde_json::json!({ "id": id, "geometry": geometry }),
            ),
            Some(&id),
        )
        .await;
    });
}

#[tauri::command]
async fn window_new(
    app: tauri::AppHandle,
    supervisor: State<'_, Arc<KernelSupervisor>>,
) -> Result<String, String> {
    let id = next_window_id();
    spawn_and_register_window(&app, &supervisor, &id, serde_json::json!({ "id": id })).await
}

#[derive(Deserialize)]
struct OpenFolderArgs {
    path: String,
    #[serde(default)]
    force_new: bool,
}

#[tauri::command]
async fn window_open_folder(
    app: tauri::AppHandle,
    supervisor: State<'_, Arc<KernelSupervisor>>,
    args: OpenFolderArgs,
) -> Result<String, String> {
    let found = kernel_dispatch(
        &supervisor,
        IpcRequest::new(
            "window.findByWorkspace",
            Uuid::new_v4().to_string(),
            serde_json::json!({ "roots": [args.path], "force_new": args.force_new }),
        ),
        None,
    )
    .await?;
    let existing = found.get("window_id").and_then(|value| value.as_str());
    match folder_open_plan(existing, args.force_new) {
        FolderOpenPlan::Focus { window_id } => {
            if let Some(window) = app.get_webview_window(&window_id) {
                let _ = window.set_focus();
            }
            kernel_dispatch(
                &supervisor,
                IpcRequest::new(
                    "window.focus",
                    Uuid::new_v4().to_string(),
                    serde_json::json!({ "id": window_id }),
                ),
                Some(&window_id),
            )
            .await?;
            Ok(window_id)
        }
        FolderOpenPlan::CreateNew => {
            let id = next_window_id();
            spawn_and_register_window(
                &app,
                &supervisor,
                &id,
                serde_json::json!({ "id": id, "roots": [args.path] }),
            )
            .await
        }
    }
}

#[tauri::command]
async fn window_duplicate(
    window: WebviewWindow,
    app: tauri::AppHandle,
    supervisor: State<'_, Arc<KernelSupervisor>>,
) -> Result<String, String> {
    let listed = kernel_dispatch(
        &supervisor,
        IpcRequest::new(
            "window.list",
            Uuid::new_v4().to_string(),
            serde_json::json!({}),
        ),
        Some(window.label()),
    )
    .await?;
    let roots = listed
        .get("windows")
        .and_then(|value| value.as_array())
        .and_then(|windows| {
            windows.iter().find(|candidate| {
                candidate.get("id").and_then(|id| id.as_str()) == Some(window.label())
            })
        })
        .and_then(|record| record.get("roots").cloned())
        .unwrap_or_else(|| serde_json::json!([]));
    let id = next_window_id();
    spawn_and_register_window(
        &app,
        &supervisor,
        &id,
        serde_json::json!({ "id": id, "roots": roots }),
    )
    .await
}

/// Opens another window on the same workspace so an editor tab can be moved
/// there (Task 2.4). Drag-out of a real editor tab is wired once Task 4.2 exists.
#[tauri::command]
async fn window_move_editor_to_new(
    window: WebviewWindow,
    app: tauri::AppHandle,
    supervisor: State<'_, Arc<KernelSupervisor>>,
) -> Result<String, String> {
    window_duplicate(window, app, supervisor).await
}

#[tauri::command]
async fn window_close(
    window: WebviewWindow,
    app: tauri::AppHandle,
    supervisor: State<'_, Arc<KernelSupervisor>>,
) -> Result<(), String> {
    let id = window.label().to_string();
    let closed = kernel_dispatch(
        &supervisor,
        IpcRequest::new(
            "window.close",
            Uuid::new_v4().to_string(),
            serde_json::json!({ "id": id }),
        ),
        Some(&id),
    )
    .await?;
    let remaining = closed
        .get("remaining")
        .and_then(|value| value.as_u64())
        .unwrap_or(0) as u32;
    let _ = window.close();
    if closing_last_window(remaining) {
        supervisor.shutdown().await;
        app.exit(0);
    }
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
    #[cfg(feature = "e2e")]
    let builder = builder
        .plugin(tauri_plugin_wdio::init())
        .plugin(tauri_plugin_wdio_webdriver::init());
    #[cfg(not(feature = "ipc-e2e"))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        ipc_dispatch,
        ipc_cancel,
        supervisor_status,
        supervisor_recovery_action,
        window_new,
        window_open_folder,
        window_duplicate,
        window_move_editor_to_new,
        window_close,
    ]);
    #[cfg(feature = "ipc-e2e")]
    let builder = builder.invoke_handler(tauri::generate_handler![
        ipc_dispatch,
        ipc_cancel,
        supervisor_status,
        supervisor_recovery_action,
        window_new,
        window_open_folder,
        window_duplicate,
        window_move_editor_to_new,
        window_close,
        ipc_e2e_restart,
        ipc_e2e_report,
    ]);
    let app = builder
        .setup(|app| {
            let supervisor = app.state::<Arc<KernelSupervisor>>().inner().clone();
            let label = app
                .webview_windows()
                .keys()
                .next()
                .cloned()
                .unwrap_or_else(|| "main".into());
            tauri::async_runtime::block_on(async {
                let session = kernel_dispatch(
                    &supervisor,
                    IpcRequest::new(
                        "window.session",
                        Uuid::new_v4().to_string(),
                        serde_json::json!({}),
                    ),
                    Some(&label),
                )
                .await
                .unwrap_or_else(|_| serde_json::json!({}));
                let restore_enabled = session
                    .get("restore_enabled")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(true);
                let ids: Vec<String> = session
                    .get("session")
                    .and_then(|value| value.get("windows"))
                    .and_then(|value| value.as_array())
                    .map(|windows| {
                        windows
                            .iter()
                            .filter_map(|window| {
                                window
                                    .get("id")
                                    .and_then(|id| id.as_str())
                                    .map(str::to_string)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                match restore_plan(restore_enabled, &ids, &label) {
                    RestorePlan::DefaultWindow => {
                        let _ = kernel_dispatch(
                            &supervisor,
                            IpcRequest::new(
                                "window.open",
                                Uuid::new_v4().to_string(),
                                serde_json::json!({ "id": label }),
                            ),
                            Some(&label),
                        )
                        .await;
                    }
                    RestorePlan::Restore { additional_ids, .. } => {
                        let first = session
                            .pointer("/session/windows/0")
                            .cloned()
                            .unwrap_or(serde_json::json!({}));
                        let open = restored_window_payload(&first, &label);
                        if let Some(geometry) = first.get("geometry")
                            && let Some(window) = app.get_webview_window(&label)
                        {
                            apply_native_geometry(&window, geometry);
                        }
                        let _ = kernel_dispatch(
                            &supervisor,
                            IpcRequest::new("window.open", Uuid::new_v4().to_string(), open),
                            Some(&label),
                        )
                        .await;
                        for extra_id in additional_ids {
                            let extra = session
                                .get("session")
                                .and_then(|value| value.get("windows"))
                                .and_then(|value| value.as_array())
                                .and_then(|windows| {
                                    windows.iter().find(|window| {
                                        window.get("id").and_then(|id| id.as_str())
                                            == Some(&extra_id)
                                    })
                                })
                                .cloned()
                                .unwrap_or(serde_json::json!({ "id": extra_id }));
                            let _ =
                                spawn_native_window(app.handle(), &extra_id, extra.get("geometry"));
                            let _ = kernel_dispatch(
                                &supervisor,
                                IpcRequest::new(
                                    "window.open",
                                    Uuid::new_v4().to_string(),
                                    restored_window_payload(&extra, &extra_id),
                                ),
                                Some(&extra_id),
                            )
                            .await;
                        }
                    }
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build Helix Host");
    app.run(move |handle, event| match event {
        tauri::RunEvent::WindowEvent {
            label,
            event: window_event,
            ..
        } => match window_event {
            tauri::WindowEvent::Destroyed => {
                let supervisor = supervisor.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = kernel_dispatch(
                        &supervisor,
                        IpcRequest::new(
                            "window.close",
                            Uuid::new_v4().to_string(),
                            serde_json::json!({ "id": label }),
                        ),
                        Some(&label),
                    )
                    .await;
                });
            }
            tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
                if let Some(window) = handle.get_webview_window(&label) {
                    persist_native_geometry(&window, supervisor.clone());
                }
            }
            tauri::WindowEvent::Focused(true) => {
                let supervisor = supervisor.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = kernel_dispatch(
                        &supervisor,
                        IpcRequest::new(
                            "window.focus",
                            Uuid::new_v4().to_string(),
                            serde_json::json!({ "id": label }),
                        ),
                        Some(&label),
                    )
                    .await
                    {
                        eprintln!("failed to update focused Helix window: {error}");
                    }
                });
            }
            _ => {}
        },
        tauri::RunEvent::ExitRequested { api, .. }
            if !shutdown_started.swap(true, Ordering::SeqCst) =>
        {
            api.prevent_exit();
            let supervisor = supervisor.clone();
            let handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                supervisor.shutdown().await;
                #[cfg(feature = "e2e")]
                if let Some(path) = std::env::var_os("HELIX_E2E_SHUTDOWN_REPORT") {
                    let report = serde_json::json!({
                        "hostPid": std::process::id(),
                        "kernelStopped": matches!(supervisor.status().await, SupervisorStatus::Stopped),
                    });
                    std::fs::write(path, report.to_string()).expect("write E2E shutdown receipt");
                }
                handle.exit(0);
            });
        }
        _ => {}
    });
}

fn main() {
    run();
}
