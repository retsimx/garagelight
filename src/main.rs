#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]
#![feature(async_fn_in_trait)]
#![allow(incomplete_features)]

use core::panic::PanicInfo;
use cyw43_pio::PioSpi;
use embassy_executor::Spawner;
use embassy_net::{IpEndpoint, Ipv4Address};
use embassy_net::udp::{PacketMetadata, UdpSocket};
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::peripherals::PIO0;
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_rp::watchdog::Watchdog;
use embassy_time::{Duration, Timer};
use {defmt_rtt as _};

mod wifi;

static REMOTE_ENDPOINT: IpEndpoint = IpEndpoint::new(embassy_net::IpAddress::Ipv4(Ipv4Address::new(10, 0, 0, 59)), 8000);

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
});

#[embassy_executor::task]
async fn wdt(mut watchdog: Watchdog) -> ! {
    watchdog.start(Duration::from_millis(500));

    loop {
        watchdog.feed();
        Timer::after(Duration::from_millis(100)).await;
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let peripherals = embassy_rp::init(Default::default());

    let watchdog = Watchdog::new(peripherals.WATCHDOG);

    spawner.spawn(wdt(watchdog)).unwrap();

    let pwr = Output::new(peripherals.PIN_23, Level::Low);
    let cs = Output::new(peripherals.PIN_25, Level::High);
    let mut pio = Pio::new(peripherals.PIO0, Irqs);
    let spi = PioSpi::new(&mut pio.common, pio.sm0, pio.irq0, cs, peripherals.PIN_24, peripherals.PIN_29, peripherals.DMA_CH0);

    let stack = wifi::init_wifi(pwr, spi, spawner).await;

    let pin = Input::new(peripherals.PIN_26, Pull::None);

    let mut rx_meta = [PacketMetadata::EMPTY; 16];
    let mut rx_buffer = [0; 4096];
    let mut tx_meta = [PacketMetadata::EMPTY; 16];
    let mut tx_buffer = [0; 4096];

    let mut socket = UdpSocket::new(stack, &mut rx_meta, &mut rx_buffer, &mut tx_meta, &mut tx_buffer);
    socket.bind(8000).unwrap();

    loop {
        let buf : [u8; 1] = [if pin.is_low() {1} else {0}];

        _ = socket.send_to(&buf, REMOTE_ENDPOINT).await;

        Timer::after(Duration::from_millis(10)).await;
    }
}

#[inline(never)]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    // Watchdog timer won't be updated, causing a reset after some time
    loop {}
}