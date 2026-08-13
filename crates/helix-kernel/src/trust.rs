//! Kernel-side wiring for workspace trust (Task 1.13).

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use helix_core::container::{
    HealthCheck, Lifetime, ManagedService, Service, ServiceContainer, ServiceContext, ServiceError,
    ServiceProbe,
};
use helix_core::error::AppError;
use helix_core::health::{ServiceHealth, ServiceMetrics};
use helix_ipc::IpcDispatcher;
use helix_log::{Logger, log_info, log_warn};
use helix_stream::StreamHub;
use helix_trust::commands::{
    LIST, PROBE, REVOKE, SET, SET_TRUST_EVERYTHING, STATUS, TrustEverythingRequest,
    TrustEverythingResponse, TrustListResponse, TrustProbeRequest, TrustProbeResponse,
    TrustRevokeRequest, TrustRevokeResponse, TrustSetRequest, TrustSetResponse, TrustStatusRequest,
    TrustStatusResponse, TrustedFolderEntry,
};
use helix_trust::model::WorkspaceTrustMode;
use helix_trust::store::{StoreHealth, default_store_path};
use helix_trust::{CHANNEL, LOG_SOURCE, TrustError, TrustService};

pub const SERVICE_NAME: &str = "trust";
pub fn build_service(logger: Arc<Logger>) -> Arc<TrustService> {
    let path = default_store_path().unwrap_or_else(|| {
        log_warn!(
            logger,
            LOG_SOURCE,
            "using a temporary trust store because the user data directory could not be resolved"
        );
        std::env::temp_dir().join("helix-trust.json")
    });
    Arc::new(TrustService::new(
        Arc::new(helix_trust::TrustStore::load(path)),
        Some(logger),
    ))
}

pub fn register_commands(dispatcher: &mut IpcDispatcher, trust: Arc<TrustService>) {
    let status_trust = trust.clone();
    dispatcher.register(STATUS, move |req: TrustStatusRequest, _ctx| {
        let trust = status_trust.clone();
        async move { Ok::<TrustStatusResponse, AppError>(build_status(&trust, &req.paths)) }
    });

    let set_trust = trust.clone();
    dispatcher.register(SET, move |req: TrustSetRequest, _ctx| {
        let trust = set_trust.clone();
        async move {
            trust
                .set_decision(Path::new(&req.path), req.decision, req.inherit_to_children)
                .map_err(map_trust_error)?;
            Ok::<TrustSetResponse, AppError>(TrustSetResponse { applied: true })
        }
    });

    let revoke_trust = trust.clone();
    dispatcher.register(REVOKE, move |req: TrustRevokeRequest, _ctx| {
        let trust = revoke_trust.clone();
        async move {
            let terminated = trust
                .revoke(Path::new(&req.path))
                .map_err(map_trust_error)?;
            Ok::<TrustRevokeResponse, AppError>(TrustRevokeResponse {
                revoked: true,
                terminated_processes: terminated.len() as u32,
            })
        }
    });

    let list_trust = trust.clone();
    dispatcher.register(LIST, move |_req: serde_json::Value, _ctx| {
        let trust = list_trust.clone();
        async move {
            let entries = trust
                .list_trusted()
                .into_iter()
                .map(|(path, inherit_to_children)| TrustedFolderEntry {
                    path,
                    inherit_to_children,
                })
                .collect();
            Ok::<TrustListResponse, AppError>(TrustListResponse { entries })
        }
    });

    let everything_trust = trust.clone();
    dispatcher.register(
        SET_TRUST_EVERYTHING,
        move |req: TrustEverythingRequest, _ctx| {
            let trust = everything_trust.clone();
            async move {
                trust
                    .set_trust_everything(req.enabled, req.acknowledged_warning)
                    .map_err(map_trust_error)?;
                Ok::<TrustEverythingResponse, AppError>(TrustEverythingResponse {
                    enabled: trust.trust_everything(),
                })
            }
        },
    );

    let probe_trust = trust.clone();
    dispatcher.register(PROBE, move |req: TrustProbeRequest, _ctx| {
        let trust = probe_trust.clone();
        async move {
            let allowed = trust.require(Path::new(&req.path), req.capability).is_ok();
            Ok::<TrustProbeResponse, AppError>(TrustProbeResponse { allowed })
        }
    });
}

pub fn register(
    container: &mut ServiceContainer,
    trust: Arc<TrustService>,
    logger: Arc<Logger>,
) -> Result<(), ServiceError> {
    let bridge_registered = Arc::new(std::sync::atomic::AtomicBool::new(false));
    container.register(
        SERVICE_NAME,
        &[crate::stream::SERVICE_NAME, crate::workspace::SERVICE_NAME],
        Lifetime::Singleton,
        move |_ctx| {
            Ok(Box::new(TrustKernelService::new(
                trust.clone(),
                logger.clone(),
                bridge_registered.clone(),
            )) as Box<dyn ManagedService>)
        },
    )
}

struct TrustKernelService {
    trust: Arc<TrustService>,
    logger: Arc<Logger>,
    bridge_registered: Arc<std::sync::atomic::AtomicBool>,
}

impl TrustKernelService {
    fn new(
        trust: Arc<TrustService>,
        logger: Arc<Logger>,
        bridge_registered: Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        Self {
            trust,
            logger,
            bridge_registered,
        }
    }
}

#[async_trait]
impl Service for TrustKernelService {
    fn name(&self) -> &'static str {
        SERVICE_NAME
    }

    fn dependencies(&self) -> &'static [&'static str] {
        &[crate::stream::SERVICE_NAME, crate::workspace::SERVICE_NAME]
    }

    async fn start(&mut self, ctx: &ServiceContext) -> Result<(), ServiceError> {
        ctx.publish(self.trust.clone());
        if let Some(hub) = ctx.resolve::<StreamHub>()
            && !self
                .bridge_registered
                .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            let hub = hub.clone();
            self.trust.add_listener(move || {
                hub.publish(CHANNEL, serde_json::json!({ "changed": true }));
            });
        }

        if self.trust.store_health() == StoreHealth::Unavailable {
            log_warn!(
                self.logger,
                LOG_SOURCE,
                "trust store is unreadable; every folder is in Restricted mode"
            );
        } else {
            log_info!(
                self.logger,
                LOG_SOURCE,
                "workspace trust started",
                "enabled" => self.trust.is_enabled(),
                "trust_everything" => self.trust.trust_everything(),
            );
        }
        Ok(())
    }

    async fn stop(&mut self) -> Result<(), ServiceError> {
        log_info!(self.logger, LOG_SOURCE, "workspace trust stopped");
        Ok(())
    }
}

impl HealthCheck for TrustKernelService {
    fn health(&self) -> ServiceHealth {
        if self.trust.store_health() == StoreHealth::Unavailable {
            return ServiceHealth::Degraded {
                reason: "trust store unreadable; fail-closed to Restricted mode".into(),
                since_ms: 0,
            };
        }
        ServiceHealth::Healthy
    }

    fn metrics(&self) -> ServiceMetrics {
        ServiceMetrics::default()
    }

    fn live_probe(&self) -> Option<ServiceProbe> {
        let trust = self.trust.clone();
        Some(ServiceProbe::new(
            move || {
                if trust.store_health() == StoreHealth::Unavailable {
                    ServiceHealth::Degraded {
                        reason: "trust store unreadable".into(),
                        since_ms: 0,
                    }
                } else {
                    ServiceHealth::Healthy
                }
            },
            ServiceMetrics::default,
        ))
    }
}

pub fn build_status(trust: &TrustService, paths: &[String]) -> TrustStatusResponse {
    let roots = paths
        .iter()
        .map(|path| trust.root_status(Path::new(path)))
        .collect::<Vec<_>>();
    let workspace_mode = if paths.is_empty() {
        WorkspaceTrustMode::Trusted
    } else {
        trust.workspace_mode(paths.iter().map(Path::new))
    };
    TrustStatusResponse {
        enabled: trust.is_enabled(),
        trust_everything: trust.trust_everything(),
        store_healthy: trust.store_health() == StoreHealth::Healthy,
        workspace_mode,
        pending_prompts: trust.pending_prompts(paths.iter().map(Path::new)),
        roots,
    }
}

pub fn map_trust_error(error: TrustError) -> AppError {
    match error {
        TrustError::Disabled => {
            AppError::permanent("TRUST_DISABLED", "workspace trust is disabled")
        }
        TrustError::Restricted { path, capability } => AppError::permanent(
            "WORKSPACE_RESTRICTED",
            format!(
                "'{path}' is in Restricted mode. Trust the folder to use {}.",
                capability.label()
            ),
        )
        .with_details(serde_json::json!({
            "path": path,
            "capability": capability,
            "grant_command": SET,
        })),
        TrustError::StoreUnavailable => AppError::permanent(
            "TRUST_STORE_UNAVAILABLE",
            "the trust store is unreadable; every folder is in Restricted mode until it is repaired",
        ),
        TrustError::InvalidPath(message) => AppError::permanent("INVALID_TRUST_PATH", message),
        TrustError::WarningNotAcknowledged => AppError::permanent(
            "TRUST_WARNING_REQUIRED",
            "trust everything requires acknowledging the security warning",
        ),
        TrustError::Storage(message) => AppError::permanent("TRUST_STORAGE_ERROR", message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_ipc::IpcRequest;
    use helix_trust::model::TrustCapability;
    use std::path::Path;
    use std::sync::Arc;

    #[tokio::test]
    async fn probe_refuses_language_servers_in_restricted_mode() {
        let trust = Arc::new(TrustService::in_memory());
        let mut dispatcher = IpcDispatcher::new();
        register_commands(&mut dispatcher, trust);

        let response = dispatcher
            .dispatch(IpcRequest::new(
                PROBE,
                "p1",
                serde_json::json!({
                    "path": "/tmp/untrusted",
                    "capability": "language_server_launch"
                }),
            ))
            .await;
        assert_eq!(response.result.unwrap()["allowed"], false);
    }

    #[tokio::test]
    async fn trusting_a_folder_allows_probes_and_revocation_terminates_processes() {
        let trust = Arc::new(TrustService::in_memory());
        let mut dispatcher = IpcDispatcher::new();
        register_commands(&mut dispatcher, trust.clone());

        trust.trust(Path::new("/tmp/repo"), false).unwrap();
        let _ = trust
            .register_launch_for(
                Path::new("/tmp/repo"),
                TrustCapability::LanguageServerLaunch,
                "mock-lsp",
                || {},
            )
            .unwrap();

        let allowed = dispatcher
            .dispatch(IpcRequest::new(
                PROBE,
                "p2",
                serde_json::json!({
                    "path": "/tmp/repo",
                    "capability": "task_execution"
                }),
            ))
            .await;
        assert_eq!(allowed.result.unwrap()["allowed"], true);

        let revoked = dispatcher
            .dispatch(IpcRequest::new(
                REVOKE,
                "p3",
                serde_json::json!({ "path": "/tmp/repo" }),
            ))
            .await;
        assert_eq!(revoked.result.unwrap()["terminated_processes"], 1);
    }

    #[test]
    fn restricted_errors_include_actionable_details() {
        let error = map_trust_error(TrustError::Restricted {
            path: "/tmp/repo".into(),
            capability: TrustCapability::LanguageServerLaunch,
        });
        assert_eq!(error.code, "WORKSPACE_RESTRICTED");
        assert!(error.message.contains("language servers"));
        assert_eq!(error.details.unwrap()["grant_command"], SET);
    }
}
