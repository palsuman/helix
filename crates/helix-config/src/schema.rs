//! The settings schema: every built-in setting, its type, its default, and
//! whether changing it requires a restart (REQ-CONFIG-001.5, .8).
//!
//! The schema is the single source of truth for four separate consumers, which
//! is why it is data rather than scattered `unwrap_or` calls at read sites:
//!
//! 1. **Defaults.** The lowest layer of the merge is built from it, so every
//!    known key always resolves to *something* and no caller needs a fallback.
//! 2. **Validation.** A wrong type in a settings file is reported and the
//!    default used, rather than propagating a `null` into a consumer that
//!    expected a number.
//! 3. **Restart flags.** A change notification says which of the changed keys
//!    the running process cannot actually apply (REQ-CONFIG-001.8).
//! 4. **The editors.** [`SchemaRegistry::json_schema`] emits a JSON Schema the
//!    JSON settings editor validates and completes against, and the GUI editor
//!    (Task 9.1) renders controls from the same entries.
//!
//! Unknown keys are *not* an error. Plugins contribute settings this system has
//! never heard of, and a workspace file written by a newer Helix must keep
//! working in an older one, so an unrecognized key is preserved and warned
//! about rather than dropped (REQ-CONFIG-001 failure mode: "warn but
//! preserve").

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

use crate::layer::ConfigScope;

/// The value shape a setting accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum SettingKind {
    Boolean,
    Integer,
    Number,
    String,
    Array,
    Object,
}

impl SettingKind {
    fn accepts(self, value: &Value) -> bool {
        match self {
            SettingKind::Boolean => value.is_boolean(),
            // An integer-typed setting accepts `2.0` and rejects `2.5`: JSON
            // has one number type and a frontend that round-trips through a
            // number input should not have to care.
            SettingKind::Integer => value
                .as_f64()
                .map(|n| n.fract() == 0.0 && n.is_finite())
                .unwrap_or(false),
            SettingKind::Number => value.as_f64().map(f64::is_finite).unwrap_or(false),
            SettingKind::String => value.is_string(),
            SettingKind::Array => value.is_array(),
            SettingKind::Object => value.is_object(),
        }
    }

    fn describe(self) -> &'static str {
        match self {
            SettingKind::Boolean => "a boolean",
            SettingKind::Integer => "an integer",
            SettingKind::Number => "a number",
            SettingKind::String => "a string",
            SettingKind::Array => "an array",
            SettingKind::Object => "an object",
        }
    }

    fn json_type(self) -> &'static str {
        match self {
            SettingKind::Boolean => "boolean",
            SettingKind::Integer => "integer",
            SettingKind::Number => "number",
            SettingKind::String => "string",
            SettingKind::Array => "array",
            SettingKind::Object => "object",
        }
    }
}

/// One setting's declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SettingSchema {
    /// Dotted key, e.g. `editor.fontSize`.
    pub key: String,
    pub kind: SettingKind,
    #[ts(type = "unknown")]
    pub default: Value,
    pub description: String,
    /// Category the settings UI groups this under.
    pub category: String,
    /// Allowed values, when the setting is an enumeration.
    #[ts(type = "unknown[] | null")]
    pub allowed: Option<Vec<Value>>,
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    /// True when a change only takes effect after a restart
    /// (REQ-CONFIG-001.8).
    pub requires_restart: bool,
    /// Layers this setting may be written to. A machine-level setting such as
    /// a shell path has no business being decided by a cloned repository.
    pub writable_scopes: Vec<ConfigScope>,
    /// True when the value names an executable, which Restricted mode must
    /// ignore (REQ-CONFIG-001.11, consumed by Task 1.13).
    pub executable_path: bool,
    /// True when a language-specific override (`[typescript].editor.tabSize`)
    /// is meaningful. Editor settings yes; a global theme no.
    pub language_overridable: bool,
}

impl SettingSchema {
    fn new(
        key: &str,
        kind: SettingKind,
        default: Value,
        category: &str,
        description: &str,
    ) -> Self {
        Self {
            key: key.to_string(),
            kind,
            default,
            description: description.to_string(),
            category: category.to_string(),
            allowed: None,
            minimum: None,
            maximum: None,
            requires_restart: false,
            writable_scopes: vec![
                ConfigScope::User,
                ConfigScope::Workspace,
                ConfigScope::Folder,
            ],
            executable_path: false,
            language_overridable: false,
        }
    }

    fn allowed(mut self, values: &[&str]) -> Self {
        self.allowed = Some(values.iter().map(|v| json!(v)).collect());
        self
    }

    fn range(mut self, minimum: f64, maximum: f64) -> Self {
        self.minimum = Some(minimum);
        self.maximum = Some(maximum);
        self
    }

    fn requires_restart(mut self) -> Self {
        self.requires_restart = true;
        self
    }

    fn user_only(mut self) -> Self {
        self.writable_scopes = vec![ConfigScope::User];
        self
    }

    fn executable_path(mut self) -> Self {
        self.executable_path = true;
        self
    }

    fn language_overridable(mut self) -> Self {
        self.language_overridable = true;
        self
    }

    /// Whether this setting may be written to the given layer.
    pub fn writable_in(&self, scope: ConfigScope) -> bool {
        self.writable_scopes.contains(&scope)
    }
}

/// Why a value in a settings file was not usable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum IssueKind {
    /// Not a known setting. Preserved, not dropped.
    UnknownKey,
    /// Wrong shape for the declared type. The default is used instead.
    TypeMismatch,
    /// Not one of the declared allowed values. The default is used instead.
    NotAllowed,
    /// Outside the declared range. The default is used instead.
    OutOfRange,
    /// Looks like a credential. Rejected outright (REQ-CONFIG-001.12).
    Secret,
    /// Written to a layer this setting may not be set from.
    WrongScope,
}

/// A single problem with a single key, carrying enough detail for the
/// settings editor to point at it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SettingIssue {
    pub key: String,
    pub scope: ConfigScope,
    pub kind: IssueKind,
    pub message: String,
    /// Set when the issue applies to a language-specific override.
    pub language: Option<String>,
}

impl SettingIssue {
    pub fn new(
        key: impl Into<String>,
        scope: ConfigScope,
        kind: IssueKind,
        message: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            scope,
            kind,
            message: message.into(),
            language: None,
        }
    }

    pub fn with_language(mut self, language: Option<String>) -> Self {
        self.language = language;
        self
    }

    /// Whether the offending value is discarded during resolution. An
    /// unknown key is kept (forward compatibility); everything else is not.
    pub fn discards_value(&self) -> bool {
        !matches!(self.kind, IssueKind::UnknownKey)
    }
}

/// Every known setting, keyed for lookup.
#[derive(Debug, Clone)]
pub struct SchemaRegistry {
    entries: BTreeMap<String, SettingSchema>,
}

impl Default for SchemaRegistry {
    fn default() -> Self {
        Self::builtin()
    }
}

impl SchemaRegistry {
    /// The built-in settings shipped with the kernel.
    ///
    /// Deliberately limited to settings something in the plan actually reads.
    /// Declaring a key here is a promise that it is honoured, so a setting
    /// arrives with the feature that consumes it rather than ahead of it.
    pub fn builtin() -> Self {
        let entries = [
            // ---- editor ------------------------------------------------
            SettingSchema::new(
                "editor.fontSize",
                SettingKind::Integer,
                json!(14),
                "Editor",
                "Font size, in pixels, used by the code editor.",
            )
            .range(6.0, 72.0)
            .language_overridable(),
            SettingSchema::new(
                "editor.fontFamily",
                SettingKind::String,
                json!("Cascadia Code, Menlo, Consolas, monospace"),
                "Editor",
                "Font family used by the code editor.",
            ),
            SettingSchema::new(
                "editor.tabSize",
                SettingKind::Integer,
                json!(4),
                "Editor",
                "Number of spaces a tab is rendered and inserted as.",
            )
            .range(1.0, 16.0)
            .language_overridable(),
            SettingSchema::new(
                "editor.insertSpaces",
                SettingKind::Boolean,
                json!(true),
                "Editor",
                "Insert spaces rather than tab characters.",
            )
            .language_overridable(),
            SettingSchema::new(
                "editor.wordWrap",
                SettingKind::String,
                json!("off"),
                "Editor",
                "How long lines wrap.",
            )
            .allowed(&["off", "on", "wordWrapColumn", "bounded"])
            .language_overridable(),
            SettingSchema::new(
                "editor.rulers",
                SettingKind::Array,
                json!([]),
                "Editor",
                "Columns to draw vertical rulers at. A higher layer replaces this list rather than adding to it.",
            )
            .language_overridable(),
            SettingSchema::new(
                "editor.minimap.enabled",
                SettingKind::Boolean,
                json!(true),
                "Editor",
                "Show the minimap.",
            ),
            SettingSchema::new(
                "editor.lineNumbers",
                SettingKind::String,
                json!("on"),
                "Editor",
                "How line numbers are displayed.",
            )
            .allowed(&["off", "on", "relative", "interval"]),
            SettingSchema::new(
                "editor.renderWhitespace",
                SettingKind::String,
                json!("selection"),
                "Editor",
                "When whitespace characters are rendered.",
            )
            .allowed(&["none", "boundary", "selection", "trailing", "all"]),
            SettingSchema::new(
                "editor.bracketPairColorization",
                SettingKind::Boolean,
                json!(true),
                "Editor",
                "Colour matching bracket pairs by nesting depth.",
            ),
            SettingSchema::new(
                "editor.formatOnSave",
                SettingKind::Boolean,
                json!(false),
                "Editor",
                "Format the document when it is saved.",
            )
            .language_overridable(),
            SettingSchema::new(
                "editor.inlineCompletion.enabled",
                SettingKind::Boolean,
                json!(true),
                "AI",
                "Offer AI inline completions as ghost text.",
            )
            .language_overridable(),
            // ---- files -------------------------------------------------
            SettingSchema::new(
                "files.autoSave",
                SettingKind::String,
                json!("off"),
                "Files",
                "When modified editors are saved automatically.",
            )
            .allowed(&["off", "afterDelay", "onFocusChange", "onWindowChange"]),
            SettingSchema::new(
                "files.autoSaveDelayMs",
                SettingKind::Integer,
                json!(1000),
                "Files",
                "Delay before an automatic save when files.autoSave is afterDelay.",
            )
            .range(100.0, 300_000.0),
            SettingSchema::new(
                "files.walIntervalMs",
                SettingKind::Integer,
                json!(1000),
                "Files",
                "How often unsaved buffers are written to the write-ahead log. This is the recovery point objective for an abrupt termination.",
            )
            .range(100.0, 60_000.0),
            SettingSchema::new(
                "state.retentionDays",
                SettingKind::Integer,
                json!(30),
                "Files",
                "Days to retain session state after every workspace root becomes unavailable.",
            )
            .range(1.0, 3650.0)
            .user_only(),
            SettingSchema::new(
                "files.encoding",
                SettingKind::String,
                json!("utf8"),
                "Files",
                "Default encoding used when a file's encoding cannot be detected.",
            )
            // Spelled the way `helix_fs::Encoding` serialises, so the setting,
            // the IPC field, and the log field are one vocabulary rather than
            // three that have to be translated between.
            .allowed(&["utf8", "utf8_bom", "utf16_le", "utf16_be", "latin1"])
            .language_overridable(),
            SettingSchema::new(
                "files.eol",
                SettingKind::String,
                json!("auto"),
                "Files",
                "Line ending written to new files. `auto` follows the platform.",
            )
            .allowed(&["auto", "lf", "crlf"]),
            SettingSchema::new(
                "files.trimTrailingWhitespace",
                SettingKind::Boolean,
                json!(false),
                "Files",
                "Remove trailing whitespace when saving.",
            ),
            SettingSchema::new(
                "files.insertFinalNewline",
                SettingKind::Boolean,
                json!(false),
                "Files",
                "Ensure a trailing newline when saving.",
            ),
            SettingSchema::new(
                "files.exclude",
                SettingKind::Object,
                json!({ "**/.git": true, "**/node_modules": true, "**/target": true }),
                "Files",
                "Glob patterns excluded from the explorer, search, and file watching. Merged key by key across layers, so a workspace can re-include a pattern the user excluded.",
            ),
            SettingSchema::new(
                "files.watcherExclude",
                SettingKind::Object,
                json!({ "**/.git/objects/**": true, "**/node_modules/**": true }),
                "Files",
                "Glob patterns excluded from file watching only.",
            ),
            SettingSchema::new(
                "files.watchDepth",
                SettingKind::Integer,
                json!(0),
                "Files",
                "How many directory levels below each root are watched. 0 means no limit.",
            )
            .range(0.0, 64.0),
            SettingSchema::new(
                "files.respectGitignore",
                SettingKind::Boolean,
                json!(true),
                "Files",
                "Skip files ignored by .gitignore when listing, searching, and watching.",
            ),
            // ---- workspace ---------------------------------------------
            // REQ-FS-001.5 sets the maximum at 20 and calls it configurable, so
            // this lowers the cap rather than raising it: the requirement's
            // ceiling holds, and a team that wants a tighter limit (or a
            // machine that cannot afford 20 watchers) can say so.
            SettingSchema::new(
                "workspace.maxRoots",
                SettingKind::Integer,
                json!(20),
                "Workspace",
                "Maximum number of folders in one multi-root workspace.",
            )
            .range(1.0, 20.0),
            // ---- workbench ---------------------------------------------
            SettingSchema::new(
                "workbench.colorTheme",
                SettingKind::String,
                json!("Helix Dark"),
                "Appearance",
                "Active colour theme.",
            ),
            SettingSchema::new(
                "workbench.iconTheme",
                SettingKind::String,
                json!("helix-colored"),
                "Appearance",
                "Active file icon theme.",
            ),
            SettingSchema::new(
                "workbench.productIconTheme",
                SettingKind::String,
                json!("helix-default"),
                "Appearance",
                "Active product icon theme.",
            ),
            SettingSchema::new(
                "workbench.startupEditor",
                SettingKind::String,
                json!("welcomePage"),
                "Appearance",
                "What is shown when a window opens with no file restored.",
            )
            .allowed(&["none", "welcomePage", "readme", "newUntitledFile"]),
            SettingSchema::new(
                "workbench.reduceMotion",
                SettingKind::String,
                json!("auto"),
                "Appearance",
                "Whether animations are reduced. `auto` follows the operating system.",
            )
            .allowed(&["auto", "on", "off"]),
            // ---- terminal ----------------------------------------------
            SettingSchema::new(
                "terminal.fontSize",
                SettingKind::Integer,
                json!(13),
                "Terminal",
                "Font size, in pixels, used by the integrated terminal.",
            )
            .range(6.0, 72.0),
            SettingSchema::new(
                "terminal.scrollback",
                SettingKind::Integer,
                json!(10_000),
                "Terminal",
                "Lines of scrollback retained per terminal.",
            )
            .range(100.0, 100_000.0),
            SettingSchema::new(
                "terminal.defaultProfile",
                SettingKind::String,
                json!(""),
                "Terminal",
                "Named shell profile used for new terminals. Empty means the detected system default.",
            ),
            SettingSchema::new(
                "terminal.shellPath",
                SettingKind::String,
                json!(""),
                "Terminal",
                "Absolute path to the shell executable. Ignored in Restricted mode.",
            )
            .executable_path()
            .user_only(),
            // ---- logging and diagnostics -------------------------------
            SettingSchema::new(
                "log.level",
                SettingKind::String,
                json!("info"),
                "Diagnostics",
                "Minimum level recorded by the logger.",
            )
            .allowed(&["trace", "debug", "info", "warn", "error"]),
            SettingSchema::new(
                "log.moduleLevels",
                SettingKind::Object,
                json!({}),
                "Diagnostics",
                "Per-module level overrides, keyed by log source name.",
            ),
            SettingSchema::new(
                "telemetry.enabled",
                SettingKind::Boolean,
                json!(false),
                "Diagnostics",
                "Send performance telemetry. Off unless explicitly enabled; local collection happens regardless.",
            )
            .user_only(),
            SettingSchema::new(
                "crashReporting.enabled",
                SettingKind::Boolean,
                json!(false),
                "Diagnostics",
                "Send crash reports. Off unless explicitly enabled.",
            )
            .user_only(),
            // ---- streaming and IPC -------------------------------------
            SettingSchema::new(
                "stream.bufferDepth",
                SettingKind::Integer,
                json!(1000),
                "Advanced",
                "Messages retained per streaming channel before the oldest is dropped.",
            )
            .range(10.0, 100_000.0)
            .requires_restart(),
            SettingSchema::new(
                "ipc.timeoutMs",
                SettingKind::Integer,
                json!(30_000),
                "Advanced",
                "Default timeout applied to an IPC command that does not specify one.",
            )
            .range(100.0, 600_000.0)
            .requires_restart(),
            SettingSchema::new(
                "config.watchIntervalMs",
                SettingKind::Integer,
                json!(400),
                "Advanced",
                "How often settings files are checked for external edits.",
            )
            .range(50.0, 10_000.0),
            // ---- platform ----------------------------------------------
            SettingSchema::new(
                "helix.locale",
                SettingKind::String,
                json!("auto"),
                "Platform",
                "User interface locale. `auto` follows the operating system.",
            )
            .requires_restart()
            .user_only(),
            SettingSchema::new(
                "update.channel",
                SettingKind::String,
                json!("stable"),
                "Platform",
                "Release channel checked for updates.",
            )
            .allowed(&["stable", "beta", "nightly"])
            .requires_restart()
            .user_only(),
            // ---- AI ----------------------------------------------------
            SettingSchema::new(
                "ai.providers",
                SettingKind::Object,
                json!({}),
                "AI",
                "Configured model providers, keyed by name. Each entry names a keychain entry by id; credentials themselves are never stored in settings.",
            ),
            SettingSchema::new(
                "ai.defaultModel",
                SettingKind::String,
                json!(""),
                "AI",
                "Model used when a task type has no specific routing rule.",
            ),
            // Named `dailyBudget` rather than `dailyTokenBudget` because
            // "token" reads as a credential to the secret detector, which
            // would make the setting impossible to write.
            SettingSchema::new(
                "ai.dailyBudget",
                SettingKind::Integer,
                json!(0),
                "AI",
                "Daily ceiling on tokens consumed across all providers. 0 means no limit.",
            )
            .range(0.0, 1_000_000_000.0),
        ]
        .into_iter()
        .map(|entry| (entry.key.clone(), entry))
        .collect();

        Self { entries }
    }

    /// An empty registry, for tests that want to reason about merge behaviour
    /// without the built-in surface.
    pub fn empty() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Add or replace a declaration. This is the seam plugin-contributed
    /// settings use in Task 17.3.
    pub fn insert(&mut self, schema: SettingSchema) {
        self.entries.insert(schema.key.clone(), schema);
    }

    pub fn get(&self, key: &str) -> Option<&SettingSchema> {
        self.entries.get(key)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &SettingSchema> {
        self.entries.values()
    }

    /// Whether a change to `key` needs a restart to take effect. An unknown
    /// key is assumed not to, because guessing "restart required" for a
    /// plugin setting would make every plugin's settings feel broken.
    pub fn requires_restart(&self, key: &str) -> bool {
        self.get(key).map(|s| s.requires_restart).unwrap_or(false)
    }

    /// The defaults layer: every declared key at its default value, as a
    /// tree.
    pub fn defaults_tree(&self) -> Value {
        let mut tree = Value::Object(serde_json::Map::new());
        for schema in self.entries.values() {
            crate::merge::set_path(&mut tree, &schema.key, schema.default.clone());
        }
        tree
    }

    /// Validate one value against its declaration.
    ///
    /// `Ok(())` for a known, valid value *and* for an unknown key: an unknown
    /// key produces an [`IssueKind::UnknownKey`] issue at the call site rather
    /// than a validation failure, because the two are handled differently
    /// (preserve vs discard).
    pub fn validate(&self, key: &str, value: &Value) -> Result<(), (IssueKind, String)> {
        let Some(schema) = self.get(key) else {
            return Ok(());
        };

        if !schema.kind.accepts(value) {
            return Err((
                IssueKind::TypeMismatch,
                format!(
                    "'{key}' expects {}, found {}; the default is used instead",
                    schema.kind.describe(),
                    describe_value(value)
                ),
            ));
        }

        if let Some(allowed) = &schema.allowed
            && !allowed.contains(value)
        {
            let rendered: Vec<String> = allowed.iter().map(render_value).collect();
            return Err((
                IssueKind::NotAllowed,
                format!(
                    "'{key}' must be one of {}; found {}",
                    rendered.join(", "),
                    render_value(value)
                ),
            ));
        }

        if let Some(number) = value.as_f64() {
            if let Some(minimum) = schema.minimum
                && number < minimum
            {
                return Err((
                    IssueKind::OutOfRange,
                    format!("'{key}' must be at least {minimum}, found {number}"),
                ));
            }
            if let Some(maximum) = schema.maximum
                && number > maximum
            {
                return Err((
                    IssueKind::OutOfRange,
                    format!("'{key}' must be at most {maximum}, found {number}"),
                ));
            }
        }

        Ok(())
    }

    /// A JSON Schema document describing every built-in setting, for the JSON
    /// settings editor's completion and validation (REQ-CONFIG-001.5).
    ///
    /// Both authoring forms are described: flat dotted keys as properties, and
    /// `[language]` sections as pattern properties, so neither form is
    /// flagged as unknown by the editor.
    pub fn json_schema(&self) -> Value {
        let mut properties = serde_json::Map::new();
        for schema in self.entries.values() {
            properties.insert(schema.key.clone(), self.property_schema(schema));
        }

        let mut language_properties = serde_json::Map::new();
        for schema in self.entries.values().filter(|s| s.language_overridable) {
            language_properties.insert(schema.key.clone(), self.property_schema(schema));
        }

        json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "title": "Helix settings",
            "type": "object",
            "properties": properties,
            "patternProperties": {
                "^\\[[^\\]]+\\]$": {
                    "type": "object",
                    "description": "Language-specific overrides, e.g. \"[typescript]\": { \"editor.tabSize\": 2 }.",
                    "properties": language_properties,
                    "additionalProperties": true
                }
            },
            "additionalProperties": true
        })
    }

    fn property_schema(&self, schema: &SettingSchema) -> Value {
        let mut property = serde_json::Map::new();
        property.insert("type".into(), json!(schema.kind.json_type()));
        property.insert("default".into(), schema.default.clone());
        property.insert("description".into(), json!(schema.description));
        property.insert("x-helixCategory".into(), json!(schema.category));
        if schema.requires_restart {
            property.insert("x-helixRequiresRestart".into(), json!(true));
            property.insert(
                "markdownDescription".into(),
                json!(format!(
                    "{} \n\n_Requires a restart to take effect._",
                    schema.description
                )),
            );
        }
        if schema.executable_path {
            property.insert("x-helixExecutablePath".into(), json!(true));
        }
        if let Some(allowed) = &schema.allowed {
            property.insert("enum".into(), json!(allowed));
        }
        if let Some(minimum) = schema.minimum {
            property.insert("minimum".into(), json!(minimum));
        }
        if let Some(maximum) = schema.maximum {
            property.insert("maximum".into(), json!(maximum));
        }
        Value::Object(property)
    }
}

fn describe_value(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

fn render_value(value: &Value) -> String {
    match value {
        Value::String(s) => format!("'{s}'"),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_setting_default_satisfies_its_own_declaration() {
        // A default that fails its own validation would make the lowest
        // layer of the merge invalid, which no consumer could recover from.
        let registry = SchemaRegistry::builtin();
        for schema in registry.iter() {
            assert!(
                registry.validate(&schema.key, &schema.default).is_ok(),
                "default for '{}' fails validation: {:?}",
                schema.key,
                registry.validate(&schema.key, &schema.default)
            );
        }
    }

    #[test]
    fn no_builtin_key_is_shaped_like_a_credential() {
        // REQ-CONFIG-001.12 rejects secrets in settings files. A built-in
        // key whose own name trips the secret detector would be unsettable.
        for schema in SchemaRegistry::builtin().iter() {
            for segment in schema.key.split('.') {
                assert!(
                    !helix_log::redact::is_secret_key(segment),
                    "built-in key '{}' contains credential-shaped segment '{segment}'",
                    schema.key
                );
            }
        }
    }

    #[test]
    fn defaults_tree_contains_every_declared_key() {
        let registry = SchemaRegistry::builtin();
        let tree = registry.defaults_tree();
        for schema in registry.iter() {
            assert_eq!(
                crate::merge::get_path(&tree, &schema.key),
                Some(&schema.default),
                "{}",
                schema.key
            );
        }
    }

    #[test]
    fn a_type_mismatch_is_reported() {
        let registry = SchemaRegistry::builtin();
        let (kind, message) = registry
            .validate("editor.fontSize", &json!("large"))
            .unwrap_err();
        assert_eq!(kind, IssueKind::TypeMismatch);
        assert!(message.contains("expects an integer"), "{message}");
    }

    #[test]
    fn an_integral_float_is_accepted_for_an_integer_setting_but_a_fraction_is_not() {
        let registry = SchemaRegistry::builtin();
        assert!(registry.validate("editor.tabSize", &json!(2.0)).is_ok());
        assert_eq!(
            registry
                .validate("editor.tabSize", &json!(2.5))
                .unwrap_err()
                .0,
            IssueKind::TypeMismatch
        );
    }

    #[test]
    fn a_value_outside_the_declared_enumeration_is_reported() {
        let registry = SchemaRegistry::builtin();
        let (kind, message) = registry
            .validate("files.autoSave", &json!("sometimes"))
            .unwrap_err();
        assert_eq!(kind, IssueKind::NotAllowed);
        assert!(message.contains("onFocusChange"), "{message}");
    }

    #[test]
    fn a_value_outside_the_declared_range_is_reported() {
        let registry = SchemaRegistry::builtin();
        assert_eq!(
            registry
                .validate("editor.fontSize", &json!(0))
                .unwrap_err()
                .0,
            IssueKind::OutOfRange
        );
        assert_eq!(
            registry
                .validate("editor.fontSize", &json!(900))
                .unwrap_err()
                .0,
            IssueKind::OutOfRange
        );
    }

    #[test]
    fn an_unknown_key_passes_validation_so_it_can_be_preserved() {
        let registry = SchemaRegistry::builtin();
        assert!(
            registry
                .validate("somePlugin.someSetting", &json!({ "anything": true }))
                .is_ok()
        );
    }

    #[test]
    fn restart_required_settings_are_flagged_and_ordinary_ones_are_not() {
        let registry = SchemaRegistry::builtin();
        assert!(registry.requires_restart("stream.bufferDepth"));
        assert!(registry.requires_restart("helix.locale"));
        assert!(!registry.requires_restart("editor.fontSize"));
        assert!(!registry.requires_restart("unknown.plugin.setting"));
    }

    #[test]
    fn a_machine_scoped_setting_is_not_writable_from_a_workspace() {
        let registry = SchemaRegistry::builtin();
        let shell = registry.get("terminal.shellPath").unwrap();
        assert!(shell.writable_in(ConfigScope::User));
        assert!(
            !shell.writable_in(ConfigScope::Workspace),
            "a cloned repository must not be able to name the shell executable"
        );
        assert!(shell.executable_path);
    }

    #[test]
    fn the_json_schema_describes_both_authoring_forms() {
        let registry = SchemaRegistry::builtin();
        let schema = registry.json_schema();
        assert_eq!(schema["properties"]["editor.fontSize"]["type"], "integer");
        assert_eq!(schema["properties"]["editor.fontSize"]["default"], 14);
        assert_eq!(
            schema["properties"]["stream.bufferDepth"]["x-helixRequiresRestart"],
            true
        );
        assert!(
            schema["patternProperties"]["^\\[[^\\]]+\\]$"]["properties"]["editor.tabSize"]
                .is_object(),
            "language sections must describe the overridable settings"
        );
        assert!(
            schema["patternProperties"]["^\\[[^\\]]+\\]$"]["properties"]["workbench.colorTheme"]
                .is_null(),
            "a global-only setting has no business in a language section"
        );
    }

    #[test]
    fn a_contributed_setting_joins_the_registry() {
        let mut registry = SchemaRegistry::empty();
        registry.insert(SettingSchema::new(
            "plugin.thing",
            SettingKind::Boolean,
            json!(false),
            "Extensions",
            "A contributed setting.",
        ));
        assert!(registry.contains("plugin.thing"));
        assert_eq!(registry.len(), 1);
        assert_eq!(
            registry
                .validate("plugin.thing", &json!("yes"))
                .unwrap_err()
                .0,
            IssueKind::TypeMismatch
        );
    }
}
