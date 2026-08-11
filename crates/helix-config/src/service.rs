//! The configuration service: layered resolution, reads, writes, and change
//! detection (Task 1.6, REQ-CONFIG-001).
//!
//! Like `Logger` in `helix-log`, this is the subsystem itself and knows nothing
//! about Tauri or the service container. `helix-kernel` wraps it as a managed
//! service, registers its commands, and bridges its change notifications onto
//! the streaming channel.
//!
//! ## Shape
//!
//! ```text
//!  defaults (schema)  ─┐
//!  ~/.helix/settings.json ─┤
//!  <root>/.helix/settings.json ─┼─► deep merge ──► resolved tree ──► get / list
//!  <folder>/.helix/settings.json ─┘        │
//!                                          └─► changed key set ──► listeners
//! ```
//!
//! ## Decisions worth naming
//!
//! **Last-known-good on a parse error.** A settings file being edited is
//! momentarily invalid on almost every keystroke. Dropping to defaults each
//! time would make the editor flicker between two personalities while someone
//! types, so an unparseable file keeps the last values that *did* parse and the
//! error is surfaced with its location instead (REQ-CONFIG-001 failure modes).
//!
//! **Invalid values lose individually.** A single type mismatch discards that
//! one key, not the file. The rest of the layer still applies, because the
//! alternative punishes a user for one typo by silently reverting twenty
//! unrelated settings.
//!
//! **Unknown keys survive.** They are warned about once and preserved, so a
//! plugin's settings and a file written by a newer build both keep working.
//!
//! **Polling, for now.** External edits are detected by re-reading the layer
//! files on an interval (`config.watchIntervalMs`, default 400ms) rather than
//! with an OS watcher. Task 1.7 owns file watching; wiring config to a watcher
//! that does not exist yet would invert the dependency order the plan is built
//! on. The files are kilobytes and there are at most three, so the cost is
//! negligible, and the 1s propagation the task's demo asks for is met with
//! room to spare.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use helix_core::error::AppError;
use helix_log::{Logger, log_debug, log_error, log_info, log_warn};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use ts_rs::TS;

use crate::jsonc::{ConfigParseError, parse_object};
use crate::layer::{ConfigPaths, ConfigScope, LayerDocument, LeafDecision, qualified_key};
use crate::merge::{changed_keys, deep_merge, flatten_leaves, get_path};
use crate::schema::{IssueKind, SchemaRegistry, SettingIssue, SettingSchema};
use crate::secrets;

/// Log source for configuration records.
pub const LOG_SOURCE: &str = "kernel.config";

/// How a change to the resolved configuration came about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum ChangeOrigin {
    /// The initial load at startup.
    Initial,
    /// A `config.set` or `config.reset` this process performed.
    Internal,
    /// An edit made outside the application, detected by watching the file.
    External,
}

/// A change to the resolved configuration, delivered to listeners and
/// published on the streaming channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct ConfigChange {
    /// Which layer changed.
    pub scope: ConfigScope,
    pub origin: ChangeOrigin,
    /// Effective keys whose value changed, in the syntax a user would type:
    /// `editor.fontSize`, or `[typescript].editor.tabSize` for a
    /// language-specific override.
    pub changed_keys: Vec<String>,
    /// The subset of `changed_keys` that cannot take effect until the
    /// application restarts (REQ-CONFIG-001.8).
    pub requires_restart: Vec<String>,
    /// Set when the layer's file could not be parsed. The previous values
    /// remain in effect.
    pub parse_error: Option<ConfigParseError>,
    /// Per-key problems found while loading the layer.
    pub issues: Vec<SettingIssue>,
}

impl ConfigChange {
    /// Whether anything a consumer cares about actually moved. A parse error
    /// with no value change still matters to the settings editor, so it counts.
    pub fn is_meaningful(&self) -> bool {
        !self.changed_keys.is_empty() || self.parse_error.is_some() || !self.issues.is_empty()
    }
}

/// One setting's effective state, as reported by `config.get` and
/// `config.list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SettingValue {
    pub key: String,
    pub language: Option<String>,
    #[ts(type = "unknown")]
    pub value: Value,
    #[ts(type = "unknown")]
    pub default: Value,
    /// The layer the effective value came from.
    pub scope: ConfigScope,
    /// True when no layer above defaults sets this key.
    pub is_default: bool,
    /// True when the key is declared in the schema. A false here is a
    /// plugin-contributed or forward-compatible key, not an error.
    pub known: bool,
    pub requires_restart: bool,
    /// Present for known settings, so the editors can render a control and a
    /// description without a second round trip.
    pub schema: Option<SettingSchema>,
}

/// Counters behind the service's health report.
#[derive(Debug, Default)]
struct Counters {
    reloads: AtomicU64,
    writes: AtomicU64,
    write_errors: AtomicU64,
    parse_errors: AtomicU64,
    secrets_rejected: AtomicU64,
}

/// Point-in-time counters, surfaced through the kernel's health model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigMetrics {
    pub reloads: u64,
    pub writes: u64,
    pub write_errors: u64,
    pub parse_errors: u64,
    pub secrets_rejected: u64,
}

/// What a layer's file looked like the last time it was read.
///
/// Content-hashed rather than only stat'ed, because two edits can share a
/// length and a coarse modification time (`14` to `16` inside one filesystem
/// timestamp tick) and a settings change that failed to propagate because of
/// timestamp granularity would be an infuriating bug to chase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Fingerprint {
    len: u64,
    hash: u64,
}

impl Fingerprint {
    fn of(body: &str) -> Self {
        Self {
            len: body.len() as u64,
            hash: fnv1a64(body.as_bytes()),
        }
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash
}

/// One layer's loaded state.
#[derive(Debug, Clone)]
struct LayerState {
    document: LayerDocument,
    fingerprint: Option<Fingerprint>,
    parse_error: Option<ConfigParseError>,
    issues: Vec<SettingIssue>,
}

impl LayerState {
    fn empty() -> Self {
        Self {
            document: LayerDocument::empty(),
            fingerprint: None,
            parse_error: None,
            issues: Vec::new(),
        }
    }
}

/// The merged view every read is served from.
#[derive(Debug, Clone, Default)]
pub struct Resolved {
    /// Merged tree with no language overrides applied.
    pub global: Value,
    /// Merged tree per language, with that language's overrides applied.
    pub languages: BTreeMap<String, Value>,
    /// Flattened effective values, keyed the way a change notification reports
    /// them. Language entries appear only for keys some layer actually
    /// overrides, so a global change does not report a change for every
    /// language.
    effective: BTreeMap<String, Value>,
}

impl Resolved {
    /// The effective value for a key, honouring a language override.
    pub fn get(&self, key: &str, language: Option<&str>) -> Option<&Value> {
        if let Some(language) = language
            && let Some(tree) = self.languages.get(language)
            && let Some(value) = get_path(tree, key)
        {
            return Some(value);
        }
        get_path(&self.global, key)
    }
}

struct State {
    layers: BTreeMap<ConfigScope, LayerState>,
    resolved: Arc<Resolved>,
}

type ChangeListener = Arc<dyn Fn(&ConfigChange) + Send + Sync>;

/// The layered configuration service.
pub struct ConfigService {
    schema: Arc<SchemaRegistry>,
    logger: Arc<Logger>,
    paths: ConfigPaths,
    state: RwLock<State>,
    listeners: RwLock<Vec<ChangeListener>>,
    counters: Counters,
}

impl ConfigService {
    /// Load every layer and resolve them.
    ///
    /// Never fails: a missing file is an empty layer, an unreadable one keeps
    /// the layer empty and logs, and an unparseable one is reported while the
    /// process continues on the layers that did load. Configuration is not
    /// allowed to be the reason the editor will not start.
    pub fn load(paths: ConfigPaths, schema: Arc<SchemaRegistry>, logger: Arc<Logger>) -> Self {
        let mut layers = BTreeMap::new();
        layers.insert(
            ConfigScope::Default,
            LayerState {
                document: LayerDocument::from_tree(schema.defaults_tree()),
                ..LayerState::empty()
            },
        );

        let service = Self {
            schema,
            logger,
            paths,
            state: RwLock::new(State {
                layers,
                resolved: Arc::new(Resolved::default()),
            }),
            listeners: RwLock::new(Vec::new()),
            counters: Counters::default(),
        };

        for scope in ConfigScope::ASCENDING {
            if !scope.is_writable() {
                continue;
            }
            let state = service.read_layer(scope);
            service.state.write().unwrap().layers.insert(scope, state);
        }
        service.recompute();

        let resolved = service.snapshot();
        log_info!(
            service.logger,
            LOG_SOURCE,
            "configuration loaded",
            "layers" => service.configured_layers().len(),
            "settings" => resolved.effective.len(),
            "parse_errors" => service.parse_errors().len(),
        );
        service
    }

    /// A service with only the defaults layer, for tests and for a window with
    /// no workspace and no home directory.
    pub fn defaults_only(schema: Arc<SchemaRegistry>, logger: Arc<Logger>) -> Self {
        Self::load(ConfigPaths::default(), schema, logger)
    }

    pub fn schema(&self) -> &Arc<SchemaRegistry> {
        &self.schema
    }

    pub fn paths(&self) -> &ConfigPaths {
        &self.paths
    }

    /// Layers that have a file configured, lowest precedence first.
    pub fn configured_layers(&self) -> Vec<ConfigScope> {
        ConfigScope::ASCENDING
            .into_iter()
            .filter(|scope| self.paths.path(*scope).is_some())
            .collect()
    }

    /// The current merged view. Cheap to clone and safe to hold: a reload
    /// replaces the `Arc` rather than mutating it, so a reader never sees a
    /// half-applied change.
    pub fn snapshot(&self) -> Arc<Resolved> {
        self.state.read().unwrap().resolved.clone()
    }

    /// Register a listener called after every change. The kernel uses this to
    /// publish onto the streaming channel.
    pub fn add_listener(&self, listener: ChangeListener) {
        self.listeners.write().unwrap().push(listener);
    }

    /// Effective state of one setting.
    ///
    /// Returns `None` only for a key that is neither declared nor set
    /// anywhere, which is a caller asking about something that does not exist.
    pub fn get(&self, key: &str, language: Option<&str>) -> Option<SettingValue> {
        let state = self.state.read().unwrap();
        let value = state.resolved.get(key, language).cloned();
        let schema = self.schema.get(key);
        let value = match (value, schema) {
            (Some(value), _) => value,
            (None, Some(schema)) => schema.default.clone(),
            (None, None) => return None,
        };

        let scope = Self::effective_scope(&state, key, language);
        Some(SettingValue {
            key: key.to_string(),
            language: language.map(str::to_string),
            value,
            default: schema.map(|s| s.default.clone()).unwrap_or(Value::Null),
            scope,
            is_default: scope == ConfigScope::Default,
            known: schema.is_some(),
            requires_restart: self.schema.requires_restart(key),
            schema: schema.cloned(),
        })
    }

    /// The effective value of a key, or `Value::Null` when it is unset and
    /// undeclared. For consumers that want a value and not a report.
    pub fn value(&self, key: &str, language: Option<&str>) -> Value {
        self.get(key, language)
            .map(|setting| setting.value)
            .unwrap_or(Value::Null)
    }

    pub fn bool_value(&self, key: &str) -> Option<bool> {
        self.value(key, None).as_bool()
    }

    pub fn integer_value(&self, key: &str) -> Option<i64> {
        self.value(key, None).as_i64()
    }

    pub fn string_value(&self, key: &str) -> Option<String> {
        self.value(key, None)
            .as_str()
            .map(std::string::ToString::to_string)
    }

    /// Every setting, declared or merely present, optionally filtered by
    /// dotted-key prefix.
    ///
    /// Both kinds are listed together because the settings editor has to show
    /// a plugin's key alongside a built-in one, and an editor that can only
    /// display what the kernel was compiled knowing about would hide exactly
    /// the settings a user is most likely to be confused by.
    pub fn list(&self, prefix: Option<&str>, language: Option<&str>) -> Vec<SettingValue> {
        let state = self.state.read().unwrap();
        let mut keys: BTreeSet<String> = self.schema.iter().map(|s| s.key.clone()).collect();
        keys.extend(flatten_leaves(&state.resolved.global).into_keys());
        if let Some(language) = language
            && let Some(tree) = state.resolved.languages.get(language)
        {
            keys.extend(flatten_leaves(tree).into_keys());
        }
        drop(state);

        keys.into_iter()
            .filter(|key| match prefix {
                Some(prefix) => key.starts_with(prefix),
                None => true,
            })
            .filter_map(|key| self.get(&key, language))
            .collect()
    }

    /// Write a value into one layer (REQ-CONFIG-001).
    ///
    /// Rejected, with nothing written, when: the layer is read-only or has no
    /// file configured, the value fails schema validation, the setting may not
    /// be set from that layer, a language override is asked for on a setting
    /// that has no per-language meaning, or the value looks like a credential
    /// (REQ-CONFIG-001.12).
    ///
    /// The file is reserialized from the parsed document, so comments in it do
    /// not survive a programmatic write. Only the named layer is touched.
    pub fn set(
        &self,
        scope: ConfigScope,
        key: &str,
        value: Value,
        language: Option<&str>,
    ) -> Result<ConfigChange, AppError> {
        let path = self.writable_path(scope)?;
        self.check_writable(scope, key, &value, language)?;

        let mut document = self.layer_document(scope);
        document.raw_set(key, value, language);
        self.write_layer(scope, &path, &document)?;

        Ok(self.reload(scope, ChangeOrigin::Internal))
    }

    /// Remove a key from one layer, so the next layer down decides it again
    /// (REQ-CONFIG-001.9).
    ///
    /// Removing a key that is not there is not an error: "make sure this layer
    /// does not set this" is a reasonable request whether or not it already
    /// held.
    pub fn reset(
        &self,
        scope: ConfigScope,
        key: &str,
        language: Option<&str>,
    ) -> Result<ConfigChange, AppError> {
        let path = self.writable_path(scope)?;
        let mut document = self.layer_document(scope);
        if !document.raw_remove(key, language) {
            return Ok(ConfigChange {
                scope,
                origin: ChangeOrigin::Internal,
                changed_keys: Vec::new(),
                requires_restart: Vec::new(),
                parse_error: self.parse_error_of(scope),
                issues: Vec::new(),
            });
        }
        self.write_layer(scope, &path, &document)?;
        Ok(self.reload(scope, ChangeOrigin::Internal))
    }

    /// Re-read every layer whose file has changed on disk, returning one
    /// change per changed layer (REQ-CONFIG-001.8: changes apply immediately).
    ///
    /// A write this process made is not reported again: the write updated the
    /// fingerprint, so the content hash matches and the layer is skipped.
    pub fn poll_external_changes(&self) -> Vec<ConfigChange> {
        let mut changes = Vec::new();
        for scope in ConfigScope::ASCENDING {
            if !scope.is_writable() {
                continue;
            }
            let Some(path) = self.paths.path(scope) else {
                continue;
            };
            let current = read_body(path).map(|body| Fingerprint::of(&body));
            let previous = self
                .state
                .read()
                .unwrap()
                .layers
                .get(&scope)
                .and_then(|layer| layer.fingerprint);
            if current == previous {
                continue;
            }
            let change = self.reload(scope, ChangeOrigin::External);
            if change.is_meaningful() {
                changes.push(change);
            }
        }
        changes
    }

    /// How often external edits should be checked for, from
    /// `config.watchIntervalMs`. Read through the service itself, so the
    /// setting that controls watching is itself watched.
    pub fn watch_interval_ms(&self) -> u64 {
        self.integer_value("config.watchIntervalMs")
            .filter(|ms| *ms > 0)
            .unwrap_or(400) as u64
    }

    /// Parse errors currently in effect, one per unparseable layer
    /// (REQ-CONFIG-001 failure modes).
    pub fn parse_errors(&self) -> Vec<ConfigParseError> {
        self.state
            .read()
            .unwrap()
            .layers
            .values()
            .filter_map(|layer| layer.parse_error.clone())
            .collect()
    }

    /// Per-key problems across every layer.
    pub fn issues(&self) -> Vec<SettingIssue> {
        self.state
            .read()
            .unwrap()
            .layers
            .values()
            .flat_map(|layer| layer.issues.clone())
            .collect()
    }

    pub fn metrics(&self) -> ConfigMetrics {
        ConfigMetrics {
            reloads: self.counters.reloads.load(Ordering::Relaxed),
            writes: self.counters.writes.load(Ordering::Relaxed),
            write_errors: self.counters.write_errors.load(Ordering::Relaxed),
            parse_errors: self.counters.parse_errors.load(Ordering::Relaxed),
            secrets_rejected: self.counters.secrets_rejected.load(Ordering::Relaxed),
        }
    }

    // ---- internals ------------------------------------------------------

    fn writable_path(&self, scope: ConfigScope) -> Result<PathBuf, AppError> {
        if !scope.is_writable() {
            return Err(AppError::permanent(
                "CONFIG_SCOPE_READ_ONLY",
                format!("the {scope} layer is compiled in and cannot be written to"),
            ));
        }
        self.paths
            .path(scope)
            .map(Path::to_path_buf)
            .ok_or_else(|| {
                AppError::permanent(
                    "CONFIG_SCOPE_UNAVAILABLE",
                    format!(
                        "no {scope} settings file is configured; open a workspace before writing {scope} settings"
                    ),
                )
            })
    }

    fn check_writable(
        &self,
        scope: ConfigScope,
        key: &str,
        value: &Value,
        language: Option<&str>,
    ) -> Result<(), AppError> {
        let findings = secrets::scan(key, value);
        if let Some(finding) = findings.first() {
            self.counters
                .secrets_rejected
                .fetch_add(1, Ordering::Relaxed);
            // Logged at warn with the key only; the value never leaves this
            // call (REQ-SEC-002.4).
            log_warn!(
                self.logger,
                LOG_SOURCE,
                "refused to write a credential into a settings file",
                "key" => qualified_key(key, language),
                "scope" => scope.as_str(),
            );
            return Err(
                AppError::permanent("CONFIG_SECRET_REJECTED", finding.guidance()).with_details(
                    serde_json::json!({
                        "key": qualified_key(key, language),
                        "scope": scope,
                    }),
                ),
            );
        }

        if let Some(schema) = self.schema.get(key) {
            if !schema.writable_in(scope) {
                let allowed: Vec<&str> =
                    schema.writable_scopes.iter().map(|s| s.as_str()).collect();
                return Err(AppError::permanent(
                    "CONFIG_SCOPE_NOT_ALLOWED",
                    format!(
                        "'{key}' may only be set in the {} layer(s)",
                        allowed.join(", ")
                    ),
                ));
            }
            if language.is_some() && !schema.language_overridable {
                return Err(AppError::permanent(
                    "CONFIG_NOT_LANGUAGE_OVERRIDABLE",
                    format!(
                        "'{key}' has no per-language meaning and cannot be overridden per language"
                    ),
                ));
            }
            if let Err((kind, message)) = self.schema.validate(key, value) {
                return Err(AppError::permanent(
                    match kind {
                        IssueKind::TypeMismatch => "CONFIG_TYPE_MISMATCH",
                        IssueKind::NotAllowed => "CONFIG_VALUE_NOT_ALLOWED",
                        IssueKind::OutOfRange => "CONFIG_VALUE_OUT_OF_RANGE",
                        _ => "CONFIG_INVALID_VALUE",
                    },
                    message,
                ));
            }
        }

        Ok(())
    }

    fn layer_document(&self, scope: ConfigScope) -> LayerDocument {
        self.state
            .read()
            .unwrap()
            .layers
            .get(&scope)
            .map(|layer| layer.document.clone())
            .unwrap_or_else(LayerDocument::empty)
    }

    fn parse_error_of(&self, scope: ConfigScope) -> Option<ConfigParseError> {
        self.state
            .read()
            .unwrap()
            .layers
            .get(&scope)
            .and_then(|layer| layer.parse_error.clone())
    }

    /// Write a layer's document atomically: temp file, flush, fsync, rename.
    /// A crash mid-write therefore leaves the previous settings intact rather
    /// than a truncated file that fails to parse on next launch.
    fn write_layer(
        &self,
        scope: ConfigScope,
        path: &Path,
        document: &LayerDocument,
    ) -> Result<(), AppError> {
        let body = document.to_pretty_json();
        match atomic_write(path, &body) {
            Ok(()) => {
                self.counters.writes.fetch_add(1, Ordering::Relaxed);
                log_debug!(
                    self.logger,
                    LOG_SOURCE,
                    "settings file written",
                    "scope" => scope.as_str(),
                    "path" => path.display().to_string(),
                );
                Ok(())
            }
            Err(error) => {
                self.counters.write_errors.fetch_add(1, Ordering::Relaxed);
                log_error!(
                    self.logger,
                    LOG_SOURCE,
                    "settings file could not be written",
                    "scope" => scope.as_str(),
                    "path" => path.display().to_string(),
                    "error" => error.to_string(),
                );
                Err(AppError::transient(
                    "CONFIG_WRITE_FAILED",
                    format!("could not write {}: {error}", path.display()),
                ))
            }
        }
    }

    /// Read one layer from disk, validating and filtering as it goes.
    fn read_layer(&self, scope: ConfigScope) -> LayerState {
        let Some(path) = self.paths.path(scope) else {
            return LayerState::empty();
        };
        let display = path.display().to_string();

        let body = match fs::read_to_string(path) {
            Ok(body) => body,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // Not yet created. Normal on a first launch, and normal for a
                // workspace that has no settings of its own.
                return LayerState::empty();
            }
            Err(error) => {
                log_warn!(
                    self.logger,
                    LOG_SOURCE,
                    "settings file could not be read; continuing without this layer",
                    "scope" => scope.as_str(),
                    "path" => display.clone(),
                    "error" => error.to_string(),
                );
                return LayerState {
                    parse_error: Some(ConfigParseError::new(
                        display,
                        format!("could not be read: {error}"),
                        1,
                        1,
                    )),
                    ..LayerState::empty()
                };
            }
        };

        let fingerprint = Some(Fingerprint::of(&body));
        let raw = match parse_object(&display, &body) {
            Ok(raw) => raw,
            Err(parse_error) => {
                self.counters.parse_errors.fetch_add(1, Ordering::Relaxed);
                log_error!(
                    self.logger,
                    LOG_SOURCE,
                    "settings file is not valid JSON; the last values that parsed remain in effect",
                    "scope" => scope.as_str(),
                    "path" => display,
                    "line" => parse_error.line,
                    "column" => parse_error.column,
                    "error" => parse_error.message.clone(),
                );
                // Last-known-good: keep the previous document, report the
                // error, and update the fingerprint so the same broken file is
                // not re-reported on every poll.
                let previous = self
                    .state
                    .read()
                    .unwrap()
                    .layers
                    .get(&scope)
                    .cloned()
                    .unwrap_or_else(LayerState::empty);
                return LayerState {
                    document: previous.document,
                    fingerprint,
                    parse_error: Some(parse_error),
                    issues: previous.issues,
                };
            }
        };

        let mut issues = Vec::new();
        let document = LayerDocument::from_raw(raw, |key, language, value| {
            self.decide(scope, key, language, value, &mut issues)
        });

        for issue in &issues {
            match issue.kind {
                IssueKind::UnknownKey => log_debug!(
                    self.logger,
                    LOG_SOURCE,
                    "unknown setting preserved",
                    "scope" => scope.as_str(),
                    "key" => issue.key.clone(),
                ),
                _ => log_warn!(
                    self.logger,
                    LOG_SOURCE,
                    "setting ignored",
                    "scope" => scope.as_str(),
                    "key" => issue.key.clone(),
                    "reason" => issue.message.clone(),
                ),
            }
        }

        LayerState {
            document,
            fingerprint,
            parse_error: None,
            issues,
        }
    }

    /// Decide the fate of one node of an authored layer.
    fn decide(
        &self,
        scope: ConfigScope,
        key: &str,
        language: Option<&str>,
        value: &Value,
        issues: &mut Vec<SettingIssue>,
    ) -> LeafDecision {
        let schema = self.schema.get(key);
        let is_container = matches!(value, Value::Object(map) if !map.is_empty());

        // An unrecognized container is a path on the way to a leaf
        // (`editor` → `editor.fontSize`), not a value in its own right.
        if schema.is_none() && is_container {
            return LeafDecision::Descend;
        }

        if let Some(finding) = secrets::scan(key, value).first() {
            self.counters
                .secrets_rejected
                .fetch_add(1, Ordering::Relaxed);
            issues.push(
                SettingIssue::new(key, scope, IssueKind::Secret, finding.guidance())
                    .with_language(language.map(str::to_string)),
            );
            return LeafDecision::Reject;
        }

        let Some(schema) = schema else {
            issues.push(
                SettingIssue::new(
                    key,
                    scope,
                    IssueKind::UnknownKey,
                    format!("'{key}' is not a known setting; it is preserved in case a plugin or a newer version defines it"),
                )
                .with_language(language.map(str::to_string)),
            );
            return LeafDecision::Accept;
        };

        if !schema.writable_in(scope) {
            issues.push(
                SettingIssue::new(
                    key,
                    scope,
                    IssueKind::WrongScope,
                    format!("'{key}' cannot be set from the {scope} layer and is ignored here"),
                )
                .with_language(language.map(str::to_string)),
            );
            return LeafDecision::Reject;
        }

        if language.is_some() && !schema.language_overridable {
            issues.push(
                SettingIssue::new(
                    key,
                    scope,
                    IssueKind::WrongScope,
                    format!("'{key}' has no per-language meaning; the language-specific value is ignored"),
                )
                .with_language(language.map(str::to_string)),
            );
            return LeafDecision::Reject;
        }

        match self.schema.validate(key, value) {
            Ok(()) => LeafDecision::Accept,
            Err((kind, message)) => {
                issues.push(
                    SettingIssue::new(key, scope, kind, message)
                        .with_language(language.map(str::to_string)),
                );
                LeafDecision::Reject
            }
        }
    }

    /// Re-read one layer, recompute the merge, and notify listeners.
    fn reload(&self, scope: ConfigScope, origin: ChangeOrigin) -> ConfigChange {
        let before = self.snapshot();
        let layer = self.read_layer(scope);
        let parse_error = layer.parse_error.clone();
        let issues = layer.issues.clone();
        self.state.write().unwrap().layers.insert(scope, layer);
        self.counters.reloads.fetch_add(1, Ordering::Relaxed);
        let after = self.recompute();

        let changed = changed_keys(&before.effective, &after.effective);
        let requires_restart = changed
            .iter()
            .filter(|key| self.schema.requires_restart(strip_language_prefix(key)))
            .cloned()
            .collect::<Vec<_>>();

        let change = ConfigChange {
            scope,
            origin,
            changed_keys: changed,
            requires_restart,
            parse_error,
            issues,
        };

        if change.is_meaningful() {
            log_info!(
                self.logger,
                LOG_SOURCE,
                "configuration changed",
                "scope" => change.scope.as_str(),
                "origin" => format!("{:?}", change.origin).to_lowercase(),
                "changed" => change.changed_keys.clone(),
                "requires_restart" => change.requires_restart.clone(),
            );
            for listener in self.listeners.read().unwrap().iter() {
                listener(&change);
            }
        }

        change
    }

    /// Merge every layer, lowest precedence first.
    ///
    /// For a language, each layer contributes its global values and then its
    /// own language section, so a higher layer's global value beats a lower
    /// layer's language-specific one. That is what makes a workspace decision
    /// hold against a personal per-language preference.
    fn recompute(&self) -> Arc<Resolved> {
        let mut state = self.state.write().unwrap();

        let mut global = Value::Object(Map::new());
        let mut overridden: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for scope in ConfigScope::ASCENDING {
            let Some(layer) = state.layers.get(&scope) else {
                continue;
            };
            deep_merge(&mut global, &layer.document.global);
            for (language, section) in &layer.document.languages {
                overridden
                    .entry(language.clone())
                    .or_default()
                    .extend(flatten_leaves(section).into_keys());
            }
        }

        let mut languages = BTreeMap::new();
        for language in overridden.keys() {
            let mut tree = Value::Object(Map::new());
            for scope in ConfigScope::ASCENDING {
                let Some(layer) = state.layers.get(&scope) else {
                    continue;
                };
                deep_merge(&mut tree, &layer.document.global);
                if let Some(section) = layer.document.languages.get(language) {
                    deep_merge(&mut tree, section);
                }
            }
            languages.insert(language.clone(), tree);
        }

        let mut effective = flatten_leaves(&global);
        for (language, keys) in &overridden {
            let Some(tree) = languages.get(language) else {
                continue;
            };
            let leaves = flatten_leaves(tree);
            for key in keys {
                if let Some(value) = leaves.get(key) {
                    effective.insert(qualified_key(key, Some(language)), value.clone());
                }
            }
        }

        let resolved = Arc::new(Resolved {
            global,
            languages,
            effective,
        });
        state.resolved = resolved.clone();
        resolved
    }

    /// The highest-precedence layer that declares a key, defaulting to
    /// `Default` when only the schema provides it.
    fn effective_scope(state: &State, key: &str, language: Option<&str>) -> ConfigScope {
        for scope in ConfigScope::ASCENDING.into_iter().rev() {
            if let Some(layer) = state.layers.get(&scope)
                && layer.document.declares(key, language)
            {
                return scope;
            }
        }
        ConfigScope::Default
    }
}

/// Strip a `[language].` prefix from a qualified key.
fn strip_language_prefix(key: &str) -> &str {
    crate::layer::split_language_key(key)
        .and_then(|(_, rest)| rest)
        .unwrap_or(key)
}

fn read_body(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

/// Write `body` to `path` without ever leaving a partial file in place.
///
/// Task 1.7 owns the general atomic-write service; this is the same
/// temp-fsync-rename sequence, kept local so the configuration service does
/// not have to wait for a dependency scheduled after it.
fn atomic_write(path: &Path, body: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let temp = path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));

    {
        let mut file = fs::File::create(&temp)?;
        file.write_all(body.as_bytes())?;
        file.flush()?;
        file.sync_all()?;
    }

    match fs::rename(&temp, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&temp);
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_log::LogLevel;
    use serde_json::json;
    use std::sync::atomic::AtomicU32;

    /// A unique temporary directory per test, removed on drop. Same approach
    /// as `helix-log`'s tests: four call sites is not worth a dependency.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let path = std::env::temp_dir().join(format!(
                "helix-config-{label}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::SeqCst)
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn write_settings(&self, name: &str, body: &str) -> PathBuf {
            let path = self.0.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, body).unwrap();
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn logger() -> Arc<Logger> {
        Arc::new(Logger::in_memory(LogLevel::Trace))
    }

    fn service(paths: ConfigPaths) -> ConfigService {
        ConfigService::load(paths, Arc::new(SchemaRegistry::builtin()), logger())
    }

    #[test]
    fn defaults_answer_every_declared_setting_with_no_files_present() {
        let config = service(ConfigPaths::default());
        let font = config.get("editor.fontSize", None).unwrap();
        assert_eq!(font.value, json!(14));
        assert_eq!(font.scope, ConfigScope::Default);
        assert!(font.is_default);
        assert!(font.known);
        assert!(config.parse_errors().is_empty());
    }

    #[test]
    fn an_unknown_and_unset_key_reports_nothing_rather_than_a_null() {
        let config = service(ConfigPaths::default());
        assert!(config.get("nothing.like.this", None).is_none());
    }

    #[test]
    fn a_workspace_value_overrides_a_user_value_which_overrides_the_default() {
        let dir = TempDir::new("precedence");
        let user = dir.write_settings("user.json", r#"{ "editor.fontSize": 15 }"#);
        let workspace =
            dir.write_settings("ws/.helix/settings.json", r#"{ "editor.fontSize": 16 }"#);

        let config = service(ConfigPaths {
            user: Some(user),
            workspace: Some(workspace),
            folder: None,
        });

        let font = config.get("editor.fontSize", None).unwrap();
        assert_eq!(font.value, json!(16));
        assert_eq!(font.scope, ConfigScope::Workspace);
        assert!(!font.is_default);
        // The user value still decides anything the workspace is silent about.
        assert_eq!(config.get("editor.tabSize", None).unwrap().value, json!(4));
    }

    #[test]
    fn a_folder_value_beats_a_workspace_value() {
        let dir = TempDir::new("folder");
        let workspace = dir.write_settings("ws/.helix/settings.json", r#"{ "editor.tabSize": 4 }"#);
        let folder =
            dir.write_settings("ws/api/.helix/settings.json", r#"{ "editor.tabSize": 8 }"#);

        let config = service(ConfigPaths {
            user: None,
            workspace: Some(workspace),
            folder: Some(folder),
        });

        let tab = config.get("editor.tabSize", None).unwrap();
        assert_eq!(tab.value, json!(8));
        assert_eq!(tab.scope, ConfigScope::Folder);
    }

    #[test]
    fn objects_deep_merge_across_layers_and_arrays_replace() {
        let dir = TempDir::new("shapes");
        let user = dir.write_settings(
            "user.json",
            r#"{ "files.exclude": { "**/dist": true }, "editor.rulers": [80, 120] }"#,
        );
        let workspace = dir.write_settings(
            "ws/.helix/settings.json",
            r#"{ "files.exclude": { "**/coverage": true }, "editor.rulers": [100] }"#,
        );

        let config = service(ConfigPaths {
            user: Some(user),
            workspace: Some(workspace),
            folder: None,
        });

        let exclude = config.get("files.exclude", None).unwrap().value;
        assert_eq!(exclude["**/dist"], true, "the user entry survives");
        assert_eq!(exclude["**/coverage"], true, "the workspace entry is added");
        assert_eq!(exclude["**/.git"], true, "the default entries survive");

        assert_eq!(
            config.get("editor.rulers", None).unwrap().value,
            json!([100]),
            "an array is replaced, not concatenated"
        );
    }

    #[test]
    fn a_language_override_applies_only_to_that_language() {
        let dir = TempDir::new("language");
        let user = dir.write_settings(
            "user.json",
            r#"{ "editor.tabSize": 4, "[typescript].editor.tabSize": 2 }"#,
        );
        let config = service(ConfigPaths {
            user: Some(user),
            ..ConfigPaths::default()
        });

        assert_eq!(
            config
                .get("editor.tabSize", Some("typescript"))
                .unwrap()
                .value,
            json!(2)
        );
        assert_eq!(
            config.get("editor.tabSize", Some("rust")).unwrap().value,
            json!(4)
        );
        assert_eq!(config.get("editor.tabSize", None).unwrap().value, json!(4));
    }

    #[test]
    fn a_higher_layers_global_value_beats_a_lower_layers_language_override() {
        let dir = TempDir::new("language-precedence");
        let user = dir.write_settings("user.json", r#"{ "[typescript].editor.tabSize": 2 }"#);
        let workspace = dir.write_settings("ws/.helix/settings.json", r#"{ "editor.tabSize": 8 }"#);
        let config = service(ConfigPaths {
            user: Some(user),
            workspace: Some(workspace),
            folder: None,
        });

        assert_eq!(
            config
                .get("editor.tabSize", Some("typescript"))
                .unwrap()
                .value,
            json!(8),
            "the workspace's project-wide decision must hold over a personal per-language preference"
        );
    }

    #[test]
    fn an_invalid_json_layer_keeps_the_last_values_that_parsed_and_reports_where() {
        let dir = TempDir::new("invalid");
        let user = dir.write_settings("user.json", r#"{ "editor.fontSize": 22 }"#);
        let config = service(ConfigPaths {
            user: Some(user.clone()),
            ..ConfigPaths::default()
        });
        assert_eq!(
            config.get("editor.fontSize", None).unwrap().value,
            json!(22)
        );

        fs::write(
            &user,
            "{\n  \"editor.fontSize\": 22\n  \"editor.tabSize\": 2\n}",
        )
        .unwrap();
        let changes = config.poll_external_changes();

        assert_eq!(changes.len(), 1);
        let error = changes[0].parse_error.clone().expect("a parse error");
        assert_eq!(error.line, 3);
        assert!(changes[0].changed_keys.is_empty(), "values must not move");
        assert_eq!(
            config.get("editor.fontSize", None).unwrap().value,
            json!(22),
            "the last-known-good value stays in effect while the file is broken"
        );

        // And recovery restores normal service.
        fs::write(&user, r#"{ "editor.fontSize": 30 }"#).unwrap();
        let recovered = config.poll_external_changes();
        assert_eq!(recovered.len(), 1);
        assert!(recovered[0].parse_error.is_none());
        assert_eq!(
            config.get("editor.fontSize", None).unwrap().value,
            json!(30)
        );
        assert!(config.parse_errors().is_empty());
    }

    #[test]
    fn a_type_mismatch_falls_back_to_the_default_and_leaves_the_rest_of_the_layer_alone() {
        let dir = TempDir::new("mismatch");
        let user = dir.write_settings(
            "user.json",
            r#"{ "editor.fontSize": "enormous", "editor.tabSize": 2 }"#,
        );
        let config = service(ConfigPaths {
            user: Some(user),
            ..ConfigPaths::default()
        });

        assert_eq!(
            config.get("editor.fontSize", None).unwrap().value,
            json!(14)
        );
        assert_eq!(
            config.get("editor.tabSize", None).unwrap().value,
            json!(2),
            "one bad key must not discard the whole layer"
        );
        let issue = config
            .issues()
            .into_iter()
            .find(|i| i.key == "editor.fontSize")
            .unwrap();
        assert_eq!(issue.kind, IssueKind::TypeMismatch);
    }

    #[test]
    fn an_unknown_key_is_preserved_and_reported_rather_than_dropped() {
        let dir = TempDir::new("unknown");
        let user = dir.write_settings("user.json", r#"{ "somePlugin.enabled": true }"#);
        let config = service(ConfigPaths {
            user: Some(user),
            ..ConfigPaths::default()
        });

        let setting = config.get("somePlugin.enabled", None).unwrap();
        assert_eq!(setting.value, json!(true));
        assert!(!setting.known);
        assert_eq!(setting.scope, ConfigScope::User);
        let issue = config
            .issues()
            .into_iter()
            .find(|i| i.key == "somePlugin.enabled")
            .unwrap();
        assert_eq!(issue.kind, IssueKind::UnknownKey);
        assert!(!issue.discards_value());
    }

    #[test]
    fn a_credential_in_a_settings_file_is_ignored_and_reported() {
        let dir = TempDir::new("secret-load");
        let user = dir.write_settings(
            "user.json",
            r#"{ "ai.apiKey": "sk-abcdefghijklmnopqrst", "editor.tabSize": 2 }"#,
        );
        let config = service(ConfigPaths {
            user: Some(user),
            ..ConfigPaths::default()
        });

        assert!(config.get("ai.apiKey", None).is_none());
        assert_eq!(config.get("editor.tabSize", None).unwrap().value, json!(2));
        let issue = config
            .issues()
            .into_iter()
            .find(|i| i.kind == IssueKind::Secret)
            .expect("the credential must be reported");
        assert!(issue.message.contains("keychain"));
        assert_eq!(config.metrics().secrets_rejected, 1);
    }

    #[test]
    fn setting_a_value_writes_it_and_reports_the_changed_key() {
        let dir = TempDir::new("set");
        let config = service(ConfigPaths {
            user: Some(dir.path().join("user.json")),
            ..ConfigPaths::default()
        });

        let change = config
            .set(ConfigScope::User, "editor.fontSize", json!(18), None)
            .unwrap();
        assert_eq!(change.changed_keys, vec!["editor.fontSize"]);
        assert_eq!(change.origin, ChangeOrigin::Internal);
        assert!(change.requires_restart.is_empty());
        assert_eq!(
            config.get("editor.fontSize", None).unwrap().value,
            json!(18)
        );

        let on_disk = fs::read_to_string(dir.path().join("user.json")).unwrap();
        assert!(on_disk.contains("\"editor.fontSize\": 18"), "{on_disk}");
    }

    #[test]
    fn setting_a_language_override_writes_the_section() {
        let dir = TempDir::new("set-language");
        let config = service(ConfigPaths {
            user: Some(dir.path().join("user.json")),
            ..ConfigPaths::default()
        });

        let change = config
            .set(
                ConfigScope::User,
                "editor.tabSize",
                json!(2),
                Some("typescript"),
            )
            .unwrap();
        assert_eq!(change.changed_keys, vec!["[typescript].editor.tabSize"]);
        assert_eq!(
            config
                .get("editor.tabSize", Some("typescript"))
                .unwrap()
                .value,
            json!(2)
        );
        assert_eq!(config.get("editor.tabSize", None).unwrap().value, json!(4));
    }

    #[test]
    fn a_restart_only_setting_is_flagged_when_it_changes() {
        let dir = TempDir::new("restart");
        let config = service(ConfigPaths {
            user: Some(dir.path().join("user.json")),
            ..ConfigPaths::default()
        });

        let change = config
            .set(ConfigScope::User, "stream.bufferDepth", json!(5000), None)
            .unwrap();
        assert_eq!(change.changed_keys, vec!["stream.bufferDepth"]);
        assert_eq!(change.requires_restart, vec!["stream.bufferDepth"]);
        assert!(
            config
                .get("stream.bufferDepth", None)
                .unwrap()
                .requires_restart
        );
    }

    #[test]
    fn resetting_a_key_hands_it_back_to_the_layer_below() {
        let dir = TempDir::new("reset");
        let user = dir.write_settings("user.json", r#"{ "editor.fontSize": 20 }"#);
        let config = service(ConfigPaths {
            user: Some(user.clone()),
            ..ConfigPaths::default()
        });
        assert_eq!(
            config.get("editor.fontSize", None).unwrap().value,
            json!(20)
        );

        let change = config
            .reset(ConfigScope::User, "editor.fontSize", None)
            .unwrap();
        assert_eq!(change.changed_keys, vec!["editor.fontSize"]);
        let font = config.get("editor.fontSize", None).unwrap();
        assert_eq!(font.value, json!(14));
        assert_eq!(font.scope, ConfigScope::Default);
        assert_eq!(fs::read_to_string(&user).unwrap().trim(), "{}");
    }

    #[test]
    fn resetting_a_key_that_was_never_set_is_not_an_error() {
        let dir = TempDir::new("reset-absent");
        let config = service(ConfigPaths {
            user: Some(dir.path().join("user.json")),
            ..ConfigPaths::default()
        });
        let change = config
            .reset(ConfigScope::User, "editor.fontSize", None)
            .unwrap();
        assert!(change.changed_keys.is_empty());
    }

    #[test]
    fn writing_a_credential_is_refused_and_the_file_is_left_alone() {
        let dir = TempDir::new("secret-write");
        let path = dir.path().join("user.json");
        let config = service(ConfigPaths {
            user: Some(path.clone()),
            ..ConfigPaths::default()
        });

        let error = config
            .set(
                ConfigScope::User,
                "ai.providers",
                json!({ "openai": { "token": "abcdef1234567890" } }),
                None,
            )
            .unwrap_err();
        assert_eq!(error.code, "CONFIG_SECRET_REJECTED");
        assert!(!error.message.contains("abcdef1234567890"));
        assert!(!path.exists(), "a rejected write must not create the file");
        assert_eq!(config.metrics().secrets_rejected, 1);
    }

    #[test]
    fn writing_an_invalid_value_is_refused_with_the_reason() {
        let dir = TempDir::new("invalid-write");
        let config = service(ConfigPaths {
            user: Some(dir.path().join("user.json")),
            ..ConfigPaths::default()
        });

        assert_eq!(
            config
                .set(ConfigScope::User, "editor.fontSize", json!("big"), None)
                .unwrap_err()
                .code,
            "CONFIG_TYPE_MISMATCH"
        );
        assert_eq!(
            config
                .set(ConfigScope::User, "files.autoSave", json!("maybe"), None)
                .unwrap_err()
                .code,
            "CONFIG_VALUE_NOT_ALLOWED"
        );
        assert_eq!(
            config
                .set(ConfigScope::User, "editor.fontSize", json!(500), None)
                .unwrap_err()
                .code,
            "CONFIG_VALUE_OUT_OF_RANGE"
        );
    }

    #[test]
    fn the_defaults_layer_refuses_to_be_written_to() {
        let config = service(ConfigPaths::default());
        assert_eq!(
            config
                .set(ConfigScope::Default, "editor.fontSize", json!(20), None)
                .unwrap_err()
                .code,
            "CONFIG_SCOPE_READ_ONLY"
        );
    }

    #[test]
    fn a_layer_with_no_file_configured_reports_that_rather_than_guessing_a_path() {
        let config = service(ConfigPaths::default());
        assert_eq!(
            config
                .set(ConfigScope::Workspace, "editor.fontSize", json!(20), None)
                .unwrap_err()
                .code,
            "CONFIG_SCOPE_UNAVAILABLE"
        );
    }

    #[test]
    fn a_machine_scoped_setting_is_refused_from_a_workspace_layer() {
        let dir = TempDir::new("scope");
        let config = service(ConfigPaths {
            workspace: Some(dir.path().join("ws/.helix/settings.json")),
            ..ConfigPaths::default()
        });
        let error = config
            .set(
                ConfigScope::Workspace,
                "terminal.shellPath",
                json!("/bin/sh"),
                None,
            )
            .unwrap_err();
        assert_eq!(error.code, "CONFIG_SCOPE_NOT_ALLOWED");
    }

    #[test]
    fn a_workspace_file_naming_an_executable_is_ignored_with_a_reason() {
        let dir = TempDir::new("scope-load");
        let workspace = dir.write_settings(
            "ws/.helix/settings.json",
            r#"{ "terminal.shellPath": "/tmp/evil" }"#,
        );
        let config = service(ConfigPaths {
            workspace: Some(workspace),
            ..ConfigPaths::default()
        });

        assert_eq!(
            config.get("terminal.shellPath", None).unwrap().value,
            json!("")
        );
        let issue = config
            .issues()
            .into_iter()
            .find(|i| i.key == "terminal.shellPath")
            .unwrap();
        assert_eq!(issue.kind, IssueKind::WrongScope);
    }

    #[test]
    fn a_language_override_of_a_global_only_setting_is_ignored() {
        let dir = TempDir::new("not-overridable");
        let user = dir.write_settings(
            "user.json",
            r#"{ "[typescript].workbench.colorTheme": "Helix Light" }"#,
        );
        let config = service(ConfigPaths {
            user: Some(user),
            ..ConfigPaths::default()
        });

        assert_eq!(
            config
                .get("workbench.colorTheme", Some("typescript"))
                .unwrap()
                .value,
            json!("Helix Dark")
        );
        assert!(
            config
                .issues()
                .iter()
                .any(|i| i.kind == IssueKind::WrongScope)
        );
    }

    #[test]
    fn an_external_edit_is_detected_and_reported_with_the_changed_keys() {
        let dir = TempDir::new("external");
        let user = dir.write_settings("user.json", r#"{ "editor.fontSize": 14 }"#);
        let config = service(ConfigPaths {
            user: Some(user.clone()),
            ..ConfigPaths::default()
        });
        assert!(
            config.poll_external_changes().is_empty(),
            "nothing changed yet"
        );

        // Same length as the previous value, which is exactly the case a
        // stat-only check would miss.
        fs::write(&user, r#"{ "editor.fontSize": 16 }"#).unwrap();
        let changes = config.poll_external_changes();

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].origin, ChangeOrigin::External);
        assert_eq!(changes[0].changed_keys, vec!["editor.fontSize"]);
        assert_eq!(
            config.get("editor.fontSize", None).unwrap().value,
            json!(16)
        );
    }

    #[test]
    fn a_write_this_process_made_is_not_re_reported_as_an_external_edit() {
        let dir = TempDir::new("no-echo");
        let config = service(ConfigPaths {
            user: Some(dir.path().join("user.json")),
            ..ConfigPaths::default()
        });
        config
            .set(ConfigScope::User, "editor.fontSize", json!(19), None)
            .unwrap();
        assert!(config.poll_external_changes().is_empty());
    }

    #[test]
    fn deleting_a_settings_file_falls_back_to_the_layers_below() {
        let dir = TempDir::new("deleted");
        let user = dir.write_settings("user.json", r#"{ "editor.fontSize": 21 }"#);
        let config = service(ConfigPaths {
            user: Some(user.clone()),
            ..ConfigPaths::default()
        });
        fs::remove_file(&user).unwrap();

        let changes = config.poll_external_changes();
        assert_eq!(changes.len(), 1);
        assert_eq!(
            config.get("editor.fontSize", None).unwrap().value,
            json!(14)
        );
    }

    #[test]
    fn listeners_are_notified_of_every_meaningful_change() {
        let dir = TempDir::new("listener");
        let config = service(ConfigPaths {
            user: Some(dir.path().join("user.json")),
            ..ConfigPaths::default()
        });
        let seen: Arc<RwLock<Vec<ConfigChange>>> = Arc::new(RwLock::new(Vec::new()));
        {
            let seen = seen.clone();
            config.add_listener(Arc::new(move |change: &ConfigChange| {
                seen.write().unwrap().push(change.clone());
            }));
        }

        config
            .set(ConfigScope::User, "editor.fontSize", json!(17), None)
            .unwrap();
        config
            .reset(ConfigScope::User, "editor.fontSize", None)
            .unwrap();

        let seen = seen.read().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].changed_keys, vec!["editor.fontSize"]);
        assert_eq!(seen[1].changed_keys, vec!["editor.fontSize"]);
    }

    #[test]
    fn a_no_op_write_notifies_nobody() {
        let dir = TempDir::new("noop");
        let config = service(ConfigPaths {
            user: Some(dir.path().join("user.json")),
            ..ConfigPaths::default()
        });
        let count = Arc::new(AtomicU64::new(0));
        {
            let count = count.clone();
            config.add_listener(Arc::new(move |_| {
                count.fetch_add(1, Ordering::SeqCst);
            }));
        }

        config
            .set(ConfigScope::User, "editor.fontSize", json!(14), None)
            .unwrap();
        assert_eq!(
            count.load(Ordering::SeqCst),
            0,
            "writing the value something already had is not a change"
        );
    }

    #[test]
    fn listing_covers_declared_and_contributed_settings_and_filters_by_prefix() {
        let dir = TempDir::new("list");
        let user = dir.write_settings(
            "user.json",
            r#"{ "editor.fontSize": 15, "somePlugin.enabled": true }"#,
        );
        let config = service(ConfigPaths {
            user: Some(user),
            ..ConfigPaths::default()
        });

        let editor = config.list(Some("editor."), None);
        assert!(editor.iter().all(|s| s.key.starts_with("editor.")));
        let font = editor.iter().find(|s| s.key == "editor.fontSize").unwrap();
        assert_eq!(font.value, json!(15));
        assert_eq!(font.scope, ConfigScope::User);
        assert!(font.schema.is_some());

        let all = config.list(None, None);
        assert!(
            all.iter()
                .any(|s| s.key == "somePlugin.enabled" && !s.known)
        );
    }

    #[test]
    fn a_settings_file_with_comments_and_trailing_commas_loads() {
        let dir = TempDir::new("jsonc");
        let user = dir.write_settings(
            "user.json",
            "{\n  // my preference\n  \"editor.fontSize\": 17,\n  /* and this */\n  \"editor.tabSize\": 2,\n}",
        );
        let config = service(ConfigPaths {
            user: Some(user),
            ..ConfigPaths::default()
        });
        assert_eq!(
            config.get("editor.fontSize", None).unwrap().value,
            json!(17)
        );
        assert_eq!(config.get("editor.tabSize", None).unwrap().value, json!(2));
    }

    #[test]
    fn an_unwritable_path_reports_a_transient_error_rather_than_panicking() {
        let dir = TempDir::new("unwritable");
        // A file standing where a directory would have to be created.
        let blocker = dir.path().join("blocked");
        fs::write(&blocker, b"x").unwrap();
        let config = service(ConfigPaths {
            user: Some(blocker.join(".helix").join("settings.json")),
            ..ConfigPaths::default()
        });

        let error = config
            .set(ConfigScope::User, "editor.fontSize", json!(18), None)
            .unwrap_err();
        assert_eq!(error.code, "CONFIG_WRITE_FAILED");
        assert_eq!(config.metrics().write_errors, 1);
    }

    #[test]
    fn the_watch_interval_comes_from_the_configuration_it_governs() {
        let dir = TempDir::new("interval");
        let user = dir.write_settings("user.json", r#"{ "config.watchIntervalMs": 120 }"#);
        let config = service(ConfigPaths {
            user: Some(user),
            ..ConfigPaths::default()
        });
        assert_eq!(config.watch_interval_ms(), 120);
    }

    #[test]
    fn an_atomic_write_replaces_the_previous_contents_without_leaving_debris() {
        let dir = TempDir::new("atomic");
        let path = dir.path().join("nested").join("settings.json");
        atomic_write(&path, "{\"a\":1}\n").unwrap();
        atomic_write(&path, "{\"b\":2}\n").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"b\":2}\n");
        let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.contains("tmp-"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );
    }
}
