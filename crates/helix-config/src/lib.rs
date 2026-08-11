//! `helix-config` — the layered configuration service (Task 1.6,
//! REQ-CONFIG-001).
//!
//! Settings come from four places and have to combine predictably, because the
//! whole point is that a project's conventions win where they should and a
//! developer's preferences apply everywhere else:
//!
//! ```text
//!  folder    <folder>/.helix/settings.json      ── highest precedence
//!  workspace <root>/.helix/settings.json
//!  user      ~/.helix/settings.json
//!  defaults  compiled in, from the schema       ── lowest precedence
//!
//!  within any layer:  "[typescript].editor.tabSize": 2
//! ```
//!
//! Module map:
//!
//! - [`schema`] — every built-in setting, its type, default, and restart flag;
//!   validation and the JSON Schema the editors use.
//! - [`layer`] — the layers, their file locations, and the language-override
//!   syntax.
//! - [`merge`] — the tree primitives: dotted-key expansion, deep merge, diff.
//! - [`jsonc`] — JSON-with-comments parsing that preserves error locations.
//! - [`secrets`] — refusing credentials in settings files.
//! - [`service`] — the service itself: resolution, reads, writes, and change
//!   detection.
//! - [`commands`] — the `config.*` IPC payloads and the streaming channel name.
//!
//! Like `helix-log` and `helix-stream`, this crate has no Tauri dependency and
//! no dependency on the service container. `helix-kernel` wraps it as a managed
//! service, registers its commands, and bridges its changes onto the stream, so
//! tests here drive the real code path with no process around it.

pub mod commands;
pub mod jsonc;
pub mod layer;
pub mod merge;
pub mod schema;
pub mod secrets;
pub mod service;

pub use commands::{
    CHANNEL, ConfigGetRequest, ConfigGetResponse, ConfigListRequest, ConfigListResponse,
    ConfigResetRequest, ConfigSchemaRequest, ConfigSchemaResponse, ConfigScopeInfo,
    ConfigSetRequest, ConfigWriteResponse,
};
pub use jsonc::ConfigParseError;
pub use layer::{
    ConfigPaths, ConfigScope, LayerDocument, LeafDecision, SETTINGS_FILE_NAME,
    WORKSPACE_CONFIG_DIR, settings_path_in, user_settings_path,
};
pub use schema::{IssueKind, SchemaRegistry, SettingIssue, SettingKind, SettingSchema};
pub use secrets::{SecretFinding, SecretSignal};
pub use service::{
    ChangeOrigin, ConfigChange, ConfigMetrics, ConfigService, LOG_SOURCE, Resolved, SettingValue,
};
