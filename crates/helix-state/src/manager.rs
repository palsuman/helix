use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use helix_workspace::WorkspaceSnapshot;

use crate::{
    AgentState, BufferState, PersistenceStatus, RecoveryReport, SessionSnapshot, StateError,
    StateStore, StateStoreConfig, TerminalState, now_ms, workspace_state_directory,
};

struct WorkspaceState {
    store: Arc<StateStore>,
    session: SessionSnapshot,
    recovery: RecoveryReport,
}

/// Kernel-facing registry of one durable store per open workspace.
pub struct StatePersistence {
    config: StateStoreConfig,
    workspaces: RwLock<BTreeMap<String, WorkspaceState>>,
}

impl StatePersistence {
    pub fn new(config: StateStoreConfig) -> Self {
        Self {
            config,
            workspaces: RwLock::new(BTreeMap::new()),
        }
    }

    /// Attach state to an opened workspace and recover it before accepting edits.
    pub fn open(&self, workspace: &WorkspaceSnapshot) -> Result<RecoveryReport, StateError> {
        if let Some(recovery) = self
            .workspaces
            .read()
            .unwrap()
            .get(&workspace.key)
            .map(|state| state.recovery.clone())
        {
            return Ok(recovery);
        }
        let roots: Vec<PathBuf> = workspace
            .roots
            .iter()
            .map(|root| PathBuf::from(&root.path))
            .collect();
        let root = workspace_state_directory(&workspace.key).ok_or_else(|| {
            StateError::Invalid("the operating-system state directory could not be resolved".into())
        })?;
        self.open_at(workspace.key.clone(), roots, root)
    }

    /// Explicit-root variant used by crash tests and embedders.
    pub fn open_at(
        &self,
        key: String,
        roots: Vec<PathBuf>,
        state_dir: PathBuf,
    ) -> Result<RecoveryReport, StateError> {
        let store = Arc::new(StateStore::new(
            state_dir,
            key.clone(),
            roots,
            self.config.clone(),
        ));
        let recovery = store.recover()?;
        let session = recovery.session.clone();
        self.workspaces.write().unwrap().insert(
            key,
            WorkspaceState {
                store,
                session,
                recovery: recovery.clone(),
            },
        );
        Ok(recovery)
    }

    pub fn recovered(&self, key: &str) -> Option<RecoveryReport> {
        self.workspaces
            .read()
            .unwrap()
            .get(key)
            .map(|state| state.recovery.clone())
    }

    pub fn update_buffer(&self, key: &str, value: BufferState) -> Result<(), StateError> {
        let mut workspaces = self.workspaces.write().unwrap();
        let state = workspaces
            .get_mut(key)
            .ok_or_else(|| StateError::Invalid(format!("workspace '{key}' has no state store")))?;
        upsert(&mut state.session.buffers, value.clone(), |v| &v.id);
        state.store.queue_buffer(value, now_ms());
        Ok(())
    }

    pub fn update_terminal(&self, key: &str, value: TerminalState) -> Result<(), StateError> {
        let mut workspaces = self.workspaces.write().unwrap();
        let state = workspaces
            .get_mut(key)
            .ok_or_else(|| StateError::Invalid(format!("workspace '{key}' has no state store")))?;
        upsert(&mut state.session.terminals, value.clone(), |v| &v.id);
        state.store.queue_terminal(value, now_ms());
        Ok(())
    }

    pub fn update_agent(&self, key: &str, value: AgentState) -> Result<(), StateError> {
        let mut workspaces = self.workspaces.write().unwrap();
        let state = workspaces
            .get_mut(key)
            .ok_or_else(|| StateError::Invalid(format!("workspace '{key}' has no state store")))?;
        upsert(&mut state.session.agents, value.clone(), |v| &v.id);
        state.store.queue_agent(value, now_ms());
        Ok(())
    }

    pub fn replace_session(&self, key: &str, session: SessionSnapshot) -> Result<(), StateError> {
        let mut workspaces = self.workspaces.write().unwrap();
        let state = workspaces
            .get_mut(key)
            .ok_or_else(|| StateError::Invalid(format!("workspace '{key}' has no state store")))?;
        state.session = session;
        Ok(())
    }

    pub fn flush_due(&self) -> Vec<(String, StateError)> {
        let now = now_ms();
        self.workspaces
            .read()
            .unwrap()
            .iter()
            .filter_map(|(key, state)| {
                state
                    .store
                    .flush_due(now, &state.session)
                    .err()
                    .map(|error| (key.clone(), error))
            })
            .collect()
    }

    pub fn close(&self, key: &str) -> Result<(), StateError> {
        let state = self.workspaces.write().unwrap().remove(key);
        if let Some(state) = state {
            state.store.flush_all(now_ms())?;
        }
        Ok(())
    }

    pub fn flush_all(&self) -> Vec<(String, StateError)> {
        self.workspaces
            .read()
            .unwrap()
            .iter()
            .filter_map(|(key, state)| {
                state
                    .store
                    .flush_all(now_ms())
                    .err()
                    .map(|error| (key.clone(), error))
            })
            .collect()
    }

    pub fn statuses(&self) -> BTreeMap<String, PersistenceStatus> {
        self.workspaces
            .read()
            .unwrap()
            .iter()
            .map(|(key, state)| (key.clone(), state.store.status()))
            .collect()
    }

    pub fn workspace_count(&self) -> usize {
        self.workspaces.read().unwrap().len()
    }
}

fn upsert<T, F>(values: &mut Vec<T>, value: T, id: F)
where
    F: Fn(&T) -> &str,
{
    if let Some(index) = values.iter().position(|current| id(current) == id(&value)) {
        values[index] = value;
    } else {
        values.push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_fs::testutil::TempDir;

    #[test]
    fn manager_replays_state_across_kernel_instances() {
        let dir = TempDir::new("state-manager-restart");
        let state_dir = dir.path().join("outside-workspace/state/key");
        let root = dir.mkdir("readonly-workspace");
        let first = StatePersistence::new(StateStoreConfig::default());
        first
            .open_at("key".into(), vec![root.clone()], state_dir.clone())
            .unwrap();
        first
            .update_buffer(
                "key",
                BufferState {
                    id: "untitled-1".into(),
                    content: "unsaved".into(),
                    language: "text".into(),
                    target: None,
                    dirty: true,
                    cursor_line: 0,
                    cursor_column: 7,
                },
            )
            .unwrap();
        assert!(first.flush_all().is_empty());
        drop(first);

        let second = StatePersistence::new(StateStoreConfig::default());
        let recovered = second.open_at("key".into(), vec![root], state_dir).unwrap();
        assert_eq!(recovered.session.buffers[0].content, "unsaved");
    }
}
