//! Errors from the secret store (REQ-SEC-002).

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SecretError {
    #[error("secret '{namespace}/{name}' was not found")]
    NotFound { namespace: String, name: String },
    #[error("access to namespace '{namespace}' is not permitted for this caller")]
    NamespaceDenied { namespace: String },
    #[error("the OS keychain is unavailable: {reason}")]
    KeychainUnavailable { reason: String },
    #[error("the encrypted fallback store is locked; unlock it with a master password")]
    FallbackLocked,
    #[error("the master password is incorrect")]
    InvalidMasterPassword,
    #[error("secret storage failed: {0}")]
    Storage(String),
    #[error("invalid secret name '{0}'")]
    InvalidName(String),
}

impl SecretError {
    pub fn storage(message: impl Into<String>) -> Self {
        Self::Storage(message.into())
    }
}
