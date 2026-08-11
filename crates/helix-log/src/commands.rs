//! Request and response payloads for the `log.*` IPC commands.
//!
//! They live in this crate, next to the model they carry, for the same reason
//! `StreamEndpoint` lives in `helix-stream`: the `ts_rs` export is generated
//! from the type definition, so keeping the definition beside the subsystem
//! keeps `frontend/src/generated/` in step with the subsystem rather than
//! with the kernel's wiring. `helix-kernel` registers the handlers.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::filter::{LevelConfig, LogQuery};
use crate::record::{Fields, LogLevel, LogRecord};

/// Command names, so the kernel and the generated client agree on the
/// strings.
pub const QUERY: &str = "log.query";
pub const EXPORT: &str = "log.export";
pub const APPEND: &str = "log.append";
pub const LEVELS: &str = "log.levels";
pub const SET_LEVEL: &str = "log.set_level";

/// Streaming channel carrying every record as it is emitted, kernel and
/// frontend alike (REQ-OBS-001.3). The viewer subscribes to it for
/// follow-tail and queries `log.query` for history.
pub const CHANNEL: &str = "log:entries";

/// `log.query` request: the viewer's filter (REQ-OBS-001.4).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct LogQueryRequest {
    pub query: LogQuery,
}

/// `log.query` response.
///
/// `matched` is the count before `limit`, and `evicted` is how many records
/// have fallen out of the ring since launch, so the viewer can distinguish
/// "no more entries" from "no more entries retained".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct LogQueryResponse {
    pub entries: Vec<LogRecord>,
    pub matched: u32,
    pub ring_len: u32,
    pub ring_capacity: u32,
    #[ts(type = "number")]
    pub evicted: u64,
    /// Distinct sources currently in the ring, so the viewer's source filter
    /// lists what actually logged.
    pub sources: Vec<String>,
}

/// `log.export` request: the same filter as a query.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct LogExportRequest {
    pub query: LogQuery,
}

/// `log.export` response: the filtered set as JSON lines
/// (REQ-OBS-001.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct LogExportResponse {
    /// `jsonl`, matching the on-disk log format exactly so an exported set
    /// and a log file can be concatenated.
    pub format: String,
    pub content: String,
    pub entry_count: u32,
    /// Suggested file name for a save dialog.
    pub suggested_file_name: String,
}

/// `log.append` request: a frontend record joining the unified stream
/// (REQ-OBS-001.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct LogAppendRequest {
    pub level: LogLevel,
    /// Namespaced under `frontend.` by the kernel if it is not already, so a
    /// renderer cannot file its records under a kernel service's name.
    pub source: String,
    pub message: String,
    #[ts(type = "Record<string, unknown>")]
    pub fields: Fields,
    /// Set when the frontend record belongs to an in-flight command, which
    /// is what links a UI action to the kernel work it caused
    /// (REQ-OBS-001.9).
    pub correlation_id: Option<String>,
    /// Client-side capture time. Ignored unless it parses as the kernel's
    /// fixed-width RFC 3339 format, so a skewed or hostile clock cannot
    /// reorder the viewer.
    pub ts: Option<String>,
}

impl Default for LogAppendRequest {
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
            source: String::from("frontend"),
            message: String::new(),
            fields: Fields::new(),
            correlation_id: None,
            ts: None,
        }
    }
}

/// `log.append` response. `recorded` is false when the record's level is
/// disabled for its source, which the frontend can use to stop shipping
/// records nobody keeps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct LogAppendResponse {
    pub recorded: bool,
    pub source: String,
}

/// `log.levels` request. Empty: levels are a property of the process.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct LogLevelsRequest {}

/// `log.set_level` request. A `module` of `None` sets the default level;
/// a `level` of `None` clears the module override (REQ-OBS-001.2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct LogSetLevelRequest {
    pub module: Option<String>,
    pub level: Option<LogLevel>,
}

/// The level configuration after a change, returned by both `log.levels`
/// and `log.set_level` so the caller never has to re-read it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct LogLevelsResponse {
    pub levels: LevelConfig,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_names_are_namespaced_under_log() {
        for name in [QUERY, EXPORT, APPEND, LEVELS, SET_LEVEL] {
            assert!(name.starts_with("log."), "{name}");
        }
        assert_eq!(CHANNEL, "log:entries");
    }

    #[test]
    fn an_append_request_defaults_every_optional_field() {
        let request: LogAppendRequest =
            serde_json::from_str(r#"{"message":"clicked save"}"#).unwrap();
        assert_eq!(request.level, LogLevel::Info);
        assert_eq!(request.message, "clicked save");
        assert!(request.fields.is_empty());
        assert_eq!(request.correlation_id, None);
    }

    #[test]
    fn a_query_request_defaults_to_an_unfiltered_query() {
        let request: LogQueryRequest = serde_json::from_str("{}").unwrap();
        assert_eq!(request.query, LogQuery::default());
    }

    #[test]
    fn a_set_level_request_distinguishes_clearing_from_setting() {
        let clear: LogSetLevelRequest = serde_json::from_str(r#"{"module":"kernel.fs"}"#).unwrap();
        assert_eq!(clear.module.as_deref(), Some("kernel.fs"));
        assert_eq!(clear.level, None);

        let set: LogSetLevelRequest =
            serde_json::from_str(r#"{"module":"kernel.fs","level":"trace"}"#).unwrap();
        assert_eq!(set.level, Some(LogLevel::Trace));
    }
}
