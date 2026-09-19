//! WiFi station association, DHCP, and reconnect supervision (GL-7).
//!
//! WiFi is strictly best-effort: the control path (radio -> BLE -> lamp) is
//! brought up in `main` before [`spawn`] is called, and nothing in this module
//! ever resets the device. The supervisor loops forever, re-associating with
//! exponential backoff after any join failure or link loss.
//!
//! [`spawn`] returns the `Copy` network handle so consumers (GL-8 MQTT, GL-10
//! OTA) receive it by value from `main` and pass a copy to their own tasks.
//! Task arguments do not need to be `Send` (see `app/src/ble.rs`, which passes a
//! `&'static Stack` to tasks), and `embassy_net::Stack` is a `Copy` handle best
//! passed by value rather than forced into a global behind a manual send marker.
//! No global is needed.

use cyw43::{Control, JoinOptions, NetDriver};
use embassy_executor::Spawner;
use embassy_net::{Config, Runner, Stack, StackResources};
use embassy_time::{Duration, Timer};
use static_cell::StaticCell;

use crate::{logln, secrets};

/// DHCP acquisition deadline before the supervisor leaves and retries.
const DHCP_TIMEOUT: Duration = Duration::from_secs(15);

/// Reconnect backoff: 1s, doubling to a 60s ceiling, reset on every association.
const BACKOFF_INITIAL_SECS: u64 = 1;
const BACKOFF_MAX_SECS: u64 = 60;

const NET_SEED: u64 = 0x1234_5678_9abc_def0;

/// Build the stack, spawn the runner and the supervisor, and return the `Copy`
/// network handle for consumers (MQTT/OTA) to pass to their own tasks. Takes
/// ownership of the cyw43 control channel so the join future cannot block the
/// control path.
pub fn spawn(
    spawner: Spawner,
    control: Control<'static>,
    net_device: NetDriver<'static>,
) -> Stack<'static> {
    static RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();
    let (stack, runner) = embassy_net::new(
        net_device,
        Config::dhcpv4(Default::default()),
        RESOURCES.init(StackResources::new()),
        NET_SEED,
    );

    spawner.spawn(runner_task(runner).unwrap());
    spawner.spawn(supervisor_task(control, stack).unwrap());

    stack
}

#[embassy_executor::task]
async fn runner_task(mut runner: Runner<'static, NetDriver<'static>>) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn supervisor_task(mut control: Control<'static>, stack: Stack<'static>) -> ! {
    let mut backoff_secs = BACKOFF_INITIAL_SECS;
    loop {
        logln!("wifi_associating ssid={}", secrets::WIFI_SSID);
        match control
            .join(
                secrets::WIFI_SSID,
                JoinOptions::new(secrets::WIFI_PASSWORD.as_bytes()),
            )
            .await
        {
            Ok(()) => {
                logln!("wifi_associated ssid={}", secrets::WIFI_SSID);
                backoff_secs = BACKOFF_INITIAL_SECS;
                match embassy_time::with_timeout(DHCP_TIMEOUT, stack.wait_config_up()).await {
                    Ok(()) => {
                        if let Some(cfg) = stack.config_v4() {
                            let o = cfg.address.address().octets();
                            logln!("wifi_dhcp_up ip={}.{}.{}.{}", o[0], o[1], o[2], o[3]);
                        }
                        stack.wait_link_down().await;
                        logln!("wifi_lost");
                    }
                    Err(_) => {
                        logln!("wifi_dhcp_timeout");
                        control.leave().await;
                        logln!("wifi_left");
                    }
                }
            }
            Err(e) => logln!("wifi_join_failed err={:?}", e),
        }
        logln!("wifi_retry_in_s={}", backoff_secs);
        Timer::after(Duration::from_secs(backoff_secs)).await;
        backoff_secs = core::cmp::min(backoff_secs * 2, BACKOFF_MAX_SECS);
    }
}
