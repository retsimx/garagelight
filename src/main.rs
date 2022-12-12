#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]
#![feature(async_fn_in_trait)]
#![allow(incomplete_features)]

mod wifi;

use cyw43::NetDevice;
use embassy_executor::Spawner;
use embassy_net::tcp::client::{TcpClient, TcpClientState, TcpConnection};
use embassy_net::tcp::Error;
use embassy_rp::gpio::{Input, Pull};
use embassy_time::{Timer, Duration};
use {defmt_rtt as _, panic_probe as _};
use crate::wifi::init_wifi;
use embassy_rp::peripherals::PIN_26;
use embedded_io::asynch::Write;
use embedded_nal_async::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpConnect};

static DEBOUNCE_MILLIS : u64 = 30;
static mut PIN: Option<Input<PIN_26>> = Option::None;
static mut CLIENT: Option<TcpClient<NetDevice, 1, 1024, 1024>> = Option::None;
static REMOTE_ENDPOINT: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(10, 0, 20, 60), 8000));

async fn update_light(state: bool, conn: &mut TcpConnection<'static, 1, 1024, 1024>) -> Result<usize, Error> {
    let mut buf : [u8; 1] = [0];
    if state {
        buf[0] = 1;
    }

    return conn.write(&buf).await;
}

#[embassy_executor::task]
async fn timer_check_task() -> ! {
    unsafe {
        loop {
            let mut conn = match CLIENT.as_mut().unwrap().connect(REMOTE_ENDPOINT).await {
                Ok(conn) => conn,
                Err(_) => {
                    Timer::after(Duration::from_secs(1)).await;
                    continue;
                }
            };
            loop {
                match update_light(true, &mut conn).await {
                    Ok(_) => {}
                    Err(_) => break
                }
                Timer::after(Duration::from_secs(1)).await;

                match update_light(false, &mut conn).await {
                    Ok(_) => {}
                    Err(_) => break
                }
                Timer::after(Duration::from_secs(1)).await;
            }
        }
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let peripherals = embassy_rp::init(Default::default());

    let stack = init_wifi(spawner).await;

    while stack.config().is_none() {
        Timer::after(Duration::from_secs(1)).await;
    }

    unsafe {
        PIN.replace(Input::new(peripherals.PIN_26, Pull::None));
    }

    static STATE: TcpClientState<1, 1024, 1024> = TcpClientState::new();
    unsafe { CLIENT.replace(TcpClient::new(&stack, &STATE)); }

    // spawner.spawn(timer_check_task()).unwrap();

    unsafe {
        loop {
            let mut conn = match CLIENT.as_mut().unwrap().connect(REMOTE_ENDPOINT).await {
                Ok(conn) => conn,
                Err(_) => {
                    Timer::after(Duration::from_secs(1)).await;
                    continue;
                }
            };
            loop {
                PIN.as_mut().unwrap().wait_for_low().await;
                Timer::after(Duration::from_millis(DEBOUNCE_MILLIS)).await;
                if PIN.as_mut().unwrap().is_high() {
                    continue;
                }

                match update_light(true, &mut conn).await {
                    Ok(_) => {}
                    Err(_) => break
                }

                PIN.as_mut().unwrap().wait_for_high().await;
                Timer::after(Duration::from_millis(DEBOUNCE_MILLIS)).await;
                if PIN.as_mut().unwrap().is_low() {
                    continue;
                }

                match update_light(false, &mut conn).await {
                    Ok(_) => {}
                    Err(_) => break
                }
            }
        }
    }
}
