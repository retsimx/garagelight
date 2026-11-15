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

use cyw43::{JoinAuth, JoinOptions, NetDriver};
use embassy_executor::Spawner;
use embassy_futures::select::{select, Either};
use embassy_net::{Config, ConfigV4, DhcpConfig, Runner, Stack, StackResources};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer};
use static_cell::StaticCell;

use garagelight_core::wifi::{NoIpWatchdog, JOIN_TIMEOUT};
use garagelight_core::RebootReason;

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

/// Signaled by the telemetry task after repeated MQTT connect failures, to
/// wake the supervisor out of `wait_link_down()` and force an immediate Wi-Fi
/// rejoin (dead-link recovery that link state alone never sees).
static REJOIN_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Whether the station is currently associated with a DHCP-configured address.
pub fn associated() -> bool {
    ASSOCIATED.load(Ordering::Relaxed)
}

/// Request a Wi-Fi rejoin (called by the telemetry task after repeated MQTT
/// connect failures).
pub fn request_rejoin() {
    REJOIN_SIGNAL.signal(());
}

/// Whether a Wi-Fi rejoin has been requested.
pub fn rejoin_requested() -> bool {
    REJOIN_SIGNAL.signaled()
}

/// Why one join attempt did not succeed.
enum JoinAttemptError {
    /// The join call did not complete within [`JOIN_TIMEOUT`].
    Timeout,
    /// The radio reported a join failure.
    Join(cyw43::JoinError),
}

/// One bounded join attempt with an explicit authentication mode.
///
/// `auth` only applies to secured networks: an empty password selects
/// [`JoinOptions::new_open`], which the driver joins as `JoinAuth::Open`. The
/// control lock is held only for the call itself, so the OTA LED beacon can
/// share the channel while the link and DHCP settle.
async fn join_with(
    control: &'static SharedControl,
    auth: JoinAuth,
) -> Result<(), JoinAttemptError> {
    let options = if secrets::WIFI_PASSWORD.is_empty() {
        JoinOptions::new_open()
    } else {
        let mut options = JoinOptions::new(secrets::WIFI_PASSWORD.as_bytes());
        options.auth = auth;
        options
    };
    match embassy_time::with_timeout(Duration::from_secs(JOIN_TIMEOUT.as_secs()), async {
        let mut c = control.lock().await;
        c.join(secrets::WIFI_SSID, options).await
    })
    .await
    {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(JoinAttemptError::Join(e)),
        Err(_) => Err(JoinAttemptError::Timeout),
    }
}

/// Bounded `control.leave()` call; lock acquisition is also covered by
/// [`JOIN_TIMEOUT`] so a contended control channel cannot stall recovery.
async fn leave_bounded(control: &'static SharedControl) {
    match embassy_time::with_timeout(Duration::from_secs(JOIN_TIMEOUT.as_secs()), async {
        let mut c = control.lock().await;
        c.leave().await;
    })
    .await
    {
        Ok(()) => logln!("wifi_left"),
        Err(_) => logln!("wifi_leave_timeout"),
    }
}

fn record_and_check_no_ip(no_ip: &mut NoIpWatchdog, since: Instant) {
    no_ip.record(core::time::Duration::from_millis(
        since.elapsed().as_millis(),
    ));
    if no_ip.is_reset_due() {
        logln!("wifi_no_ip_reset");
        no_ip.reset();
        crate::update::reboot(RebootReason::NoIp);
    }
}

/// Join the configured network, preferring WPA2-PSK over the driver default.
///
/// [`JoinOptions::new`] defaults to `JoinAuth::Wpa2Wpa3`, which sets `AUTH_SAE`.
/// The CYW43439 firmware drops a WPA3 (SAE) association every ~11 hours and
/// then re-associates on its own, so every drop costs a rejoin; see
/// `georgerobotics/cyw43-driver#161` and the Infineon report it cites. Trying
/// `Wpa2` first keeps an AP that still offers WPA2-PSK off the SAE path.
///
/// The WPA2/WPA3 default is retried once if WPA2 fails (error or timeout) and a
/// password is configured, so an AP that no longer advertises WPA2-PSK (WPA3
/// only) still associates, at the cost of one extra failed attempt per
/// reconnect on such a network. Open networks are unaffected: they join with
/// [`JoinOptions::new_open`] and get no fallback attempt.
async fn attempt_join(control: &'static SharedControl) -> Result<(), JoinAttemptError> {
    let wpa2 = join_with(control, JoinAuth::Wpa2).await;
    if wpa2.is_ok() || secrets::WIFI_PASSWORD.is_empty() {
        return wpa2;
    }

    logln!("wifi_wpa2_join_failed retrying=wpa2wpa3");
    // No `leave()` here: `Control::join()` re-issues the full security
    // configuration, the timed-out attempt's future and lock guard are dropped
    // together, and the supervisor leaves on its next iteration.
    join_with(control, JoinAuth::Wpa2Wpa3).await
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

async fn acquire_dhcp(stack: Stack<'static>) -> bool {
    if embassy_time::with_timeout(DHCP_TIMEOUT, stack.wait_config_up())
        .await
        .is_ok()
    {
        return true;
    }
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
            return true;
        }
    }
    false
}

#[embassy_executor::task]
async fn supervisor_task(control: &'static SharedControl, stack: Stack<'static>) -> ! {
    let mut backoff_secs = BACKOFF_INITIAL_SECS;
    let mut no_ip = NoIpWatchdog::new();
    loop {
        let mut unassociated_start = Instant::now();
        let mut skip_wait = false;

        ASSOCIATED.store(false, Ordering::Relaxed);
        logln!("wifi_associating ssid={}", secrets::WIFI_SSID);
        let joined = {
            // The join helper takes the control lock only for the join call
            // itself, so the OTA LED beacon can share the channel.
            match attempt_join(control).await {
                Ok(()) => true,
                Err(JoinAttemptError::Join(e)) => {
                    logln!("wifi_join_failed err={:?}", e);
                    false
                }
                Err(JoinAttemptError::Timeout) => {
                    logln!("wifi_join_timeout");
                    false
                }
            }
        };
        if joined {
            logln!("wifi_associated ssid={}", secrets::WIFI_SSID);
            backoff_secs = BACKOFF_INITIAL_SECS;
            if acquire_dhcp(stack).await {
                // Discard any stale rejoin signal posted before this link
                // finished associating so it cannot immediately tear down a
                // fresh connection.
                REJOIN_SIGNAL.reset();
                if let Some(cfg) = stack.config_v4() {
                    ASSOCIATED.store(true, Ordering::Relaxed);
                    no_ip.reset();
                    let o = cfg.address.address().octets();
                    logln!("wifi_dhcp_up ip={}.{}.{}.{}", o[0], o[1], o[2], o[3]);
                }
                let outcome = select(stack.wait_link_down(), REJOIN_SIGNAL.wait()).await;
                ASSOCIATED.store(false, Ordering::Relaxed);
                unassociated_start = Instant::now();
                match outcome {
                    Either::First(()) => logln!("wifi_lost"),
                    Either::Second(()) => {
                        logln!("wifi_rejoin_requested");
                        leave_bounded(control).await;
                        stack.set_config_v4(ConfigV4::Dhcp(DhcpConfig::default()));
                        backoff_secs = BACKOFF_INITIAL_SECS;
                        skip_wait = true;
                    }
                }
            } else {
                logln!("wifi_dhcp_failed");
                leave_bounded(control).await;
                stack.set_config_v4(ConfigV4::Dhcp(DhcpConfig::default()));
            }
        }
        record_and_check_no_ip(&mut no_ip, unassociated_start);
        if !skip_wait {
            logln!("wifi_retry_in_s={}", backoff_secs);
            let wait_start = Instant::now();
            Timer::after(Duration::from_secs(backoff_secs)).await;
            record_and_check_no_ip(&mut no_ip, wait_start);
            backoff_secs = core::cmp::min(backoff_secs * 2, BACKOFF_MAX_SECS);
        }
    }
}
