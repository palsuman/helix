//! Kernel-side wiring for the WebSocket streaming layer (Task 1.4).
//!
//! The hub and server live in `helix-stream` and know nothing about Tauri or
//! the service container. This module does three things, mirroring
//! [`crate::ipc`]:
//!
//! 1. Binds the server on launch and captures the endpoint the frontend
//!    needs, since the port is assigned by the OS
//!    ([`bind`]).
//! 2. Registers the `stream.endpoint` command so the frontend can discover
//!    that port and its launch token over IPC
//!    ([`register_commands`]).
//! 3. Registers [`StreamService`] as a container-managed singleton so
//!    streaming participates in the Task 1.2 lifecycle: dependency-ordered
//!    start, health reporting, ordered shutdown.
//!
//! The 100Hz demo counter also lives here, as the service's steady-state
//! run loop. It is the Task 1.4 demo criterion, and it is deliberately
//! silent while nothing is subscribed so it costs nothing in a normal
//! session.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use helix_core::container::{
    HealthCheck, Lifetime, ManagedService, Service, ServiceContainer, ServiceContext, ServiceError,
};
use helix_core::error::AppError;
use helix_core::health::{ServiceHealth, ServiceMetrics};
use helix_ipc::IpcDispatcher;
use helix_stream::{
    HEARTBEAT_INTERVAL_MS, HubConfig, MISSED_HEARTBEAT_LIMIT, ServerConfig, StreamEndpoint,
    StreamEndpointRequest, StreamHub, StreamServer,
};

/// Container service name for the streaming layer.
pub const SERVICE_NAME: &str = "stream";

/// IPC command the frontend calls to discover the socket.
pub const ENDPOINT: &str = "stream.endpoint";

/// Channel carrying the Task 1.4 demo counter.
pub const COUNTER_CHANNEL: &str = "demo:counter";

/// Demo counter cadence: 100Hz.
const COUNTER_INTERVAL: Duration = Duration::from_millis(10);

/// The started streaming layer: the hub kernel services publish into, the
/// bound server, and the endpoint description handed to the frontend.
///
/// Cheap to clone (three `Arc`s), because the container's service factory
/// may be invoked again when a service is restarted in isolation and must
/// not re-bind the port the frontend already holds.
#[derive(Clone)]
pub struct StreamRuntime {
    hub: Arc<StreamHub>,
    server: Arc<StreamServer>,
    endpoint: Arc<StreamEndpoint>,
    counter: Arc<AtomicU64>,
}

impl StreamRuntime {
    pub fn hub(&self) -> &Arc<StreamHub> {
        &self.hub
    }

    pub fn endpoint(&self) -> &StreamEndpoint {
        &self.endpoint
    }

    pub fn port(&self) -> u16 {
        self.server.port()
    }
}

/// Bind the streaming server and describe its endpoint.
///
/// Awaited during bootstrap so the port is known before the frontend can
/// ask for it, which removes any race between the webview loading and the
/// listener being ready.
pub async fn bind(
    hub_config: HubConfig,
    server_config: ServerConfig,
) -> Result<StreamRuntime, ServiceError> {
    let default_buffer_depth = hub_config.default_buffer_depth as u32;
    let hub = Arc::new(StreamHub::new(hub_config));
    let server = StreamServer::bind(hub.clone(), server_config)
        .await
        .map_err(|e| {
            ServiceError::StartFailed(
                SERVICE_NAME.to_string(),
                format!("could not bind the streaming server: {e}"),
            )
        })?;

    let endpoint = StreamEndpoint {
        url: server.url(),
        port: server.port(),
        token: server.token().to_string(),
        heartbeat_interval_ms: HEARTBEAT_INTERVAL_MS,
        missed_heartbeat_limit: MISSED_HEARTBEAT_LIMIT,
        default_buffer_depth,
    };

    Ok(StreamRuntime {
        hub,
        server: Arc::new(server),
        endpoint: Arc::new(endpoint),
        counter: Arc::new(AtomicU64::new(0)),
    })
}

/// Register the streaming commands on the kernel's dispatcher.
pub fn register_commands(dispatcher: &mut IpcDispatcher, runtime: &StreamRuntime) {
    let endpoint = runtime.endpoint.clone();
    dispatcher.register(ENDPOINT, move |_req: StreamEndpointRequest, _ctx| {
        let endpoint = endpoint.clone();
        async move { Ok::<StreamEndpoint, AppError>(endpoint.as_ref().clone()) }
    });
}

/// Container-managed wrapper around the streaming layer.
pub struct StreamService {
    runtime: StreamRuntime,
}

impl StreamService {
    pub fn new(runtime: StreamRuntime) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl Service for StreamService {
    fn name(&self) -> &'static str {
        SERVICE_NAME
    }

    async fn start(&mut self, ctx: &ServiceContext) -> Result<(), ServiceError> {
        // Published so later kernel services (terminal, search, diagnostics,
        // health) resolve the same hub the transport is serving rather than
        // creating their own.
        ctx.publish(self.runtime.hub.clone());
        ctx.publish(self.runtime.endpoint.clone());
        Ok(())
    }

    /// Steady state: emit the demo counter at 100Hz while anyone is
    /// listening.
    ///
    /// `MissedTickBehavior::Delay` rather than `Burst`: if the loop is
    /// starved, catching up by publishing a backlog all at once would create
    /// synthetic backpressure that says nothing about the system's real
    /// throughput.
    async fn run(&mut self) -> Result<(), ServiceError> {
        let mut ticker = tokio::time::interval(COUNTER_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if self.runtime.hub.subscriber_count(COUNTER_CHANNEL) == 0 {
                continue;
            }
            let value = self.runtime.counter.fetch_add(1, Ordering::Relaxed) + 1;
            self.runtime.hub.publish(
                COUNTER_CHANNEL,
                serde_json::json!({
                    "value": value,
                    "emitted_at_ms": epoch_millis(),
                }),
            );
        }
    }

    async fn stop(&mut self) -> Result<(), ServiceError> {
        // Stop accepting, then wake every writer so connections close
        // instead of holding the runtime open at exit.
        self.runtime.server.shutdown();
        Ok(())
    }
}

impl HealthCheck for StreamService {
    fn health(&self) -> ServiceHealth {
        let hub = self.runtime.hub.metrics();
        // Dropped messages are a real quality loss for whoever was reading,
        // so the service reports Degraded rather than hiding it behind a
        // metric nobody reads (REQ-OBS-004.3).
        if hub.backpressure_events > 0 {
            return ServiceHealth::Degraded {
                reason: format!(
                    "{} message(s) dropped across {} backpressure event(s)",
                    hub.dropped, hub.backpressure_events
                ),
                since_ms: 0,
            };
        }
        ServiceHealth::Healthy
    }

    fn metrics(&self) -> ServiceMetrics {
        let hub = self.runtime.hub.metrics();
        ServiceMetrics {
            memory_bytes: 0,
            uptime_ms: 0,
            // "Requests" for a streaming service is messages published;
            // "errors" is messages a subscriber lost to backpressure.
            request_count: hub.published,
            error_count: hub.dropped,
        }
    }
}

/// Register [`StreamService`] on a container as a supervised singleton.
pub fn register(
    container: &mut ServiceContainer,
    runtime: StreamRuntime,
) -> Result<(), ServiceError> {
    container.register(SERVICE_NAME, &[], Lifetime::Singleton, move |_ctx| {
        Ok(Box::new(StreamService::new(runtime.clone())) as Box<dyn ManagedService>)
    })
}

fn epoch_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_ipc::IpcRequest;
    use helix_stream::ChannelSubscription;

    async fn runtime() -> StreamRuntime {
        bind(HubConfig::default(), ServerConfig::default())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn binding_produces_an_endpoint_the_frontend_can_use() {
        let runtime = runtime().await;
        let endpoint = runtime.endpoint();

        assert_ne!(endpoint.port, 0, "the OS must assign a concrete port");
        assert!(endpoint.url.starts_with("ws://127.0.0.1:"));
        assert!(
            endpoint.url.contains(&endpoint.token),
            "the URL must carry the launch token"
        );
        assert!(!endpoint.token.is_empty());
        assert_eq!(endpoint.heartbeat_interval_ms, 5_000);
        assert_eq!(endpoint.missed_heartbeat_limit, 3);
        assert_eq!(endpoint.default_buffer_depth, 1_000);
    }

    #[tokio::test]
    async fn two_launches_get_different_ports_and_tokens() {
        let a = runtime().await;
        let b = runtime().await;
        assert_ne!(a.endpoint().port, b.endpoint().port);
        assert_ne!(a.endpoint().token, b.endpoint().token);
    }

    #[tokio::test]
    async fn the_endpoint_command_returns_the_bound_endpoint() {
        let runtime = runtime().await;
        let mut dispatcher = IpcDispatcher::new();
        register_commands(&mut dispatcher, &runtime);

        let response = dispatcher
            .dispatch(IpcRequest::new(ENDPOINT, "e1", serde_json::json!({})))
            .await;

        let result = response.result.expect("stream.endpoint must succeed");
        assert_eq!(result["port"], runtime.endpoint().port);
        assert_eq!(result["token"], runtime.endpoint().token);
    }

    #[tokio::test]
    async fn the_service_publishes_the_hub_and_reports_health() {
        let runtime = runtime().await;
        let mut container = ServiceContainer::new();
        register(&mut container, runtime.clone()).unwrap();
        container.start_all().await.unwrap();

        assert_eq!(
            container.health_summary().get(SERVICE_NAME),
            Some(&ServiceHealth::Healthy)
        );
        let hub = container
            .context()
            .resolve::<StreamHub>()
            .expect("dependents must resolve the same hub the socket serves");
        assert!(Arc::ptr_eq(&hub, runtime.hub()));

        container.stop_all().await.unwrap();
    }

    #[tokio::test]
    async fn the_demo_counter_runs_only_while_something_is_subscribed() {
        let runtime = runtime().await;
        let mut container = ServiceContainer::new();
        register(&mut container, runtime.clone()).unwrap();
        container.start_all().await.unwrap();

        // Nobody is listening: the 100Hz loop must stay silent.
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(runtime.hub().next_sequence(COUNTER_CHANNEL), 1);

        let session = runtime.hub().open_session();
        session.subscribe(&[ChannelSubscription::new(COUNTER_CHANNEL)]);

        let frames = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(frames) = session.next_frames().await {
                    if frames
                        .iter()
                        .any(|f| matches!(f, helix_stream::StreamFrame::Data(_)))
                    {
                        return frames;
                    }
                } else {
                    panic!("session closed unexpectedly");
                }
            }
        })
        .await
        .expect("the counter must start emitting once subscribed");

        assert!(!frames.is_empty());
        container.stop_all().await.unwrap();
    }

    #[tokio::test]
    async fn health_degrades_once_a_subscriber_loses_messages() {
        let runtime = bind(
            HubConfig::default().with_channel_depth("noisy", 2),
            ServerConfig::default(),
        )
        .await
        .unwrap();
        let service = StreamService::new(runtime.clone());
        assert_eq!(service.health(), ServiceHealth::Healthy);

        let session = runtime.hub().open_session();
        session.subscribe(&[ChannelSubscription::new("noisy")]);
        for i in 0..10 {
            runtime.hub().publish("noisy", serde_json::json!(i));
        }
        let _ = session.drain();

        assert!(
            matches!(service.health(), ServiceHealth::Degraded { .. }),
            "dropped messages must be visible in health, not only in a counter"
        );
        assert_eq!(service.metrics().request_count, 10);
        assert_eq!(service.metrics().error_count, 8);
    }
}
