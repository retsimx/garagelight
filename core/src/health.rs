//! Pure health supervision policy and watchdog scratch reset-reason codec.
//!
//! No hardware and no clock live here. The caller injects `now_ms` and raw
//! signal snapshots; this module decides whether a bounded chip reset is due
//! and encodes/decodes the persistent `WATCHDOG_SCRATCH0` reset reason.

/// Magic high half-word stored in `WATCHDOG_SCRATCH0` (`0xA55A_0000`).
const SCRATCH_MAGIC: u32 = 0xA55A_0000;

/// Maximum time after boot before `boot_ready` must be reached (60 s).
pub const BOOT_TIMEOUT_MS: u64 = 60_000;

/// Maximum time without a core1 sensor loop heartbeat before reset (120 s).
pub const CORE1_STALL_TIMEOUT_MS: u64 = 120_000;

/// Maximum continuous time with BLE unhealthy before reset (60 s).
pub const BLE_ERROR_TIMEOUT_MS: u64 = 60_000;

/// Maximum continuous time without Wi-Fi IP association before reset (600 s).
pub const NO_IP_RESET_MS: u64 = 600_000;

/// Maximum time without MQTT liveness progress before reset (600 s).
pub const MQTT_OUTAGE_TIMEOUT_MS: u64 = 600_000;

const _: () = assert!(crate::wifi::NO_IP_RESET.as_secs() * 1000 == NO_IP_RESET_MS);

/// Reason recorded across an RP2040 watchdog reset via `WATCHDOG_SCRATCH0`.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RebootReason {
    BootTimeout = 1,
    Core1Stall = 2,
    BleError = 3,
    NoIp = 4,
    MqttOutage = 5,
    OtaSwap = 6,
    SelftestRevert = 7,
}

impl RebootReason {
    /// Kebab-case identifier for logs and diagnostics.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BootTimeout => "boot-timeout",
            Self::Core1Stall => "core1-stall",
            Self::BleError => "ble-error",
            Self::NoIp => "no-ip",
            Self::MqttOutage => "mqtt-outage",
            Self::OtaSwap => "ota-swap",
            Self::SelftestRevert => "selftest-revert",
        }
    }
}

/// Encode a [`RebootReason`] into a `u32` suitable for `WATCHDOG_SCRATCH0`.
///
/// Layout: `[16-bit magic 0xA55A][8-bit !code][8-bit code]`.
pub const fn encode_scratch(reason: RebootReason) -> u32 {
    let code = reason as u8;
    let inv = !code;
    SCRATCH_MAGIC | ((inv as u32) << 8) | (code as u32)
}

/// Decode a `WATCHDOG_SCRATCH0` word into a [`RebootReason`].
///
/// Rejects `0`, `0xFFFF_FFFF`, mismatched magic, corrupted bitwise complement,
/// and unknown variant codes.
pub const fn decode_scratch(word: u32) -> Option<RebootReason> {
    if (word & 0xFFFF_0000) != SCRATCH_MAGIC {
        return None;
    }
    let code = (word & 0xFF) as u8;
    let inv = ((word >> 8) & 0xFF) as u8;
    if inv != !code {
        return None;
    }
    match code {
        1 => Some(RebootReason::BootTimeout),
        2 => Some(RebootReason::Core1Stall),
        3 => Some(RebootReason::BleError),
        4 => Some(RebootReason::NoIp),
        5 => Some(RebootReason::MqttOutage),
        6 => Some(RebootReason::OtaSwap),
        7 => Some(RebootReason::SelftestRevert),
        _ => None,
    }
}

/// Snapshot of system liveness signals fed to [`HealthMonitor::evaluate`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HealthSignals {
    pub boot_ready: bool,
    pub core1_seq: u32,
    pub ble_healthy: bool,
    pub wifi_associated: bool,
    pub mqtt_seq: u32,
    pub ota_flashing: bool,
}

/// Pure, monotonic health policy evaluator.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HealthMonitor {
    boot_started_ms: Option<u64>,
    boot_ready: bool,
    last_core1_seq: u32,
    core1_progress_ms: u64,
    ble_unhealthy_since_ms: Option<u64>,
    wifi_down_since_ms: Option<u64>,
    last_mqtt_seq: u32,
    mqtt_progress_ms: u64,
}

impl HealthMonitor {
    /// Create a new monitor with no recorded timestamps yet.
    pub const fn new() -> Self {
        Self {
            boot_started_ms: None,
            boot_ready: false,
            last_core1_seq: 0,
            core1_progress_ms: 0,
            ble_unhealthy_since_ms: None,
            wifi_down_since_ms: None,
            last_mqtt_seq: 0,
            mqtt_progress_ms: 0,
        }
    }

    /// Evaluate current [`HealthSignals`] at `now_ms`, returning a
    /// [`RebootReason`] if any supervised subsystem has exceeded its timeout.
    pub fn evaluate(&mut self, now_ms: u64, signals: HealthSignals) -> Option<RebootReason> {
        if !signals.boot_ready {
            let started = *self.boot_started_ms.get_or_insert(now_ms);
            if now_ms.saturating_sub(started) >= BOOT_TIMEOUT_MS {
                return Some(RebootReason::BootTimeout);
            }
            return None;
        }

        if !self.boot_ready {
            self.boot_ready = true;
            self.last_core1_seq = signals.core1_seq;
            self.core1_progress_ms = now_ms;
            self.last_mqtt_seq = signals.mqtt_seq;
            self.mqtt_progress_ms = now_ms;
            self.ble_unhealthy_since_ms = if signals.ble_healthy {
                None
            } else {
                Some(now_ms)
            };
            self.wifi_down_since_ms = if signals.wifi_associated {
                None
            } else {
                Some(now_ms)
            };
        } else {
            if signals.core1_seq != self.last_core1_seq {
                self.last_core1_seq = signals.core1_seq;
                self.core1_progress_ms = now_ms;
            }
            if signals.mqtt_seq != self.last_mqtt_seq {
                self.last_mqtt_seq = signals.mqtt_seq;
                self.mqtt_progress_ms = now_ms;
            }
            if signals.ble_healthy {
                self.ble_unhealthy_since_ms = None;
            } else if self.ble_unhealthy_since_ms.is_none() {
                self.ble_unhealthy_since_ms = Some(now_ms);
            }
            if signals.wifi_associated {
                self.wifi_down_since_ms = None;
            } else if self.wifi_down_since_ms.is_none() {
                self.wifi_down_since_ms = Some(now_ms);
            }
        }

        if signals.ota_flashing {
            self.core1_progress_ms = now_ms;
            return None;
        }

        if let Some(since) = self.ble_unhealthy_since_ms {
            if now_ms.saturating_sub(since) >= BLE_ERROR_TIMEOUT_MS {
                return Some(RebootReason::BleError);
            }
        }

        if now_ms.saturating_sub(self.core1_progress_ms) >= CORE1_STALL_TIMEOUT_MS {
            return Some(RebootReason::Core1Stall);
        }

        if let Some(since) = self.wifi_down_since_ms {
            if now_ms.saturating_sub(since) >= NO_IP_RESET_MS {
                return Some(RebootReason::NoIp);
            }
        }

        if now_ms.saturating_sub(self.mqtt_progress_ms) >= MQTT_OUTAGE_TIMEOUT_MS {
            return Some(RebootReason::MqttOutage);
        }

        None
    }
}

impl Default for HealthMonitor {
    fn default() -> Self {
        Self::new()
    }
}
