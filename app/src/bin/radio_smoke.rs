#![no_std]
#![no_main]

use defmt_rtt as _;
use panic_probe as _;

use bt_hci::cmd::info::ReadLocalVersionInformation;
use bt_hci::controller::{Controller, ControllerCmdSync};
use embassy_executor::Spawner;
use embassy_net::{Runner, StackResources};
use embassy_rp::uart;
use embassy_time::{Duration, Timer};
use garagelight_app::radio::{self, BleController, RadioPeripherals};
use garagelight_app::{logging, logln};
use static_cell::StaticCell;

const NET_SEED: u64 = 0x1234_5678_9abc_def0;

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, cyw43::NetDriver<'static>>) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn hci_drain_task(ble: &'static BleController) -> ! {
    let mut buf = [0u8; 259];
    loop {
        let _ = ble.read(&mut buf).await;
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    let tx = uart::UartTx::new_blocking(p.UART0, p.PIN_0, uart::Config::default());
    logging::init(tx);

    logln!("radio_smoke start");

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

    static RESOURCES: StaticCell<StackResources<2>> = StaticCell::new();
    let (_stack, runner) = embassy_net::new(
        radio.net_device,
        embassy_net::Config::dhcpv4(Default::default()),
        RESOURCES.init(StackResources::new()),
        NET_SEED,
    );
    spawner.spawn(net_task(runner).unwrap());
    logln!("net runner spawned");

    static BLE: StaticCell<BleController> = StaticCell::new();
    let ble: &'static BleController = BLE.init(radio.ble);
    spawner.spawn(hci_drain_task(ble).unwrap());
    logln!("hci drain spawned");

    let mut control = radio.control;
    logln!("led on");
    control.gpio_set(0, true).await;
    Timer::after(Duration::from_millis(500)).await;
    logln!("led off");
    control.gpio_set(0, false).await;
    Timer::after(Duration::from_millis(500)).await;

    let mut scanner = control.scan(Default::default()).await;
    let mut ssids = 0u32;
    while let Some(bss) = scanner.next().await {
        let ssid = core::str::from_utf8(&bss.ssid).unwrap_or("<invalid-utf8>");
        logln!("wifi_ssid={}", ssid);
        ssids += 1;
    }
    logln!("wifi_scan ssids={}", ssids);

    let mut ble_ok = false;
    match ble.exec(&ReadLocalVersionInformation::new()).await {
        Ok(info) => {
            let company_identifier = info.company_identifier;
            let lmp_subversion = info.lmp_subversion;
            let hci_version = info.hci_version.into_inner();
            logln!(
                "ble_hci_ok company_identifier={} lmp_subversion={} hci_version={}",
                company_identifier,
                lmp_subversion,
                hci_version
            );
            ble_ok = true;
        }
        Err(_) => logln!("ble_hci_err"),
    }

    let wifi = if ssids >= 1 { "up" } else { "down" };
    let ble_state = if ble_ok { "up" } else { "down" };
    logln!("coexistence wifi={} ble={}", wifi, ble_state);

    loop {
        Timer::after(Duration::from_secs(3600)).await;
    }
}
