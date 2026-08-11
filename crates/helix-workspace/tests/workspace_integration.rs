//! End-to-end tests for the workspace manager (Task 1.8, REQ-FS-001).
//!
//! These drive the public surface exactly as `helix-kernel` does, against real
//! temp directories, a real configuration service, and a real file system
//! service. They cover the behaviours the task lists as its tests — multi-root
//! resolution, settings merge, add and remove, unavailable root handling — plus
//! the properties only visible across modules: the `id` appearing on first
//! write, cleanup running on close, and one workspace shared by two windows.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use helix_config::{ConfigPaths, ConfigService, SchemaRegistry};
use helix_core::error::AppError;
use helix_fs::FileSystemService;
use helix_fs::testutil::TempDir;
use helix_log::{LogLevel, Logger};
use helix_workspace::model::workspace_file_path_in;
use helix_workspace::{
    RootAvailability, RootEvent, WorkspaceEvent, WorkspaceEventKind, WorkspaceHooks,
    WorkspaceService, WorkspaceSnapshot, same_path,
};

/// Paths are compared with [`same_path`] rather than `==` throughout, because a
/// canonicalized Windows path is spelled `\\?\C:\…` and the manager stores the
/// plain form. Comparing spellings would test the platform's path syntax rather
/// than the workspace manager.
fn contains_path(paths: &[String], expected: &Path) -> bool {
    paths
        .iter()
        .any(|path| same_path(Path::new(path), expected))
}

fn sorted_keys(paths: &[String]) -> Vec<String> {
    let mut keys: Vec<String> = paths
        .iter()
        .map(|path| helix_workspace::identity::comparison_key(Path::new(path)))
        .collect();
    keys.sort();
    keys
}

fn keys_of(paths: &[&Path]) -> Vec<String> {
    let mut keys: Vec<String> = paths
        .iter()
        .map(|path| helix_workspace::identity::comparison_key(path))
        .collect();
    keys.sort();
    keys
}

/// A workspace manager over a defaults-only configuration service, with its
/// recent list inside the scratch directory.
///
/// Deliberately not `ConfigPaths::for_user()` and not the real
/// `~/.helix/recent.json`: a developer's own settings must not be able to change
/// the outcome of the suite, and a test run must not rewrite their recent
/// workspace list.
fn manager(scratch: &TempDir) -> Arc<WorkspaceService> {
    let logger = Arc::new(Logger::in_memory(LogLevel::Trace));
    let config = Arc::new(ConfigService::load(
        ConfigPaths::default(),
        Arc::new(SchemaRegistry::builtin()),
        logger.clone(),
    ));
    let fs = Arc::new(FileSystemService::with_defaults(logger.clone()));
    Arc::new(WorkspaceService::with_recent_path(
        config,
        fs,
        logger,
        Some(scratch.path().join("recent.json")),
    ))
}

/// A manager whose configuration comes from a settings file, for the cases that
/// need a non-default `workspace.maxRoots`.
fn manager_with_settings(scratch: &TempDir, settings: &str) -> Arc<WorkspaceService> {
    let logger = Arc::new(Logger::in_memory(LogLevel::Trace));
    let path = scratch.write("user-settings.json", settings);
    let config = Arc::new(ConfigService::load(
        ConfigPaths {
            user: Some(path),
            ..ConfigPaths::default()
        },
        Arc::new(SchemaRegistry::builtin()),
        logger.clone(),
    ));
    let fs = Arc::new(FileSystemService::with_defaults(logger.clone()));
    Arc::new(WorkspaceService::with_recent_path(
        config,
        fs,
        logger,
        Some(scratch.path().join("recent.json")),
    ))
}

/// Records every root bound and unbound, standing in for the watcher, language
/// servers, and terminals.
#[derive(Default)]
struct RecordingHook {
    opened: Mutex<Vec<String>>,
    closed: Mutex<Vec<String>>,
    workspaces_closed: Mutex<Vec<String>>,
    fail_on: Mutex<Option<PathBuf>>,
    failures: AtomicU32,
}

impl RecordingHook {
    fn opened(&self) -> Vec<String> {
        self.opened.lock().unwrap().clone()
    }

    fn closed(&self) -> Vec<String> {
        self.closed.lock().unwrap().clone()
    }

    fn fail_binding(&self, root: &Path) {
        *self.fail_on.lock().unwrap() = Some(root.to_path_buf());
    }
}

impl WorkspaceHooks for RecordingHook {
    fn name(&self) -> &'static str {
        "test.recorder"
    }

    fn root_opened(&self, event: &RootEvent<'_>) -> Result<(), AppError> {
        if self
            .fail_on
            .lock()
            .unwrap()
            .as_deref()
            .is_some_and(|failing| same_path(failing, event.root))
        {
            self.failures.fetch_add(1, Ordering::SeqCst);
            return Err(AppError::transient("HOOK_FAILED", "simulated bind failure"));
        }
        self.opened
            .lock()
            .unwrap()
            .push(event.root.to_string_lossy().to_string());
        Ok(())
    }

    fn root_closed(&self, event: &RootEvent<'_>) {
        self.closed
            .lock()
            .unwrap()
            .push(event.root.to_string_lossy().to_string());
    }

    fn workspace_closed(&self, key: &str) {
        self.workspaces_closed.lock().unwrap().push(key.to_string());
    }
}

/// A two-root workspace: `api` (primary) and `web`, siblings in one scratch
/// directory.
fn two_roots(dir: &TempDir) -> (PathBuf, PathBuf) {
    let api = dir.mkdir("api");
    let web = dir.mkdir("web");
    (api, web)
}

fn collect_events(workspace: &WorkspaceService) -> Arc<Mutex<Vec<WorkspaceEvent>>> {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    workspace.add_listener(Arc::new(move |event: &WorkspaceEvent| {
        sink.lock().unwrap().push(event.clone());
    }));
    events
}

fn root_named(snapshot: &WorkspaceSnapshot, name: &str) -> Option<PathBuf> {
    snapshot
        .roots
        .iter()
        .find(|root| root.name == name)
        .map(|root| root.as_path().to_path_buf())
}

// ---- multi-root resolution ----------------------------------------------

#[test]
fn a_workspace_document_opens_every_folder_it_names() {
    let dir = TempDir::new("workspace-multi-root");
    let (api, web) = two_roots(&dir);
    let tools = dir.mkdir("tools");
    std::fs::create_dir_all(workspace_file_path_in(&api).parent().unwrap()).unwrap();
    std::fs::write(
        workspace_file_path_in(&api),
        r#"{
            "id": "fixed-id",
            "name": "Payments",
            "folders": [{ "path": ".", "name": "api" }, "../web", "../tools"]
        }"#,
    )
    .unwrap();

    let workspace = manager(&dir);
    let snapshot = workspace.open(std::slice::from_ref(&api), None).unwrap();

    assert_eq!(snapshot.key, "fixed-id", "the document's id is the key");
    assert_eq!(snapshot.name, "Payments");
    assert_eq!(snapshot.roots.len(), 3);
    assert!(snapshot.roots[0].primary);
    assert_eq!(snapshot.roots[0].name, "api");
    assert!(same_path(&root_named(&snapshot, "web").unwrap(), &web));
    assert!(same_path(&root_named(&snapshot, "tools").unwrap(), &tools));
    assert!(snapshot.issues.is_empty(), "{:?}", snapshot.issues);
}

#[test]
fn opening_folders_directly_needs_no_document_and_writes_nothing() {
    let dir = TempDir::new("workspace-no-document");
    let (api, web) = two_roots(&dir);

    let workspace = manager(&dir);
    let snapshot = workspace.open(&[api.clone(), web.clone()], None).unwrap();

    assert_eq!(snapshot.roots.len(), 2);
    assert!(!snapshot.has_file);
    assert_eq!(
        snapshot.id, None,
        "the id appears on first write, not on open"
    );
    assert!(
        !workspace_file_path_in(&api).exists(),
        "opening a folder must not create a file in someone's repository"
    );
    assert_eq!(
        snapshot.key,
        helix_workspace::key_from_roots(&[api, web]),
        "with no id the key is the root-set hash"
    );
}

#[test]
fn the_key_does_not_depend_on_the_order_roots_were_opened_in() {
    let dir = TempDir::new("workspace-key-order");
    let (api, web) = two_roots(&dir);

    let one = manager(&dir);
    let first = one.open(&[api.clone(), web.clone()], None).unwrap();
    let two = manager(&dir);
    let second = two.open(&[web, api], None).unwrap();

    assert_eq!(first.key, second.key);
}

#[test]
fn a_file_is_owned_by_the_root_it_sits_in() {
    let dir = TempDir::new("workspace-ownership");
    let (api, web) = two_roots(&dir);
    let workspace = manager(&dir);
    let snapshot = workspace.open(&[api.clone(), web.clone()], None).unwrap();

    assert!(same_path(
        &workspace
            .owning_root(&snapshot.key, &web.join("src/app.ts"))
            .unwrap(),
        &web
    ));
    assert!(same_path(
        &workspace
            .owning_root(&snapshot.key, &api.join("src/main.rs"))
            .unwrap(),
        &api
    ));
    assert_eq!(
        workspace.owning_root(&snapshot.key, Path::new("/elsewhere/x.txt")),
        None
    );
}

// ---- settings merge ------------------------------------------------------

#[test]
fn folder_settings_override_workspace_settings_for_their_own_root() {
    let dir = TempDir::new("workspace-settings-merge");
    let (api, web) = two_roots(&dir);
    std::fs::create_dir_all(workspace_file_path_in(&api).parent().unwrap()).unwrap();
    std::fs::write(
        workspace_file_path_in(&api),
        r#"{
            "id": "settings-demo",
            "folders": [".", "../web"],
            "settings": { "editor.tabSize": 2 }
        }"#,
    )
    .unwrap();
    // The `web` root disagrees, for its own files only.
    std::fs::create_dir_all(web.join(".helix")).unwrap();
    std::fs::write(
        helix_config::settings_path_in(&web),
        r#"{ "editor.tabSize": 8, "files.insertFinalNewline": true }"#,
    )
    .unwrap();

    let workspace = manager(&dir);
    let snapshot = workspace.open(std::slice::from_ref(&api), None).unwrap();

    assert_eq!(
        workspace
            .setting_value(&snapshot.key, Some(&web.join("app.ts")), "editor.tabSize")
            .unwrap(),
        Some(serde_json::json!(8))
    );
    assert_eq!(
        workspace
            .setting_value(&snapshot.key, Some(&api.join("main.rs")), "editor.tabSize")
            .unwrap(),
        Some(serde_json::json!(2)),
        "one root's folder settings must not reach its neighbour"
    );
    assert_eq!(
        workspace
            .setting_value(&snapshot.key, None, "editor.fontSize")
            .unwrap(),
        Some(serde_json::json!(14)),
        "a key no layer sets keeps the schema default"
    );
    assert_eq!(
        workspace
            .setting_value(
                &snapshot.key,
                Some(&api.join("main.rs")),
                "files.insertFinalNewline"
            )
            .unwrap(),
        Some(serde_json::json!(false)),
        "and a folder-only setting does not leak either"
    );
}

#[test]
fn the_primary_roots_settings_file_layers_over_the_workspace_document() {
    let dir = TempDir::new("workspace-settings-primary");
    let api = dir.mkdir("api");
    std::fs::create_dir_all(api.join(".helix")).unwrap();
    std::fs::write(
        workspace_file_path_in(&api),
        r#"{ "id": "p", "folders": ["."], "settings": { "editor.tabSize": 2, "editor.fontSize": 20 } }"#,
    )
    .unwrap();
    std::fs::write(
        helix_config::settings_path_in(&api),
        r#"{ "editor.tabSize": 3 }"#,
    )
    .unwrap();

    let workspace = manager(&dir);
    let snapshot = workspace.open(&[api], None).unwrap();
    let tree = workspace.settings_tree(&snapshot.key, None).unwrap();

    assert_eq!(tree["editor"]["tabSize"], 3);
    assert_eq!(tree["editor"]["fontSize"], 20);
}

#[test]
fn a_root_added_at_runtime_brings_its_settings_with_it() {
    let dir = TempDir::new("workspace-settings-added");
    let (api, web) = two_roots(&dir);
    std::fs::create_dir_all(web.join(".helix")).unwrap();
    std::fs::write(
        helix_config::settings_path_in(&web),
        r#"{ "editor.tabSize": 7 }"#,
    )
    .unwrap();

    let workspace = manager(&dir);
    let snapshot = workspace.open(&[api], None).unwrap();
    assert_eq!(
        workspace
            .setting_value(&snapshot.key, Some(&web.join("app.ts")), "editor.tabSize")
            .unwrap(),
        Some(serde_json::json!(4)),
        "before the root is added the file belongs to no root"
    );

    workspace.add_root(&snapshot.key, &web, None).unwrap();
    assert_eq!(
        workspace
            .setting_value(&snapshot.key, Some(&web.join("app.ts")), "editor.tabSize")
            .unwrap(),
        Some(serde_json::json!(7))
    );
}

// ---- add and remove ------------------------------------------------------

#[test]
fn adding_a_root_writes_the_document_and_assigns_the_id() {
    let dir = TempDir::new("workspace-add-root");
    let (api, web) = two_roots(&dir);
    let workspace = manager(&dir);
    let hook = Arc::new(RecordingHook::default());
    workspace.add_hook(hook.clone());

    let opened = workspace.open(std::slice::from_ref(&api), None).unwrap();
    assert!(opened.id.is_none());

    let after = workspace
        .add_root(&opened.key, &web, Some("frontend"))
        .unwrap();

    assert_eq!(after.roots.len(), 2);
    assert_eq!(after.roots[1].name, "frontend");
    assert!(after.has_file, "the change is persisted");
    assert!(
        after.id.is_some(),
        "the stable id is generated on the first write (REQ-FS-001.2)"
    );
    assert_eq!(after.persist_error, None);

    // The document on disk names both folders, relatively.
    let body = std::fs::read_to_string(workspace_file_path_in(&api)).unwrap();
    let document: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(document["id"], serde_json::json!(after.id.clone().unwrap()));
    let folders: Vec<String> = document["folders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["path"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(folders, vec![".".to_string(), "../web".to_string()]);

    // And the new root was bound.
    assert!(contains_path(&hook.opened(), after.roots[1].as_path()));
}

#[test]
fn the_assigned_id_becomes_the_key_the_next_time_the_workspace_opens() {
    let dir = TempDir::new("workspace-id-key");
    let (api, web) = two_roots(&dir);

    let first = manager(&dir);
    let opened = first.open(std::slice::from_ref(&api), None).unwrap();
    let after = first.add_root(&opened.key, &web, None).unwrap();
    let id = after.id.clone().expect("an id was assigned");
    assert_eq!(
        after.key, opened.key,
        "an open workspace's key never moves under it"
    );
    first.close(&after.key).unwrap();

    let second = manager(&dir);
    let reopened = second.open(&[api], None).unwrap();
    assert_eq!(reopened.key, id, "and the id is the key from then on");
    assert_eq!(reopened.roots.len(), 2, "both folders come back");
}

#[test]
fn removing_a_root_releases_it_and_leaves_the_rest_alone() {
    let dir = TempDir::new("workspace-remove-root");
    let (api, web) = two_roots(&dir);
    let workspace = manager(&dir);
    let hook = Arc::new(RecordingHook::default());
    workspace.add_hook(hook.clone());

    let opened = workspace.open(&[api.clone(), web.clone()], None).unwrap();
    let after = workspace.remove_root(&opened.key, &web).unwrap();

    assert_eq!(after.roots.len(), 1);
    assert!(same_path(after.roots[0].as_path(), &api));
    assert_eq!(
        sorted_keys(&hook.closed()),
        keys_of(&[web.as_path()]),
        "exactly the removed root is cleaned up"
    );
    assert!(
        workspace
            .owning_root(&after.key, &web.join("app.ts"))
            .is_none(),
        "the removed root no longer owns its files"
    );

    let document: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(workspace_file_path_in(&api)).unwrap())
            .unwrap();
    assert_eq!(document["folders"].as_array().unwrap().len(), 1);
}

#[test]
fn the_only_root_cannot_be_removed() {
    let dir = TempDir::new("workspace-last-root");
    let api = dir.mkdir("api");
    let workspace = manager(&dir);
    let opened = workspace.open(std::slice::from_ref(&api), None).unwrap();

    let error = workspace.remove_root(&opened.key, &api).unwrap_err();
    assert_eq!(error.code, "WORKSPACE_LAST_ROOT");
    assert_eq!(workspace.snapshot(&opened.key).unwrap().roots.len(), 1);
}

#[test]
fn adding_a_root_twice_is_refused_rather_than_duplicated() {
    let dir = TempDir::new("workspace-duplicate-root");
    let (api, web) = two_roots(&dir);
    let workspace = manager(&dir);
    let opened = workspace.open(&[api], None).unwrap();
    workspace.add_root(&opened.key, &web, None).unwrap();

    let error = workspace.add_root(&opened.key, &web, None).unwrap_err();
    assert_eq!(error.code, "WORKSPACE_ROOT_EXISTS");
    assert_eq!(workspace.snapshot(&opened.key).unwrap().roots.len(), 2);
}

#[test]
fn the_root_limit_is_enforced_and_configurable() {
    let dir = TempDir::new("workspace-root-limit");
    let workspace = manager_with_settings(&dir, r#"{ "workspace.maxRoots": 2 }"#);
    let (api, web) = two_roots(&dir);
    let tools = dir.mkdir("tools");

    let opened = workspace.open(&[api, web], None).unwrap();
    assert_eq!(opened.max_roots, 2);
    assert!(
        opened.at_root_limit,
        "the snapshot says so at the threshold, which is what the UI warns on"
    );

    let error = workspace.add_root(&opened.key, &tools, None).unwrap_err();
    assert_eq!(error.code, "WORKSPACE_ROOT_LIMIT");
    assert!(
        error.message.contains("workspace.maxRoots"),
        "the error names the setting to change: {}",
        error.message
    );
}

#[test]
fn a_document_naming_more_folders_than_the_limit_opens_on_the_ones_that_fit() {
    let dir = TempDir::new("workspace-limit-document");
    let api = dir.mkdir("api");
    let web = dir.mkdir("web");
    let tools = dir.mkdir("tools");
    std::fs::create_dir_all(api.join(".helix")).unwrap();
    std::fs::write(
        workspace_file_path_in(&api),
        r#"{ "id": "x", "folders": [".", "../web", "../tools"] }"#,
    )
    .unwrap();

    let workspace = manager_with_settings(&dir, r#"{ "workspace.maxRoots": 2 }"#);
    let snapshot = workspace.open(&[api], None).unwrap();

    assert_eq!(snapshot.roots.len(), 2);
    assert!(
        snapshot
            .issues
            .iter()
            .any(|issue| issue.message.contains("at most 2")),
        "{:?}",
        snapshot.issues
    );
    assert!(root_named(&snapshot, "web").is_some());
    assert!(root_named(&snapshot, "tools").is_none());
    let _ = (web, tools);
}

// ---- unavailable roots ---------------------------------------------------

#[test]
fn an_unavailable_root_does_not_block_the_others() {
    let dir = TempDir::new("workspace-unavailable");
    let (api, web) = two_roots(&dir);
    let deleted = dir.mkdir("gone");
    std::fs::remove_dir_all(&deleted).unwrap();
    let unmounted = if cfg!(windows) {
        PathBuf::from(r"Q:\share\project")
    } else {
        PathBuf::from("/nonexistent-mount-for-tests/share/project")
    };

    let workspace = manager(&dir);
    let hook = Arc::new(RecordingHook::default());
    workspace.add_hook(hook.clone());

    let snapshot = workspace
        .open(
            &[api.clone(), deleted.clone(), unmounted.clone(), web.clone()],
            None,
        )
        .unwrap();

    let availability_of = |path: &Path| {
        snapshot
            .roots
            .iter()
            .find(|root| same_path(root.as_path(), path))
            .map(|root| root.availability)
    };

    assert_eq!(snapshot.roots.len(), 4, "every root is still listed");
    assert_eq!(snapshot.available_roots().len(), 2);
    assert_eq!(availability_of(&deleted), Some(RootAvailability::Missing));
    assert_eq!(
        availability_of(&unmounted),
        Some(RootAvailability::Unavailable)
    );

    // Only the usable roots were bound, and the usable ones still work.
    assert_eq!(hook.opened().len(), 2);
    assert!(same_path(
        &workspace
            .owning_root(&snapshot.key, &web.join("app.ts"))
            .unwrap(),
        &web
    ));
}

#[test]
fn a_root_that_comes_back_is_bound_on_the_next_retry() {
    let dir = TempDir::new("workspace-root-returns");
    let api = dir.mkdir("api");
    let late = dir.path().join("late");

    let workspace = manager(&dir);
    let hook = Arc::new(RecordingHook::default());
    workspace.add_hook(hook.clone());

    let snapshot = workspace.open(&[api, late.clone()], None).unwrap();
    assert_eq!(snapshot.available_roots().len(), 1);
    assert_eq!(hook.opened().len(), 1);

    std::fs::create_dir_all(&late).unwrap();
    let events = workspace.refresh_availability();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, WorkspaceEventKind::AvailabilityChanged);
    assert_eq!(events[0].roots.len(), 1);
    assert!(events[0].roots[0].availability.is_available());
    assert_eq!(
        hook.opened().len(),
        2,
        "the root that returned is now bound"
    );
    assert!(
        workspace
            .snapshot(&snapshot.key)
            .unwrap()
            .available_roots()
            .len()
            == 2
    );
}

#[test]
fn a_root_deleted_underneath_the_workspace_is_noticed_and_released() {
    let dir = TempDir::new("workspace-root-deleted");
    let (api, web) = two_roots(&dir);
    let workspace = manager(&dir);
    let hook = Arc::new(RecordingHook::default());
    workspace.add_hook(hook.clone());
    let snapshot = workspace.open(&[api.clone(), web.clone()], None).unwrap();

    std::fs::remove_dir_all(&web).unwrap();
    let events = workspace.refresh_availability();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].roots[0].availability, RootAvailability::Missing);
    assert_eq!(
        sorted_keys(&hook.closed()),
        keys_of(&[web.as_path()]),
        "the vanished root is released, and only it"
    );
    let after = workspace.snapshot(&snapshot.key).unwrap();
    assert_eq!(after.roots.len(), 2, "it is still offered for removal");
    assert_eq!(after.available_roots().len(), 1);
}

#[test]
fn a_hook_that_fails_to_bind_a_root_does_not_fail_the_open() {
    let dir = TempDir::new("workspace-hook-failure");
    let (api, web) = two_roots(&dir);
    let workspace = manager(&dir);
    let hook = Arc::new(RecordingHook::default());
    hook.fail_binding(&web);
    workspace.add_hook(hook.clone());

    let snapshot = workspace.open(&[api.clone(), web], None).unwrap();

    assert_eq!(snapshot.roots.len(), 2);
    assert_eq!(hook.failures.load(Ordering::SeqCst), 1);
    assert_eq!(sorted_keys(&hook.opened()), keys_of(&[api.as_path()]));
    assert_eq!(workspace.metrics().hook_errors, 1);
}

// ---- document failure modes ---------------------------------------------

#[test]
fn a_document_that_will_not_parse_still_opens_the_requested_folder() {
    let dir = TempDir::new("workspace-bad-document");
    let api = dir.mkdir("api");
    std::fs::create_dir_all(api.join(".helix")).unwrap();
    std::fs::write(workspace_file_path_in(&api), "{ \"folders\": [ oops").unwrap();

    let workspace = manager(&dir);
    let snapshot = workspace.open(std::slice::from_ref(&api), None).unwrap();

    assert_eq!(snapshot.roots.len(), 1);
    let error = snapshot.parse_error.expect("the parse error is reported");
    assert!(error.path.ends_with("workspace.json"), "{}", error.path);
    assert!(error.line >= 1);
    assert_eq!(workspace.metrics().parse_errors, 1);
}

#[test]
fn a_read_only_workspace_still_accepts_a_root_for_the_session() {
    let dir = TempDir::new("workspace-read-only");
    let (api, web) = two_roots(&dir);
    // A directory where `.helix/` cannot be created, which is what a read-only
    // checkout looks like from here: the path is occupied by a file.
    std::fs::write(api.join(".helix"), "not a directory").unwrap();

    let workspace = manager(&dir);
    let opened = workspace.open(&[api], None).unwrap();
    let after = workspace.add_root(&opened.key, &web, None).unwrap();

    assert_eq!(after.roots.len(), 2, "the root is added regardless");
    assert!(
        after.persist_error.is_some(),
        "and the failure to save is reported rather than hidden"
    );
    assert_eq!(workspace.metrics().document_write_errors, 1);
}

// ---- lifecycle, sharing, and recents ------------------------------------

#[test]
fn closing_the_last_holder_releases_every_root_and_the_workspace_scope() {
    let dir = TempDir::new("workspace-close");
    let (api, web) = two_roots(&dir);
    let workspace = manager(&dir);
    let hook = Arc::new(RecordingHook::default());
    workspace.add_hook(hook.clone());

    let snapshot = workspace.open(&[api.clone(), web.clone()], None).unwrap();
    let key = snapshot.key.clone();
    assert_eq!(workspace.registry().ref_count(&key), 1);

    assert!(workspace.close(&key).unwrap(), "the last holder tears down");

    assert!(!workspace.is_open(&key));
    assert_eq!(workspace.registry().ref_count(&key), 0);
    assert_eq!(
        sorted_keys(&hook.closed()),
        keys_of(&[api.as_path(), web.as_path()]),
        "every root is cleaned up"
    );
    assert_eq!(hook.workspaces_closed.lock().unwrap().len(), 1);
}

#[test]
fn two_windows_share_one_workspace_and_the_first_close_changes_nothing() {
    let dir = TempDir::new("workspace-shared");
    let (api, web) = two_roots(&dir);
    let workspace = manager(&dir);
    let hook = Arc::new(RecordingHook::default());
    workspace.add_hook(hook.clone());

    let first = workspace.open(&[api.clone(), web.clone()], None).unwrap();
    let second = workspace.open(&[api, web], None).unwrap();

    assert_eq!(first.key, second.key);
    assert_eq!(second.holders, 2);
    assert_eq!(
        hook.opened().len(),
        2,
        "the second window joins the roots already bound rather than binding them again"
    );

    assert!(
        !workspace.close(&second.key).unwrap(),
        "one window closing must not tear down a shared workspace"
    );
    assert!(workspace.is_open(&second.key));
    assert!(hook.closed().is_empty());
    assert_eq!(workspace.snapshot(&second.key).unwrap().holders, 1);

    assert!(workspace.close(&second.key).unwrap());
    assert_eq!(hook.closed().len(), 2);
}

#[test]
fn workspace_scoped_resources_are_shared_and_dropped_with_the_scope() {
    let dir = TempDir::new("workspace-scoped-resources");
    let api = dir.mkdir("api");
    let workspace = manager(&dir);
    let snapshot = workspace.open(&[api], None).unwrap();

    let index = Arc::new(String::from("search index"));
    workspace.registry().publish(&snapshot.key, index.clone());
    assert_eq!(Arc::strong_count(&index), 2);
    assert_eq!(
        workspace.registry().resolve::<String>(&snapshot.key),
        Some(index.clone())
    );

    workspace.close(&snapshot.key).unwrap();
    assert_eq!(
        Arc::strong_count(&index),
        1,
        "a closed workspace keeps nothing alive"
    );
}

#[test]
fn opening_records_the_workspace_in_the_recent_list() {
    let dir = TempDir::new("workspace-recent");
    let (api, web) = two_roots(&dir);
    let workspace = manager(&dir);

    let snapshot = workspace.open(&[api.clone(), web], None).unwrap();
    let recent = workspace.recent();

    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].key, snapshot.key);
    assert_eq!(recent[0].roots.len(), 2);
    assert!(
        dir.path().join("recent.json").exists(),
        "the list is persisted in user data, not in the workspace"
    );

    assert!(workspace.forget_recent(&snapshot.key));
    assert!(workspace.recent().is_empty());
}

#[test]
fn the_recent_list_survives_a_restart_of_the_manager() {
    let dir = TempDir::new("workspace-recent-restart");
    let api = dir.mkdir("api");

    let first = manager(&dir);
    let snapshot = first.open(&[api], None).unwrap();
    first.close(&snapshot.key).unwrap();

    let second = manager(&dir);
    let recent = second.recent();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].key, snapshot.key);
}

#[test]
fn every_change_is_announced_on_the_event_stream() {
    let dir = TempDir::new("workspace-events");
    let (api, web) = two_roots(&dir);
    let workspace = manager(&dir);
    let events = collect_events(&workspace);

    let opened = workspace.open(&[api], None).unwrap();
    workspace.add_root(&opened.key, &web, None).unwrap();
    workspace.remove_root(&opened.key, &web).unwrap();
    workspace.close(&opened.key).unwrap();

    let kinds: Vec<WorkspaceEventKind> = events
        .lock()
        .unwrap()
        .iter()
        .map(|event| event.kind)
        .collect();
    assert_eq!(kinds[0], WorkspaceEventKind::Opened);
    assert!(kinds.contains(&WorkspaceEventKind::RootsChanged));
    assert!(kinds.contains(&WorkspaceEventKind::DocumentWritten));
    assert_eq!(kinds.last(), Some(&WorkspaceEventKind::Closed));
    assert!(
        events
            .lock()
            .unwrap()
            .last()
            .map(|event| event.torn_down)
            .unwrap_or(false)
    );
}

#[test]
fn a_workspace_that_is_not_open_is_a_typed_error_rather_than_a_panic() {
    let dir = TempDir::new("workspace-unknown-key");
    let workspace = manager(&dir);

    for error in [
        workspace.close("nope").unwrap_err(),
        workspace
            .add_root("nope", Path::new("/tmp/x"), None)
            .unwrap_err(),
        workspace
            .remove_root("nope", Path::new("/tmp/x"))
            .unwrap_err(),
        workspace.settings_tree("nope", None).unwrap_err(),
    ] {
        assert_eq!(error.code, "WORKSPACE_NOT_OPEN");
    }
    assert!(workspace.open(&[], None).is_err());
}
