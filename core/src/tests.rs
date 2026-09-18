use serde::Deserialize;

use crate::budget::{fits, ACTIVE_FLASH_BYTES, BLOB_TOTAL_BYTES, VERSION};
use crate::contract::{
    validate_write, BeamFact, CHARACTERISTIC_UUID, CONN_INTERVAL_US, CONN_SLAVE_LATENCY,
    CONN_SUPERVISION_TIMEOUT_MS, FACT_BROKEN, FACT_INTACT, INITIAL_READ_VALUE,
    KEEPALIVE_INTERVAL_MS, LEASH_TIMEOUT_MS, MQTT_TOPIC, SERVICE_UUID, VALUE_LEN_BYTES,
};

#[derive(Deserialize)]
struct ContractFile {
    service_uuid: String,
    characteristic_uuid: String,
    value_len_bytes: u32,
    fact_intact: u8,
    fact_broken: u8,
    initial_read_value: u8,
    conn_interval_us: u32,
    conn_slave_latency: u16,
    conn_supervision_timeout_ms: u32,
    keepalive_interval_ms: u32,
    leash_timeout_ms: u32,
    mqtt_topic: String,
}

fn load_contract() -> ContractFile {
    let raw = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../contract.toml"))
        .expect("contract.toml must be readable");
    toml::from_str(&raw).expect("contract.toml must parse")
}

#[test]
fn contract_constants_match_file() {
    let c = load_contract();
    assert_eq!(SERVICE_UUID, c.service_uuid.as_str());
    assert_eq!(CHARACTERISTIC_UUID, c.characteristic_uuid.as_str());
    assert_eq!(VALUE_LEN_BYTES, c.value_len_bytes as usize);
    assert_eq!(FACT_INTACT, c.fact_intact);
    assert_eq!(FACT_BROKEN, c.fact_broken);
    assert_eq!(INITIAL_READ_VALUE, c.initial_read_value);
    assert_eq!(CONN_INTERVAL_US, c.conn_interval_us);
    assert_eq!(CONN_SLAVE_LATENCY, c.conn_slave_latency);
    assert_eq!(CONN_SUPERVISION_TIMEOUT_MS, c.conn_supervision_timeout_ms);
    assert_eq!(KEEPALIVE_INTERVAL_MS, c.keepalive_interval_ms);
    assert_eq!(LEASH_TIMEOUT_MS, c.leash_timeout_ms);
    assert_eq!(MQTT_TOPIC, c.mqtt_topic.as_str());
}

#[test]
fn value_rules() {
    assert_eq!(validate_write(1, 0x00), Some(BeamFact::Intact));
    assert_eq!(validate_write(1, 0x01), Some(BeamFact::Broken));
    assert_eq!(validate_write(1, 0x02), None);
    assert_eq!(validate_write(2, 0x00), None);
    assert_eq!(validate_write(0, 0x00), None);
}

#[test]
fn invariants_and_budget() {
    let leash = LEASH_TIMEOUT_MS;
    let keepalive = KEEPALIVE_INTERVAL_MS;
    assert!(leash > keepalive);
    assert!(fits(ACTIVE_FLASH_BYTES));
    assert!(!fits(ACTIVE_FLASH_BYTES + 1));
    assert!(fits(BLOB_TOTAL_BYTES));
    assert!(!VERSION.is_empty());
}
