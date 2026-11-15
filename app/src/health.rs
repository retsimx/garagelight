//! Atomic health-signal snapshot and RAII guards for firmware supervision.
//!
//! Policy and timeout thresholds live in [`garagelight_core::health`]; this
//! module holds only the `portable_atomic` counters/flags updated by tasks and
//! sampled by the watchdog loop.

use portable_atomic::{AtomicBool, AtomicU32, Ordering};

static BOOT_READY: AtomicBool = AtomicBool::new(false);
static CORE1_SEQ: AtomicU32 = AtomicU32::new(0);
static BLE_HEALTHY: AtomicBool = AtomicBool::new(true);
static MQTT_SEQ: AtomicU32 = AtomicU32::new(0);
static OTA_FLASHING: AtomicBool = AtomicBool::new(false);

/// Mark that boot initialization (watchdog, core1, lamp, radio, BLE, net,
/// telemetry) has completed.
pub fn mark_boot_ready() {
    BOOT_READY.store(true, Ordering::Relaxed);
}

/// Record one iteration of the core1 sensor sampling loop.
pub fn heartbeat_core1() {
    CORE1_SEQ.fetch_add(1, Ordering::Relaxed);
}

/// Update BLE stack health (`true` while advertising/connected normally,
/// `false` on fatal/repeated runner or advertise errors).
pub fn set_ble_healthy(healthy: bool) {
    BLE_HEALTHY.store(healthy, Ordering::Relaxed);
}

/// Record MQTT liveness progress (publish ok, inbound message, or healthy poll
/// interval).
pub fn heartbeat_mqtt() {
    MQTT_SEQ.fetch_add(1, Ordering::Relaxed);
}

/// Set whether an OTA flash write/erase session is currently in progress.
fn set_ota_flashing(active: bool) {
    OTA_FLASHING.store(active, Ordering::Relaxed);
}

/// RAII guard that marks `ota_flashing` active on entry and clears it on drop,
/// even if the OTA update returns early on error.
pub struct OtaFlashGuard {
    _private: (),
}

impl OtaFlashGuard {
    /// Enter an OTA flash-writing section and defer post-boot health resets.
    pub fn enter() -> Self {
        set_ota_flashing(true);
        Self { _private: () }
    }
}

impl Drop for OtaFlashGuard {
    fn drop(&mut self) {
        set_ota_flashing(false);
    }
}

/// Snapshot all subsystem health signals for [`garagelight_core::HealthMonitor`].
pub fn snapshot() -> garagelight_core::HealthSignals {
    garagelight_core::HealthSignals {
        boot_ready: BOOT_READY.load(Ordering::Relaxed),
        core1_seq: CORE1_SEQ.load(Ordering::Relaxed),
        ble_healthy: BLE_HEALTHY.load(Ordering::Relaxed),
        wifi_associated: crate::net::associated(),
        mqtt_seq: MQTT_SEQ.load(Ordering::Relaxed),
        ota_flashing: OTA_FLASHING.load(Ordering::Relaxed),
    }
}
