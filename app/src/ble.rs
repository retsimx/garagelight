//! BLE peripheral bring-up and the beam-fact handoff to the control path.
use core::sync::atomic::{AtomicBool, Ordering};

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use static_cell::StaticCell;
use trouble_host::att::{AttClient, AttReq};
use trouble_host::prelude::*;

use crate::gatt::Server;
use crate::lamp::LampEvent;
use crate::logln;
use crate::radio::BleController;

const CONNECTIONS_MAX: usize = 1;
const L2CAP_CHANNELS_MAX: usize = 3;

static RESOURCES: StaticCell<
    HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX>,
> = StaticCell::new();
static STACK: StaticCell<Stack<'static, BleController, DefaultPacketPool>> = StaticCell::new();

/// Latched "the peripheral has started advertising at least once" (GL-11).
static ADVERTISING: AtomicBool = AtomicBool::new(false);

/// Whether the BLE peripheral has started advertising at least once.
pub fn advertising() -> bool {
    ADVERTISING.load(Ordering::Relaxed)
}

/// Build the BLE stack and start the peripheral. Call before WiFi/OTA work.
pub fn spawn(spawner: Spawner, controller: BleController) {
    let resources = RESOURCES.init(HostResources::new());
    let stack: &'static Stack<'static, BleController, DefaultPacketPool> =
        STACK.init(trouble_host::new(controller, resources).build());
    spawner.spawn(host_runner_task(stack).unwrap());
    spawner.spawn(peripheral_task(stack).unwrap());
    logln!("ble_started");
}

#[embassy_executor::task]
async fn host_runner_task(stack: &'static Stack<'static, BleController, DefaultPacketPool>) {
    let mut runner = stack.runner();
    loop {
        if runner.run().await.is_err() {
            logln!("ble_runner_err");
            Timer::after(Duration::from_millis(1000)).await;
        }
    }
}

#[embassy_executor::task]
async fn peripheral_task(stack: &'static Stack<'static, BleController, DefaultPacketPool>) {
    let server: Server<'static> =
        match Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
            name: "glgt",
            appearance: &appearance::power_device::GENERIC_POWER_DEVICE,
        })) {
            Ok(server) => server,
            Err(_) => {
                logln!("ble_server_err");
                return;
            }
        };

    let params = AdvertisementParameters {
        interval_min: Duration::from_millis(30),
        interval_max: Duration::from_millis(30),
        ..Default::default()
    };

    let mut adv = [0u8; 31];
    let adv_len = match AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::CompleteServiceUuids128(&[garagelight_core::SERVICE_UUID_BYTES]),
        ],
        &mut adv[..],
    ) {
        Ok(len) => len,
        Err(_) => {
            logln!("ble_adv_encode_err");
            return;
        }
    };

    let mut scan = [0u8; 31];
    let scan_len = match AdStructure::encode_slice(
        &[AdStructure::CompleteLocalName(b"glgt")],
        &mut scan[..],
    ) {
        Ok(len) => len,
        Err(_) => {
            logln!("ble_scan_encode_err");
            return;
        }
    };

    let advertisement = Advertisement::ConnectableScannableUndirected {
        adv_data: &adv[..adv_len],
        scan_data: &scan[..scan_len],
    };

    let mut peripheral = stack.peripheral();

    loop {
        let acceptor = match peripheral.advertise(&params, advertisement).await {
            Ok(acceptor) => {
                ADVERTISING.store(true, Ordering::Relaxed);
                logln!("ble_advertising");
                acceptor
            }
            Err(_) => {
                logln!("ble_advertise_err");
                Timer::after(Duration::from_millis(500)).await;
                continue;
            }
        };

        let conn = match acceptor.accept().await {
            Ok(conn) => conn,
            Err(_) => {
                logln!("ble_advertise_stopped");
                crate::lamp::post(LampEvent::LinkDown);
                continue;
            }
        };

        let conn = match conn.with_attribute_server(&server) {
            Ok(conn) => {
                logln!("ble_connected");
                conn
            }
            Err(_) => {
                logln!("ble_gatt_err");
                crate::lamp::post(LampEvent::LinkDown);
                continue;
            }
        };

        loop {
            match conn.next().await {
                GattConnectionEvent::Disconnected { .. } => {
                    logln!("ble_disconnected");
                    crate::lamp::post(LampEvent::LinkDown);
                    break;
                }
                GattConnectionEvent::Gatt {
                    event: GattEvent::Write(event),
                } => {
                    // The characteristic advertises `write_without_response` only,
                    // so an ATT Write Request (which expects a response) is not a
                    // permitted procedure. Reject it explicitly; a Write Command
                    // (`AttClient::Command`) falls through untouched.
                    if let AttClient::Request(AttReq::Write { .. }) = event.payload().incoming() {
                        match event.reject(AttErrorCode::WRITE_NOT_PERMITTED) {
                            Ok(_) => logln!("ble_write_req_rejected"),
                            Err(_) => logln!("ble_write_req_reject_err"),
                        }
                        continue;
                    }
                    let (len, first, parsed) = event.with_data(|_offset, data| {
                        let len = data.len();
                        let first = data.first().copied();
                        let parsed = if len == garagelight_core::VALUE_LEN_BYTES {
                            garagelight_core::validate_write(len, data[0])
                        } else {
                            None
                        };
                        (len, first, parsed)
                    });
                    match parsed {
                        Some(fact) => match event.accept() {
                            Ok(_) => {
                                crate::lamp::post_fact(fact);
                                logln!("ble_fact value={}", first.unwrap_or(0));
                            }
                            Err(_) => logln!("ble_write_apply_err"),
                        },
                        None => {
                            logln!("ble_write_ignored len={} value={:?}", len, first);
                            let _ = event.accept_unprocessed();
                        }
                    }
                }
                // No application action needed for other events. In particular,
                // `RequestConnectionParams` is central-only in trouble-host 0.8.0:
                // it is posted only from the HCI `LeRemoteConnectionParameterRequest`
                // event (host.rs:1583) and the L2CAP `CONN_PARAM_UPDATE_REQ` path,
                // which rejects non-centrals (channel_manager.rs:679). Reads
                // auto-respond when the event is dropped.
                _ => {}
            }
        }
    }
}
