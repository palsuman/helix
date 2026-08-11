//! `helix-stream` — the local WebSocket streaming layer (Task 1.4,
//! REQ-ARCH-003.5-.10).
//!
//! Tauri IPC (see `helix-ipc`) carries request-response commands. This crate
//! carries everything high-frequency and one-directional: terminal output,
//! agent progress, log tailing, diagnostics push, search results, health
//! status. The design document's rationale for the split is backpressure,
//! ordering, and per-channel routing, none of which Tauri's event system
//! provides.
//!
//! Three pieces live here:
//!
//! - [`envelope`] — the wire shapes, deriving `ts_rs::TS` so
//!   `frontend/src/generated/` is produced from this Rust source of truth.
//! - [`ring`] — the bounded per-channel history that gives backpressure its
//!   oldest-dropped semantics and gives reconnects something to resume from.
//! - [`hub`] — channel sequencing, subscriptions as cursors, and the
//!   backpressure accounting built on top of the ring.
//! - [`server`] — the loopback WebSocket server: random free port, token
//!   handshake, heartbeats.
//!
//! As with the IPC dispatcher, the hub knows nothing about its transport, so
//! ordering and backpressure are testable without a socket and
//! REQ-REMOTE-001.2 stays satisfiable.

pub mod envelope;
pub mod hub;
pub mod ring;
pub mod server;

pub use envelope::{
    ChannelSubscription, HEARTBEAT_INTERVAL_MS, MISSED_HEARTBEAT_LIMIT, StreamControl,
    StreamEndpoint, StreamEndpointRequest, StreamEnvelope, StreamFrame,
};
pub use hub::{HubConfig, HubMetrics, SessionHandle, StreamHub};
pub use ring::{DEFAULT_BUFFER_DEPTH, RingBuffer};
pub use server::{STREAM_PATH, ServerConfig, ServerMetrics, StreamServer};
