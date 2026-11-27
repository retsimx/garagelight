//! Atomic health-signal snapshot and RAII guards for firmware supervision.
//!
//! Policy and timeout thresholds live in [`garagelight_core::health`]; this
//! module holds only the `portable_atomic` counters/flags updated by tasks and
//! sampled by the watchdog loop.

use portable_atomic::{AtomicBool, AtomicU32, Ordering};

fn saturating_fetch_add(counter: &AtomicU32) {
    let mut current = counter.load(Ordering::Relaxed);
    loop {
        let next = garagelight_core::saturating_counter_next(current);
        match counter.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return,
            Err(actual) => current = actual,
        }
    }
}

static BOOT_READY: AtomicBool = AtomicBool::new(false);
static CORE1_SEQ: AtomicU32 = AtomicU32::new(0);
static BLE_HEALTHY: AtomicBool = AtomicBool::new(true);
static MQTT_SEQ: AtomicU32 = AtomicU32::new(0);
static OTA_FLASHING: AtomicBool = AtomicBool::new(false);
static RX_FACTS: AtomicU32 = AtomicU32::new(0);
static LAMP_APPLIED: AtomicU32 = AtomicU32::new(0);
static FAULT_ENTRIES: AtomicU32 = AtomicU32::new(0);
static BLE_ERRS: AtomicU32 = AtomicU32::new(0);
static RESET_REASON: AtomicU32 = AtomicU32::new(0);
static REBOOT_REASON: AtomicU32 = AtomicU32::new(0);

/// Mark that boot initialization (watchdog, core1, lamp, radio, BLE, net,
/// telemetry) has completed.
pub fn mark_boot_ready() {
    BOOT_READY.store(true, Ordering::Relaxed);
}

/// Record one iteration of the core1 sensor sampling loop.
pub fn heartbeat_core1() {
    saturating_fetch_add(&CORE1_SEQ);
}

/// Update BLE stack health (`true` while advertising/connected normally,
/// `false` on fatal/repeated runner or advertise errors).
pub fn set_ble_healthy(healthy: bool) {
    BLE_HEALTHY.store(healthy, Ordering::Relaxed);
}

/// Record MQTT liveness progress (publish ok, inbound message, or healthy poll
/// interval).
pub fn heartbeat_mqtt() {
    saturating_fetch_add(&MQTT_SEQ);
}

/// Record one validated GATT fact received at the BLE boundary.
pub fn record_rx_fact() {
    saturating_fetch_add(&RX_FACTS);
}

/// Record one fact after the lamp task applies it and drives GPIO28.
pub fn record_lamp_applied() {
    saturating_fetch_add(&LAMP_APPLIED);
}

/// Record one transition into a fault reason.
pub fn record_fault_entry() {
    saturating_fetch_add(&FAULT_ENTRIES);
}

/// Record one invalid write or GATT boundary processing error.
pub fn record_ble_error() {
    saturating_fetch_add(&BLE_ERRS);
}

pub fn rx_facts() -> u32 {
    RX_FACTS.load(Ordering::Relaxed)
}

pub fn lamp_applied() -> u32 {
    LAMP_APPLIED.load(Ordering::Relaxed)
}

pub fn fault_entries() -> u32 {
    FAULT_ENTRIES.load(Ordering::Relaxed)
}

pub fn ble_errs() -> u32 {
    BLE_ERRS.load(Ordering::Relaxed)
}

/// Store the RP2040 reset-reason identifier from the boot-time Watchdog state.
pub fn set_reset_reason(reason: &str) {
    let code = match reason {
        "watchdog-forced" => 1,
        "watchdog-timeout" => 2,
        "power-on-or-debugger" => 3,
        _ => 0,
    };
    RESET_REASON.store(code, Ordering::Relaxed);
}

/// Get the compact reset-reason identifier for diagnostics.
pub fn reset_reason() -> &'static str {
    match RESET_REASON.load(Ordering::Relaxed) {
        1 => "watchdog-forced",
        2 => "watchdog-timeout",
        3 => "power-on-or-debugger",
        _ => "unknown",
    }
}

/// Store the reason decoded from `WATCHDOG_SCRATCH0` after boot.
pub fn set_reboot_reason(reason: Option<garagelight_core::RebootReason>) {
    REBOOT_REASON.store(reason.map_or(0, |value| value as u32), Ordering::Relaxed);
}

/// Get the compact reboot-reason identifier for diagnostics.
pub fn reboot_reason() -> &'static str {
    match REBOOT_REASON.load(Ordering::Relaxed) {
        1 => "boot-timeout",
        2 => "core1-stall",
        3 => "ble-error",
        4 => "no-ip",
        5 => "mqtt-outage",
        6 => "ota-swap",
        7 => "selftest-revert",
        _ => "none",
    }
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
