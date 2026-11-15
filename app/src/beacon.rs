//! Shared cyw43 control + a boot/OTA LED code beacon.
use cyw43::Control;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Timer};

pub type SharedControl = Mutex<CriticalSectionRawMutex, Control<'static>>;

const GPIO_TIMEOUT: Duration = Duration::from_secs(1);

async fn set_led_bounded(control: &SharedControl, on: bool) -> bool {
    if embassy_time::with_timeout(GPIO_TIMEOUT, async {
        let mut c = control.lock().await;
        c.gpio_set(0, on).await;
    })
    .await
    .is_err()
    {
        crate::logln!("beacon_gpio_timeout");
        return false;
    }
    true
}

/// Blink the onboard LED `n` times as long pulses (on 700 ms / off 400 ms),
/// then a 2.5 s gap. Long pulses are easy to count and distinct from the short
/// version beacon. `n == 0` blinks nothing.
///
/// The shared CYW43 control lock is acquired only around each `gpio_set` call
/// (bounded by [`GPIO_TIMEOUT`]) and released across sleeps so Wi-Fi
/// association and recovery are never stalled.
pub async fn code(control: &SharedControl, n: u32) {
    if n == 0 {
        return;
    }
    for _ in 0..n {
        if !set_led_bounded(control, true).await {
            return;
        }
        Timer::after(Duration::from_millis(700)).await;
        if !set_led_bounded(control, false).await {
            return;
        }
        Timer::after(Duration::from_millis(400)).await;
    }
    Timer::after(Duration::from_millis(2500)).await;
}
