#![cfg_attr(not(test), no_std)]

pub mod budget;
pub mod contract;

#[cfg(test)]
mod tests;

pub use budget::{fits, ACTIVE_BASE, ACTIVE_FLASH_BYTES, BLOB_TOTAL_BYTES, VERSION};
pub use contract::{
    validate_write, BeamFact, CHARACTERISTIC_UUID, CONN_INTERVAL_US, CONN_SLAVE_LATENCY,
    CONN_SUPERVISION_TIMEOUT_MS, FACT_BROKEN, FACT_INTACT, INITIAL_READ_VALUE,
    KEEPALIVE_INTERVAL_MS, LEASH_TIMEOUT_MS, MQTT_TOPIC, SERVICE_UUID, VALUE_LEN_BYTES,
};
