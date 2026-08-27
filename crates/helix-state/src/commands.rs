//! Typed IPC contract for durable workbench layout state (Task 2.1).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

pub const LAYOUT_GET: &str = "state.layout.get";
pub const LAYOUT_SET: &str = "state.layout.set";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct LayoutGetRequest {
    pub workspace_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct LayoutGetResponse {
    #[ts(type = "unknown")]
    pub layout: Value,
    /// Stable content hash used by frontend projection reconciliation.
    pub projection_hash: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct LayoutSetRequest {
    pub workspace_key: String,
    #[ts(type = "unknown")]
    pub layout: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct LayoutSetResponse {
    pub persisted: bool,
    pub projection_hash: String,
}

pub fn layout_projection_hash(layout: &Value) -> Result<String, serde_json::Error> {
    let bytes = serde_json::to_vec(layout)?;
    Ok(format!("{:08x}", crc32fast::hash(&bytes)))
}
