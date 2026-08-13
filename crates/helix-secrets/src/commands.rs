//! Request and response payloads for the `secrets.*` IPC commands.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use zeroize::Zeroize;

pub const STORE: &str = "secrets.store";
pub const DELETE: &str = "secrets.delete";
pub const LIST: &str = "secrets.list";
pub const EXISTS: &str = "secrets.exists";
pub const UNLOCK: &str = "secrets.unlock";
pub const STATUS: &str = "secrets.status";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum SecretBackendKind {
    Keychain,
    EncryptedFile,
    Memory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SecretRef {
    pub namespace: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SecretsStoreRequest {
    pub namespace: String,
    pub name: String,
    pub value: String,
}

impl Drop for SecretsStoreRequest {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SecretsStoreResponse {
    pub stored: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SecretsDeleteRequest {
    pub namespace: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SecretsDeleteResponse {
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct SecretsListRequest {
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SecretsListResponse {
    pub entries: Vec<SecretRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SecretsExistsRequest {
    pub namespace: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SecretsExistsResponse {
    pub exists: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SecretsUnlockRequest {
    pub master_password: String,
}

impl Drop for SecretsUnlockRequest {
    fn drop(&mut self) {
        self.master_password.zeroize();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SecretsUnlockResponse {
    pub unlocked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SecretsStatusResponse {
    pub backend: SecretBackendKind,
    pub fallback_unlocked: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_names_are_namespaced_under_secrets() {
        for name in [STORE, DELETE, LIST, EXISTS, UNLOCK, STATUS] {
            assert!(name.starts_with("secrets."), "{name}");
        }
    }
}
