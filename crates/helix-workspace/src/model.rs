//! The multi-root model and the `.helix/workspace.json` document
//! (REQ-FS-001.1, .2, .5).
//!
//! ## Why validation never fails the open
//!
//! REQ-FS-001's failure modes are explicit: a workspace config that will not
//! parse opens with the roots that *are* usable and reports the problem. So
//! nothing here returns an error. [`WorkspaceFile::from_raw`] takes whatever
//! the file contained and returns the document it could make of it plus a list
//! of [`WorkspaceIssue`]s describing everything it had to drop. A missing `id`,
//! a folder entry that is a number, a `settings` value that is an array — each
//! costs that one field, not the workspace.
//!
//! ## Why unknown keys survive
//!
//! `.helix/workspace.json` is committed and shared, so a file written by a
//! newer build, or carrying a key a plugin owns, is routine. Unrecognized
//! top-level keys are preserved verbatim in [`WorkspaceFile::extra`] and
//! written back untouched, because silently deleting a colleague's
//! configuration on the first `addRoot` would be indefensible.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use ts_rs::TS;

/// Maximum roots in one workspace (REQ-FS-001.5). Configurable through
/// `workspace.maxRoots`; this is the default and the ceiling the schema
/// allows.
pub const MAX_ROOTS: usize = 20;

/// File name of the workspace document, inside the `.helix` directory the
/// design document's Storage Locations table assigns to committed workspace
/// configuration.
pub const WORKSPACE_FILE_NAME: &str = "workspace.json";

/// `<root>/.helix/workspace.json`.
pub fn workspace_file_path_in(root: impl AsRef<Path>) -> PathBuf {
    root.as_ref()
        .join(helix_config::WORKSPACE_CONFIG_DIR)
        .join(WORKSPACE_FILE_NAME)
}

/// Whether a root is usable right now (REQ-FS-001 failure modes).
///
/// The distinction between `Missing` and `Unavailable` is not cosmetic: a
/// deleted folder is worth offering to remove from the workspace, while an
/// unmounted drive is worth retrying, and offering to remove a root because a
/// VPN dropped would be actively harmful.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum RootAvailability {
    /// The directory exists and can be read.
    Available,
    /// The path does not exist but its parent does, so it was deleted or
    /// renamed. Offer to remove it from the workspace.
    Missing,
    /// Neither the path nor its ancestors can be reached: an unmounted drive,
    /// a disconnected share. Retry periodically, do not offer removal.
    Unavailable,
}

impl RootAvailability {
    pub fn is_available(self) -> bool {
        matches!(self, RootAvailability::Available)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            RootAvailability::Available => "available",
            RootAvailability::Missing => "missing",
            RootAvailability::Unavailable => "unavailable",
        }
    }

    /// Classify a path on disk.
    ///
    /// `is_dir` being false covers both "deleted" and "cannot be reached", and
    /// the parent directory is what separates them: if the parent is readable,
    /// something removed this folder specifically; if it is not, the volume or
    /// share it lived on is gone. That holds for a Windows drive letter
    /// (`Q:\share\project`), a macOS mount (`/Volumes/share/project`), and a
    /// Linux mount point alike, without any platform-specific probing.
    ///
    /// A path that exists but is a file, not a directory, counts as `Missing`:
    /// it is not a usable root, and its parent is there to say so.
    ///
    /// A folder deleted together with its parent therefore reads as
    /// `Unavailable` and is retried rather than offered for removal. The retry
    /// reclassifies it as soon as the parent reappears, which costs one
    /// interval and never loses a root.
    pub fn probe(path: &Path) -> Self {
        if path.is_dir() {
            return RootAvailability::Available;
        }
        match path.parent() {
            Some(parent) if parent.as_os_str().is_empty() => RootAvailability::Missing,
            Some(parent) if parent.is_dir() => RootAvailability::Missing,
            Some(_) => RootAvailability::Unavailable,
            // No parent at all means this *is* a filesystem root, and it does
            // not answer.
            None => RootAvailability::Unavailable,
        }
    }
}

/// One root of an open workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct WorkspaceRoot {
    /// Absolute path, as resolved when the workspace opened.
    pub path: String,
    /// Display name: the authored `name`, or the final path segment.
    pub name: String,
    pub availability: RootAvailability,
    /// True for the root the workspace document and workspace-level settings
    /// live under.
    pub primary: bool,
}

impl WorkspaceRoot {
    pub fn as_path(&self) -> &Path {
        Path::new(&self.path)
    }
}

/// A folder entry as authored in `.helix/workspace.json`.
///
/// `path` is kept in the form it was written. Relative entries resolve against
/// the directory holding the workspace file's `.helix` directory, which is what
/// makes a workspace file committable: two developers with different checkout
/// locations share one document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderEntry {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl FolderEntry {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            name: None,
        }
    }
}

/// What a workspace document could not be taken at its word about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceIssueKind {
    /// No `id`, or an unusable one. A fresh id is generated on the next write.
    MissingId,
    /// A field had the wrong JSON type and was ignored.
    TypeMismatch,
    /// The same folder appeared twice; the duplicate was dropped.
    DuplicateFolder,
    /// More folders than the configured maximum; the excess was dropped.
    TooManyFolders,
    /// A top-level key this build does not know. Preserved, not dropped.
    UnknownKey,
}

/// One problem with the workspace document, with enough detail to point at it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct WorkspaceIssue {
    pub kind: WorkspaceIssueKind,
    /// The offending field, e.g. `folders[2].path`.
    pub field: String,
    pub message: String,
}

impl WorkspaceIssue {
    pub fn new(
        kind: WorkspaceIssueKind,
        field: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            field: field.into(),
            message: message.into(),
        }
    }
}

/// The parsed `.helix/workspace.json`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorkspaceFile {
    /// Stable identifier, and the state and cache key Task 1.10 uses
    /// (REQ-FS-001.2, REQ-NFR-002.11). Empty until first written.
    pub id: String,
    pub name: Option<String>,
    pub folders: Vec<FolderEntry>,
    /// Workspace-level settings, authored in the same syntax as
    /// `.helix/settings.json`.
    pub settings: Map<String, Value>,
    /// Top-level keys this build does not recognize, preserved for the next
    /// write.
    pub extra: Map<String, Value>,
}

impl WorkspaceFile {
    /// Validate an authored document, keeping what is usable.
    ///
    /// `max_roots` caps `folders`; entries past the cap are dropped with a
    /// [`WorkspaceIssueKind::TooManyFolders`] issue rather than silently
    /// truncating a file someone will later wonder about.
    pub fn from_raw(raw: Map<String, Value>, max_roots: usize) -> (Self, Vec<WorkspaceIssue>) {
        let mut issues = Vec::new();
        let mut file = WorkspaceFile::default();

        for (key, value) in raw {
            match key.as_str() {
                "id" => match value.as_str() {
                    Some(id) if !id.trim().is_empty() => file.id = id.trim().to_string(),
                    _ => issues.push(WorkspaceIssue::new(
                        WorkspaceIssueKind::MissingId,
                        "id",
                        "`id` must be a non-empty string; a new one will be generated on the \
                         next write, and state stored under the old one will not be found",
                    )),
                },
                "name" => match value.as_str() {
                    Some(name) => file.name = Some(name.to_string()),
                    None => issues.push(WorkspaceIssue::new(
                        WorkspaceIssueKind::TypeMismatch,
                        "name",
                        "`name` must be a string; the folder name is used instead",
                    )),
                },
                "folders" => match value {
                    Value::Array(entries) => {
                        for (index, entry) in entries.into_iter().enumerate() {
                            let field = format!("folders[{index}]");
                            let Some(folder) = folder_entry(&entry, &field, &mut issues) else {
                                continue;
                            };
                            if file
                                .folders
                                .iter()
                                .any(|existing| existing.path == folder.path)
                            {
                                issues.push(WorkspaceIssue::new(
                                    WorkspaceIssueKind::DuplicateFolder,
                                    field,
                                    format!("`{}` is listed more than once", folder.path),
                                ));
                                continue;
                            }
                            if file.folders.len() >= max_roots {
                                issues.push(WorkspaceIssue::new(
                                    WorkspaceIssueKind::TooManyFolders,
                                    field,
                                    format!(
                                        "a workspace holds at most {max_roots} folders; \
                                         `{}` and any after it were not opened",
                                        folder.path
                                    ),
                                ));
                                break;
                            }
                            file.folders.push(folder);
                        }
                    }
                    _ => issues.push(WorkspaceIssue::new(
                        WorkspaceIssueKind::TypeMismatch,
                        "folders",
                        "`folders` must be an array; no folders were read from the document",
                    )),
                },
                "settings" => match value {
                    Value::Object(map) => file.settings = map,
                    _ => issues.push(WorkspaceIssue::new(
                        WorkspaceIssueKind::TypeMismatch,
                        "settings",
                        "`settings` must be an object; workspace settings were ignored",
                    )),
                },
                _ => {
                    issues.push(WorkspaceIssue::new(
                        WorkspaceIssueKind::UnknownKey,
                        key.clone(),
                        format!("`{key}` is not a known workspace field; it is preserved as-is"),
                    ));
                    file.extra.insert(key, value);
                }
            }
        }

        if file.id.is_empty()
            && !issues
                .iter()
                .any(|i| i.kind == WorkspaceIssueKind::MissingId)
        {
            issues.push(WorkspaceIssue::new(
                WorkspaceIssueKind::MissingId,
                "id",
                "the document has no `id`; one is generated on the next write",
            ));
        }

        (file, issues)
    }

    /// Serialize for writing back to disk.
    ///
    /// Field order is fixed and `id` comes first, so a committed file produces
    /// a readable diff when a root is added rather than a whole-file rewrite.
    /// Comments do not survive, which is why the service only writes this
    /// document when the user asked for a change to it.
    pub fn to_pretty_json(&self) -> String {
        let mut map = Map::new();
        map.insert("id".to_string(), json!(self.id));
        if let Some(name) = &self.name {
            map.insert("name".to_string(), json!(name));
        }
        map.insert(
            "folders".to_string(),
            Value::Array(
                self.folders
                    .iter()
                    .map(|folder| match &folder.name {
                        Some(name) => json!({ "path": folder.path, "name": name }),
                        None => json!({ "path": folder.path }),
                    })
                    .collect(),
            ),
        );
        if !self.settings.is_empty() {
            map.insert("settings".to_string(), Value::Object(self.settings.clone()));
        }
        for (key, value) in &self.extra {
            map.insert(key.clone(), value.clone());
        }

        let mut text =
            serde_json::to_string_pretty(&Value::Object(map)).unwrap_or_else(|_| "{}".to_string());
        text.push('\n');
        text
    }

    /// Absolute paths of every authored folder, resolved against `base` (the
    /// directory holding the `.helix` directory).
    pub fn resolved_folders(&self, base: &Path) -> Vec<PathBuf> {
        self.folders
            .iter()
            .map(|folder| resolve_relative(base, &folder.path))
            .collect()
    }

    /// The authored name for a resolved path, if the document gave one.
    pub fn name_for(&self, base: &Path, path: &Path) -> Option<String> {
        self.folders
            .iter()
            .find(|folder| resolve_relative(base, &folder.path) == path)
            .and_then(|folder| folder.name.clone())
    }
}

/// Read one `folders` element, in either authoring form.
///
/// The string shorthand (`"folders": ["../api"]`) is accepted because it is
/// what a person writes by hand, and rejecting it would make the object form
/// feel like ceremony.
fn folder_entry(
    entry: &Value,
    field: &str,
    issues: &mut Vec<WorkspaceIssue>,
) -> Option<FolderEntry> {
    match entry {
        Value::String(path) if !path.trim().is_empty() => Some(FolderEntry::new(path.trim())),
        Value::Object(map) => {
            let path = map.get("path").and_then(Value::as_str).unwrap_or("").trim();
            if path.is_empty() {
                issues.push(WorkspaceIssue::new(
                    WorkspaceIssueKind::TypeMismatch,
                    format!("{field}.path"),
                    "a folder entry needs a non-empty `path` string; the entry was skipped",
                ));
                return None;
            }
            let name = match map.get("name") {
                None | Some(Value::Null) => None,
                Some(Value::String(name)) => Some(name.clone()),
                Some(_) => {
                    issues.push(WorkspaceIssue::new(
                        WorkspaceIssueKind::TypeMismatch,
                        format!("{field}.name"),
                        "a folder's `name` must be a string; the folder name is used instead",
                    ));
                    None
                }
            };
            Some(FolderEntry {
                path: path.to_string(),
                name,
            })
        }
        _ => {
            issues.push(WorkspaceIssue::new(
                WorkspaceIssueKind::TypeMismatch,
                field.to_string(),
                "a folder entry must be a path string or an object with a `path`; \
                 the entry was skipped",
            ));
            None
        }
    }
}

/// Resolve an authored folder path against the workspace file's base
/// directory, normalizing `.` and `..` textually.
///
/// Textual rather than `canonicalize`: an entry pointing at an unmounted drive
/// still has to resolve to *something*, because a root that cannot be reached
/// is reported as unavailable rather than dropped (REQ-FS-001 failure modes),
/// and `canonicalize` on a missing path fails.
pub fn resolve_relative(base: &Path, authored: &str) -> PathBuf {
    let candidate = Path::new(authored);
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        base.join(candidate)
    };
    normalize(&joined)
}

/// Collapse `.` and `..` without touching the file system.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // A leading `..` has nothing to pop, so it is kept: dropping it
                // would silently retarget the path at the filesystem root.
                if !out.pop() {
                    out.push(Component::ParentDir.as_os_str());
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

/// Express `target` relative to `base` when that is short and obvious, and
/// absolutely otherwise.
///
/// Only two cases produce a relative entry: `target` inside `base`, and
/// `target` beside `base` (a sibling repository, the common multi-root shape).
/// Anything further away is written absolute, because a path with four `..`
/// segments in a committed file is less portable than an absolute one and much
/// harder to read.
pub fn authored_path(base: &Path, target: &Path) -> String {
    let base = normalize(base);
    let target = normalize(target);

    if let Ok(inside) = target.strip_prefix(&base) {
        let text = inside.to_string_lossy().replace('\\', "/");
        return if text.is_empty() {
            ".".to_string()
        } else {
            text
        };
    }
    if let (Some(base_parent), Some(target_parent)) = (base.parent(), target.parent())
        && base_parent == target_parent
        && let Some(name) = target.file_name()
    {
        return format!("../{}", name.to_string_lossy());
    }
    target.to_string_lossy().replace('\\', "/")
}

/// JSON Schema for `.helix/workspace.json`, served to the JSON editor so a
/// hand-edited workspace file validates and completes the same way settings do
/// (REQ-FS-001.2).
pub fn workspace_json_schema(max_roots: usize) -> Value {
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "Helix workspace",
        "type": "object",
        "properties": {
            "id": {
                "type": "string",
                "description": "Stable workspace identifier. Generated on first write and used \
                                as the state and cache key; changing it orphans saved state."
            },
            "name": {
                "type": "string",
                "description": "Display name for the workspace."
            },
            "folders": {
                "type": "array",
                "maxItems": max_roots,
                "description": "Roots of the workspace, resolved relative to this file's parent \
                                directory.",
                "items": {
                    "oneOf": [
                        { "type": "string" },
                        {
                            "type": "object",
                            "required": ["path"],
                            "properties": {
                                "path": { "type": "string" },
                                "name": { "type": "string" }
                            },
                            "additionalProperties": false
                        }
                    ]
                }
            },
            "settings": {
                "type": "object",
                "description": "Workspace-level settings, in the same syntax as settings.json."
            }
        },
        "additionalProperties": true
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn a_valid_document_reads_every_field() {
        let (file, issues) = WorkspaceFile::from_raw(
            raw(json!({
                "id": "abc123",
                "name": "Payments",
                "folders": [{ "path": ".", "name": "api" }, "../web"],
                "settings": { "editor.tabSize": 2 }
            })),
            MAX_ROOTS,
        );

        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(file.id, "abc123");
        assert_eq!(file.name.as_deref(), Some("Payments"));
        assert_eq!(file.folders.len(), 2);
        assert_eq!(file.folders[0].name.as_deref(), Some("api"));
        assert_eq!(file.folders[1].path, "../web");
        assert_eq!(file.settings["editor.tabSize"], 2);
    }

    #[test]
    fn a_missing_id_is_reported_rather_than_fatal() {
        let (file, issues) = WorkspaceFile::from_raw(raw(json!({ "folders": ["."] })), MAX_ROOTS);
        assert!(file.id.is_empty());
        assert_eq!(file.folders.len(), 1);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].kind, WorkspaceIssueKind::MissingId);
    }

    #[test]
    fn a_malformed_field_costs_that_field_and_nothing_else() {
        let (file, issues) = WorkspaceFile::from_raw(
            raw(json!({
                "id": "keep-me",
                "name": 7,
                "folders": [".", 42, { "name": "no path" }],
                "settings": ["not", "an", "object"]
            })),
            MAX_ROOTS,
        );

        assert_eq!(file.id, "keep-me", "a good field survives a bad neighbour");
        assert_eq!(file.folders.len(), 1);
        assert!(file.name.is_none());
        assert!(file.settings.is_empty());
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.kind == WorkspaceIssueKind::TypeMismatch)
                .count(),
            4
        );
    }

    #[test]
    fn folders_past_the_maximum_are_dropped_with_an_issue() {
        let folders: Vec<Value> = (0..25).map(|i| json!(format!("../p{i}"))).collect();
        let (file, issues) =
            WorkspaceFile::from_raw(raw(json!({ "id": "x", "folders": folders })), MAX_ROOTS);

        assert_eq!(file.folders.len(), MAX_ROOTS);
        assert!(
            issues
                .iter()
                .any(|i| i.kind == WorkspaceIssueKind::TooManyFolders)
        );
    }

    #[test]
    fn a_duplicate_folder_is_dropped_once() {
        let (file, issues) = WorkspaceFile::from_raw(
            raw(json!({ "id": "x", "folders": ["../api", "../api"] })),
            MAX_ROOTS,
        );
        assert_eq!(file.folders.len(), 1);
        assert!(
            issues
                .iter()
                .any(|i| i.kind == WorkspaceIssueKind::DuplicateFolder)
        );
    }

    #[test]
    fn an_unknown_key_is_preserved_through_a_write() {
        let (file, issues) = WorkspaceFile::from_raw(
            raw(json!({ "id": "x", "folders": ["."], "futureFeature": { "on": true } })),
            MAX_ROOTS,
        );
        assert!(
            issues
                .iter()
                .any(|i| i.kind == WorkspaceIssueKind::UnknownKey)
        );

        let text = file.to_pretty_json();
        let round_tripped: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(round_tripped["futureFeature"]["on"], true);
        assert_eq!(round_tripped["id"], "x");
    }

    #[test]
    fn a_written_document_reparses_to_the_same_thing() {
        let (file, _) = WorkspaceFile::from_raw(
            raw(json!({
                "id": "round-trip",
                "name": "Two repos",
                "folders": [{ "path": ".", "name": "api" }, { "path": "../web" }],
                "settings": { "editor.tabSize": 2 }
            })),
            MAX_ROOTS,
        );
        let (again, issues) = WorkspaceFile::from_raw(
            raw(serde_json::from_str(&file.to_pretty_json()).unwrap()),
            MAX_ROOTS,
        );
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(file, again);
    }

    #[test]
    fn relative_folders_resolve_against_the_workspace_files_base() {
        let base = Path::new("/work/api");
        assert_eq!(resolve_relative(base, "."), PathBuf::from("/work/api"));
        assert_eq!(resolve_relative(base, "../web"), PathBuf::from("/work/web"));
        assert_eq!(
            resolve_relative(base, "packages/core"),
            PathBuf::from("/work/api/packages/core")
        );
    }

    #[test]
    fn an_authored_path_stays_relative_for_the_shapes_that_read_well() {
        let base = Path::new("/work/api");
        assert_eq!(authored_path(base, Path::new("/work/api")), ".");
        assert_eq!(
            authored_path(base, Path::new("/work/api/packages/core")),
            "packages/core"
        );
        assert_eq!(authored_path(base, Path::new("/work/web")), "../web");
        assert_eq!(
            authored_path(base, Path::new("/elsewhere/tools")),
            "/elsewhere/tools",
            "a distant root is clearer absolute than as a pile of parent segments"
        );
    }

    #[test]
    fn availability_distinguishes_deleted_from_unreachable() {
        let dir = helix_fs::testutil::TempDir::new("workspace-availability");
        assert_eq!(
            RootAvailability::probe(dir.path()),
            RootAvailability::Available
        );
        assert_eq!(
            RootAvailability::probe(&dir.path().join("gone")),
            RootAvailability::Missing
        );

        // A root whose parent is also unreachable is the unmounted case: a
        // drive letter with nothing behind it, or a share that went away.
        let unmounted = if cfg!(windows) {
            PathBuf::from(r"Q:\share\project")
        } else {
            PathBuf::from("/nonexistent-mount-for-tests/share/project")
        };
        assert_eq!(
            RootAvailability::probe(&unmounted),
            RootAvailability::Unavailable
        );
    }

    #[test]
    fn a_file_is_not_a_root() {
        let dir = helix_fs::testutil::TempDir::new("workspace-file-root");
        let file = dir.write("notadir.txt", "x");
        assert_eq!(RootAvailability::probe(&file), RootAvailability::Missing);
    }
}
