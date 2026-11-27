//! MQTT telemetry policy: payload bytes, broker parsing, and reset dispatch.
//!
//! Pure and host-testable (L0). The network session and reconnect loop live in
//! `app`; nothing here touches a peripheral.

use core::fmt::{self, Write};
use core::net::Ipv4Addr;

use crate::lamp::FaultReason;
use crate::sensor::Sample;

pub const CLIENT_ID: &str = "garagelight";
pub const BROKER_PORT: u16 = 1883;
pub const KEEPALIVE_SECS: u16 = 60;
pub const RESET_TOPIC: &str = "garagelight/reset";
/// Upper bound for the largest sample payload
/// (`{"temp": -128, "humidity": 255, "version": 4294967295}`, 54 bytes).
pub const MAX_SAMPLE_PAYLOAD: usize = 54;
/// MQTT topic for compact production diagnostics.
pub const DIAG_TOPIC: &str = "garagelight/diag";
/// Periodic diagnostic publication interval.
pub const DIAG_INTERVAL_MS: u64 = 60_000;
/// Upper bound for a compact `garagelight/diag` JSON payload.
pub const MAX_DIAG_PAYLOAD: usize = 160;

/// Saturating policy for production counters exposed over diagnostics.
pub const fn saturating_counter_next(value: u32) -> u32 {
    value.saturating_add(1)
}

/// Pure snapshot consumed by the compact `garagelight/diag` encoder.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LampDiagnosticSnapshot<'a> {
    pub v: u32,
    pub up_s: u64,
    pub rx: u32,
    pub app: u32,
    pub flt: u32,
    pub why: FaultReason,
    pub ble_err: u32,
    pub rst: &'a str,
    pub reb: &'a str,
}

impl<'a> LampDiagnosticSnapshot<'a> {
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        v: u32,
        up_s: u64,
        rx: u32,
        app: u32,
        flt: u32,
        why: FaultReason,
        ble_err: u32,
        rst: &'a str,
        reb: &'a str,
    ) -> Self {
        Self {
            v,
            up_s,
            rx,
            app,
            flt,
            why,
            ble_err,
            rst,
            reb,
        }
    }
}

/// Whether a compact diagnostic publication is due for a MQTT session.
///
/// This is pure policy: publish at the 60 s periodic boundary or immediately
/// when the lamp fault-entry counter differs from the last published snapshot.
pub const fn diag_publish_due(
    elapsed_period_ms: u64,
    last_fault_entries: u32,
    fault_entries: u32,
) -> bool {
    elapsed_period_ms >= DIAG_INTERVAL_MS || last_fault_entries != fault_entries
}

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

/// Write `{"temp": <t>, "humidity": <h>, "version": <v>}` with the exact
/// default separators Python's `json.dumps` emits. Returns the byte length,
/// or `None` if `out` is too small. Never panics and never writes out of
/// bounds. The firmware version is passed in (keeps this pure and free of app
/// identity).
pub fn encode_sample(sample: Sample, version: u32, out: &mut [u8]) -> Option<usize> {
    let mut writer = SliceWriter { buf: out, len: 0 };
    write!(
        writer,
        "{{\"temp\": {}, \"humidity\": {}, \"version\": {}}}",
        sample.temperature, sample.relative_humidity, version
    )
    .ok()?;
    Some(writer.len)
}

/// Return only one of the three hardware reset identifiers accepted by the
/// diagnostic contract. Unknown values stay valid JSON and become `unknown`.
fn safe_reset_reason(reset_reason: &str) -> &'static str {
    match reset_reason {
        "watchdog-forced" => "watchdog-forced",
        "watchdog-timeout" => "watchdog-timeout",
        "power-on-or-debugger" => "power-on-or-debugger",
        _ => "unknown",
    }
}

/// Return a valid compact reboot identifier. Unknown values become `unknown`.
fn safe_reboot_reason(reboot_reason: &str) -> &'static str {
    match reboot_reason {
        "none" => "none",
        "boot-timeout" => "boot-timeout",
        "core1-stall" => "core1-stall",
        "ble-error" => "ble-error",
        "no-ip" => "no-ip",
        "mqtt-outage" => "mqtt-outage",
        "ota-swap" => "ota-swap",
        "selftest-revert" => "selftest-revert",
        _ => "unknown",
    }
}

/// Encode exactly one compact `garagelight/diag` JSON object.
///
/// Integer fields use decimal formatting only; string reasons are allow-listed
/// before formatting, and the result is bounded to [`MAX_DIAG_PAYLOAD`].
pub fn encode_diag(snapshot: LampDiagnosticSnapshot<'_>, out: &mut [u8]) -> Option<usize> {
    let limited_len = out.len().min(MAX_DIAG_PAYLOAD);
    let limited = &mut out[..limited_len];
    let mut writer = SliceWriter {
        buf: limited,
        len: 0,
    };
    let rst = safe_reset_reason(snapshot.rst);
    let reb = safe_reboot_reason(snapshot.reb);
    write!(
        writer,
        "{{\"v\":{},\"up_s\":{},\"rx\":{},\"app\":{},\"flt\":{},\"why\":\"{}\",\"ble_err\":{},\"rst\":\"{}\",\"reb\":\"{}\"}}",
        snapshot.v,
        snapshot.up_s,
        snapshot.rx,
        snapshot.app,
        snapshot.flt,
        snapshot.why.as_str(),
        snapshot.ble_err,
        rst,
        reb,
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
