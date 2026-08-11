//! Integration tests for the configuration service (Task 1.6).
//!
//! These drive the real service against real files on disk, in the four
//! scenarios the task calls out: merge precedence, hot reload, invalid JSON
//! recovery, and restart-flag propagation. The unit tests beside the source
//! cover the pieces; these cover the behaviour a user would recognize.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, RwLock};

use helix_config::{
    ChangeOrigin, ConfigChange, ConfigPaths, ConfigScope, ConfigService, IssueKind, SchemaRegistry,
};
use helix_log::{LogLevel, LogQuery, Logger};
use serde_json::json;

/// A unique temporary workspace per test, removed on drop.
struct TempWorkspace(PathBuf);

impl TempWorkspace {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "helix-config-it-{label}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    /// Write a settings file at `<temp>/<relative>`, creating parents.
    fn write(&self, relative: &str, body: &str) -> PathBuf {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, body).unwrap();
        path
    }

    fn settings_path(&self, relative: &str) -> PathBuf {
        self.0.join(relative)
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn logger() -> Arc<Logger> {
    Arc::new(Logger::in_memory(LogLevel::Trace))
}

fn service(paths: ConfigPaths, logger: Arc<Logger>) -> ConfigService {
    ConfigService::load(paths, Arc::new(SchemaRegistry::builtin()), logger)
}

/// Collect every change the service reports, in order.
fn recorder(config: &ConfigService) -> Arc<RwLock<Vec<ConfigChange>>> {
    let seen: Arc<RwLock<Vec<ConfigChange>>> = Arc::new(RwLock::new(Vec::new()));
    let sink = seen.clone();
    config.add_listener(Arc::new(move |change: &ConfigChange| {
        sink.write().unwrap().push(change.clone());
    }));
    seen
}

#[test]
fn four_layers_resolve_in_precedence_order_with_language_overrides_on_top() {
    let temp = TempWorkspace::new("precedence");
    let user = temp.write(
        "home/.helix/settings.json",
        r#"{
            // personal preferences
            "editor.fontSize": 15,
            "editor.tabSize": 4,
            "editor.wordWrap": "on",
            "files.exclude": { "**/.cache": true },
            "editor.rulers": [80, 120],
            "[python].editor.tabSize": 4,
        }"#,
    );
    let workspace = temp.write(
        "repo/.helix/settings.json",
        r#"{
            "editor.tabSize": 2,
            "editor.rulers": [100],
            "files.exclude": { "**/generated": true },
            "[typescript]": { "editor.tabSize": 2, "editor.formatOnSave": true }
        }"#,
    );
    let folder = temp.write(
        "repo/services/api/.helix/settings.json",
        r#"{ "editor.tabSize": 8 }"#,
    );

    let config = service(
        ConfigPaths {
            user: Some(user),
            workspace: Some(workspace),
            folder: Some(folder),
        },
        logger(),
    );

    // Folder beats workspace beats user beats default.
    let tab = config.get("editor.tabSize", None).unwrap();
    assert_eq!(tab.value, json!(8));
    assert_eq!(tab.scope, ConfigScope::Folder);

    // A setting only the user sets still applies.
    let wrap = config.get("editor.wordWrap", None).unwrap();
    assert_eq!(wrap.value, json!("on"));
    assert_eq!(wrap.scope, ConfigScope::User);

    // A setting nobody sets comes from the schema.
    let minimap = config.get("editor.minimap.enabled", None).unwrap();
    assert_eq!(minimap.value, json!(true));
    assert!(minimap.is_default);

    // Objects deep-merge across all three files plus the defaults.
    let exclude = config.get("files.exclude", None).unwrap().value;
    assert_eq!(exclude["**/.cache"], true);
    assert_eq!(exclude["**/generated"], true);
    assert_eq!(exclude["**/node_modules"], true);

    // Arrays replace.
    assert_eq!(
        config.get("editor.rulers", None).unwrap().value,
        json!([100])
    );

    // A language override in the workspace applies to that language only, and
    // still loses to the folder layer's global value for the same key.
    assert_eq!(
        config
            .get("editor.formatOnSave", Some("typescript"))
            .unwrap()
            .value,
        json!(true)
    );
    assert_eq!(
        config.get("editor.formatOnSave", None).unwrap().value,
        json!(false)
    );
    assert_eq!(
        config
            .get("editor.tabSize", Some("typescript"))
            .unwrap()
            .value,
        json!(8),
        "the folder's project-wide decision outranks a lower layer's language override"
    );
}

#[test]
fn an_edit_made_outside_the_application_is_picked_up_with_the_changed_key_named() {
    let temp = TempWorkspace::new("hot-reload");
    let user = temp.write("home/.helix/settings.json", r#"{ "editor.fontSize": 14 }"#);
    let config = service(
        ConfigPaths {
            user: Some(user.clone()),
            ..ConfigPaths::default()
        },
        logger(),
    );
    let seen = recorder(&config);

    // An external editor rewrites the file. Same byte length as before, which
    // is the case a modification-time check alone would miss.
    fs::write(&user, r#"{ "editor.fontSize": 16 }"#).unwrap();
    let changes = config.poll_external_changes();

    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].origin, ChangeOrigin::External);
    assert_eq!(changes[0].changed_keys, vec!["editor.fontSize"]);
    assert_eq!(
        config.get("editor.fontSize", None).unwrap().value,
        json!(16)
    );
    assert_eq!(seen.read().unwrap().len(), 1, "listeners see it too");

    // Polling again reports nothing: the layer is up to date.
    assert!(config.poll_external_changes().is_empty());
    assert_eq!(seen.read().unwrap().len(), 1);
}

#[test]
fn a_workspace_appearing_later_takes_precedence_from_that_point_on() {
    // The sequence a user actually experiences: settings resolve on their own
    // preferences, then a workspace with its own conventions is opened.
    let temp = TempWorkspace::new("workspace-added");
    let user = temp.write("home/.helix/settings.json", r#"{ "editor.tabSize": 4 }"#);
    let workspace_path = temp.settings_path("repo/.helix/settings.json");

    let config = service(
        ConfigPaths {
            user: Some(user),
            workspace: Some(workspace_path.clone()),
            folder: None,
        },
        logger(),
    );
    assert_eq!(config.get("editor.tabSize", None).unwrap().value, json!(4));

    // The workspace file did not exist at load time; creating it is an
    // external change like any other.
    fs::create_dir_all(workspace_path.parent().unwrap()).unwrap();
    fs::write(&workspace_path, r#"{ "editor.tabSize": 2 }"#).unwrap();
    let changes = config.poll_external_changes();

    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].scope, ConfigScope::Workspace);
    let tab = config.get("editor.tabSize", None).unwrap();
    assert_eq!(tab.value, json!(2));
    assert_eq!(tab.scope, ConfigScope::Workspace);
}

#[test]
fn an_unparseable_file_keeps_the_last_good_values_and_recovers_when_fixed() {
    let temp = TempWorkspace::new("recovery");
    let user = temp.write(
        "home/.helix/settings.json",
        r#"{ "editor.fontSize": 18, "editor.tabSize": 2 }"#,
    );
    let log = logger();
    let config = service(
        ConfigPaths {
            user: Some(user.clone()),
            ..ConfigPaths::default()
        },
        log.clone(),
    );
    assert_eq!(
        config.get("editor.fontSize", None).unwrap().value,
        json!(18)
    );

    // Mid-edit: a missing comma on line 3.
    fs::write(
        &user,
        "{\n  \"editor.fontSize\": 18\n  \"editor.tabSize\": 2\n}",
    )
    .unwrap();
    let changes = config.poll_external_changes();

    assert_eq!(changes.len(), 1);
    let error = changes[0].parse_error.clone().expect("a parse error");
    assert_eq!(error.line, 3, "the reported line is the line in the file");
    assert!(error.path.ends_with("settings.json"));
    assert!(
        changes[0].changed_keys.is_empty(),
        "a broken file must not move any value"
    );
    assert_eq!(
        config.get("editor.fontSize", None).unwrap().value,
        json!(18),
        "the last values that parsed stay in effect"
    );
    assert_eq!(config.parse_errors().len(), 1);

    // The same broken file is not re-reported on every poll.
    assert!(config.poll_external_changes().is_empty());

    // The user finishes the edit.
    fs::write(
        &user,
        "{\n  \"editor.fontSize\": 18,\n  \"editor.tabSize\": 8\n}",
    )
    .unwrap();
    let recovered = config.poll_external_changes();
    assert_eq!(recovered.len(), 1);
    assert!(recovered[0].parse_error.is_none());
    assert_eq!(recovered[0].changed_keys, vec!["editor.tabSize"]);
    assert!(config.parse_errors().is_empty());
    assert_eq!(config.get("editor.tabSize", None).unwrap().value, json!(8));

    // And the failure is in the log with its location, for the settings editor
    // and for a bug report.
    let records = log.query(&LogQuery::new()).entries;
    let parse_record = records
        .iter()
        .find(|record| record.message.contains("not valid JSON"))
        .expect("the parse failure must be logged");
    assert_eq!(parse_record.fields["line"], 3);
}

#[test]
fn restart_only_settings_are_flagged_and_ordinary_ones_are_not() {
    let temp = TempWorkspace::new("restart");
    let user = temp.settings_path("home/.helix/settings.json");
    let config = service(
        ConfigPaths {
            user: Some(user.clone()),
            ..ConfigPaths::default()
        },
        logger(),
    );

    // Applied immediately: nothing to flag.
    let immediate = config
        .set(ConfigScope::User, "editor.fontSize", json!(17), None)
        .unwrap();
    assert_eq!(immediate.changed_keys, vec!["editor.fontSize"]);
    assert!(immediate.requires_restart.is_empty());

    // Written by hand, and needing a restart: flagged, and the value is still
    // reported so the UI can show what it will become.
    fs::write(
        &user,
        r#"{ "editor.fontSize": 17, "stream.bufferDepth": 4000, "helix.locale": "fr" }"#,
    )
    .unwrap();
    let changes = config.poll_external_changes();

    assert_eq!(changes.len(), 1);
    let change = &changes[0];
    assert_eq!(
        change.changed_keys,
        vec!["helix.locale", "stream.bufferDepth"]
    );
    assert_eq!(
        change.requires_restart,
        vec!["helix.locale", "stream.bufferDepth"],
        "both changed keys are restart-only and both must be named"
    );
    assert!(
        config
            .get("stream.bufferDepth", None)
            .unwrap()
            .requires_restart
    );
    assert!(
        !config
            .get("editor.fontSize", None)
            .unwrap()
            .requires_restart
    );
}

#[test]
fn a_language_specific_restart_only_change_is_reported_under_its_qualified_key() {
    // The restart flag is a property of the setting, not of the syntax used to
    // change it, so a language-qualified key must resolve to the same schema
    // entry when deciding whether a restart is needed.
    let temp = TempWorkspace::new("restart-language");
    let user = temp.write(
        "home/.helix/settings.json",
        r#"{ "[typescript].editor.formatOnSave": true }"#,
    );
    let config = service(
        ConfigPaths {
            user: Some(user),
            ..ConfigPaths::default()
        },
        logger(),
    );

    let change = config
        .reset(ConfigScope::User, "editor.formatOnSave", Some("typescript"))
        .unwrap();
    assert_eq!(
        change.changed_keys,
        vec!["[typescript].editor.formatOnSave"]
    );
    assert!(
        change.requires_restart.is_empty(),
        "formatOnSave applies immediately, qualified or not"
    );
}

#[test]
fn a_credential_in_a_committed_workspace_file_never_becomes_a_setting() {
    // The scenario the rule exists for: someone commits a token to a shared
    // settings file. It must not load, it must be reported, and it must not
    // appear in the log.
    let temp = TempWorkspace::new("secret");
    let workspace = temp.write(
        "repo/.helix/settings.json",
        r#"{
            "ai.token": "ghp_abcdefghij1234567890",
            "editor.tabSize": 2
        }"#,
    );
    let log = logger();
    let config = service(
        ConfigPaths {
            workspace: Some(workspace),
            ..ConfigPaths::default()
        },
        log.clone(),
    );

    assert!(config.get("ai.token", None).is_none());
    assert_eq!(
        config.get("editor.tabSize", None).unwrap().value,
        json!(2),
        "the rest of the file still applies"
    );

    let issue = config
        .issues()
        .into_iter()
        .find(|issue| issue.kind == IssueKind::Secret)
        .expect("the credential must be reported to the settings editor");
    assert_eq!(issue.key, "ai.token");
    assert_eq!(issue.scope, ConfigScope::Workspace);

    let exported = log.export(&LogQuery::new()).0;
    assert!(
        !exported.contains("ghp_abcdefghij1234567890"),
        "the credential must not reach any log sink"
    );
}

#[test]
fn a_write_survives_a_reload_and_reads_back_identically() {
    let temp = TempWorkspace::new("round-trip");
    let path = temp.settings_path("home/.helix/settings.json");
    let paths = ConfigPaths {
        user: Some(path.clone()),
        ..ConfigPaths::default()
    };

    {
        let config = service(paths.clone(), logger());
        config
            .set(ConfigScope::User, "editor.fontSize", json!(19), None)
            .unwrap();
        config
            .set(
                ConfigScope::User,
                "editor.tabSize",
                json!(2),
                Some("typescript"),
            )
            .unwrap();
        config
            .set(
                ConfigScope::User,
                "files.exclude",
                json!({ "**/dist": true }),
                None,
            )
            .unwrap();
    }

    // A second process reading the same file must see the same configuration.
    let reloaded = service(paths, logger());
    assert_eq!(
        reloaded.get("editor.fontSize", None).unwrap().value,
        json!(19)
    );
    assert_eq!(
        reloaded
            .get("editor.tabSize", Some("typescript"))
            .unwrap()
            .value,
        json!(2)
    );
    assert_eq!(
        reloaded.get("files.exclude", None).unwrap().value["**/dist"],
        json!(true)
    );
    assert!(reloaded.parse_errors().is_empty());
    assert!(
        reloaded.issues().is_empty(),
        "a file this service wrote must load without complaint: {:?}",
        reloaded.issues()
    );
}
