use core::net::Ipv4Addr;
use std::collections::HashMap;

use serde::Deserialize;
use sha2::Digest;

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
use crate::ota::{
    apply_update, base64, basic_authorization, decide, parse_sha256_hex, parse_url, parse_version,
    self_test, BodyReader, Clock, Decision, Flasher, HeadError, HeadEvent, HeadParser, Probe,
    ResponseHead, Signals, UpdateError, UrlError, Verdict, CHUNK_BYTES, SELF_TEST_WINDOW_MS,
};
use crate::sensor::{
    in_range, read_with_retries, Attempt, Sample, MAX_ATTEMPTS, READ_STALL_US, RETRY_SETTLE_MS,
    SAMPLE_INTERVAL_SECS,
};
use crate::telemetry::{
    classify_inbound, encode_sample, parse_broker, Inbound, MAX_SAMPLE_PAYLOAD,
};
use crate::wifi::{NoIpWatchdog, RejoinCounter, NO_IP_RESET, REJOIN_AFTER_FAILURES};

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

#[test]
fn telemetry_encode_sample_exact_bytes() {
    let cases: [(Sample, u32, &[u8]); 5] = [
        (
            Sample {
                temperature: 25,
                relative_humidity: 60,
            },
            8,
            b"{\"temp\": 25, \"humidity\": 60, \"version\": 8}",
        ),
        (
            Sample {
                temperature: -5,
                relative_humidity: 40,
            },
            8,
            b"{\"temp\": -5, \"humidity\": 40, \"version\": 8}",
        ),
        (
            Sample {
                temperature: 25,
                relative_humidity: 0,
            },
            0,
            b"{\"temp\": 25, \"humidity\": 0, \"version\": 0}",
        ),
        (
            Sample {
                temperature: 25,
                relative_humidity: 100,
            },
            8,
            b"{\"temp\": 25, \"humidity\": 100, \"version\": 8}",
        ),
        (
            Sample {
                temperature: 0,
                relative_humidity: 0,
            },
            8,
            b"{\"temp\": 0, \"humidity\": 0, \"version\": 8}",
        ),
    ];

    for (sample, version, expected) in cases {
        let mut out = [0u8; MAX_SAMPLE_PAYLOAD];
        let len =
            encode_sample(sample, version, &mut out).expect("sample fits in MAX_SAMPLE_PAYLOAD");
        assert_eq!(len, expected.len(), "length for {sample:?}");
        assert_eq!(&out[..len], expected, "bytes for {sample:?}");
    }
}

#[test]
fn telemetry_encode_sample_exact_buffer_and_one_byte_short() {
    let sample = Sample {
        temperature: 25,
        relative_humidity: 60,
    };
    let mut scratch = [0u8; MAX_SAMPLE_PAYLOAD];
    let len = encode_sample(sample, 8, &mut scratch).expect("sample fits");

    let mut short = [0u8; MAX_SAMPLE_PAYLOAD];
    assert_eq!(encode_sample(sample, 8, &mut short[..len - 1]), None);
    assert_eq!(encode_sample(sample, 8, &mut short[..len]), Some(len));
    assert_eq!(encode_sample(sample, 8, &mut []), None);
}

#[test]
fn telemetry_parse_broker_accepts_ipv4_with_optional_scheme_and_port() {
    let cases: [(&str, Ipv4Addr, u16); 4] = [
        ("192.168.1.10", Ipv4Addr::new(192, 168, 1, 10), 1883),
        ("mqtt://192.168.1.10", Ipv4Addr::new(192, 168, 1, 10), 1883),
        (
            "mqtt://192.168.1.10:1883",
            Ipv4Addr::new(192, 168, 1, 10),
            1883,
        ),
        ("10.0.0.5:1884", Ipv4Addr::new(10, 0, 0, 5), 1884),
    ];

    for (input, host, port) in cases {
        assert_eq!(parse_broker(input), Some((host, port)), "{input}");
    }
}

#[test]
fn telemetry_parse_broker_rejects_hostnames_malformed_and_ipv6() {
    for input in [
        "example.com",
        "1.2.3",
        "",
        "mqtt://[fe80::1]:1883",
        "192.168.1.10:notaport",
        "192.168.1.10:0",
    ] {
        assert_eq!(parse_broker(input), None, "{input}");
    }
}

#[test]
fn telemetry_classify_inbound_exact_match() {
    assert_eq!(
        classify_inbound("garagelight/reset"),
        Inbound::RequestUpdate
    );
    for topic in [
        "garage/temperature",
        "garagelight/resetx",
        "garagelight/reset/extra",
        "",
    ] {
        assert_eq!(classify_inbound(topic), Inbound::Ignore, "{topic}");
    }
}

#[test]
fn telemetry_max_sample_payload_holds_widest_sample() {
    let widest = Sample {
        temperature: -128,
        relative_humidity: 255,
    };
    let mut out = [0u8; MAX_SAMPLE_PAYLOAD];
    let len = encode_sample(widest, u32::MAX, &mut out)
        .expect("widest sample fits in MAX_SAMPLE_PAYLOAD");
    assert_eq!(
        &out[..len],
        b"{\"temp\": -128, \"humidity\": 255, \"version\": 4294967295}"
    );
    assert_eq!(len, MAX_SAMPLE_PAYLOAD);
    assert!(len <= MAX_SAMPLE_PAYLOAD);
}
#[test]
fn ota_decision_table() {
    assert_eq!(CHUNK_BYTES, 4096);
    assert_eq!(decide(5, 4), Decision::Update);
    assert_eq!(decide(5, 5), Decision::Skip);
    assert_eq!(decide(5, 6), Decision::Update);
    assert_eq!(decide(0, 0), Decision::Skip);
}

const OK_HEAD: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 4096\r\nX-Trace: abc\r\n\r\n";

#[test]
fn head_parser_completes_one_byte_at_a_time() {
    let mut parser = HeadParser::new();
    let mut event = HeadEvent::NeedMore;
    for &byte in OK_HEAD {
        event = parser.push(byte);
    }
    assert_eq!(
        event,
        HeadEvent::Complete(ResponseHead {
            status: 200,
            content_length: Some(4096),
        })
    );
    assert_eq!(parser.consumed(), OK_HEAD.len());
}

#[test]
fn head_parser_completes_split_at_every_boundary() {
    for split in 0..=OK_HEAD.len() {
        let mut parser = HeadParser::new();
        let mut event = HeadEvent::NeedMore;
        for &byte in &OK_HEAD[..split] {
            event = parser.push(byte);
        }
        if split < OK_HEAD.len() {
            assert_eq!(
                event,
                HeadEvent::NeedMore,
                "premature event at split {split}"
            );
        }
        for &byte in &OK_HEAD[split..] {
            event = parser.push(byte);
        }
        assert_eq!(
            event,
            HeadEvent::Complete(ResponseHead {
                status: 200,
                content_length: Some(4096),
            }),
            "split at {split}"
        );
    }
}

#[test]
fn head_parser_accepts_404_as_a_head() {
    let mut parser = HeadParser::new();
    let mut event = HeadEvent::NeedMore;
    for &byte in b"HTTP/1.1 404 Not Found\r\n\r\n" {
        event = parser.push(byte);
    }
    assert_eq!(
        event,
        HeadEvent::Complete(ResponseHead {
            status: 404,
            content_length: None,
        })
    );

    let mut parser = HeadParser::new();
    let mut event = HeadEvent::NeedMore;
    for &byte in b"HTTP/1.0 404 Not Found\r\nContent-Length: 0\r\n\r\n" {
        event = parser.push(byte);
    }
    assert_eq!(
        event,
        HeadEvent::Complete(ResponseHead {
            status: 404,
            content_length: Some(0),
        })
    );
}

#[test]
fn head_parser_is_case_insensitive_on_header_names() {
    let mut parser = HeadParser::new();
    let mut event = HeadEvent::NeedMore;
    for &byte in b"HTTP/1.1 200 OK\r\ncOnTeNt-LeNgTh: 17\r\n\r\n" {
        event = parser.push(byte);
    }
    assert_eq!(
        event,
        HeadEvent::Complete(ResponseHead {
            status: 200,
            content_length: Some(17),
        })
    );
}

#[test]
fn head_parser_rejects_transfer_encoding() {
    let mut parser = HeadParser::new();
    let mut event = HeadEvent::NeedMore;
    for &byte in b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 10\r\n\r\n" {
        event = parser.push(byte);
    }
    assert_eq!(event, HeadEvent::Reject(HeadError::TransferEncoding));
}

#[test]
fn head_parser_rejects_missing_content_length() {
    let mut parser = HeadParser::new();
    let mut event = HeadEvent::NeedMore;
    for &byte in b"HTTP/1.1 200 OK\r\nX-Trace: abc\r\n\r\n" {
        event = parser.push(byte);
    }
    assert_eq!(event, HeadEvent::Reject(HeadError::MissingContentLength));
}

#[test]
fn head_parser_rejects_duplicate_content_length() {
    let mut parser = HeadParser::new();
    let mut event = HeadEvent::NeedMore;
    for &byte in b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\n" {
        event = parser.push(byte);
    }
    assert_eq!(event, HeadEvent::Reject(HeadError::DuplicateContentLength));
}

#[test]
fn head_parser_rejects_malformed_and_unexpected_status() {
    let cases: &[(&[u8], HeadError)] = &[
        (b"HTTX/1.1 200 OK\r\n\r\n", HeadError::MalformedStatusLine),
        (b"HTTP/1.1 \r\n\r\n", HeadError::TooFewStatusTokens),
        (b"HTTP/1.1 200\r\n\r\n", HeadError::TooFewStatusTokens),
        (b"HTTP/1.1 abc OK\r\n\r\n", HeadError::NonNumericStatus),
        (
            b"HTTP/1.1 500 Boom\r\nContent-Length: 0\r\n\r\n",
            HeadError::UnexpectedStatus,
        ),
    ];
    for (raw, expected) in cases {
        let mut parser = HeadParser::new();
        let mut event = HeadEvent::NeedMore;
        for &byte in *raw {
            event = parser.push(byte);
        }
        assert_eq!(event, HeadEvent::Reject(*expected), "{raw:?}");
    }
}

#[test]
fn head_parser_rejects_malformed_header_line() {
    let mut parser = HeadParser::new();
    let mut event = HeadEvent::NeedMore;
    for &byte in b"HTTP/1.1 200 OK\r\nno-colon-here\r\n\r\n" {
        event = parser.push(byte);
    }
    assert_eq!(event, HeadEvent::Reject(HeadError::MalformedHeader));
}

#[test]
fn head_parser_rejects_oversized_header_block() {
    let mut parser = HeadParser::new();
    let mut event = HeadEvent::NeedMore;
    for &byte in b"HTTP/1.1 200 OK\r\nX-Pad: " {
        event = parser.push(byte);
    }
    for _ in 0..3000 {
        event = parser.push(b'a');
    }
    assert_eq!(event, HeadEvent::Reject(HeadError::OversizedHeader));
}

#[test]
fn parse_url_shapes() {
    let uri = parse_url("https://host", "proj").unwrap();
    assert!(uri.tls);
    assert_eq!(uri.host, "host");
    assert_eq!(uri.port, 443);
    assert_eq!(uri.path(), "/proj");

    let uri = parse_url("https://host:8443/prefix", "proj").unwrap();
    assert!(uri.tls);
    assert_eq!(uri.host, "host");
    assert_eq!(uri.port, 8443);
    assert_eq!(uri.path(), "/prefix/proj");

    let uri = parse_url("http://host/", "proj").unwrap();
    assert!(!uri.tls);
    assert_eq!(uri.port, 80);
    assert_eq!(uri.path(), "/proj");

    let uri = parse_url("http://192.168.0.10:8080/a/b/", "proj").unwrap();
    assert!(!uri.tls);
    assert_eq!(uri.host, "192.168.0.10");
    assert_eq!(uri.port, 8080);
    assert_eq!(uri.path(), "/a/b/proj");
}

#[test]
fn parse_url_rejects_bad_input() {
    assert_eq!(parse_url("host", "p").unwrap_err(), UrlError::Scheme);
    assert_eq!(parse_url("ftp://host", "p").unwrap_err(), UrlError::Scheme);
    assert_eq!(parse_url("https://", "p").unwrap_err(), UrlError::Host);
    assert_eq!(
        parse_url("https://host:0", "p").unwrap_err(),
        UrlError::Port
    );
    assert_eq!(
        parse_url("https://host:abc", "p").unwrap_err(),
        UrlError::Port
    );
    assert_eq!(
        parse_url("https://host", "").unwrap_err(),
        UrlError::Project
    );
    assert_eq!(
        parse_url("https://host", "a/b").unwrap_err(),
        UrlError::Project
    );
}

#[test]
fn parse_version_accepts_trimmed_digits() {
    assert_eq!(parse_version(b"7"), Some(7));
    assert_eq!(parse_version(b"  42\n"), Some(42));
    assert_eq!(parse_version(b"0"), Some(0));
    assert_eq!(parse_version(b"4294967295"), Some(u32::MAX));
    assert_eq!(parse_version(b"4294967296"), None);
    assert_eq!(parse_version(b""), None);
    assert_eq!(parse_version(b" \n"), None);
    assert_eq!(parse_version(b"1a"), None);
    assert_eq!(parse_version(b"1 2"), None);
}

#[test]
fn parse_sha256_hex_shapes() {
    let hex = b"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    let parsed = parse_sha256_hex(hex).expect("valid lowercase hex");
    assert_eq!(parsed[0], 0xe3);
    assert_eq!(parsed[31], 0x55);

    let mut with_newline = hex.to_vec();
    with_newline.push(b'\n');
    assert_eq!(parse_sha256_hex(&with_newline), Some(parsed));
    assert_eq!(
        parse_sha256_hex(b"  e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  "),
        Some(parsed)
    );

    let upper = b"E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855";
    assert_eq!(parse_sha256_hex(upper), None);
    assert_eq!(parse_sha256_hex(&hex[..63]), None);
    let mut non_hex = hex.to_vec();
    non_hex[63] = b'g';
    assert_eq!(parse_sha256_hex(&non_hex), None);
    assert_eq!(parse_sha256_hex(b""), None);
}

fn b64_str<'a>(input: &[u8], out: &'a mut [u8]) -> Option<&'a str> {
    let n = base64(input, out)?;
    core::str::from_utf8(&out[..n]).ok()
}

#[test]
fn base64_matches_rfc4648_vectors() {
    let mut out = [0u8; 16];
    assert_eq!(b64_str(b"", &mut out), Some(""));
    assert_eq!(b64_str(b"f", &mut out), Some("Zg=="));
    assert_eq!(b64_str(b"fo", &mut out), Some("Zm8="));
    assert_eq!(b64_str(b"foo", &mut out), Some("Zm9v"));
    assert_eq!(b64_str(b"foob", &mut out), Some("Zm9vYg=="));
    assert_eq!(b64_str(b"fooba", &mut out), Some("Zm9vYmE="));
    assert_eq!(b64_str(b"foobar", &mut out), Some("Zm9vYmFy"));
}

#[test]
fn base64_reports_small_buffer() {
    let mut out = [0u8; 3];
    assert_eq!(base64(b"foo", &mut out), None);
    let mut out = [0u8; 4];
    assert_eq!(base64(b"foo", &mut out), Some(4));
}

#[test]
fn basic_authorization_encodes_user_colon_pass() {
    let mut out = [0u8; 64];
    let n = basic_authorization("user", "pass", &mut out).unwrap();
    assert_eq!(&out[..n], b"Basic dXNlcjpwYXNz");

    let n = basic_authorization("Aladdin", "open sesame", &mut out).unwrap();
    assert_eq!(&out[..n], b"Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==");
}

#[test]
fn basic_authorization_omits_empty_or_small_buffer() {
    let mut out = [0u8; 64];
    assert_eq!(basic_authorization("", "", &mut out), None);
    let mut out = [0u8; 8];
    assert_eq!(basic_authorization("user", "pass", &mut out), None);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlashOp {
    Write { offset: usize, len: usize },
    MarkUpdated,
}

#[derive(Default)]
struct MockFlasher {
    ops: Vec<FlashOp>,
}

impl MockFlasher {
    fn writes(&self) -> Vec<(usize, usize)> {
        self.ops
            .iter()
            .filter_map(|op| match op {
                FlashOp::Write { offset, len } => Some((*offset, *len)),
                FlashOp::MarkUpdated => None,
            })
            .collect()
    }

    fn marks(&self) -> usize {
        self.ops
            .iter()
            .filter(|op| matches!(op, FlashOp::MarkUpdated))
            .count()
    }
}

impl Flasher for MockFlasher {
    type Error = ();

    async fn write(&mut self, offset: usize, data: &[u8]) -> Result<(), ()> {
        self.ops.push(FlashOp::Write {
            offset,
            len: data.len(),
        });
        Ok(())
    }

    async fn mark_updated(&mut self) -> Result<(), ()> {
        self.ops.push(FlashOp::MarkUpdated);
        Ok(())
    }
}

struct SliceReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> SliceReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
}

impl BodyReader for SliceReader<'_> {
    type Error = ();

    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        let remaining = self.data.len() - self.pos;
        let n = remaining.min(buf.len());
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

const HELLO: &[u8] = b"hello world";
const HELLO_SHA256: &[u8] = b"b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";

#[test]
fn apply_update_rejects_oversize_before_any_flash_op() {
    let mut flasher = MockFlasher::default();
    let mut body = SliceReader::new(b"");
    let length = u64::from(ACTIVE_BYTES) + 1;
    let expected = parse_sha256_hex(HELLO_SHA256).unwrap();
    let result = pollster::block_on(apply_update(&mut flasher, &mut body, length, &expected));
    assert_eq!(
        result,
        Err(UpdateError::Oversize {
            length,
            capacity: ACTIVE_BYTES,
        })
    );
    assert!(flasher.ops.is_empty(), "size check must precede flashing");
    assert_eq!(flasher.writes(), Vec::<(usize, usize)>::new());
    assert_eq!(flasher.marks(), 0);
    assert_eq!(body.pos, 0, "the body must not be read on oversize");
}

#[test]
fn apply_update_happy_path_marks_exactly_once() {
    let expected = parse_sha256_hex(HELLO_SHA256).unwrap();
    let mut flasher = MockFlasher::default();
    let mut body = SliceReader::new(HELLO);
    let result = pollster::block_on(apply_update(
        &mut flasher,
        &mut body,
        HELLO.len() as u64,
        &expected,
    ));
    assert_eq!(result, Ok(()));
    assert_eq!(flasher.writes(), vec![(0, HELLO.len())]);
    assert_eq!(flasher.marks(), 1);
    assert_eq!(flasher.ops.last(), Some(&FlashOp::MarkUpdated));
}

#[test]
fn apply_update_hash_mismatch_does_not_mark() {
    let expected = parse_sha256_hex(HELLO_SHA256).unwrap();
    let flipped = b"hello worle";
    let mut flasher = MockFlasher::default();
    let mut body = SliceReader::new(flipped);
    let result = pollster::block_on(apply_update(
        &mut flasher,
        &mut body,
        flipped.len() as u64,
        &expected,
    ));
    assert_eq!(result, Err(UpdateError::HashMismatch));
    assert_eq!(flasher.marks(), 0);
}

#[test]
fn apply_update_truncated_body_is_too_short_and_does_not_mark() {
    let expected = parse_sha256_hex(HELLO_SHA256).unwrap();
    let mut flasher = MockFlasher::default();
    let mut body = SliceReader::new(b"hello");
    let result = pollster::block_on(apply_update(&mut flasher, &mut body, 11, &expected));
    assert_eq!(result, Err(UpdateError::TooShort));
    assert_eq!(flasher.marks(), 0);
}

#[test]
fn apply_update_reads_exactly_content_length_and_does_not_over_read() {
    // The body has more bytes than content_length; only the first
    // content_length are read and hashed, never a byte past it.
    let expected: [u8; 32] = sha2::Sha256::digest(b"hell").into();
    let mut flasher = MockFlasher::default();
    let mut body = SliceReader::new(HELLO);
    let result = pollster::block_on(apply_update(&mut flasher, &mut body, 4, &expected));
    assert_eq!(result, Ok(()));
    assert_eq!(body.pos, 4, "must not read past content_length");
    assert_eq!(flasher.writes(), vec![(0, 4)]);
    assert_eq!(flasher.marks(), 1);
}

/// Reader that yields `data` then returns an error on the next read, modelling
/// a server that drops the socket (without a TLS close_notify) after the body.
struct ErrorAfterReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl BodyReader for ErrorAfterReader<'_> {
    type Error = ();
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        if self.pos >= self.data.len() {
            return Err(());
        }
        let n = (self.data.len() - self.pos).min(buf.len());
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

#[test]
fn apply_update_ignores_post_body_read_error() {
    // The reader yields exactly content_length bytes then errors. The error
    // must not surface: the loop never reads past the body.
    let expected: [u8; 32] = sha2::Sha256::digest(HELLO).into();
    let mut flasher = MockFlasher::default();
    let mut body = ErrorAfterReader {
        data: HELLO,
        pos: 0,
    };
    let result = pollster::block_on(apply_update(
        &mut flasher,
        &mut body,
        HELLO.len() as u64,
        &expected,
    ));
    assert_eq!(result, Ok(()));
    assert_eq!(flasher.marks(), 1);
}

#[test]
fn apply_update_streams_multiple_chunks_in_order() {
    let len = 2 * CHUNK_BYTES + 100;
    let data = vec![0x5au8; len];
    let digest = sha2::Sha256::digest(&data);
    let mut expected = [0u8; 32];
    expected.copy_from_slice(&digest);

    let mut flasher = MockFlasher::default();
    let mut body = SliceReader::new(&data);
    let result = pollster::block_on(apply_update(&mut flasher, &mut body, len as u64, &expected));
    assert_eq!(result, Ok(()));
    assert_eq!(
        flasher.writes(),
        vec![
            (0, CHUNK_BYTES),
            (CHUNK_BYTES, CHUNK_BYTES),
            (2 * CHUNK_BYTES, 100),
        ]
    );
    assert!(flasher.writes().iter().all(|(_, n)| *n <= CHUNK_BYTES));
    assert_eq!(flasher.marks(), 1);
}

fn signal(ble: bool, lamp: bool, wifi: bool, mqtt: bool) -> Signals {
    Signals {
        ble_advertising: ble,
        lamp_ready: lamp,
        wifi_associated: wifi,
        mqtt_connected: mqtt,
    }
}

struct ScriptedProbe {
    timeline: Vec<Signals>,
    calls: usize,
}

impl ScriptedProbe {
    fn new(timeline: Vec<Signals>) -> Self {
        Self { timeline, calls: 0 }
    }
}

impl Probe for ScriptedProbe {
    fn sample(&mut self) -> Signals {
        assert!(
            !self.timeline.is_empty(),
            "probe timeline must not be empty"
        );
        let last = self.timeline.len() - 1;
        let idx = self.calls.min(last);
        self.calls += 1;
        self.timeline[idx]
    }
}

struct VirtualClock {
    now: u64,
}

impl VirtualClock {
    fn new() -> Self {
        Self { now: 0 }
    }
}

impl Clock for VirtualClock {
    fn now_ms(&self) -> u64 {
        self.now
    }

    async fn wait(&mut self, ms: u64) {
        self.now += ms;
    }
}

#[test]
fn self_test_confirms_when_mandatory_pass() {
    let mut probe = ScriptedProbe::new(vec![signal(true, true, true, true)]);
    let mut clock = VirtualClock::new();
    let report = pollster::block_on(self_test(&mut probe, &mut clock));
    assert_eq!(report.verdict, Verdict::Confirm);
    assert!(report.ble_advertising);
    assert!(report.lamp_ready);
    assert!(report.elapsed_ms < SELF_TEST_WINDOW_MS);
}

#[test]
fn self_test_reverts_at_window_when_mandatory_never_pass() {
    let mut probe = ScriptedProbe::new(vec![signal(false, false, false, false)]);
    let mut clock = VirtualClock::new();
    let report = pollster::block_on(self_test(&mut probe, &mut clock));
    assert_eq!(report.verdict, Verdict::Revert);
    assert_eq!(report.elapsed_ms, SELF_TEST_WINDOW_MS);
}

#[test]
fn self_test_confirms_with_network_down() {
    let mut probe = ScriptedProbe::new(vec![signal(true, true, false, false)]);
    let mut clock = VirtualClock::new();
    let report = pollster::block_on(self_test(&mut probe, &mut clock));
    assert_eq!(report.verdict, Verdict::Confirm);
    assert!(!report.wifi_associated && !report.mqtt_connected);
}

#[test]
fn self_test_waits_for_late_mandatory() {
    let mut probe = ScriptedProbe::new(vec![
        signal(false, true, true, true),
        signal(false, true, true, true),
        signal(false, true, true, true),
        signal(true, true, true, true),
    ]);
    let mut clock = VirtualClock::new();
    let report = pollster::block_on(self_test(&mut probe, &mut clock));
    assert_eq!(report.verdict, Verdict::Confirm);
    assert!(report.elapsed_ms < SELF_TEST_WINDOW_MS);
}

#[test]
fn self_test_reverts_when_only_one_mandatory_holds() {
    let mut lamp_only = ScriptedProbe::new(vec![signal(false, true, true, true)]);
    let mut clock = VirtualClock::new();
    let report = pollster::block_on(self_test(&mut lamp_only, &mut clock));
    assert_eq!(report.verdict, Verdict::Revert);

    let mut ble_only = ScriptedProbe::new(vec![signal(true, false, true, true)]);
    let mut clock = VirtualClock::new();
    let report = pollster::block_on(self_test(&mut ble_only, &mut clock));
    assert_eq!(report.verdict, Verdict::Revert);
}

#[test]
fn no_ip_watchdog_not_due_below_threshold() {
    let mut wd = NoIpWatchdog::new();
    wd.record(NO_IP_RESET - core::time::Duration::from_secs(1));
    assert!(!wd.is_reset_due());
}

#[test]
fn no_ip_watchdog_due_at_threshold() {
    let mut wd = NoIpWatchdog::new();
    wd.record(NO_IP_RESET);
    assert!(wd.is_reset_due());
}

#[test]
fn no_ip_watchdog_due_above_threshold() {
    let mut wd = NoIpWatchdog::new();
    wd.record(NO_IP_RESET + core::time::Duration::from_secs(10));
    assert!(wd.is_reset_due());
}

#[test]
fn no_ip_watchdog_reset_rearms() {
    let mut wd = NoIpWatchdog::new();
    wd.record(NO_IP_RESET);
    assert!(wd.is_reset_due());
    wd.reset();
    assert!(!wd.is_reset_due());
}

#[test]
fn rejoin_not_due_below_threshold() {
    let mut counter = RejoinCounter::new();
    for _ in 0..REJOIN_AFTER_FAILURES - 1 {
        assert!(!counter.record_failure());
    }
}

#[test]
fn rejoin_due_at_threshold() {
    let mut counter = RejoinCounter::new();
    for _ in 0..REJOIN_AFTER_FAILURES {
        counter.record_failure();
    }
    assert!(counter.record_failure());
}

#[test]
fn rejoin_reset_after_rejoin() {
    let mut counter = RejoinCounter::new();
    for _ in 0..REJOIN_AFTER_FAILURES {
        counter.record_failure();
    }
    counter.reset();
    assert!(!counter.record_failure());
}

#[test]
fn rejoin_reset_on_clean_connect() {
    let mut counter = RejoinCounter::new();
    counter.record_failure();
    counter.record_failure();
    counter.reset();
    assert!(!counter.record_failure());
}
