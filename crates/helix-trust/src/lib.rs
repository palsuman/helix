//! `helix-trust` — workspace trust enforcement (Task 1.13, REQ-FS-005).

pub mod commands;
pub mod error;
pub mod gate;
pub mod model;
pub mod service;
pub mod store;

pub use commands::{
    LIST, PROBE, REVOKE, SET, SET_TRUST_EVERYTHING, STATUS, TrustEverythingRequest,
    TrustEverythingResponse, TrustListResponse, TrustProbeRequest, TrustProbeResponse,
    TrustRevokeRequest, TrustRevokeResponse, TrustSetRequest, TrustSetResponse, TrustStatusRequest,
    TrustStatusResponse, TrustedFolderEntry,
};
pub use error::TrustError;
pub use gate::{ManagedProcess, ProcessRegistry};
pub use model::{RootTrustStatus, TrustCapability, TrustDecision, TrustEntry, WorkspaceTrustMode};
pub use service::{CHANNEL, LOG_SOURCE, TrustService};
pub use store::{StoreHealth, TrustStore, default_store_path};
