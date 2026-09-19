//! Post-swap self-test policy.
//!
//! A freshly swapped image must prove itself before it is confirmed. This is
//! fail-closed on the **control path only**: the two mandatory conditions are
//! BLE advertising and lamp readiness, and a missing one reverts the image.
//! WiFi association and MQTT connectivity are **best-effort** — they are
//! carried in the report for logging and never affect the verdict.
//!
//! [`self_test`] confirms the moment both mandatory conditions have latched.
//! The window is a deadline, not a dwell: a good image is confirmed early, so
//! a power loss has the smallest possible window in which to revert it.

#![allow(async_fn_in_trait)]

pub const SELF_TEST_WINDOW_MS: u64 = 30_000;
pub const SELF_TEST_POLL_MS: u64 = 250;

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Signals {
    pub ble_advertising: bool,
    pub lamp_ready: bool,
    pub wifi_associated: bool,
    pub mqtt_connected: bool,
}

pub trait Probe {
    fn sample(&mut self) -> Signals;
}

pub trait Clock {
    fn now_ms(&self) -> u64;
    async fn wait(&mut self, ms: u64);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    Confirm,
    Revert,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Report {
    pub verdict: Verdict,
    pub ble_advertising: bool,
    pub lamp_ready: bool,
    pub wifi_associated: bool,
    pub mqtt_connected: bool,
    pub elapsed_ms: u64,
}

pub async fn self_test<P: Probe, C: Clock>(probe: &mut P, clock: &mut C) -> Report {
    let start = clock.now_ms();
    let mut ble_advertising = false;
    let mut lamp_ready = false;
    let mut latest;
    loop {
        let signals = probe.sample();
        ble_advertising |= signals.ble_advertising;
        lamp_ready |= signals.lamp_ready;
        latest = signals;
        if ble_advertising && lamp_ready {
            return Report {
                verdict: Verdict::Confirm,
                ble_advertising,
                lamp_ready,
                wifi_associated: latest.wifi_associated,
                mqtt_connected: latest.mqtt_connected,
                elapsed_ms: clock.now_ms() - start,
            };
        }
        let elapsed = clock.now_ms() - start;
        if elapsed >= SELF_TEST_WINDOW_MS {
            return Report {
                verdict: Verdict::Revert,
                ble_advertising,
                lamp_ready,
                wifi_associated: latest.wifi_associated,
                mqtt_connected: latest.mqtt_connected,
                elapsed_ms: elapsed,
            };
        }
        clock.wait(SELF_TEST_POLL_MS).await;
    }
}
