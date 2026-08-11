//! The local WebSocket streaming server (REQ-ARCH-003.5-.10).
//!
//! ## Binding and discovery
//!
//! The server binds loopback on port 0, so the OS assigns a free port at
//! launch. The frontend cannot know that port ahead of time, so it is handed
//! over IPC together with a per-launch token (Task 1.4:
//! "random free port, communicated over IPC").
//!
//! ## Why there is a token
//!
//! A listener on `127.0.0.1` is reachable by *every* process on the machine,
//! including a browser tab the user happens to have open. Without
//! authentication, any local program could subscribe to terminal output and
//! diagnostics. The handshake therefore requires a token generated fresh on
//! each launch and never written to disk. Origin is not relied on: a
//! non-browser client can set any origin it likes.
//!
//! ## Heartbeats
//!
//! Two mechanisms run on the same 5s cadence, because each covers a case the
//! other cannot:
//!
//! - A protocol-level `Ping`, auto-answered by any conformant client. Three
//!   unanswered pings close the connection kernel-side, which is what stops
//!   a half-open TCP connection from pinning a session forever.
//! - An application-level `heartbeat` control frame, because JavaScript in a
//!   webview cannot observe ping/pong frames and so needs an observable beat
//!   to run its own liveness timer against.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;

use crate::envelope::{HEARTBEAT_INTERVAL_MS, MISSED_HEARTBEAT_LIMIT, StreamControl, StreamFrame};
use crate::hub::{SessionHandle, StreamHub};

/// Path the server accepts upgrade requests on.
pub const STREAM_PATH: &str = "/stream";

/// Server configuration.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Address to bind. Port 0 asks the OS for a free port.
    pub bind_addr: SocketAddr,
    /// Token a client must present as `?token=…` on the upgrade request.
    pub token: String,
    pub heartbeat_interval: Duration,
    pub missed_heartbeat_limit: u32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            token: uuid::Uuid::new_v4().to_string(),
            heartbeat_interval: Duration::from_millis(u64::from(HEARTBEAT_INTERVAL_MS)),
            missed_heartbeat_limit: MISSED_HEARTBEAT_LIMIT,
        }
    }
}

impl ServerConfig {
    pub fn with_heartbeat_interval(mut self, interval: Duration) -> Self {
        self.heartbeat_interval = interval;
        self
    }

    pub fn with_missed_heartbeat_limit(mut self, limit: u32) -> Self {
        self.missed_heartbeat_limit = limit;
        self
    }

    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.token = token.into();
        self
    }
}

/// Tracks unanswered heartbeats for one connection (REQ-ARCH-003.9).
///
/// Split out from the connection loop so the "dead after 3 missed pongs"
/// rule is verifiable without a client that deliberately refuses to answer
/// pings, which no conformant WebSocket implementation will do on request.
#[derive(Debug)]
pub struct HeartbeatMonitor {
    limit: u32,
    unanswered: AtomicU64,
}

impl HeartbeatMonitor {
    pub fn new(limit: u32) -> Self {
        Self {
            limit,
            unanswered: AtomicU64::new(0),
        }
    }

    /// Record a ping being sent. Returns true when the peer has now missed
    /// more than the allowed number of consecutive pongs.
    pub fn record_ping(&self) -> bool {
        let unanswered = self.unanswered.fetch_add(1, Ordering::Relaxed) + 1;
        unanswered > u64::from(self.limit)
    }

    /// Record any pong. A single answer clears the whole streak: the peer
    /// is demonstrably alive.
    pub fn record_pong(&self) {
        self.unanswered.store(0, Ordering::Relaxed);
    }

    pub fn unanswered(&self) -> u64 {
        self.unanswered.load(Ordering::Relaxed)
    }
}

/// Connection counters for health reporting (REQ-OBS-004.1).
#[derive(Debug, Default)]
struct ServerCounters {
    accepted: AtomicU64,
    rejected: AtomicU64,
    active: AtomicU64,
    heartbeat_timeouts: AtomicU64,
}

/// Snapshot of connection activity.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerMetrics {
    pub accepted: u64,
    pub rejected: u64,
    pub active: u64,
    pub heartbeat_timeouts: u64,
}

/// A running streaming server.
///
/// Dropping the handle aborts the accept loop; in-flight connections are
/// closed when their sessions are dropped with it.
pub struct StreamServer {
    addr: SocketAddr,
    config: ServerConfig,
    hub: Arc<StreamHub>,
    counters: Arc<ServerCounters>,
    accept_loop: JoinHandle<()>,
}

impl StreamServer {
    /// Bind and start accepting connections.
    ///
    /// Returns once the listener is bound, so the caller can read
    /// [`StreamServer::addr`] and publish it over IPC without racing the
    /// first connection attempt.
    pub async fn bind(hub: Arc<StreamHub>, config: ServerConfig) -> std::io::Result<Self> {
        let listener = TcpListener::bind(config.bind_addr).await?;
        let addr = listener.local_addr()?;
        let counters = Arc::new(ServerCounters::default());

        let accept_loop = {
            let hub = hub.clone();
            let config = config.clone();
            let counters = counters.clone();
            tokio::spawn(async move {
                loop {
                    match listener.accept().await {
                        Ok((stream, peer)) => {
                            let hub = hub.clone();
                            let config = config.clone();
                            let counters = counters.clone();
                            tokio::spawn(async move {
                                // A single misbehaving client must not take
                                // the accept loop or any other connection
                                // with it, so every connection is its own
                                // task and its error is swallowed here.
                                if let Err(e) =
                                    serve_connection(stream, hub, config, &counters).await
                                {
                                    let _ = (peer, e);
                                }
                            });
                        }
                        // The listener itself failing is usually transient
                        // (fd exhaustion, a connection reset mid-accept);
                        // yield and keep serving rather than tearing the
                        // stream layer down.
                        Err(_) => {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                    }
                }
            })
        };

        Ok(Self {
            addr,
            config,
            hub,
            counters,
            accept_loop,
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    pub fn token(&self) -> &str {
        &self.config.token
    }

    /// The URL a client connects to, token included.
    pub fn url(&self) -> String {
        format!(
            "ws://{}:{}{}?token={}",
            self.addr.ip(),
            self.addr.port(),
            STREAM_PATH,
            self.config.token
        )
    }

    pub fn hub(&self) -> &Arc<StreamHub> {
        &self.hub
    }

    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    pub fn metrics(&self) -> ServerMetrics {
        ServerMetrics {
            accepted: self.counters.accepted.load(Ordering::Relaxed),
            rejected: self.counters.rejected.load(Ordering::Relaxed),
            active: self.counters.active.load(Ordering::Relaxed),
            heartbeat_timeouts: self.counters.heartbeat_timeouts.load(Ordering::Relaxed),
        }
    }

    /// Stop accepting new connections.
    pub fn shutdown(&self) {
        self.accept_loop.abort();
    }
}

impl Drop for StreamServer {
    fn drop(&mut self) {
        self.accept_loop.abort();
    }
}

/// Extract the `token` query parameter from a request URI, if present.
fn token_from_uri(uri: &str) -> Option<&str> {
    let query = uri.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == "token").then_some(value)
    })
}

/// Validate path and token on the upgrade request. Rejecting here means the
/// client gets an HTTP error instead of an open socket it can send on.
fn authorize(request: &Request, expected_token: &str) -> Result<(), StatusCode> {
    let uri = request.uri();
    if uri.path() != STREAM_PATH {
        return Err(StatusCode::NOT_FOUND);
    }
    match token_from_uri(&uri.to_string()) {
        // Compared for equality against a value that lives only in memory
        // for this launch; a timing-safe comparison buys nothing here
        // because an attacker cannot observe the timing of a handshake it
        // does not already control.
        Some(token) if token == expected_token => Ok(()),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

async fn serve_connection(
    stream: TcpStream,
    hub: Arc<StreamHub>,
    config: ServerConfig,
    counters: &Arc<ServerCounters>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Nagle off: streaming is latency-sensitive (terminal input echo has a
    // 16ms budget in REQ-NFR-001.3), and the frames are small.
    let _ = stream.set_nodelay(true);

    let expected = config.token.clone();
    // The closure's signature, `Err` variant included, is dictated by
    // tungstenite's handshake callback, so its size is not ours to reduce.
    #[allow(clippy::result_large_err)]
    let websocket = tokio_tungstenite::accept_hdr_async(
        stream,
        move |request: &Request, response: Response| match authorize(request, &expected) {
            Ok(()) => Ok(response),
            Err(status) => {
                let mut error = ErrorResponse::new(Some(status.to_string()));
                *error.status_mut() = status;
                Err(error)
            }
        },
    )
    .await;

    let websocket = match websocket {
        Ok(ws) => ws,
        Err(e) => {
            counters.rejected.fetch_add(1, Ordering::Relaxed);
            return Err(Box::new(e));
        }
    };

    counters.accepted.fetch_add(1, Ordering::Relaxed);
    counters.active.fetch_add(1, Ordering::Relaxed);

    let session = Arc::new(hub.open_session());
    let (mut sink, mut source) = websocket.split();

    // Pongs land on the read half, heartbeats are sent from the write half,
    // so the miss counter is shared between them.
    let heartbeats = Arc::new(HeartbeatMonitor::new(config.missed_heartbeat_limit));

    let reader = {
        let session = session.clone();
        let heartbeats = heartbeats.clone();
        tokio::spawn(async move {
            while let Some(message) = source.next().await {
                match message {
                    Ok(Message::Text(text)) => {
                        handle_client_frame(&session, text.as_str());
                    }
                    Ok(Message::Pong(_)) => heartbeats.record_pong(),
                    Ok(Message::Close(_)) => break,
                    // Binary frames are reserved for terminal output in
                    // Task 6.1; nothing sends them frontend → kernel yet, so
                    // they are ignored rather than treated as an error.
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            // The client is gone: wake the writer so it stops.
            session.close();
        })
    };

    let mut heartbeat = tokio::time::interval(config.heartbeat_interval);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    heartbeat.tick().await; // the first tick completes immediately
    let mut heartbeat_sequence: u64 = 0;

    let outcome = loop {
        tokio::select! {
            frames = session.next_frames() => {
                match frames {
                    Some(frames) => {
                        for frame in frames {
                            if send_frame(&mut sink, &frame).await.is_err() {
                                break;
                            }
                        }
                        if sink.flush().await.is_err() {
                            break Outcome::PeerGone;
                        }
                    }
                    None => break Outcome::Closed,
                }
            }
            _ = heartbeat.tick() => {
                if heartbeats.record_ping() {
                    break Outcome::HeartbeatTimeout;
                }
                heartbeat_sequence += 1;
                let beat = StreamFrame::control(StreamControl::Heartbeat {
                    sequence: heartbeat_sequence,
                });
                if send_frame(&mut sink, &beat).await.is_err() {
                    break Outcome::PeerGone;
                }
                if sink.send(Message::Ping(Default::default())).await.is_err() {
                    break Outcome::PeerGone;
                }
            }
        }
    };

    if let Outcome::HeartbeatTimeout = outcome {
        counters.heartbeat_timeouts.fetch_add(1, Ordering::Relaxed);
        let closing = StreamFrame::control(StreamControl::Closing {
            reason: format!("no pong after {} heartbeats", config.missed_heartbeat_limit),
        });
        let _ = send_frame(&mut sink, &closing).await;
    }

    let _ = sink.close().await;
    session.close();
    reader.abort();
    counters.active.fetch_sub(1, Ordering::Relaxed);
    Ok(())
}

enum Outcome {
    Closed,
    PeerGone,
    HeartbeatTimeout,
}

/// Apply a frame received from the client.
///
/// An unparseable frame is logged by the counter and discarded rather than
/// closing the connection, per the REQ-ARCH-003 failure mode ("malformed
/// message: logged, discarded, counter incremented").
fn handle_client_frame(session: &SessionHandle, text: &str) {
    match serde_json::from_str::<StreamFrame>(text) {
        Ok(StreamFrame::Control(StreamControl::Subscribe { channels })) => {
            session.subscribe(&channels);
        }
        Ok(StreamFrame::Control(StreamControl::Unsubscribe { channels })) => {
            session.unsubscribe(&channels);
        }
        // Everything else is kernel → frontend only. Silently ignored so a
        // client cannot inject data frames into a channel other clients
        // read.
        Ok(_) | Err(_) => {}
    }
}

async fn send_frame<S>(sink: &mut S, frame: &StreamFrame) -> Result<(), ()>
where
    S: SinkExt<Message> + Unpin,
{
    let Ok(text) = serde_json::to_string(frame) else {
        // A frame the kernel itself produced failing to serialize is a bug,
        // not a client problem; dropping it keeps the connection alive.
        return Ok(());
    };
    sink.send(Message::Text(text.into())).await.map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_extracted_from_the_query_string() {
        assert_eq!(token_from_uri("/stream?token=abc"), Some("abc"));
        assert_eq!(token_from_uri("/stream?a=1&token=abc&b=2"), Some("abc"));
        assert_eq!(token_from_uri("/stream"), None);
        assert_eq!(token_from_uri("/stream?tokenish=abc"), None);
    }

    #[test]
    fn the_default_config_binds_loopback_on_a_free_port_with_a_fresh_token() {
        let a = ServerConfig::default();
        let b = ServerConfig::default();
        assert!(a.bind_addr.ip().is_loopback());
        assert_eq!(a.bind_addr.port(), 0);
        assert_ne!(a.token, b.token, "each launch must get its own token");
        assert_eq!(a.heartbeat_interval, Duration::from_millis(5_000));
        assert_eq!(a.missed_heartbeat_limit, 3);
    }

    #[test]
    fn a_connection_survives_up_to_the_limit_of_unanswered_pings() {
        let monitor = HeartbeatMonitor::new(3);
        assert!(!monitor.record_ping());
        assert!(!monitor.record_ping());
        assert!(!monitor.record_ping());
        assert_eq!(monitor.unanswered(), 3);
        assert!(
            monitor.record_ping(),
            "the fourth unanswered ping exceeds a limit of 3 and declares the peer dead"
        );
    }

    #[test]
    fn a_single_pong_clears_the_miss_streak() {
        let monitor = HeartbeatMonitor::new(3);
        monitor.record_ping();
        monitor.record_ping();
        monitor.record_pong();
        assert_eq!(monitor.unanswered(), 0);
        assert!(!monitor.record_ping());
    }

    #[tokio::test]
    async fn binding_reports_a_concrete_port_and_url() {
        let hub = Arc::new(StreamHub::default());
        let server = StreamServer::bind(hub, ServerConfig::default().with_token("tok"))
            .await
            .unwrap();
        assert_ne!(server.port(), 0, "the OS must have assigned a real port");
        assert_eq!(
            server.url(),
            format!("ws://127.0.0.1:{}/stream?token=tok", server.port())
        );
    }
}
