//! The stream hub: channel registry, sequencing, subscriptions, and
//! backpressure accounting (REQ-ARCH-003.5, .6, .8, .10).
//!
//! The hub is transport-agnostic on purpose, exactly as the IPC dispatcher
//! is. [`crate::server`] binds it to a WebSocket; tests and later kernel
//! services publish into it directly. That keeps the ordering and
//! backpressure guarantees testable without a socket, and keeps
//! REQ-REMOTE-001.2 satisfiable (nothing here assumes the consumer is on
//! this machine).
//!
//! ## Model
//!
//! - A **channel** owns a monotonic sequence counter and a
//!   [`RingBuffer`](crate::ring::RingBuffer) of its most recent messages.
//! - A **session** is one connected client. It holds a cursor per
//!   subscribed channel rather than a queue of its own.
//!
//! Two consequences follow directly from that shape:
//!
//! - Delivery is ordered per channel, because a cursor only ever moves
//!   forward through a buffer that is only ever appended to.
//! - A slow session falls off the back of the buffer instead of growing
//!   unboundedly, and the distance between its cursor and the oldest
//!   retained sequence is the exact number of messages it lost, which is
//!   what `backpressure_warning` reports.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::Notify;

use crate::envelope::{ChannelSubscription, StreamControl, StreamEnvelope, StreamFrame};
use crate::ring::{DEFAULT_BUFFER_DEPTH, RingBuffer};

/// Buffer depth configuration (REQ-ARCH-003.8).
#[derive(Debug, Clone)]
pub struct HubConfig {
    /// Depth applied to any channel without a specific override.
    pub default_buffer_depth: usize,
    /// Per-channel overrides, for channels whose traffic profile differs
    /// (terminal output wants more headroom than health status).
    pub channel_buffer_depths: HashMap<String, usize>,
}

impl Default for HubConfig {
    fn default() -> Self {
        Self {
            default_buffer_depth: DEFAULT_BUFFER_DEPTH,
            channel_buffer_depths: HashMap::new(),
        }
    }
}

impl HubConfig {
    pub fn with_default_depth(mut self, depth: usize) -> Self {
        self.default_buffer_depth = depth;
        self
    }

    pub fn with_channel_depth(mut self, channel: impl Into<String>, depth: usize) -> Self {
        self.channel_buffer_depths.insert(channel.into(), depth);
        self
    }

    fn depth_for(&self, channel: &str) -> usize {
        self.channel_buffer_depths
            .get(channel)
            .copied()
            .unwrap_or(self.default_buffer_depth)
    }
}

/// Counters backing the hub's health report (REQ-OBS-004.1).
#[derive(Debug, Default)]
struct Metrics {
    published: AtomicU64,
    delivered: AtomicU64,
    dropped: AtomicU64,
    backpressure_events: AtomicU64,
    sessions_opened: AtomicU64,
}

/// Point-in-time snapshot of hub activity, surfaced through the kernel's
/// health dashboard.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HubMetrics {
    pub published: u64,
    pub delivered: u64,
    pub dropped: u64,
    pub backpressure_events: u64,
    pub sessions_opened: u64,
    pub active_sessions: u64,
    pub active_subscriptions: u64,
    pub channels: u64,
}

struct ChannelState {
    ring: RingBuffer<Arc<StreamEnvelope>>,
    next_sequence: u64,
}

impl ChannelState {
    fn new(depth: usize) -> Self {
        Self {
            ring: RingBuffer::new(depth),
            // Sequences start at 1 so a cursor of 0 means "nothing
            // delivered yet" without needing a sentinel.
            next_sequence: 1,
        }
    }
}

struct SessionState {
    /// Last sequence delivered per subscribed channel. Presence in this map
    /// *is* the subscription.
    cursors: HashMap<String, u64>,
    /// Control frames waiting to be delivered (acknowledgements,
    /// backpressure warnings, heartbeats).
    pending_control: VecDeque<StreamControl>,
    notify: Arc<Notify>,
    closed: bool,
}

struct Inner {
    channels: HashMap<String, ChannelState>,
    sessions: HashMap<u64, SessionState>,
    next_session_id: u64,
}

/// Publish/subscribe hub for all kernel-to-frontend streaming.
pub struct StreamHub {
    inner: Mutex<Inner>,
    config: HubConfig,
    metrics: Metrics,
}

impl Default for StreamHub {
    fn default() -> Self {
        Self::new(HubConfig::default())
    }
}

impl StreamHub {
    pub fn new(config: HubConfig) -> Self {
        Self {
            inner: Mutex::new(Inner {
                channels: HashMap::new(),
                sessions: HashMap::new(),
                next_session_id: 1,
            }),
            config,
            metrics: Metrics::default(),
        }
    }

    pub fn config(&self) -> &HubConfig {
        &self.config
    }

    /// Publish a payload to a channel, returning the sequence it was
    /// assigned.
    pub fn publish(&self, channel: &str, payload: serde_json::Value) -> u64 {
        self.publish_envelope(channel, payload, None)
    }

    /// Publish a payload tied to the IPC command that requested the stream
    /// (REQ-ARCH-003.5).
    pub fn publish_correlated(
        &self,
        channel: &str,
        payload: serde_json::Value,
        correlation_id: impl Into<String>,
    ) -> u64 {
        self.publish_envelope(channel, payload, Some(correlation_id.into()))
    }

    fn publish_envelope(
        &self,
        channel: &str,
        payload: serde_json::Value,
        correlation_id: Option<String>,
    ) -> u64 {
        let mut to_wake: Vec<Arc<Notify>> = Vec::new();
        let sequence = {
            let mut inner = self.inner.lock().unwrap();
            let Inner {
                channels, sessions, ..
            } = &mut *inner;

            let state = channels
                .entry(channel.to_string())
                .or_insert_with(|| ChannelState::new(self.config.depth_for(channel)));

            let sequence = state.next_sequence;
            state.next_sequence += 1;

            let mut envelope = StreamEnvelope::new(channel, sequence, payload);
            envelope.correlation_id = correlation_id;
            if state.ring.push(sequence, Arc::new(envelope)).is_some() {
                self.metrics.dropped.fetch_add(1, Ordering::Relaxed);
            }

            for session in sessions.values() {
                if session.cursors.contains_key(channel) && !session.closed {
                    to_wake.push(session.notify.clone());
                }
            }
            sequence
        };

        self.metrics.published.fetch_add(1, Ordering::Relaxed);
        // Woken outside the lock so a subscriber that wakes immediately does
        // not contend with the publisher still holding it.
        for notify in to_wake {
            notify.notify_one();
        }
        sequence
    }

    /// Open a session for a connected client.
    pub fn open_session(self: &Arc<Self>) -> SessionHandle {
        let notify = Arc::new(Notify::new());
        let id = {
            let mut inner = self.inner.lock().unwrap();
            let id = inner.next_session_id;
            inner.next_session_id += 1;
            inner.sessions.insert(
                id,
                SessionState {
                    cursors: HashMap::new(),
                    pending_control: VecDeque::new(),
                    notify: notify.clone(),
                    closed: false,
                },
            );
            id
        };
        self.metrics.sessions_opened.fetch_add(1, Ordering::Relaxed);
        SessionHandle {
            id,
            hub: self.clone(),
            notify,
        }
    }

    /// Subscribe a session to channels, positioning each cursor.
    ///
    /// `from_sequence: None` starts at the channel's current head, so a
    /// fresh subscriber is not flooded with buffered history it never asked
    /// for. `Some(n)` resumes after `n`, which is how a reconnecting client
    /// closes its gap (REQ-ARCH-003.7).
    pub fn subscribe(&self, session_id: u64, subscriptions: &[ChannelSubscription]) -> Vec<String> {
        let mut accepted = Vec::new();
        let notify = {
            let mut inner = self.inner.lock().unwrap();
            let Inner {
                channels, sessions, ..
            } = &mut *inner;
            let Some(session) = sessions.get_mut(&session_id) else {
                return accepted;
            };

            for sub in subscriptions {
                let state = channels
                    .entry(sub.channel.clone())
                    .or_insert_with(|| ChannelState::new(self.config.depth_for(&sub.channel)));
                let cursor = match sub.from_sequence {
                    // Never rewind past what the channel has ever emitted:
                    // a client claiming a future sequence must not be able
                    // to stall its own delivery.
                    Some(seq) => seq.min(state.next_sequence - 1),
                    None => state.next_sequence - 1,
                };
                session.cursors.insert(sub.channel.clone(), cursor);
                accepted.push(sub.channel.clone());
            }

            session
                .pending_control
                .push_back(StreamControl::Subscribed {
                    channels: accepted.clone(),
                });
            session.notify.clone()
        };
        notify.notify_one();
        accepted
    }

    /// Unsubscribe a session from channels. Unknown channels are ignored
    /// rather than reported as errors: unsubscribing twice is idempotent,
    /// which matters when a component unmounts during a reconnect.
    pub fn unsubscribe(&self, session_id: u64, channels: &[String]) -> Vec<String> {
        let mut removed = Vec::new();
        let notify = {
            let mut inner = self.inner.lock().unwrap();
            let Some(session) = inner.sessions.get_mut(&session_id) else {
                return removed;
            };
            for channel in channels {
                if session.cursors.remove(channel).is_some() {
                    removed.push(channel.clone());
                }
            }
            session
                .pending_control
                .push_back(StreamControl::Unsubscribed {
                    channels: removed.clone(),
                });
            session.notify.clone()
        };
        notify.notify_one();
        removed
    }

    /// Queue a control frame for one session.
    pub fn push_control(&self, session_id: u64, control: StreamControl) {
        let notify = {
            let mut inner = self.inner.lock().unwrap();
            let Some(session) = inner.sessions.get_mut(&session_id) else {
                return;
            };
            session.pending_control.push_back(control);
            session.notify.clone()
        };
        notify.notify_one();
    }

    /// Collect everything currently owed to a session, advancing its
    /// cursors.
    ///
    /// Returns `None` once the session is closed and drained, which is the
    /// signal for its writer loop to exit. An empty vector means "nothing
    /// right now, wait for the next notification".
    pub fn drain(&self, session_id: u64) -> Option<Vec<StreamFrame>> {
        let mut inner = self.inner.lock().unwrap();
        let Inner {
            channels, sessions, ..
        } = &mut *inner;
        let session = sessions.get_mut(&session_id)?;

        let mut frames: Vec<StreamFrame> = session
            .pending_control
            .drain(..)
            .map(StreamFrame::control)
            .collect();

        if session.closed && frames.is_empty() {
            return None;
        }

        let mut dropped_total = 0u64;
        let mut backpressure_events = 0u64;
        let mut delivered = 0u64;

        // Sorted so a drain covering several channels is deterministic.
        // Ordering *within* a channel is what the requirement constrains,
        // and that is guaranteed by the ring's append-only iteration.
        let mut subscribed: Vec<String> = session.cursors.keys().cloned().collect();
        subscribed.sort_unstable();

        for channel in subscribed {
            let Some(state) = channels.get(&channel) else {
                continue;
            };
            let cursor = session.cursors.get(&channel).copied().unwrap_or(0);

            let gap = state.ring.gap_after(cursor);
            if gap > 0 {
                dropped_total += gap;
                backpressure_events += 1;
                frames.push(StreamFrame::control(StreamControl::BackpressureWarning {
                    channel: channel.clone(),
                    dropped: gap,
                    buffer_depth: state.ring.depth() as u32,
                }));
            }

            let mut last = cursor;
            for (sequence, envelope) in state.ring.iter_after(cursor) {
                frames.push(StreamFrame::Data(envelope.as_ref().clone()));
                last = sequence;
                delivered += 1;
            }
            if last != cursor {
                session.cursors.insert(channel, last);
            }
        }

        if dropped_total > 0 {
            self.metrics
                .backpressure_events
                .fetch_add(backpressure_events, Ordering::Relaxed);
        }
        if delivered > 0 {
            self.metrics
                .delivered
                .fetch_add(delivered, Ordering::Relaxed);
        }

        Some(frames)
    }

    /// Mark a session closed and wake its writer so it can exit.
    pub fn close_session(&self, session_id: u64) {
        let notify = {
            let mut inner = self.inner.lock().unwrap();
            match inner.sessions.get_mut(&session_id) {
                Some(session) => {
                    session.closed = true;
                    session.cursors.clear();
                    session.notify.clone()
                }
                None => return,
            }
        };
        notify.notify_one();
    }

    /// Remove a session's state entirely.
    pub fn remove_session(&self, session_id: u64) {
        self.inner.lock().unwrap().sessions.remove(&session_id);
    }

    /// Channels a session is currently subscribed to, sorted.
    pub fn subscriptions_of(&self, session_id: u64) -> Vec<String> {
        let inner = self.inner.lock().unwrap();
        let mut channels: Vec<String> = inner
            .sessions
            .get(&session_id)
            .map(|s| s.cursors.keys().cloned().collect())
            .unwrap_or_default();
        channels.sort_unstable();
        channels
    }

    /// How many live sessions are subscribed to a channel. Used to avoid
    /// generating traffic nobody is listening to.
    pub fn subscriber_count(&self, channel: &str) -> usize {
        let inner = self.inner.lock().unwrap();
        inner
            .sessions
            .values()
            .filter(|s| !s.closed && s.cursors.contains_key(channel))
            .count()
    }

    /// The next sequence a channel will assign. Equals 1 for a channel that
    /// has never published.
    pub fn next_sequence(&self, channel: &str) -> u64 {
        self.inner
            .lock()
            .unwrap()
            .channels
            .get(channel)
            .map(|c| c.next_sequence)
            .unwrap_or(1)
    }

    /// Configured buffer depth for a channel.
    pub fn buffer_depth(&self, channel: &str) -> usize {
        self.config.depth_for(channel)
    }

    pub fn metrics(&self) -> HubMetrics {
        let inner = self.inner.lock().unwrap();
        HubMetrics {
            published: self.metrics.published.load(Ordering::Relaxed),
            delivered: self.metrics.delivered.load(Ordering::Relaxed),
            dropped: self.metrics.dropped.load(Ordering::Relaxed),
            backpressure_events: self.metrics.backpressure_events.load(Ordering::Relaxed),
            sessions_opened: self.metrics.sessions_opened.load(Ordering::Relaxed),
            active_sessions: inner.sessions.values().filter(|s| !s.closed).count() as u64,
            active_subscriptions: inner
                .sessions
                .values()
                .map(|s| s.cursors.len() as u64)
                .sum(),
            channels: inner.channels.len() as u64,
        }
    }
}

/// A connected client's handle onto the hub.
///
/// Dropping the handle removes the session, so a disconnected client leaves
/// no state behind even if its connection task unwinds.
pub struct SessionHandle {
    id: u64,
    hub: Arc<StreamHub>,
    notify: Arc<Notify>,
}

impl SessionHandle {
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn hub(&self) -> &Arc<StreamHub> {
        &self.hub
    }

    pub fn subscribe(&self, subscriptions: &[ChannelSubscription]) -> Vec<String> {
        self.hub.subscribe(self.id, subscriptions)
    }

    pub fn unsubscribe(&self, channels: &[String]) -> Vec<String> {
        self.hub.unsubscribe(self.id, channels)
    }

    pub fn push_control(&self, control: StreamControl) {
        self.hub.push_control(self.id, control);
    }

    pub fn drain(&self) -> Option<Vec<StreamFrame>> {
        self.hub.drain(self.id)
    }

    pub fn close(&self) {
        self.hub.close_session(self.id);
    }

    /// Wait for the next non-empty batch of frames owed to this session.
    ///
    /// Resolves `None` when the session is closed and drained. The
    /// notification is registered *before* draining, which closes the race
    /// where a publish lands between the drain and the await and would
    /// otherwise be missed until the next publish.
    pub async fn next_frames(&self) -> Option<Vec<StreamFrame>> {
        loop {
            let notified = self.notify.notified();
            match self.hub.drain(self.id) {
                None => return None,
                Some(frames) if !frames.is_empty() => return Some(frames),
                Some(_) => notified.await,
            }
        }
    }
}

impl Drop for SessionHandle {
    fn drop(&mut self) {
        self.hub.remove_session(self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn hub() -> Arc<StreamHub> {
        Arc::new(StreamHub::default())
    }

    fn payloads(frames: &[StreamFrame]) -> Vec<u64> {
        frames
            .iter()
            .filter_map(|f| match f {
                StreamFrame::Data(envelope) => envelope.payload.as_u64(),
                _ => None,
            })
            .collect()
    }

    fn sequences(frames: &[StreamFrame]) -> Vec<u64> {
        frames
            .iter()
            .filter_map(|f| match f {
                StreamFrame::Data(envelope) => Some(envelope.sequence),
                _ => None,
            })
            .collect()
    }

    fn controls(frames: &[StreamFrame]) -> Vec<&StreamControl> {
        frames
            .iter()
            .filter_map(|f| match f {
                StreamFrame::Control(control) => Some(control),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn sequences_are_monotonic_per_channel_and_independent_across_channels() {
        let hub = hub();
        assert_eq!(hub.publish("a", serde_json::json!(1)), 1);
        assert_eq!(hub.publish("a", serde_json::json!(2)), 2);
        assert_eq!(hub.publish("b", serde_json::json!(1)), 1);
        assert_eq!(hub.publish("a", serde_json::json!(3)), 3);
        assert_eq!(hub.next_sequence("a"), 4);
        assert_eq!(hub.next_sequence("b"), 2);
    }

    #[test]
    fn a_fresh_subscriber_receives_only_messages_published_after_subscribing() {
        let hub = hub();
        hub.publish("a", serde_json::json!(1));
        let session = hub.open_session();
        session.subscribe(&[ChannelSubscription::new("a")]);
        hub.publish("a", serde_json::json!(2));

        let frames = session.drain().unwrap();
        assert_eq!(payloads(&frames), vec![2]);
    }

    #[test]
    fn delivery_is_ordered_within_a_channel() {
        let hub = hub();
        let session = hub.open_session();
        session.subscribe(&[ChannelSubscription::new("a")]);
        for i in 1..=50 {
            hub.publish("a", serde_json::json!(i));
        }
        let frames = session.drain().unwrap();
        assert_eq!(sequences(&frames), (1..=50).collect::<Vec<u64>>());
    }

    #[test]
    fn resuming_from_a_sequence_replays_what_is_still_buffered() {
        let hub = hub();
        for i in 1..=10 {
            hub.publish("a", serde_json::json!(i));
        }
        let session = hub.open_session();
        session.subscribe(&[ChannelSubscription::resume_from("a", 7)]);

        let frames = session.drain().unwrap();
        assert_eq!(sequences(&frames), vec![8, 9, 10]);
    }

    #[test]
    fn resuming_from_a_future_sequence_is_clamped_to_the_channel_head() {
        let hub = hub();
        hub.publish("a", serde_json::json!(1));
        let session = hub.open_session();
        session.subscribe(&[ChannelSubscription::resume_from("a", 9_999)]);
        hub.publish("a", serde_json::json!(2));

        let frames = session.drain().unwrap();
        assert_eq!(
            sequences(&frames),
            vec![2],
            "a bogus resume point must not stall the subscriber"
        );
    }

    #[test]
    fn a_slow_subscriber_is_told_exactly_how_many_messages_it_lost() {
        let hub = Arc::new(StreamHub::new(HubConfig::default().with_default_depth(4)));
        let session = hub.open_session();
        session.subscribe(&[ChannelSubscription::new("a")]);
        let _ = session.drain(); // clear the subscribe acknowledgement

        for i in 1..=10 {
            hub.publish("a", serde_json::json!(i));
        }

        let frames = session.drain().unwrap();
        let warnings = controls(&frames);
        assert_eq!(
            warnings,
            vec![&StreamControl::BackpressureWarning {
                channel: "a".into(),
                dropped: 6,
                buffer_depth: 4,
            }]
        );
        assert_eq!(sequences(&frames), vec![7, 8, 9, 10]);
        assert_eq!(hub.metrics().backpressure_events, 1);
        assert_eq!(hub.metrics().dropped, 6);
    }

    #[test]
    fn a_backpressure_warning_is_not_repeated_once_the_cursor_has_moved_on() {
        let hub = Arc::new(StreamHub::new(HubConfig::default().with_default_depth(2)));
        let session = hub.open_session();
        session.subscribe(&[ChannelSubscription::new("a")]);
        for i in 1..=5 {
            hub.publish("a", serde_json::json!(i));
        }
        let first = session.drain().unwrap();
        assert_eq!(controls(&first).len(), 2, "subscribe ack plus one warning");

        hub.publish("a", serde_json::json!(6));
        let second = session.drain().unwrap();
        assert!(
            controls(&second).is_empty(),
            "a caught-up subscriber must not be warned again"
        );
        assert_eq!(sequences(&second), vec![6]);
    }

    #[test]
    fn per_channel_depth_overrides_the_default() {
        let hub = Arc::new(StreamHub::new(
            HubConfig::default()
                .with_default_depth(10)
                .with_channel_depth("terminal:output", 3),
        ));
        assert_eq!(hub.buffer_depth("terminal:output"), 3);
        assert_eq!(hub.buffer_depth("health:status"), 10);
    }

    #[test]
    fn unsubscribing_stops_delivery_and_is_idempotent() {
        let hub = hub();
        let session = hub.open_session();
        session.subscribe(&[ChannelSubscription::new("a")]);
        assert_eq!(session.unsubscribe(&["a".to_string()]), vec!["a"]);
        assert!(session.unsubscribe(&["a".to_string()]).is_empty());

        hub.publish("a", serde_json::json!(1));
        let frames = session.drain().unwrap();
        assert!(sequences(&frames).is_empty());
        assert!(session.hub().subscriptions_of(session.id()).is_empty());
    }

    #[test]
    fn resubscribing_after_unsubscribe_can_resume_from_the_last_sequence() {
        let hub = hub();
        let session = hub.open_session();
        session.subscribe(&[ChannelSubscription::new("a")]);
        hub.publish("a", serde_json::json!(1));
        let first = session.drain().unwrap();
        let last = *sequences(&first).last().unwrap();

        session.unsubscribe(&["a".to_string()]);
        hub.publish("a", serde_json::json!(2));
        session.subscribe(&[ChannelSubscription::resume_from("a", last)]);

        let frames = session.drain().unwrap();
        assert_eq!(
            sequences(&frames),
            vec![2],
            "messages published while unsubscribed are recoverable from the buffer"
        );
    }

    #[test]
    fn subscriber_count_ignores_closed_sessions() {
        let hub = hub();
        let session = hub.open_session();
        session.subscribe(&[ChannelSubscription::new("a")]);
        assert_eq!(hub.subscriber_count("a"), 1);
        session.close();
        assert_eq!(hub.subscriber_count("a"), 0);
    }

    #[test]
    fn dropping_a_session_handle_removes_its_state() {
        let hub = hub();
        let id = {
            let session = hub.open_session();
            session.subscribe(&[ChannelSubscription::new("a")]);
            session.id()
        };
        assert!(hub.subscriptions_of(id).is_empty());
        assert_eq!(hub.metrics().active_sessions, 0);
    }

    #[test]
    fn subscribing_acknowledges_the_accepted_channels() {
        let hub = hub();
        let session = hub.open_session();
        session.subscribe(&[ChannelSubscription::new("a"), ChannelSubscription::new("b")]);
        let frames = session.drain().unwrap();
        assert_eq!(
            controls(&frames),
            vec![&StreamControl::Subscribed {
                channels: vec!["a".into(), "b".into()],
            }]
        );
    }

    #[tokio::test]
    async fn next_frames_wakes_on_publish() {
        let hub = hub();
        let session = hub.open_session();
        session.subscribe(&[ChannelSubscription::new("a")]);
        let _ = session.drain();

        let publisher = {
            let hub = hub.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(10)).await;
                hub.publish("a", serde_json::json!(42));
            })
        };

        let frames = tokio::time::timeout(Duration::from_secs(2), session.next_frames())
            .await
            .expect("a published message must wake a waiting session")
            .expect("session is open");
        assert_eq!(payloads(&frames), vec![42]);
        publisher.await.unwrap();
    }

    #[tokio::test]
    async fn next_frames_resolves_none_once_the_session_closes() {
        let hub = hub();
        let session = hub.open_session();
        session.subscribe(&[ChannelSubscription::new("a")]);
        let _ = session.drain();

        let closer = {
            let hub = hub.clone();
            let id = session.id();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(10)).await;
                hub.close_session(id);
            })
        };

        let result = tokio::time::timeout(Duration::from_secs(2), session.next_frames())
            .await
            .expect("closing a session must wake its writer");
        assert!(result.is_none());
        closer.await.unwrap();
    }

    #[test]
    fn one_session_falling_behind_does_not_affect_another() {
        let hub = Arc::new(StreamHub::new(HubConfig::default().with_default_depth(3)));
        let fast = hub.open_session();
        let slow = hub.open_session();
        fast.subscribe(&[ChannelSubscription::new("a")]);
        slow.subscribe(&[ChannelSubscription::new("a")]);
        let _ = fast.drain();
        let _ = slow.drain();

        for i in 1..=3 {
            hub.publish("a", serde_json::json!(i));
            let frames = fast.drain().unwrap();
            assert_eq!(sequences(&frames), vec![i as u64]);
        }
        for i in 4..=8 {
            hub.publish("a", serde_json::json!(i));
            let frames = fast.drain().unwrap();
            assert!(controls(&frames).is_empty(), "the fast reader keeps up");
        }

        let slow_frames = slow.drain().unwrap();
        assert_eq!(
            controls(&slow_frames).len(),
            1,
            "only the slow reader is warned"
        );
        assert_eq!(sequences(&slow_frames), vec![6, 7, 8]);
    }

    #[test]
    fn correlation_id_is_carried_through_to_the_subscriber() {
        let hub = hub();
        let session = hub.open_session();
        session.subscribe(&[ChannelSubscription::new("search:results")]);
        hub.publish_correlated("search:results", serde_json::json!({}), "corr-9");

        let frames = session.drain().unwrap();
        let envelope = frames
            .iter()
            .find_map(|f| match f {
                StreamFrame::Data(e) => Some(e),
                _ => None,
            })
            .unwrap();
        assert_eq!(envelope.correlation_id.as_deref(), Some("corr-9"));
    }

    #[test]
    fn draining_an_unknown_session_reports_none() {
        let hub = hub();
        assert!(hub.drain(9_999).is_none());
    }
}
