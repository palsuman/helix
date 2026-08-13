//! Storage backends for credentials.

use std::path::{Path, PathBuf};

use crate::error::SecretError;

pub const KEYRING_SERVICE: &str = "dev.helix.ide";

/// Metadata returned by list operations — never includes secret values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretEntry {
    pub namespace: String,
    pub name: String,
}

impl SecretEntry {
    pub fn qualified_name(&self) -> String {
        format!("{}/{}", self.namespace, self.name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Keychain,
    EncryptedFile,
    Memory,
}

/// Low-level secret persistence.
pub trait SecretBackend: Send + Sync {
    fn kind(&self) -> BackendKind;

    fn store(&self, namespace: &str, name: &str, value: &str) -> Result<(), SecretError>;

    fn get(&self, namespace: &str, name: &str) -> Result<String, SecretError>;

    fn delete(&self, namespace: &str, name: &str) -> Result<(), SecretError>;

    fn list(&self, namespace: Option<&str>) -> Result<Vec<SecretEntry>, SecretError>;
}

/// In-memory backend for tests and deterministic CI.
#[derive(Default)]
pub struct MemoryBackend {
    entries: std::sync::Mutex<std::collections::BTreeMap<(String, String), String>>,
}

impl MemoryBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretBackend for MemoryBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Memory
    }

    fn store(&self, namespace: &str, name: &str, value: &str) -> Result<(), SecretError> {
        self.entries
            .lock()
            .unwrap()
            .insert((namespace.to_string(), name.to_string()), value.to_string());
        Ok(())
    }

    fn get(&self, namespace: &str, name: &str) -> Result<String, SecretError> {
        self.entries
            .lock()
            .unwrap()
            .get(&(namespace.to_string(), name.to_string()))
            .cloned()
            .ok_or_else(|| SecretError::NotFound {
                namespace: namespace.to_string(),
                name: name.to_string(),
            })
    }

    fn delete(&self, namespace: &str, name: &str) -> Result<(), SecretError> {
        self.entries
            .lock()
            .unwrap()
            .remove(&(namespace.to_string(), name.to_string()));
        Ok(())
    }

    fn list(&self, namespace: Option<&str>) -> Result<Vec<SecretEntry>, SecretError> {
        let entries = self.entries.lock().unwrap();
        Ok(entries
            .keys()
            .filter(|(ns, _)| namespace.is_none_or(|wanted| wanted == ns))
            .map(|(ns, name)| SecretEntry {
                namespace: ns.clone(),
                name: name.clone(),
            })
            .collect())
    }
}

/// OS keychain via the `keyring` crate (REQ-SEC-002.1).
pub struct KeyringBackend;

impl Default for KeyringBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyringBackend {
    pub fn new() -> Self {
        Self
    }

    fn entry(namespace: &str, name: &str) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(KEYRING_SERVICE, &format!("{namespace}/{name}"))
            .map_err(|error| SecretError::storage(error.to_string()))
    }

    fn map_error(namespace: &str, name: &str, error: keyring::Error) -> SecretError {
        match error {
            keyring::Error::NoEntry => SecretError::NotFound {
                namespace: namespace.to_string(),
                name: name.to_string(),
            },
            keyring::Error::NoStorageAccess(_)
            | keyring::Error::PlatformFailure(_)
            | keyring::Error::NoDefaultStore
            | keyring::Error::NotSupportedByStore(_) => SecretError::KeychainUnavailable {
                reason: error.to_string(),
            },
            other => SecretError::storage(other.to_string()),
        }
    }
}

impl SecretBackend for KeyringBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Keychain
    }

    fn store(&self, namespace: &str, name: &str, value: &str) -> Result<(), SecretError> {
        Self::entry(namespace, name)?
            .set_password(value)
            .map_err(|error| Self::map_error(namespace, name, error))
    }

    fn get(&self, namespace: &str, name: &str) -> Result<String, SecretError> {
        Self::entry(namespace, name)?
            .get_password()
            .map_err(|error| Self::map_error(namespace, name, error))
    }

    fn delete(&self, namespace: &str, name: &str) -> Result<(), SecretError> {
        match Self::entry(namespace, name)?.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(Self::map_error(namespace, name, error)),
        }
    }

    fn list(&self, namespace: Option<&str>) -> Result<Vec<SecretEntry>, SecretError> {
        let _ = namespace;
        // Platform keychains do not expose a portable list API; the service
        // maintains a side index for fallback mode and returns an empty list
        // here. Callers that need inventory use the encrypted index when the
        // fallback backend is active.
        Ok(Vec::new())
    }
}

/// Chooses keychain first, falling back to the encrypted file store when the
/// OS credential service is unavailable (REQ-SEC-002 failure modes).
pub struct CompositeBackend {
    keyring: KeyringBackend,
    fallback: crate::fallback::EncryptedFileBackend,
    prefer_fallback: std::sync::atomic::AtomicBool,
}

impl CompositeBackend {
    pub fn new(fallback_path: PathBuf) -> Self {
        Self {
            keyring: KeyringBackend::new(),
            fallback: crate::fallback::EncryptedFileBackend::new(fallback_path),
            prefer_fallback: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn with_memory_fallback() -> Self {
        let dir = std::env::temp_dir().join(format!("helix-secrets-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        Self::new(dir.join("vault.json"))
    }

    pub fn fallback_path(&self) -> &Path {
        self.fallback.path()
    }

    pub fn unlock_fallback(&self, master_password: &str) -> Result<(), SecretError> {
        self.fallback.unlock(master_password)
    }

    pub fn is_fallback_unlocked(&self) -> bool {
        self.fallback.is_unlocked()
    }

    pub fn fallback_load_error(&self) -> Option<SecretError> {
        self.fallback.load_error()
    }

    pub fn active_kind(&self) -> BackendKind {
        if self
            .prefer_fallback
            .load(std::sync::atomic::Ordering::Acquire)
            && self.fallback.is_unlocked()
        {
            BackendKind::EncryptedFile
        } else {
            BackendKind::Keychain
        }
    }

    fn write(&self, namespace: &str, name: &str, value: &str) -> Result<(), SecretError> {
        if self
            .prefer_fallback
            .load(std::sync::atomic::Ordering::Acquire)
            && self.fallback.is_unlocked()
        {
            return self.fallback.store(namespace, name, value);
        }
        match self.keyring.store(namespace, name, value) {
            Ok(()) => {
                if let Err(error) = self.fallback.record_index(namespace, name) {
                    // A successful store must also be listable. Roll back the
                    // credential if its metadata cannot be committed.
                    let _ = self.keyring.delete(namespace, name);
                    return Err(error);
                }
                Ok(())
            }
            Err(SecretError::KeychainUnavailable { .. }) if self.fallback.is_unlocked() => {
                self.prefer_fallback
                    .store(true, std::sync::atomic::Ordering::Release);
                self.fallback.store(namespace, name, value)
            }
            Err(SecretError::KeychainUnavailable { reason }) => {
                if self.fallback.is_unlocked() {
                    self.prefer_fallback
                        .store(true, std::sync::atomic::Ordering::Release);
                    self.fallback.store(namespace, name, value)
                } else {
                    Err(SecretError::KeychainUnavailable { reason })
                }
            }
            other => other,
        }
    }

    fn read(&self, namespace: &str, name: &str) -> Result<String, SecretError> {
        if self
            .prefer_fallback
            .load(std::sync::atomic::Ordering::Acquire)
            && self.fallback.is_unlocked()
        {
            return self.fallback.get(namespace, name);
        }
        match self.keyring.get(namespace, name) {
            Ok(value) => Ok(value),
            Err(SecretError::KeychainUnavailable { .. }) if self.fallback.is_unlocked() => {
                let value = self.fallback.get(namespace, name)?;
                self.prefer_fallback
                    .store(true, std::sync::atomic::Ordering::Release);
                Ok(value)
            }
            Err(SecretError::NotFound { .. }) if self.fallback.is_unlocked() => {
                let value = self.fallback.get(namespace, name)?;
                self.prefer_fallback
                    .store(true, std::sync::atomic::Ordering::Release);
                Ok(value)
            }
            Err(error @ SecretError::NotFound { .. }) => {
                if self.fallback.contains_index(namespace, name)? {
                    Err(SecretError::FallbackLocked)
                } else {
                    Err(error)
                }
            }
            other => other,
        }
    }
}

impl SecretBackend for CompositeBackend {
    fn kind(&self) -> BackendKind {
        self.active_kind()
    }

    fn store(&self, namespace: &str, name: &str, value: &str) -> Result<(), SecretError> {
        self.write(namespace, name, value)
    }

    fn get(&self, namespace: &str, name: &str) -> Result<String, SecretError> {
        self.read(namespace, name)
    }

    fn delete(&self, namespace: &str, name: &str) -> Result<(), SecretError> {
        let keyring_result = self.keyring.delete(namespace, name);
        let fallback_result = self.fallback.delete(namespace, name);
        fallback_result?;
        match keyring_result {
            Ok(()) => Ok(()),
            Err(SecretError::KeychainUnavailable { .. })
                if self
                    .prefer_fallback
                    .load(std::sync::atomic::Ordering::Acquire) =>
            {
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    fn list(&self, namespace: Option<&str>) -> Result<Vec<SecretEntry>, SecretError> {
        self.fallback.list(namespace)
    }
}

/// Default encrypted vault location under the Helix user data directory.
pub fn default_vault_path() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|p| p.join("Helix"))
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|p| p.join("Library").join("Application Support").join("Helix"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|p| p.join(".local").join("share"))
            })
            .map(|p| p.join("helix"))
    };
    base.map(|p| p.join("secrets").join("vault.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_backend_round_trips_a_secret() {
        let backend = MemoryBackend::new();
        backend.store("helix", "openai.work", "sk-test").unwrap();
        assert_eq!(backend.get("helix", "openai.work").unwrap(), "sk-test");
        let listed = backend.list(Some("helix")).unwrap();
        assert_eq!(listed.len(), 1);
        backend.delete("helix", "openai.work").unwrap();
        assert!(backend.get("helix", "openai.work").is_err());
    }

    #[test]
    fn unlocking_fallback_does_not_hide_a_healthy_keychain() {
        let dir = tempfile::tempdir().unwrap();
        let backend = CompositeBackend::new(dir.path().join("vault.json"));
        backend.unlock_fallback("master-password").unwrap();
        assert!(backend.is_fallback_unlocked());
        assert_eq!(backend.active_kind(), BackendKind::Keychain);
    }
}
