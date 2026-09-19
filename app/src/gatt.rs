//! GATT server for the garage beam-fact contract.
use trouble_host::prelude::*;

#[gatt_server]
pub struct Server<'values> {
    pub beam: BeamService,
}

#[gatt_service(uuid = garagelight_core::SERVICE_UUID_BYTES)]
pub struct BeamService {
    #[characteristic(
        uuid = garagelight_core::CHARACTERISTIC_UUID_BYTES,
        value = garagelight_core::INITIAL_READ_VALUE,
        read,
        write_without_response
    )]
    pub fact: u8,
}
