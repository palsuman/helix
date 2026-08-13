//! Trust model types (REQ-FS-005).

use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// User decision for a folder path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum TrustDecision {
    /// Explicitly trusted; workspace-supplied execution is allowed.
    Trusted,
    /// Explicitly restricted; execution is blocked.
    Restricted,
    /// No decision recorded yet — treated as Restricted until the user chooses.
    Unknown,
}

impl TrustDecision {
    pub fn is_trusted(self) -> bool {
        matches!(self, Self::Trusted)
    }
}

/// Effective mode for a workspace window (REQ-FS-005.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceTrustMode {
    Trusted,
    Restricted,
}

/// Capabilities blocked in Restricted mode (REQ-FS-005.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum TrustCapability {
    LanguageServerLaunch,
    DebugAdapterLaunch,
    TaskExecution,
    TaskAutoDetection,
    McpServerLaunch,
    WorkspaceFormatter,
    WorkspacePluginActivation,
    AgentExecution,
    ExecutablePathSetting,
}

impl TrustCapability {
    pub fn label(self) -> &'static str {
        match self {
            Self::LanguageServerLaunch => "language servers",
            Self::DebugAdapterLaunch => "debug adapters",
            Self::TaskExecution => "task execution",
            Self::TaskAutoDetection => "task auto-detection",
            Self::McpServerLaunch => "MCP servers",
            Self::WorkspaceFormatter => "workspace formatters",
            Self::WorkspacePluginActivation => "workspace-recommended plugins",
            Self::AgentExecution => "the autonomous agent",
            Self::ExecutablePathSetting => "executable path settings",
        }
    }
}

impl fmt::Display for TrustCapability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// One persisted trust entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustEntry {
    pub decision: TrustDecision,
    pub inherit_to_children: bool,
    pub granted_ms: u64,
}

/// Trust state for one workspace root, returned to the frontend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct RootTrustStatus {
    pub path: String,
    pub decision: TrustDecision,
    pub inherited_from: Option<String>,
}
