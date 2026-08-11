//! Request and response payloads for the `config.*` IPC commands, and the
//! streaming channel change notifications travel on.
//!
//! They live beside the subsystem, as `log.*` and `stream.*` do, so the
//! `ts_rs` export in `frontend/src/generated/` tracks the subsystem rather than
//! the kernel's wiring. `helix-kernel` registers the handlers.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::jsonc::ConfigParseError;
use crate::layer::ConfigScope;
use crate::schema::SettingIssue;
use crate::service::SettingValue;

/// Command names, so the kernel and the generated client agree on the
/// strings.
pub const GET: &str = "config.get";
pub const SET: &str = "config.set";
pub const RESET: &str = "config.reset";
pub const LIST: &str = "config.list";
/// Not in the task's four-command list, but REQ-CONFIG-001.5 asks the JSON
/// settings editor to validate and complete against a schema, and the schema
/// lives in the kernel. Serving it is one handler; duplicating it in the
/// frontend would guarantee the two drift.
pub const SCHEMA: &str = "config.schema";

/// Streaming channel carrying the changed key set after every change
/// (REQ-CONFIG-001.8, .10). Every window subscribes, which is how a setting
/// changed in one window reaches the others.
pub const CHANNEL: &str = "config:changed";

/// `config.get` request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct ConfigGetRequest {
    /// Dotted key, e.g. `editor.fontSize`.
    pub key: String,
    /// Language id, when the caller wants the value that applies to a file of
    /// that language (REQ-CONFIG-001.2).
    pub language: Option<String>,
    /// Open workspace key when workspace or folder layers should participate.
    pub workspace_key: Option<String>,
    /// Path whose owning root supplies the folder layer.
    pub path: Option<String>,
}

/// `config.get` response. `setting` is absent for a key that is neither
/// declared nor set anywhere, which the frontend renders as "no such setting"
/// rather than as a null value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct ConfigGetResponse {
    pub setting: Option<SettingValue>,
}

/// `config.set` request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct ConfigSetRequest {
    pub scope: ConfigScope,
    pub key: String,
    #[ts(type = "unknown")]
    pub value: Value,
    pub language: Option<String>,
    pub workspace_key: Option<String>,
    pub path: Option<String>,
}

impl Default for ConfigSetRequest {
    fn default() -> Self {
        Self {
            scope: ConfigScope::User,
            key: String::new(),
            value: Value::Null,
            language: None,
            workspace_key: None,
            path: None,
        }
    }
}

/// `config.reset` request (REQ-CONFIG-001.9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct ConfigResetRequest {
    pub scope: ConfigScope,
    pub key: String,
    pub language: Option<String>,
    pub workspace_key: Option<String>,
    pub path: Option<String>,
}

impl Default for ConfigResetRequest {
    fn default() -> Self {
        Self {
            scope: ConfigScope::User,
            key: String::new(),
            language: None,
            workspace_key: None,
            path: None,
        }
    }
}

/// Result of a write: what moved, what needs a restart, and the setting's new
/// effective state, so the caller does not have to follow up with a `get`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct ConfigWriteResponse {
    pub scope: ConfigScope,
    pub changed_keys: Vec<String>,
    pub requires_restart: Vec<String>,
    pub setting: Option<SettingValue>,
}

/// `config.list` request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct ConfigListRequest {
    /// Dotted-key prefix, e.g. `editor.` to list one category.
    pub prefix: Option<String>,
    pub language: Option<String>,
    pub workspace_key: Option<String>,
    pub path: Option<String>,
}

/// `config.list` response.
///
/// Parse errors and per-key issues ride along with the list because the
/// settings editor has to show them at the top of the view it is already
/// rendering (REQ-CONFIG-001 failure modes), not fetch them separately and
/// risk showing a stale pairing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct ConfigListResponse {
    pub settings: Vec<SettingValue>,
    pub parse_errors: Vec<ConfigParseError>,
    pub issues: Vec<SettingIssue>,
    /// Layers that have a file behind them, lowest precedence first, so the
    /// scope selector only offers scopes that can actually be written.
    pub scopes: Vec<ConfigScopeInfo>,
}

/// One layer and where it lives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct ConfigScopeInfo {
    pub scope: ConfigScope,
    pub path: Option<String>,
    pub writable: bool,
}

/// `config.schema` request. Empty: the schema is a property of the build.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct ConfigSchemaRequest {}

/// `config.schema` response: a JSON Schema document for the JSON settings
/// editor (REQ-CONFIG-001.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct ConfigSchemaResponse {
    #[ts(type = "unknown")]
    pub schema: Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_names_are_namespaced_under_config() {
        for name in [GET, SET, RESET, LIST, SCHEMA] {
            assert!(name.starts_with("config."), "{name}");
        }
        assert_eq!(CHANNEL, "config:changed");
    }

    #[test]
    fn a_get_request_needs_only_a_key() {
        let request: ConfigGetRequest =
            serde_json::from_str(r#"{"key":"editor.fontSize"}"#).unwrap();
        assert_eq!(request.key, "editor.fontSize");
        assert_eq!(request.language, None);
    }

    #[test]
    fn a_set_request_defaults_to_the_user_layer() {
        let request: ConfigSetRequest =
            serde_json::from_str(r#"{"key":"editor.fontSize","value":16}"#).unwrap();
        assert_eq!(request.scope, ConfigScope::User);
        assert_eq!(request.value, serde_json::json!(16));
    }

    #[test]
    fn a_scope_round_trips_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&ConfigScope::Workspace).unwrap(),
            "\"workspace\""
        );
        let request: ConfigResetRequest =
            serde_json::from_str(r#"{"scope":"folder","key":"editor.tabSize"}"#).unwrap();
        assert_eq!(request.scope, ConfigScope::Folder);
    }
}
