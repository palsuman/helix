//! `helix-ipc` — the typed IPC command layer shared between the kernel and
//! the generated TypeScript client (Task 1.3, REQ-ARCH-003).
//!
//! Three pieces live here:
//!
//! - [`envelope`] — the request/response/cancel wire shapes. They derive
//!   `ts_rs::TS`, so `frontend/src/generated/` is produced from this Rust
//!   source of truth instead of being hand-maintained beside it.
//! - [`dispatcher`] — the transport-agnostic command registry and executor:
//!   correlation IDs, per-request timeouts, cancellation, and
//!   `Result<T, AppError>` to typed frontend error mapping.
//! - [`commands`] — the built-in commands (`ipc.ping`, `ipc.sleep`).
//!
//! The dispatcher has no Tauri dependency on purpose. `helix-supervisor`
//! terminates Tauri invokes and forwards these envelopes over authenticated
//! loopback RPC to `helix-kernel`. Tests can still drive the identical domain
//! dispatcher without either transport boundary.

pub mod commands;
pub mod dispatcher;
pub mod envelope;
pub mod internal_rpc;

pub use commands::{
    PING, PingRequest, PingResponse, SLEEP, SleepRequest, SleepResponse, register_builtins,
};
pub use dispatcher::{CancelToken, CommandContext, IpcDispatcher};
pub use envelope::{CancelRequest, CancelResponse, DEFAULT_TIMEOUT_MS, IpcRequest, IpcResponse};
pub use internal_rpc::{
    AuthenticatedRpcRequest, InternalRpcClient, InternalRpcRequest, InternalRpcResponse,
    KERNEL_EPOCH_ENV, KERNEL_LAUNCH_TOKEN_ENV, KERNEL_READY_PREFIX, KernelReady,
    MAX_INTERNAL_RPC_BYTES, serve_internal_rpc_request,
};
