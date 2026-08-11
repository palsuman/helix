//! Kernel-side wiring for the IPC command layer (Task 1.3).
//!
//! The dispatcher itself lives in `helix-ipc` and knows nothing about Tauri.
//! This module does three things:
//!
//! 1. Builds the dispatcher with the built-in commands registered.
//! 2. Registers it as a container-managed singleton ([`IpcService`]) so it
//!    participates in the Task 1.2 lifecycle: dependency-ordered start,
//!    health reporting, ordered shutdown (which cancels anything still in
//!    flight).
//! 3. Exposes the two `#[tauri::command]` entry points the frontend calls:
//!    `ipc_dispatch` and `ipc_cancel`.

use std::sync::Arc;

use async_trait::async_trait;
use helix_core::container::{
    HealthCheck, Lifetime, ManagedService, Service, ServiceContainer, ServiceContext, ServiceError,
};
use helix_core::health::{ServiceHealth, ServiceMetrics};
use helix_ipc::{CancelRequest, CancelResponse, IpcDispatcher, IpcRequest, IpcResponse};

/// Container service name for the IPC layer.
pub const SERVICE_NAME: &str = "ipc";

/// Build the kernel's dispatcher with every built-in command registered.
///
/// Returned unwrapped so the caller can register subsystem commands
/// (`stream.endpoint`, and the `file.*` / `config.*` families in later
/// tasks) before sharing it behind an `Arc` for the life of the process.
pub fn build_dispatcher(kernel_version: &'static str) -> IpcDispatcher {
    let mut dispatcher = IpcDispatcher::new();
    helix_ipc::register_builtins(&mut dispatcher, kernel_version);
    dispatcher
}

/// Container-managed wrapper around the shared dispatcher.
pub struct IpcService {
    dispatcher: Arc<IpcDispatcher>,
}

impl IpcService {
    pub fn new(dispatcher: Arc<IpcDispatcher>) -> Self {
        Self { dispatcher }
    }
}

#[async_trait]
impl Service for IpcService {
    fn name(&self) -> &'static str {
        "ipc"
    }

    async fn start(&mut self, ctx: &ServiceContext) -> Result<(), ServiceError> {
        // Published so later services can dispatch internally and resolve the
        // same instance the transport uses.
        ctx.publish(self.dispatcher.clone());
        Ok(())
    }

    async fn stop(&mut self) -> Result<(), ServiceError> {
        // Anything still in flight at shutdown is aborted rather than left
        // holding resources while the process exits.
        self.dispatcher.cancel_all();
        Ok(())
    }
}

impl HealthCheck for IpcService {
    fn health(&self) -> ServiceHealth {
        ServiceHealth::Healthy
    }

    fn metrics(&self) -> ServiceMetrics {
        ServiceMetrics {
            memory_bytes: 0,
            uptime_ms: 0,
            request_count: self.dispatcher.request_count(),
            error_count: self.dispatcher.error_count(),
        }
    }
}

/// Register [`IpcService`] on a container as a supervised singleton.
pub fn register(
    container: &mut ServiceContainer,
    dispatcher: Arc<IpcDispatcher>,
) -> Result<(), ServiceError> {
    container.register(SERVICE_NAME, &[], Lifetime::Singleton, move |_ctx| {
        Ok(Box::new(IpcService::new(dispatcher.clone())) as Box<dyn ManagedService>)
    })
}

/// Frontend entry point for every command. Returns a typed response
/// envelope; transport-level failures are impossible here by construction,
/// because command failures are carried *in* the envelope rather than as a
/// rejected promise, which is what lets the frontend branch on error
/// category (REQ-ARCH-003.1).
#[tauri::command]
pub async fn ipc_dispatch(
    dispatcher: tauri::State<'_, Arc<IpcDispatcher>>,
    request: IpcRequest<serde_json::Value>,
) -> Result<IpcResponse<serde_json::Value>, String> {
    Ok(dispatcher.dispatch(request).await)
}

/// Frontend entry point for cancellation by correlation ID
/// (REQ-ARCH-003.2).
#[tauri::command]
pub fn ipc_cancel(
    dispatcher: tauri::State<'_, Arc<IpcDispatcher>>,
    request: CancelRequest,
) -> CancelResponse {
    let cancelled = dispatcher.cancel(&request.correlation_id);
    CancelResponse {
        correlation_id: request.correlation_id,
        cancelled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_ipc::PING;

    #[tokio::test]
    async fn ipc_service_publishes_the_dispatcher_and_reports_health() {
        let dispatcher = Arc::new(build_dispatcher("test"));
        let mut container = ServiceContainer::new();
        register(&mut container, dispatcher.clone()).unwrap();

        container.start_all().await.unwrap();
        assert_eq!(
            container.health_summary().get(SERVICE_NAME),
            Some(&ServiceHealth::Healthy)
        );
        assert_eq!(
            container
                .context()
                .resolve::<IpcDispatcher>()
                .map(|d| d.is_registered(PING)),
            Some(true),
            "dependents must be able to resolve the dispatcher from the container context"
        );

        container.stop_all().await.unwrap();
    }

    #[tokio::test]
    async fn built_dispatcher_registers_the_builtin_commands() {
        let dispatcher = Arc::new(build_dispatcher("test"));
        assert_eq!(dispatcher.commands(), vec![PING, helix_ipc::SLEEP]);
        assert_eq!(dispatcher.default_timeout_ms(), 30_000);
    }

    #[tokio::test]
    async fn metrics_reflect_dispatch_activity() {
        let dispatcher = Arc::new(build_dispatcher("test"));
        let service = IpcService::new(dispatcher.clone());

        let _ = dispatcher
            .dispatch(IpcRequest::new(
                PING,
                "m1",
                serde_json::json!({ "message": "hi" }),
            ))
            .await;
        let _ = dispatcher
            .dispatch(IpcRequest::new("nope", "m2", serde_json::json!({})))
            .await;

        let metrics = service.metrics();
        assert_eq!(metrics.request_count, 2);
        assert_eq!(metrics.error_count, 1);
    }

    #[tokio::test]
    async fn stopping_the_service_cancels_in_flight_commands() {
        let dispatcher = Arc::new(build_dispatcher("test"));
        let mut service = IpcService::new(dispatcher.clone());

        let call = {
            let dispatcher = dispatcher.clone();
            tokio::spawn(async move {
                dispatcher
                    .dispatch(IpcRequest::new(
                        helix_ipc::SLEEP,
                        "in-flight",
                        serde_json::json!({ "duration_ms": 10_000 }),
                    ))
                    .await
            })
        };
        while dispatcher.inflight_count() == 0 {
            tokio::task::yield_now().await;
        }

        service.stop().await.unwrap();

        let response = call.await.unwrap();
        assert_eq!(
            response.error.unwrap().category,
            helix_core::error::ErrorCategory::Cancelled
        );
    }
}
