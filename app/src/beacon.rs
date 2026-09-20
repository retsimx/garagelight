//! Shared cyw43 control + a boot/OTA LED code beacon.
use cyw43::Control;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Timer};

pub type SharedControl = Mutex<CriticalSectionRawMutex, Control<'static>>;

/// Blink the onboard LED `n` times as long pulses (on 700 ms / off 400 ms),
/// then a 2.5 s gap. Long pulses are easy to count and distinct from the short
/// version beacon. `n == 0` blinks nothing.
pub async fn code(control: &SharedControl, n: u32) {
    if n == 0 {
        return;
    }
    let mut c = control.lock().await;
    for _ in 0..n {
        c.gpio_set(0, true).await;
        Timer::after(Duration::from_millis(700)).await;
        c.gpio_set(0, false).await;
        Timer::after(Duration::from_millis(400)).await;
    }
    Timer::after(Duration::from_millis(2500)).await;
}
