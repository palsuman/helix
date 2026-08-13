//! Kernel lifecycle wiring for crash-safe state persistence (Task 1.10).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use helix_config::ConfigService;
use helix_core::container::{
    HealthCheck, Lifetime, ManagedService, Service, ServiceContainer, ServiceContext, ServiceError,
    ServiceProbe,
};
use helix_core::health::{ServiceHealth, ServiceMetrics};
use helix_log::{Logger, log_info, log_warn};
use helix_state::{
    StatePersistence, StateStoreConfig, now_ms, prune_stale_state, state_root_directory,
};
use helix_workspace::{WorkspaceEvent, WorkspaceEventKind, WorkspaceListener, WorkspaceService};

pub const SERVICE_NAME: &str = "state";
pub const LOG_SOURCE: &str = "kernel.state";

pub fn build_service(config: &ConfigService) -> Arc<StatePersistence> {
    let wal_interval = config
        .integer_value("files.walIntervalMs")
        .unwrap_or(1_000)
        .max(100) as u64;
    let retention_days = config
        .integer_value("state.retentionDays")
        .unwrap_or(30)
        .max(1) as u64;
    Arc::new(StatePersistence::new(StateStoreConfig {
        wal_interval: Duration::from_millis(wal_interval),
        retention: Duration::from_secs(retention_days * 24 * 60 * 60),
        ..StateStoreConfig::default()
    }))
}

struct StateKernelService {
    state: Arc<StatePersistence>,
    workspace: Arc<WorkspaceService>,
    logger: Arc<Logger>,
    listener_registered: Arc<AtomicBool>,
    retention: Duration,
}

#[async_trait]
impl Service for StateKernelService {
    fn name(&self) -> &'static str {
        SERVICE_NAME
    }

    fn dependencies(&self) -> &'static [&'static str] {
        &[
            crate::workspace::SERVICE_NAME,
            crate::config::SERVICE_NAME,
            crate::log::SERVICE_NAME,
        ]
    }

    async fn start(&mut self, ctx: &ServiceContext) -> Result<(), ServiceError> {
        ctx.publish(self.state.clone());
        if let Some(root) = state_root_directory() {
            match prune_stale_state(&root, now_ms(), self.retention) {
                Ok(removed) if !removed.is_empty() => {
                    log_info!(self.logger, LOG_SOURCE, "expired workspace state pruned", "directories" => removed.len())
                }
                Ok(_) => {}
                Err(error) => {
                    log_warn!(self.logger, LOG_SOURCE, "expired workspace state could not be pruned", "error" => error.to_string())
                }
            }
        }
        for snapshot in self.workspace.snapshots() {
            self.open(&snapshot);
        }
        if !self.listener_registered.swap(true, Ordering::SeqCst) {
            let state = self.state.clone();
            let logger = self.logger.clone();
            let listener: WorkspaceListener = Arc::new(move |event: &WorkspaceEvent| {
                if event.kind == WorkspaceEventKind::Closed && event.torn_down {
                    if let Err(error) = state.close(&event.key) {
                        log_warn!(logger, LOG_SOURCE, "workspace state could not flush on close", "workspace_key" => event.key.clone(), "error" => error.to_string());
                    }
                } else if matches!(
                    event.kind,
                    WorkspaceEventKind::Opened
                        | WorkspaceEventKind::RootsChanged
                        | WorkspaceEventKind::DocumentChanged
                ) && let Some(snapshot) = &event.workspace
                    && let Err(error) = state.open(snapshot)
                {
                    log_warn!(logger, LOG_SOURCE, "workspace state could not be recovered", "workspace_key" => event.key.clone(), "error" => error.to_string());
                }
            });
            self.workspace.add_listener(listener);
        }
        log_info!(self.logger, LOG_SOURCE, "state persistence started", "wal_interval_ms" => 1_000_u64, "snapshot_interval_ms" => 300_000_u64, "open_workspaces" => self.state.workspace_count());
        Ok(())
    }

    async fn run(&mut self) -> Result<(), ServiceError> {
        loop {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let state = self.state.clone();
            let errors = tokio::task::spawn_blocking(move || state.flush_due())
                .await
                .map_err(|error| {
                    ServiceError::StartFailed(SERVICE_NAME.into(), error.to_string())
                })?;
            for (key, error) in errors {
                log_warn!(self.logger, LOG_SOURCE, "state persistence degraded; pending WAL data retained for retry", "workspace_key" => key, "error" => error.to_string(), "priority" => "wal");
            }
        }
    }

    async fn stop(&mut self) -> Result<(), ServiceError> {
        let state = self.state.clone();
        let errors = tokio::task::spawn_blocking(move || state.flush_all())
            .await
            .map_err(|error| ServiceError::StopFailed(SERVICE_NAME.into(), error.to_string()))?;
        if let Some((key, error)) = errors.into_iter().next() {
            return Err(ServiceError::StopFailed(
                SERVICE_NAME.into(),
                format!("workspace {key}: {error}"),
            ));
        }
        Ok(())
    }
}

impl StateKernelService {
    fn open(&self, snapshot: &helix_workspace::WorkspaceSnapshot) {
        match self.state.open(snapshot) {
            Ok(report) => {
                log_info!(self.logger, LOG_SOURCE, "workspace state recovered", "workspace_key" => snapshot.key.clone(), "buffers" => report.session.buffers.len(), "terminals" => report.session.terminals.len(), "agents" => report.session.agents.len(), "discarded_entries" => report.discarded_entries, "snapshot_corrupt" => report.snapshot_corrupt);
            }
            Err(error) => {
                log_warn!(self.logger, LOG_SOURCE, "workspace state could not be recovered", "workspace_key" => snapshot.key.clone(), "error" => error.to_string())
            }
        }
    }
}

impl HealthCheck for StateKernelService {
    fn health(&self) -> ServiceHealth {
        let degraded: Vec<String> = self
            .state
            .statuses()
            .into_iter()
            .filter_map(|(key, status)| {
                status.degraded.then(|| {
                    format!(
                        "{key}: {}",
                        status
                            .last_error
                            .unwrap_or_else(|| "persistence unavailable".into())
                    )
                })
            })
            .collect();
        if degraded.is_empty() {
            ServiceHealth::Healthy
        } else {
            ServiceHealth::Degraded {
                reason: degraded.join("; "),
                since_ms: 0,
            }
        }
    }

    fn metrics(&self) -> ServiceMetrics {
        let statuses = self.state.statuses();
        ServiceMetrics {
            memory_bytes: 0,
            uptime_ms: 0,
            request_count: statuses.values().map(|s| s.wal_entries_written).sum(),
            error_count: statuses
                .values()
                .map(|s| s.corrupt_entries_discarded + u64::from(s.degraded))
                .sum(),
        }
    }

    fn live_probe(&self) -> Option<ServiceProbe> {
        let health_state = self.state.clone();
        let metrics_state = self.state.clone();
        Some(ServiceProbe::new(
            move || {
                let degraded: Vec<String> = health_state
                    .statuses()
                    .into_iter()
                    .filter_map(|(key, status)| {
                        status
                            .degraded
                            .then(|| format!("{key}: persistence unavailable"))
                    })
                    .collect();
                if degraded.is_empty() {
                    ServiceHealth::Healthy
                } else {
                    ServiceHealth::Degraded {
                        reason: degraded.join("; "),
                        since_ms: 0,
                    }
                }
            },
            move || {
                let statuses = metrics_state.statuses();
                ServiceMetrics {
                    memory_bytes: 0,
                    uptime_ms: 0,
                    request_count: statuses.values().map(|s| s.wal_entries_written).sum(),
                    error_count: statuses
                        .values()
                        .map(|s| s.corrupt_entries_discarded + u64::from(s.degraded))
                        .sum(),
                }
            },
        ))
    }
}

pub fn register(
    container: &mut ServiceContainer,
    state: Arc<StatePersistence>,
    workspace: Arc<WorkspaceService>,
    logger: Arc<Logger>,
    config: Arc<ConfigService>,
) -> Result<(), ServiceError> {
    let listener_registered = Arc::new(AtomicBool::new(false));
    let retention_days = config
        .integer_value("state.retentionDays")
        .unwrap_or(30)
        .max(1) as u64;
    let retention = Duration::from_secs(retention_days * 24 * 60 * 60);
    container.register(
        SERVICE_NAME,
        &[
            crate::workspace::SERVICE_NAME,
            crate::config::SERVICE_NAME,
            crate::log::SERVICE_NAME,
        ],
        Lifetime::Singleton,
        move |_ctx| {
            Ok(Box::new(StateKernelService {
                state: state.clone(),
                workspace: workspace.clone(),
                logger: logger.clone(),
                listener_registered: listener_registered.clone(),
                retention,
            }) as Box<dyn ManagedService>)
        },
    )
}
