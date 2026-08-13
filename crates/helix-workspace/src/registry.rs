//! The workspace-scoped service registry, reference-counted across windows
//! (REQ-ARCH-006, and the design document's Window and Workspace Scoping
//! section).
//!
//! ```text
//!  Window A ──acquire("ws1")──► refs 1 ─┐
//!  Window C ──acquire("ws1")──► refs 2  ├─ one set of language servers,
//!  Window A ──release──────────► refs 1 ─┘  watchers, terminals, index
//!  Window C ──release──────────► refs 0 ──► resources dropped, cleanup runs
//! ```
//!
//! The design document names this the single most likely source of bugs in
//! multi-window support, so the counting is not left to callers. [`acquire`]
//! hands back a [`WorkspaceLease`] whose `Drop` releases it, and cloning a
//! lease acquires another reference. A window that closes releases exactly what
//! it took, and a window that panics still releases it, because unwinding runs
//! destructors and a forgotten `release()` call would not.
//!
//! Resources are stored by [`TypeId`], mirroring
//! [`helix_core::container::ServiceContext`]: same resolution style as the
//! global container, one entry per type, per workspace.
//!
//! [`acquire`]: WorkspaceRegistry::acquire

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// A callback run when a workspace's last reference goes away, before its
/// resources are dropped.
pub type TeardownHook = Arc<dyn Fn(&str) + Send + Sync>;

#[derive(Default)]
struct Entry {
    refs: u32,
    resources: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    teardown: Vec<TeardownHook>,
}

/// Workspace-scoped resources, keyed by workspace key and reference-counted by
/// holder.
#[derive(Default)]
pub struct WorkspaceRegistry {
    entries: RwLock<HashMap<String, Entry>>,
}

impl WorkspaceRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Take a reference to a workspace's scope, creating it if this is the
    /// first holder. The returned lease releases it when dropped.
    pub fn acquire(self: &Arc<Self>, key: &str) -> WorkspaceLease {
        let mut entries = self.entries.write().unwrap();
        let entry = entries.entry(key.to_string()).or_default();
        entry.refs += 1;
        WorkspaceLease {
            key: key.to_string(),
            registry: self.clone(),
        }
    }

    /// Whether this is the first holder of a scope, answered at acquire time.
    /// Used by the caller that has to decide whether to start the workspace's
    /// services or join the ones already running.
    pub fn ref_count(&self, key: &str) -> u32 {
        self.entries
            .read()
            .unwrap()
            .get(key)
            .map(|entry| entry.refs)
            .unwrap_or(0)
    }

    /// Every scope with at least one holder.
    pub fn keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.entries.read().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    }

    /// Publish a workspace-scoped resource. Replaces any earlier value of the
    /// same type for that workspace.
    pub fn publish<T: Any + Send + Sync>(&self, key: &str, value: Arc<T>) {
        let mut entries = self.entries.write().unwrap();
        entries
            .entry(key.to_string())
            .or_default()
            .resources
            .insert(TypeId::of::<T>(), value as Arc<dyn Any + Send + Sync>);
    }

    /// Publish only when a workspace still has at least one holder. The check
    /// and insert share the registry write lock, preventing a background task
    /// from recreating a scope after its last window closed.
    pub fn publish_if_active<T: Any + Send + Sync>(&self, key: &str, value: Arc<T>) -> bool {
        let mut entries = self.entries.write().unwrap();
        let Some(entry) = entries.get_mut(key).filter(|entry| entry.refs > 0) else {
            return false;
        };
        entry
            .resources
            .insert(TypeId::of::<T>(), value as Arc<dyn Any + Send + Sync>);
        true
    }

    /// Resolve a workspace-scoped resource.
    pub fn resolve<T: Any + Send + Sync>(&self, key: &str) -> Option<Arc<T>> {
        self.entries
            .read()
            .unwrap()
            .get(key)
            .and_then(|entry| entry.resources.get(&TypeId::of::<T>()))
            .cloned()
            .and_then(|value| value.downcast::<T>().ok())
    }

    /// Register a callback for when this workspace's last reference goes away.
    pub fn on_teardown(&self, key: &str, hook: TeardownHook) {
        let mut entries = self.entries.write().unwrap();
        entries
            .entry(key.to_string())
            .or_default()
            .teardown
            .push(hook);
    }

    /// Drop one reference. Returns the count that remains.
    ///
    /// At zero the scope's teardown hooks run and its resources are dropped —
    /// outside the lock, because a hook that touched the registry while it was
    /// held would deadlock, and a teardown path is exactly where that mistake
    /// is easiest to make.
    fn release(&self, key: &str) -> u32 {
        let (remaining, teardown, resources) = {
            let mut entries = self.entries.write().unwrap();
            let Some(entry) = entries.get_mut(key) else {
                return 0;
            };
            entry.refs = entry.refs.saturating_sub(1);
            if entry.refs > 0 {
                return entry.refs;
            }
            let entry = entries.remove(key).unwrap_or_default();
            (0, entry.teardown, entry.resources)
        };

        for hook in &teardown {
            hook(key);
        }
        drop(resources);
        remaining
    }
}

/// One holder's reference to a workspace scope.
///
/// Cloning takes another reference, which is what a second window opening the
/// same workspace does. Dropping releases this one.
pub struct WorkspaceLease {
    key: String,
    registry: Arc<WorkspaceRegistry>,
}

impl WorkspaceLease {
    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn registry(&self) -> &Arc<WorkspaceRegistry> {
        &self.registry
    }

    pub fn ref_count(&self) -> u32 {
        self.registry.ref_count(&self.key)
    }

    pub fn publish<T: Any + Send + Sync>(&self, value: Arc<T>) {
        self.registry.publish(&self.key, value);
    }

    pub fn resolve<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.registry.resolve(&self.key)
    }
}

impl Clone for WorkspaceLease {
    fn clone(&self) -> Self {
        self.registry.acquire(&self.key)
    }
}

impl std::fmt::Debug for WorkspaceLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceLease")
            .field("key", &self.key)
            .field("refs", &self.ref_count())
            .finish()
    }
}

impl Drop for WorkspaceLease {
    fn drop(&mut self) {
        self.registry.release(&self.key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[derive(Debug, PartialEq, Eq)]
    struct SearchIndex(u32);

    #[test]
    fn the_first_holder_creates_the_scope_and_the_last_one_ends_it() {
        let registry = WorkspaceRegistry::new();
        let torn_down = Arc::new(AtomicU32::new(0));

        let window_a = registry.acquire("ws1");
        assert_eq!(window_a.ref_count(), 1);
        let counter = torn_down.clone();
        registry.on_teardown(
            "ws1",
            Arc::new(move |_key| {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
        );

        let window_c = registry.acquire("ws1");
        assert_eq!(registry.ref_count("ws1"), 2);

        drop(window_a);
        assert_eq!(registry.ref_count("ws1"), 1);
        assert_eq!(
            torn_down.load(Ordering::SeqCst),
            0,
            "closing one of two windows must not tear down the shared workspace"
        );

        drop(window_c);
        assert_eq!(registry.ref_count("ws1"), 0);
        assert_eq!(torn_down.load(Ordering::SeqCst), 1);
        assert!(registry.keys().is_empty());
    }

    #[test]
    fn resources_are_shared_between_holders_of_one_workspace() {
        let registry = WorkspaceRegistry::new();
        let window_a = registry.acquire("ws1");
        window_a.publish(Arc::new(SearchIndex(42)));

        let window_c = registry.acquire("ws1");
        let shared = window_c.resolve::<SearchIndex>().expect("shared resource");
        assert_eq!(*shared, SearchIndex(42));
        assert!(
            Arc::ptr_eq(&shared, &window_a.resolve::<SearchIndex>().unwrap()),
            "both windows must see one instance, not a copy each"
        );
    }

    #[test]
    fn two_workspaces_do_not_see_each_others_resources() {
        let registry = WorkspaceRegistry::new();
        let one = registry.acquire("ws1");
        let two = registry.acquire("ws2");
        one.publish(Arc::new(SearchIndex(1)));

        assert!(two.resolve::<SearchIndex>().is_none());
        assert_eq!(registry.keys(), vec!["ws1".to_string(), "ws2".to_string()]);
    }

    #[test]
    fn resources_are_dropped_when_the_scope_ends() {
        let registry = WorkspaceRegistry::new();
        let held = Arc::new(SearchIndex(7));
        {
            let lease = registry.acquire("ws1");
            lease.publish(held.clone());
            assert_eq!(Arc::strong_count(&held), 2);
        }
        assert_eq!(
            Arc::strong_count(&held),
            1,
            "the registry must not keep a workspace's resources alive after its last window"
        );
    }

    #[test]
    fn a_late_background_publish_cannot_recreate_a_closed_scope() {
        let registry = WorkspaceRegistry::new();
        let lease = registry.acquire("ws1");
        drop(lease);

        assert!(!registry.publish_if_active("ws1", Arc::new(SearchIndex(9))));
        assert!(registry.keys().is_empty());
    }

    #[test]
    fn cloning_a_lease_takes_another_reference() {
        let registry = WorkspaceRegistry::new();
        let lease = registry.acquire("ws1");
        let second = lease.clone();
        assert_eq!(registry.ref_count("ws1"), 2);
        drop(second);
        assert_eq!(registry.ref_count("ws1"), 1);
        drop(lease);
        assert_eq!(registry.ref_count("ws1"), 0);
    }

    #[test]
    fn a_teardown_hook_may_talk_to_the_registry_without_deadlocking() {
        let registry = WorkspaceRegistry::new();
        let lease = registry.acquire("ws1");
        let inner = registry.clone();
        registry.on_teardown(
            "ws1",
            Arc::new(move |key| {
                // Ran outside the registry's lock, so this is safe rather than
                // a hang that only shows up on shutdown.
                assert_eq!(inner.ref_count(key), 0);
            }),
        );
        drop(lease);
        assert_eq!(registry.ref_count("ws1"), 0);
    }
}
