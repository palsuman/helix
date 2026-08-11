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

use helix_config::layer::{LayerDocument, LeafDecision};
use helix_config::merge::{deep_merge, get_path};
use helix_config::schema::SchemaRegistry;
use serde_json::{Map, Value};

use crate::identity::comparison_key;

/// The workspace and folder settings layers, resolved and ready to answer for
/// any path.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceSettings {
    /// Resolved defaults + user tree, as the global configuration service sees
    /// it.
    base: Value,
    /// `workspace.json`'s `settings` merged under the primary root's
    /// `.helix/settings.json`.
    workspace: Value,
    /// One tree per root that has a `.helix/settings.json`, keyed by the
    /// root's comparison key.
    folders: BTreeMap<String, Value>,
    /// Roots, longest comparison key first, so lookup finds the most specific
    /// owner without sorting per question.
    roots: Vec<(String, PathBuf)>,
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
        workspace_settings: &Map<String, Value>,
        primary: &Path,
        roots: &[PathBuf],
        mut read: impl FnMut(&Path) -> Option<String>,
    ) -> Self {
        let mut workspace = normalize(schema, workspace_settings.clone());
        if let Some(body) = read(&helix_config::settings_path_in(primary))
            && let Some(tree) = parse(schema, &body)
        {
            deep_merge(&mut workspace, &tree);
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
            if let Some(body) = read(&helix_config::settings_path_in(root))
                && let Some(tree) = parse(schema, &body)
            {
                folders.insert(key, tree);
            }
        }
        // Longest first: `/work/api/packages/core` has to be tested before
        // `/work/api`.
        ordered.sort_by_key(|(key, _)| std::cmp::Reverse(key.len()));

        Self {
            base,
            workspace,
            folders,
            roots: ordered,
        }
    }

    /// The root that owns `path`, by longest match.
    pub fn owning_root(&self, path: &Path) -> Option<&Path> {
        let target = comparison_key(path);
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
        let mut tree = self.base.clone();
        deep_merge(&mut tree, &self.workspace);
        if let Some(folder) = path
            .and_then(|path| self.owning_root(path))
            .map(comparison_key)
            .and_then(|key| self.folders.get(&key))
        {
            deep_merge(&mut tree, folder);
        }
        tree
    }

    /// One setting's effective value for a path, or `None` when no layer sets
    /// it.
    pub fn value(&self, key: &str, path: Option<&Path>) -> Option<Value> {
        get_path(&self.effective(path), key).cloned()
    }

    /// The workspace tree, with no folder layer applied.
    pub fn workspace_tree(&self) -> Value {
        let mut tree = self.base.clone();
        deep_merge(&mut tree, &self.workspace);
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
fn parse(schema: &SchemaRegistry, body: &str) -> Option<Value> {
    let raw = helix_config::jsonc::parse_object("", body).ok()?;
    Some(normalize(schema, raw))
}

/// Expand dotted keys and drop language sections, using the schema to decide
/// where a value starts.
///
/// Language-specific overrides are not resolved here. They are a property of
/// the file being edited, which the editor asks the configuration service
/// about; a workspace-level answer has no language.
fn normalize(schema: &SchemaRegistry, raw: Map<String, Value>) -> Value {
    LayerDocument::from_raw(raw, |key, _language, value| match schema.get(key) {
        // A declared key *is* a value, whatever its JSON shape. That is what
        // keeps `files.exclude` — keyed by glob, and globs contain dots — from
        // being expanded into nested objects that exclude nothing.
        Some(_) => LeafDecision::Accept,
        None => match value {
            Value::Object(map) if !map.is_empty() => LeafDecision::Descend,
            _ => LeafDecision::Accept,
        },
    })
    .global
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
