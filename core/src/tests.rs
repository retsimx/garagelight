use std::collections::HashMap;

use serde::Deserialize;

use crate::budget::{fits, ACTIVE_FLASH_BYTES, BLOB_TOTAL_BYTES, VERSION};
use crate::contract::{
    validate_write, BeamFact, CHARACTERISTIC_UUID, CONN_INTERVAL_US, CONN_SLAVE_LATENCY,
    CONN_SUPERVISION_TIMEOUT_MS, FACT_BROKEN, FACT_INTACT, INITIAL_READ_VALUE,
    KEEPALIVE_INTERVAL_MS, LEASH_TIMEOUT_MS, MQTT_TOPIC, SERVICE_UUID, VALUE_LEN_BYTES,
};
use crate::layout::{
    ACTIVE_BASE, ACTIVE_BYTES, BOOTLOADER_BASE, BOOTLOADER_BYTES, DFU_BASE, DFU_BYTES, FLASH_BYTES,
    FLASH_END, PAGE_BYTES, SPARE_BASE, SPARE_BYTES, STATE_BASE, STATE_BYTES, WRITE_BYTES,
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

#[test]
fn layout_const_invariants() {
    assert_eq!(BOOTLOADER_BASE + BOOTLOADER_BYTES, ACTIVE_BASE);
    assert_eq!(ACTIVE_BASE + ACTIVE_BYTES, DFU_BASE);
    assert_eq!(DFU_BYTES, ACTIVE_BYTES + PAGE_BYTES);
    assert_eq!(DFU_BASE + DFU_BYTES, STATE_BASE);
    assert_eq!(STATE_BASE + STATE_BYTES, SPARE_BASE);
    assert_eq!(SPARE_BASE + SPARE_BYTES, FLASH_END);

    for base in [
        BOOTLOADER_BASE,
        ACTIVE_BASE,
        DFU_BASE,
        STATE_BASE,
        SPARE_BASE,
    ] {
        assert!(
            base.is_multiple_of(PAGE_BYTES),
            "base {base:#x} is not page aligned"
        );
    }
    for len in [ACTIVE_BYTES, DFU_BYTES, STATE_BYTES, SPARE_BYTES] {
        assert!(
            len.is_multiple_of(PAGE_BYTES),
            "length {len:#x} is not page aligned"
        );
    }

    const {
        assert!(2 + 4 * (ACTIVE_BYTES / PAGE_BYTES) <= STATE_BYTES / WRITE_BYTES);
        assert!(SPARE_BASE + SPARE_BYTES <= FLASH_END);
    }
    assert_eq!(FLASH_BYTES, 2 * 1024 * 1024);
}

#[test]
fn layout_matches_budget() {
    assert_eq!(ACTIVE_BASE, crate::budget::ACTIVE_BASE);
    assert_eq!(ACTIVE_BYTES, ACTIVE_FLASH_BYTES);
}

#[test]
fn layout_matches_bootloader_memory_x() {
    let r = load_memory_x("bootloader/memory.x");
    assert_eq!(r["BOOT2"].origin, BOOTLOADER_BASE);
    assert_eq!(r["BOOT2"].length, 0x100);
    assert_eq!(r["FLASH"].origin, BOOTLOADER_BASE + 0x100);
    assert_eq!(r["FLASH"].origin + r["FLASH"].length, ACTIVE_BASE);
    assert_eq!(r["ACTIVE"].origin, ACTIVE_BASE);
    assert_eq!(r["ACTIVE"].length, ACTIVE_BYTES);
    assert_eq!(r["DFU"].origin, DFU_BASE);
    assert_eq!(r["DFU"].length, DFU_BYTES);
    assert_eq!(r["STATE"].origin, STATE_BASE);
    assert_eq!(r["STATE"].length, STATE_BYTES);
}

#[test]
fn layout_matches_app_memory_x() {
    let r = load_memory_x("app/memory.x");
    assert_eq!(r["BOOT2"].origin, BOOTLOADER_BASE);
    assert_eq!(r["BOOT2"].length, 0x100);
    assert_eq!(r["FLASH"].origin, ACTIVE_BASE);
    assert_eq!(r["FLASH"].length, ACTIVE_BYTES);
    assert_eq!(r["DFU"].origin, DFU_BASE);
    assert_eq!(r["DFU"].length, DFU_BYTES);
    assert_eq!(r["STATE"].origin, STATE_BASE);
    assert_eq!(r["STATE"].length, STATE_BYTES);
}

struct Region {
    origin: u32,
    length: u32,
}

fn load_memory_x(relative: &str) -> HashMap<String, Region> {
    let path = format!("{}/../{relative}", env!("CARGO_MANIFEST_DIR"));
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    parse_memory_x(&raw)
}

fn parse_memory_x(text: &str) -> HashMap<String, Region> {
    let text = strip_comments(text);
    let body = text
        .split_once("MEMORY")
        .and_then(|(_, rest)| rest.split_once('{'))
        .and_then(|(_, rest)| rest.split_once('}'))
        .map(|(body, _)| body)
        .expect("memory.x must contain a brace-delimited MEMORY block");

    let mut regions = HashMap::new();
    for line in body.lines() {
        let Some((name, rest)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        if let Some(origin) = value_after(rest, "ORIGIN") {
            if let Some(length) = value_after(rest, "LENGTH") {
                regions.insert(name.to_string(), Region { origin, length });
            }
        }
    }
    regions
}

fn value_after(rest: &str, key: &str) -> Option<u32> {
    let (_, rest) = rest.split_once(key)?;
    let (_, rest) = rest.split_once('=')?;
    Some(eval_expr(rest.split(',').next().unwrap()))
}

fn eval_expr(expr: &str) -> u32 {
    let mut total = 0i64;
    let mut sign = 1i64;
    let mut term = String::new();
    for ch in expr.chars().chain(std::iter::once('+')) {
        match ch {
            '+' | '-' => {
                if !term.trim().is_empty() {
                    total += sign * parse_term(term.trim());
                    term.clear();
                }
                sign = if ch == '-' { -1 } else { 1 };
            }
            _ => term.push(ch),
        }
    }
    total as u32
}

fn parse_term(term: &str) -> i64 {
    let (digits, scale) = match term.chars().last() {
        Some('K' | 'k') => (&term[..term.len() - 1], 1024),
        Some('M' | 'm') => (&term[..term.len() - 1], 1024 * 1024),
        _ => (term, 1),
    };
    let value = if let Some(hex) = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        i64::from_str_radix(hex.trim(), 16).expect("hex term in memory.x")
    } else {
        digits
            .trim()
            .parse::<i64>()
            .expect("decimal term in memory.x")
    };
    value * scale
}

fn strip_comments(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            while let Some(c) = chars.next() {
                if c == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
            }
        } else if ch == '/' && chars.peek() == Some(&'/') {
            for c in chars.by_ref() {
                if c == '\n' {
                    out.push('\n');
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}
