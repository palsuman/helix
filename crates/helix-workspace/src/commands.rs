//! Request and response payloads for the `workspace.*` IPC commands, and the
//! streaming channel workspace changes travel on.
//!
//! They live beside the subsystem, as `config.*` and `fs.*` do, so the `ts_rs`
//! export in `frontend/src/generated/` tracks the subsystem rather than the
//! kernel's wiring. `helix-kernel` registers the handlers.
//!
//! Adding and removing a root are commands rather than a settings write because
//! REQ-FS-001.4 puts them behind the command palette and the explorer context
//! menu, and both of those dispatch commands. The workspace document is written
//! as a consequence, not as the mechanism.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::recent::RecentWorkspace;
use crate::service::WorkspaceSnapshot;

/// Command names, so the kernel and the generated client agree on the strings.
pub const OPEN: &str = "workspace.open";
pub const CLOSE: &str = "workspace.close";
pub const LIST: &str = "workspace.list";
pub const ADD_ROOT: &str = "workspace.addRoot";
pub const REMOVE_ROOT: &str = "workspace.removeRoot";
pub const SETTINGS: &str = "workspace.settings";
pub const RECENT: &str = "workspace.recent";
pub const FORGET_RECENT: &str = "workspace.forgetRecent";
pub const REFRESH: &str = "workspace.refresh";
pub const SCHEMA: &str = "workspace.schema";

/// Streaming channel carrying workspace lifecycle and root changes. Every
/// window subscribes, which is how a root added in one window reaches the
/// explorer in another window bound to the same workspace.
pub const CHANNEL: &str = "workspace:changed";

/// `workspace.open` request. The first root is the primary: the one whose
/// `.helix/workspace.json` and workspace settings apply.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct WorkspaceOpenRequest {
    pub roots: Vec<String>,
    /// Display name for a workspace with no document yet. Ignored when the
    /// document names itself.
    pub name: Option<String>,
}

/// `workspace.open`, `workspace.addRoot`, and `workspace.removeRoot` all answer
/// with the resulting workspace, so a caller never has to follow up with a
/// read to find out what happened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct WorkspaceResponse {
    pub workspace: WorkspaceSnapshot,
}

/// `workspace.close` request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct WorkspaceCloseRequest {
    pub key: String,
}

/// `workspace.close` response.
///
/// `torn_down` is false when another window still holds the workspace, which is
/// the reference-counted behaviour the design document requires: closing one of
/// two windows on a workspace must not stop its language servers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct WorkspaceCloseResponse {
    pub closed: bool,
    pub torn_down: bool,
    pub remaining_holders: u32,
}

/// `workspace.list` and `workspace.refresh` request. Empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct WorkspaceListRequest {}

/// Every open workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct WorkspaceListResponse {
    pub workspaces: Vec<WorkspaceSnapshot>,
}

/// `workspace.addRoot` and `workspace.removeRoot` request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct WorkspaceRootRequest {
    /// Workspace key, from a previous `workspace.open`.
    pub key: String,
    pub path: String,
    /// Display name for the new root. Only meaningful for `addRoot`.
    pub name: Option<String>,
}

/// `workspace.settings` request: the effective settings for a path, honouring
/// per-folder overrides (REQ-FS-001.3).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct WorkspaceSettingsRequest {
    pub key: String,
    /// A path inside the workspace. Omitted asks for the workspace-level view,
    /// with no folder layer applied.
    pub path: Option<String>,
    /// A single dotted setting key, when the caller wants one value rather than
    /// the whole tree.
    pub setting: Option<String>,
    /// Language id whose overrides should participate in resolution.
    pub language: Option<String>,
}

/// `workspace.settings` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct WorkspaceSettingsResponse {
    /// The root that owned the requested path, so the settings editor can say
    /// which folder's settings are in play.
    pub root: Option<String>,
    #[ts(type = "unknown")]
    pub settings: Value,
    /// Present when `setting` was given.
    #[ts(type = "unknown | null")]
    pub value: Option<Value>,
}

/// `workspace.recent` request. Empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct WorkspaceRecentRequest {}

/// `workspace.recent` response, most recently opened first (REQ-FS-001.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct WorkspaceRecentResponse {
    pub entries: Vec<RecentWorkspace>,
}

/// `workspace.forgetRecent` request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct WorkspaceForgetRecentRequest {
    pub key: String,
}

/// `workspace.forgetRecent` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct WorkspaceForgetRecentResponse {
    pub forgotten: bool,
}

/// `workspace.schema` request. Empty: the schema is a property of the build.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct WorkspaceSchemaRequest {}

/// `workspace.schema` response: the JSON Schema for `.helix/workspace.json`,
/// so a hand-edited document validates in the editor (REQ-FS-001.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct WorkspaceSchemaResponse {
    #[ts(type = "unknown")]
    pub schema: Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_names_are_namespaced_under_workspace() {
        for name in [
            OPEN,
            CLOSE,
            LIST,
            ADD_ROOT,
            REMOVE_ROOT,
            SETTINGS,
            RECENT,
            FORGET_RECENT,
            REFRESH,
            SCHEMA,
        ] {
            assert!(name.starts_with("workspace."), "{name}");
        }
        assert_eq!(CHANNEL, "workspace:changed");
    }

    #[test]
    fn an_open_request_needs_only_roots() {
        let request: WorkspaceOpenRequest =
            serde_json::from_str(r#"{"roots":["/work/api","/work/web"]}"#).unwrap();
        assert_eq!(request.roots.len(), 2);
        assert_eq!(request.name, None);
    }

    #[test]
    fn a_root_request_round_trips() {
        let request: WorkspaceRootRequest =
            serde_json::from_str(r#"{"key":"abc","path":"/work/tools"}"#).unwrap();
        assert_eq!(request.key, "abc");
        assert_eq!(request.path, "/work/tools");
        assert!(request.name.is_none());
    }

    #[test]
    fn a_settings_request_may_ask_for_one_key() {
        let request: WorkspaceSettingsRequest = serde_json::from_str(
            r#"{"key":"abc","path":"/work/web/app.ts","setting":"editor.tabSize"}"#,
        )
        .unwrap();
        assert_eq!(request.setting.as_deref(), Some("editor.tabSize"));
    }
}
