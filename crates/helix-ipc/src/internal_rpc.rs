//! Private Host-to-kernel transport envelopes.
//!
//! These types never cross into the WebView. The per-launch token rejects
//! stale or unrelated local peers, while the nested public IPC envelopes keep
//! correlation, timeout, cancellation, and typed errors identical across both
//! transport boundaries.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use crate::{CancelRequest, CancelResponse, IpcDispatcher, IpcRequest, IpcResponse};

pub const KERNEL_LAUNCH_TOKEN_ENV: &str = "HELIX_KERNEL_LAUNCH_TOKEN";
pub const KERNEL_EPOCH_ENV: &str = "HELIX_KERNEL_EPOCH";
pub const KERNEL_READY_PREFIX: &str = "HELIX_READY ";
pub const KERNEL_CRASH_HANDOFF_ENV: &str = "HELIX_KERNEL_CRASH_HANDOFF";
pub const KERNEL_SAFE_MODE_ENV: &str = "HELIX_SAFE_MODE";
pub const KERNEL_SKIP_SESSION_RESTORE_ENV: &str = "HELIX_SKIP_SESSION_RESTORE";
pub const MAX_INTERNAL_RPC_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
pub struct KernelReady {
    pub port: u16,
    pub process_id: u32,
    pub epoch: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AuthenticatedRpcRequest {
    pub launch_token: String,
    pub epoch: String,
    pub request: InternalRpcRequest,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum InternalRpcRequest {
    Dispatch(IpcRequest<serde_json::Value>),
    Cancel(CancelRequest),
    /// Transport liveness probe. This deliberately invokes no domain handler.
    Health,
    /// Clean-quit handshake. The acknowledgement is written before shutdown begins.
    Shutdown,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum InternalRpcResponse {
    Dispatch(IpcResponse<serde_json::Value>),
    Cancel(CancelResponse),
    Health { epoch: String },
    ShutdownAcknowledged,
    ProtocolError { message: String },
}

/// Host-side client for one-request-per-connection internal RPC.
#[derive(Clone, Debug)]
pub struct InternalRpcClient {
    address: String,
    launch_token: String,
    epoch: String,
}

impl InternalRpcClient {
    pub fn new(
        address: impl Into<String>,
        launch_token: impl Into<String>,
        epoch: impl Into<String>,
    ) -> Self {
        Self {
            address: address.into(),
            launch_token: launch_token.into(),
            epoch: epoch.into(),
        }
    }

    pub async fn call(
        &self,
        request: InternalRpcRequest,
        timeout: Duration,
    ) -> io::Result<InternalRpcResponse> {
        tokio::time::timeout(timeout, self.call_inner(request))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "kernel transport timed out"))?
    }

    async fn call_inner(&self, request: InternalRpcRequest) -> io::Result<InternalRpcResponse> {
        let mut stream = TcpStream::connect(&self.address).await?;
        let frame = AuthenticatedRpcRequest {
            launch_token: self.launch_token.clone(),
            epoch: self.epoch.clone(),
            request,
        };
        write_frame(&mut stream, &frame).await?;

        let mut reader = BufReader::new(stream);
        let response = read_frame(&mut reader).await?;
        serde_json::from_slice(&response).map_err(invalid_data)
    }
}

/// Serve one authenticated request using the same framing as
/// [`InternalRpcClient`]. The kernel accepts connections and delegates each
/// one here, keeping all protocol behavior in one tested crate.
pub async fn serve_internal_rpc_request(
    stream: TcpStream,
    launch_token: &str,
    epoch: &str,
    dispatcher: Arc<IpcDispatcher>,
) -> io::Result<()> {
    serve_internal_rpc_request_with_shutdown(stream, launch_token, epoch, dispatcher, None).await
}

/// Controlled variant used by the kernel executable. Tests and embedders that
/// do not own process lifecycle keep using [`serve_internal_rpc_request`].
pub async fn serve_internal_rpc_request_with_shutdown(
    stream: TcpStream,
    launch_token: &str,
    epoch: &str,
    dispatcher: Arc<IpcDispatcher>,
    shutdown: Option<tokio::sync::mpsc::Sender<()>>,
) -> io::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut request_shutdown = false;
    let response = match read_frame(&mut reader).await {
        Ok(frame) => match serde_json::from_slice::<AuthenticatedRpcRequest>(&frame) {
            Ok(frame) if frame.launch_token == launch_token && frame.epoch == epoch => {
                match frame.request {
                    InternalRpcRequest::Dispatch(request) => {
                        InternalRpcResponse::Dispatch(dispatcher.dispatch(request).await)
                    }
                    InternalRpcRequest::Cancel(request) => {
                        let cancelled = dispatcher.cancel(&request.correlation_id);
                        InternalRpcResponse::Cancel(CancelResponse {
                            correlation_id: request.correlation_id,
                            cancelled,
                        })
                    }
                    InternalRpcRequest::Health => InternalRpcResponse::Health {
                        epoch: epoch.to_string(),
                    },
                    InternalRpcRequest::Shutdown if shutdown.is_some() => {
                        request_shutdown = true;
                        InternalRpcResponse::ShutdownAcknowledged
                    }
                    InternalRpcRequest::Shutdown => InternalRpcResponse::ProtocolError {
                        message: "kernel shutdown is not available in this server".into(),
                    },
                }
            }
            Ok(_) => InternalRpcResponse::ProtocolError {
                message: "unauthorized or stale kernel peer".into(),
            },
            Err(error) => InternalRpcResponse::ProtocolError {
                message: format!("invalid internal RPC request: {error}"),
            },
        },
        Err(error) => InternalRpcResponse::ProtocolError {
            message: error.to_string(),
        },
    };

    let mut stream = reader.into_inner();
    write_frame(&mut stream, &response).await?;
    stream.shutdown().await?;
    if request_shutdown && let Some(shutdown) = shutdown {
        let _ = shutdown.send(()).await;
    }
    Ok(())
}

async fn read_frame<R: AsyncBufRead + Unpin>(reader: &mut R) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut limited = reader.take((MAX_INTERNAL_RPC_BYTES + 2) as u64);
    limited.read_until(b'\n', &mut bytes).await?;
    if bytes.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "internal RPC peer closed without a response",
        ));
    }
    if bytes.last() != Some(&b'\n') || bytes.len() > MAX_INTERNAL_RPC_BYTES + 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "internal RPC frame exceeds size limit or is unterminated",
        ));
    }
    bytes.pop();
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    Ok(bytes)
}

async fn write_frame<T: Serialize>(stream: &mut TcpStream, value: &T) -> io::Result<()> {
    let mut encoded = serde_json::to_vec(value).map_err(invalid_data)?;
    if encoded.len() > MAX_INTERNAL_RPC_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "internal RPC frame exceeds size limit",
        ));
    }
    encoded.push(b'\n');
    stream.write_all(&encoded).await
}

fn invalid_data(error: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}
