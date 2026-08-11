//! Thin Tauri Host: window ownership, invoke termination, and authenticated
//! forwarding to the authoritative kernel process.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::error::Error;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use helix_ipc::{
    CancelRequest, CancelResponse, DEFAULT_TIMEOUT_MS, InternalRpcClient, InternalRpcRequest,
    InternalRpcResponse, IpcRequest, IpcResponse, KERNEL_EPOCH_ENV, KERNEL_LAUNCH_TOKEN_ENV,
    KERNEL_READY_PREFIX, KernelReady,
};
use tauri::State;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, RwLock};

struct KernelConnection {
    rpc: InternalRpcClient,
    child: Mutex<Child>,
    #[cfg(feature = "ipc-e2e")]
    address: String,
    #[cfg(feature = "ipc-e2e")]
    launch_token: String,
    #[cfg(feature = "ipc-e2e")]
    epoch: String,
}

impl KernelConnection {
    async fn launch() -> Result<Self, Box<dyn Error + Send + Sync>> {
        let launch_token = uuid::Uuid::new_v4().to_string();
        let epoch = uuid::Uuid::new_v4().to_string();
        let mut child = Command::new(kernel_binary_path()?)
            .env(KERNEL_LAUNCH_TOKEN_ENV, &launch_token)
            .env(KERNEL_EPOCH_ENV, &epoch)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()?;
        let stdout = child.stdout.take().ok_or("kernel stdout was not piped")?;
        let mut reader = BufReader::new(stdout);
        let ready = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).await? == 0 {
                    return Err::<KernelReady, Box<dyn Error + Send + Sync>>(
                        "kernel exited before readiness handshake".into(),
                    );
                }
                if let Some(payload) = line.strip_prefix(KERNEL_READY_PREFIX) {
                    return Ok::<KernelReady, Box<dyn Error + Send + Sync>>(serde_json::from_str(
                        payload,
                    )?);
                }
                eprint!("{line}");
            }
        })
        .await
        .map_err(|_| "kernel readiness handshake timed out")??;
        if ready.epoch != epoch {
            return Err("kernel readiness handshake carried a stale epoch".into());
        }
        tokio::spawn(async move {
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => eprint!("{line}"),
                }
            }
        });
        let address = format!("127.0.0.1:{}", ready.port);
        Ok(Self {
            rpc: InternalRpcClient::new(&address, &launch_token, &epoch),
            child: Mutex::new(child),
            #[cfg(feature = "ipc-e2e")]
            address,
            #[cfg(feature = "ipc-e2e")]
            launch_token,
            #[cfg(feature = "ipc-e2e")]
            epoch,
        })
    }

    async fn call(
        &self,
        request: InternalRpcRequest,
        timeout: Duration,
    ) -> Result<InternalRpcResponse, String> {
        self.rpc
            .call(request, timeout)
            .await
            .map_err(|error| error.to_string())
    }

    fn terminate(&self) {
        if let Ok(mut child) = self.child.try_lock() {
            let _ = child.start_kill();
        }
    }
}

struct KernelClient {
    connection: RwLock<Arc<KernelConnection>>,
}

impl KernelClient {
    async fn launch() -> Result<Self, Box<dyn Error + Send + Sync>> {
        Ok(Self {
            connection: RwLock::new(Arc::new(KernelConnection::launch().await?)),
        })
    }

    async fn call(
        &self,
        request: InternalRpcRequest,
        timeout: Duration,
    ) -> Result<InternalRpcResponse, String> {
        let connection = self.connection.read().await.clone();
        connection.call(request, timeout).await
    }

    fn terminate(&self) {
        if let Ok(connection) = self.connection.try_read() {
            connection.terminate();
        }
    }

    #[cfg(feature = "ipc-e2e")]
    async fn restart_and_probe_stale_peer(&self) -> Result<bool, String> {
        let replacement = Arc::new(
            KernelConnection::launch()
                .await
                .map_err(|error| error.to_string())?,
        );
        let (previous, stale_client) = {
            let mut connection = self.connection.write().await;
            let previous = std::mem::replace(&mut *connection, replacement.clone());
            let stale_client = InternalRpcClient::new(
                &replacement.address,
                &previous.launch_token,
                &previous.epoch,
            );
            (previous, stale_client)
        };
        previous.terminate();

        let response = stale_client
            .call(
                InternalRpcRequest::Dispatch(IpcRequest::new(
                    helix_ipc::PING,
                    "ipc-e2e-stale-peer",
                    serde_json::json!({ "message": "must be rejected" }),
                )),
                Duration::from_secs(5),
            )
            .await
            .map_err(|error| error.to_string())?;
        Ok(matches!(
            response,
            InternalRpcResponse::ProtocolError { message }
                if message.contains("unauthorized or stale")
        ))
    }
}

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
    client: State<'_, Arc<KernelClient>>,
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
    let response = match client
        .call(InternalRpcRequest::Dispatch(request), timeout)
        .await
    {
        Ok(response) => response,
        Err(error) => {
            let _ = client
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
        InternalRpcResponse::Cancel(_) => Err("kernel returned an unexpected response".into()),
    }
}

#[tauri::command]
async fn ipc_cancel(
    client: State<'_, Arc<KernelClient>>,
    request: CancelRequest,
) -> Result<CancelResponse, String> {
    match client
        .call(InternalRpcRequest::Cancel(request), Duration::from_secs(5))
        .await?
    {
        InternalRpcResponse::Cancel(response) => Ok(response),
        InternalRpcResponse::ProtocolError { message } => Err(message),
        InternalRpcResponse::Dispatch(_) => Err("kernel returned an unexpected response".into()),
    }
}

#[cfg(feature = "ipc-e2e")]
#[tauri::command]
async fn ipc_e2e_restart(client: State<'_, Arc<KernelClient>>) -> Result<bool, String> {
    client.restart_and_probe_stale_peer().await
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
    let client = Arc::new(
        tauri::async_runtime::block_on(KernelClient::launch())
            .expect("failed to launch helix-kernel"),
    );
    let shutdown_client = client.clone();
    let builder = tauri::Builder::default().manage(client);
    #[cfg(not(feature = "ipc-e2e"))]
    let builder = builder.invoke_handler(tauri::generate_handler![ipc_dispatch, ipc_cancel]);
    #[cfg(feature = "ipc-e2e")]
    let builder = builder.invoke_handler(tauri::generate_handler![
        ipc_dispatch,
        ipc_cancel,
        ipc_e2e_restart,
        ipc_e2e_report
    ]);
    let app = builder
        .build(tauri::generate_context!())
        .expect("failed to build Helix Host");
    app.run(move |_handle, event| {
        if let tauri::RunEvent::Exit = event {
            shutdown_client.terminate();
        }
    });
}

fn main() {
    run();
}
