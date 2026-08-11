//! Layers, layer documents, and the language-override syntax.
//!
//! A layer is one source of settings. Four exist (REQ-CONFIG-001.1), and their
//! precedence is the whole reason this system exists:
//!
//! ```text
//! folder  .helix/settings.json in a sub-folder   ── highest
//! workspace  .helix/settings.json at the root
//! user    ~/.helix/settings.json
//! default  compiled into the kernel              ── lowest
//! ```
//!
//! Within a layer, a `[language]` section overrides that layer's global
//! values, and only for files of that language (REQ-CONFIG-001.2). Both
//! authoring forms are accepted, because both appear in the wild and a user
//! copying a snippet should not have to know which one this editor prefers:
//!
//! ```jsonc
//! { "[typescript]": { "editor.tabSize": 2 } }   // section form
//! { "[typescript].editor.tabSize": 2 }          // flat form
//! ```
//!
//! Resolution order for a language `L` interleaves the two axes rather than
//! applying one after the other: `default`, `default[L]`, `user`, `user[L]`,
//! `workspace`, `workspace[L]`, `folder`, `folder[L]`. A language override in
//! a *lower* layer therefore loses to a global value in a *higher* one, which
//! is what makes "the workspace decided this" hold even against a
//! language-specific personal preference.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use ts_rs::TS;

use crate::merge::{deep_merge, get_path, remove_path, set_path};

/// One layer of the settings stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum ConfigScope {
    /// Compiled-in defaults. Read-only.
    Default,
    /// `~/.helix/settings.json`.
    User,
    /// `.helix/settings.json` at the workspace root.
    Workspace,
    /// `.helix/settings.json` in a specific folder of a multi-root workspace.
    Folder,
}

impl ConfigScope {
    /// Every layer, lowest precedence first. Merging in this order makes the
    /// last write win, which is exactly the precedence rule.
    pub const ASCENDING: [ConfigScope; 4] = [
        ConfigScope::Default,
        ConfigScope::User,
        ConfigScope::Workspace,
        ConfigScope::Folder,
    ];

    /// Whether a layer can be written to at all.
    pub fn is_writable(self) -> bool {
        !matches!(self, ConfigScope::Default)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ConfigScope::Default => "default",
            ConfigScope::User => "user",
            ConfigScope::Workspace => "workspace",
            ConfigScope::Folder => "folder",
        }
    }
}

impl std::fmt::Display for ConfigScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where each writable layer's file lives.
///
/// Any of them may be absent: a window with no workspace open has only the
/// user layer, and a workspace with no `.helix/` directory yet still resolves
/// (the file is created on first write).
#[derive(Debug, Clone, Default)]
pub struct ConfigPaths {
    pub user: Option<PathBuf>,
    pub workspace: Option<PathBuf>,
    pub folder: Option<PathBuf>,
}

/// File name every settings layer uses.
pub const SETTINGS_FILE_NAME: &str = "settings.json";

/// Directory holding workspace-scoped configuration, per the design
/// document's Storage Locations table.
pub const WORKSPACE_CONFIG_DIR: &str = ".helix";

impl ConfigPaths {
    /// The user layer at `~/.helix/settings.json` (REQ-CONFIG-001.3), and no
    /// workspace or folder layer.
    ///
    /// Resolved from `HOME`, falling back to `USERPROFILE` on Windows. When
    /// neither exists there is no user layer, and the kernel still starts on
    /// defaults rather than refusing to run.
    pub fn for_user() -> Self {
        Self {
            user: user_settings_path(),
            ..Self::default()
        }
    }

    /// Add the workspace layer for a workspace root.
    pub fn with_workspace_root(mut self, root: impl AsRef<Path>) -> Self {
        self.workspace = Some(settings_path_in(root));
        self
    }

    /// Add the folder layer for a folder inside a multi-root workspace.
    pub fn with_folder_root(mut self, folder: impl AsRef<Path>) -> Self {
        self.folder = Some(settings_path_in(folder));
        self
    }

    pub fn path(&self, scope: ConfigScope) -> Option<&Path> {
        match scope {
            ConfigScope::Default => None,
            ConfigScope::User => self.user.as_deref(),
            ConfigScope::Workspace => self.workspace.as_deref(),
            ConfigScope::Folder => self.folder.as_deref(),
        }
    }
}

/// `<root>/.helix/settings.json`.
pub fn settings_path_in(root: impl AsRef<Path>) -> PathBuf {
    root.as_ref()
        .join(WORKSPACE_CONFIG_DIR)
        .join(SETTINGS_FILE_NAME)
}

/// `~/.helix/settings.json`, when a home directory can be determined.
pub fn user_settings_path() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .map(|home| home.join(WORKSPACE_CONFIG_DIR).join(SETTINGS_FILE_NAME))
}

/// One layer's settings, in both the form it was authored in and the form the
/// merge consumes.
///
/// `raw` is retained verbatim so a write preserves the author's key style and
/// any keys this build does not recognize. `global` and `languages` are the
/// normalized trees, with dotted keys expanded and language sections split
/// out.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LayerDocument {
    pub raw: Map<String, Value>,
    pub global: Value,
    pub languages: BTreeMap<String, Value>,
}

/// What a document builder decides about one node of an authored tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeafDecision {
    /// Take the value as it stands, whole.
    Accept,
    /// Drop the value from resolution. The authored file is untouched.
    Reject,
    /// The node is a container, not a value: look at its children instead.
    Descend,
}

impl LayerDocument {
    /// Build the normalized trees from an authored object.
    ///
    /// `decide` inspects each node and says whether it is a value to keep, a
    /// value to drop, or a container to descend into. That third case is what
    /// makes an object-typed setting such as `files.exclude` validate as a
    /// whole while `editor` remains a container whose children are validated
    /// individually — the schema, not the JSON shape, decides where a value
    /// starts.
    pub fn from_raw(
        raw: Map<String, Value>,
        mut decide: impl FnMut(&str, Option<&str>, &Value) -> LeafDecision,
    ) -> Self {
        let mut global = Value::Object(Map::new());
        let mut languages: BTreeMap<String, Value> = BTreeMap::new();

        for (key, value) in &raw {
            match split_language_key(key) {
                Some((language, rest)) => {
                    // "[ts]": { … } is a whole section; "[ts].editor.tabSize"
                    // is one override. Both are normalized the same way, so
                    // the two forms cannot diverge in behaviour.
                    let entries = match rest {
                        None => value.as_object().cloned().unwrap_or_default(),
                        Some(rest) => {
                            let mut one = Map::new();
                            one.insert(rest.to_string(), value.clone());
                            one
                        }
                    };
                    let built = build_tree(&entries, "", Some(language), &mut decide);
                    let section = languages
                        .entry(language.to_string())
                        .or_insert_with(|| Value::Object(Map::new()));
                    deep_merge(section, &built);
                }
                None => {
                    let mut one = Map::new();
                    one.insert(key.clone(), value.clone());
                    let built = build_tree(&one, "", None, &mut decide);
                    deep_merge(&mut global, &built);
                }
            }
        }

        Self {
            raw,
            global,
            languages,
        }
    }

    /// The layer with nothing in it.
    pub fn empty() -> Self {
        Self {
            raw: Map::new(),
            global: Value::Object(Map::new()),
            languages: BTreeMap::new(),
        }
    }

    /// A layer built directly from a tree, used for the defaults layer.
    pub fn from_tree(tree: Value) -> Self {
        Self {
            raw: tree.as_object().cloned().unwrap_or_default(),
            global: tree,
            languages: BTreeMap::new(),
        }
    }

    /// Whether the authored document declares this key, globally or for the
    /// given language. Used to report which layer a value came from.
    pub fn declares(&self, key: &str, language: Option<&str>) -> bool {
        if let Some(language) = language
            && self
                .languages
                .get(language)
                .and_then(|section| get_path(section, key))
                .is_some()
        {
            return true;
        }
        get_path(&self.global, key).is_some()
    }

    /// Set a key in the authored document, matching the style already in use:
    /// an existing flat dotted key is updated in place, an existing nested
    /// path is updated where it sits, and a brand new key is written flat.
    ///
    /// Writing new keys flat keeps a hand-edited file readable — one line per
    /// setting, greppable, with no nesting to unpick.
    pub fn raw_set(&mut self, key: &str, value: Value, language: Option<&str>) {
        match language {
            Some(language) => {
                let section_key = format!("[{language}]");
                // The flat form wins when it is already present, so a file
                // written by hand as "[ts].editor.tabSize" is not silently
                // restructured.
                let flat_key = format!("{section_key}.{key}");
                if self.raw.contains_key(&flat_key) {
                    self.raw.insert(flat_key, value);
                    return;
                }
                let section = self
                    .raw
                    .entry(section_key)
                    .or_insert_with(|| Value::Object(Map::new()));
                if !section.is_object() {
                    *section = Value::Object(Map::new());
                }
                set_in_existing_style(section.as_object_mut().unwrap(), key, value);
            }
            None => set_in_existing_style(&mut self.raw, key, value),
        }
    }

    /// Remove a key from the authored document. Returns whether it was there.
    pub fn raw_remove(&mut self, key: &str, language: Option<&str>) -> bool {
        match language {
            Some(language) => {
                let section_key = format!("[{language}]");
                let flat_key = format!("{section_key}.{key}");
                if self.raw.remove(&flat_key).is_some() {
                    return true;
                }
                let Some(section) = self.raw.get_mut(&section_key) else {
                    return false;
                };
                let Some(map) = section.as_object_mut() else {
                    return false;
                };
                let removed = remove_in_existing_style(map, key);
                if removed && map.is_empty() {
                    self.raw.remove(&section_key);
                }
                removed
            }
            None => remove_in_existing_style(&mut self.raw, key),
        }
    }

    /// Serialize the authored document for writing back to disk.
    ///
    /// Comments do not survive: this reserializes the parsed document rather
    /// than editing the source text. A programmatic write (the settings UI,
    /// a command) therefore costs the file's comments, which is why the
    /// service only ever writes the layer the caller explicitly named.
    pub fn to_pretty_json(&self) -> String {
        let mut text = serde_json::to_string_pretty(&Value::Object(self.raw.clone()))
            .unwrap_or_else(|_| "{}".to_string());
        text.push('\n');
        text
    }
}

/// Split `[typescript]` and `[typescript].editor.tabSize` into their language
/// and remainder. Returns `None` for an ordinary key.
pub fn split_language_key(key: &str) -> Option<(&str, Option<&str>)> {
    let rest = key.strip_prefix('[')?;
    let close = rest.find(']')?;
    let language = &rest[..close];
    if language.is_empty() {
        return None;
    }
    let tail = &rest[close + 1..];
    let tail = tail.strip_prefix('.').unwrap_or(tail);
    if tail.is_empty() {
        Some((language, None))
    } else {
        Some((language, Some(tail)))
    }
}

/// Format a key for display, including its language when it has one, in the
/// syntax the user would type: `[typescript].editor.tabSize`.
pub fn qualified_key(key: &str, language: Option<&str>) -> String {
    match language {
        Some(language) => format!("[{language}].{key}"),
        None => key.to_string(),
    }
}

/// Normalize an authored object into a settings tree, asking `decide` about
/// each node as the walk reaches it.
///
/// Dotted keys are expanded *as keys*, never inside a value. That distinction
/// is not cosmetic: `files.exclude` is keyed by glob, and globs contain dots.
/// Expanding `{"files.exclude": {"**/.cache": true}}` blindly would produce
/// `files.exclude["**/"]["cache"]` and quietly stop excluding anything. So the
/// walk descends only where `decide` says the node is a container, which the
/// schema decides, and copies accepted values verbatim.
fn build_tree(
    entries: &Map<String, Value>,
    prefix: &str,
    language: Option<&str>,
    decide: &mut impl FnMut(&str, Option<&str>, &Value) -> LeafDecision,
) -> Value {
    let mut out = Value::Object(Map::new());
    for (key, value) in entries {
        let full = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        let placed = match decide(&full, language, value) {
            LeafDecision::Accept => Some(value.clone()),
            LeafDecision::Reject => None,
            LeafDecision::Descend => value
                .as_object()
                .map(|children| build_tree(children, &full, language, decide))
                .filter(|built| !built.as_object().map(Map::is_empty).unwrap_or(true)),
        };
        if let Some(placed) = placed {
            let mut fragment = Value::Object(Map::new());
            set_path(&mut fragment, key, placed);
            deep_merge(&mut out, &fragment);
        }
    }
    out
}

/// Update `key` where it already lives, or insert it as a flat dotted key.
fn set_in_existing_style(map: &mut Map<String, Value>, key: &str, value: Value) {
    if map.contains_key(key) {
        map.insert(key.to_string(), value);
        return;
    }
    // Longest existing dotted prefix wins, so `{"editor.minimap": {…}}` plus
    // a write to `editor.minimap.enabled` lands inside that object rather
    // than creating a second, shadowing key.
    let segments: Vec<&str> = key.split('.').collect();
    for boundary in (1..segments.len()).rev() {
        let prefix = segments[..boundary].join(".");
        if let Some(existing) = map.get_mut(&prefix)
            && existing.is_object()
        {
            let remainder = segments[boundary..].join(".");
            set_path(existing, &remainder, value);
            return;
        }
    }
    map.insert(key.to_string(), value);
}

/// Remove `key` wherever it lives, pruning objects the removal empties.
fn remove_in_existing_style(map: &mut Map<String, Value>, key: &str) -> bool {
    if map.remove(key).is_some() {
        return true;
    }
    let segments: Vec<&str> = key.split('.').collect();
    for boundary in (1..segments.len()).rev() {
        let prefix = segments[..boundary].join(".");
        let remainder = segments[boundary..].join(".");
        if let Some(existing) = map.get_mut(&prefix)
            && existing.is_object()
            && remove_path(existing, &remainder)
        {
            if existing.as_object().map(Map::is_empty).unwrap_or(false) {
                map.remove(&prefix);
            }
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Treat every non-empty object as a container and every other node as a
    /// value, which is what a registry with no object-typed settings does.
    fn accept_everything(_: &str, _: Option<&str>, value: &Value) -> LeafDecision {
        match value {
            Value::Object(map) if !map.is_empty() => LeafDecision::Descend,
            _ => LeafDecision::Accept,
        }
    }

    fn document(raw: Value) -> LayerDocument {
        LayerDocument::from_raw(raw.as_object().unwrap().clone(), accept_everything)
    }

    #[test]
    fn precedence_runs_from_default_up_to_folder() {
        assert_eq!(
            ConfigScope::ASCENDING,
            [
                ConfigScope::Default,
                ConfigScope::User,
                ConfigScope::Workspace,
                ConfigScope::Folder
            ]
        );
        assert!(ConfigScope::Folder > ConfigScope::Workspace);
        assert!(ConfigScope::Workspace > ConfigScope::User);
        assert!(ConfigScope::User > ConfigScope::Default);
        assert!(!ConfigScope::Default.is_writable());
    }

    #[test]
    fn a_language_key_is_recognized_in_both_authoring_forms() {
        assert_eq!(
            split_language_key("[typescript]"),
            Some(("typescript", None))
        );
        assert_eq!(
            split_language_key("[typescript].editor.tabSize"),
            Some(("typescript", Some("editor.tabSize")))
        );
        assert_eq!(split_language_key("editor.tabSize"), None);
        assert_eq!(split_language_key("[]"), None);
        assert_eq!(split_language_key("[unterminated"), None);
    }

    #[test]
    fn both_language_forms_produce_the_same_document() {
        let section = document(json!({ "[typescript]": { "editor.tabSize": 2 } }));
        let flat = document(json!({ "[typescript].editor.tabSize": 2 }));
        assert_eq!(section.languages, flat.languages);
        assert_eq!(section.languages["typescript"]["editor"]["tabSize"], 2);
        assert!(section.global.as_object().unwrap().is_empty());
    }

    #[test]
    fn global_and_language_settings_are_kept_apart() {
        let doc = document(json!({
            "editor.tabSize": 4,
            "[python].editor.tabSize": 2
        }));
        assert_eq!(doc.global["editor"]["tabSize"], 4);
        assert_eq!(doc.languages["python"]["editor"]["tabSize"], 2);
        assert!(doc.declares("editor.tabSize", None));
        assert!(doc.declares("editor.tabSize", Some("python")));
        assert!(!doc.declares("editor.fontSize", None));
    }

    #[test]
    fn dotted_and_nested_authoring_forms_produce_the_same_tree() {
        let dotted = document(json!({ "editor.fontSize": 14, "editor.tabSize": 2 }));
        let nested = document(json!({ "editor": { "fontSize": 14, "tabSize": 2 } }));
        let mixed = document(json!({ "editor": { "fontSize": 14 }, "editor.tabSize": 2 }));
        assert_eq!(dotted.global, nested.global);
        assert_eq!(dotted.global, mixed.global);
        assert_eq!(dotted.global["editor"]["tabSize"], 2);
    }

    #[test]
    fn a_dotted_key_inside_a_value_is_left_alone() {
        // `files.exclude` is keyed by glob, and globs contain dots. Expanding
        // inside a value would turn "**/.cache" into "**/" → "cache" and
        // silently stop excluding anything.
        let raw = json!({ "files.exclude": { "**/.cache": true, "**/dist": true } })
            .as_object()
            .unwrap()
            .clone();
        let doc = LayerDocument::from_raw(raw, |key, _language, _value| {
            // Stands in for the schema declaring `files.exclude` object-typed.
            if key == "files.exclude" {
                LeafDecision::Accept
            } else {
                LeafDecision::Descend
            }
        });

        assert_eq!(doc.global["files"]["exclude"]["**/.cache"], true);
        assert_eq!(doc.global["files"]["exclude"]["**/dist"], true);
    }

    #[test]
    fn a_rejected_leaf_is_dropped_from_resolution_but_kept_in_the_authored_document() {
        let raw = json!({ "editor.tabSize": 2, "editor.fontSize": "huge" })
            .as_object()
            .unwrap()
            .clone();
        let doc = LayerDocument::from_raw(raw, |key, language, value| {
            if key == "editor.fontSize" {
                LeafDecision::Reject
            } else {
                accept_everything(key, language, value)
            }
        });

        assert_eq!(doc.global["editor"]["tabSize"], 2);
        assert!(doc.global["editor"].get("fontSize").is_none());
        assert!(
            doc.raw.contains_key("editor.fontSize"),
            "the authored file is never silently rewritten"
        );
    }

    #[test]
    fn writing_a_new_key_uses_the_flat_form() {
        let mut doc = LayerDocument::empty();
        doc.raw_set("editor.fontSize", json!(16), None);
        assert_eq!(doc.raw["editor.fontSize"], 16);
        assert!(doc.to_pretty_json().contains("\"editor.fontSize\": 16"));
    }

    #[test]
    fn writing_an_existing_key_follows_the_style_already_in_the_file() {
        let mut doc = document(json!({ "editor": { "fontSize": 14 } }));
        doc.raw_set("editor.fontSize", json!(18), None);
        assert_eq!(doc.raw["editor"]["fontSize"], 18);
        assert!(
            !doc.raw.contains_key("editor.fontSize"),
            "a nested file must not sprout a shadowing flat key"
        );
    }

    #[test]
    fn writing_a_language_override_creates_the_section() {
        let mut doc = LayerDocument::empty();
        doc.raw_set("editor.tabSize", json!(2), Some("typescript"));
        assert_eq!(doc.raw["[typescript]"]["editor.tabSize"], 2);

        let rebuilt = LayerDocument::from_raw(doc.raw.clone(), accept_everything);
        assert_eq!(rebuilt.languages["typescript"]["editor"]["tabSize"], 2);
    }

    #[test]
    fn removing_a_key_prunes_what_it_empties() {
        let mut doc = document(json!({ "editor": { "fontSize": 14 } }));
        assert!(doc.raw_remove("editor.fontSize", None));
        assert!(doc.raw.is_empty());
        assert!(!doc.raw_remove("editor.fontSize", None));

        let mut lang = LayerDocument::empty();
        lang.raw_set("editor.tabSize", json!(2), Some("go"));
        assert!(lang.raw_remove("editor.tabSize", Some("go")));
        assert!(lang.raw.is_empty(), "an emptied language section goes too");
    }

    #[test]
    fn the_user_layer_path_sits_under_the_home_directory() {
        if let Some(path) = user_settings_path() {
            assert!(path.ends_with(PathBuf::from(".helix").join("settings.json")));
        }
    }

    #[test]
    fn workspace_and_folder_paths_sit_inside_their_roots() {
        let paths = ConfigPaths::for_user()
            .with_workspace_root("/tmp/project")
            .with_folder_root("/tmp/project/packages/api");
        assert!(
            paths
                .path(ConfigScope::Workspace)
                .unwrap()
                .ends_with(PathBuf::from(".helix").join("settings.json"))
        );
        assert!(
            paths
                .path(ConfigScope::Folder)
                .unwrap()
                .to_string_lossy()
                .contains("api")
        );
        assert_eq!(paths.path(ConfigScope::Default), None);
    }

    #[test]
    fn a_qualified_key_reads_the_way_a_user_would_type_it() {
        assert_eq!(
            qualified_key("editor.tabSize", Some("typescript")),
            "[typescript].editor.tabSize"
        );
        assert_eq!(qualified_key("editor.tabSize", None), "editor.tabSize");
    }
}
