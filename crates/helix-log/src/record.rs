//! The log record model (REQ-OBS-001.1) and its level ordering
//! (REQ-OBS-001.2).
//!
//! The shape is the design document's "Log Record Model": timestamp, level,
//! source, correlation ID, message, and structured fields. Field names are
//! serialized in `snake_case`, matching every other wire type in the
//! workspace (`IpcRequest`, `StreamEnvelope`), rather than the illustrative
//! camelCase of the design snippet — one convention across the transport is
//! worth more than a literal match to an example.
//!
//! One record is one line of JSON. The line is produced by
//! [`LogRecord::to_json_line`] and consumed unchanged by the file sink, the
//! stdout sink, and the export path, so the format a user reports a bug with
//! is byte-identical to what the viewer showed them.

use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Structured fields attached to a record. A JSON object, so a field value
/// can be a number, a bool, or a nested object without the emitter having
/// to stringify it first.
pub type Fields = serde_json::Map<String, serde_json::Value>;

/// Severity, ordered from most to least verbose (REQ-OBS-001.2).
///
/// The `Ord` derive follows declaration order, so `Trace < Debug < Info <
/// Warn < Error` and a level check is a single comparison.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS, Default,
)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

impl LogLevel {
    /// Every level, most verbose first. Used by the viewer to build its
    /// level filter without hard-coding the list in TypeScript.
    pub const ALL: [LogLevel; 5] = [
        LogLevel::Trace,
        LogLevel::Debug,
        LogLevel::Info,
        LogLevel::Warn,
        LogLevel::Error,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Trace => "trace",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }

    /// Rank used by the level fast path, which compares a `u8` loaded from
    /// an atomic rather than taking a lock.
    pub fn rank(&self) -> u8 {
        *self as u8
    }

    pub fn parse(value: &str) -> Option<LogLevel> {
        match value.trim().to_ascii_lowercase().as_str() {
            "trace" => Some(LogLevel::Trace),
            "debug" => Some(LogLevel::Debug),
            "info" => Some(LogLevel::Info),
            "warn" | "warning" => Some(LogLevel::Warn),
            "error" => Some(LogLevel::Error),
            _ => None,
        }
    }

    /// Inverse of [`LogLevel::rank`], for a level recovered from the
    /// logger's cached atomic threshold.
    pub fn from_rank(rank: u8) -> LogLevel {
        match rank {
            0 => LogLevel::Trace,
            1 => LogLevel::Debug,
            2 => LogLevel::Info,
            3 => LogLevel::Warn,
            _ => LogLevel::Error,
        }
    }
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One structured log entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct LogRecord {
    /// RFC 3339 UTC with millisecond precision, e.g.
    /// `2026-08-07T10:30:00.123Z`. Fixed width, so a lexicographic
    /// comparison is also a chronological one, which is what lets the
    /// viewer's time-range filter work on the string without parsing.
    pub ts: String,
    pub level: LogLevel,
    /// The emitting service or module, e.g. `lsp_host`, `kernel.ipc`,
    /// `frontend.app`. Dot-separated segments make per-module level
    /// configuration a prefix match.
    pub source: String,
    /// The IPC correlation ID in scope when the record was emitted
    /// (REQ-OBS-001.9). Populated automatically inside a command handler.
    pub correlation_id: Option<String>,
    pub message: String,
    #[ts(type = "Record<string, unknown>")]
    pub fields: Fields,
}

impl LogRecord {
    /// A record stamped with the current time.
    pub fn new(level: LogLevel, source: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            ts: crate::time::now_rfc3339_millis(),
            level,
            source: source.into(),
            correlation_id: None,
            message: message.into(),
            fields: Fields::new(),
        }
    }

    /// A record with an explicit timestamp, for a frontend record that was
    /// captured before it could be shipped to the kernel, and for tests
    /// that need deterministic ordering.
    pub fn at(
        ts: impl Into<String>,
        level: LogLevel,
        source: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            ts: ts.into(),
            level,
            source: source.into(),
            correlation_id: None,
            message: message.into(),
            fields: Fields::new(),
        }
    }

    pub fn with_correlation_id(mut self, correlation_id: impl Into<String>) -> Self {
        self.correlation_id = Some(correlation_id.into());
        self
    }

    pub fn with_fields(mut self, fields: Fields) -> Self {
        self.fields = fields;
        self
    }

    pub fn with_field(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.fields.insert(key.into(), value);
        self
    }

    /// The record as one line of JSON, without the trailing newline.
    ///
    /// Serialization of a record cannot fail (every field is a plain string,
    /// enum, or JSON value), so the error arm degrades to a minimal
    /// hand-built line rather than panicking inside a logging call.
    pub fn to_json_line(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            format!(
                "{{\"ts\":\"{}\",\"level\":\"error\",\"source\":\"log\",\"correlation_id\":null,\"message\":\"a log record could not be serialized\",\"fields\":{{}}}}",
                self.ts
            )
        })
    }
}

/// Convert a value into a field value. Used by the logging macros so a call
/// site can pass a `&str`, a number, or any `Serialize` type.
///
/// A type that fails to serialize becomes `null` rather than taking the
/// process down: a diagnostic facility must never be the thing that crashes
/// the program it is diagnosing.
pub fn to_field<T: Serialize>(value: T) -> serde_json::Value {
    serde_json::to_value(value).unwrap_or(serde_json::Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_are_ordered_from_trace_to_error() {
        assert!(LogLevel::Trace < LogLevel::Debug);
        assert!(LogLevel::Debug < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Error);
    }

    #[test]
    fn levels_parse_from_their_serialized_names() {
        for level in LogLevel::ALL {
            assert_eq!(LogLevel::parse(level.as_str()), Some(level));
        }
        assert_eq!(LogLevel::parse("WARNING"), Some(LogLevel::Warn));
        assert_eq!(LogLevel::parse("chatty"), None);
    }

    #[test]
    fn rank_round_trips() {
        for level in LogLevel::ALL {
            assert_eq!(LogLevel::from_rank(level.rank()), level);
        }
    }

    #[test]
    fn a_record_serializes_to_one_json_line_with_every_documented_field() {
        let record = LogRecord::at("2026-08-07T10:30:00.123Z", LogLevel::Info, "lsp_host", "up")
            .with_correlation_id("cmd-abc123")
            .with_field("language", to_field("typescript"))
            .with_field("startup_ms", to_field(1200));

        let line = record.to_json_line();
        assert!(
            !line.contains('\n'),
            "a record must occupy exactly one line"
        );

        let parsed: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed["ts"], "2026-08-07T10:30:00.123Z");
        assert_eq!(parsed["level"], "info");
        assert_eq!(parsed["source"], "lsp_host");
        assert_eq!(parsed["correlation_id"], "cmd-abc123");
        assert_eq!(parsed["message"], "up");
        assert_eq!(parsed["fields"]["language"], "typescript");
        assert_eq!(parsed["fields"]["startup_ms"], 1200);
    }

    #[test]
    fn a_record_round_trips_through_json() {
        let record = LogRecord::new(LogLevel::Warn, "fs", "slow disk")
            .with_field("path", to_field("/tmp/x"));
        let back: LogRecord = serde_json::from_str(&record.to_json_line()).unwrap();
        assert_eq!(back, record);
    }

    #[test]
    fn a_field_that_cannot_serialize_becomes_null_rather_than_panicking() {
        // f64::NAN has no JSON representation.
        assert_eq!(to_field(f64::NAN), serde_json::Value::Null);
    }
}
