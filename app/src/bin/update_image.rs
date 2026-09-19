//! GL-3 test-only candidate image (`--features update-selftest`).
//!
//! Booted from the ACTIVE partition by the `embassy-boot-rp` bootloader. In the
//! normal build it confirms itself with `mark_booted()`; built with
//! `--features selftest-broken` it deliberately never confirms, so the
//! bootloader must revert it on the next reset.

#![no_std]
#![no_main]

use defmt_rtt as _;
use panic_probe as _;

use embassy_boot::State;
use embassy_executor::Spawner;
use embassy_rp::uart;
use embassy_time::{Duration, Timer};
use garagelight_app::update::Updater;
use garagelight_app::{logging, logln};

fn state_name(state: State) -> &'static str {
    match state {
        State::Boot => "Boot",
        State::Swap => "Swap",
        State::Revert => "Revert",
        State::DfuDetach => "DfuDetach",
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    let tx = uart::UartTx::new_blocking(p.UART0, p.PIN_0, uart::Config::default());
    logging::init(tx);

    let mut updater = Updater::new(p.FLASH);
    let state = match updater.get_state().await {
        Ok(state) => state_name(state),
        Err(_) => "error",
    };
    logln!(
        "update_image active_base=0x{:x} version={} boot_state={}",
        garagelight_core::layout::ACTIVE_BASE,
        garagelight_app::VERSION,
        state
    );

    #[cfg(not(feature = "selftest-broken"))]
    {
        match updater.mark_booted().await {
            Ok(()) => logln!("image confirmed"),
            Err(_) => logln!("image confirm failed"),
        }
    }

    #[cfg(feature = "selftest-broken")]
    {
        logln!("BROKEN image: not confirming");
    }

    let mut beat: u32 = 0;
    loop {
        Timer::after(Duration::from_secs(2)).await;
        beat = beat.wrapping_add(1);
        logln!("update_image heartbeat={}", beat);
    }
}
