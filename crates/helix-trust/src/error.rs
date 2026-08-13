//! Errors from the workspace trust service (REQ-FS-005).

use thiserror::Error;

use crate::model::TrustCapability;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TrustError {
    #[error("workspace trust is disabled")]
    Disabled,
    #[error("'{path}' is in Restricted mode; trust the folder to use {capability}")]
    Restricted {
        path: String,
        capability: TrustCapability,
    },
    #[error(
        "the trust store is unreadable; all folders are in Restricted mode until it is repaired"
    )]
    StoreUnavailable,
    #[error("trust storage failed: {0}")]
    Storage(String),
    #[error("invalid path '{0}'")]
    InvalidPath(String),
    #[error("trust everything requires acknowledging the security warning")]
    WarningNotAcknowledged,
}

impl TrustError {
    pub fn storage(message: impl Into<String>) -> Self {
        Self::Storage(message.into())
    }
}
