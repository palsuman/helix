//! Task 1.3 integration tests: typed round trip and cancellation of a long
//! command, driven through the same dispatcher the kernel wires to Tauri
//! (REQ-ARCH-003.1, .2, .3).

use std::sync::Arc;
use std::time::{Duration, Instant};

use helix_core::error::{AppError, ErrorCategory};
use helix_ipc::{
    IpcDispatcher, IpcRequest, PING, PingRequest, PingResponse, SLEEP, SleepRequest, SleepResponse,
    register_builtins,
};

const TEST_KERNEL_VERSION: &str = "1.3.0-test";

fn dispatcher() -> Arc<IpcDispatcher> {
    let mut d = IpcDispatcher::new();
    register_builtins(&mut d, TEST_KERNEL_VERSION);
    Arc::new(d)
}

/// Serialize a typed request, dispatch it, and deserialize the typed
/// response — the full path a frontend call takes, minus the webview.
async fn call<Req: serde::Serialize, Res: serde::de::DeserializeOwned>(
    dispatcher: &IpcDispatcher,
    command: &str,
    correlation_id: &str,
    payload: Req,
    timeout_ms: Option<u32>,
) -> Result<Res, AppError> {
    let mut request = IpcRequest::new(
        command,
        correlation_id,
        serde_json::to_value(payload).expect("request payload must serialize"),
    );
    request.timeout_ms = timeout_ms;

    let response = dispatcher.dispatch(request).await;
    assert_eq!(
        response.correlation_id, correlation_id,
        "every response must echo the originating correlation id"
    );

    match (response.result, response.error) {
        (Some(value), None) => {
            Ok(serde_json::from_value(value).expect("response payload must deserialize"))
        }
        (None, Some(error)) => Err(error),
        other => panic!("response must carry exactly one of result/error, got {other:?}"),
    }
}

#[tokio::test]
async fn typed_round_trip_preserves_types_and_correlation() {
    let dispatcher = dispatcher();

    let response: PingResponse = call(
        &dispatcher,
        PING,
        "round-trip-1",
        PingRequest {
            message: "hello kernel".into(),
        },
        None,
    )
    .await
    .expect("ping must succeed");

    assert_eq!(response.echo, "hello kernel");
    assert_eq!(response.kernel_version, TEST_KERNEL_VERSION);
    assert_eq!(dispatcher.inflight_count(), 0);
}

#[tokio::test]
async fn concurrent_commands_do_not_cross_correlation_ids() {
    let dispatcher = dispatcher();

    let mut handles = Vec::new();
    for i in 0..25 {
        let dispatcher = dispatcher.clone();
        handles.push(tokio::spawn(async move {
            let id = format!("concurrent-{i}");
            let response = dispatcher
                .dispatch(IpcRequest::new(
                    PING,
                    id.clone(),
                    serde_json::json!({ "message": format!("msg-{i}") }),
                ))
                .await;
            assert_eq!(response.correlation_id, id);
            assert_eq!(response.result.unwrap()["echo"], format!("msg-{i}"));
        }));
    }
    for handle in handles {
        handle.await.unwrap();
    }
    assert_eq!(dispatcher.inflight_count(), 0);
}

#[tokio::test]
async fn cancelling_a_ten_second_command_aborts_it_within_100ms() {
    // Task 1.3 demo criterion: "cancel aborts a simulated 10s command
    // within 100ms".
    let dispatcher = dispatcher();

    let call_handle = {
        let dispatcher = dispatcher.clone();
        tokio::spawn(async move {
            dispatcher
                .dispatch(IpcRequest::new(
                    SLEEP,
                    "long-running",
                    serde_json::json!({ "duration_ms": 10_000 }),
                ))
                .await
        })
    };

    // Wait for the kernel to register the command as in flight.
    while dispatcher.inflight_count() == 0 {
        tokio::task::yield_now().await;
    }

    let cancelled_at = Instant::now();
    assert!(
        dispatcher.cancel("long-running"),
        "an in-flight correlation id must be cancellable"
    );

    let response = call_handle.await.unwrap();
    let elapsed = cancelled_at.elapsed();

    let error = response
        .error
        .expect("a cancelled command returns an error");
    assert_eq!(error.category, ErrorCategory::Cancelled);
    assert!(
        elapsed < Duration::from_millis(100),
        "cancellation must abort within 100ms, took {elapsed:?}"
    );
    assert_eq!(
        dispatcher.inflight_count(),
        0,
        "cancellation must release the in-flight slot"
    );
    assert_eq!(dispatcher.cancellation_count(), 1);
}

#[tokio::test]
async fn cancelling_one_command_leaves_others_running() {
    let dispatcher = dispatcher();

    let victim = {
        let dispatcher = dispatcher.clone();
        tokio::spawn(async move {
            dispatcher
                .dispatch(IpcRequest::new(
                    SLEEP,
                    "victim",
                    serde_json::json!({ "duration_ms": 10_000 }),
                ))
                .await
        })
    };
    let survivor = {
        let dispatcher = dispatcher.clone();
        tokio::spawn(async move {
            dispatcher
                .dispatch(IpcRequest::new(
                    SLEEP,
                    "survivor",
                    serde_json::json!({ "duration_ms": 40 }),
                ))
                .await
        })
    };

    while dispatcher.inflight_count() < 2 {
        tokio::task::yield_now().await;
    }
    assert!(dispatcher.cancel("victim"));

    let victim = victim.await.unwrap();
    let survivor = survivor.await.unwrap();

    assert_eq!(
        victim.error.unwrap().category,
        ErrorCategory::Cancelled,
        "the targeted command must be cancelled"
    );
    let slept: SleepResponse = serde_json::from_value(survivor.result.expect("survivor completes"))
        .expect("typed response");
    assert_eq!(slept.slept_ms, 40);
}

#[tokio::test]
async fn a_command_exceeding_its_timeout_is_cancelled_kernel_side() {
    let dispatcher = dispatcher();

    let error = call::<SleepRequest, SleepResponse>(
        &dispatcher,
        SLEEP,
        "times-out",
        SleepRequest {
            duration_ms: 10_000,
        },
        Some(40),
    )
    .await
    .expect_err("a 10s command under a 40ms timeout must fail");

    assert_eq!(error.category, ErrorCategory::Timeout);
    assert_eq!(error.code, "TIMEOUT");
    assert!(error.message.contains("40ms"));
    assert_eq!(dispatcher.inflight_count(), 0);
}

#[tokio::test]
async fn error_categories_reach_the_caller_distinctly() {
    // The frontend branches on category, so each one must arrive intact
    // rather than collapsed into a generic failure.
    let mut d = IpcDispatcher::new();
    register_builtins(&mut d, TEST_KERNEL_VERSION);
    d.register("test.transient", |_r: serde_json::Value, _c| async move {
        Err::<serde_json::Value, _>(AppError::transient("LOCKED", "resource busy"))
    });
    d.register("test.permanent", |_r: serde_json::Value, _c| async move {
        Err::<serde_json::Value, _>(AppError::permanent("NOT_FOUND", "no such file"))
    });
    let d = Arc::new(d);

    let transient = d
        .dispatch(IpcRequest::new(
            "test.transient",
            "e1",
            serde_json::json!({}),
        ))
        .await;
    assert_eq!(transient.error.unwrap().category, ErrorCategory::Transient);

    let permanent = d
        .dispatch(IpcRequest::new(
            "test.permanent",
            "e2",
            serde_json::json!({}),
        ))
        .await;
    assert_eq!(permanent.error.unwrap().category, ErrorCategory::Permanent);

    let timeout = d
        .dispatch(
            IpcRequest::new(SLEEP, "e3", serde_json::json!({ "duration_ms": 5_000 }))
                .with_timeout_ms(20),
        )
        .await;
    assert_eq!(timeout.error.unwrap().category, ErrorCategory::Timeout);

    let unknown = d
        .dispatch(IpcRequest::new("test.nope", "e4", serde_json::json!({})))
        .await;
    let unknown = unknown.error.unwrap();
    assert_eq!(unknown.code, "UNKNOWN_COMMAND");
    assert_eq!(unknown.category, ErrorCategory::Permanent);
}

#[tokio::test]
async fn cancel_all_releases_every_in_flight_command_on_shutdown() {
    let dispatcher = dispatcher();

    let mut handles = Vec::new();
    for i in 0..3 {
        let dispatcher = dispatcher.clone();
        handles.push(tokio::spawn(async move {
            dispatcher
                .dispatch(IpcRequest::new(
                    SLEEP,
                    format!("shutdown-{i}"),
                    serde_json::json!({ "duration_ms": 10_000 }),
                ))
                .await
        }));
    }

    while dispatcher.inflight_count() < 3 {
        tokio::task::yield_now().await;
    }
    assert_eq!(dispatcher.cancel_all(), 3);

    for handle in handles {
        let response = handle.await.unwrap();
        assert_eq!(response.error.unwrap().category, ErrorCategory::Cancelled);
    }
    assert_eq!(dispatcher.inflight_count(), 0);
}
