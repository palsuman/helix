//! Built-in command definitions and their handlers.
//!
//! Two commands exist at this point in the plan, both of which the Task 1.3
//! demo criteria call for: a trivial typed round trip (`ipc.ping`) and a
//! deliberately long-running, cooperatively cancellable command
//! (`ipc.sleep`) that proves cancellation aborts kernel-side work rather
//! than merely abandoning a response. Real domain commands (`file.*`,
//! `config.*`, …) register through the same [`crate::IpcDispatcher::register`]
//! path in later tasks.

use std::time::Duration;

use helix_core::error::AppError;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::dispatcher::{CommandContext, IpcDispatcher};

/// `ipc.ping` request.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct PingRequest {
    pub message: String,
}

/// `ipc.ping` response.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct PingResponse {
    pub echo: String,
    pub kernel_version: String,
}

/// `ipc.sleep` request — a simulated long-running command.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SleepRequest {
    pub duration_ms: u32,
}

/// `ipc.sleep` response, reporting how long the kernel actually slept
/// before returning.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SleepResponse {
    pub slept_ms: u32,
}

pub const PING: &str = "ipc.ping";
pub const SLEEP: &str = "ipc.sleep";

/// Register the built-in commands on a dispatcher.
pub fn register_builtins(dispatcher: &mut IpcDispatcher, kernel_version: &'static str) {
    dispatcher.register(PING, move |req: PingRequest, _ctx| async move {
        Ok(PingResponse {
            echo: req.message,
            kernel_version: kernel_version.to_string(),
        })
    });

    dispatcher.register(SLEEP, |req: SleepRequest, ctx: CommandContext| async move {
        let requested = Duration::from_millis(u64::from(req.duration_ms));
        // Cooperative: the moment the frontend cancels (or the timeout
        // fires) this returns instead of holding the task open for the
        // remainder of the sleep.
        tokio::select! {
            _ = tokio::time::sleep(requested) => Ok(SleepResponse {
                slept_ms: req.duration_ms,
            }),
            _ = ctx.cancelled() => Err(AppError::cancelled(format!(
                "ipc.sleep for {}ms aborted before completing",
                req.duration_ms
            ))),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::IpcRequest;
    use helix_core::error::ErrorCategory;
    use std::sync::Arc;
    use std::time::Instant;

    fn dispatcher() -> IpcDispatcher {
        let mut d = IpcDispatcher::new();
        register_builtins(&mut d, "0.0.0-test");
        d
    }

    #[tokio::test]
    async fn ping_echoes_the_message_and_reports_the_kernel_version() {
        let d = dispatcher();
        let response = d
            .dispatch(IpcRequest::new(
                PING,
                "c1",
                serde_json::json!({ "message": "hello" }),
            ))
            .await;

        let result = response.result.expect("ping should succeed");
        assert_eq!(result["echo"], "hello");
        assert_eq!(result["kernel_version"], "0.0.0-test");
    }

    #[tokio::test]
    async fn sleep_completes_when_left_alone() {
        let d = dispatcher();
        let response = d
            .dispatch(IpcRequest::new(
                SLEEP,
                "c1",
                serde_json::json!({ "duration_ms": 10 }),
            ))
            .await;
        assert_eq!(response.result.unwrap()["slept_ms"], 10);
    }

    #[tokio::test]
    async fn sleep_aborts_promptly_when_cancelled() {
        let d = Arc::new(dispatcher());
        let started = Instant::now();

        let call = {
            let d = d.clone();
            tokio::spawn(async move {
                d.dispatch(IpcRequest::new(
                    SLEEP,
                    "long",
                    serde_json::json!({ "duration_ms": 10_000 }),
                ))
                .await
            })
        };

        while d.inflight_count() == 0 {
            tokio::task::yield_now().await;
        }
        assert!(d.cancel("long"));

        let response = call.await.unwrap();
        let error = response
            .error
            .expect("cancelled command must report an error");
        assert_eq!(error.category, ErrorCategory::Cancelled);
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "cancellation should abort within 100ms, took {:?}",
            started.elapsed()
        );
    }
}
