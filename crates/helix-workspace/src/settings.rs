//! Per-folder settings resolution layered onto workspace settings
//! (REQ-FS-001.3, REQ-CONFIG-001.1).
//!
//! ```text
//!  defaults + user          ── from the global configuration service
//!  workspace.json "settings"
//!  <primary>/.helix/settings.json
//!  <folder>/.helix/settings.json   ── highest, and only for files in that root
//! ```
//!
//! ## Why this is not four more layers inside `ConfigService`
//!
//! `ConfigService` has one workspace slot and one folder slot, because a
//! settings *layer* is a single file. A multi-root workspace has one folder
//! layer per root, all live at once, and which one applies depends on the file
//! being asked about. Pushing that into the configuration service would mean
//! either reloading it on every question or giving it a notion of roots, which
//! is the workspace manager's job by definition.
//!
//! So the workspace layers sit on top of the resolved global tree, and the
//! merge and dotted-key primitives are the configuration service's own
//! ([`helix_config::merge`], [`helix_config::LayerDocument`]). Same precedence
//! rules, same array-replaces-rather-than-concatenates semantics, same
//! treatment of object-typed settings such as `files.exclude` — reused rather
//! than reimplemented, so the two cannot drift.
//!
//! ## Which root owns a file
//!
//! Longest matching root wins. Nested roots are legal (a monorepo root plus one
//! of its packages, opened deliberately), and in that case the more specific
//! root is the one whose settings a file inside it should follow.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use helix_config::ConfigParseError;
use helix_config::layer::{ConfigScope, LayerDocument, LeafDecision};
use helix_config::merge::{deep_merge, get_path};
use helix_config::schema::{IssueKind, SchemaRegistry, SettingIssue};
use serde_json::{Map, Value};

use crate::identity::{canonical_path, comparison_key};

/// The workspace and folder settings layers, resolved and ready to answer for
/// any path.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorkspaceSettings {
    /// Resolved defaults + user tree, as the global configuration service sees
    /// it.
    base: Value,
    base_languages: BTreeMap<String, Value>,
    /// `workspace.json`'s `settings` merged under the primary root's
    /// `.helix/settings.json`.
    workspace: LayerDocument,
    /// One tree per root that has a `.helix/settings.json`, keyed by the
    /// root's comparison key.
    folders: BTreeMap<String, LayerDocument>,
    /// Roots, longest comparison key first, so lookup finds the most specific
    /// owner without sorting per question.
    roots: Vec<(String, PathBuf)>,
}

pub(crate) struct WorkspaceSettingsResolution {
    pub settings: WorkspaceSettings,
    pub parse_errors: Vec<ConfigParseError>,
    pub issues: Vec<SettingIssue>,
}

impl WorkspaceSettings {
    /// Resolve the workspace and folder layers.
    ///
    /// `read` returns a settings file's body, or `None` when it is absent or
    /// unreadable. Passing it in keeps this module free of any opinion about
    /// *how* files are read: the service hands it the file system service, and
    /// tests hand it a map.
    pub fn resolve(
        schema: &SchemaRegistry,
        base: Value,
        base_languages: BTreeMap<String, Value>,
        workspace_settings: &Map<String, Value>,
        primary: &Path,
        roots: &[PathBuf],
        read: impl FnMut(&Path) -> Option<String>,
    ) -> Self {
        Self::resolve_with_previous(
            schema,
            (base, base_languages),
            workspace_settings,
            primary,
            roots,
            None,
            read,
        )
        .settings
    }

    pub(crate) fn resolve_with_previous(
        schema: &SchemaRegistry,
        base: (Value, BTreeMap<String, Value>),
        workspace_settings: &Map<String, Value>,
        primary: &Path,
        roots: &[PathBuf],
        previous: Option<&Self>,
        mut read: impl FnMut(&Path) -> Option<String>,
    ) -> WorkspaceSettingsResolution {
        let (base, base_languages) = base;
        let (mut workspace, mut issues) =
            normalize(schema, ConfigScope::Workspace, workspace_settings.clone());
        let mut parse_errors = Vec::new();
        let primary_settings = helix_config::settings_path_in(primary);
        if let Some(body) = read(&primary_settings) {
            match parse(schema, ConfigScope::Workspace, &primary_settings, &body) {
                Ok((tree, layer_issues)) => {
                    merge_document(&mut workspace, &tree);
                    issues.extend(layer_issues);
                }
                Err(error) => {
                    parse_errors.push(error);
                    if let Some(previous) = previous {
                        workspace = previous.workspace.clone();
                    }
                }
            }
        }

        let mut folders = BTreeMap::new();
        let mut ordered = Vec::new();
        for root in roots {
            let key = comparison_key(root);
            ordered.push((key.clone(), root.clone()));
            // The primary root's own file is the workspace layer, not a folder
            // layer. Counting it twice would be harmless today and confusing
            // the first time someone debugged a precedence question.
            if comparison_key(primary) == key {
                continue;
            }
            let settings_path = helix_config::settings_path_in(root);
            if let Some(body) = read(&settings_path) {
                match parse(schema, ConfigScope::Folder, &settings_path, &body) {
                    Ok((tree, layer_issues)) => {
                        folders.insert(key.clone(), tree);
                        issues.extend(layer_issues);
                    }
                    Err(error) => {
                        parse_errors.push(error);
                        if let Some(tree) = previous.and_then(|previous| previous.folders.get(&key))
                        {
                            folders.insert(key.clone(), tree.clone());
                        }
                    }
                }
            }
        }
        // Longest first: `/work/api/packages/core` has to be tested before
        // `/work/api`.
        ordered.sort_by_key(|(key, _)| std::cmp::Reverse(key.len()));

        WorkspaceSettingsResolution {
            settings: Self {
                base,
                base_languages,
                workspace,
                folders,
                roots: ordered,
            },
            parse_errors,
            issues,
        }
    }

    /// The root that owns `path`, by longest match.
    pub fn owning_root(&self, path: &Path) -> Option<&Path> {
        let target = comparison_key(&canonical_path(path));
        self.roots
            .iter()
            .find(|(key, _)| {
                target == *key
                    || target
                        .strip_prefix(key.as_str())
                        .is_some_and(|rest| rest.starts_with('/'))
            })
            .map(|(_, root)| root.as_path())
    }

    /// The effective settings tree for a path inside the workspace.
    ///
    /// A path outside every root still resolves, to the workspace tree: an
    /// untitled buffer and a file opened from outside the workspace both have
    /// to answer questions about tab size.
    pub fn effective(&self, path: Option<&Path>) -> Value {
        self.effective_for_language(path, None)
    }

    /// Effective settings for a path and optional language override.
    pub fn effective_for_language(&self, path: Option<&Path>, language: Option<&str>) -> Value {
        let mut tree = language
            .and_then(|language| self.base_languages.get(language))
            .cloned()
            .unwrap_or_else(|| self.base.clone());
        merge_language(&mut tree, &self.workspace, language);
        if let Some(folder) = path
            .and_then(|path| self.owning_root(path))
            .map(comparison_key)
            .and_then(|key| self.folders.get(&key))
        {
            merge_language(&mut tree, folder, language);
        }
        tree
    }

    /// One setting's effective value for a path, or `None` when no layer sets
    /// it.
    pub fn value(&self, key: &str, path: Option<&Path>) -> Option<Value> {
        get_path(&self.effective(path), key).cloned()
    }

    /// One setting's language-aware effective value.
    pub fn value_for_language(
        &self,
        key: &str,
        path: Option<&Path>,
        language: Option<&str>,
    ) -> Option<Value> {
        get_path(&self.effective_for_language(path, language), key).cloned()
    }

    /// The workspace tree, with no folder layer applied.
    pub fn workspace_tree(&self) -> Value {
        let mut tree = self.base.clone();
        merge_language(&mut tree, &self.workspace, None);
        tree
    }

    /// Roots that contribute a folder layer, for reporting.
    pub fn folder_layers(&self) -> Vec<PathBuf> {
        self.roots
            .iter()
            .filter(|(key, _)| self.folders.contains_key(key))
            .map(|(_, root)| root.clone())
            .collect()
    }
}

/// Parse a settings file body into a normalized tree, or `None` when it will
/// not parse.
///
/// A folder whose settings file is mid-edit keeps the layers above it rather
/// than reverting the whole workspace, the same call the configuration service
/// makes for the same reason.
fn parse(
    schema: &SchemaRegistry,
    scope: ConfigScope,
    path: &Path,
    body: &str,
) -> Result<(LayerDocument, Vec<SettingIssue>), ConfigParseError> {
    let raw = helix_config::jsonc::parse_object(&path.to_string_lossy(), body)?;
    Ok(normalize(schema, scope, raw))
}

/// Expand dotted keys and drop language sections, using the schema to decide
/// where a value starts.
///
/// Language-specific overrides are not resolved here. They are a property of
/// the file being edited, which the editor asks the configuration service
/// about; a workspace-level answer has no language.
pub(crate) fn normalize(
    schema: &SchemaRegistry,
    scope: ConfigScope,
    raw: Map<String, Value>,
) -> (LayerDocument, Vec<SettingIssue>) {
    let mut issues = Vec::new();
    let document = LayerDocument::from_raw(raw, |key, language, value| match schema.get(key) {
        // A declared key *is* a value, whatever its JSON shape. That is what
        // keeps `files.exclude` — keyed by glob, and globs contain dots — from
        // being expanded into nested objects that exclude nothing.
        Some(setting) if !setting.writable_in(scope) => {
            issues.push(
                SettingIssue::new(
                    key,
                    scope,
                    IssueKind::WrongScope,
                    format!("'{key}' cannot be set from the {scope} layer and is ignored here"),
                )
                .with_language(language.map(str::to_string)),
            );
            LeafDecision::Reject
        }
        Some(setting) if language.is_some() && !setting.language_overridable => {
            issues.push(
                SettingIssue::new(
                    key,
                    scope,
                    IssueKind::WrongScope,
                    format!("'{key}' has no per-language meaning; the override is ignored"),
                )
                .with_language(language.map(str::to_string)),
            );
            LeafDecision::Reject
        }
        Some(_) => match schema.validate(key, value) {
            Ok(()) => LeafDecision::Accept,
            Err((kind, message)) => {
                issues.push(
                    SettingIssue::new(key, scope, kind, message)
                        .with_language(language.map(str::to_string)),
                );
                LeafDecision::Reject
            }
        },
        None => match value {
            Value::Object(map) if !map.is_empty() => LeafDecision::Descend,
            _ => LeafDecision::Accept,
        },
    });
    (document, issues)
}

fn merge_document(target: &mut LayerDocument, incoming: &LayerDocument) {
    deep_merge(&mut target.global, &incoming.global);
    for (language, section) in &incoming.languages {
        deep_merge(
            target
                .languages
                .entry(language.clone())
                .or_insert_with(|| Value::Object(Map::new())),
            section,
        );
    }
}

fn merge_language(tree: &mut Value, document: &LayerDocument, language: Option<&str>) {
    deep_merge(tree, &document.global);
    if let Some(section) = language.and_then(|language| document.languages.get(language)) {
        deep_merge(tree, section);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> SchemaRegistry {
        SchemaRegistry::builtin()
    }

    fn base() -> Value {
        json!({ "editor": { "tabSize": 4, "fontSize": 14 } })
    }

    fn settings(
        primary: &Path,
        roots: &[PathBuf],
        workspace: Value,
        files: Vec<(PathBuf, &str)>,
    ) -> WorkspaceSettings {
        let map: BTreeMap<String, String> = files
            .into_iter()
            .map(|(path, body)| (comparison_key(&path), body.to_string()))
            .collect();
        WorkspaceSettings::resolve(
            &schema(),
            base(),
            BTreeMap::new(),
            workspace.as_object().unwrap(),
            primary,
            roots,
            move |path| map.get(&comparison_key(path)).cloned(),
        )
    }

    #[test]
    fn workspace_settings_override_the_user_layer() {
        let primary = PathBuf::from("/work/api");
        let resolved = settings(
            &primary,
            std::slice::from_ref(&primary),
            json!({ "editor.tabSize": 2 }),
            vec![],
        );

        assert_eq!(resolved.value("editor.tabSize", None), Some(json!(2)));
        assert_eq!(
            resolved.value("editor.fontSize", None),
            Some(json!(14)),
            "a key the workspace does not mention keeps the user value"
        );
    }

    #[test]
    fn language_overrides_apply_with_layer_precedence() {
        let primary = PathBuf::from("/work/api");
        let mut base_languages = BTreeMap::new();
        base_languages.insert(
            "typescript".to_string(),
            json!({ "editor": { "tabSize": 6, "fontSize": 14 } }),
        );
        let resolved = WorkspaceSettings::resolve(
            &schema(),
            base(),
            base_languages,
            json!({
                "editor.tabSize": 4,
                "[typescript].editor.fontSize": 18
            })
            .as_object()
            .unwrap(),
            &primary,
            std::slice::from_ref(&primary),
            |_| None,
        );

        assert_eq!(
            resolved.value_for_language("editor.tabSize", None, Some("typescript")),
            Some(json!(4)),
            "a higher workspace global must beat a lower user language override"
        );
        assert_eq!(
            resolved.value_for_language("editor.fontSize", None, Some("typescript")),
            Some(json!(18))
        );
        assert_eq!(
            resolved.value_for_language("editor.fontSize", None, Some("rust")),
            Some(json!(14))
        );
    }

    #[test]
    fn invalid_known_workspace_values_fall_back_to_the_lower_layer() {
        let primary = PathBuf::from("/work/api");
        let resolved = settings(
            &primary,
            std::slice::from_ref(&primary),
            json!({ "editor.tabSize": "wide" }),
            vec![],
        );

        assert_eq!(resolved.value("editor.tabSize", None), Some(json!(4)));
    }

    #[test]
    fn a_folder_layer_overrides_the_workspace_layer_for_its_own_files_only() {
        let api = PathBuf::from("/work/api");
        let web = PathBuf::from("/work/web");
        let resolved = settings(
            &api,
            &[api.clone(), web.clone()],
            json!({ "editor.tabSize": 2 }),
            vec![(
                helix_config::settings_path_in(&web),
                r#"{ "editor.tabSize": 8 }"#,
            )],
        );

        assert_eq!(
            resolved.value("editor.tabSize", Some(&web.join("src/app.ts"))),
            Some(json!(8))
        );
        assert_eq!(
            resolved.value("editor.tabSize", Some(&api.join("src/main.rs"))),
            Some(json!(2)),
            "the other root is untouched by its neighbour's folder settings"
        );
        assert_eq!(resolved.folder_layers(), vec![web]);
    }

    #[test]
    fn the_primary_roots_settings_file_layers_over_the_workspace_document() {
        let api = PathBuf::from("/work/api");
        let resolved = settings(
            &api,
            std::slice::from_ref(&api),
            json!({ "editor.tabSize": 2, "editor.fontSize": 20 }),
            vec![(
                helix_config::settings_path_in(&api),
                r#"{ "editor.tabSize": 3 }"#,
            )],
        );

        assert_eq!(resolved.value("editor.tabSize", None), Some(json!(3)));
        assert_eq!(
            resolved.value("editor.fontSize", None),
            Some(json!(20)),
            "the document still supplies what the file does not"
        );
    }

    #[test]
    fn the_most_specific_root_owns_a_file_when_roots_nest() {
        let outer = PathBuf::from("/work/monorepo");
        let inner = PathBuf::from("/work/monorepo/packages/core");
        let resolved = settings(
            &outer,
            &[outer.clone(), inner.clone()],
            json!({ "editor.tabSize": 2 }),
            vec![(
                helix_config::settings_path_in(&inner),
                r#"{ "editor.tabSize": 6 }"#,
            )],
        );

        assert_eq!(
            resolved.owning_root(&inner.join("src/lib.rs")),
            Some(inner.as_path())
        );
        assert_eq!(
            resolved.value("editor.tabSize", Some(&inner.join("src/lib.rs"))),
            Some(json!(6))
        );
        assert_eq!(
            resolved.value("editor.tabSize", Some(&outer.join("README.md"))),
            Some(json!(2))
        );
    }

    #[test]
    fn a_path_outside_every_root_resolves_to_the_workspace_tree() {
        let api = PathBuf::from("/work/api");
        let resolved = settings(
            &api,
            std::slice::from_ref(&api),
            json!({ "editor.tabSize": 2 }),
            vec![],
        );
        assert!(
            resolved
                .owning_root(Path::new("/elsewhere/file.txt"))
                .is_none()
        );
        assert_eq!(
            resolved.value("editor.tabSize", Some(Path::new("/elsewhere/file.txt"))),
            Some(json!(2))
        );
    }

    #[test]
    fn a_sibling_root_with_a_matching_prefix_is_not_treated_as_inside() {
        let api = PathBuf::from("/work/api");
        let api2 = PathBuf::from("/work/api-tools");
        let resolved = settings(
            &api,
            &[api.clone(), api2.clone()],
            json!({}),
            vec![(
                helix_config::settings_path_in(&api2),
                r#"{ "editor.tabSize": 9 }"#,
            )],
        );

        assert_eq!(
            resolved.owning_root(&api.join("main.rs")),
            Some(api.as_path()),
            "`/work/api-tools` must not claim a file in `/work/api`"
        );
        assert_eq!(
            resolved.value("editor.tabSize", Some(&api.join("main.rs"))),
            Some(json!(4))
        );
    }

    #[test]
    fn an_object_typed_setting_keeps_its_dotted_keys() {
        let api = PathBuf::from("/work/api");
        let resolved = settings(
            &api,
            std::slice::from_ref(&api),
            json!({ "files.exclude": { "**/.cache": true } }),
            vec![],
        );
        assert_eq!(
            resolved.value("files.exclude", None),
            Some(json!({ "**/.cache": true }))
        );
    }

    #[test]
    fn an_unparseable_folder_file_leaves_the_layers_above_it_standing() {
        let api = PathBuf::from("/work/api");
        let web = PathBuf::from("/work/web");
        let resolved = settings(
            &api,
            &[api.clone(), web.clone()],
            json!({ "editor.tabSize": 2 }),
            vec![(helix_config::settings_path_in(&web), "{ not json")],
        );

        assert_eq!(
            resolved.value("editor.tabSize", Some(&web.join("app.ts"))),
            Some(json!(2))
        );
        assert!(resolved.folder_layers().is_empty());
    }

    #[test]
    fn comments_in_a_settings_file_are_tolerated() {
        let api = PathBuf::from("/work/api");
        let web = PathBuf::from("/work/web");
        let resolved = settings(
            &api,
            &[api.clone(), web.clone()],
            json!({}),
            vec![(
                helix_config::settings_path_in(&web),
                "{\n  // two, for this repo\n  \"editor.tabSize\": 2\n}",
            )],
        );
        assert_eq!(
            resolved.value("editor.tabSize", Some(&web.join("app.ts"))),
            Some(json!(2))
        );
    }
}
