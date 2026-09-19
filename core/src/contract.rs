pub const SERVICE_UUID: &str = "6a4c0001-b5a3-4f1e-9c2d-7e8f9a0b1c2d";
pub const CHARACTERISTIC_UUID: &str = "6a4c0002-b5a3-4f1e-9c2d-7e8f9a0b1c2d";
// BLE little-endian form of the pinned UUID strings; the single source used by the GATT macros and advertising data.
pub const SERVICE_UUID_BYTES: [u8; 16] = [
    0x2d, 0x1c, 0x0b, 0x9a, 0x8f, 0x7e, 0x2d, 0x9c, 0x1e, 0x4f, 0xa3, 0xb5, 0x01, 0x00, 0x4c, 0x6a,
];
pub const CHARACTERISTIC_UUID_BYTES: [u8; 16] = [
    0x2d, 0x1c, 0x0b, 0x9a, 0x8f, 0x7e, 0x2d, 0x9c, 0x1e, 0x4f, 0xa3, 0xb5, 0x02, 0x00, 0x4c, 0x6a,
];
pub const VALUE_LEN_BYTES: usize = 1;
pub const FACT_INTACT: u8 = 0x00;
pub const FACT_BROKEN: u8 = 0x01;
pub const INITIAL_READ_VALUE: u8 = 0x00;
pub const CONN_INTERVAL_US: u32 = 7_500;
pub const CONN_SLAVE_LATENCY: u16 = 0;
pub const CONN_SUPERVISION_TIMEOUT_MS: u32 = 1_000;
pub const KEEPALIVE_INTERVAL_MS: u32 = 10_000;
pub const LEASH_TIMEOUT_MS: u32 = 30_000;
pub const MQTT_TOPIC: &str = "garage/temperature";

const _: () = assert!(LEASH_TIMEOUT_MS > KEEPALIVE_INTERVAL_MS);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BeamFact {
    Intact,
    Broken,
}

pub fn validate_write(len: usize, value: u8) -> Option<BeamFact> {
    if len != VALUE_LEN_BYTES {
        return None;
    }
    match value {
        FACT_INTACT => Some(BeamFact::Intact),
        FACT_BROKEN => Some(BeamFact::Broken),
        _ => None,
    }
}
