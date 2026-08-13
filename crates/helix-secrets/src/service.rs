//! The secret management service (Task 1.12, REQ-SEC-002).

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use helix_log::Logger;

use crate::backend::{BackendKind, CompositeBackend, MemoryBackend, SecretBackend, SecretEntry};
use crate::error::SecretError;
use crate::namespace::{HELIX_NAMESPACE, SecretCaller, validate_name, validate_namespace};

pub const LOG_SOURCE: &str = "kernel.secrets";

enum StoreBackend {
    Composite(Arc<CompositeBackend>),
    Memory(Arc<MemoryBackend>),
}

impl StoreBackend {
    fn kind(&self) -> BackendKind {
        match self {
            Self::Composite(backend) => backend.kind(),
            Self::Memory(backend) => backend.kind(),
        }
    }

    fn store(&self, namespace: &str, name: &str, value: &str) -> Result<(), SecretError> {
        match self {
            Self::Composite(backend) => backend.store(namespace, name, value),
            Self::Memory(backend) => backend.store(namespace, name, value),
        }
    }

    fn get(&self, namespace: &str, name: &str) -> Result<String, SecretError> {
        match self {
            Self::Composite(backend) => backend.get(namespace, name),
            Self::Memory(backend) => backend.get(namespace, name),
        }
    }

    fn delete(&self, namespace: &str, name: &str) -> Result<(), SecretError> {
        match self {
            Self::Composite(backend) => backend.delete(namespace, name),
            Self::Memory(backend) => backend.delete(namespace, name),
        }
    }

    fn list(&self, namespace: Option<&str>) -> Result<Vec<SecretEntry>, SecretError> {
        match self {
            Self::Composite(backend) => backend.list(namespace),
            Self::Memory(backend) => backend.list(namespace),
        }
    }

    fn unlock_fallback(&self, master_password: &str) -> Result<(), SecretError> {
        match self {
            Self::Composite(backend) => backend.unlock_fallback(master_password),
            Self::Memory(_) => Err(SecretError::storage(
                "fallback unlock is unavailable in memory mode",
            )),
        }
    }

    fn fallback_unlocked(&self) -> bool {
        match self {
            Self::Composite(backend) => backend.is_fallback_unlocked(),
            Self::Memory(_) => false,
        }
    }

    fn storage_error(&self) -> Option<SecretError> {
        match self {
            Self::Composite(backend) => backend.fallback_load_error(),
            Self::Memory(_) => None,
        }
    }
}

/// Kernel-facing secret store with namespace isolation and log redaction.
pub struct SecretService {
    backend: StoreBackend,
    logger: Option<Arc<Logger>>,
    loaded: RwLock<BTreeMap<(String, String), String>>,
}

impl SecretService {
    pub fn with_composite(logger: Arc<Logger>, vault_path: std::path::PathBuf) -> Self {
        Self {
            backend: StoreBackend::Composite(Arc::new(CompositeBackend::new(vault_path))),
            logger: Some(logger),
            loaded: RwLock::new(BTreeMap::new()),
        }
    }

    pub fn in_memory() -> Self {
        Self {
            backend: StoreBackend::Memory(Arc::new(MemoryBackend::new())),
            logger: None,
            loaded: RwLock::new(BTreeMap::new()),
        }
    }

    pub fn with_memory_logger(logger: Arc<Logger>) -> Self {
        Self {
            backend: StoreBackend::Memory(Arc::new(MemoryBackend::new())),
            logger: Some(logger),
            loaded: RwLock::new(BTreeMap::new()),
        }
    }

    pub fn backend_kind(&self) -> BackendKind {
        self.backend.kind()
    }

    pub fn unlock_fallback(&self, master_password: &str) -> Result<(), SecretError> {
        self.backend.unlock_fallback(master_password)
    }

    pub fn fallback_unlocked(&self) -> bool {
        self.backend.fallback_unlocked()
    }

    pub fn storage_error(&self) -> Option<SecretError> {
        self.backend.storage_error()
    }

    pub fn store(
        &self,
        caller: &SecretCaller,
        namespace: &str,
        name: &str,
        value: &str,
    ) -> Result<(), SecretError> {
        validate_namespace(namespace)?;
        validate_name(name)?;
        caller.deny_if_needed(namespace)?;
        if value.is_empty() {
            return Err(SecretError::storage("secret value must not be empty"));
        }
        self.backend.store(namespace, name, value)?;
        self.track_redaction(namespace, name, value);
        Ok(())
    }

    /// Load a secret for kernel-side consumers. Never exposed over IPC.
    pub fn get(
        &self,
        caller: &SecretCaller,
        namespace: &str,
        name: &str,
    ) -> Result<String, SecretError> {
        validate_namespace(namespace)?;
        validate_name(name)?;
        caller.deny_if_needed(namespace)?;
        let value = self.backend.get(namespace, name)?;
        self.track_redaction(namespace, name, &value);
        Ok(value)
    }

    pub fn delete(
        &self,
        caller: &SecretCaller,
        namespace: &str,
        name: &str,
    ) -> Result<(), SecretError> {
        validate_namespace(namespace)?;
        validate_name(name)?;
        caller.deny_if_needed(namespace)?;
        self.backend.delete(namespace, name)?;
        self.untrack_redaction(namespace, name);
        Ok(())
    }

    pub fn list(
        &self,
        caller: &SecretCaller,
        namespace: Option<&str>,
    ) -> Result<Vec<SecretEntry>, SecretError> {
        if let Some(ns) = namespace {
            validate_namespace(ns)?;
            caller.deny_if_needed(ns)?;
        } else {
            match caller {
                SecretCaller::Kernel => {}
                SecretCaller::SettingsUi => {
                    return self.backend.list(Some(HELIX_NAMESPACE));
                }
                SecretCaller::Plugin { plugin_id } => {
                    return self.backend.list(Some(&format!("plugin.{plugin_id}")));
                }
            }
        }
        self.backend.list(namespace)
    }

    pub fn exists(
        &self,
        caller: &SecretCaller,
        namespace: &str,
        name: &str,
    ) -> Result<bool, SecretError> {
        match self.get(caller, namespace, name) {
            Ok(_) => Ok(true),
            Err(SecretError::NotFound { .. }) => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Resolve a provider credential referenced from `ai.providers` settings.
    pub fn resolve_provider_key(&self, key_id: &str) -> Result<String, SecretError> {
        self.get(&SecretCaller::Kernel, HELIX_NAMESPACE, key_id)
    }

    fn track_redaction(&self, namespace: &str, name: &str, value: &str) {
        if let Some(logger) = &self.logger {
            let mut loaded = self.loaded.write().unwrap();
            let key = (namespace.to_string(), name.to_string());
            if loaded.get(&key).is_some_and(|existing| existing == value) {
                return;
            }
            let value_was_loaded = loaded.values().any(|existing| existing == value);
            let previous = loaded.insert(key, value.to_string());
            if !value_was_loaded {
                logger.register_secret(value);
            }
            if let Some(previous) = previous
                && previous != value
                && !loaded.values().any(|existing| existing == &previous)
            {
                logger.forget_secret(&previous);
            }
        }
    }

    fn untrack_redaction(&self, namespace: &str, name: &str) {
        if let Some(logger) = &self.logger {
            let mut loaded = self.loaded.write().unwrap();
            let key = (namespace.to_string(), name.to_string());
            if let Some(previous) = loaded.remove(&key)
                && !loaded.values().any(|existing| existing == &previous)
            {
                logger.forget_secret(&previous);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::namespace::GIT_NAMESPACE;

    #[test]
    fn namespace_isolation_blocks_cross_plugin_reads() {
        let service = SecretService::in_memory();
        service
            .store(
                &SecretCaller::Plugin {
                    plugin_id: "acme".into(),
                },
                "plugin.acme",
                "token",
                "secret-one",
            )
            .unwrap();
        let denied = service.get(
            &SecretCaller::Plugin {
                plugin_id: "other".into(),
            },
            "plugin.acme",
            "token",
        );
        assert!(matches!(denied, Err(SecretError::NamespaceDenied { .. })));
    }

    #[test]
    fn provider_keys_resolve_from_the_helix_namespace() {
        let service = SecretService::in_memory();
        service
            .store(
                &SecretCaller::SettingsUi,
                HELIX_NAMESPACE,
                "openai.work",
                "sk-provider-key",
            )
            .unwrap();
        assert_eq!(
            service.resolve_provider_key("openai.work").unwrap(),
            "sk-provider-key"
        );
    }

    #[test]
    fn list_never_returns_values() {
        let service = SecretService::in_memory();
        service
            .store(
                &SecretCaller::SettingsUi,
                GIT_NAMESPACE,
                "github.com",
                "pat-1234567890",
            )
            .unwrap();
        let entries = service
            .list(&SecretCaller::SettingsUi, Some(GIT_NAMESPACE))
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "github.com");
    }

    #[test]
    fn rotation_updates_redaction_only_after_storage_succeeds() {
        let logger = Arc::new(Logger::in_memory(helix_log::LogLevel::Trace));
        let service = SecretService::with_memory_logger(logger.clone());
        service
            .store(
                &SecretCaller::SettingsUi,
                HELIX_NAMESPACE,
                "provider",
                "old-secret-value",
            )
            .unwrap();
        service
            .store(
                &SecretCaller::SettingsUi,
                HELIX_NAMESPACE,
                "provider",
                "new-secret-value",
            )
            .unwrap();
        assert_eq!(
            logger
                .redactor()
                .redact_text("old-secret-value new-secret-value"),
            format!("old-secret-value {}", helix_log::REDACTED)
        );
    }

    #[test]
    fn deleting_one_duplicate_value_keeps_the_other_redacted() {
        let logger = Arc::new(Logger::in_memory(helix_log::LogLevel::Trace));
        let service = SecretService::with_memory_logger(logger.clone());
        for name in ["one", "two"] {
            service
                .store(
                    &SecretCaller::SettingsUi,
                    HELIX_NAMESPACE,
                    name,
                    "shared-secret-value",
                )
                .unwrap();
        }
        service
            .delete(&SecretCaller::SettingsUi, HELIX_NAMESPACE, "one")
            .unwrap();
        assert_eq!(
            logger.redactor().redact_text("shared-secret-value"),
            helix_log::REDACTED
        );
    }
}
