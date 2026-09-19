//! MQTT telemetry policy: payload bytes, broker parsing, and reset dispatch.
//!
//! Pure and host-testable (L0). The network session and reconnect loop live in
//! `app`; nothing here touches a peripheral.

use core::fmt::{self, Write};
use core::net::Ipv4Addr;

use crate::sensor::Sample;

pub const CLIENT_ID: &str = "garagelight";
pub const BROKER_PORT: u16 = 1883;
pub const KEEPALIVE_SECS: u16 = 60;
pub const RESET_TOPIC: &str = "garagelight/reset";
/// Upper bound for the largest sample payload
/// (`{"temp": -128, "humidity": 255}`, 31 bytes).
pub const MAX_SAMPLE_PAYLOAD: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Inbound {
    Ignore,
    RequestUpdate,
}

/// Exact-match classification: only `garagelight/reset` requests an update;
/// any other topic (including `garagelight/resetx`) is ignored.
pub fn classify_inbound(topic: &str) -> Inbound {
    if topic == RESET_TOPIC {
        Inbound::RequestUpdate
    } else {
        Inbound::Ignore
    }
}

struct SliceWriter<'a> {
    buf: &'a mut [u8],
    len: usize,
}

impl Write for SliceWriter<'_> {
    // Bounds are checked before every copy, so a short slice yields `Err`
    // instead of panicking. A failed write may leave partial bytes behind.
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let bytes = s.as_bytes();
        let end = self.len.checked_add(bytes.len()).ok_or(fmt::Error)?;
        if end > self.buf.len() {
            return Err(fmt::Error);
        }
        self.buf[self.len..end].copy_from_slice(bytes);
        self.len = end;
        Ok(())
    }
}

/// Write `{"temp": <t>, "humidity": <h>}` with the exact default separators
/// Python's `json.dumps` emits. Returns the byte length, or `None` if `out` is
/// too small. Never panics and never writes out of bounds.
pub fn encode_sample(sample: Sample, out: &mut [u8]) -> Option<usize> {
    let mut writer = SliceWriter { buf: out, len: 0 };
    write!(
        writer,
        "{{\"temp\": {}, \"humidity\": {}}}",
        sample.temperature, sample.relative_humidity
    )
    .ok()?;
    Some(writer.len)
}

/// Parse `MQTT_BROKER`: an IPv4 literal with an optional `mqtt://` scheme and
/// optional `:port` defaulting to [`BROKER_PORT`]. Hostnames, malformed
/// addresses, IPv6 literals, and port `0` are rejected (no DNS on the device).
pub fn parse_broker(value: &str) -> Option<(Ipv4Addr, u16)> {
    let rest = value.strip_prefix("mqtt://").unwrap_or(value);
    let (host, port) = match rest.rsplit_once(':') {
        Some((host, port)) => (host, port.parse::<u16>().ok()?),
        None => (rest, BROKER_PORT),
    };
    if port == 0 {
        return None;
    }
    Some((host.parse::<Ipv4Addr>().ok()?, port))
}
