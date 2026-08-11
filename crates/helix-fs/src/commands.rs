//! Request and response payloads for the `fs.*` IPC commands, and the
//! streaming channel change notifications travel on.
//!
//! They live beside the subsystem, as the `log.*`, `stream.*`, and `config.*`
//! payloads do, so the `ts_rs` export in `frontend/src/generated/` tracks the
//! subsystem rather than the kernel's wiring. `helix-kernel` registers the
//! handlers.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::change::FileChange;
use crate::encoding::Encoding;
use crate::eol::LineEnding;
use crate::listing::{FileEntry, Listing};
use crate::service::{FileContent, WriteOutcome};
use crate::watch::RootReport;

/// Command names, so the kernel and the generated client agree on the strings.
pub const READ: &str = "fs.read";
pub const WRITE: &str = "fs.write";
pub const LIST: &str = "fs.list";
pub const STAT: &str = "fs.stat";
pub const WATCH: &str = "fs.watch";
pub const UNWATCH: &str = "fs.unwatch";

/// Streaming channel carrying debounced change batches (REQ-FS-004.1).
///
/// Batches rather than one frame per change: a `git checkout` produces
/// thousands of changes, and a frame each would flood the channel it is trying
/// to inform.
pub const CHANNEL: &str = "fs:changed";

/// `fs.read` request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct FsReadRequest {
    pub path: String,
}

/// `fs.read` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct FsReadResponse {
    pub content: FileContent,
}

/// `fs.write` request.
///
/// `expected_hash` is how the frontend opts into conflict detection: pass the
/// hash from the `fs.read` that populated the buffer, and a file changed on
/// disk in the meantime fails the write instead of silently losing the external
/// change (REQ-FS-004.3).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct FsWriteRequest {
    pub path: String,
    /// LF-normalised text, as the editor holds it.
    pub text: String,
    /// Absent keeps the file's existing encoding, or `files.encoding` for a new
    /// file.
    pub encoding: Option<Encoding>,
    /// Absent keeps the file's existing line ending style.
    pub eol: Option<LineEnding>,
    pub expected_hash: Option<String>,
}

/// `fs.write` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct FsWriteResponse {
    pub outcome: WriteOutcome,
}

/// `fs.list` request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct FsListRequest {
    pub path: String,
    /// Walk the whole subtree. The explorer expands one level at a time, so
    /// this defaults to false; the index and the watcher budget want it true.
    pub recursive: bool,
}

/// `fs.list` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct FsListResponse {
    pub listing: Listing,
}

/// `fs.stat` request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct FsStatRequest {
    pub path: String,
}

/// `fs.stat` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct FsStatResponse {
    pub entry: FileEntry,
}

/// `fs.watch` request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct FsWatchRequest {
    pub root: String,
}

/// `fs.watch` response.
///
/// Carries the budget verdict and the exclusion suggestions, so the frontend
/// can raise the REQ-FS-004.6 warning from the same round trip that started the
/// watch rather than polling for it afterwards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct FsWatchResponse {
    pub report: RootReport,
}

/// `fs.unwatch` request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct FsUnwatchRequest {
    pub root: String,
}

/// `fs.unwatch` response.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct FsUnwatchResponse {
    pub stopped: bool,
}

/// The payload published on [`CHANNEL`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct FsChangeNotification {
    pub changes: Vec<FileChange>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_names_are_namespaced_under_fs() {
        for name in [READ, WRITE, LIST, STAT, WATCH, UNWATCH] {
            assert!(name.starts_with("fs."), "{name}");
        }
        assert_eq!(CHANNEL, "fs:changed");
    }

    #[test]
    fn a_read_request_needs_only_a_path() {
        let request: FsReadRequest = serde_json::from_str(r#"{"path":"/a/main.rs"}"#).unwrap();
        assert_eq!(request.path, "/a/main.rs");
    }

    #[test]
    fn a_write_request_omitting_encoding_and_eol_means_keep_them() {
        let request: FsWriteRequest =
            serde_json::from_str(r#"{"path":"/a/main.rs","text":"x\n"}"#).unwrap();
        assert_eq!(request.encoding, None);
        assert_eq!(request.eol, None);
        assert_eq!(request.expected_hash, None);
    }

    #[test]
    fn a_write_request_carries_the_encoding_and_eol_names_the_service_uses() {
        let request: FsWriteRequest = serde_json::from_str(
            r#"{"path":"/a/main.rs","text":"x\n","encoding":"utf16_le","eol":"crlf","expected_hash":"abc"}"#,
        )
        .unwrap();
        assert_eq!(request.encoding, Some(Encoding::Utf16Le));
        assert_eq!(request.eol, Some(LineEnding::Crlf));
        assert_eq!(request.expected_hash.as_deref(), Some("abc"));
    }

    #[test]
    fn a_list_request_is_shallow_unless_asked_otherwise() {
        let request: FsListRequest = serde_json::from_str(r#"{"path":"/a"}"#).unwrap();
        assert!(!request.recursive);
    }

    #[test]
    fn a_change_notification_serialises_as_a_batch() {
        let notification = FsChangeNotification {
            changes: vec![FileChange {
                root: "/a".into(),
                path: "/a/main.rs".into(),
                kind: crate::change::ChangeKind::Modified,
                is_dir: false,
                coalesced: 3,
            }],
        };
        let json = serde_json::to_value(&notification).unwrap();
        assert_eq!(json["changes"][0]["kind"], "modified");
        assert_eq!(json["changes"][0]["coalesced"], 3);
    }
}
