//! WiFi station association, DHCP, and reconnect supervision (GL-7).
//!
//! WiFi is strictly best-effort: the control path (radio -> BLE -> lamp) is
//! brought up in `main` before [`spawn`] is called. The supervisor loops
//! forever, re-associating with exponential backoff after any join failure or
//! link loss. The one deliberate exception to "network loss never resets" is a
//! bounded no-IP escalation: after [`garagelight_core::wifi::NO_IP_RESET`]
//! without an IP lease the device resets itself to recover a wedged radio (it
//! cannot reach the network in that state anyway).
//!
//! [`spawn`] returns the `Copy` network handle so consumers (GL-8 MQTT, GL-10
//! OTA) receive it by value from `main` and pass a copy to their own tasks.
//! Task arguments do not need to be `Send` (see `app/src/ble.rs`, which passes a
//! `&'static Stack` to tasks), and `embassy_net::Stack` is a `Copy` handle best
//! passed by value rather than forced into a global behind a manual send marker.
//! No global is needed.

use core::sync::atomic::{AtomicBool, Ordering};

use cyw43::{JoinOptions, NetDriver};
use embassy_executor::Spawner;
use embassy_net::{Config, ConfigV4, DhcpConfig, Runner, Stack, StackResources};
use embassy_time::{Duration, Timer};
use static_cell::StaticCell;

use garagelight_core::wifi::{NoIpWatchdog, JOIN_TIMEOUT};

use crate::beacon::SharedControl;
use crate::{logln, secrets};

/// DHCP acquisition deadline before the supervisor leaves and retries.
const DHCP_TIMEOUT: Duration = Duration::from_secs(15);

/// In-place DHCP re-issue attempts before falling back to a full leave + rejoin.
const DHCP_RETRY_ATTEMPTS: u32 = 3;

/// Reconnect backoff: 1s, doubling to a 60s ceiling, reset on every association.
const BACKOFF_INITIAL_SECS: u64 = 30;
const BACKOFF_MAX_SECS: u64 = 120;

const NET_SEED: u64 = 0x1234_5678_9abc_def0;

/// Set after DHCP config-up, cleared before a join and on link loss, so it
/// reflects "currently associated and configured" (GL-11).
static ASSOCIATED: AtomicBool = AtomicBool::new(false);

/// Set by the telemetry task after repeated MQTT connect failures, to force an
/// immediate Wi-Fi rejoin (dead-link recovery that link state alone never sees).
static REJOIN_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Whether the station is currently associated with a DHCP-configured address.
pub fn associated() -> bool {
    ASSOCIATED.load(Ordering::Relaxed)
}

/// Request a Wi-Fi rejoin (called by the telemetry task after repeated MQTT
/// connect failures).
pub fn request_rejoin() {
    REJOIN_REQUESTED.store(true, Ordering::Relaxed);
}

/// Whether a Wi-Fi rejoin has been requested.
pub fn rejoin_requested() -> bool {
    REJOIN_REQUESTED.load(Ordering::Relaxed)
}

fn clear_rejoin_requested() {
    REJOIN_REQUESTED.store(false, Ordering::Relaxed);
}

/// Build the stack, spawn the runner and the supervisor, and return the `Copy`
/// network handle for consumers (MQTT/OTA) to pass to their own tasks. Takes a
/// shared handle to the cyw43 control channel so the join future cannot block
/// the control path and other tasks (the OTA LED beacon) can share it.
pub fn spawn(
    spawner: Spawner,
    control: &'static SharedControl,
    net_device: NetDriver<'static>,
) -> Stack<'static> {
    // Four concurrent sockets: embassy-net's permanent DNS socket, the DHCP
    // lease, GL-8's long-lived telemetry TCP, and GL-10's OTA TCP during a check.
    static RESOURCES: StaticCell<StackResources<4>> = StaticCell::new();
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
async fn supervisor_task(control: &'static SharedControl, stack: Stack<'static>) -> ! {
    let mut backoff_secs = BACKOFF_INITIAL_SECS;
    let mut no_ip = NoIpWatchdog::new();
    loop {
        // A dead-link rejoin request (from the telemetry task) forces an
        // immediate rejoin, skipping the backoff wait.
        let skip_wait = if rejoin_requested() {
            clear_rejoin_requested();
            logln!("wifi_rejoin_requested");
            true
        } else {
            false
        };

        ASSOCIATED.store(false, Ordering::Relaxed);
        logln!("wifi_associating ssid={}", secrets::WIFI_SSID);
        let joined = {
            // Hold the lock only while driving the control channel; the long
            // link/DHCP waits run with the lock released so the OTA LED beacon
            // can share the channel.
            let mut c = control.lock().await;
            match embassy_time::with_timeout(
                Duration::from_secs(JOIN_TIMEOUT.as_secs()),
                c.join(
                    secrets::WIFI_SSID,
                    JoinOptions::new(secrets::WIFI_PASSWORD.as_bytes()),
                ),
            )
            .await
            {
                Ok(Ok(())) => true,
                Ok(Err(e)) => {
                    logln!("wifi_join_failed err={:?}", e);
                    false
                }
                Err(_) => {
                    logln!("wifi_join_timeout");
                    false
                }
            }
        };
        if joined {
            logln!("wifi_associated ssid={}", secrets::WIFI_SSID);
            backoff_secs = BACKOFF_INITIAL_SECS;
            let mut dhcp_up = false;
            if embassy_time::with_timeout(DHCP_TIMEOUT, stack.wait_config_up())
                .await
                .is_ok()
            {
                dhcp_up = true;
            } else {
                logln!("wifi_dhcp_timeout");
                // Keep the association and re-issue DHCP in place, bounded,
                // before falling back to a full leave + rejoin.
                for _ in 0..DHCP_RETRY_ATTEMPTS {
                    stack.set_config_v4(ConfigV4::Dhcp(DhcpConfig::default()));
                    logln!("wifi_dhcp_retry");
                    if embassy_time::with_timeout(DHCP_TIMEOUT, stack.wait_config_up())
                        .await
                        .is_ok()
                    {
                        dhcp_up = true;
                        break;
                    }
                }
            }
            if dhcp_up {
                if let Some(cfg) = stack.config_v4() {
                    ASSOCIATED.store(true, Ordering::Relaxed);
                    no_ip.reset();
                    let o = cfg.address.address().octets();
                    logln!("wifi_dhcp_up ip={}.{}.{}.{}", o[0], o[1], o[2], o[3]);
                }
                stack.wait_link_down().await;
                ASSOCIATED.store(false, Ordering::Relaxed);
                logln!("wifi_lost");
            } else {
                logln!("wifi_dhcp_failed");
                let mut c = control.lock().await;
                match embassy_time::with_timeout(
                    Duration::from_secs(JOIN_TIMEOUT.as_secs()),
                    c.leave(),
                )
                .await
                {
                    Ok(()) => logln!("wifi_left"),
                    Err(_) => logln!("wifi_leave_timeout"),
                }
            }
        }
        if skip_wait {
            // Forced rejoin: no backoff wait this iteration.
        } else {
            logln!("wifi_retry_in_s={}", backoff_secs);
            Timer::after(Duration::from_secs(backoff_secs)).await;
            no_ip.record(core::time::Duration::from_secs(backoff_secs));
        }
        if no_ip.is_reset_due() {
            logln!("wifi_no_ip_reset");
            no_ip.reset();
            cortex_m::peripheral::SCB::sys_reset();
        }
        backoff_secs = core::cmp::min(backoff_secs * 2, BACKOFF_MAX_SECS);
    }
}
