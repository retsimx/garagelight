#![no_std]
#![no_main]

use defmt_rtt as _;
use panic_probe as _;

use embassy_executor::Spawner;
use embassy_rp::uart;
use embassy_rp::watchdog::{ResetReason, Watchdog};
use embassy_time::{Duration, Timer};

mod blobs;
mod logging;
#[allow(dead_code)]
mod secrets;

/// Watchdog timeout. Used for both the initial arm and every feed, so the two
/// must stay equal or the watchdog fires early; one constant keeps them coupled.
const WATCHDOG_PERIOD: Duration = Duration::from_secs(8);

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    let tx = uart::UartTx::new_blocking(p.UART0, p.PIN_0, uart::Config::default());
    logging::init(tx);

    let mut wd = Watchdog::new(p.WATCHDOG);
    let reset_reason = match wd.reset_reason() {
        Some(ResetReason::Forced) => "watchdog-forced",
        Some(ResetReason::TimedOut) => "watchdog-timeout",
        None => "power-on-or-debugger",
    };

    crate::logln!("garagelight version={}", garagelight_core::VERSION);
    crate::logln!("reset_reason={}", reset_reason);

    let wifi = core::hint::black_box(blobs::WIFI_FW).len();
    let bt = core::hint::black_box(blobs::BT_FW).len();
    let nvram = core::hint::black_box(blobs::NVRAM).len();
    let clm = core::hint::black_box(blobs::CLM).len();
    crate::logln!(
        "blobs wifi={} bt={} nvram={} clm={} total={}",
        wifi,
        bt,
        nvram,
        clm,
        wifi + bt + nvram + clm
    );

    wd.start(WATCHDOG_PERIOD);
    spawner.spawn(watchdog_task(wd).unwrap());

    loop {
        Timer::after(Duration::from_secs(3600)).await;
    }
}

#[embassy_executor::task]
async fn watchdog_task(mut wd: Watchdog) -> ! {
    loop {
        wd.feed(WATCHDOG_PERIOD);
        Timer::after(Duration::from_millis(100)).await;
    }
}
