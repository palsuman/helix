//! Task 1.4 integration tests, driven over a real loopback WebSocket
//! against the same server the kernel starts (REQ-ARCH-003.5-.10).
//!
//! Three properties the task calls out explicitly:
//!
//! - **Ordered delivery** — sequences arrive monotonically per channel.
//! - **Sequence continuity across a disconnect** — a client that drops and
//!   reconnects resumes with no gap, which is the Task 1.4 demo criterion.
//! - **Backpressure signalling** — a client that fell behind is told which
//!   channel truncated and by exactly how many messages.

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use helix_stream::{
    ChannelSubscription, HubConfig, ServerConfig, StreamControl, StreamFrame, StreamHub,
    StreamServer,
};
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

type Client = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

const COUNTER: &str = "demo:counter";

async fn server_with(config: HubConfig) -> (Arc<StreamHub>, StreamServer) {
    let hub = Arc::new(StreamHub::new(config));
    let server = StreamServer::bind(
        hub.clone(),
        // A short heartbeat keeps the tests quick while exercising the same
        // code path as the 5s production cadence.
        ServerConfig::default()
            .with_token("test-token")
            .with_heartbeat_interval(Duration::from_millis(50)),
    )
    .await
    .expect("server must bind on a free loopback port");
    (hub, server)
}

async fn connect(server: &StreamServer) -> Client {
    let (client, response) = tokio_tungstenite::connect_async(server.url())
        .await
        .expect("a token-bearing client must be accepted");
    assert_eq!(response.status().as_u16(), 101);
    client
}

async fn send_control(client: &mut Client, control: StreamControl) {
    let frame = StreamFrame::control(control);
    client
        .send(Message::Text(serde_json::to_string(&frame).unwrap().into()))
        .await
        .expect("control frame must be accepted");
}

/// Read the next frame, ignoring heartbeats (which are timing-dependent and
/// asserted separately).
async fn next_frame(client: &mut Client) -> StreamFrame {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(5), client.next())
            .await
            .expect("the server must answer within 5s")
            .expect("the stream must not end")
            .expect("the frame must not be an error");
        let Message::Text(text) = message else {
            continue;
        };
        let frame: StreamFrame =
            serde_json::from_str(text.as_str()).expect("every frame must parse as a StreamFrame");
        if let StreamFrame::Control(StreamControl::Heartbeat { .. }) = frame {
            continue;
        }
        return frame;
    }
}

/// Collect the next `count` data envelope sequences on a channel.
async fn next_sequences(client: &mut Client, channel: &str, count: usize) -> Vec<u64> {
    let mut sequences = Vec::with_capacity(count);
    while sequences.len() < count {
        if let StreamFrame::Data(envelope) = next_frame(client).await {
            assert_eq!(envelope.channel, channel);
            sequences.push(envelope.sequence);
        }
    }
    sequences
}

async fn expect_subscribed(client: &mut Client, channels: &[&str]) {
    match next_frame(client).await {
        StreamFrame::Control(StreamControl::Subscribed { channels: acked }) => {
            assert_eq!(
                acked,
                channels.iter().map(|c| c.to_string()).collect::<Vec<_>>()
            );
        }
        other => panic!("expected a subscribe acknowledgement, got {other:?}"),
    }
}

#[tokio::test]
async fn messages_are_delivered_in_monotonic_sequence_order() {
    let (hub, server) = server_with(HubConfig::default()).await;
    let mut client = connect(&server).await;

    send_control(
        &mut client,
        StreamControl::Subscribe {
            channels: vec![ChannelSubscription::new(COUNTER)],
        },
    )
    .await;
    expect_subscribed(&mut client, &[COUNTER]).await;

    for tick in 0..200u64 {
        hub.publish(COUNTER, serde_json::json!({ "tick": tick }));
    }

    let sequences = next_sequences(&mut client, COUNTER, 200).await;
    assert_eq!(
        sequences,
        (1..=200).collect::<Vec<u64>>(),
        "delivery must be ordered with no gaps and no duplicates"
    );
}

#[tokio::test]
async fn a_reconnecting_client_resumes_with_no_gap() {
    // The Task 1.4 demo criterion: killing the socket shows "reconnecting",
    // then the stream resumes with no gap.
    let (hub, server) = server_with(HubConfig::default()).await;

    let mut client = connect(&server).await;
    send_control(
        &mut client,
        StreamControl::Subscribe {
            channels: vec![ChannelSubscription::new(COUNTER)],
        },
    )
    .await;
    expect_subscribed(&mut client, &[COUNTER]).await;

    for tick in 0..10u64 {
        hub.publish(COUNTER, serde_json::json!({ "tick": tick }));
    }
    let before = next_sequences(&mut client, COUNTER, 10).await;
    let last_seen = *before.last().unwrap();

    // Simulate the socket being killed: drop it without a close handshake.
    drop(client);

    // Traffic the client is not there to receive.
    for tick in 10..25u64 {
        hub.publish(COUNTER, serde_json::json!({ "tick": tick }));
    }

    let mut reconnected = connect(&server).await;
    send_control(
        &mut reconnected,
        StreamControl::Subscribe {
            channels: vec![ChannelSubscription::resume_from(COUNTER, last_seen)],
        },
    )
    .await;
    expect_subscribed(&mut reconnected, &[COUNTER]).await;

    let after = next_sequences(&mut reconnected, COUNTER, 15).await;
    let mut all = before;
    all.extend(after);
    assert_eq!(
        all,
        (1..=25).collect::<Vec<u64>>(),
        "the sequence must be continuous across the disconnect"
    );
}

#[tokio::test]
async fn a_client_that_fell_behind_is_told_what_it_lost() {
    // Depth 4, 100 messages published before the client asks to resume from
    // the very beginning: sequences 1..=96 are gone, and the client must be
    // told so rather than silently seeing a jump.
    let (hub, server) = server_with(HubConfig::default().with_channel_depth(COUNTER, 4)).await;
    for tick in 0..100u64 {
        hub.publish(COUNTER, serde_json::json!({ "tick": tick }));
    }

    let mut client = connect(&server).await;
    send_control(
        &mut client,
        StreamControl::Subscribe {
            channels: vec![ChannelSubscription::resume_from(COUNTER, 0)],
        },
    )
    .await;
    expect_subscribed(&mut client, &[COUNTER]).await;

    match next_frame(&mut client).await {
        StreamFrame::Control(StreamControl::BackpressureWarning {
            channel,
            dropped,
            buffer_depth,
        }) => {
            assert_eq!(channel, COUNTER);
            assert_eq!(dropped, 96);
            assert_eq!(buffer_depth, 4);
        }
        other => panic!("expected a backpressure warning, got {other:?}"),
    }

    let sequences = next_sequences(&mut client, COUNTER, 4).await;
    assert_eq!(
        sequences,
        vec![97, 98, 99, 100],
        "the newest messages are retained; the oldest are the ones dropped"
    );
    assert_eq!(hub.metrics().backpressure_events, 1);
}

#[tokio::test]
async fn the_server_beats_on_its_configured_interval() {
    let (_hub, server) = server_with(HubConfig::default()).await;
    let mut client = connect(&server).await;

    let mut beats = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while beats < 2 && tokio::time::Instant::now() < deadline {
        let message = tokio::time::timeout(Duration::from_secs(2), client.next())
            .await
            .expect("a heartbeat must arrive within 2s")
            .unwrap()
            .unwrap();
        if let Message::Text(text) = message
            && let Ok(StreamFrame::Control(StreamControl::Heartbeat { sequence })) =
                serde_json::from_str::<StreamFrame>(text.as_str())
        {
            beats += 1;
            assert_eq!(
                sequence, beats as u64,
                "heartbeat sequences must be monotonic"
            );
        }
    }
    assert_eq!(beats, 2, "the server must beat repeatedly, not once");
}

#[tokio::test]
async fn a_connection_without_the_launch_token_is_refused() {
    let (_hub, server) = server_with(HubConfig::default()).await;

    let unauthenticated = format!("ws://127.0.0.1:{}/stream", server.port());
    assert!(
        tokio_tungstenite::connect_async(&unauthenticated)
            .await
            .is_err(),
        "a local listener must not serve any process that finds the port"
    );

    let wrong_token = format!("ws://127.0.0.1:{}/stream?token=guess", server.port());
    assert!(
        tokio_tungstenite::connect_async(&wrong_token)
            .await
            .is_err(),
        "a wrong token must be refused"
    );

    // The server keeps serving legitimate clients afterwards.
    let mut client = connect(&server).await;
    send_control(
        &mut client,
        StreamControl::Subscribe {
            channels: vec![ChannelSubscription::new(COUNTER)],
        },
    )
    .await;
    expect_subscribed(&mut client, &[COUNTER]).await;
    assert_eq!(server.metrics().rejected, 2);
}

#[tokio::test]
async fn unsubscribing_stops_delivery_over_the_socket() {
    let (hub, server) = server_with(HubConfig::default()).await;
    let mut client = connect(&server).await;

    send_control(
        &mut client,
        StreamControl::Subscribe {
            channels: vec![ChannelSubscription::new(COUNTER)],
        },
    )
    .await;
    expect_subscribed(&mut client, &[COUNTER]).await;
    hub.publish(COUNTER, serde_json::json!({ "tick": 0 }));
    assert_eq!(next_sequences(&mut client, COUNTER, 1).await, vec![1]);

    send_control(
        &mut client,
        StreamControl::Unsubscribe {
            channels: vec![COUNTER.to_string()],
        },
    )
    .await;
    match next_frame(&mut client).await {
        StreamFrame::Control(StreamControl::Unsubscribed { channels }) => {
            assert_eq!(channels, vec![COUNTER.to_string()]);
        }
        other => panic!("expected an unsubscribe acknowledgement, got {other:?}"),
    }

    // Wait for the hub to observe the unsubscription, then publish.
    while hub.subscriber_count(COUNTER) > 0 {
        tokio::task::yield_now().await;
    }
    hub.publish(COUNTER, serde_json::json!({ "tick": 1 }));

    // Only heartbeats should arrive now. `next_frame` filters them, so a
    // data frame here would be a delivery to a channel nobody subscribed to.
    let quiet = tokio::time::timeout(Duration::from_millis(300), next_frame(&mut client)).await;
    assert!(
        quiet.is_err(),
        "no data may be delivered after unsubscribing, got {quiet:?}"
    );
}

#[tokio::test]
async fn a_malformed_frame_is_discarded_without_dropping_the_connection() {
    let (hub, server) = server_with(HubConfig::default()).await;
    let mut client = connect(&server).await;

    client
        .send(Message::Text("{not json at all".into()))
        .await
        .unwrap();
    client
        .send(Message::Text(
            r#"{"kind":"control","type":"nonsense"}"#.into(),
        ))
        .await
        .unwrap();

    send_control(
        &mut client,
        StreamControl::Subscribe {
            channels: vec![ChannelSubscription::new(COUNTER)],
        },
    )
    .await;
    expect_subscribed(&mut client, &[COUNTER]).await;
    hub.publish(COUNTER, serde_json::json!({ "tick": 0 }));
    assert_eq!(next_sequences(&mut client, COUNTER, 1).await, vec![1]);
}

#[tokio::test]
async fn two_clients_receive_the_same_stream_independently() {
    let (hub, server) = server_with(HubConfig::default()).await;
    let mut first = connect(&server).await;
    let mut second = connect(&server).await;

    for client in [&mut first, &mut second] {
        send_control(
            client,
            StreamControl::Subscribe {
                channels: vec![ChannelSubscription::new(COUNTER)],
            },
        )
        .await;
        expect_subscribed(client, &[COUNTER]).await;
    }

    for tick in 0..20u64 {
        hub.publish(COUNTER, serde_json::json!({ "tick": tick }));
    }

    let expected: Vec<u64> = (1..=20).collect();
    assert_eq!(next_sequences(&mut first, COUNTER, 20).await, expected);
    assert_eq!(next_sequences(&mut second, COUNTER, 20).await, expected);
    assert_eq!(server.metrics().accepted, 2);
}

#[tokio::test]
async fn a_hundred_hertz_stream_arrives_complete_and_in_order() {
    // The Task 1.4 demo stream: a 100Hz counter. Published at cadence rather
    // than in a burst, so this exercises the wake-per-message path the
    // frontend actually sees.
    let (hub, server) = server_with(HubConfig::default()).await;
    let mut client = connect(&server).await;
    send_control(
        &mut client,
        StreamControl::Subscribe {
            channels: vec![ChannelSubscription::new(COUNTER)],
        },
    )
    .await;
    expect_subscribed(&mut client, &[COUNTER]).await;

    let publisher = {
        let hub = hub.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_millis(10));
            for tick in 0..100u64 {
                ticker.tick().await;
                hub.publish(COUNTER, serde_json::json!({ "tick": tick }));
            }
        })
    };

    let sequences = next_sequences(&mut client, COUNTER, 100).await;
    assert_eq!(sequences, (1..=100).collect::<Vec<u64>>());
    assert_eq!(
        hub.metrics().dropped,
        0,
        "a reader keeping up must lose nothing"
    );
    publisher.await.unwrap();
}
