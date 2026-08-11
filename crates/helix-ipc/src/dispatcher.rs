//! The typed IPC command dispatcher (REQ-ARCH-003.1-.4).
//!
//! The dispatcher is deliberately transport-agnostic: it knows nothing
//! about Tauri or the Host-to-kernel socket. The kernel RPC server and the
//! integration tests drive the same code path directly.
//!
//! ## Guarantees
//!
//! - **Typed** — handlers are registered with concrete request/response
//!   types; deserialization failures become a `permanent` error rather than
//!   a panic.
//! - **Correlated** — every response echoes the request's correlation ID,
//!   and the ID is the handle used for cancellation.
//! - **Bounded** — every command runs under a timeout: the request's
//!   `timeout_ms` if set, otherwise the dispatcher default (30s).
//! - **Cancellable** — [`IpcDispatcher::cancel`] aborts an in-flight
//!   command by correlation ID. The handler future is dropped, so
//!   kernel-side work stops; cooperative handlers can additionally await
//!   [`CommandContext::cancelled`] to unwind early and clean up.
//! - **Panic-isolated** — a panicking handler becomes a `permanent`
//!   `HANDLER_PANIC` error instead of taking the kernel down.

use std::collections::HashMap;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use helix_core::error::AppError;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::Notify;

use self::panic_guard::catch_unwind;
use crate::envelope::{DEFAULT_TIMEOUT_MS, IpcRequest, IpcResponse};

/// Cooperative cancellation signal handed to every command handler.
///
/// Dropping the handler future already stops its work; this token exists so
/// a handler doing something interruptible (a long sleep, a chunked loop,
/// a child process wait) can notice cancellation at once and release
/// resources deterministically.
#[derive(Clone, Debug, Default)]
pub struct CancelToken {
    flag: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// Mark the token cancelled and wake everyone awaiting it.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    /// Resolves as soon as the token is cancelled. Registering the
    /// notification before re-checking the flag closes the race where
    /// cancellation lands between the check and the await.
    pub async fn cancelled(&self) {
        loop {
            let notified = self.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

/// Per-invocation context passed to a handler.
#[derive(Clone, Debug)]
pub struct CommandContext {
    correlation_id: String,
    command: String,
    timeout: Duration,
    cancel: CancelToken,
}

impl CommandContext {
    pub fn correlation_id(&self) -> &str {
        &self.correlation_id
    }

    pub fn command(&self) -> &str {
        &self.command
    }

    /// The effective timeout this invocation runs under.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    pub fn cancel_token(&self) -> &CancelToken {
        &self.cancel
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Resolves when this invocation is cancelled by the frontend or times
    /// out. Handlers `select!` on this to abort promptly.
    pub async fn cancelled(&self) {
        self.cancel.cancelled().await;
    }
}

type HandlerFuture = Pin<Box<dyn Future<Output = Result<serde_json::Value, AppError>> + Send>>;
type Handler = Arc<dyn Fn(serde_json::Value, CommandContext) -> HandlerFuture + Send + Sync>;

/// Counters backing the dispatcher's health report.
#[derive(Debug, Default)]
struct Counters {
    requests: AtomicU64,
    errors: AtomicU64,
    timeouts: AtomicU64,
    cancellations: AtomicU64,
}

/// Registry and executor for typed IPC commands.
///
/// Handlers are registered up front (`&mut self`), then the dispatcher is
/// shared behind an `Arc` for the life of the kernel and dispatches
/// concurrently (`&self`).
pub struct IpcDispatcher {
    handlers: HashMap<String, Handler>,
    inflight: Mutex<HashMap<String, CancelToken>>,
    default_timeout_ms: u32,
    counters: Counters,
}

impl Default for IpcDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl IpcDispatcher {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
            inflight: Mutex::new(HashMap::new()),
            default_timeout_ms: DEFAULT_TIMEOUT_MS,
            counters: Counters::default(),
        }
    }

    /// Override the default per-command timeout (REQ-ARCH-003.3). A
    /// `timeout_ms` on an individual request still wins over this.
    pub fn with_default_timeout_ms(mut self, timeout_ms: u32) -> Self {
        self.default_timeout_ms = timeout_ms;
        self
    }

    pub fn default_timeout_ms(&self) -> u32 {
        self.default_timeout_ms
    }

    /// Register a typed command handler. Later registrations of the same
    /// command name replace earlier ones, which is what dynamic
    /// (plugin-contributed) registration in later phases needs.
    pub fn register<Req, Res, F, Fut>(&mut self, command: impl Into<String>, handler: F)
    where
        Req: DeserializeOwned + Send + 'static,
        Res: Serialize + Send + 'static,
        F: Fn(Req, CommandContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Res, AppError>> + Send + 'static,
    {
        let command = command.into();
        let command_for_errors = command.clone();
        let handler = Arc::new(handler);

        let erased: Handler = Arc::new(move |payload, ctx| {
            let handler = handler.clone();
            let command = command_for_errors.clone();
            Box::pin(async move {
                let request: Req = serde_json::from_value(payload).map_err(|e| {
                    AppError::permanent(
                        "INVALID_PAYLOAD",
                        format!("command '{command}' received an unusable payload: {e}"),
                    )
                })?;
                let response = handler(request, ctx).await?;
                serde_json::to_value(response).map_err(|e| {
                    AppError::permanent(
                        "SERIALIZATION_FAILED",
                        format!("command '{command}' produced an unserializable response: {e}"),
                    )
                })
            })
        });

        self.handlers.insert(command, erased);
    }

    /// Command names currently registered, sorted, for diagnostics and the
    /// contract tests in Task 3.5.
    pub fn commands(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.handlers.keys().map(String::as_str).collect();
        names.sort_unstable();
        names
    }

    pub fn is_registered(&self, command: &str) -> bool {
        self.handlers.contains_key(command)
    }

    /// Execute a command, resolving to a typed response. Never panics and
    /// never returns without echoing the correlation ID.
    pub async fn dispatch(
        &self,
        request: IpcRequest<serde_json::Value>,
    ) -> IpcResponse<serde_json::Value> {
        let IpcRequest {
            command,
            correlation_id,
            payload,
            timeout_ms,
        } = request;

        self.counters.requests.fetch_add(1, Ordering::Relaxed);

        let Some(handler) = self.handlers.get(&command).cloned() else {
            return self.fail(
                correlation_id,
                AppError::permanent(
                    "UNKNOWN_COMMAND",
                    format!("no handler registered for command '{command}'"),
                ),
            );
        };

        let cancel = CancelToken::new();
        {
            let mut inflight = self.inflight.lock().unwrap();
            if inflight.contains_key(&correlation_id) {
                return self.fail(
                    correlation_id.clone(),
                    AppError::permanent(
                        "DUPLICATE_CORRELATION_ID",
                        format!("correlation id '{correlation_id}' is already in flight"),
                    ),
                );
            }
            inflight.insert(correlation_id.clone(), cancel.clone());
        }

        // 0 is treated as "unset" so a frontend that forgets to populate the
        // field cannot accidentally request a zero-length timeout.
        let effective_ms = match timeout_ms {
            Some(ms) if ms > 0 => ms,
            _ => self.default_timeout_ms,
        };
        let timeout = Duration::from_millis(u64::from(effective_ms));

        let ctx = CommandContext {
            correlation_id: correlation_id.clone(),
            command: command.clone(),
            timeout,
            cancel: cancel.clone(),
        };

        // A panicking handler must not unwind into the transport or take the
        // kernel with it (REQ-ARCH-003 failure modes: malformed input is
        // logged and discarded, never fatal).
        //
        // The handler runs inside a correlation scope, so every log record a
        // kernel service emits while serving this command carries this
        // command's correlation ID without the service having to know one
        // exists (REQ-OBS-001.9). Boxing the scoped future keeps it `Unpin`,
        // which is what `catch_unwind` needs.
        let scoped: Pin<Box<dyn Future<Output = Result<serde_json::Value, AppError>> + Send>> =
            Box::pin(helix_log::correlation::scope(
                correlation_id.clone(),
                handler(payload, ctx),
            ));
        let invocation = catch_unwind(AssertUnwindSafe(scoped));

        let outcome = tokio::select! {
            biased;
            _ = cancel.cancelled() => Outcome::Cancelled,
            result = invocation => match result {
                Ok(Ok(value)) => Outcome::Done(value),
                Ok(Err(err)) => Outcome::Failed(err),
                Err(panic_message) => Outcome::Panicked(panic_message),
            },
            _ = tokio::time::sleep(timeout) => Outcome::TimedOut,
        };

        self.inflight.lock().unwrap().remove(&correlation_id);

        match outcome {
            Outcome::Done(value) => IpcResponse::ok(correlation_id, value),
            Outcome::Failed(err) => self.fail(correlation_id, err),
            Outcome::Panicked(message) => self.fail(
                correlation_id,
                AppError::permanent(
                    "HANDLER_PANIC",
                    format!("command '{command}' panicked: {message}"),
                ),
            ),
            Outcome::Cancelled => {
                self.counters.cancellations.fetch_add(1, Ordering::Relaxed);
                self.fail(
                    correlation_id,
                    AppError::cancelled(format!("command '{command}' was cancelled by the client")),
                )
            }
            Outcome::TimedOut => {
                // Signal the token as well: work the handler spawned
                // elsewhere observes the same abort as an explicit cancel.
                cancel.cancel();
                self.counters.timeouts.fetch_add(1, Ordering::Relaxed);
                self.fail(
                    correlation_id,
                    AppError::timeout(format!(
                        "command '{command}' exceeded its {effective_ms}ms timeout and was cancelled kernel-side"
                    )),
                )
            }
        }
    }

    /// Abort an in-flight command by correlation ID (REQ-ARCH-003.2).
    /// Returns false when the ID is not in flight, which is the benign race
    /// of a command that already completed.
    pub fn cancel(&self, correlation_id: &str) -> bool {
        let token = self.inflight.lock().unwrap().get(correlation_id).cloned();
        match token {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    /// Cancel everything in flight, used on shutdown.
    pub fn cancel_all(&self) -> usize {
        let tokens: Vec<CancelToken> = self.inflight.lock().unwrap().values().cloned().collect();
        for token in &tokens {
            token.cancel();
        }
        tokens.len()
    }

    pub fn inflight_count(&self) -> usize {
        self.inflight.lock().unwrap().len()
    }

    pub fn request_count(&self) -> u64 {
        self.counters.requests.load(Ordering::Relaxed)
    }

    pub fn error_count(&self) -> u64 {
        self.counters.errors.load(Ordering::Relaxed)
    }

    pub fn timeout_count(&self) -> u64 {
        self.counters.timeouts.load(Ordering::Relaxed)
    }

    pub fn cancellation_count(&self) -> u64 {
        self.counters.cancellations.load(Ordering::Relaxed)
    }

    fn fail(&self, correlation_id: String, error: AppError) -> IpcResponse<serde_json::Value> {
        self.counters.errors.fetch_add(1, Ordering::Relaxed);
        IpcResponse::err(correlation_id, error)
    }
}

enum Outcome {
    Done(serde_json::Value),
    Failed(AppError),
    Panicked(String),
    Cancelled,
    TimedOut,
}

/// Minimal `catch_unwind` for futures.
///
/// `futures::FutureExt::catch_unwind` would do this, but pulling the whole
/// `futures` crate in for one combinator is not worth the dependency. The
/// handler futures are already boxed (`Pin<Box<dyn Future>>`), hence `Unpin`,
/// so this needs no unsafe pin projection.
mod panic_guard {
    use std::any::Any;
    use std::future::Future;
    use std::panic::{AssertUnwindSafe, catch_unwind as std_catch_unwind};
    use std::pin::Pin;
    use std::task::{Context, Poll};

    pub struct CatchUnwind<F> {
        future: F,
    }

    /// Wrap a future so a panic during `poll` is returned as
    /// `Err(message)` instead of unwinding into the caller.
    pub fn catch_unwind<F: Future + Unpin>(
        future: AssertUnwindSafe<F>,
    ) -> CatchUnwind<AssertUnwindSafe<F>> {
        CatchUnwind { future }
    }

    impl<F: Future + Unpin> Future for CatchUnwind<AssertUnwindSafe<F>> {
        type Output = Result<F::Output, String>;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            let future = &mut self.get_mut().future.0;
            match std_catch_unwind(AssertUnwindSafe(|| Pin::new(future).poll(cx))) {
                Ok(Poll::Pending) => Poll::Pending,
                Ok(Poll::Ready(value)) => Poll::Ready(Ok(value)),
                Err(payload) => Poll::Ready(Err(describe_panic(payload.as_ref()))),
            }
        }
    }

    fn describe_panic(payload: &(dyn Any + Send)) -> String {
        if let Some(s) = payload.downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown panic payload".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_core::error::ErrorCategory;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, Serialize)]
    struct Echo {
        message: String,
    }

    fn echo_dispatcher() -> IpcDispatcher {
        let mut dispatcher = IpcDispatcher::new();
        dispatcher.register("echo", |req: Echo, _ctx| async move {
            Ok(Echo {
                message: req.message,
            })
        });
        dispatcher
    }

    fn request(command: &str, payload: serde_json::Value) -> IpcRequest<serde_json::Value> {
        IpcRequest::new(command, "corr-1", payload)
    }

    #[tokio::test]
    async fn typed_command_round_trips_and_echoes_the_correlation_id() {
        let dispatcher = echo_dispatcher();
        let response = dispatcher
            .dispatch(request("echo", serde_json::json!({ "message": "hello" })))
            .await;

        assert_eq!(response.correlation_id, "corr-1");
        assert!(response.error.is_none());
        assert_eq!(response.result.unwrap()["message"], "hello");
    }

    #[tokio::test]
    async fn a_handler_runs_inside_the_requests_correlation_scope() {
        // REQ-OBS-001.9: a kernel service that logs while serving a command
        // must produce records attributable to that command, without the
        // service being handed the id explicitly.
        let mut dispatcher = IpcDispatcher::new();
        dispatcher.register("correlated", |_req: serde_json::Value, _ctx| async move {
            Ok::<serde_json::Value, AppError>(serde_json::json!(helix_log::correlation::current()))
        });

        let response = dispatcher
            .dispatch(request("correlated", serde_json::json!({})))
            .await;
        assert_eq!(response.result.unwrap(), "corr-1");
        assert_eq!(
            helix_log::correlation::current(),
            None,
            "the scope must not leak past the invocation"
        );
    }

    #[tokio::test]
    async fn unknown_command_is_a_permanent_error() {
        let dispatcher = echo_dispatcher();
        let response = dispatcher
            .dispatch(request("nope", serde_json::json!({})))
            .await;

        let error = response.error.unwrap();
        assert_eq!(error.code, "UNKNOWN_COMMAND");
        assert_eq!(error.category, ErrorCategory::Permanent);
    }

    #[tokio::test]
    async fn malformed_payload_is_a_permanent_error_not_a_panic() {
        let dispatcher = echo_dispatcher();
        let response = dispatcher
            .dispatch(request("echo", serde_json::json!({ "wrong": 1 })))
            .await;

        let error = response.error.unwrap();
        assert_eq!(error.code, "INVALID_PAYLOAD");
        assert_eq!(error.category, ErrorCategory::Permanent);
    }

    #[tokio::test]
    async fn handler_error_is_surfaced_with_its_category_intact() {
        let mut dispatcher = IpcDispatcher::new();
        dispatcher.register("boom", |_req: serde_json::Value, _ctx| async move {
            Err::<serde_json::Value, _>(AppError::transient("DISK_BUSY", "try again"))
        });

        let response = dispatcher
            .dispatch(request("boom", serde_json::json!({})))
            .await;
        let error = response.error.unwrap();
        assert_eq!(error.code, "DISK_BUSY");
        assert_eq!(error.category, ErrorCategory::Transient);
    }

    #[tokio::test]
    async fn panicking_handler_becomes_an_error_rather_than_killing_the_dispatcher() {
        let mut dispatcher = IpcDispatcher::new();
        dispatcher.register("panics", |_req: serde_json::Value, _ctx| async move {
            panic!("handler exploded");
            #[allow(unreachable_code)]
            Ok::<serde_json::Value, AppError>(serde_json::Value::Null)
        });
        dispatcher.register("fine", |req: Echo, _ctx| async move { Ok(req) });

        let response = dispatcher
            .dispatch(request("panics", serde_json::json!({})))
            .await;
        let error = response.error.unwrap();
        assert_eq!(error.code, "HANDLER_PANIC");
        assert!(error.message.contains("handler exploded"));

        // The dispatcher is still usable afterwards.
        let ok = dispatcher
            .dispatch(request(
                "fine",
                serde_json::json!({ "message": "still here" }),
            ))
            .await;
        assert!(ok.is_ok());
    }

    #[tokio::test]
    async fn request_timeout_overrides_the_default_and_reports_the_timeout_category() {
        let mut dispatcher = IpcDispatcher::new();
        dispatcher.register("slow", |_req: serde_json::Value, _ctx| async move {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok::<serde_json::Value, AppError>(serde_json::Value::Null)
        });

        let response = dispatcher
            .dispatch(request("slow", serde_json::json!({})).with_timeout_ms(30))
            .await;

        let error = response.error.unwrap();
        assert_eq!(error.code, "TIMEOUT");
        assert_eq!(error.category, ErrorCategory::Timeout);
        assert_eq!(dispatcher.timeout_count(), 1);
        assert_eq!(dispatcher.inflight_count(), 0);
    }

    #[tokio::test]
    async fn zero_timeout_falls_back_to_the_dispatcher_default() {
        let dispatcher = echo_dispatcher();
        let mut req = request("echo", serde_json::json!({ "message": "hi" }));
        req.timeout_ms = Some(0);
        let response = dispatcher.dispatch(req).await;
        assert!(response.is_ok());
    }

    #[tokio::test]
    async fn duplicate_correlation_id_is_rejected_while_the_first_is_in_flight() {
        let mut dispatcher = IpcDispatcher::new();
        dispatcher.register(
            "slow",
            |_req: serde_json::Value, ctx: CommandContext| async move {
                ctx.cancelled().await;
                Ok::<serde_json::Value, AppError>(serde_json::Value::Null)
            },
        );
        let dispatcher = Arc::new(dispatcher);

        let first = {
            let dispatcher = dispatcher.clone();
            tokio::spawn(async move {
                dispatcher
                    .dispatch(request("slow", serde_json::json!({})))
                    .await
            })
        };

        // Wait until the first invocation is registered as in flight.
        while dispatcher.inflight_count() == 0 {
            tokio::task::yield_now().await;
        }

        let duplicate = dispatcher
            .dispatch(request("slow", serde_json::json!({})))
            .await;
        assert_eq!(
            duplicate.error.unwrap().code,
            "DUPLICATE_CORRELATION_ID",
            "a second command must not silently hijack an in-flight correlation id"
        );

        assert!(dispatcher.cancel("corr-1"));
        let _ = first.await.unwrap();
    }

    #[tokio::test]
    async fn cancelling_an_unknown_correlation_id_reports_false() {
        let dispatcher = echo_dispatcher();
        assert!(!dispatcher.cancel("never-existed"));
    }

    #[tokio::test]
    async fn counters_track_requests_and_errors() {
        let dispatcher = echo_dispatcher();
        let _ = dispatcher
            .dispatch(request("echo", serde_json::json!({ "message": "a" })))
            .await;
        let _ = dispatcher
            .dispatch(request("missing", serde_json::json!({})))
            .await;

        assert_eq!(dispatcher.request_count(), 2);
        assert_eq!(dispatcher.error_count(), 1);
    }

    #[tokio::test]
    async fn registered_commands_are_listed_sorted() {
        let mut dispatcher = IpcDispatcher::new();
        dispatcher.register("b", |req: Echo, _ctx| async move { Ok(req) });
        dispatcher.register("a", |req: Echo, _ctx| async move { Ok(req) });
        assert_eq!(dispatcher.commands(), vec!["a", "b"]);
        assert!(dispatcher.is_registered("a"));
        assert!(!dispatcher.is_registered("c"));
    }
}
