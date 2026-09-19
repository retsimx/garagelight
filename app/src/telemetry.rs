//! MQTT telemetry over WiFi (GL-8, retsimx/garagelight#9).
//!
//! One task owns one long-lived [`minimq::Session`] for the life of the program.
//! Each reconnect attempt opens a fresh [`TcpSocket`] and a fresh `Connection`;
//! the `Session` and its task-local packet buffers survive broker restarts, so
//! a deploy never needs a device reboot. Telemetry is best-effort: it is spawned
//! after the control path and WiFi, and it never resets the device.
//!
//! # Buffer sizing (design 010 section 4.4)
//!
//! `minimq`'s model: RX holds one inbound packet; TX holds outbound encodes plus
//! the CONNECT workspace plus retained in-flight session state.
//!
//! - `RX_BYTES = 256` holds the largest inbound packet, a `garagelight/reset`
//!   `PUBLISH`: fixed header (<= 5) + topic (2 + 16) + properties (1) +
//!   payload (<= ~200) + varint margin. Operators may send arbitrary reset
//!   payloads; a larger one yields a clean `ResourceError`/disconnect, not
//!   corruption.
//! - `TX_BYTES = 512` holds the CONNECT workspace (~50 B) + the retained
//!   SUBSCRIBE (~25 B) + the QoS-0 publish scratch (`MAX_FIXED_HEADER_SIZE` +
//!   topic + 32-byte payload ~= 60 B) + reconnect headroom. QoS 0 carries no
//!   replay state, but the SUBSCRIBE is retained until SUBACK; 512 is ~4x the
//!   worst-case single encode.

use core::sync::atomic::{AtomicBool, Ordering};

use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_net::{IpAddress, IpEndpoint, Stack};
use embassy_time::{with_timeout, Duration, Timer};
use minimq::{Buffers, ConfigBuilder, ConnectEvent, Publication, QoS, Session, TopicFilter};

use garagelight_core::contract::MQTT_TOPIC;
use garagelight_core::telemetry::{
    self, classify_inbound, encode_sample, Inbound, CLIENT_ID, KEEPALIVE_SECS, MAX_SAMPLE_PAYLOAD,
    RESET_TOPIC,
};

use crate::{logln, secrets, sensors, update};

const RX_BYTES: usize = 256;
const TX_BYTES: usize = 512;
const TCP_RX_BYTES: usize = 512;
const TCP_TX_BYTES: usize = 512;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const TICK: Duration = Duration::from_secs(1);

/// Bound on polls spent waiting for the SUBSCRIBE `Op` to complete.
const SUBSCRIBE_POLL_LIMIT: u32 = 8;

/// Reconnect backoff: 1s doubling to a 60s ceiling, reset on a clean disconnect
/// (mirrors `net.rs`).
const BACKOFF_INITIAL_SECS: u64 = 1;
const BACKOFF_MAX_SECS: u64 = 60;

/// Reason a connection attempt ended, logged and used by the supervisor.
type SessionEnd = &'static str;

/// True while an MQTT session is connected to the broker, cleared when
/// `connect_and_run` returns (GL-11).
static CONNECTED: AtomicBool = AtomicBool::new(false);

/// Whether the MQTT session is currently connected to the broker.
pub fn connected() -> bool {
    CONNECTED.load(Ordering::Relaxed)
}

/// Spawn the telemetry task, passing it the `Copy` network handle.
pub fn spawn(spawner: Spawner, stack: Stack<'static>) {
    spawner.spawn(telemetry_task(stack).unwrap());
}

#[embassy_executor::task]
async fn telemetry_task(stack: Stack<'static>) -> ! {
    let mut rx = [0u8; RX_BYTES];
    let mut tx = [0u8; TX_BYTES];
    let mut session = Session::new(
        ConfigBuilder::new(Buffers::new(&mut rx, &mut tx))
            .client_id(CLIENT_ID)
            .unwrap()
            .keepalive_interval(KEEPALIVE_SECS),
    );

    let mut backoff_secs = BACKOFF_INITIAL_SECS;
    loop {
        // Wait for DHCP before spending a connect attempt on an unconfigured link.
        stack.wait_config_up().await;
        match connect_and_run(&mut session, stack).await {
            Ok(()) => backoff_secs = BACKOFF_INITIAL_SECS,
            Err(reason) => logln!("mqtt_session_end err={}", reason),
        }
        logln!("mqtt_retry_in_s={}", backoff_secs);
        Timer::after(Duration::from_secs(backoff_secs)).await;
        backoff_secs = core::cmp::min(backoff_secs * 2, BACKOFF_MAX_SECS);
    }
}

/// Open one TCP connection, handshake, subscribe (on a fresh session) and drive
/// the poll/publish loop until the connection ends. The `Session` is borrowed,
/// so it persists across attempts.
async fn connect_and_run(
    session: &mut Session<'_>,
    stack: Stack<'static>,
) -> Result<(), SessionEnd> {
    CONNECTED.store(false, Ordering::Relaxed);
    let result = run_session(session, stack).await;
    CONNECTED.store(false, Ordering::Relaxed);
    result
}

/// One session attempt; see [`connect_and_run`].
async fn run_session(session: &mut Session<'_>, stack: Stack<'static>) -> Result<(), SessionEnd> {
    let (addr, port) = telemetry::parse_broker(secrets::MQTT_BROKER).ok_or_else(|| {
        logln!("mqtt_broker_invalid");
        "broker_invalid"
    })?;

    let mut tcp_rx = [0u8; TCP_RX_BYTES];
    let mut tcp_tx = [0u8; TCP_TX_BYTES];
    let mut socket = TcpSocket::new(stack, &mut tcp_rx, &mut tcp_tx);
    let endpoint = IpEndpoint::new(IpAddress::Ipv4(addr), port);

    match with_timeout(CONNECT_TIMEOUT, socket.connect(endpoint)).await {
        Ok(Ok(())) => {}
        Ok(Err(_)) => {
            logln!("mqtt_connect_failed");
            return Err("connect_failed");
        }
        Err(_) => {
            logln!("mqtt_connect_timeout");
            return Err("connect_timeout");
        }
    }

    let mut conn = match with_timeout(HANDSHAKE_TIMEOUT, session.connect(socket)).await {
        Ok(Ok(conn)) => conn,
        Ok(Err(_)) => {
            logln!("mqtt_handshake_failed");
            return Err("handshake_failed");
        }
        Err(_) => {
            logln!("mqtt_handshake_timeout");
            return Err("handshake_timeout");
        }
    };
    logln!("mqtt_connected");

    match conn.connect_event() {
        ConnectEvent::Connected => {
            CONNECTED.store(true, Ordering::Relaxed);
            let filters = [TopicFilter::new(RESET_TOPIC)];
            let op = conn.subscribe(&filters, &[]).await.map_err(|_| {
                logln!("mqtt_subscribe_failed");
                "subscribe_failed"
            })?;
            // Drive the SUBACK to completion before entering the steady-state
            // loop. Bounded so a silent broker cannot stall the task.
            let mut polls = 0;
            while !conn.is_complete(&op)
                && !conn.is_invalidated(&op)
                && polls < SUBSCRIBE_POLL_LIMIT
            {
                match with_timeout(TICK, conn.poll()).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(minimq::Error::Disconnected)) => {
                        logln!("mqtt_disconnected");
                        return Ok(());
                    }
                    Ok(Err(_)) => {
                        logln!("mqtt_error");
                        return Err("subscribe_poll");
                    }
                    Err(_) => {}
                }
                polls += 1;
            }
            if conn.is_complete(&op) {
                logln!("mqtt_subscribed");
            } else {
                logln!("mqtt_subscribe_incomplete");
            }
        }
        // A resumed session already has the broker-side subscription.
        ConnectEvent::Reconnected => {
            CONNECTED.store(true, Ordering::Relaxed);
            logln!("mqtt_reconnected");
        }
    }

    loop {
        match with_timeout(TICK, conn.poll()).await {
            Ok(Ok(Some(msg))) => match classify_inbound(msg.topic()) {
                Inbound::RequestUpdate => {
                    logln!("mqtt_reset_received payload_len={}", msg.payload().len());
                    update::request_check();
                }
                Inbound::Ignore => logln!("mqtt_unknown_topic topic={}", msg.topic()),
            },
            Ok(Ok(None)) => {}
            Ok(Err(minimq::Error::Disconnected)) => {
                logln!("mqtt_disconnected");
                return Ok(());
            }
            Ok(Err(_)) => {
                logln!("mqtt_error");
                return Err("poll");
            }
            Err(_) => {}
        }

        // Publish only when a fresh sample is waiting; a failed DHT read
        // signals nothing and is therefore never published.
        if sensors::sample_pending() {
            let sample = sensors::wait_sample().await;
            let mut payload = [0u8; MAX_SAMPLE_PAYLOAD];
            let len = encode_sample(sample, &mut payload).ok_or_else(|| {
                logln!("mqtt_encode_failed");
                "encode_failed"
            })?;
            let publication = Publication::new(MQTT_TOPIC, &payload[..len]).qos(QoS::AtMostOnce);
            match conn.publish(publication).await {
                Ok(_) => {}
                Err(minimq::PubError::Session(minimq::Error::Disconnected)) => {
                    logln!("mqtt_disconnected");
                    return Ok(());
                }
                Err(_) => {
                    logln!("mqtt_publish_failed");
                    return Err("publish");
                }
            }
        }
    }
}
