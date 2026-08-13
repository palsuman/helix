//! Process registry terminated when trust is revoked (REQ-FS-005.6).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use helix_workspace::identity::canonical_path;

#[derive(Debug, Clone)]
pub struct ManagedProcess {
    pub id: u64,
    pub path: PathBuf,
    pub label: String,
}

struct RegisteredProcess {
    metadata: ManagedProcess,
    terminate: Arc<dyn Fn() + Send + Sync>,
}

#[derive(Default)]
pub struct ProcessRegistry {
    next_id: Mutex<u64>,
    processes: RwLock<BTreeMap<u64, RegisteredProcess>>,
}

impl ProcessRegistry {
    pub fn register(&self, path: &Path, label: impl Into<String>) -> u64 {
        self.register_with_terminator(path, label, || {})
    }

    /// Register a process together with the operation that actually stops it.
    /// Launching subsystems provide a handle-backed callback here so trust
    /// revocation cannot degrade into merely forgetting process metadata.
    pub fn register_with_terminator(
        &self,
        path: &Path,
        label: impl Into<String>,
        terminate: impl Fn() + Send + Sync + 'static,
    ) -> u64 {
        let id = {
            let mut next = self.next_id.lock().unwrap();
            *next += 1;
            *next
        };
        self.processes.write().unwrap().insert(
            id,
            RegisteredProcess {
                metadata: ManagedProcess {
                    id,
                    path: canonical_path(path),
                    label: label.into(),
                },
                terminate: Arc::new(terminate),
            },
        );
        id
    }

    pub fn terminate(&self, id: u64) -> bool {
        let process = self.processes.write().unwrap().remove(&id);
        if let Some(process) = process {
            (process.terminate)();
            true
        } else {
            false
        }
    }

    pub fn unregister(&self, id: u64) -> bool {
        self.processes.write().unwrap().remove(&id).is_some()
    }

    pub fn terminate_for_path(&self, path: &Path) -> Vec<ManagedProcess> {
        let canonical = canonical_path(path);
        let mut processes = self.processes.write().unwrap();
        let ids: Vec<u64> = processes
            .iter()
            .filter(|(_, process)| path_matches(&process.metadata.path, &canonical))
            .map(|(id, _)| *id)
            .collect();
        let removed = ids
            .into_iter()
            .filter_map(|id| processes.remove(&id))
            .collect::<Vec<_>>();
        drop(processes);

        removed
            .into_iter()
            .map(|process| {
                (process.terminate)();
                process.metadata
            })
            .collect()
    }

    pub fn terminate_all(&self) -> Vec<ManagedProcess> {
        let removed = {
            let mut processes = self.processes.write().unwrap();
            std::mem::take(&mut *processes)
        };
        removed
            .into_values()
            .map(|process| {
                (process.terminate)();
                process.metadata
            })
            .collect()
    }

    pub fn active_for_path(&self, path: &Path) -> Vec<ManagedProcess> {
        let canonical = canonical_path(path);
        self.processes
            .read()
            .unwrap()
            .values()
            .filter(|process| path_matches(&process.metadata.path, &canonical))
            .map(|process| process.metadata.clone())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.processes.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn path_matches(process_root: &Path, target: &Path) -> bool {
    process_root == target || process_root.starts_with(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn revocation_terminates_processes_under_the_path() {
        let registry = ProcessRegistry::default();
        let stopped = Arc::new(AtomicBool::new(false));
        let stopped_by_hook = stopped.clone();
        let id = registry.register_with_terminator(
            Path::new("/tmp/repo/packages/app"),
            "typescript-lsp",
            move || stopped_by_hook.store(true, Ordering::SeqCst),
        );
        registry.register(Path::new("/tmp/other"), "eslint-lsp");
        let terminated = registry.terminate_for_path(Path::new("/tmp/repo"));
        assert_eq!(terminated.len(), 1);
        assert_eq!(terminated[0].id, id);
        assert!(stopped.load(Ordering::SeqCst));
        assert!(!registry.terminate(id));
    }
}
