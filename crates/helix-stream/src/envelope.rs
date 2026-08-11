//! Streaming wire types (REQ-ARCH-003.5, .7, .8, .9).
//!
//! These are the shapes described in the design document's "WebSocket
//! Protocol" section. They derive `ts_rs::TS`, so the TypeScript interfaces
//! in `frontend/src/generated/` come from this Rust source of truth exactly
//! as the IPC envelopes do.
//!
//! Two frame kinds share one socket, discriminated by `kind`:
//!
//! - [`StreamEnvelope`] carries channel data.
//! - [`StreamControl`] carries subscription management, heartbeats, and
//!   backpressure notices.
//!
//! An explicit discriminator is used rather than structural sniffing so a
//! malformed or hostile frame fails to parse instead of being guessed at
//! (Task 18.3 fuzzes this parser).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Heartbeat interval (REQ-ARCH-003.9).
pub const HEARTBEAT_INTERVAL_MS: u32 = 5_000;

/// Consecutive missed heartbeats/pongs after which the connection is
/// declared dead (REQ-ARCH-003.9: 3 misses, so 15s).
pub const MISSED_HEARTBEAT_LIMIT: u32 = 3;

/// A single data message on a channel.
///
/// `sequence` is monotonically increasing per channel and assigned by the
/// kernel at publish time, not per subscriber. That is what lets a
/// reconnecting client resume from where it left off and lets any client
/// detect a gap as evidence of a drop (REQ-ARCH-003.10).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct StreamEnvelope {
    pub channel: String,
    /// Links the message to the IPC command that started the stream, when
    /// there was one. Absent for ambient channels (health, diagnostics).
    pub correlation_id: Option<String>,
    /// Exposed to TypeScript as `number` rather than `bigint`: the wire
    /// format is JSON, so `JSON.parse` yields a `number` regardless of the
    /// Rust width. `Number.MAX_SAFE_INTEGER` is 2^53, which at the 100Hz
    /// cadence of the busiest channel is roughly 2.8 million years.
    #[ts(type = "number")]
    pub sequence: u64,
    #[ts(type = "unknown")]
    pub payload: serde_json::Value,
}

impl StreamEnvelope {
    pub fn new(channel: impl Into<String>, sequence: u64, payload: serde_json::Value) -> Self {
        Self {
            channel: channel.into(),
            correlation_id: None,
            sequence,
            payload,
        }
    }

    pub fn with_correlation_id(mut self, correlation_id: impl Into<String>) -> Self {
        self.correlation_id = Some(correlation_id.into());
        self
    }
}

/// One channel a client wants to receive, and where it wants to resume
/// from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct ChannelSubscription {
    pub channel: String,
    /// Last sequence the client already has. `None` means "only messages
    /// published from now on"; `Some(n)` replays everything after `n` that
    /// is still buffered, which is how a reconnect closes its gap.
    #[ts(type = "number | null")]
    pub from_sequence: Option<u64>,
}

impl ChannelSubscription {
    pub fn new(channel: impl Into<String>) -> Self {
        Self {
            channel: channel.into(),
            from_sequence: None,
        }
    }

    pub fn resume_from(channel: impl Into<String>, sequence: u64) -> Self {
        Self {
            channel: channel.into(),
            from_sequence: Some(sequence),
        }
    }
}

/// Bidirectional control messages.
///
/// `subscribe` and `unsubscribe` travel frontend → kernel; the rest travel
/// kernel → frontend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamControl {
    /// Begin receiving the listed channels.
    Subscribe { channels: Vec<ChannelSubscription> },
    /// Stop receiving the listed channels.
    Unsubscribe { channels: Vec<String> },
    /// Acknowledgement naming the channels now active for this connection.
    Subscribed { channels: Vec<String> },
    /// Acknowledgement naming the channels dropped from this connection.
    Unsubscribed { channels: Vec<String> },
    /// Messages were evicted before this client read them
    /// (REQ-ARCH-003.8). `dropped` is the exact count, so the UI can say
    /// "output truncated" with a number rather than a guess.
    BackpressureWarning {
        channel: String,
        #[ts(type = "number")]
        dropped: u64,
        buffer_depth: u32,
    },
    /// Liveness beat, every [`HEARTBEAT_INTERVAL_MS`]. Sent as an
    /// application frame in addition to the protocol-level ping, because a
    /// browser client cannot observe ping/pong frames from JavaScript.
    Heartbeat {
        #[ts(type = "number")]
        sequence: u64,
    },
    /// The connection is being closed by the kernel, with a reason the
    /// frontend can log or display.
    Closing { reason: String },
}

/// Everything that travels over the socket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StreamFrame {
    Data(StreamEnvelope),
    Control(StreamControl),
}

impl StreamFrame {
    pub fn data(envelope: StreamEnvelope) -> Self {
        StreamFrame::Data(envelope)
    }

    pub fn control(control: StreamControl) -> Self {
        StreamFrame::Control(control)
    }

    /// The channel this frame concerns, where it concerns one.
    pub fn channel(&self) -> Option<&str> {
        match self {
            StreamFrame::Data(envelope) => Some(&envelope.channel),
            StreamFrame::Control(StreamControl::BackpressureWarning { channel, .. }) => {
                Some(channel)
            }
            StreamFrame::Control(_) => None,
        }
    }
}

/// Request payload for the `stream.endpoint` command. Empty: the endpoint
/// is a property of the launch, not of the caller.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct StreamEndpointRequest {}

/// Connection details for the local streaming server, handed to the
/// frontend over IPC because the port is chosen at random on launch
/// (Task 1.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct StreamEndpoint {
    /// Fully-formed `ws://127.0.0.1:<port>/stream?token=…` URL.
    pub url: String,
    pub port: u16,
    /// Per-launch bearer token. The server binds to loopback, which any
    /// local process can reach, so the token is what distinguishes this
    /// application's frontend from anything else on the machine.
    pub token: String,
    pub heartbeat_interval_ms: u32,
    pub missed_heartbeat_limit: u32,
    pub default_buffer_depth: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_frame_round_trips_through_json() {
        let frame = StreamFrame::data(
            StreamEnvelope::new("terminal:output", 7, serde_json::json!({ "text": "ok" }))
                .with_correlation_id("corr-1"),
        );
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.contains("\"kind\":\"data\""));
        assert_eq!(serde_json::from_str::<StreamFrame>(&json).unwrap(), frame);
    }

    #[test]
    fn control_frame_round_trips_through_json() {
        let frame = StreamFrame::control(StreamControl::Subscribe {
            channels: vec![ChannelSubscription::resume_from("demo:counter", 41)],
        });
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.contains("\"kind\":\"control\""));
        assert!(json.contains("\"type\":\"subscribe\""));
        assert_eq!(serde_json::from_str::<StreamFrame>(&json).unwrap(), frame);
    }

    #[test]
    fn a_frame_without_a_discriminator_is_rejected_rather_than_guessed() {
        let json = r#"{"channel":"a","sequence":1,"payload":null}"#;
        assert!(serde_json::from_str::<StreamFrame>(json).is_err());
    }

    #[test]
    fn an_unknown_control_type_is_rejected() {
        let json = r#"{"kind":"control","type":"take_over_the_kernel"}"#;
        assert!(serde_json::from_str::<StreamFrame>(json).is_err());
    }

    #[test]
    fn correlation_id_is_omitted_as_null_but_still_optional() {
        let envelope = StreamEnvelope::new("health:status", 1, serde_json::Value::Null);
        assert!(envelope.correlation_id.is_none());
        let json = serde_json::to_string(&envelope).unwrap();
        let back: StreamEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(back, envelope);
    }

    #[test]
    fn frame_reports_the_channel_it_concerns() {
        assert_eq!(
            StreamFrame::data(StreamEnvelope::new("a", 1, serde_json::Value::Null)).channel(),
            Some("a")
        );
        assert_eq!(
            StreamFrame::control(StreamControl::BackpressureWarning {
                channel: "b".into(),
                dropped: 2,
                buffer_depth: 10,
            })
            .channel(),
            Some("b")
        );
        assert_eq!(
            StreamFrame::control(StreamControl::Heartbeat { sequence: 1 }).channel(),
            None
        );
    }

    #[test]
    fn heartbeat_constants_match_the_requirement() {
        assert_eq!(HEARTBEAT_INTERVAL_MS, 5_000);
        assert_eq!(MISSED_HEARTBEAT_LIMIT, 3);
    }
}
