//! Workspace trust service (REQ-FS-005).

use std::path::Path;
use std::sync::{Arc, RwLock};

use helix_log::Logger;

use crate::error::TrustError;
use crate::gate::{ManagedProcess, ProcessRegistry};
use crate::model::{RootTrustStatus, TrustCapability, TrustDecision, WorkspaceTrustMode};
use crate::store::{StoreHealth, TrustStore, default_store_path};

pub const LOG_SOURCE: &str = "kernel.trust";
pub const CHANNEL: &str = "trust:changed";

pub struct TrustService {
    store: Arc<TrustStore>,
    processes: Arc<ProcessRegistry>,
    logger: Option<Arc<Logger>>,
    listeners: RwLock<Vec<Box<dyn Fn() + Send + Sync>>>,
}

impl TrustService {
    pub fn new(store: Arc<TrustStore>, logger: Option<Arc<Logger>>) -> Self {
        Self {
            store,
            processes: Arc::new(ProcessRegistry::default()),
            logger,
            listeners: RwLock::new(Vec::new()),
        }
    }

    pub fn with_default_store(logger: Option<Arc<Logger>>) -> Self {
        let path =
            default_store_path().unwrap_or_else(|| std::env::temp_dir().join("helix-trust.json"));
        Self::new(Arc::new(TrustStore::load(path)), logger)
    }

    pub fn in_memory() -> Self {
        Self::new(Arc::new(TrustStore::in_memory()), None)
    }

    pub fn is_enabled(&self) -> bool {
        true
    }

    pub fn store_health(&self) -> StoreHealth {
        self.store.health()
    }

    pub fn trust_everything(&self) -> bool {
        self.store.health() == StoreHealth::Healthy && self.store.trust_everything()
    }

    pub fn add_listener(&self, listener: impl Fn() + Send + Sync + 'static) {
        self.listeners.write().unwrap().push(Box::new(listener));
    }

    pub fn root_status(&self, path: &Path) -> RootTrustStatus {
        let (decision, inherited_from) = self.resolve(path);
        RootTrustStatus {
            path: canonical_display(path),
            decision,
            inherited_from,
        }
    }

    pub fn workspace_mode<P, I>(&self, roots: I) -> WorkspaceTrustMode
    where
        P: AsRef<Path>,
        I: IntoIterator<Item = P>,
    {
        if self.store.health() == StoreHealth::Unavailable {
            return WorkspaceTrustMode::Restricted;
        }
        if self.store.trust_everything() {
            return WorkspaceTrustMode::Trusted;
        }
        for root in roots {
            if !self.is_path_trusted(root.as_ref()) {
                return WorkspaceTrustMode::Restricted;
            }
        }
        WorkspaceTrustMode::Trusted
    }

    pub fn pending_prompts<P, I>(&self, roots: I) -> Vec<String>
    where
        P: AsRef<Path>,
        I: IntoIterator<Item = P>,
    {
        if self.store.trust_everything() {
            return Vec::new();
        }
        roots
            .into_iter()
            .filter(|root| {
                matches!(
                    self.store.resolve(root.as_ref()),
                    (TrustDecision::Unknown, _)
                )
            })
            .map(|root| canonical_display(root.as_ref()))
            .collect()
    }

    pub fn set_decision(
        &self,
        path: &Path,
        decision: TrustDecision,
        inherit_to_children: bool,
    ) -> Result<(), TrustError> {
        let persisted = self.store.set_entry(path, decision, inherit_to_children);
        if matches!(decision, TrustDecision::Restricted) {
            self.processes.terminate_for_path(path);
        }
        persisted?;
        self.notify_changed();
        Ok(())
    }

    pub fn trust(&self, path: &Path, inherit_to_children: bool) -> Result<(), TrustError> {
        self.set_decision(path, TrustDecision::Trusted, inherit_to_children)
    }

    pub fn restrict(&self, path: &Path) -> Result<(), TrustError> {
        self.set_decision(path, TrustDecision::Restricted, false)
    }

    pub fn revoke(&self, path: &Path) -> Result<Vec<ManagedProcess>, TrustError> {
        let terminated = self.processes.terminate_for_path(path);
        self.store.remove_entry(path)?;
        self.notify_changed();
        Ok(terminated)
    }

    pub fn set_trust_everything(
        &self,
        enabled: bool,
        acknowledged_warning: bool,
    ) -> Result<(), TrustError> {
        if enabled && !acknowledged_warning {
            return Err(TrustError::WarningNotAcknowledged);
        }
        let persisted = self.store.set_trust_everything(enabled);
        if !enabled {
            self.processes.terminate_all();
        }
        persisted?;
        self.notify_changed();
        Ok(())
    }

    pub fn list_trusted(&self) -> Vec<(String, bool)> {
        self.store
            .entries()
            .into_iter()
            .filter(|(_, entry)| entry.decision.is_trusted())
            .map(|(path, entry)| (path, entry.inherit_to_children))
            .collect()
    }

    pub fn require(&self, path: &Path, capability: TrustCapability) -> Result<(), TrustError> {
        if self.store.health() == StoreHealth::Unavailable {
            return Err(TrustError::StoreUnavailable);
        }
        if self.store.trust_everything() {
            return Ok(());
        }
        if self.is_path_trusted(path) {
            return Ok(());
        }
        Err(TrustError::Restricted {
            path: canonical_display(path),
            capability,
        })
    }

    pub fn register_launch_for(
        &self,
        path: &Path,
        capability: TrustCapability,
        label: impl Into<String>,
        terminate: impl Fn() + Send + Sync + 'static,
    ) -> Result<u64, TrustError> {
        self.require(path, capability)?;
        Ok(self
            .processes
            .register_with_terminator(path, label, terminate))
    }

    pub fn setting_key_requires_trust(key: &str) -> bool {
        let lower = key.to_ascii_lowercase();
        lower.ends_with("path")
            || lower.ends_with(".command")
            || lower.contains("executable")
            || lower.contains("shellpath")
            || lower.contains("interpreter")
    }

    pub fn require_setting(&self, path: &Path, key: &str) -> Result<(), TrustError> {
        if Self::setting_key_requires_trust(key) {
            self.require(path, TrustCapability::ExecutablePathSetting)
        } else {
            Ok(())
        }
    }

    pub fn active_processes(&self, path: &Path) -> Vec<ManagedProcess> {
        self.processes.active_for_path(path)
    }

    /// Remove a process that exited normally without invoking its revocation
    /// callback.
    pub fn unregister_launch(&self, id: u64) -> bool {
        self.processes.unregister(id)
    }

    fn is_path_trusted(&self, path: &Path) -> bool {
        matches!(self.resolve(path), (TrustDecision::Trusted, _))
    }

    fn resolve(&self, path: &Path) -> (TrustDecision, Option<String>) {
        let (decision, inherited) = self.store.resolve(path);
        match decision {
            TrustDecision::Unknown => (TrustDecision::Restricted, inherited),
            other => (other, inherited),
        }
    }

    fn notify_changed(&self) {
        if let Some(logger) = &self.logger {
            helix_log::log_info!(logger, LOG_SOURCE, "workspace trust changed");
        }
        for listener in self.listeners.read().unwrap().iter() {
            listener();
        }
    }
}

fn canonical_display(path: &Path) -> String {
    helix_workspace::identity::canonical_path(path)
        .to_string_lossy()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn each_blocked_capability_refuses_in_restricted_mode() {
        let service = TrustService::in_memory();
        let path = Path::new("/tmp/repo");
        for capability in [
            TrustCapability::LanguageServerLaunch,
            TrustCapability::DebugAdapterLaunch,
            TrustCapability::TaskExecution,
            TrustCapability::TaskAutoDetection,
            TrustCapability::McpServerLaunch,
            TrustCapability::WorkspaceFormatter,
            TrustCapability::WorkspacePluginActivation,
            TrustCapability::AgentExecution,
            TrustCapability::ExecutablePathSetting,
        ] {
            assert!(
                service.require(path, capability).is_err(),
                "{capability:?} should be blocked"
            );
        }
    }

    #[test]
    fn unknown_paths_are_restricted_until_trusted() {
        let service = TrustService::in_memory();
        assert!(
            service
                .require(Path::new("/tmp/repo"), TrustCapability::TaskExecution)
                .is_err()
        );
        service.trust(Path::new("/tmp/repo"), true).unwrap();
        assert!(
            service
                .require(Path::new("/tmp/repo"), TrustCapability::TaskExecution)
                .is_ok()
        );
    }

    #[test]
    fn unknown_paths_prompt_once_and_restricted_choices_are_remembered() {
        let service = TrustService::in_memory();
        let path = Path::new("/tmp/unfamiliar");
        assert_eq!(
            service.pending_prompts([path]),
            vec![canonical_display(path)]
        );

        service.restrict(path).unwrap();
        assert!(service.pending_prompts([path]).is_empty());
        assert_eq!(
            service.root_status(path).decision,
            TrustDecision::Restricted
        );
    }

    #[test]
    fn one_untrusted_root_restricts_the_workspace() {
        let service = TrustService::in_memory();
        service.trust(Path::new("/tmp/a"), false).unwrap();
        assert_eq!(
            service.workspace_mode([Path::new("/tmp/a"), Path::new("/tmp/b")]),
            WorkspaceTrustMode::Restricted
        );
    }

    #[test]
    fn revocation_terminates_registered_processes() {
        let service = TrustService::in_memory();
        service.trust(Path::new("/tmp/repo"), true).unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let stopped_by_hook = stopped.clone();
        let id = service
            .register_launch_for(
                Path::new("/tmp/repo/packages/app"),
                TrustCapability::LanguageServerLaunch,
                "typescript-lsp",
                move || stopped_by_hook.store(true, Ordering::SeqCst),
            )
            .unwrap();
        assert!(id > 0);
        let terminated = service.revoke(Path::new("/tmp/repo")).unwrap();
        assert_eq!(terminated.len(), 1);
        assert!(stopped.load(Ordering::SeqCst));
    }
}
