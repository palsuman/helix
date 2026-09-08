//! IPC contract tests: every built-in command crosses the JSON boundary with
//! the same shapes used by the frontend and kernel transports.

use helix_core::error::AppError;
use helix_ipc::{
    IpcDispatcher, IpcRequest, PING, PingRequest, PingResponse, SLEEP, SleepRequest, SleepResponse,
    register_builtins,
};
use serde::Serialize;
use serde::de::DeserializeOwned;

const KERNEL_VERSION: &str = "contract-test";

fn dispatcher() -> IpcDispatcher {
    let mut dispatcher = IpcDispatcher::new();
    register_builtins(&mut dispatcher, KERNEL_VERSION);
    dispatcher
}

async fn round_trip<Request, Response>(
    dispatcher: &IpcDispatcher,
    command: &str,
    correlation_id: &str,
    request: Request,
) -> Result<Response, AppError>
where
    Request: Serialize,
    Response: DeserializeOwned,
{
    let payload = serde_json::to_value(request).expect("request must serialize");
    let response = dispatcher
        .dispatch(IpcRequest::new(command, correlation_id, payload))
        .await;

    assert_eq!(response.correlation_id, correlation_id);
    match (response.result, response.error) {
        (Some(result), None) => {
            Ok(serde_json::from_value(result).expect("response must deserialize"))
        }
        (None, Some(error)) => Err(error),
        _ => panic!("IPC response must contain exactly one result or error"),
    }
}

#[tokio::test]
async fn every_builtin_command_round_trips_through_json() {
    let dispatcher = dispatcher();

    let ping: PingResponse = round_trip(
        &dispatcher,
        PING,
        "contract-ping",
        PingRequest {
            message: "hello".into(),
        },
    )
    .await
    .expect("ping should succeed");
    assert_eq!(ping.echo, "hello");
    assert_eq!(ping.kernel_version, KERNEL_VERSION);

    let sleep: SleepResponse = round_trip(
        &dispatcher,
        SLEEP,
        "contract-sleep",
        SleepRequest { duration_ms: 1 },
    )
    .await
    .expect("sleep should succeed");
    assert_eq!(sleep.slept_ms, 1);
}

#[tokio::test]
async fn malformed_payloads_return_typed_errors_without_panicking() {
    let dispatcher = dispatcher();

    for (command, payload) in [
        (PING, serde_json::json!({ "message": 42 })),
        (SLEEP, serde_json::json!({ "duration_ms": "later" })),
    ] {
        let response = dispatcher
            .dispatch(IpcRequest::new(
                command,
                format!("malformed-{command}"),
                payload,
            ))
            .await;
        let error = response
            .error
            .expect("malformed payload must return an error");
        assert_eq!(error.code, "INVALID_PAYLOAD");
        assert_eq!(error.category, helix_core::error::ErrorCategory::Permanent);
    }
}
