//! Kernel-side wiring for secret management (Task 1.12).

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
use helix_secrets::backend::default_vault_path;
use helix_secrets::commands::{
    DELETE, EXISTS, LIST, STATUS, STORE, SecretBackendKind, SecretRef, SecretsDeleteRequest,
    SecretsDeleteResponse, SecretsExistsRequest, SecretsExistsResponse, SecretsListRequest,
    SecretsListResponse, SecretsStatusResponse, SecretsStoreRequest, SecretsStoreResponse,
    SecretsUnlockRequest, SecretsUnlockResponse, UNLOCK,
};
use helix_secrets::namespace::SecretCaller;
use helix_secrets::{LOG_SOURCE, SecretError, SecretService};

pub const SERVICE_NAME: &str = "secrets";

pub fn build_service(logger: Arc<Logger>) -> Arc<SecretService> {
    let vault = default_vault_path().unwrap_or_else(|| {
        log_warn!(
            logger,
            LOG_SOURCE,
            "using a temporary secret vault because the platform data directory could not be resolved"
        );
        std::env::temp_dir().join("helix-secrets-vault.json")
    });
    Arc::new(SecretService::with_composite(logger, vault))
}

pub fn register_commands(dispatcher: &mut IpcDispatcher, secrets: Arc<SecretService>) {
    let store_secrets = secrets.clone();
    dispatcher.register(STORE, move |req: SecretsStoreRequest, _ctx| {
        let secrets = store_secrets.clone();
        async move {
            secrets
                .store(
                    &SecretCaller::SettingsUi,
                    &req.namespace,
                    &req.name,
                    &req.value,
                )
                .map_err(map_secret_error)?;
            Ok::<SecretsStoreResponse, AppError>(SecretsStoreResponse { stored: true })
        }
    });

    let delete_secrets = secrets.clone();
    dispatcher.register(DELETE, move |req: SecretsDeleteRequest, _ctx| {
        let secrets = delete_secrets.clone();
        async move {
            secrets
                .delete(&SecretCaller::SettingsUi, &req.namespace, &req.name)
                .map_err(map_secret_error)?;
            Ok::<SecretsDeleteResponse, AppError>(SecretsDeleteResponse { deleted: true })
        }
    });

    let list_secrets = secrets.clone();
    dispatcher.register(LIST, move |req: SecretsListRequest, _ctx| {
        let secrets = list_secrets.clone();
        async move {
            let entries = secrets
                .list(&SecretCaller::SettingsUi, req.namespace.as_deref())
                .map_err(map_secret_error)?
                .into_iter()
                .map(|entry| SecretRef {
                    namespace: entry.namespace,
                    name: entry.name,
                })
                .collect();
            Ok::<SecretsListResponse, AppError>(SecretsListResponse { entries })
        }
    });

    let exists_secrets = secrets.clone();
    dispatcher.register(EXISTS, move |req: SecretsExistsRequest, _ctx| {
        let secrets = exists_secrets.clone();
        async move {
            let exists = secrets
                .exists(&SecretCaller::SettingsUi, &req.namespace, &req.name)
                .map_err(map_secret_error)?;
            Ok::<SecretsExistsResponse, AppError>(SecretsExistsResponse { exists })
        }
    });

    let unlock_secrets = secrets.clone();
    dispatcher.register(UNLOCK, move |req: SecretsUnlockRequest, _ctx| {
        let secrets = unlock_secrets.clone();
        async move {
            secrets
                .unlock_fallback(&req.master_password)
                .map_err(map_secret_error)?;
            Ok::<SecretsUnlockResponse, AppError>(SecretsUnlockResponse { unlocked: true })
        }
    });

    dispatcher.register(STATUS, move |_req: serde_json::Value, _ctx| {
        let secrets = secrets.clone();
        async move {
            Ok::<SecretsStatusResponse, AppError>(SecretsStatusResponse {
                backend: map_backend_kind(secrets.backend_kind()),
                fallback_unlocked: secrets.fallback_unlocked(),
            })
        }
    });
}

pub fn register(
    container: &mut ServiceContainer,
    secrets: Arc<SecretService>,
    logger: Arc<Logger>,
) -> Result<(), ServiceError> {
    container.register(
        SERVICE_NAME,
        &[crate::log::SERVICE_NAME],
        Lifetime::Singleton,
        move |_ctx| {
            Ok(
                Box::new(SecretsKernelService::new(secrets.clone(), logger.clone()))
                    as Box<dyn ManagedService>,
            )
        },
    )
}

struct SecretsKernelService {
    secrets: Arc<SecretService>,
    logger: Arc<Logger>,
}

impl SecretsKernelService {
    fn new(secrets: Arc<SecretService>, logger: Arc<Logger>) -> Self {
        Self { secrets, logger }
    }
}

#[async_trait]
impl Service for SecretsKernelService {
    fn name(&self) -> &'static str {
        SERVICE_NAME
    }

    fn dependencies(&self) -> &'static [&'static str] {
        &[crate::log::SERVICE_NAME]
    }

    async fn start(&mut self, ctx: &ServiceContext) -> Result<(), ServiceError> {
        ctx.publish(self.secrets.clone());
        log_info!(
            self.logger,
            LOG_SOURCE,
            "secret service started",
            "backend" => format!("{:?}", self.secrets.backend_kind()),
        );
        Ok(())
    }

    async fn stop(&mut self) -> Result<(), ServiceError> {
        log_info!(self.logger, LOG_SOURCE, "secret service stopped");
        Ok(())
    }
}

impl HealthCheck for SecretsKernelService {
    fn health(&self) -> ServiceHealth {
        match self.secrets.storage_error() {
            Some(error) => ServiceHealth::Degraded {
                reason: format!("secret metadata store is unreadable: {error}"),
                since_ms: 0,
            },
            None => ServiceHealth::Healthy,
        }
    }

    fn metrics(&self) -> ServiceMetrics {
        ServiceMetrics::default()
    }

    fn live_probe(&self) -> Option<ServiceProbe> {
        let secrets = self.secrets.clone();
        Some(ServiceProbe::new(
            move || {
                if let Some(error) = secrets.storage_error() {
                    ServiceHealth::Degraded {
                        reason: format!("secret metadata store is unreadable: {error}"),
                        since_ms: 0,
                    }
                } else if secrets.fallback_unlocked()
                    || secrets.backend_kind() != helix_secrets::BackendKind::EncryptedFile
                {
                    ServiceHealth::Healthy
                } else {
                    ServiceHealth::Degraded {
                        reason: "encrypted fallback vault is locked".into(),
                        since_ms: 0,
                    }
                }
            },
            ServiceMetrics::default,
        ))
    }
}

fn map_secret_error(error: SecretError) -> AppError {
    match error {
        SecretError::NotFound { namespace, name } => AppError::permanent(
            "SECRET_NOT_FOUND",
            format!(
                "no secret is configured at {namespace}/{name}; add the credential in Settings and retry"
            ),
        ),
        SecretError::NamespaceDenied { namespace } => AppError::permanent(
            "SECRET_NAMESPACE_DENIED",
            format!("this caller cannot access namespace '{namespace}'"),
        ),
        SecretError::KeychainUnavailable { reason } => AppError::transient(
            "KEYCHAIN_UNAVAILABLE",
            format!(
                "the OS keychain is locked, unavailable, or access was denied: {reason}. Unlock the system keychain or grant Helix credential access; if that is not possible, unlock the encrypted fallback vault"
            ),
        ),
        SecretError::FallbackLocked => AppError::transient(
            "SECRET_VAULT_LOCKED",
            "unlock the encrypted fallback vault with secrets.unlock before storing credentials",
        ),
        SecretError::InvalidMasterPassword => AppError::permanent(
            "INVALID_MASTER_PASSWORD",
            "the master password is incorrect",
        ),
        SecretError::InvalidName(message) => AppError::permanent("INVALID_SECRET_NAME", message),
        SecretError::Storage(message) => AppError::permanent("SECRET_STORAGE_ERROR", message),
    }
}

fn map_backend_kind(kind: helix_secrets::BackendKind) -> SecretBackendKind {
    match kind {
        helix_secrets::BackendKind::Keychain => SecretBackendKind::Keychain,
        helix_secrets::BackendKind::EncryptedFile => SecretBackendKind::EncryptedFile,
        helix_secrets::BackendKind::Memory => SecretBackendKind::Memory,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_ipc::IpcRequest;
    use helix_log::{LogLevel, Logger};
    use helix_secrets::HELIX_NAMESPACE;

    fn logger() -> Arc<Logger> {
        Arc::new(Logger::in_memory(LogLevel::Trace))
    }

    fn dispatcher(logger: Arc<Logger>) -> (IpcDispatcher, Arc<SecretService>) {
        let secrets = Arc::new(SecretService::in_memory());
        let mut dispatcher = IpcDispatcher::new();
        register_commands(&mut dispatcher, secrets.clone());
        // Wire logger for redaction tests via composite-less memory service
        let _ = logger;
        (dispatcher, secrets)
    }

    #[tokio::test]
    async fn store_list_and_exists_never_return_the_secret_value() {
        let (dispatcher, _) = dispatcher(logger());
        let store = dispatcher
            .dispatch(IpcRequest::new(
                STORE,
                "s1",
                serde_json::json!({
                    "namespace": HELIX_NAMESPACE,
                    "name": "openai.work",
                    "value": "sk-never-leak-this-value"
                }),
            ))
            .await;
        assert!(store.result.is_some());

        let list = dispatcher
            .dispatch(IpcRequest::new(LIST, "s2", serde_json::json!({})))
            .await;
        let list_result = list.result.unwrap();
        let entries = list_result["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["name"], "openai.work");
        assert!(entries[0].get("value").is_none());

        let exists = dispatcher
            .dispatch(IpcRequest::new(
                EXISTS,
                "s3",
                serde_json::json!({
                    "namespace": HELIX_NAMESPACE,
                    "name": "openai.work"
                }),
            ))
            .await;
        assert_eq!(exists.result.unwrap()["exists"], true);
    }

    #[tokio::test]
    async fn stored_secrets_are_registered_for_log_redaction() {
        let logger = logger();
        let secrets = Arc::new(SecretService::with_memory_logger(logger.clone()));
        secrets
            .store(
                &SecretCaller::SettingsUi,
                HELIX_NAMESPACE,
                "openai.work",
                "sk-redact-me-please",
            )
            .unwrap();
        let mut record = helix_log::LogRecord::new(
            LogLevel::Info,
            "kernel.ai",
            "using sk-redact-me-please for request",
        );
        logger.redactor().redact_record(&mut record);
        assert!(!record.message.contains("sk-redact-me-please"));
        assert!(record.message.contains(helix_log::REDACTED));
    }

    #[tokio::test]
    async fn kernel_can_resolve_a_provider_key_without_an_ipc_get_command() {
        let secrets = Arc::new(SecretService::in_memory());
        secrets
            .store(
                &SecretCaller::SettingsUi,
                HELIX_NAMESPACE,
                "openai.work",
                "sk-internal-only",
            )
            .unwrap();
        assert_eq!(
            secrets.resolve_provider_key("openai.work").unwrap(),
            "sk-internal-only"
        );
    }
}
