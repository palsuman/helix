//! Common error type shared by kernel services.
//!
//! `AppError` is the type kernel services return internally. It carries a
//! stable `code`, a user-facing `message`, and an `ErrorCategory` so the IPC
//! layer (see `helix-ipc`) can map it to a typed frontend error without each
//! call site re-deriving that classification.

use serde::{Deserialize, Serialize};
use std::fmt;
use ts_rs::TS;

/// Broad classification of an error, used by the frontend to decide how to
/// react (retry, surface permanently, ignore because cancelled, etc).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    /// Likely to succeed on retry (network blip, lock contention).
    Transient,
    /// Will not succeed without a change in state (missing file, bad config).
    Permanent,
    /// The operation was cancelled by the caller.
    Cancelled,
    /// The operation exceeded its configured timeout.
    Timeout,
}

/// The error type returned by kernel service operations.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct AppError {
    pub code: String,
    pub category: ErrorCategory,
    pub message: String,
    #[ts(type = "unknown | null")]
    pub details: Option<serde_json::Value>,
}

impl AppError {
    pub fn new(
        code: impl Into<String>,
        category: ErrorCategory,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            category,
            message: message.into(),
            details: None,
        }
    }

    pub fn transient(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(code, ErrorCategory::Transient, message)
    }

    pub fn permanent(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(code, ErrorCategory::Permanent, message)
    }

    pub fn cancelled(message: impl Into<String>) -> Self {
        Self::new("CANCELLED", ErrorCategory::Cancelled, message)
    }

    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new("TIMEOUT", ErrorCategory::Timeout, message)
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_error_has_transient_category() {
        let err = AppError::transient("IO_RETRY", "disk busy");
        assert_eq!(err.category, ErrorCategory::Transient);
        assert_eq!(err.code, "IO_RETRY");
    }

    #[test]
    fn error_serializes_round_trip() {
        let err = AppError::permanent("NOT_FOUND", "file missing")
            .with_details(serde_json::json!({ "path": "/tmp/x" }));
        let json = serde_json::to_string(&err).unwrap();
        let back: AppError = serde_json::from_str(&json).unwrap();
        assert_eq!(back.code, "NOT_FOUND");
        assert_eq!(back.category, ErrorCategory::Permanent);
        assert!(back.details.is_some());
    }

    #[test]
    fn display_formats_code_and_message() {
        let err = AppError::cancelled("aborted by user");
        assert_eq!(format!("{err}"), "[CANCELLED] aborted by user");
    }
}
