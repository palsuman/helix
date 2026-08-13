//! Request and response payloads for the `trust.*` IPC commands.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{RootTrustStatus, TrustCapability, TrustDecision, WorkspaceTrustMode};

pub const STATUS: &str = "trust.status";
pub const SET: &str = "trust.set";
pub const REVOKE: &str = "trust.revoke";
pub const LIST: &str = "trust.list";
pub const SET_TRUST_EVERYTHING: &str = "trust.setTrustEverything";
pub const PROBE: &str = "trust.probe";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct TrustStatusRequest {
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct TrustStatusResponse {
    pub enabled: bool,
    pub trust_everything: bool,
    pub store_healthy: bool,
    pub workspace_mode: WorkspaceTrustMode,
    pub roots: Vec<RootTrustStatus>,
    pub pending_prompts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct TrustSetRequest {
    pub path: String,
    pub decision: TrustDecision,
    pub inherit_to_children: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct TrustSetResponse {
    pub applied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct TrustRevokeRequest {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct TrustRevokeResponse {
    pub revoked: bool,
    pub terminated_processes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct TrustListResponse {
    pub entries: Vec<TrustedFolderEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct TrustedFolderEntry {
    pub path: String,
    pub inherit_to_children: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct TrustEverythingRequest {
    pub enabled: bool,
    pub acknowledged_warning: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct TrustEverythingResponse {
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct TrustProbeRequest {
    pub path: String,
    pub capability: TrustCapability,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct TrustProbeResponse {
    pub allowed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_names_are_namespaced_under_trust() {
        for name in [STATUS, SET, REVOKE, LIST, SET_TRUST_EVERYTHING, PROBE] {
            assert!(name.starts_with("trust."), "{name}");
        }
    }
}
