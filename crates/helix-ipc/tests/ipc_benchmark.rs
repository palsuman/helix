//! Task 1.3 benchmark: IPC round-trip latency for simple commands must stay
//! under 5ms at p95 (REQ-ARCH-003.4, REQ-NFR-001).
//!
//! This measures the dispatcher path — serialize, correlate, dispatch,
//! deserialize — which is the part of the round trip Helix owns. The webview
//! `invoke` bridge on top of it is measured end to end by the E2E suite in
//! Task 3.3, and the same budget is enforced by the Criterion suite and CI
//! gate in Task 3.4. Kept as an ordinary test so the budget is checked on
//! every `cargo test` run rather than only when someone remembers to
//! benchmark.

use std::time::{Duration, Instant};

use helix_ipc::{IpcDispatcher, IpcRequest, PING, register_builtins};

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

    let payload = serde_json::json!({ "message": "ping" });

    for i in 0..WARMUP_ITERATIONS {
        let response = dispatcher
            .dispatch(IpcRequest::new(
                PING,
                format!("warmup-{i}"),
                payload.clone(),
            ))
            .await;
        assert!(response.is_ok());
    }

    let mut samples = Vec::with_capacity(MEASURED_ITERATIONS);
    for i in 0..MEASURED_ITERATIONS {
        // The payload clone is inside the timed region on purpose: a real
        // invocation also pays for serializing its arguments.
        let request = IpcRequest::new(PING, format!("bench-{i}"), payload.clone());
        let started = Instant::now();
        let response = dispatcher.dispatch(request).await;
        samples.push(started.elapsed());
        assert!(response.is_ok(), "benchmark requests must all succeed");
    }

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
