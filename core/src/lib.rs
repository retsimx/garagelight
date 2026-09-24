#![cfg_attr(not(test), no_std)]

pub mod budget;
pub mod contract;
pub mod lamp;
pub mod layout;
pub mod ota;
pub mod sensor;
pub mod telemetry;
pub mod wifi;

#[cfg(test)]
mod tests;

pub use budget::{fits, ACTIVE_BASE, ACTIVE_FLASH_BYTES, BLOB_TOTAL_BYTES, VERSION};
pub use contract::{
    validate_write, BeamFact, CHARACTERISTIC_UUID, CHARACTERISTIC_UUID_BYTES, CONN_INTERVAL_US,
    CONN_SLAVE_LATENCY, CONN_SUPERVISION_TIMEOUT_MS, FACT_BROKEN, FACT_INTACT, INITIAL_READ_VALUE,
    KEEPALIVE_INTERVAL_MS, LEASH_TIMEOUT_MS, MQTT_TOPIC, SERVICE_UUID, SERVICE_UUID_BYTES,
    VALUE_LEN_BYTES,
};
pub use lamp::{
    fault_level, LampEvent, LampMachine, FAULT_CYCLE_MS, FAULT_OFF1_MS, FAULT_OFF2_MS,
    FAULT_ON1_MS, FAULT_ON2_MS, LAMP_ON_LEVEL,
};
pub use layout::{
    ACTIVE_BYTES, BOOTLOADER_BASE, BOOTLOADER_BYTES, DFU_BASE, DFU_BYTES, FLASH_BASE, FLASH_BYTES,
    FLASH_END, PAGE_BYTES, SPARE_BASE, SPARE_BYTES, STATE_BASE, STATE_BYTES, WRITE_BYTES,
};
pub use ota::{
    apply_update, base64, basic_authorization, decide, parse_sha256_hex, parse_url, parse_version,
    self_test, BodyReader, Clock, Decision, Flasher, HeadError, HeadEvent, HeadParser, Probe,
    Report, ResponseHead, Signals, UpdateError, Uri, UrlError, Verdict, CHUNK_BYTES,
    SELF_TEST_POLL_MS, SELF_TEST_WINDOW_MS,
};
pub use sensor::{
    in_range, read_with_retries, Attempt, Sample, MAX_ATTEMPTS, READ_STALL_US, RETRY_SETTLE_MS,
    SAMPLE_INTERVAL_SECS,
};
pub use telemetry::{
    classify_inbound, encode_sample, parse_broker, Inbound, BROKER_PORT, CLIENT_ID, KEEPALIVE_SECS,
    MAX_SAMPLE_PAYLOAD, RESET_TOPIC,
};
pub use wifi::{NoIpWatchdog, RejoinCounter, JOIN_TIMEOUT, NO_IP_RESET, REJOIN_AFTER_FAILURES};
