#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]
#![feature(async_fn_in_trait)]
#![allow(incomplete_features)]

mod wifi;

use embassy_executor::Spawner;
use embassy_net::{PacketMetadata};
use embassy_net::udp::UdpSocket;
use embassy_rp::gpio::{Level, Output};
use {defmt_rtt as _, panic_probe as _};
use crate::wifi::init_wifi;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let peripherals = embassy_rp::init(Default::default());

    let stack = init_wifi(spawner).await;

    let mut rx_meta = [PacketMetadata::EMPTY; 16];
    let mut rx_buffer = [0; 4096];
    let mut tx_meta = [PacketMetadata::EMPTY; 16];
    let mut tx_buffer = [0; 4096];
    let mut buf = [0; 4096];

    let mut socket = UdpSocket::new(stack, &mut rx_meta, &mut rx_buffer, &mut tx_meta, &mut tx_buffer);
    socket.bind(8000).unwrap();

    let mut led = Output::new(peripherals.PIN_28 , Level::High);

    loop {
        let (_n, _ep) = socket.recv_from(&mut buf).await.unwrap();

        if buf[0] == 1 {
            led.set_high()
        }
        else {
            led.set_low()
        }
    }
}
