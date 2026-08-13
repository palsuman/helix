use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use helix_ipc::{
    InternalRpcClient, InternalRpcRequest, InternalRpcResponse, KERNEL_CRASH_HANDOFF_ENV,
    KERNEL_EPOCH_ENV, KERNEL_LAUNCH_TOKEN_ENV, KERNEL_READY_PREFIX, KERNEL_SAFE_MODE_ENV,
    KERNEL_SKIP_SESSION_RESTORE_ENV, KernelReady,
};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, Notify, RwLock};
use uuid::Uuid;

use crate::{
    CrashCause, ExitDecision, HEARTBEAT_INTERVAL, MISSED_HEARTBEAT_LIMIT, RESTART_DEADLINE,
    RecoveryAction, RestartPolicy, SupervisorStatus,
};

const READY_TIMEOUT: Duration = Duration::from_secs(10);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(2);
const CLEAN_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const LOG_TAIL_LINES: usize = 20;

type DynError = Box<dyn Error + Send + Sync>;

pub struct KernelConnection {
    pub rpc: InternalRpcClient,
    child: Mutex<Child>,
    epoch: String,
    logs: Arc<Mutex<VecDeque<String>>>,
    crash_handoff: PathBuf,
    #[cfg(feature = "ipc-e2e")]
    pub address: String,
    #[cfg(feature = "ipc-e2e")]
    pub launch_token: String,
}

impl KernelConnection {
    async fn launch(
        kernel_binary: &Path,
        host_state: &Path,
        safe_mode: bool,
        launch_env: &BTreeMap<String, String>,
        launch_args: &[String],
    ) -> Result<Self, DynError> {
        let launch_token = Uuid::new_v4().to_string();
        let epoch = Uuid::new_v4().to_string();
        let crash_handoff = host_state.join(format!("panic-{epoch}.json"));
        std::fs::create_dir_all(host_state)?;
        let mut command = Command::new(kernel_binary);
        command
            .args(launch_args)
            .env(KERNEL_LAUNCH_TOKEN_ENV, &launch_token)
            .env(KERNEL_EPOCH_ENV, &epoch)
            .env(KERNEL_CRASH_HANDOFF_ENV, &crash_handoff)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if safe_mode {
            command
                .env(KERNEL_SAFE_MODE_ENV, "1")
                .env(KERNEL_SKIP_SESSION_RESTORE_ENV, "1");
        }
        command.envs(launch_env);
        let mut child = command.spawn()?;
        let stdout = child.stdout.take().ok_or("kernel stdout was not piped")?;
        let stderr = child.stderr.take().ok_or("kernel stderr was not piped")?;
        let logs = Arc::new(Mutex::new(VecDeque::with_capacity(LOG_TAIL_LINES)));
        let mut reader = BufReader::new(stdout);
        let ready = tokio::time::timeout(READY_TIMEOUT, async {
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).await? == 0 {
                    return Err::<KernelReady, DynError>(
                        "kernel exited before readiness handshake".into(),
                    );
                }
                if let Some(payload) = line.strip_prefix(KERNEL_READY_PREFIX) {
                    return Ok(serde_json::from_str(payload)?);
                }
                remember_line(&logs, line.trim_end()).await;
            }
        })
        .await
        .map_err(|_| "kernel readiness handshake timed out")??;
        if ready.epoch != epoch {
            let _ = child.start_kill();
            return Err("kernel readiness handshake carried a stale epoch".into());
        }
        spawn_log_reader(reader, logs.clone());
        spawn_log_reader(BufReader::new(stderr), logs.clone());
        let address = format!("127.0.0.1:{}", ready.port);
        Ok(Self {
            rpc: InternalRpcClient::new(&address, &launch_token, &epoch),
            child: Mutex::new(child),
            epoch,
            logs,
            crash_handoff,
            #[cfg(feature = "ipc-e2e")]
            address,
            #[cfg(feature = "ipc-e2e")]
            launch_token,
        })
    }

    pub async fn call(
        &self,
        request: InternalRpcRequest,
        timeout: Duration,
    ) -> Result<InternalRpcResponse, String> {
        self.rpc
            .call(request, timeout)
            .await
            .map_err(|error| error.to_string())
    }

    async fn try_wait(&self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.lock().await.try_wait()
    }

    async fn terminate(&self) {
        let _ = self.child.lock().await.start_kill();
    }

    async fn crash_cause(
        &self,
        status: Option<std::process::ExitStatus>,
        missed_heartbeats: u32,
    ) -> CrashCause {
        let panic_message = std::fs::read_to_string(&self.crash_handoff)
            .ok()
            .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
            .and_then(|value| {
                value
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            });
        let exit_code = status.as_ref().and_then(std::process::ExitStatus::code);
        #[cfg(unix)]
        let signal = status
            .as_ref()
            .and_then(std::os::unix::process::ExitStatusExt::signal);
        #[cfg(not(unix))]
        let signal = None;
        sanitize_crash(CrashCause {
            timestamp_ms: now_ms(),
            exit_code,
            signal,
            panic_message,
            missed_heartbeats,
            last_log_lines: self.logs.lock().await.iter().cloned().collect(),
        })
    }
}

/// Live supervisor shared by Tauri invoke forwarding and the monitor task.
pub struct KernelSupervisor {
    kernel_binary: PathBuf,
    host_state: PathBuf,
    connection: RwLock<Option<Arc<KernelConnection>>>,
    status: RwLock<SupervisorStatus>,
    policy: Mutex<RestartPolicy>,
    action: Mutex<Option<RecoveryAction>>,
    action_notify: Notify,
    launch_env: BTreeMap<String, String>,
    launch_args: Vec<String>,
    stopping: AtomicBool,
}

impl KernelSupervisor {
    pub async fn launch(kernel_binary: PathBuf, host_state: PathBuf) -> Arc<Self> {
        Self::launch_with_env(kernel_binary, host_state, BTreeMap::new()).await
    }

    /// Launch with additional child-only environment. Used by process fixtures
    /// without mutating the test runner's global environment.
    pub async fn launch_with_env(
        kernel_binary: PathBuf,
        host_state: PathBuf,
        launch_env: BTreeMap<String, String>,
    ) -> Arc<Self> {
        Self::launch_with_options(kernel_binary, host_state, launch_env, Vec::new()).await
    }

    pub async fn launch_with_options(
        kernel_binary: PathBuf,
        host_state: PathBuf,
        launch_env: BTreeMap<String, String>,
        launch_args: Vec<String>,
    ) -> Arc<Self> {
        let supervisor = Arc::new(Self {
            kernel_binary,
            host_state,
            connection: RwLock::new(None),
            status: RwLock::new(SupervisorStatus::Starting { safe_mode: false }),
            policy: Mutex::new(RestartPolicy::default()),
            action: Mutex::new(None),
            action_notify: Notify::new(),
            launch_env,
            launch_args,
            stopping: AtomicBool::new(false),
        });
        let monitor = supervisor.clone();
        tokio::spawn(async move { monitor.run().await });
        supervisor
    }

    pub async fn status(&self) -> SupervisorStatus {
        self.status.read().await.clone()
    }

    pub async fn wait_until_ready(&self, timeout: Duration) -> Result<(), String> {
        tokio::time::timeout(timeout, async {
            loop {
                match self.status().await {
                    SupervisorStatus::Running { .. } => return Ok(()),
                    SupervisorStatus::RecoveryRequired { cause, .. } => {
                        return Err(cause
                            .panic_message
                            .unwrap_or_else(|| "kernel failed to start".into()));
                    }
                    SupervisorStatus::Stopped => return Err("kernel supervisor stopped".into()),
                    _ => tokio::time::sleep(Duration::from_millis(10)).await,
                }
            }
        })
        .await
        .map_err(|_| "kernel readiness handshake timed out".to_string())?
    }

    pub async fn call(
        &self,
        request: InternalRpcRequest,
        timeout: Duration,
    ) -> Result<InternalRpcResponse, String> {
        let connection = self
            .connection
            .read()
            .await
            .clone()
            .ok_or_else(|| "kernel is not running".to_string())?;
        connection.call(request, timeout).await
    }

    pub async fn recovery_action(&self, action: RecoveryAction) {
        if self.stopping.load(Ordering::SeqCst) {
            return;
        }
        *self.action.lock().await = Some(action);
        self.action_notify.notify_one();
    }

    #[cfg(feature = "ipc-e2e")]
    pub async fn restart_and_probe_stale_peer(&self) -> Result<bool, String> {
        let previous = self
            .connection
            .read()
            .await
            .clone()
            .ok_or("kernel is not running")?;
        let stale_token = previous.launch_token.clone();
        let stale_epoch = previous.epoch.clone();
        previous.terminate().await;
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let current = self.connection.read().await.clone();
                if let Some(current) = current && current.epoch != stale_epoch {
                    let stale = InternalRpcClient::new(&current.address, &stale_token, &stale_epoch);
                    let response = stale.call(
                        InternalRpcRequest::Health,
                        Duration::from_secs(2),
                    ).await.map_err(|error| error.to_string())?;
                    return Ok(matches!(response, InternalRpcResponse::ProtocolError { message } if message.contains("unauthorized or stale")));
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.map_err(|_| "replacement kernel did not become ready".to_string())?
    }

    /// Acknowledged graceful shutdown, followed by a bounded wait and force kill.
    pub async fn shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.action_notify.notify_waiters();
        let connection = self.connection.read().await.clone();
        let Some(connection) = connection else {
            *self.status.write().await = SupervisorStatus::Stopped;
            return;
        };
        if matches!(
            connection
                .call(InternalRpcRequest::Shutdown, Duration::from_secs(2))
                .await,
            Ok(InternalRpcResponse::ShutdownAcknowledged)
        ) {
            self.policy.lock().await.acknowledge_quit();
        }
        let exited = tokio::time::timeout(CLEAN_SHUTDOWN_TIMEOUT, async {
            loop {
                if connection.try_wait().await.ok().flatten().is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .is_ok();
        if !exited {
            connection.terminate().await;
        }
        *self.status.write().await = SupervisorStatus::Stopped;
    }

    async fn run(self: Arc<Self>) {
        loop {
            if self.stopping.load(Ordering::SeqCst) {
                *self.status.write().await = SupervisorStatus::Stopped;
                break;
            }
            let safe_mode = self.policy.lock().await.safe_mode();
            *self.status.write().await = SupervisorStatus::Starting { safe_mode };
            let launched = KernelConnection::launch(
                &self.kernel_binary,
                &self.host_state,
                safe_mode,
                &self.launch_env,
                &self.launch_args,
            )
            .await;
            let connection = match launched {
                Ok(connection) => Arc::new(connection),
                Err(error) => {
                    let safe_mode = self.policy.lock().await.record_start_failure();
                    let cause = CrashCause {
                        timestamp_ms: now_ms(),
                        panic_message: Some(error.to_string()),
                        ..CrashCause::default()
                    };
                    if !self.restart_or_recover(cause, safe_mode).await {
                        break;
                    }
                    continue;
                }
            };
            self.policy.lock().await.record_ready();
            *self.connection.write().await = Some(connection.clone());
            *self.status.write().await = SupervisorStatus::Running {
                safe_mode,
                epoch: connection.epoch.clone(),
            };

            let (status, missed) = self.monitor_connection(&connection).await;
            *self.connection.write().await = None;
            let success = status
                .as_ref()
                .is_some_and(std::process::ExitStatus::success);
            let cause = connection.crash_cause(status, missed).await;
            persist_crash(&self.host_state, &cause);
            if self.stopping.load(Ordering::SeqCst) {
                *self.status.write().await = SupervisorStatus::Stopped;
                break;
            }
            let decision = self.policy.lock().await.classify_exit(now_ms(), success);
            match decision {
                ExitDecision::Stop => {
                    *self.status.write().await = SupervisorStatus::Stopped;
                    break;
                }
                ExitDecision::Restart { safe_mode, attempt } => {
                    *self.status.write().await = SupervisorStatus::Recovering {
                        attempt,
                        safe_mode,
                        cause,
                    };
                    // Spawning starts immediately; the deadline leaves headroom for scheduling.
                    tokio::time::sleep(RESTART_DEADLINE.min(Duration::from_millis(100))).await;
                }
                ExitDecision::ShowRecovery { safe_mode } => {
                    *self.status.write().await =
                        SupervisorStatus::RecoveryRequired { safe_mode, cause };
                    if !self.wait_for_recovery_action().await {
                        break;
                    }
                }
            }
        }
    }

    async fn monitor_connection(
        &self,
        connection: &KernelConnection,
    ) -> (Option<std::process::ExitStatus>, u32) {
        let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
        heartbeat.tick().await;
        let mut exit_poll = tokio::time::interval(Duration::from_millis(25));
        exit_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut missed = 0;
        loop {
            tokio::select! {
                _ = exit_poll.tick() => {
                    if let Ok(Some(status)) = connection.try_wait().await {
                        return (Some(status), missed);
                    }
                }
                _ = heartbeat.tick() => {
                    match connection.call(InternalRpcRequest::Health, HEALTH_TIMEOUT).await {
                        Ok(InternalRpcResponse::Health { epoch }) if epoch == connection.epoch => missed = 0,
                        _ => {
                            missed += 1;
                            if missed >= MISSED_HEARTBEAT_LIMIT {
                                connection.terminate().await;
                                return (None, missed);
                            }
                        }
                    }
                }
            }
        }
    }

    async fn restart_or_recover(&self, cause: CrashCause, safe_mode: bool) -> bool {
        let decision = self.policy.lock().await.classify_exit(now_ms(), false);
        match decision {
            ExitDecision::Restart { attempt, .. } => {
                *self.status.write().await = SupervisorStatus::Recovering {
                    attempt,
                    safe_mode,
                    cause,
                };
                tokio::time::sleep(Duration::from_millis(100)).await;
                true
            }
            ExitDecision::ShowRecovery { .. } => {
                *self.status.write().await =
                    SupervisorStatus::RecoveryRequired { safe_mode, cause };
                self.wait_for_recovery_action().await
            }
            ExitDecision::Stop => false,
        }
    }

    async fn wait_for_recovery_action(&self) -> bool {
        loop {
            self.action_notify.notified().await;
            if self.stopping.load(Ordering::SeqCst) {
                return false;
            }
            let Some(action) = self.action.lock().await.take() else {
                continue;
            };
            match action {
                RecoveryAction::Retry => {
                    self.policy.lock().await.retry_from_ui(false);
                    return true;
                }
                RecoveryAction::StartWithoutSessionRestore => {
                    self.policy.lock().await.retry_from_ui(true);
                    return true;
                }
                RecoveryAction::OpenLogs => open_directory(&self.host_state),
            }
        }
    }
}

fn spawn_log_reader<R>(mut reader: BufReader<R>, logs: Arc<Mutex<VecDeque<String>>>)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line).await {
                Ok(0) | Err(_) => break,
                Ok(_) => remember_line(&logs, line.trim_end()).await,
            }
        }
    });
}

async fn remember_line(logs: &Mutex<VecDeque<String>>, line: &str) {
    let mut logs = logs.lock().await;
    if logs.len() == LOG_TAIL_LINES {
        logs.pop_front();
    }
    logs.push_back(line.to_string());
}

fn persist_crash(root: &Path, cause: &CrashCause) {
    let cause = sanitize_crash(cause.clone());
    if std::fs::create_dir_all(root).is_ok()
        && let Ok(bytes) = serde_json::to_vec_pretty(&cause)
    {
        let _ = std::fs::write(root.join("last-crash.json"), bytes);
    }
}

fn sanitize_crash(mut cause: CrashCause) -> CrashCause {
    let redactor = helix_log::Redactor::new();
    cause.panic_message = cause
        .panic_message
        .map(|message| redactor.redact_text(&message));
    cause.last_log_lines = cause
        .last_log_lines
        .into_iter()
        .map(|line| redactor.redact_text(&line))
        .collect();
    cause
}

pub fn default_host_state_directory() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|p| p.join("Helix").join("state").join("host"))
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(PathBuf::from).map(|p| {
            p.join("Library")
                .join("Application Support")
                .join("Helix")
                .join("state")
                .join("host")
        })
    } else {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|p| p.join(".local").join("state"))
            })
            .map(|p| p.join("helix").join("state").join("host"))
    }
}

fn open_directory(path: &Path) {
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod secret_redaction_tests {
    use super::*;

    #[test]
    fn crash_reports_redact_tokens_before_persistence() {
        let token = "sk-abcdefghijklmnopqrstuvwxyz";
        let sanitized = sanitize_crash(CrashCause {
            panic_message: Some(format!("provider failed with {token}")),
            last_log_lines: vec![format!("Authorization: Bearer {token}")],
            ..CrashCause::default()
        });
        let serialized = serde_json::to_string(&sanitized).unwrap();
        assert!(!serialized.contains(token));
        assert!(serialized.contains(helix_log::REDACTED));
    }
}
