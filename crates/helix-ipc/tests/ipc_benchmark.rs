//! Task 1.3 benchmark: IPC round-trip latency for simple commands must stay
//! under 5ms at p95 (REQ-ARCH-003.4, REQ-NFR-001).
//!
//! This measures WebView-envelope serialization plus the real authenticated
//! Host-to-kernel TCP transport, kernel dispatch, and response
//! deserialization. Kept as an ordinary test so the budget is checked on
//! every `cargo test` run rather than only when someone remembers to
//! benchmark.

use std::sync::Arc;
use std::time::{Duration, Instant};

use helix_ipc::{
    InternalRpcClient, InternalRpcRequest, InternalRpcResponse, IpcDispatcher, IpcRequest, PING,
    register_builtins, serve_internal_rpc_request,
};
use tokio::net::TcpListener;

const WARMUP_ITERATIONS: usize = 200;
const MEASURED_ITERATIONS: usize = 2_000;
const P95_BUDGET: Duration = Duration::from_millis(5);

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    assert!(!sorted.is_empty());
    let rank = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

#[tokio::test]
async fn simple_command_round_trip_p95_is_under_5ms() {
    let mut dispatcher = IpcDispatcher::new();
    register_builtins(&mut dispatcher, "bench");
    let dispatcher = Arc::new(dispatcher);
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let server = {
        let dispatcher = dispatcher.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let dispatcher = dispatcher.clone();
                tokio::spawn(async move {
                    serve_internal_rpc_request(
                        stream,
                        "benchmark-token",
                        "benchmark-epoch",
                        dispatcher,
                    )
                    .await
                    .unwrap();
                });
            }
        })
    };
    let client = InternalRpcClient::new(address, "benchmark-token", "benchmark-epoch");

    let payload = serde_json::json!({ "message": "ping" });

    for i in 0..WARMUP_ITERATIONS {
        let request = IpcRequest::new(PING, format!("warmup-{i}"), payload.clone());
        let webview_frame = serde_json::to_vec(&request).unwrap();
        let request = serde_json::from_slice(&webview_frame).unwrap();
        let response = client
            .call(
                InternalRpcRequest::Dispatch(request),
                Duration::from_secs(1),
            )
            .await
            .unwrap();
        assert!(matches!(
            response,
            InternalRpcResponse::Dispatch(response) if response.is_ok()
        ));
    }

    let mut samples = Vec::with_capacity(MEASURED_ITERATIONS);
    for i in 0..MEASURED_ITERATIONS {
        let request = IpcRequest::new(PING, format!("bench-{i}"), payload.clone());
        let started = Instant::now();
        let webview_frame = serde_json::to_vec(&request).unwrap();
        let request = serde_json::from_slice(&webview_frame).unwrap();
        let response = client
            .call(
                InternalRpcRequest::Dispatch(request),
                Duration::from_secs(1),
            )
            .await;
        samples.push(started.elapsed());
        assert!(
            matches!(
                response,
                Ok(InternalRpcResponse::Dispatch(response)) if response.is_ok()
            ),
            "benchmark requests must all succeed"
        );
    }

    server.abort();

    samples.sort_unstable();
    let p50 = percentile(&samples, 50.0);
    let p95 = percentile(&samples, 95.0);
    let p99 = percentile(&samples, 99.0);
    let max = *samples.last().unwrap();

    println!(
        "IPC round-trip over {MEASURED_ITERATIONS} iterations: \
         p50={p50:?} p95={p95:?} p99={p99:?} max={max:?} (budget: p95 < {P95_BUDGET:?})"
    );

    assert!(
        p95 < P95_BUDGET,
        "IPC round-trip p95 budget exceeded: p95={p95:?} (budget {P95_BUDGET:?}), p50={p50:?}, p99={p99:?}, max={max:?}"
    );
}
