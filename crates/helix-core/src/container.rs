//! Service container with dependency injection (Task 1.2).
//!
//! Implements the `Service` / `HealthCheck` contract sketched in the design
//! document's "Service Container Interface" section (REQ-ARCH-002) plus the
//! isolated-restart guarantee required by REQ-OBS-004.
//!
//! ## Model
//!
//! - Services are registered with a name, a declared dependency list (by
//!   name), a [`Lifetime`], and a factory closure.
//! - Cycle detection runs at registration time via DFS over the dependency
//!   graph, so a circular dependency is rejected with a clear error the
//!   moment it is introduced rather than discovered at start time.
//! - [`ServiceContainer::start_all`] starts every singleton in dependency
//!   order (dependencies before dependents) and spawns each one under its
//!   own supervisor task. A panic inside a service's `start`/`run` is caught
//!   by that supervisor (via `tokio::spawn`'s panic isolation) and the
//!   *single* affected service is restarted, up to a bounded number of
//!   attempts, without touching any other service or the kernel process.
//! - [`ServiceContainer::stop_all`] shuts down singletons in reverse
//!   registration order, per REQ-ARCH-002.4.
//! - [`ServiceProvider`] is the trait dependents and tests interact with, so
//!   a test can substitute a mock provider without wiring a real container.
//!
//! Rust has no first-class way to prevent a circular dependency at compile
//! time for an arbitrary, dynamically-registered graph, so "compile-time
//! where possible" is satisfied narrowly: a service's `dependencies()` list
//! is `&'static [&'static str]`, so at minimum a typo'd or empty dependency
//! list is a compile error, not a runtime surprise. The general case is the
//! runtime DFS check described above.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock as SyncRwLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::health::{ServiceHealth, ServiceMetrics};

/// Errors produced by the service container.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ServiceError {
    #[error("service '{0}' is already registered")]
    AlreadyRegistered(String),
    #[error("circular dependency detected: {0}")]
    CircularDependency(String),
    #[error("service '{0}' depends on unregistered service '{1}'")]
    MissingDependency(String, String),
    #[error("service '{0}' failed to start: {1}")]
    StartFailed(String, String),
    #[error("service '{0}' failed to stop: {1}")]
    StopFailed(String, String),
    #[error("service '{0}' is not registered")]
    NotFound(String),
    #[error("service '{0}' panicked and exceeded its restart budget: {1}")]
    RestartBudgetExceeded(String, String),
}

/// Lifetime under which a service is managed by the container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifetime {
    /// One instance for the life of the container, supervised and
    /// restarted in isolation on panic.
    Singleton,
    /// A fresh instance constructed on every resolution; the caller owns
    /// its lifecycle and the container does not supervise it.
    Transient,
    /// One instance per scope key (e.g. a workspace or window id),
    /// supervised like a singleton but torn down when its scope ends.
    Scoped,
}

/// Shared context passed to every factory at construction time. Carries
/// already-started dependency handles and a typed resource map so a
/// dependent can resolve what it needs, guaranteed available because
/// dependencies are always constructed before their dependents.
#[derive(Clone, Default)]
pub struct ServiceContext {
    handles: Arc<SyncRwLock<HashMap<String, ManagedHandle>>>,
    resources: Arc<SyncRwLock<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>>,
}

impl ServiceContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve the shared handle of an already-started dependency by name.
    pub fn resolve_handle(&self, name: &str) -> Option<ManagedHandle> {
        self.handles.read().unwrap().get(name).cloned()
    }

    fn publish_handle(&self, name: &str, handle: ManagedHandle) {
        self.handles
            .write()
            .unwrap()
            .insert(name.to_string(), handle);
    }

    /// Publish a typed shared resource for later retrieval via [`resolve`].
    pub fn publish<T: Any + Send + Sync>(&self, value: Arc<T>) {
        self.resources
            .write()
            .unwrap()
            .insert(TypeId::of::<T>(), value as Arc<dyn Any + Send + Sync>);
    }

    /// Resolve a typed shared resource published earlier via [`publish`].
    pub fn resolve<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.resources
            .read()
            .unwrap()
            .get(&TypeId::of::<T>())
            .cloned()
            .and_then(|arc| arc.downcast::<T>().ok())
    }
}

/// The lifecycle contract every managed service implements.
///
/// Mirrors the design document's `Service` trait. `run` is the service's
/// steady-state loop; the default implementation idles forever, which is
/// correct for services with no ongoing background work beyond `start`.
#[async_trait]
pub trait Service: Send + Sync + 'static {
    fn name(&self) -> &'static str;

    /// Declared dependencies, resolved (via [`ServiceContext`]) before this
    /// service's factory is invoked.
    fn dependencies(&self) -> &'static [&'static str] {
        &[]
    }

    async fn start(&mut self, ctx: &ServiceContext) -> Result<(), ServiceError>;

    /// Steady-state run loop, driven after a successful `start`. Returning
    /// `Ok(())` means the service completed gracefully; returning `Err`
    /// or panicking triggers the container's restart policy.
    async fn run(&mut self) -> Result<(), ServiceError> {
        std::future::pending::<()>().await;
        #[allow(unreachable_code)]
        Ok(())
    }

    async fn stop(&mut self) -> Result<(), ServiceError> {
        Ok(())
    }
}

/// Liveness/metrics probe every service exposes (REQ-ARCH-002.6).
pub trait HealthCheck: Send + Sync {
    fn health(&self) -> ServiceHealth;
    fn metrics(&self) -> ServiceMetrics;
}

/// A service that is both lifecycle-managed and health-reportable. Blanket
/// implementation: nothing beyond implementing both traits is required.
pub trait ManagedService: Service + HealthCheck {}
impl<T: Service + HealthCheck> ManagedService for T {}

type Factory =
    Arc<dyn Fn(&ServiceContext) -> Result<Box<dyn ManagedService>, ServiceError> + Send + Sync>;

struct ServiceDescriptor {
    name: &'static str,
    dependencies: Vec<&'static str>,
    lifetime: Lifetime,
    factory: Factory,
}

/// A cloneable handle to a running (or most recently constructed) managed
/// service, shared between the container's supervisor task and anything
/// that needs to inspect health without owning the instance.
#[derive(Clone)]
pub struct ManagedHandle {
    name: &'static str,
    health: Arc<SyncRwLock<ServiceHealth>>,
    metrics: Arc<SyncRwLock<ServiceMetrics>>,
    restart_count: Arc<std::sync::atomic::AtomicU32>,
}

impl ManagedHandle {
    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn health(&self) -> ServiceHealth {
        self.health.read().unwrap().clone()
    }

    pub fn metrics(&self) -> ServiceMetrics {
        self.metrics.read().unwrap().clone()
    }

    pub fn restart_count(&self) -> u32 {
        self.restart_count.load(Ordering::SeqCst)
    }
}

struct RunningService {
    handle: ManagedHandle,
    stop_requested: Arc<AtomicBool>,
    stop_notify: Arc<Notify>,
    join: JoinHandle<()>,
}

/// The default maximum number of consecutive panic/error restarts a single
/// service is granted before the container gives up and marks it `Failed`.
/// Kept small and private: the point is isolation, not infinite retries.
const MAX_RESTART_ATTEMPTS: u32 = 3;

/// A trait dependents (and tests) use to resolve container state without
/// depending on the concrete [`ServiceContainer`]. Object-safe so a mock
/// implementation can be substituted wholesale in unit tests.
pub trait ServiceProvider: Send + Sync {
    fn health_of(&self, name: &str) -> Option<ServiceHealth>;
    fn metrics_of(&self, name: &str) -> Option<ServiceMetrics>;
    fn resolve_any(&self, type_id: TypeId) -> Option<Arc<dyn Any + Send + Sync>>;
}

impl dyn ServiceProvider {
    /// Typed convenience wrapper over [`ServiceProvider::resolve_any`].
    pub fn resolve<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.resolve_any(TypeId::of::<T>())
            .and_then(|arc| arc.downcast::<T>().ok())
    }
}

/// The service container: registration, dependency-ordered startup,
/// isolated panic recovery, and reverse-order shutdown.
pub struct ServiceContainer {
    ctx: ServiceContext,
    descriptors: HashMap<&'static str, ServiceDescriptor>,
    registration_order: Vec<&'static str>,
    running: HashMap<&'static str, RunningService>,
}

impl Default for ServiceContainer {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceContainer {
    pub fn new() -> Self {
        Self {
            ctx: ServiceContext::new(),
            descriptors: HashMap::new(),
            registration_order: Vec::new(),
            running: HashMap::new(),
        }
    }

    /// Register a service. Cycle detection runs immediately: a
    /// dependency graph that would become circular is rejected here, and
    /// the registration is rolled back rather than left half-applied.
    pub fn register<F>(
        &mut self,
        name: &'static str,
        dependencies: &[&'static str],
        lifetime: Lifetime,
        factory: F,
    ) -> Result<(), ServiceError>
    where
        F: Fn(&ServiceContext) -> Result<Box<dyn ManagedService>, ServiceError>
            + Send
            + Sync
            + 'static,
    {
        if self.descriptors.contains_key(name) {
            return Err(ServiceError::AlreadyRegistered(name.to_string()));
        }

        self.descriptors.insert(
            name,
            ServiceDescriptor {
                name,
                dependencies: dependencies.to_vec(),
                lifetime,
                factory: Arc::new(factory),
            },
        );

        if let Err(e) = self.detect_cycles() {
            self.descriptors.remove(name);
            return Err(e);
        }

        self.registration_order.push(name);
        Ok(())
    }

    /// DFS-based cycle detection over the currently registered graph.
    /// Edges to not-yet-registered names are ignored here (they are caught
    /// as [`ServiceError::MissingDependency`] at start time), so services
    /// may be registered in any order.
    fn detect_cycles(&self) -> Result<(), ServiceError> {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Mark {
            Unvisited,
            InProgress,
            Done,
        }

        let graph: HashMap<String, Vec<String>> = self
            .descriptors
            .values()
            .map(|d| {
                (
                    d.name.to_string(),
                    d.dependencies.iter().map(|s| s.to_string()).collect(),
                )
            })
            .collect();

        let mut marks: HashMap<String, Mark> =
            graph.keys().map(|k| (k.clone(), Mark::Unvisited)).collect();

        fn visit(
            node: &str,
            graph: &HashMap<String, Vec<String>>,
            marks: &mut HashMap<String, Mark>,
            path: &mut Vec<String>,
        ) -> Result<(), ServiceError> {
            match marks.get(node) {
                Some(Mark::Done) => return Ok(()),
                Some(Mark::InProgress) => {
                    path.push(node.to_string());
                    let start = path.iter().position(|n| n == node).unwrap();
                    return Err(ServiceError::CircularDependency(path[start..].join(" -> ")));
                }
                _ => {}
            }
            marks.insert(node.to_string(), Mark::InProgress);
            path.push(node.to_string());
            if let Some(deps) = graph.get(node) {
                for dep in deps {
                    if graph.contains_key(dep) {
                        visit(dep, graph, marks, path)?;
                    }
                }
            }
            path.pop();
            marks.insert(node.to_string(), Mark::Done);
            Ok(())
        }

        let names: Vec<String> = graph.keys().cloned().collect();
        for name in names {
            if marks.get(&name) == Some(&Mark::Unvisited) {
                let mut path = Vec::new();
                visit(&name, &graph, &mut marks, &mut path)?;
            }
        }
        Ok(())
    }

    /// Dependencies-first topological order of all registered services.
    /// Errors if a declared dependency was never registered.
    fn topo_order(&self) -> Result<Vec<&'static str>, ServiceError> {
        let mut order = Vec::new();
        let mut visited: HashMap<&'static str, bool> = HashMap::new(); // true = done

        fn visit(
            name: &'static str,
            descriptors: &HashMap<&'static str, ServiceDescriptor>,
            visited: &mut HashMap<&'static str, bool>,
            order: &mut Vec<&'static str>,
        ) -> Result<(), ServiceError> {
            if visited.get(name).copied().unwrap_or(false) {
                return Ok(());
            }
            let desc = descriptors
                .get(name)
                .ok_or_else(|| ServiceError::NotFound(name.to_string()))?;
            for dep in &desc.dependencies {
                if !descriptors.contains_key(dep) {
                    return Err(ServiceError::MissingDependency(
                        name.to_string(),
                        dep.to_string(),
                    ));
                }
                visit(dep, descriptors, visited, order)?;
            }
            visited.insert(name, true);
            order.push(name);
            Ok(())
        }

        // Iterate in registration order so ties (independent services) keep
        // a deterministic, predictable start order.
        for name in &self.registration_order {
            visit(name, &self.descriptors, &mut visited, &mut order)?;
        }
        Ok(order)
    }

    /// Start every registered singleton, dependencies before dependents.
    /// Scoped and transient services are not started here; see
    /// [`resolve_scoped`] and [`resolve_transient`].
    pub async fn start_all(&mut self) -> Result<(), ServiceError> {
        let order = self.topo_order()?;
        for name in order {
            let is_singleton = self
                .descriptors
                .get(name)
                .map(|d| d.lifetime == Lifetime::Singleton)
                .unwrap_or(false);
            if is_singleton && !self.running.contains_key(name) {
                self.start_singleton(name).await?;
            }
        }
        Ok(())
    }

    /// Construct and supervise a single singleton service, publishing its
    /// handle into the shared context so dependents can resolve it.
    async fn start_singleton(&mut self, name: &'static str) -> Result<(), ServiceError> {
        let descriptor = self
            .descriptors
            .get(name)
            .ok_or_else(|| ServiceError::NotFound(name.to_string()))?;
        let factory = descriptor.factory.clone();
        let ctx = self.ctx.clone();

        let health = Arc::new(SyncRwLock::new(ServiceHealth::Degraded {
            reason: "starting".into(),
            since_ms: 0,
        }));
        let metrics = Arc::new(SyncRwLock::new(ServiceMetrics::default()));
        let restart_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let handle = ManagedHandle {
            name,
            health: health.clone(),
            metrics: metrics.clone(),
            restart_count: restart_count.clone(),
        };
        ctx.publish_handle(name, handle.clone());

        let stop_requested = Arc::new(AtomicBool::new(false));
        let stop_notify = Arc::new(Notify::new());
        let (started_tx, started_rx) = tokio::sync::oneshot::channel::<Result<(), ServiceError>>();

        let join = {
            let stop_requested = stop_requested.clone();
            let stop_notify = stop_notify.clone();
            let started_at = Instant::now();
            tokio::spawn(async move {
                let mut attempt: u32 = 0;
                let mut started_tx = Some(started_tx);

                loop {
                    let ctx = ctx.clone();
                    let factory = factory.clone();
                    let stop_requested_inner = stop_requested.clone();
                    let stop_notify_inner = stop_notify.clone();

                    // Signalled as soon as `start()` returns, independent
                    // of how long the subsequent `run()` loop lives (which
                    // is typically "forever" for a healthy service). This
                    // is what lets `start_all()` return promptly instead
                    // of blocking on a service's entire run loop.
                    let (phase_tx, phase_rx) =
                        tokio::sync::oneshot::channel::<Result<(), ServiceError>>();

                    // Run one attempt (construct + start + run) inside its
                    // own spawned task so a panic anywhere in it is caught
                    // by tokio and surfaces as a JoinError here, never
                    // unwinding into this supervisor loop or any other
                    // service's task.
                    let attempt_task = tokio::spawn(async move {
                        let mut instance = match factory(&ctx) {
                            Ok(i) => i,
                            Err(e) => {
                                let _ = phase_tx.send(Err(e.clone()));
                                return Err(e);
                            }
                        };
                        if let Err(e) = instance.start(&ctx).await {
                            let _ = phase_tx.send(Err(e.clone()));
                            return Err(e);
                        }
                        let _ = phase_tx.send(Ok(()));
                        loop {
                            tokio::select! {
                                res = instance.run() => {
                                    return res.map(|_| AttemptOutcome::Completed);
                                }
                                _ = stop_notify_inner.notified() => {
                                    if stop_requested_inner.load(Ordering::SeqCst) {
                                        instance.stop().await?;
                                        return Ok(AttemptOutcome::Stopped);
                                    }
                                }
                            }
                        }
                    });

                    // Report the outcome of the start phase specifically,
                    // without waiting for the (possibly unending) run
                    // phase. A dropped sender (Err from phase_rx) means the
                    // task panicked before reaching either send, i.e. a
                    // panic during construction or `start()`.
                    let start_phase_ok = match phase_rx.await {
                        Ok(Ok(())) => true,
                        Ok(Err(_)) => false,
                        Err(_) => false,
                    };

                    if start_phase_ok {
                        *health.write().unwrap() = ServiceHealth::Healthy;
                        if let Some(tx) = started_tx.take() {
                            let _ = tx.send(Ok(()));
                        }
                    }

                    // Now await the full attempt (blocks until run() ends,
                    // panics, or the service is stopped). This is the
                    // supervision half; it runs in the background and does
                    // not block callers of start_all() because they only
                    // waited on `phase_rx` above via `started_rx`.
                    let attempt_result = attempt_task.await;

                    match attempt_result {
                        Ok(Ok(AttemptOutcome::Stopped)) => {
                            *health.write().unwrap() = ServiceHealth::Healthy;
                            break;
                        }
                        Ok(Ok(AttemptOutcome::Completed)) => {
                            *health.write().unwrap() = ServiceHealth::Healthy;
                            break;
                        }
                        Ok(Err(service_err)) => {
                            let reason = service_err.to_string();
                            attempt += 1;
                            restart_count.store(attempt, Ordering::SeqCst);
                            if attempt > MAX_RESTART_ATTEMPTS {
                                let msg = format!(
                                    "gave up after {attempt} attempts, last error: {reason}"
                                );
                                *health.write().unwrap() = ServiceHealth::Failed {
                                    reason: msg.clone(),
                                    since_ms: started_at.elapsed().as_millis() as u64,
                                };
                                if let Some(tx) = started_tx.take() {
                                    let _ = tx.send(Err(ServiceError::RestartBudgetExceeded(
                                        name.to_string(),
                                        msg,
                                    )));
                                }
                                break;
                            }
                            *health.write().unwrap() = ServiceHealth::Degraded {
                                reason,
                                since_ms: started_at.elapsed().as_millis() as u64,
                            };
                            if stop_requested.load(Ordering::SeqCst) {
                                break;
                            }
                            continue;
                        }
                        Err(join_err) => {
                            // Panic (or cancellation) inside the attempt task,
                            // whether during construct/start or during run().
                            attempt += 1;
                            restart_count.store(attempt, Ordering::SeqCst);
                            let reason = if join_err.is_panic() {
                                format!("panicked: {join_err}")
                            } else {
                                format!("task ended abnormally: {join_err}")
                            };
                            if attempt > MAX_RESTART_ATTEMPTS {
                                *health.write().unwrap() = ServiceHealth::Failed {
                                    reason: reason.clone(),
                                    since_ms: started_at.elapsed().as_millis() as u64,
                                };
                                if let Some(tx) = started_tx.take() {
                                    let _ = tx.send(Err(ServiceError::RestartBudgetExceeded(
                                        name.to_string(),
                                        reason,
                                    )));
                                }
                                break;
                            }
                            *health.write().unwrap() = ServiceHealth::Degraded {
                                reason,
                                since_ms: started_at.elapsed().as_millis() as u64,
                            };
                            if stop_requested.load(Ordering::SeqCst) {
                                break;
                            }
                            continue;
                        }
                    }
                }
            })
        };

        // Wait for the start phase of the first attempt to either succeed
        // or exhaust its restart budget before returning from
        // start_all(), so callers see a definitive start-up result while
        // ongoing run-phase supervision continues independently in the
        // background.
        match started_rx.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                // Sender dropped without sending: treat as started, health
                // reflects the real state.
            }
        }

        self.running.insert(
            name,
            RunningService {
                handle,
                stop_requested,
                stop_notify,
                join,
            },
        );
        Ok(())
    }

    /// Stop every running singleton in reverse registration order
    /// (REQ-ARCH-002.4).
    pub async fn stop_all(&mut self) -> Result<(), ServiceError> {
        let mut first_err = None;
        let order: Vec<&'static str> = self.registration_order.iter().rev().copied().collect();
        for name in order {
            if let Some(running) = self.running.remove(name)
                && let Err(e) = Self::stop_one(name, running).await
                && first_err.is_none()
            {
                first_err = Some(e);
            }
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    async fn stop_one(name: &'static str, running: RunningService) -> Result<(), ServiceError> {
        running.stop_requested.store(true, Ordering::SeqCst);
        running.stop_notify.notify_waiters();
        match tokio::time::timeout(Duration::from_secs(5), running.join).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(join_err)) => Err(ServiceError::StopFailed(
                name.to_string(),
                format!("supervisor task ended abnormally: {join_err}"),
            )),
            Err(_) => Err(ServiceError::StopFailed(
                name.to_string(),
                "timed out waiting for shutdown".to_string(),
            )),
        }
    }

    /// Resolve a fresh transient instance. The container does not
    /// supervise it; the caller owns start/run/stop.
    pub fn resolve_transient(&self, name: &str) -> Result<Box<dyn ManagedService>, ServiceError> {
        let descriptor = self
            .descriptors
            .get(name)
            .ok_or_else(|| ServiceError::NotFound(name.to_string()))?;
        if descriptor.lifetime != Lifetime::Transient {
            return Err(ServiceError::NotFound(format!(
                "{name} is not registered as Transient"
            )));
        }
        (descriptor.factory)(&self.ctx)
    }

    /// The handle of a running singleton, if started.
    pub fn handle(&self, name: &str) -> Option<ManagedHandle> {
        self.running.get(name).map(|r| r.handle.clone())
    }

    /// Snapshot of every running singleton's health, keyed by name.
    pub fn health_summary(&self) -> HashMap<&'static str, ServiceHealth> {
        self.running
            .iter()
            .map(|(name, running)| (*name, running.handle.health()))
            .collect()
    }

    pub fn context(&self) -> &ServiceContext {
        &self.ctx
    }

    pub fn is_registered(&self, name: &str) -> bool {
        self.descriptors.contains_key(name)
    }

    pub fn registration_order(&self) -> &[&'static str] {
        &self.registration_order
    }
}

impl ServiceProvider for ServiceContainer {
    fn health_of(&self, name: &str) -> Option<ServiceHealth> {
        self.handle(name).map(|h| h.health())
    }

    fn metrics_of(&self, name: &str) -> Option<ServiceMetrics> {
        self.handle(name).map(|h| h.metrics())
    }

    fn resolve_any(&self, type_id: TypeId) -> Option<Arc<dyn Any + Send + Sync>> {
        self.ctx.resources.read().unwrap().get(&type_id).cloned()
    }
}

enum AttemptOutcome {
    Completed,
    Stopped,
}

/// A trivial in-memory [`ServiceProvider`] a test can populate directly,
/// so consumers written against `&dyn ServiceProvider` can be tested
/// without constructing a real [`ServiceContainer`].
#[derive(Default)]
pub struct MockServiceProvider {
    health: SyncRwLock<HashMap<String, ServiceHealth>>,
    metrics: SyncRwLock<HashMap<String, ServiceMetrics>>,
    resources: SyncRwLock<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

impl MockServiceProvider {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_health(&self, name: impl Into<String>, health: ServiceHealth) {
        self.health.write().unwrap().insert(name.into(), health);
    }

    pub fn set_metrics(&self, name: impl Into<String>, metrics: ServiceMetrics) {
        self.metrics.write().unwrap().insert(name.into(), metrics);
    }

    pub fn provide<T: Any + Send + Sync>(&self, value: Arc<T>) {
        self.resources
            .write()
            .unwrap()
            .insert(TypeId::of::<T>(), value as Arc<dyn Any + Send + Sync>);
    }
}

impl ServiceProvider for MockServiceProvider {
    fn health_of(&self, name: &str) -> Option<ServiceHealth> {
        self.health.read().unwrap().get(name).cloned()
    }

    fn metrics_of(&self, name: &str) -> Option<ServiceMetrics> {
        self.metrics.read().unwrap().get(name).cloned()
    }

    fn resolve_any(&self, type_id: TypeId) -> Option<Arc<dyn Any + Send + Sync>> {
        self.resources.read().unwrap().get(&type_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    /// A minimal well-behaved service used across several tests.
    struct SimpleService {
        name: &'static str,
        deps: &'static [&'static str],
        started: Arc<AtomicBool>,
        stopped: Arc<AtomicBool>,
    }

    #[async_trait]
    impl Service for SimpleService {
        fn name(&self) -> &'static str {
            self.name
        }
        fn dependencies(&self) -> &'static [&'static str] {
            self.deps
        }
        async fn start(&mut self, _ctx: &ServiceContext) -> Result<(), ServiceError> {
            self.started.store(true, Ordering::SeqCst);
            Ok(())
        }
        async fn stop(&mut self) -> Result<(), ServiceError> {
            self.stopped.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    impl HealthCheck for SimpleService {
        fn health(&self) -> ServiceHealth {
            ServiceHealth::Healthy
        }
        fn metrics(&self) -> ServiceMetrics {
            ServiceMetrics::default()
        }
    }

    fn simple_factory(
        name: &'static str,
        deps: &'static [&'static str],
        started: Arc<AtomicBool>,
        stopped: Arc<AtomicBool>,
    ) -> impl Fn(&ServiceContext) -> Result<Box<dyn ManagedService>, ServiceError> + Send + Sync + 'static
    {
        move |_ctx: &ServiceContext| {
            Ok(Box::new(SimpleService {
                name,
                deps,
                started: started.clone(),
                stopped: stopped.clone(),
            }) as Box<dyn ManagedService>)
        }
    }

    // ---- registration ---------------------------------------------------

    #[test]
    fn registers_a_service() {
        let mut container = ServiceContainer::new();
        let started = Arc::new(AtomicBool::new(false));
        let stopped = Arc::new(AtomicBool::new(false));
        let result = container.register(
            "fs",
            &[],
            Lifetime::Singleton,
            simple_factory("fs", &[], started, stopped),
        );
        assert!(result.is_ok());
        assert!(container.is_registered("fs"));
    }

    #[test]
    fn duplicate_registration_is_rejected() {
        let mut container = ServiceContainer::new();
        let started = Arc::new(AtomicBool::new(false));
        let stopped = Arc::new(AtomicBool::new(false));
        container
            .register(
                "fs",
                &[],
                Lifetime::Singleton,
                simple_factory("fs", &[], started.clone(), stopped.clone()),
            )
            .unwrap();
        let result = container.register(
            "fs",
            &[],
            Lifetime::Singleton,
            simple_factory("fs", &[], started, stopped),
        );
        assert_eq!(result, Err(ServiceError::AlreadyRegistered("fs".into())));
    }

    // ---- resolution / dependency ordering --------------------------------

    #[tokio::test]
    async fn starts_dependencies_before_dependents() {
        let mut container = ServiceContainer::new();
        let started_a = Arc::new(AtomicBool::new(false));
        let started_b = Arc::new(AtomicBool::new(false));
        let stopped_a = Arc::new(AtomicBool::new(false));
        let stopped_b = Arc::new(AtomicBool::new(false));

        // "b" depends on "a".
        container
            .register(
                "a",
                &[],
                Lifetime::Singleton,
                simple_factory("a", &[], started_a.clone(), stopped_a.clone()),
            )
            .unwrap();
        container
            .register(
                "b",
                &["a"],
                Lifetime::Singleton,
                simple_factory("b", &["a"], started_b.clone(), stopped_b.clone()),
            )
            .unwrap();

        let order = container.topo_order().unwrap();
        let pos_a = order.iter().position(|n| *n == "a").unwrap();
        let pos_b = order.iter().position(|n| *n == "b").unwrap();
        assert!(
            pos_a < pos_b,
            "dependency 'a' must start before dependent 'b'"
        );

        container.start_all().await.unwrap();
        assert!(started_a.load(Ordering::SeqCst));
        assert!(started_b.load(Ordering::SeqCst));
        container.stop_all().await.unwrap();
    }

    #[tokio::test]
    async fn missing_dependency_is_reported() {
        let mut container = ServiceContainer::new();
        let started = Arc::new(AtomicBool::new(false));
        let stopped = Arc::new(AtomicBool::new(false));
        container
            .register(
                "b",
                &["ghost"],
                Lifetime::Singleton,
                simple_factory("b", &["ghost"], started, stopped),
            )
            .unwrap();

        let result = container.start_all().await;
        assert_eq!(
            result,
            Err(ServiceError::MissingDependency("b".into(), "ghost".into()))
        );
    }

    // ---- full lifecycle ---------------------------------------------------

    #[tokio::test]
    async fn full_lifecycle_start_and_stop() {
        let mut container = ServiceContainer::new();
        let started = Arc::new(AtomicBool::new(false));
        let stopped = Arc::new(AtomicBool::new(false));
        container
            .register(
                "svc",
                &[],
                Lifetime::Singleton,
                simple_factory("svc", &[], started.clone(), stopped.clone()),
            )
            .unwrap();

        container.start_all().await.unwrap();
        assert!(started.load(Ordering::SeqCst));
        assert_eq!(
            container.health_summary().get("svc"),
            Some(&ServiceHealth::Healthy)
        );

        container.stop_all().await.unwrap();
        assert!(stopped.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn five_services_with_dependencies_start_resolve_and_shut_down_cleanly() {
        // Mirrors the Task 1.2 demo: 5+ services with dependencies,
        // resolved and cleanly shut down in reverse registration order.
        let mut container = ServiceContainer::new();
        let mut starts = Vec::new();
        let mut stops = Vec::new();

        let specs: [(&'static str, &'static [&'static str]); 5] = [
            ("config", &[]),
            ("fs", &["config"]),
            ("workspace", &["fs", "config"]),
            ("search", &["workspace"]),
            ("git", &["fs"]),
        ];

        for (name, deps) in specs {
            let started = Arc::new(AtomicBool::new(false));
            let stopped = Arc::new(AtomicBool::new(false));
            container
                .register(
                    name,
                    deps,
                    Lifetime::Singleton,
                    simple_factory(name, deps, started.clone(), stopped.clone()),
                )
                .unwrap();
            starts.push(started);
            stops.push(stopped);
        }

        container.start_all().await.unwrap();
        for s in &starts {
            assert!(s.load(Ordering::SeqCst));
        }

        container.stop_all().await.unwrap();
        for s in &stops {
            assert!(s.load(Ordering::SeqCst));
        }
    }

    // ---- cycle detection ---------------------------------------------------

    #[test]
    fn direct_cycle_is_rejected() {
        let mut container = ServiceContainer::new();
        let started = Arc::new(AtomicBool::new(false));
        let stopped = Arc::new(AtomicBool::new(false));
        container
            .register(
                "a",
                &["b"],
                Lifetime::Singleton,
                simple_factory("a", &["b"], started.clone(), stopped.clone()),
            )
            .unwrap();
        let result = container.register(
            "b",
            &["a"],
            Lifetime::Singleton,
            simple_factory("b", &["a"], started, stopped),
        );
        assert!(matches!(result, Err(ServiceError::CircularDependency(_))));
    }

    #[test]
    fn indirect_cycle_is_rejected() {
        let mut container = ServiceContainer::new();
        let s = || Arc::new(AtomicBool::new(false));
        container
            .register(
                "a",
                &["c"],
                Lifetime::Singleton,
                simple_factory("a", &["c"], s(), s()),
            )
            .unwrap();
        container
            .register(
                "b",
                &["a"],
                Lifetime::Singleton,
                simple_factory("b", &["a"], s(), s()),
            )
            .unwrap();
        let result = container.register(
            "c",
            &["b"],
            Lifetime::Singleton,
            simple_factory("c", &["b"], s(), s()),
        );
        assert!(matches!(result, Err(ServiceError::CircularDependency(_))));
    }

    #[test]
    fn acyclic_graph_is_accepted() {
        let mut container = ServiceContainer::new();
        let s = || Arc::new(AtomicBool::new(false));
        container
            .register(
                "a",
                &[],
                Lifetime::Singleton,
                simple_factory("a", &[], s(), s()),
            )
            .unwrap();
        container
            .register(
                "b",
                &["a"],
                Lifetime::Singleton,
                simple_factory("b", &["a"], s(), s()),
            )
            .unwrap();
        let result = container.register(
            "c",
            &["a", "b"],
            Lifetime::Singleton,
            simple_factory("c", &["a", "b"], s(), s()),
        );
        assert!(result.is_ok());
    }

    // ---- panic recovery / isolated restart --------------------------------

    /// A service that panics on its Nth `run()` call, then behaves.
    struct FlakyService {
        panics_remaining: Arc<AtomicU32>,
    }

    #[async_trait]
    impl Service for FlakyService {
        fn name(&self) -> &'static str {
            "flaky"
        }
        async fn start(&mut self, _ctx: &ServiceContext) -> Result<(), ServiceError> {
            Ok(())
        }
        async fn run(&mut self) -> Result<(), ServiceError> {
            let remaining = self.panics_remaining.load(Ordering::SeqCst);
            if remaining > 0 {
                self.panics_remaining.store(remaining - 1, Ordering::SeqCst);
                panic!("simulated crash in flaky service");
            }
            std::future::pending::<()>().await;
            Ok(())
        }
    }

    impl HealthCheck for FlakyService {
        fn health(&self) -> ServiceHealth {
            ServiceHealth::Healthy
        }
        fn metrics(&self) -> ServiceMetrics {
            ServiceMetrics::default()
        }
    }

    #[tokio::test]
    async fn panicked_service_is_restarted_in_isolation() {
        let mut container = ServiceContainer::new();
        let panics_remaining = Arc::new(AtomicU32::new(1));

        // A healthy, independent service that must remain unaffected.
        let other_started = Arc::new(AtomicBool::new(false));
        let other_stopped = Arc::new(AtomicBool::new(false));
        container
            .register(
                "unrelated",
                &[],
                Lifetime::Singleton,
                simple_factory(
                    "unrelated",
                    &[],
                    other_started.clone(),
                    other_stopped.clone(),
                ),
            )
            .unwrap();

        container
            .register("flaky", &[], Lifetime::Singleton, {
                let panics_remaining = panics_remaining.clone();
                move |_ctx: &ServiceContext| {
                    Ok(Box::new(FlakyService {
                        panics_remaining: panics_remaining.clone(),
                    }) as Box<dyn ManagedService>)
                }
            })
            .unwrap();

        container.start_all().await.unwrap();

        // Give the supervisor time to hit the panic and restart.
        tokio::time::sleep(Duration::from_millis(200)).await;

        assert!(
            other_started.load(Ordering::SeqCst),
            "unrelated service must have started normally"
        );
        let flaky_handle = container.handle("flaky").expect("flaky service handle");
        assert!(
            flaky_handle.restart_count() >= 1,
            "flaky service should have recorded at least one restart"
        );

        // The container itself (and the unrelated service) must still be
        // fully operational after the isolated restart.
        assert!(!other_stopped.load(Ordering::SeqCst));
        container.stop_all().await.unwrap();
        assert!(other_stopped.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn service_exceeding_restart_budget_is_marked_failed() {
        // A service whose `run()` panics repeatedly starts successfully
        // (start_all() does not block on the run loop, matching normal
        // operation), but the background supervisor gives up once the
        // restart budget is exhausted and marks it Failed rather than
        // restarting forever.
        let mut container = ServiceContainer::new();
        let panics_remaining = Arc::new(AtomicU32::new(50));

        container
            .register("always_panics", &[], Lifetime::Singleton, {
                let panics_remaining = panics_remaining.clone();
                move |_ctx: &ServiceContext| {
                    Ok(Box::new(FlakyService {
                        panics_remaining: panics_remaining.clone(),
                    }) as Box<dyn ManagedService>)
                }
            })
            .unwrap();

        container.start_all().await.unwrap();

        let handle = container.handle("always_panics").unwrap();
        let mut attempts = 0;
        while attempts < 50 && !matches!(handle.health(), ServiceHealth::Failed { .. }) {
            tokio::time::sleep(Duration::from_millis(50)).await;
            attempts += 1;
        }

        assert!(
            matches!(handle.health(), ServiceHealth::Failed { .. }),
            "service should be marked Failed after exhausting its restart budget, got {:?}",
            handle.health()
        );
        assert!(handle.restart_count() > MAX_RESTART_ATTEMPTS);
    }

    // ---- ServiceProvider / mock injection ----------------------------------

    #[test]
    fn mock_provider_supplies_health_and_resources_without_a_real_container() {
        let mock = MockServiceProvider::new();
        mock.set_health("fs", ServiceHealth::Healthy);
        mock.provide(Arc::new(42u32));

        let provider: &dyn ServiceProvider = &mock;
        assert_eq!(provider.health_of("fs"), Some(ServiceHealth::Healthy));
        assert_eq!(provider.resolve::<u32>(), Some(Arc::new(42u32)));
        assert_eq!(provider.health_of("missing"), None);
    }

    #[tokio::test]
    async fn real_container_implements_service_provider() {
        let mut container = ServiceContainer::new();
        let started = Arc::new(AtomicBool::new(false));
        let stopped = Arc::new(AtomicBool::new(false));
        container
            .register(
                "svc",
                &[],
                Lifetime::Singleton,
                simple_factory("svc", &[], started, stopped),
            )
            .unwrap();
        container.start_all().await.unwrap();

        let provider: &dyn ServiceProvider = &container;
        assert_eq!(provider.health_of("svc"), Some(ServiceHealth::Healthy));
        assert!(provider.health_of("nonexistent").is_none());
        container.stop_all().await.unwrap();
    }
}
