//! The workspace manager: open and close, roots, settings, and cleanup
//! (Task 1.8, REQ-FS-001).
//!
//! Like `ConfigService` and `FileSystemService`, this is the subsystem itself
//! and knows nothing about Tauri or the service container. `helix-kernel` wraps
//! it as a managed service, registers its commands, bridges its events onto the
//! streaming channel, and drives the availability retry.
//!
//! ```text
//!  open(roots) ──► read .helix/workspace.json ──► resolve roots ──► probe each
//!                            │                        │              │
//!                            │                        │              ▼
//!                            │                        │        available? hooks
//!                            │                        │        bind: watchers,
//!                            │                        │        servers, terminals
//!                            ▼                        ▼
//!                     settings layers          workspace key ──► lease (refcount)
//!
//!  close(key) ──► drop one reference ──► last one? unbind every root, tear down
//! ```
//!
//! ## Decisions worth naming
//!
//! **One unusable root never blocks the others.** Every root is probed
//! independently and a root that is missing or on an unmounted drive is opened
//! as unavailable: it appears in the workspace, it is not watched, and the rest
//! of the workspace behaves exactly as if it were alone (REQ-FS-001 failure
//! modes). A workspace that refused to open because a colleague's VPN dropped
//! would be worse than useless.
//!
//! **Opening never writes.** Opening a folder that has no `.helix/workspace.json`
//! creates nothing. A user who opens someone else's repository to read it should
//! not find a new file in `git status` afterwards. The document is written when
//! the user changes the workspace — adding or removing a root — and that write
//! is where the stable `id` is generated (REQ-FS-001.2).
//!
//! **An open workspace's key never changes.** The key is resolved once, at open:
//! the document's `id` when it has one, the root-set hash otherwise. Generating
//! an `id` on the first write therefore takes effect from the *next* open, and
//! the write is announced as [`WorkspaceEventKind::DocumentWritten`] carrying
//! the new id. The alternative — rekeying a live workspace — would move the
//! session state directory and every workspace-scoped service out from under a
//! window that is using them, to gain nothing this session.
//!
//! **A failed document write does not fail the operation.** A root added in a
//! read-only checkout is added; the persistence failure is reported on the
//! snapshot as `persist_error`. Refusing to add a root because the repository is
//! read-only would break the case REQ-NFR-002 explicitly requires to keep
//! working.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use helix_config::{
    ConfigChange, ConfigParseError, ConfigPaths, ConfigScope, ConfigService, SettingIssue,
    SettingValue,
};
use helix_core::error::AppError;
use helix_fs::{FileSystemService, WriteOptions};
use helix_log::{Logger, log_info, log_warn};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::identity::{canonical_path, comparison_key, generate_id, same_path, workspace_key};
use crate::model::{
    FolderEntry, MAX_ROOTS, RootAvailability, WorkspaceFile, WorkspaceIssue, WorkspaceIssueKind,
    WorkspaceRoot, authored_path, workspace_file_path_in,
};
use crate::recent::{RecentWorkspace, RecentWorkspaces, recent_path};
use crate::registry::{WorkspaceLease, WorkspaceRegistry};
use crate::settings::WorkspaceSettings;

/// Log source for workspace records.
pub const LOG_SOURCE: &str = "kernel.workspace";

/// Setting that caps the root count (REQ-FS-001.5).
pub const MAX_ROOTS_SETTING: &str = "workspace.maxRoots";

/// How often an unavailable root is retried, unless the kernel says otherwise.
pub const DEFAULT_RETRY_INTERVAL: Duration = Duration::from_secs(5);

/// A root, at the moment something is being done to it. Passed to hooks so a
/// subsystem does not have to look the workspace back up to act on one root.
#[derive(Debug, Clone, Copy)]
pub struct RootEvent<'a> {
    /// The workspace key, which is what a workspace-scoped subsystem keys its
    /// own state by.
    pub key: &'a str,
    pub root: &'a Path,
    pub availability: RootAvailability,
}

/// What a subsystem implements to be bound to a workspace's roots.
///
/// This is the seam the open/close lifecycle cleans up through: the file
/// watcher, language servers, and terminals each register a hook and are
/// unbound when a root leaves the workspace or the workspace closes. Cleanup
/// being a registration rather than a hard-coded list is what keeps the next
/// subsystem from being the one that leaks — a hook that is registered is a hook
/// that runs.
///
/// Hooks are called on the caller's thread, so an implementation does the
/// minimum and defers anything slow. `root_closed` and `workspace_closed`
/// return nothing on purpose: cleanup cannot be refused.
pub trait WorkspaceHooks: Send + Sync {
    /// Name used in logs when this hook reports a problem.
    fn name(&self) -> &'static str;

    /// A root became part of an open workspace. Only called for roots that are
    /// actually available.
    ///
    /// An error is logged and does not fail the open: a language server that
    /// will not start is not a reason to refuse to show a user their files.
    fn root_opened(&self, event: &RootEvent<'_>) -> Result<(), AppError> {
        let _ = event;
        Ok(())
    }

    /// A root left the workspace, or the workspace is closing.
    fn root_closed(&self, event: &RootEvent<'_>) {
        let _ = event;
    }

    /// The workspace's last holder went away, after every root was closed.
    fn workspace_closed(&self, key: &str) {
        let _ = key;
    }
}

/// What happened to a workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceEventKind {
    /// A workspace was opened, or an already-open one gained a holder.
    Opened,
    /// A holder released it. `torn_down` on the event says whether it was the
    /// last one.
    Closed,
    /// A root was added or removed (REQ-FS-001.4).
    RootsChanged,
    /// A root's availability changed, in either direction.
    AvailabilityChanged,
    /// User, workspace, or folder settings changed while the workspace was open.
    SettingsChanged,
    /// `.helix/workspace.json` was written, possibly assigning the `id` for the
    /// first time.
    DocumentWritten,
    /// `.helix/workspace.json` changed outside the application and was reloaded.
    DocumentChanged,
}

/// A workspace change, delivered to listeners and published on the streaming
/// channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct WorkspaceEvent {
    pub kind: WorkspaceEventKind,
    pub key: String,
    /// The workspace after the change. Absent for the close that tore it down,
    /// because there is nothing left to describe.
    pub workspace: Option<WorkspaceSnapshot>,
    /// Roots this change was about, for a listener that only cares about those.
    pub roots: Vec<WorkspaceRoot>,
    /// True on a `Closed` event that released the last holder.
    pub torn_down: bool,
}

/// Everything a window needs to know about an open workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct WorkspaceSnapshot {
    /// State and cache key (REQ-NFR-002.11). Stable for the life of this open
    /// workspace.
    pub key: String,
    /// The document's `id`, once it has one.
    pub id: Option<String>,
    pub name: String,
    /// Roots in workspace order; the first is primary.
    pub roots: Vec<WorkspaceRoot>,
    /// Where `.helix/workspace.json` is, or would be.
    pub file_path: String,
    /// Whether that file exists yet.
    pub has_file: bool,
    /// Validation findings from the document (REQ-FS-001 failure modes).
    pub issues: Vec<WorkspaceIssue>,
    /// Set when the document would not parse. The workspace opened anyway, on
    /// the roots that were requested.
    pub parse_error: Option<ConfigParseError>,
    /// Invalid workspace/folder settings files, retaining their last valid layer.
    pub settings_parse_errors: Vec<ConfigParseError>,
    /// Per-setting schema and scope problems in workspace/folder layers.
    pub settings_issues: Vec<SettingIssue>,
    /// Set when the last write of the document failed, e.g. a read-only
    /// checkout. The change is in effect for this session regardless.
    pub persist_error: Option<String>,
    pub max_roots: usize,
    /// True at the cap, which is the point the UI warns at (REQ-FS-001.5).
    pub at_root_limit: bool,
    /// How many holders (windows) share this workspace.
    pub holders: u32,
    pub opened_ms: u64,
}

impl WorkspaceSnapshot {
    /// Roots that can be used right now.
    pub fn available_roots(&self) -> Vec<&WorkspaceRoot> {
        self.roots
            .iter()
            .filter(|root| root.availability.is_available())
            .collect()
    }

    pub fn primary_root(&self) -> Option<&WorkspaceRoot> {
        self.roots.first()
    }
}

/// Counters behind the service's health report.
#[derive(Debug, Default)]
struct Counters {
    opens: AtomicU64,
    closes: AtomicU64,
    roots_added: AtomicU64,
    roots_removed: AtomicU64,
    document_writes: AtomicU64,
    document_write_errors: AtomicU64,
    parse_errors: AtomicU64,
    hook_errors: AtomicU64,
}

/// Point-in-time counters, surfaced through the kernel's health model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceMetrics {
    pub open_workspaces: u64,
    pub unavailable_roots: u64,
    pub opens: u64,
    pub closes: u64,
    pub roots_added: u64,
    pub roots_removed: u64,
    pub document_writes: u64,
    pub document_write_errors: u64,
    pub parse_errors: u64,
    pub hook_errors: u64,
}

/// One open workspace's state.
struct OpenWorkspace {
    key: String,
    name: String,
    primary: PathBuf,
    file_path: PathBuf,
    has_file: bool,
    document: WorkspaceFile,
    roots: Vec<WorkspaceRoot>,
    issues: Vec<WorkspaceIssue>,
    parse_error: Option<ConfigParseError>,
    persist_error: Option<String>,
    settings: WorkspaceSettings,
    settings_parse_errors: Vec<ConfigParseError>,
    settings_issues: Vec<SettingIssue>,
    /// One lease per holder. The registry's reference count follows this vector,
    /// and the scope ends when the last one is dropped.
    leases: Vec<WorkspaceLease>,
    opened_ms: u64,
}

impl OpenWorkspace {
    fn snapshot(&self, max_roots: usize) -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            key: self.key.clone(),
            id: if self.document.id.is_empty() {
                None
            } else {
                Some(self.document.id.clone())
            },
            name: self.name.clone(),
            roots: self.roots.clone(),
            file_path: self.file_path.to_string_lossy().to_string(),
            has_file: self.has_file,
            issues: self.issues.clone(),
            parse_error: self.parse_error.clone(),
            settings_parse_errors: self.settings_parse_errors.clone(),
            settings_issues: self.settings_issues.clone(),
            persist_error: self.persist_error.clone(),
            max_roots,
            at_root_limit: self.roots.len() >= max_roots,
            holders: self.leases.len() as u32,
            opened_ms: self.opened_ms,
        }
    }

    fn root_paths(&self) -> Vec<PathBuf> {
        self.roots
            .iter()
            .map(|root| root.as_path().to_path_buf())
            .collect()
    }

    fn find(&self, path: &Path) -> Option<usize> {
        self.roots
            .iter()
            .position(|root| same_path(root.as_path(), path))
    }
}

/// A callback invoked after every workspace change. The kernel uses one to
/// publish onto the streaming channel.
pub type WorkspaceListener = Arc<dyn Fn(&WorkspaceEvent) + Send + Sync>;

/// A workspace- or folder-layer configuration change.
pub type ConfigChangeListener = Arc<dyn Fn(&ConfigChange) + Send + Sync>;

/// The workspace manager.
pub struct WorkspaceService {
    config: Arc<ConfigService>,
    fs: Arc<FileSystemService>,
    logger: Arc<Logger>,
    registry: Arc<WorkspaceRegistry>,
    open: RwLock<BTreeMap<String, OpenWorkspace>>,
    hooks: RwLock<Vec<Arc<dyn WorkspaceHooks>>>,
    listeners: RwLock<Vec<WorkspaceListener>>,
    config_listeners: RwLock<Vec<ConfigChangeListener>>,
    recent: RwLock<RecentWorkspaces>,
    /// Where the recent list is stored. `None` on a machine with no home
    /// directory, where the list lives for the session and is not persisted.
    recent_path: Option<PathBuf>,
    counters: Counters,
}

impl WorkspaceService {
    /// Build the manager and load the recent list.
    pub fn new(
        config: Arc<ConfigService>,
        fs: Arc<FileSystemService>,
        logger: Arc<Logger>,
    ) -> Self {
        Self::with_recent_path(config, fs, logger, recent_path())
    }

    /// [`new`](Self::new) with the recent list stored somewhere specific, so a
    /// test never touches the developer's own `~/.helix/recent.json`.
    pub fn with_recent_path(
        config: Arc<ConfigService>,
        fs: Arc<FileSystemService>,
        logger: Arc<Logger>,
        recent_path: Option<PathBuf>,
    ) -> Self {
        let recent = recent_path
            .as_deref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|body| RecentWorkspaces::parse(&body))
            .unwrap_or_default();

        Self {
            config,
            fs,
            logger,
            registry: WorkspaceRegistry::new(),
            open: RwLock::new(BTreeMap::new()),
            hooks: RwLock::new(Vec::new()),
            listeners: RwLock::new(Vec::new()),
            config_listeners: RwLock::new(Vec::new()),
            recent: RwLock::new(recent),
            recent_path,
            counters: Counters::default(),
        }
    }

    /// The workspace-scoped service registry, for subsystems that hold
    /// per-workspace state (REQ-ARCH-006).
    pub fn registry(&self) -> &Arc<WorkspaceRegistry> {
        &self.registry
    }

    /// Register a lifecycle hook. Hooks are called in registration order for
    /// open, and in reverse for close, so a subsystem is torn down before
    /// whatever it was built on.
    pub fn add_hook(&self, hook: Arc<dyn WorkspaceHooks>) {
        self.hooks.write().unwrap().push(hook);
    }

    /// Register a listener called after every change. The kernel uses this to
    /// publish onto the streaming channel.
    pub fn add_listener(&self, listener: WorkspaceListener) {
        self.listeners.write().unwrap().push(listener);
    }

    /// Register a listener for scoped configuration changes.
    pub fn add_config_listener(&self, listener: ConfigChangeListener) {
        self.config_listeners.write().unwrap().push(listener);
    }

    /// The configured root cap, never above [`MAX_ROOTS`].
    pub fn max_roots(&self) -> usize {
        self.config
            .integer_value(MAX_ROOTS_SETTING)
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value > 0)
            .unwrap_or(MAX_ROOTS)
            .min(MAX_ROOTS)
    }

    // ---- lifecycle -------------------------------------------------------

    /// Open a workspace on `roots`, the first of which is primary.
    ///
    /// Opening a workspace that is already open takes another reference and
    /// returns the workspace as it stands, which is what a second window on the
    /// same project does.
    pub fn open(
        &self,
        roots: &[PathBuf],
        name: Option<&str>,
    ) -> Result<WorkspaceSnapshot, AppError> {
        if roots.is_empty() {
            return Err(AppError::permanent(
                "WORKSPACE_NO_ROOTS",
                "a workspace needs at least one folder",
            ));
        }
        let max_roots = self.max_roots();
        let primary = canonical_path(&roots[0]);
        let file_path = workspace_file_path_in(&primary);

        // The document decides the id, contributes folders, and may fail to
        // parse — in which case the workspace still opens on what was asked for.
        let (document, mut issues, parse_error, has_file) =
            self.read_document(&file_path, max_roots);
        if parse_error.is_some() {
            self.counters.parse_errors.fetch_add(1, Ordering::Relaxed);
        }

        let mut ordered = vec![primary.clone()];
        for folder in document.resolved_folders(&primary) {
            push_unique(&mut ordered, canonical_path(&folder));
        }
        for requested in roots.iter().skip(1) {
            push_unique(&mut ordered, canonical_path(requested));
        }
        if ordered.len() > max_roots {
            let dropped: Vec<String> = ordered
                .split_off(max_roots)
                .iter()
                .map(|path| path.to_string_lossy().to_string())
                .collect();
            issues.push(WorkspaceIssue::new(
                WorkspaceIssueKind::TooManyFolders,
                "folders",
                format!(
                    "a workspace holds at most {max_roots} folders; {} were not opened: {}",
                    dropped.len(),
                    dropped.join(", ")
                ),
            ));
        }

        let key = workspace_key(Some(document.id.as_str()), &ordered);

        let resolved_roots: Vec<WorkspaceRoot> = ordered
            .iter()
            .enumerate()
            .map(|(index, path)| WorkspaceRoot {
                path: path.to_string_lossy().to_string(),
                name: document
                    .name_for(&primary, path)
                    .unwrap_or_else(|| display_name(path)),
                availability: RootAvailability::probe(path),
                primary: index == 0,
            })
            .collect();

        let display = document
            .name
            .clone()
            .or_else(|| name.map(str::to_string))
            .unwrap_or_else(|| display_name(&primary));

        let mut workspace = OpenWorkspace {
            key: key.clone(),
            name: display.clone(),
            primary: primary.clone(),
            file_path,
            has_file,
            document,
            roots: resolved_roots,
            issues,
            parse_error,
            persist_error: None,
            settings: WorkspaceSettings::default(),
            settings_parse_errors: Vec::new(),
            settings_issues: Vec::new(),
            leases: vec![self.registry.acquire(&key)],
            opened_ms: now_ms(),
        };

        let settings = self.resolve_settings(&workspace);
        apply_settings(&mut workspace, settings);
        // Whether this open created the workspace or joined one already open is
        // decided under a single write lock rather than by a read-then-write, so
        // two windows opening the same project at the same moment cannot both
        // create it and bind its roots twice.
        let (snapshot, created) = {
            let mut open = self.open.write().unwrap();
            match open.get_mut(&key) {
                Some(existing) => {
                    existing.leases.push(self.registry.acquire(&key));
                    (existing.snapshot(max_roots), false)
                }
                None => {
                    let entry = open.entry(key.clone()).or_insert(workspace);
                    (entry.snapshot(max_roots), true)
                }
            }
        };

        self.counters.opens.fetch_add(1, Ordering::Relaxed);

        if !created {
            // Another window joining an open workspace: its roots are already
            // bound, and rebinding them would start a second watcher for each.
            log_info!(
                self.logger,
                LOG_SOURCE,
                "workspace gained a holder",
                "key" => key.clone(),
                "holders" => snapshot.holders,
            );
            self.emit(WorkspaceEvent {
                kind: WorkspaceEventKind::Opened,
                key,
                workspace: Some(snapshot.clone()),
                roots: snapshot.roots.clone(),
                torn_down: false,
            });
            return Ok(snapshot);
        }

        // Bind the roots that are usable. An unavailable root is simply not
        // bound; the retry loop binds it when it comes back.
        for root in snapshot
            .roots
            .iter()
            .filter(|r| r.availability.is_available())
        {
            self.bind_root(&key, root);
        }

        let unavailable = snapshot
            .roots
            .iter()
            .filter(|root| !root.availability.is_available())
            .count();
        log_info!(
            self.logger,
            LOG_SOURCE,
            "workspace opened",
            "key" => key.clone(),
            "name" => display,
            "roots" => snapshot.roots.len(),
            "unavailable_roots" => unavailable,
            "has_document" => snapshot.has_file,
            "issues" => snapshot.issues.len(),
        );
        if snapshot.at_root_limit {
            log_warn!(
                self.logger,
                LOG_SOURCE,
                "workspace is at its folder limit",
                "key" => key.clone(),
                "max_roots" => max_roots,
            );
        }

        self.record_recent(&snapshot);
        self.emit(WorkspaceEvent {
            kind: WorkspaceEventKind::Opened,
            key,
            workspace: Some(snapshot.clone()),
            roots: snapshot.roots.clone(),
            torn_down: false,
        });
        Ok(snapshot)
    }

    /// Release one holder's reference. The workspace is torn down — watchers,
    /// servers, terminals, and its scoped services — when the last one goes.
    ///
    /// Returns whether this close tore the workspace down.
    pub fn close(&self, key: &str) -> Result<bool, AppError> {
        let max_roots = self.max_roots();
        // The removed workspace is carried out of the lock rather than dropped
        // inside it: dropping it releases its lease, which runs the registry's
        // teardown hooks, and a hook that asked this service a question while
        // the map was still write-locked would deadlock on shutdown — the worst
        // possible place to find that out.
        let (retired, remaining, roots, snapshot) = {
            let mut open = self.open.write().unwrap();
            let Some(workspace) = open.get_mut(key) else {
                return Err(not_open(key));
            };
            workspace.leases.pop();
            if workspace.leases.is_empty() {
                let retired = open.remove(key).expect("checked above");
                let roots = retired.roots.clone();
                (Some(retired), 0, roots, None)
            } else {
                let snapshot = workspace.snapshot(max_roots);
                (None, snapshot.holders, Vec::new(), Some(snapshot))
            }
        };
        let torn_down = retired.is_some();

        if torn_down {
            // Reverse hook order, so a subsystem is unbound before whatever it
            // was layered on.
            for root in roots.iter().rev() {
                self.unbind_root(key, root);
            }
            for hook in self.hooks.read().unwrap().iter().rev() {
                hook.workspace_closed(key);
            }
            log_info!(
                self.logger,
                LOG_SOURCE,
                "workspace closed",
                "key" => key.to_string(),
                "roots_released" => roots.len(),
            );
        } else {
            log_info!(
                self.logger,
                LOG_SOURCE,
                "workspace released one holder",
                "key" => key.to_string(),
                "holders" => remaining,
            );
        }

        self.counters.closes.fetch_add(1, Ordering::Relaxed);
        self.emit(WorkspaceEvent {
            kind: WorkspaceEventKind::Closed,
            key: key.to_string(),
            workspace: snapshot,
            roots,
            torn_down,
        });
        // Last, and outside every lock: this releases the workspace's leases,
        // which ends its scope in the registry and drops its scoped services.
        drop(retired);
        Ok(torn_down)
    }

    // ---- roots -----------------------------------------------------------

    /// Add a root at runtime (REQ-FS-001.4).
    pub fn add_root(
        &self,
        key: &str,
        path: &Path,
        name: Option<&str>,
    ) -> Result<WorkspaceSnapshot, AppError> {
        let max_roots = self.max_roots();
        let resolved = canonical_path(path);

        let (snapshot, added) = {
            let mut open = self.open.write().unwrap();
            let workspace = open.get_mut(key).ok_or_else(|| not_open(key))?;

            if workspace.find(&resolved).is_some() {
                return Err(AppError::permanent(
                    "WORKSPACE_ROOT_EXISTS",
                    format!(
                        "`{}` is already a folder of this workspace",
                        resolved.display()
                    ),
                ));
            }
            if workspace.roots.len() >= max_roots {
                return Err(AppError::permanent(
                    "WORKSPACE_ROOT_LIMIT",
                    format!(
                        "this workspace already has the maximum of {max_roots} folders; \
                         remove one, or raise `{MAX_ROOTS_SETTING}`, before adding another"
                    ),
                ));
            }

            let root = WorkspaceRoot {
                path: resolved.to_string_lossy().to_string(),
                name: name
                    .map(str::to_string)
                    .unwrap_or_else(|| display_name(&resolved)),
                availability: RootAvailability::probe(&resolved),
                primary: false,
            };
            workspace.roots.push(root.clone());
            let settings = self.resolve_settings(workspace);
            apply_settings(workspace, settings);
            (workspace.snapshot(max_roots), root)
        };

        if added.availability.is_available() {
            self.bind_root(key, &added);
        }
        self.counters.roots_added.fetch_add(1, Ordering::Relaxed);
        log_info!(
            self.logger,
            LOG_SOURCE,
            "workspace root added",
            "key" => key.to_string(),
            "root" => added.path.clone(),
            "availability" => added.availability.as_str(),
            "roots" => snapshot.roots.len(),
        );
        if snapshot.at_root_limit {
            log_warn!(
                self.logger,
                LOG_SOURCE,
                "workspace is at its folder limit",
                "key" => key.to_string(),
                "max_roots" => max_roots,
            );
        }

        let snapshot = self.persist(key).unwrap_or(snapshot);
        self.record_recent(&snapshot);
        self.emit(WorkspaceEvent {
            kind: WorkspaceEventKind::RootsChanged,
            key: key.to_string(),
            workspace: Some(snapshot.clone()),
            roots: vec![added],
            torn_down: false,
        });
        Ok(snapshot)
    }

    /// Remove a root at runtime, releasing everything bound to it
    /// (REQ-FS-001.4).
    ///
    /// The last root cannot be removed: a workspace with no folders is a closed
    /// workspace, and closing is a different command with different
    /// consequences.
    pub fn remove_root(&self, key: &str, path: &Path) -> Result<WorkspaceSnapshot, AppError> {
        let max_roots = self.max_roots();
        let resolved = canonical_path(path);

        let (snapshot, removed) = {
            let mut open = self.open.write().unwrap();
            let workspace = open.get_mut(key).ok_or_else(|| not_open(key))?;
            let index = workspace.find(&resolved).ok_or_else(|| {
                AppError::permanent(
                    "WORKSPACE_ROOT_NOT_FOUND",
                    format!("`{}` is not a folder of this workspace", resolved.display()),
                )
            })?;
            if workspace.roots.len() == 1 {
                return Err(AppError::permanent(
                    "WORKSPACE_LAST_ROOT",
                    "the only folder of a workspace cannot be removed; close the workspace instead",
                ));
            }

            let removed = workspace.roots.remove(index);
            // Removing the primary promotes the next root, which is what keeps
            // "the workspace document lives under the primary root" true.
            if removed.primary
                && let Some(first) = workspace.roots.first_mut()
            {
                first.primary = true;
                let primary = first.as_path().to_path_buf();
                workspace.primary = primary.clone();
                workspace.file_path = workspace_file_path_in(&primary);
                workspace.has_file = workspace.file_path.exists();
            }
            let settings = self.resolve_settings(workspace);
            apply_settings(workspace, settings);
            (workspace.snapshot(max_roots), removed)
        };

        self.unbind_root(key, &removed);
        self.counters.roots_removed.fetch_add(1, Ordering::Relaxed);
        log_info!(
            self.logger,
            LOG_SOURCE,
            "workspace root removed",
            "key" => key.to_string(),
            "root" => removed.path.clone(),
            "roots" => snapshot.roots.len(),
        );

        let snapshot = self.persist(key).unwrap_or(snapshot);
        self.emit(WorkspaceEvent {
            kind: WorkspaceEventKind::RootsChanged,
            key: key.to_string(),
            workspace: Some(snapshot.clone()),
            roots: vec![removed],
            torn_down: false,
        });
        Ok(snapshot)
    }

    /// Re-probe every root of every open workspace, binding roots that came
    /// back and unbinding roots that went away.
    ///
    /// This is the periodic retry REQ-FS-001's failure modes ask for. It is
    /// driven by the kernel's run loop, and is also what the file watcher's
    /// deletion events lead to, so a root deleted externally is noticed either
    /// way.
    pub fn refresh_availability(&self) -> Vec<WorkspaceEvent> {
        let max_roots = self.max_roots();
        let mut events = Vec::new();

        let keys: Vec<String> = self.open.read().unwrap().keys().cloned().collect();
        for key in keys {
            let mut changed = Vec::new();
            let snapshot = {
                let mut open = self.open.write().unwrap();
                let Some(workspace) = open.get_mut(&key) else {
                    continue;
                };
                for root in &mut workspace.roots {
                    let now = RootAvailability::probe(root.as_path());
                    if now != root.availability {
                        root.availability = now;
                        changed.push(root.clone());
                    }
                }
                if changed.is_empty() {
                    continue;
                }
                // A root that reappeared may bring settings with it.
                let settings = self.resolve_settings(workspace);
                apply_settings(workspace, settings);
                workspace.snapshot(max_roots)
            };

            for root in &changed {
                if root.availability.is_available() {
                    self.bind_root(&key, root);
                    log_info!(
                        self.logger,
                        LOG_SOURCE,
                        "workspace root is available again",
                        "key" => key.clone(),
                        "root" => root.path.clone(),
                    );
                } else {
                    self.unbind_root(&key, root);
                    log_warn!(
                        self.logger,
                        LOG_SOURCE,
                        "workspace root became unusable",
                        "key" => key.clone(),
                        "root" => root.path.clone(),
                        "availability" => root.availability.as_str(),
                    );
                }
            }

            let event = WorkspaceEvent {
                kind: WorkspaceEventKind::AvailabilityChanged,
                key: key.clone(),
                workspace: Some(snapshot),
                roots: changed,
                torn_down: false,
            };
            self.emit(event.clone());
            events.push(event);
        }
        events
    }

    /// Re-read the settings layers of every open workspace.
    ///
    /// The kernel calls this after a user configuration change or a watched
    /// `.helix/settings.json` edit. Only workspaces whose effective settings
    /// actually moved emit an event, so an unrelated file change is silent.
    pub fn refresh_settings(&self) -> Vec<WorkspaceEvent> {
        let max_roots = self.max_roots();
        let keys: Vec<String> = self.open.read().unwrap().keys().cloned().collect();
        let mut events = Vec::new();

        for key in keys {
            let snapshot = {
                let mut open = self.open.write().unwrap();
                let Some(workspace) = open.get_mut(&key) else {
                    continue;
                };
                let settings = self.resolve_settings(workspace);
                if settings.settings == workspace.settings
                    && settings.parse_errors == workspace.settings_parse_errors
                    && settings.issues == workspace.settings_issues
                {
                    continue;
                }
                apply_settings(workspace, settings);
                workspace.snapshot(max_roots)
            };

            let event = WorkspaceEvent {
                kind: WorkspaceEventKind::SettingsChanged,
                key: key.clone(),
                workspace: Some(snapshot),
                roots: Vec::new(),
                torn_down: false,
            };
            self.emit(event.clone());
            events.push(event);
        }

        events
    }

    /// Re-read one open `.helix/workspace.json` after an external change.
    /// Parse failures update diagnostics while preserving the last valid roots,
    /// settings, and lifecycle bindings.
    pub fn refresh_document(&self, file_path: &Path) -> Option<WorkspaceEvent> {
        let max_roots = self.max_roots();
        let (key, primary, current_id) = self
            .open
            .read()
            .unwrap()
            .iter()
            .find(|(_, workspace)| same_path(&workspace.file_path, file_path))
            .map(|(key, workspace)| {
                (
                    key.clone(),
                    workspace.primary.clone(),
                    workspace.document.id.clone(),
                )
            })?;

        let (mut document, mut issues, parse_error, has_file) =
            self.read_document(file_path, max_roots);
        if parse_error.is_some() {
            self.counters.parse_errors.fetch_add(1, Ordering::Relaxed);
            let snapshot = {
                let mut open = self.open.write().unwrap();
                let workspace = open.get_mut(&key)?;
                if workspace.parse_error == parse_error && workspace.has_file == has_file {
                    return None;
                }
                workspace.parse_error = parse_error;
                workspace.has_file = has_file;
                workspace.snapshot(max_roots)
            };
            let event = WorkspaceEvent {
                kind: WorkspaceEventKind::DocumentChanged,
                key,
                workspace: Some(snapshot),
                roots: Vec::new(),
                torn_down: false,
            };
            self.emit(event.clone());
            return Some(event);
        }

        if document.id != current_id {
            issues.push(WorkspaceIssue::new(
                WorkspaceIssueKind::ChangedId,
                "id",
                "the workspace id cannot change while the workspace is open; the running id was kept",
            ));
            document.id = current_id;
        }

        let roots = resolved_roots(&primary, &document, max_roots);
        let name = document
            .name
            .clone()
            .unwrap_or_else(|| display_name(&primary));
        let (snapshot, removed, added, changed) = {
            let mut open = self.open.write().unwrap();
            let workspace = open.get_mut(&key)?;
            let removed = workspace
                .roots
                .iter()
                .filter(|old| {
                    !roots
                        .iter()
                        .any(|new| same_path(old.as_path(), new.as_path()))
                })
                .cloned()
                .collect::<Vec<_>>();
            let added = roots
                .iter()
                .filter(|new| {
                    !workspace
                        .roots
                        .iter()
                        .any(|old| same_path(old.as_path(), new.as_path()))
                })
                .cloned()
                .collect::<Vec<_>>();

            let previous_document = workspace.document.clone();
            let previous_roots = workspace.roots.clone();
            let previous_issues = workspace.issues.clone();
            let previous_name = workspace.name.clone();
            let previous_settings = workspace.settings.clone();
            workspace.document = document;
            workspace.roots = roots;
            workspace.issues = issues;
            workspace.parse_error = None;
            workspace.has_file = has_file;
            workspace.name = name;
            let settings = self.resolve_settings(workspace);
            apply_settings(workspace, settings);

            let state_changed = workspace.document != previous_document
                || workspace.roots != previous_roots
                || workspace.issues != previous_issues
                || workspace.name != previous_name
                || workspace.settings != previous_settings;
            if !state_changed {
                return None;
            }

            let mut changed = removed.clone();
            changed.extend(added.iter().cloned());
            (workspace.snapshot(max_roots), removed, added, changed)
        };

        for root in removed.iter().rev() {
            if root.availability.is_available() {
                self.unbind_root(&key, root);
            }
        }
        for root in &added {
            if root.availability.is_available() {
                self.bind_root(&key, root);
            }
        }
        self.counters
            .roots_removed
            .fetch_add(removed.len() as u64, Ordering::Relaxed);
        self.counters
            .roots_added
            .fetch_add(added.len() as u64, Ordering::Relaxed);
        self.record_recent(&snapshot);

        let event = WorkspaceEvent {
            kind: WorkspaceEventKind::DocumentChanged,
            key,
            workspace: Some(snapshot),
            roots: changed,
            torn_down: false,
        };
        self.emit(event.clone());
        Some(event)
    }

    // ---- reads -----------------------------------------------------------

    pub fn snapshot(&self, key: &str) -> Option<WorkspaceSnapshot> {
        let max_roots = self.max_roots();
        self.open
            .read()
            .unwrap()
            .get(key)
            .map(|workspace| workspace.snapshot(max_roots))
    }

    /// Every open workspace, in key order.
    pub fn snapshots(&self) -> Vec<WorkspaceSnapshot> {
        let max_roots = self.max_roots();
        self.open
            .read()
            .unwrap()
            .values()
            .map(|workspace| workspace.snapshot(max_roots))
            .collect()
    }

    pub fn is_open(&self, key: &str) -> bool {
        self.open.read().unwrap().contains_key(key)
    }

    /// Build the settings layer paths for an open workspace and optional file.
    pub fn config_paths(&self, key: &str, path: Option<&Path>) -> Result<ConfigPaths, AppError> {
        let open = self.open.read().unwrap();
        let workspace = open.get(key).ok_or_else(|| not_open(key))?;
        let mut paths = ConfigPaths::for_user().with_workspace_root(&workspace.primary);
        if let Some(path) = path {
            let root = workspace
                .settings
                .owning_root(path)
                .ok_or_else(|| path_outside_workspace(key, path))?;
            if !same_path(root, &workspace.primary) {
                paths = paths.with_folder_root(root);
            }
        }
        Ok(paths)
    }

    /// Write a workspace or folder setting through the configuration service's
    /// validation and JSONC persistence path, then refresh the open workspace.
    pub fn set_config(
        &self,
        key: &str,
        path: Option<&Path>,
        scope: ConfigScope,
        setting: &str,
        value: Value,
        language: Option<&str>,
    ) -> Result<(ConfigChange, Option<SettingValue>), AppError> {
        let config = self.scoped_config(key, path, scope)?;
        let change = config.set(scope, setting, value, language)?;
        let effective = config.get(setting, language);
        self.refresh_settings();
        self.emit_config(&change);
        Ok((change, effective))
    }

    /// Reset a workspace or folder setting and refresh its effective cache.
    pub fn reset_config(
        &self,
        key: &str,
        path: Option<&Path>,
        scope: ConfigScope,
        setting: &str,
        language: Option<&str>,
    ) -> Result<(ConfigChange, Option<SettingValue>), AppError> {
        let config = self.scoped_config(key, path, scope)?;
        let change = config.reset(scope, setting, language)?;
        let effective = config.get(setting, language);
        self.refresh_settings();
        self.emit_config(&change);
        Ok((change, effective))
    }

    /// Load the configuration view for an open workspace and optional path.
    pub fn config_view(&self, key: &str, path: Option<&Path>) -> Result<ConfigService, AppError> {
        Ok(ConfigService::load(
            self.config_paths(key, path)?,
            self.config.schema().clone(),
            self.logger.clone(),
        ))
    }

    fn scoped_config(
        &self,
        key: &str,
        path: Option<&Path>,
        scope: ConfigScope,
    ) -> Result<ConfigService, AppError> {
        if !matches!(scope, ConfigScope::Workspace | ConfigScope::Folder) {
            return Err(AppError::permanent(
                "CONFIG_SCOPE_CONTEXT_INVALID",
                format!("the workspace manager cannot write the {scope} layer"),
            ));
        }
        let config = self.config_view(key, path)?;
        if scope == ConfigScope::Folder && config.paths().folder.is_none() {
            return Err(AppError::permanent(
                "CONFIG_FOLDER_REQUIRED",
                "folder-scoped settings need a path owned by a non-primary workspace root",
            ));
        }
        Ok(config)
    }

    /// The root of `key`'s workspace that owns `path`, by longest match.
    pub fn owning_root(&self, key: &str, path: &Path) -> Option<PathBuf> {
        self.open.read().unwrap().get(key).and_then(|workspace| {
            workspace
                .settings
                .owning_root(path)
                .map(|root| root.to_path_buf())
        })
    }

    /// The effective settings tree for a path, with the owning root's folder
    /// settings applied (REQ-FS-001.3).
    pub fn settings_tree(&self, key: &str, path: Option<&Path>) -> Result<Value, AppError> {
        self.settings_tree_for_language(key, path, None)
    }

    /// Effective settings for a path and optional language override.
    pub fn settings_tree_for_language(
        &self,
        key: &str,
        path: Option<&Path>,
        language: Option<&str>,
    ) -> Result<Value, AppError> {
        let open = self.open.read().unwrap();
        let workspace = open.get(key).ok_or_else(|| not_open(key))?;
        Ok(workspace.settings.effective_for_language(path, language))
    }

    /// One setting's effective value for a path.
    pub fn setting_value(
        &self,
        key: &str,
        path: Option<&Path>,
        setting: &str,
    ) -> Result<Option<Value>, AppError> {
        self.setting_value_for_language(key, path, setting, None)
    }

    /// One setting's effective value for a path and optional language.
    pub fn setting_value_for_language(
        &self,
        key: &str,
        path: Option<&Path>,
        setting: &str,
        language: Option<&str>,
    ) -> Result<Option<Value>, AppError> {
        let open = self.open.read().unwrap();
        let workspace = open.get(key).ok_or_else(|| not_open(key))?;
        Ok(workspace
            .settings
            .value_for_language(setting, path, language))
    }

    /// The recent workspace list, most recent first (REQ-FS-001.6).
    pub fn recent(&self) -> Vec<RecentWorkspace> {
        self.recent.read().unwrap().entries.clone()
    }

    /// Drop one workspace from the recent list.
    pub fn forget_recent(&self, key: &str) -> bool {
        let forgotten = self.recent.write().unwrap().forget(key);
        if forgotten {
            self.write_recent();
        }
        forgotten
    }

    /// JSON Schema for the workspace document, for the JSON editor.
    pub fn document_schema(&self) -> Value {
        let mut schema = crate::model::workspace_json_schema(self.max_roots());
        if let Some(properties) = schema
            .as_object_mut()
            .and_then(|schema| schema.get_mut("properties"))
            .and_then(Value::as_object_mut)
        {
            properties.insert("settings".to_string(), self.config.schema().json_schema());
        }
        schema
    }

    pub fn metrics(&self) -> WorkspaceMetrics {
        let open = self.open.read().unwrap();
        WorkspaceMetrics {
            open_workspaces: open.len() as u64,
            unavailable_roots: open
                .values()
                .flat_map(|workspace| workspace.roots.iter())
                .filter(|root| !root.availability.is_available())
                .count() as u64,
            opens: self.counters.opens.load(Ordering::Relaxed),
            closes: self.counters.closes.load(Ordering::Relaxed),
            roots_added: self.counters.roots_added.load(Ordering::Relaxed),
            roots_removed: self.counters.roots_removed.load(Ordering::Relaxed),
            document_writes: self.counters.document_writes.load(Ordering::Relaxed),
            document_write_errors: self.counters.document_write_errors.load(Ordering::Relaxed),
            parse_errors: self.counters.parse_errors.load(Ordering::Relaxed),
            hook_errors: self.counters.hook_errors.load(Ordering::Relaxed),
        }
    }

    // ---- internals -------------------------------------------------------

    /// Read and validate `.helix/workspace.json`.
    ///
    /// Returns an empty document when the file is absent, and an empty document
    /// plus a parse error when it will not parse — the failure mode REQ-FS-001
    /// specifies, which is to open with the available roots and report the
    /// problem rather than refuse.
    fn read_document(
        &self,
        file_path: &Path,
        max_roots: usize,
    ) -> (
        WorkspaceFile,
        Vec<WorkspaceIssue>,
        Option<ConfigParseError>,
        bool,
    ) {
        if !file_path.exists() {
            return (WorkspaceFile::default(), Vec::new(), None, false);
        }
        let body = match self.fs.read(file_path) {
            Ok(content) => content.text.unwrap_or_default(),
            Err(error) => {
                let parse_error = ConfigParseError {
                    path: file_path.to_string_lossy().to_string(),
                    message: format!("the workspace file could not be read: {}", error.message),
                    line: 1,
                    column: 1,
                };
                log_warn!(
                    self.logger,
                    LOG_SOURCE,
                    "the workspace file could not be read",
                    "path" => parse_error.path.clone(),
                    "error" => error.message.clone(),
                );
                return (
                    WorkspaceFile::default(),
                    Vec::new(),
                    Some(parse_error),
                    true,
                );
            }
        };

        // Parsed with the same comment-tolerant parser settings files use: a
        // committed workspace document is exactly the kind of file people leave
        // notes in.
        match helix_config::jsonc::parse_object(&file_path.to_string_lossy(), &body) {
            Ok(raw) => {
                let (document, mut issues) = WorkspaceFile::from_raw(raw, max_roots);
                let (_, setting_issues) = crate::settings::normalize(
                    self.config.schema(),
                    ConfigScope::Workspace,
                    document.settings.clone(),
                );
                issues.extend(setting_issues.into_iter().map(|issue| {
                    let language = issue
                        .language
                        .as_deref()
                        .map(|language| format!("[{language}]."))
                        .unwrap_or_default();
                    WorkspaceIssue::new(
                        WorkspaceIssueKind::InvalidSetting,
                        format!("settings.{language}{}", issue.key),
                        issue.message,
                    )
                }));
                if !issues.is_empty() {
                    log_warn!(
                        self.logger,
                        LOG_SOURCE,
                        "the workspace file has problems; opening with what could be read",
                        "path" => file_path.to_string_lossy().to_string(),
                        "issues" => issues.len(),
                    );
                }
                (document, issues, None, true)
            }
            Err(parse_error) => {
                log_warn!(
                    self.logger,
                    LOG_SOURCE,
                    "the workspace file will not parse; opening on the requested folders",
                    "path" => parse_error.path.clone(),
                    "error" => parse_error.message.clone(),
                    "line" => parse_error.line,
                );
                (
                    WorkspaceFile::default(),
                    Vec::new(),
                    Some(parse_error),
                    true,
                )
            }
        }
    }

    /// Write `.helix/workspace.json` for an open workspace, generating the `id`
    /// if this is its first write (REQ-FS-001.2).
    ///
    /// A write failure is recorded on the snapshot rather than returned: the
    /// in-memory change stands, because a read-only checkout is a workspace a
    /// user still has to be able to work in.
    fn persist(&self, key: &str) -> Option<WorkspaceSnapshot> {
        let max_roots = self.max_roots();
        let (path, body, id_assigned) = {
            let mut open = self.open.write().unwrap();
            let workspace = open.get_mut(key)?;

            let id_assigned = workspace.document.id.is_empty();
            if id_assigned {
                workspace.document.id = generate_id();
            }
            workspace.document.name = Some(workspace.name.clone());
            let primary = workspace.primary.clone();
            workspace.document.folders = workspace
                .roots
                .iter()
                .map(|root| FolderEntry {
                    path: authored_path(&primary, root.as_path()),
                    name: (root.name != display_name(root.as_path())).then(|| root.name.clone()),
                })
                .collect();

            (
                workspace.file_path.clone(),
                workspace.document.to_pretty_json(),
                id_assigned,
            )
        };

        let outcome = self.fs.write(&path, WriteOptions::new(body));
        let written = outcome.is_ok();
        let snapshot = {
            let mut open = self.open.write().unwrap();
            let workspace = open.get_mut(key)?;
            match &outcome {
                Ok(_) => {
                    workspace.has_file = true;
                    workspace.persist_error = None;
                    self.counters
                        .document_writes
                        .fetch_add(1, Ordering::Relaxed);
                }
                Err(error) => {
                    workspace.persist_error = Some(error.message.clone());
                    self.counters
                        .document_write_errors
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
            workspace.snapshot(max_roots)
        };

        match &outcome {
            Ok(_) => {
                log_info!(
                    self.logger,
                    LOG_SOURCE,
                    "workspace file written",
                    "path" => snapshot.file_path.clone(),
                    "id_assigned" => id_assigned,
                    "folders" => snapshot.roots.len(),
                );
            }
            Err(error) => {
                log_warn!(
                    self.logger,
                    LOG_SOURCE,
                    "the workspace file could not be written; the change applies to this \
                     session only",
                    "path" => snapshot.file_path.clone(),
                    "error" => error.message.clone(),
                );
            }
        }

        if written {
            self.emit(WorkspaceEvent {
                kind: WorkspaceEventKind::DocumentWritten,
                key: key.to_string(),
                workspace: Some(snapshot.clone()),
                roots: Vec::new(),
                torn_down: false,
            });
        }
        Some(snapshot)
    }

    /// Resolve the workspace and folder settings layers over the global tree.
    fn resolve_settings(
        &self,
        workspace: &OpenWorkspace,
    ) -> crate::settings::WorkspaceSettingsResolution {
        let base = self.config.snapshot();
        let available: Vec<PathBuf> = workspace
            .roots
            .iter()
            .filter(|root| root.availability.is_available())
            .map(|root| root.as_path().to_path_buf())
            .collect();
        // Unavailable roots are still roots for ownership questions, so they are
        // included in the ordering even though no file can be read from them.
        let all = workspace.root_paths();
        let fs = self.fs.clone();
        WorkspaceSettings::resolve_with_previous(
            self.config.schema(),
            (base.global.clone(), base.languages.clone()),
            &workspace.document.settings,
            &workspace.primary,
            &all,
            Some(&workspace.settings),
            move |path| {
                // A settings file under an unavailable root cannot be read, and
                // trying costs a filesystem timeout per resolution on a dropped
                // share.
                if !available.iter().any(|root| path.starts_with(root)) {
                    return None;
                }
                fs.read(path).ok().and_then(|content| content.text)
            },
        )
    }

    fn bind_root(&self, key: &str, root: &WorkspaceRoot) {
        let event = RootEvent {
            key,
            root: root.as_path(),
            availability: root.availability,
        };
        for hook in self.hooks.read().unwrap().iter() {
            if let Err(error) = hook.root_opened(&event) {
                self.counters.hook_errors.fetch_add(1, Ordering::Relaxed);
                log_warn!(
                    self.logger,
                    LOG_SOURCE,
                    "a workspace hook failed to bind a root; the workspace is open regardless",
                    "hook" => hook.name(),
                    "key" => key.to_string(),
                    "root" => root.path.clone(),
                    "error" => error.message.clone(),
                );
            }
        }
    }

    fn unbind_root(&self, key: &str, root: &WorkspaceRoot) {
        let event = RootEvent {
            key,
            root: root.as_path(),
            availability: root.availability,
        };
        for hook in self.hooks.read().unwrap().iter().rev() {
            hook.root_closed(&event);
        }
    }

    fn record_recent(&self, snapshot: &WorkspaceSnapshot) {
        let roots: Vec<PathBuf> = snapshot
            .roots
            .iter()
            .map(|root| root.as_path().to_path_buf())
            .collect();
        self.recent
            .write()
            .unwrap()
            .record(&snapshot.key, &snapshot.name, &roots);
        self.write_recent();
    }

    /// Persist the recent list.
    ///
    /// Written atomically, and a failure is logged rather than surfaced: a most
    /// recently used list that could not be saved is not worth interrupting
    /// anyone over.
    fn write_recent(&self) {
        let Some(path) = self.recent_path.clone() else {
            return;
        };
        let body = self.recent.read().unwrap().to_pretty_json();
        if let Err(error) = self.fs.write(&path, WriteOptions::new(body)) {
            log_warn!(
                self.logger,
                LOG_SOURCE,
                "the recent workspace list could not be saved",
                "path" => path.to_string_lossy().to_string(),
                "error" => error.message.clone(),
            );
        }
    }

    fn emit(&self, event: WorkspaceEvent) {
        for listener in self.listeners.read().unwrap().iter() {
            listener(&event);
        }
    }

    fn emit_config(&self, change: &ConfigChange) {
        if !change.is_meaningful() {
            return;
        }
        for listener in self.config_listeners.read().unwrap().iter() {
            listener(change);
        }
    }
}

fn push_unique(paths: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !paths.iter().any(|existing| same_path(existing, &candidate)) {
        paths.push(candidate);
    }
}

fn apply_settings(
    workspace: &mut OpenWorkspace,
    resolution: crate::settings::WorkspaceSettingsResolution,
) {
    workspace.settings = resolution.settings;
    workspace.settings_parse_errors = resolution.parse_errors;
    workspace.settings_issues = resolution.issues;
}

fn resolved_roots(
    primary: &Path,
    document: &WorkspaceFile,
    max_roots: usize,
) -> Vec<WorkspaceRoot> {
    let mut paths = vec![primary.to_path_buf()];
    for folder in document.resolved_folders(primary) {
        push_unique(&mut paths, canonical_path(&folder));
        if paths.len() >= max_roots {
            break;
        }
    }
    paths
        .into_iter()
        .enumerate()
        .map(|(index, path)| WorkspaceRoot {
            name: document
                .name_for(primary, &path)
                .unwrap_or_else(|| display_name(&path)),
            availability: RootAvailability::probe(&path),
            path: path.to_string_lossy().to_string(),
            primary: index == 0,
        })
        .collect()
}

/// Display name for a root: its final segment, or the whole path for a
/// filesystem root such as `C:\` that has no final segment to show.
fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

fn not_open(key: &str) -> AppError {
    AppError::permanent(
        "WORKSPACE_NOT_OPEN",
        format!("no open workspace has the key `{key}`"),
    )
}

fn path_outside_workspace(key: &str, path: &Path) -> AppError {
    AppError::permanent(
        "WORKSPACE_PATH_OUTSIDE_ROOTS",
        format!(
            "{} is not inside any root of workspace '{key}'",
            path.display()
        ),
    )
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Comparison key of a path, re-exported for consumers that key their own
/// per-root state the same way this service does.
pub fn root_key(path: &Path) -> String {
    comparison_key(path)
}
