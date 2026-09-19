use std::collections::HashMap;

use serde::Deserialize;

use crate::budget::{fits, ACTIVE_FLASH_BYTES, BLOB_TOTAL_BYTES, VERSION};
use crate::contract::{
    validate_write, BeamFact, CHARACTERISTIC_UUID, CHARACTERISTIC_UUID_BYTES, CONN_INTERVAL_US,
    CONN_SLAVE_LATENCY, CONN_SUPERVISION_TIMEOUT_MS, FACT_BROKEN, FACT_INTACT, INITIAL_READ_VALUE,
    KEEPALIVE_INTERVAL_MS, LEASH_TIMEOUT_MS, MQTT_TOPIC, SERVICE_UUID, SERVICE_UUID_BYTES,
    VALUE_LEN_BYTES,
};
use crate::lamp::{
    fault_level, LampEvent, LampMachine, FAULT_CYCLE_MS, FAULT_OFF1_MS, FAULT_OFF2_MS,
    FAULT_ON1_MS, FAULT_ON2_MS,
};
use crate::layout::{
    ACTIVE_BASE, ACTIVE_BYTES, BOOTLOADER_BASE, BOOTLOADER_BYTES, DFU_BASE, DFU_BYTES, FLASH_BYTES,
    FLASH_END, PAGE_BYTES, SPARE_BASE, SPARE_BYTES, STATE_BASE, STATE_BYTES, WRITE_BYTES,
};
use crate::sensor::{
    in_range, read_with_retries, Attempt, Sample, MAX_ATTEMPTS, READ_STALL_US, RETRY_SETTLE_MS,
    SAMPLE_INTERVAL_SECS,
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

fn uuid_string_from_le_bytes(bytes: [u8; 16]) -> String {
    let be: Vec<u8> = bytes.iter().rev().copied().collect();
    let hex = |range: std::ops::Range<usize>| -> String {
        be[range].iter().map(|b| format!("{b:02x}")).collect()
    };
    format!(
        "{}-{}-{}-{}-{}",
        hex(0..4),
        hex(4..6),
        hex(6..8),
        hex(8..10),
        hex(10..16)
    )
}

#[test]
fn uuid_bytes_match_strings() {
    assert_eq!(uuid_string_from_le_bytes(SERVICE_UUID_BYTES), SERVICE_UUID);
    assert_eq!(
        uuid_string_from_le_bytes(CHARACTERISTIC_UUID_BYTES),
        CHARACTERISTIC_UUID
    );
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
fn fault_pattern_edges() {
    assert!(fault_level(0), "cycle start is lit");
    assert!(
        fault_level(FAULT_ON1_MS - 1),
        "first lit phase ends at 1500"
    );
    assert!(!fault_level(FAULT_ON1_MS), "1500 is the first dark edge");
    assert!(!fault_level(FAULT_ON1_MS + FAULT_OFF1_MS - 1));
    assert!(
        fault_level(FAULT_ON1_MS + FAULT_OFF1_MS),
        "1650 lights again"
    );
    assert!(fault_level(FAULT_ON1_MS + FAULT_OFF1_MS + FAULT_ON2_MS - 1));
    assert!(
        !fault_level(FAULT_ON1_MS + FAULT_OFF1_MS + FAULT_ON2_MS),
        "1850 starts the second dark phase"
    );
    assert!(!fault_level(FAULT_CYCLE_MS - 1));
    assert!(fault_level(FAULT_CYCLE_MS), "2000 wraps to the cycle start");
    assert_eq!(
        FAULT_ON1_MS + FAULT_OFF1_MS + FAULT_ON2_MS + FAULT_OFF2_MS,
        FAULT_CYCLE_MS
    );
}

#[test]
fn fault_pattern_is_85_percent_lit_and_blinks_once_per_second() {
    let lit = (0..FAULT_CYCLE_MS).filter(|&ms| fault_level(ms)).count() as u64;
    assert_eq!(lit * 100, FAULT_CYCLE_MS * 85, "~85% lit over one cycle");

    let window_ms = 2 * FAULT_CYCLE_MS;
    let mut falling_edges = 0u64;
    let mut prev = fault_level(0);
    for ms in 1..window_ms {
        let now = fault_level(ms);
        if prev && !now {
            falling_edges += 1;
        }
        prev = now;
    }
    assert_eq!(
        falling_edges,
        window_ms / 1000,
        "one dark edge per second (1 flash/s)"
    );
}

#[test]
fn lamp_machine_scripted_timeline() {
    let mut m = LampMachine::new();

    // Boot, before any write: the fault pattern.
    assert!(m.level(0));
    assert!(!m.level(FAULT_ON1_MS));
    assert!(m.level(FAULT_ON1_MS + FAULT_OFF1_MS));
    assert!(!m.level(FAULT_ON1_MS + FAULT_OFF1_MS + FAULT_ON2_MS));
    assert!(m.level(FAULT_CYCLE_MS));

    // 0x01 (Broken) -> lit immediately, fault cleared.
    m.apply(LampEvent::Fact(BeamFact::Broken), 2500);
    assert!(m.level(2500));
    assert!(
        m.level(2500 + FAULT_ON1_MS),
        "held lit, not the pattern's dark edge"
    );

    // 0x00 (Intact) -> dark immediately.
    m.apply(LampEvent::Fact(BeamFact::Intact), 3000);
    assert!(!m.level(3000));
    assert!(!m.level(3000 + FAULT_CYCLE_MS));

    // An invalid write (0x02) is ignored before any event reaches the machine.
    assert_eq!(validate_write(1, 0x02), None);
    assert!(!m.level(4000), "no event, no state change");

    // Link loss -> fault immediately (LinkDown models the supervision timeout).
    m.apply(LampEvent::LinkDown, 5000);
    assert!(m.level(5000));
    assert!(
        !m.level(5000 + FAULT_ON1_MS),
        "fault pattern resumes from the link-down instant"
    );

    // Reconnect + write -> fault clears and the fact drives.
    m.apply(LampEvent::Fact(BeamFact::Broken), 7000);
    assert!(m.level(7000));
    assert!(m.level(7000 + FAULT_CYCLE_MS));
}

#[test]
fn lamp_machine_leash_faults_at_exactly_30s() {
    let leash = u64::from(LEASH_TIMEOUT_MS);
    let mut m = LampMachine::new();
    m.apply(LampEvent::Fact(BeamFact::Broken), 0);

    // 29_999 ms of live silence -> held (still the fact).
    m.apply(LampEvent::Tick, leash - 1);
    assert!(m.level(leash - 1));
    assert!(
        m.level(leash - 1 + FAULT_ON1_MS),
        "held lit, not the fault pattern"
    );

    // 30_000 ms -> leash fault; the lamp now follows the pattern.
    m.apply(LampEvent::Tick, leash);
    assert!(m.level(leash));
    assert!(
        !m.level(leash + FAULT_ON1_MS),
        "the pattern's dark edge proves the fault, not a held fact"
    );
}

#[test]
fn late_tick_does_not_overwrite_fresh_fact() {
    let mut m = LampMachine::new();
    m.apply(LampEvent::Fact(BeamFact::Intact), 1234);
    m.apply(LampEvent::Tick, 1234);
    assert!(
        !m.level(1234),
        "a tick at the fact's instant keeps the fact's level"
    );
    assert!(!m.level(1234 + FAULT_CYCLE_MS));

    let mut m = LampMachine::new();
    m.apply(LampEvent::Fact(BeamFact::Broken), 1234);
    m.apply(LampEvent::Tick, 1234);
    assert!(m.level(1234));
    assert!(
        m.level(1234 + FAULT_ON1_MS),
        "held lit instead of the pattern's dark edge"
    );
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

#[test]
fn sensor_constants() {
    assert_eq!(MAX_ATTEMPTS, 3);
    assert_eq!(READ_STALL_US, 50_000);
    assert_eq!(RETRY_SETTLE_MS, 1_000);
    assert_eq!(SAMPLE_INTERVAL_SECS, 15);
}

#[test]
fn sensor_in_range_boundaries() {
    let at = |temperature: i8, relative_humidity: u8| Sample {
        temperature,
        relative_humidity,
    };

    for temperature in [-20, 0, 50, 60] {
        assert!(in_range(at(temperature, 50)), "temp {temperature}");
    }
    for temperature in [-21, 61, 127] {
        assert!(!in_range(at(temperature, 50)), "temp {temperature}");
    }
    for relative_humidity in [0, 100] {
        assert!(
            in_range(at(25, relative_humidity)),
            "rh {relative_humidity}"
        );
    }
    for relative_humidity in [101, 255] {
        assert!(
            !in_range(at(25, relative_humidity)),
            "rh {relative_humidity}"
        );
    }
}

#[test]
fn sensor_succeeds_on_third_attempt() {
    let valid = Sample {
        temperature: 21,
        relative_humidity: 45,
    };
    let script = [Attempt::Checksum, Attempt::Checksum, Attempt::Ok(valid)];
    let mut calls = 0u32;

    let result = read_with_retries(|| {
        let attempt = script[calls as usize];
        calls += 1;
        attempt
    });

    assert_eq!(result, Some(valid));
    assert_eq!(calls, 3);
}

#[test]
fn sensor_exhausts_after_max_attempts() {
    let mut calls = 0u32;

    let result = read_with_retries(|| {
        calls += 1;
        Attempt::Checksum
    });

    assert_eq!(result, None);
    assert_eq!(calls, u32::from(MAX_ATTEMPTS));
}

#[test]
fn sensor_discards_stall_and_retries() {
    let valid = Sample {
        temperature: 30,
        relative_humidity: 55,
    };
    let script = [Attempt::Stalled, Attempt::Ok(valid)];
    let mut calls = 0u32;

    let result = read_with_retries(|| {
        let attempt = script[calls as usize];
        calls += 1;
        attempt
    });

    assert_eq!(result, Some(valid));
    assert_eq!(calls, 2);

    let mut stall_calls = 0u32;
    let exhausted = read_with_retries(|| {
        stall_calls += 1;
        Attempt::Stalled
    });

    assert_eq!(exhausted, None);
    assert_eq!(stall_calls, u32::from(MAX_ATTEMPTS));
}

#[test]
fn sensor_retries_out_of_range_ok() {
    let bad = Sample {
        temperature: 61,
        relative_humidity: 50,
    };
    let good = Sample {
        temperature: 22,
        relative_humidity: 40,
    };
    let script = [Attempt::Ok(bad), Attempt::Ok(good)];
    let mut calls = 0u32;

    let result = read_with_retries(|| {
        let attempt = script[calls as usize];
        calls += 1;
        attempt
    });

    assert_eq!(result, Some(good));
    assert_eq!(calls, 2);
}

#[test]
fn sensor_negative_temperature_round_trips() {
    let valid = Sample {
        temperature: -5,
        relative_humidity: 40,
    };

    let result = read_with_retries(|| Attempt::Ok(valid));

    assert_eq!(result, Some(valid));
    assert_eq!(result.unwrap().temperature, -5);
}
