#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]
#![feature(async_fn_in_trait)]
#![allow(incomplete_features)]

mod wifi;

use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_rp::gpio::{Level, Output};
use embassy_time::{Duration, Timer};
use {defmt_rtt as _, panic_probe as _};
use crate::wifi::init_wifi;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let peripherals = embassy_rp::init(Default::default());

    let mut led = Output::new(peripherals.PIN_28 , Level::High);

    let stack = init_wifi(spawner).await;

    while stack.config().is_none() {
        Timer::after(Duration::from_secs(1)).await;
    }

    led.set_low();

    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];
    let mut buf = [0; 4096];

    loop {
        Timer::after(Duration::from_secs(1)).await;

        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(embassy_net::SmolDuration::from_secs(10)));

        if let Err(_) = socket.accept(8000).await {
            continue;
        }

        loop {
            match socket.read(&mut buf).await {
                Ok(0) => {
                    break;
                }
                Ok(n) => n,
                Err(_) => {
                    break;
                }
            };

            if buf[0] == 1 {
                led.set_high()
            }
            else {
                led.set_low()
            }
        }
    }
}
