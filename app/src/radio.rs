use cyw43_pio::{PioSpi, DEFAULT_CLOCK_DIVIDER};
use embassy_executor::Spawner;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH0, DMA_CH1, PIN_23, PIN_24, PIN_25, PIN_29, PIO0};
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_rp::{bind_interrupts, dma, Peri};
use static_cell::StaticCell;

use crate::{blobs, logln};

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
    DMA_IRQ_0 => dma::InterruptHandler<DMA_CH0>, dma::InterruptHandler<DMA_CH1>;
});

pub struct RadioPeripherals {
    pub pwr: Peri<'static, PIN_23>,
    pub cs: Peri<'static, PIN_25>,
    pub dio: Peri<'static, PIN_24>,
    pub clk: Peri<'static, PIN_29>,
    pub pio: Peri<'static, PIO0>,
    pub dma_tx: Peri<'static, DMA_CH0>,
    pub dma_rx: Peri<'static, DMA_CH1>,
}

pub type BleController =
    trouble_host::prelude::ExternalController<cyw43::bluetooth::BtDriver<'static>, 1>;

pub struct Radio {
    pub net_device: cyw43::NetDriver<'static>,
    pub control: cyw43::Control<'static>,
    pub ble: BleController,
}

#[embassy_executor::task]
async fn cyw43_task(
    runner: cyw43::Runner<
        'static,
        cyw43::SpiBus<Output<'static>, PioSpi<'static, PIO0, 0>>,
        cyw43::Cyw43439,
    >,
) -> ! {
    runner.run().await
}

pub async fn init(spawner: Spawner, p: RadioPeripherals) -> Radio {
    let pwr = Output::new(p.pwr, Level::Low);
    let cs = Output::new(p.cs, Level::High);
    let mut pio = Pio::new(p.pio, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        DEFAULT_CLOCK_DIVIDER,
        pio.irq0,
        cs,
        p.dio,
        p.clk,
        dma::Channel::new(p.dma_tx, Irqs),
        dma::Channel::new(p.dma_rx, Irqs),
    );

    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let state = STATE.init(cyw43::State::new());
    let (net_device, bt_driver, mut control, runner) =
        cyw43::new_with_bluetooth(state, pwr, spi, blobs::WIFI_FW, blobs::BT_FW, blobs::NVRAM)
            .await;
    spawner.spawn(cyw43_task(runner).unwrap());

    control.init(blobs::CLM).await;
    control
        .set_power_management(cyw43::PowerManagementMode::None)
        .await;
    logln!("radio power_management=none");

    let ble = trouble_host::prelude::ExternalController::<_, 1>::new(bt_driver);

    Radio {
        net_device,
        control,
        ble,
    }
}
