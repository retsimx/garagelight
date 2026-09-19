//! Post-swap self-test glue (GL-11).
//!
//! The window, the two mandatory conditions and the two best-effort signals
//! live in the pure, host-testable [`garagelight_core::ota::self_test`]. This
//! module is the thin adapter: [`BoardProbe`] snapshots the four module-local
//! status flags, [`BoardClock`] supplies embassy-time, and [`run`] drives the
//! policy only on a `State::Swap` boot.

#[cfg(not(feature = "selftest-fail-ble"))]
use crate::ble;
use crate::{lamp, logln, net, telemetry, update};
use embassy_boot::State;
use embassy_time::{Duration, Instant, Timer};
use garagelight_core::ota::{self_test, Clock, Probe, Signals, Verdict};

/// Samples the four observable subsystem flags.
struct BoardProbe;

impl Probe for BoardProbe {
    fn sample(&mut self) -> Signals {
        #[cfg(feature = "selftest-fail-ble")]
        let ble_advertising = false;
        #[cfg(not(feature = "selftest-fail-ble"))]
        let ble_advertising = ble::advertising();
        Signals {
            ble_advertising,
            lamp_ready: lamp::ready(),
            wifi_associated: net::associated(),
            mqtt_connected: telemetry::connected(),
        }
    }
}

/// embassy-time-backed clock for the self-test window.
struct BoardClock;

impl Clock for BoardClock {
    fn now_ms(&self) -> u64 {
        Instant::now().as_millis()
    }

    async fn wait(&mut self, ms: u64) {
        Timer::after(Duration::from_millis(ms)).await;
    }
}

fn verdict_name(v: Verdict) -> &'static str {
    match v {
        Verdict::Confirm => "confirm",
        Verdict::Revert => "revert",
    }
}

/// Self-test the freshly swapped image; confirm it or reset so the bootloader
/// reverts. Every other boot state returns immediately.
pub async fn run(updater: &mut update::Updater) {
    match updater.get_state().await {
        Ok(State::Swap) => {
            logln!("ota_selftest_start");
            let report = self_test(&mut BoardProbe, &mut BoardClock).await;
            logln!(
                "ota_selftest ble={} lamp={} wifi={} mqtt={} elapsed_ms={} verdict={}",
                report.ble_advertising,
                report.lamp_ready,
                report.wifi_associated,
                report.mqtt_connected,
                report.elapsed_ms,
                verdict_name(report.verdict)
            );
            match report.verdict {
                Verdict::Confirm => match updater.mark_booted().await {
                    Ok(()) => logln!("ota_selftest_confirmed"),
                    Err(_) => logln!("ota_selftest_confirm_failed"),
                },
                Verdict::Revert => {
                    logln!("ota_selftest_failed");
                    cortex_m::peripheral::SCB::sys_reset();
                }
            }
        }
        Ok(State::Boot) => logln!("ota_boot state=boot"),
        Ok(State::Revert) => logln!("ota_boot state=revert"),
        Ok(State::DfuDetach) => logln!("ota_boot state=dfu_detach"),
        Err(_) => logln!("ota_boot state=error"),
    }
}
