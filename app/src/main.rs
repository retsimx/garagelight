#![no_std]
#![no_main]

use defmt_rtt as _;
use panic_probe as _;

use embassy_executor::Spawner;
use embassy_net::{Runner, StackResources};
use embassy_rp::uart;
use embassy_rp::watchdog::{ResetReason, Watchdog};
use embassy_time::{Duration, Timer};
use garagelight_app::radio::{self, RadioPeripherals};
use garagelight_app::{ble, blobs, logging, logln, update};
use static_cell::StaticCell;

const NET_SEED: u64 = 0x1234_5678_9abc_def0;

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, cyw43::NetDriver<'static>>) -> ! {
    runner.run().await
}

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

    logln!("garagelight version={}", garagelight_app::VERSION);
    logln!("reset_reason={}", reset_reason);

    let wifi = core::hint::black_box(blobs::WIFI_FW).len();
    let bt = core::hint::black_box(blobs::BT_FW).len();
    let nvram = core::hint::black_box(blobs::NVRAM).len();
    let clm = core::hint::black_box(blobs::CLM).len();
    logln!(
        "blobs wifi={} bt={} nvram={} clm={} total={}",
        wifi,
        bt,
        nvram,
        clm,
        wifi + bt + nvram + clm
    );

    // Arm and feed the one shared timeout, so the armed timeout and the flash
    // feed cadence cannot drift apart.
    wd.start(update::WATCHDOG_TIMEOUT);
    update::init_watchdog(wd);
    spawner.spawn(watchdog_task().unwrap());

    let radio = radio::init(
        spawner,
        RadioPeripherals {
            pwr: p.PIN_23,
            cs: p.PIN_25,
            dio: p.PIN_24,
            clk: p.PIN_29,
            pio: p.PIO0,
            dma_tx: p.DMA_CH0,
            dma_rx: p.DMA_CH1,
        },
    )
    .await;
    logln!("radio initialized");

    ble::spawn(spawner, radio.ble);
    logln!("ble control path started");

    static RESOURCES: StaticCell<StackResources<2>> = StaticCell::new();
    let (_stack, runner) = embassy_net::new(
        radio.net_device,
        embassy_net::Config::dhcpv4(Default::default()),
        RESOURCES.init(StackResources::new()),
        NET_SEED,
    );
    spawner.spawn(net_task(runner).unwrap());
    logln!("net runner spawned");

    let mut control = radio.control;
    logln!("led on");
    control.gpio_set(0, true).await;
    Timer::after(Duration::from_millis(500)).await;
    logln!("led off");
    control.gpio_set(0, false).await;
    Timer::after(Duration::from_millis(500)).await;

    loop {
        Timer::after(Duration::from_secs(3600)).await;
    }
}

#[embassy_executor::task]
async fn watchdog_task() -> ! {
    loop {
        update::feed(update::WATCHDOG_TIMEOUT);
        Timer::after(Duration::from_millis(100)).await;
    }
}
