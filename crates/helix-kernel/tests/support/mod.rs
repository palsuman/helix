use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use helix_fs::testutil::TempDir;
use helix_ipc::{
    InternalRpcClient, InternalRpcRequest, InternalRpcResponse, IpcRequest, KERNEL_EPOCH_ENV,
    KERNEL_LAUNCH_TOKEN_ENV, KERNEL_READY_PREFIX, KernelReady,
};
use helix_stream::{
    ChannelSubscription, StreamControl, StreamEndpoint, StreamEndpointRequest, StreamEnvelope,
    StreamFrame,
};
use serde::{Serialize, de::DeserializeOwned};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};

pub const TIMEOUT: Duration = Duration::from_secs(10);

pub struct TestWorkspace {
    directory: TempDir,
}

impl TestWorkspace {
    pub fn new() -> Self {
        Self {
            directory: TempDir::new("kernel-integration"),
        }
    }

    pub fn root(&self) -> PathBuf {
        self.directory.path().join("workspace")
    }

    pub fn write(&self, relative: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> PathBuf {
        let relative = relative.as_ref();
        assert!(
            relative
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
        );
        let path = self.root().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }

    pub fn assert_file(&self, relative: impl AsRef<Path>, expected: &str) {
        assert_eq!(
            std::fs::read_to_string(self.root().join(relative)).unwrap(),
            expected
        );
    }

    pub fn mock_nx(&self) {
        self.write("nx.json", "{}");
        self.write(
            "apps/fixture/project.json",
            r#"{"name":"fixture-app","root":"apps/fixture"}"#,
        );
        let script = if cfg!(windows) {
            "@echo off\r\necho invoked>mock-nx-called\r\necho [\"fixture-app\"]\r\n"
        } else {
            "#!/bin/sh\nprintf invoked > mock-nx-called\nprintf '[\"fixture-app\"]\\n'\n"
        };
        let name = if cfg!(windows) {
            "node_modules/.bin/nx.cmd"
        } else {
            "node_modules/.bin/nx"
        };
        let path = self.write(name, script);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let _ = path;
    }
}

pub struct KernelHarness {
    child: Child,
    pub ipc: InternalRpcClient,
    pub epoch: String,
    generation: u64,
    next_request: AtomicU64,
    workspace: TestWorkspace,
}

impl KernelHarness {
    pub async fn start(workspace: TestWorkspace) -> Self {
        let (child, ipc, epoch) = Self::spawn(&workspace, 1).await;
        Self {
            child,
            ipc,
            epoch,
            generation: 1,
            next_request: AtomicU64::new(1),
            workspace,
        }
    }

    async fn spawn(
        workspace: &TestWorkspace,
        generation: u64,
    ) -> (Child, InternalRpcClient, String) {
        let home = workspace.directory.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(workspace.root()).unwrap();
        let epoch = format!("integration-{generation}");
        let token = format!("integration-{}-{generation}", std::process::id());
        let mut child = Command::new(env!("CARGO_BIN_EXE_helix-kernel"))
            .env_clear()
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("APPDATA", &home)
            .env("LOCALAPPDATA", &home)
            .env("XDG_CONFIG_HOME", &home)
            .env("XDG_DATA_HOME", &home)
            .env("XDG_STATE_HOME", &home)
            .env("XDG_CACHE_HOME", &home)
            .env("PATH", &home)
            .env("TMPDIR", &home)
            .env("TEMP", &home)
            .env("TMP", &home)
            .env(
                "SystemRoot",
                std::env::var_os("SystemRoot").unwrap_or_default(),
            )
            .env(KERNEL_EPOCH_ENV, &epoch)
            .env(KERNEL_LAUNCH_TOKEN_ENV, &token)
            .current_dir(workspace.root())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .expect("kernel must launch");
        let mut stdout = BufReader::new(child.stdout.take().unwrap()).lines();
        let ready: KernelReady = tokio::time::timeout(TIMEOUT, async {
            loop {
                let line = stdout
                    .next_line()
                    .await
                    .unwrap()
                    .expect("kernel exited before readiness");
                if let Some(json) = line.strip_prefix(KERNEL_READY_PREFIX) {
                    break serde_json::from_str(json).expect("valid kernel readiness record");
                }
            }
        })
        .await
        .expect("kernel startup timed out");
        assert_eq!(ready.epoch, epoch);
        assert_eq!(Some(ready.process_id), child.id());
        tokio::spawn(async move { while let Ok(Some(_)) = stdout.next_line().await {} });
        let ipc = InternalRpcClient::new(format!("127.0.0.1:{}", ready.port), token, epoch.clone());
        (child, ipc, epoch)
    }

    pub async fn command<Request: Serialize, Response: DeserializeOwned>(
        &self,
        command: &str,
        request: Request,
    ) -> Response {
        let correlation = format!(
            "test-{}-{}",
            self.generation,
            self.next_request.fetch_add(1, Ordering::Relaxed)
        );
        let response = self
            .ipc
            .call(
                InternalRpcRequest::Dispatch(IpcRequest::new(
                    command,
                    &correlation,
                    serde_json::to_value(request).unwrap(),
                )),
                TIMEOUT,
            )
            .await
            .expect("RPC transport must succeed");
        let InternalRpcResponse::Dispatch(response) = response else {
            panic!("unexpected RPC response: {response:?}")
        };
        assert_eq!(response.correlation_id, correlation);
        assert!(
            response.error.is_none(),
            "command {command} failed: {:?}",
            response.error
        );
        serde_json::from_value(response.result.expect("command result")).expect("typed response")
    }

    pub async fn crash_and_restart(&mut self) {
        self.child.kill().await.expect("force-kill kernel");
        self.child.wait().await.expect("reap killed kernel");
        self.generation += 1;
        let (child, ipc, epoch) = Self::spawn(&self.workspace, self.generation).await;
        self.child = child;
        self.ipc = ipc;
        self.epoch = epoch;
    }

    pub fn assert_mock_nx_called(&self) {
        assert_eq!(
            std::fs::read_to_string(self.workspace.root().join("mock-nx-called"))
                .unwrap()
                .trim(),
            "invoked"
        );
    }

    pub async fn shutdown(mut self) {
        assert!(matches!(
            self.ipc
                .call(InternalRpcRequest::Shutdown, TIMEOUT)
                .await
                .unwrap(),
            InternalRpcResponse::ShutdownAcknowledged
        ));
        let exit = tokio::time::timeout(TIMEOUT, self.child.wait())
            .await
            .expect("kernel shutdown timed out")
            .unwrap();
        assert!(exit.success(), "kernel exited with {exit}");
    }
}

pub struct StreamClient {
    socket: WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    channel: String,
    last_sequence: Option<u64>,
}

impl StreamClient {
    pub async fn subscribe(kernel: &KernelHarness, channel: &str) -> Self {
        let endpoint: StreamEndpoint = kernel
            .command("stream.endpoint", StreamEndpointRequest {})
            .await;
        tokio::time::timeout(TIMEOUT, async {
            let (socket, _) = tokio_tungstenite::connect_async(&endpoint.url)
                .await
                .unwrap();
            let mut client = Self {
                socket,
                channel: channel.into(),
                last_sequence: None,
            };
            let control = StreamFrame::control(StreamControl::Subscribe {
                channels: vec![ChannelSubscription::new(channel)],
            });
            client
                .socket
                .send(Message::Text(
                    serde_json::to_string(&control).unwrap().into(),
                ))
                .await
                .unwrap();
            match client.next_frame().await {
                StreamFrame::Control(StreamControl::Subscribed { channels }) => {
                    assert_eq!(channels, vec![channel])
                }
                other => panic!("expected subscription acknowledgement, got {other:?}"),
            }
            client
        })
        .await
        .expect("stream subscription timed out")
    }

    async fn next_frame(&mut self) -> StreamFrame {
        loop {
            match self
                .socket
                .next()
                .await
                .expect("stream ended")
                .expect("WebSocket error")
            {
                Message::Text(text) => {
                    let frame = serde_json::from_str(&text).expect("typed stream frame");
                    if !matches!(frame, StreamFrame::Control(StreamControl::Heartbeat { .. })) {
                        return frame;
                    }
                }
                Message::Ping(_) => self.socket.flush().await.unwrap(),
                Message::Close(reason) => panic!("stream closed: {reason:?}"),
                _ => {}
            }
        }
    }

    pub async fn collect_ordered(&mut self, count: usize) -> Vec<StreamEnvelope> {
        tokio::time::timeout(TIMEOUT, async {
            let mut messages = Vec::with_capacity(count);
            while messages.len() < count {
                let StreamFrame::Data(envelope) = self.next_frame().await else {
                    panic!("unexpected stream control during collection");
                };
                assert_eq!(envelope.channel, self.channel);
                if let Some(previous) = self.last_sequence {
                    assert_eq!(
                        envelope.sequence,
                        previous + 1,
                        "stream gap, duplicate, or reordering"
                    );
                }
                self.last_sequence = Some(envelope.sequence);
                messages.push(envelope);
            }
            messages
        })
        .await
        .expect("channel collection timed out")
    }
}
