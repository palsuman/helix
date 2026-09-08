use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use criterion::Criterion;
use helix_core::container::{
    HealthCheck, Lifetime, Service, ServiceContainer, ServiceContext, ServiceError,
};
use helix_core::health::{ServiceHealth, ServiceMetrics};
use helix_fs::{FileSystemService, WriteOptions, testutil::TempDir};
use helix_ipc::{
    InternalRpcClient, InternalRpcRequest, InternalRpcResponse, IpcDispatcher, IpcRequest, PING,
    register_builtins, serve_internal_rpc_request,
};
use helix_log::{LogLevel, Logger};
use tokio::net::TcpListener;

struct StartupService(&'static str);

#[async_trait]
impl Service for StartupService {
    fn name(&self) -> &'static str {
        self.0
    }
    async fn start(&mut self, _context: &ServiceContext) -> Result<(), ServiceError> {
        Ok(())
    }
}

impl HealthCheck for StartupService {
    fn health(&self) -> ServiceHealth {
        ServiceHealth::Healthy
    }
    fn metrics(&self) -> ServiceMetrics {
        ServiceMetrics::default()
    }
}

fn container() -> ServiceContainer {
    let mut container = ServiceContainer::new();
    let mut previous = None;
    for name in ["logging", "config", "files", "workspace", "commands"] {
        let dependencies: Vec<_> = previous.into_iter().collect();
        container
            .register(name, &dependencies, Lifetime::Singleton, move |_| {
                Ok(Box::new(StartupService(name)))
            })
            .unwrap();
        previous = Some(name);
    }
    container
}

fn measure(criterion: &mut Criterion, name: &str) {
    match name {
        "container_startup" => {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            criterion.bench_function(name, |bench| {
                bench.iter_custom(|iterations| {
                    let mut elapsed = Duration::ZERO;
                    for _iteration in 0..iterations {
                        let mut container = container();
                        let started = Instant::now();
                        runtime.block_on(container.start_all()).unwrap();
                        elapsed += started.elapsed();
                        assert_eq!(container.health_summary().len(), 5);
                        runtime.block_on(container.stop_all()).unwrap();
                    }
                    elapsed
                })
            });
        }
        "ipc_round_trip" => {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let (client, server) = runtime.block_on(async {
                let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
                let client = InternalRpcClient::new(
                    listener.local_addr().unwrap().to_string(),
                    "bench-token",
                    "bench-epoch",
                );
                let mut dispatcher = IpcDispatcher::new();
                register_builtins(&mut dispatcher, "bench");
                let dispatcher = Arc::new(dispatcher);
                let server = tokio::spawn(async move {
                    loop {
                        let (socket, _) = listener.accept().await.unwrap();
                        let dispatcher = dispatcher.clone();
                        tokio::spawn(async move {
                            serve_internal_rpc_request(
                                socket,
                                "bench-token",
                                "bench-epoch",
                                dispatcher,
                            )
                            .await
                            .unwrap();
                        });
                    }
                });
                (client, server)
            });
            criterion.bench_function(name, |bench| bench.iter(|| {
                let request = IpcRequest::new(PING, "bench", serde_json::json!({ "message": "ping" }));
                let encoded = serde_json::to_vec(&request).unwrap();
                let decoded = serde_json::from_slice(&encoded).unwrap();
                let response = runtime.block_on(client.call(InternalRpcRequest::Dispatch(decoded), Duration::from_secs(5))).unwrap();
                let encoded = serde_json::to_vec(&response).unwrap();
                let decoded: InternalRpcResponse = serde_json::from_slice(&encoded).unwrap();
                assert!(matches!(decoded, InternalRpcResponse::Dispatch(response) if response.result.as_ref().unwrap()["echo"] == "ping"));
                black_box(encoded);
            }));
            server.abort();
            runtime.block_on(async {
                let _ = server.await;
            });
        }
        "file_read" | "file_write" => {
            let directory = TempDir::new("performance");
            let path = directory.path().join("source.ts");
            let contents = "export const value = 42;\n".repeat(2730);
            std::fs::write(&path, &contents).unwrap();
            let service =
                FileSystemService::with_defaults(Arc::new(Logger::in_memory(LogLevel::Error)));
            if name == "file_read" {
                criterion.bench_function(name, |bench| {
                    bench.iter(|| {
                        let result = service.read(&path).unwrap();
                        assert_eq!(result.text.as_deref(), Some(contents.as_str()));
                        black_box(result);
                    })
                });
            } else {
                criterion.bench_function(name, |bench| {
                    bench.iter(|| {
                        black_box(
                            service
                                .write(&path, WriteOptions::new(contents.clone()))
                                .unwrap(),
                        );
                    })
                });
                assert_eq!(std::fs::read_to_string(&path).unwrap(), contents);
            }
        }
        "config_merge" => {
            let defaults: serde_json::Value = serde_json::Value::Object(
                (0..200)
                    .map(|index| {
                        (
                            format!("section{index}"),
                            serde_json::json!({ "enabled": true, "size": 14, "items": [1, 2, 3] }),
                        )
                    })
                    .collect(),
            );
            let overlay: serde_json::Value = serde_json::Value::Object(
                (0..200)
                    .map(|index| {
                        (
                            format!("section{index}"),
                            serde_json::json!({ "size": 18, "items": [4] }),
                        )
                    })
                    .collect(),
            );
            criterion.bench_function(name, |bench| {
                bench.iter_batched(
                    || defaults.clone(),
                    |mut merged| {
                        helix_config::merge::deep_merge(&mut merged, &overlay);
                        assert_eq!(merged["section0"]["items"], serde_json::json!([4]));
                        black_box(merged);
                    },
                    criterion::BatchSize::PerIteration,
                )
            });
        }
        _ => panic!("unknown benchmark: {name}"),
    }
}

#[cfg(unix)]
fn peak_rss_bytes() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    assert_eq!(
        unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) },
        0
    );
    let peak = unsafe { usage.assume_init() }.ru_maxrss as u64;
    if cfg!(target_os = "macos") {
        peak
    } else {
        peak * 1024
    }
}

#[cfg(windows)]
fn peak_rss_bytes() -> u64 {
    use windows_sys::Win32::System::{
        ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        Threading::GetCurrentProcess,
    };
    let mut counters = std::mem::MaybeUninit::<PROCESS_MEMORY_COUNTERS>::uninit();
    assert_ne!(
        unsafe {
            GetProcessMemoryInfo(
                GetCurrentProcess(),
                counters.as_mut_ptr(),
                std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            )
        },
        0
    );
    unsafe { counters.assume_init() }.PeakWorkingSetSize as u64
}

fn main() {
    let selected = std::env::var("HELIX_BENCH_CASE").ok();
    let cases = [
        "container_startup",
        "ipc_round_trip",
        "file_read",
        "file_write",
        "config_merge",
    ];
    assert!(
        selected.as_deref().is_none_or(|name| cases.contains(&name)),
        "unknown benchmark selection"
    );
    let mut criterion = Criterion::default()
        .sample_size(30)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3))
        .configure_from_args();
    for name in cases {
        if selected.as_deref().is_none_or(|selected| selected == name) {
            measure(&mut criterion, name);
        }
    }
    criterion.final_summary();
    if let Ok(path) = std::env::var("HELIX_BENCH_RSS") {
        assert!(
            selected.is_some(),
            "RSS reports require one benchmark per process"
        );
        std::fs::write(
            path,
            serde_json::json!({ "benchmark": selected, "peakRssBytes": peak_rss_bytes() })
                .to_string(),
        )
        .unwrap();
    }
}
