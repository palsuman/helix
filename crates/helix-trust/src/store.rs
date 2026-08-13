//! Durable trust store in user data, never inside the workspace (REQ-FS-005.8).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};

use helix_fs::write_atomic;
use helix_workspace::identity::canonical_path;
use serde::{Deserialize, Serialize};

use crate::error::TrustError;
use crate::model::{TrustDecision, TrustEntry};

const STORE_VERSION: u32 = 1;
const STORE_FILE: &str = "trust.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct TrustDocument {
    version: u32,
    trust_everything: bool,
    entries: BTreeMap<String, TrustEntry>,
}

impl Default for TrustDocument {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            trust_everything: false,
            entries: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreHealth {
    Healthy,
    Unavailable,
}

pub struct TrustStore {
    path: PathBuf,
    document: RwLock<TrustDocument>,
    health: Mutex<StoreHealth>,
    persist: bool,
}

impl TrustStore {
    pub fn load(path: PathBuf) -> Self {
        let (document, health) = match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<TrustDocument>(&text) {
                Ok(mut document) if document.version == STORE_VERSION => {
                    normalize_entries(&mut document.entries);
                    (document, StoreHealth::Healthy)
                }
                _ => (TrustDocument::default(), StoreHealth::Unavailable),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                (TrustDocument::default(), StoreHealth::Healthy)
            }
            Err(_) => (TrustDocument::default(), StoreHealth::Unavailable),
        };
        Self {
            path,
            document: RwLock::new(document),
            health: Mutex::new(health),
            persist: true,
        }
    }

    pub fn in_memory() -> Self {
        Self {
            path: PathBuf::from("/dev/null/trust.json"),
            document: RwLock::new(TrustDocument::default()),
            health: Mutex::new(StoreHealth::Healthy),
            persist: false,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn health(&self) -> StoreHealth {
        *self.health.lock().unwrap()
    }

    pub fn trust_everything(&self) -> bool {
        self.document.read().unwrap().trust_everything
    }

    pub fn set_trust_everything(&self, enabled: bool) -> Result<(), TrustError> {
        self.update(|document| {
            document.trust_everything = enabled;
        })
    }

    pub fn set_entry(
        &self,
        path: &Path,
        decision: TrustDecision,
        inherit_to_children: bool,
    ) -> Result<(), TrustError> {
        let key = store_key(path)?;
        let entry = TrustEntry {
            decision,
            inherit_to_children,
            granted_ms: now_ms(),
        };
        self.update(|document| {
            document.entries.insert(key, entry);
        })
    }

    pub fn remove_entry(&self, path: &Path) -> Result<(), TrustError> {
        let key = store_key(path)?;
        self.update(|document| {
            document.entries.remove(&key);
        })
    }

    pub fn entries(&self) -> Vec<(String, TrustEntry)> {
        self.document
            .read()
            .unwrap()
            .entries
            .iter()
            .map(|(path, entry)| (path.clone(), entry.clone()))
            .collect()
    }

    pub fn resolve(&self, path: &Path) -> (TrustDecision, Option<String>) {
        if *self.health.lock().unwrap() == StoreHealth::Unavailable {
            return (TrustDecision::Restricted, None);
        }
        let document = self.document.read().unwrap();
        if document.trust_everything {
            return (TrustDecision::Trusted, None);
        }
        let canonical = canonical_path(path);
        if let Some(entry) = document.entries.get(&path_key(&canonical)) {
            return (entry.decision, None);
        }
        let mut current = canonical.as_path();
        while let Some(parent) = current.parent() {
            if parent == current {
                break;
            }
            if let Some(entry) = document.entries.get(&path_key(parent))
                && entry.inherit_to_children
                && entry.decision.is_trusted()
            {
                return (TrustDecision::Trusted, Some(parent.display().to_string()));
            }
            current = parent;
        }
        (TrustDecision::Unknown, None)
    }

    fn update(&self, mutate: impl FnOnce(&mut TrustDocument)) -> Result<(), TrustError> {
        if *self.health.lock().unwrap() == StoreHealth::Unavailable {
            return Err(TrustError::StoreUnavailable);
        }
        let mut document = self.document.write().unwrap();
        let mut next = document.clone();
        mutate(&mut next);
        if self.persist {
            let text = serde_json::to_string_pretty(&next)
                .map_err(|error| TrustError::storage(error.to_string()))?;
            if let Err(error) = write_atomic(&self.path, text.as_bytes()) {
                *self.health.lock().unwrap() = StoreHealth::Unavailable;
                return Err(TrustError::storage(error.to_string()));
            }
        }
        *document = next;
        Ok(())
    }
}

pub fn default_store_path() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|path| path.join("Helix"))
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(PathBuf::from).map(|home| {
            home.join("Library")
                .join("Application Support")
                .join("Helix")
        })
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join(".local").join("share"))
            })
            .map(|path| path.join("helix"))
    };
    base.map(|path| path.join(STORE_FILE))
}

fn store_key(path: &Path) -> Result<String, TrustError> {
    if path.as_os_str().is_empty() {
        return Err(TrustError::InvalidPath("path must not be empty".into()));
    }
    Ok(path_key(&canonical_path(path)))
}

fn path_key(path: &Path) -> String {
    canonical_path(path).to_string_lossy().to_string()
}

fn normalize_entries(entries: &mut BTreeMap<String, TrustEntry>) {
    let normalized: BTreeMap<String, TrustEntry> = entries
        .iter()
        .map(|(path, entry)| (path_key(Path::new(path)), entry.clone()))
        .collect();
    *entries = normalized;
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_trust_is_inherited_when_flagged() {
        let store = TrustStore::in_memory();
        store
            .set_entry(Path::new("/tmp/parent"), TrustDecision::Trusted, true)
            .unwrap();
        let (decision, inherited) = store.resolve(Path::new("/tmp/parent/child/repo"));
        assert_eq!(decision, TrustDecision::Trusted);
        assert!(inherited.is_some());
        assert!(inherited.unwrap().ends_with("parent"));
    }

    #[test]
    fn corrupt_store_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trust.json");
        std::fs::write(&path, "{ not valid json").unwrap();
        let store = TrustStore::load(path);
        assert_eq!(store.health(), StoreHealth::Unavailable);
        assert_eq!(
            store.resolve(Path::new("/any/path")).0,
            TrustDecision::Restricted
        );
    }

    #[test]
    fn persistence_failure_changes_health_and_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let parent_file = dir.path().join("not-a-directory");
        std::fs::write(&parent_file, "x").unwrap();
        let store = TrustStore::load(parent_file.join("trust.json"));

        assert!(
            store
                .set_entry(Path::new("/tmp/repo"), TrustDecision::Trusted, false)
                .is_err()
        );
        assert_eq!(store.health(), StoreHealth::Unavailable);
        assert_eq!(
            store.resolve(Path::new("/tmp/repo")).0,
            TrustDecision::Restricted
        );
    }
}
