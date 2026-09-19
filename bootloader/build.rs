use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;

const FLASH_BASE: u32 = 0x1000_0000;
const FLASH_BYTES: u32 = 2 * 1024 * 1024;
const FLASH_END: u32 = FLASH_BASE + FLASH_BYTES;
const PAGE_BYTES: u32 = 4096;

const PARTITIONS: [&str; 5] = ["BOOT2", "FLASH", "ACTIVE", "DFU", "STATE"];

const SYMBOLS: [&str; 6] = [
    "__bootloader_state_start",
    "__bootloader_state_end",
    "__bootloader_active_start",
    "__bootloader_active_end",
    "__bootloader_dfu_start",
    "__bootloader_dfu_end",
];

fn main() {
    // Copy the memory map into OUT_DIR and put it on the linker search path.
    // Refuse to build if the pinned partition table is internally inconsistent.
    let memory = fs::read_to_string("memory.x").expect("memory.x must be readable");
    validate(&memory);

    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::copy("memory.x", out.join("memory.x")).unwrap();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=memory.x");

    // Linker args: link.x from cortex-m-rt, link-rp.x from embassy-rp (boot2
    // section placement), defmt.x from defmt. --nmagic disables page padding.
    println!("cargo:rustc-link-arg-bins=--nmagic");
    println!("cargo:rustc-link-arg-bins=-Tlink.x");
    println!("cargo:rustc-link-arg-bins=-Tlink-rp.x");
    println!("cargo:rustc-link-arg-bins=-Tdefmt.x");
}

fn validate(text: &str) {
    let regions = parse_memory_x(text);

    for name in PARTITIONS {
        if !regions.contains_key(name) {
            panic!("memory.x is missing the {name} region");
        }
    }

    for name in PARTITIONS {
        let region = &regions[name];
        if region.origin + region.length > FLASH_END {
            panic!(
                "memory.x region {name} ends at {:#x}, past the 2 MiB flash end {FLASH_END:#x}",
                region.origin + region.length
            );
        }
    }
    // The partition regions are page aligned; BOOT2/FLASH deliberately split the
    // 24 KiB bootloader window at the 0x100-byte boot2 vector table, so FLASH
    // starts unaligned.
    for name in ["ACTIVE", "DFU", "STATE"] {
        let region = &regions[name];
        if !region.origin.is_multiple_of(PAGE_BYTES) {
            panic!(
                "memory.x region {name} origin {:#x} is not {PAGE_BYTES}-byte aligned",
                region.origin
            );
        }
        if !region.length.is_multiple_of(PAGE_BYTES) {
            panic!(
                "memory.x region {name} length {:#x} is not {PAGE_BYTES}-byte aligned",
                region.length
            );
        }
    }

    let active = &regions["ACTIVE"];
    let dfu = &regions["DFU"];
    if dfu.origin != active.origin + active.length {
        panic!(
            "memory.x DFU origin {:#x} must equal ACTIVE origin {:#x} + length {:#x}",
            dfu.origin, active.origin, active.length
        );
    }
    if dfu.length != active.length + PAGE_BYTES {
        panic!(
            "memory.x DFU length {:#x} must equal ACTIVE length {:#x} + one page {PAGE_BYTES}",
            dfu.length, active.length
        );
    }

    let mut ordered: Vec<(&str, u32, u32)> = PARTITIONS
        .iter()
        .map(|name| (*name, regions[*name].origin, regions[*name].length))
        .collect();
    ordered.sort_by_key(|(_, origin, _)| *origin);
    for pair in ordered.windows(2) {
        let (first, first_origin, first_length) = pair[0];
        let (second, second_origin, _) = pair[1];
        if first_origin + first_length > second_origin {
            panic!("memory.x region {first} overlaps region {second}");
        }
    }

    for symbol in SYMBOLS {
        if !text.contains(symbol) {
            panic!("memory.x is missing the {symbol} linker symbol");
        }
    }
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("__bootloader_") && !line.contains("ORIGIN(") {
            panic!("memory.x symbol must be flash-relative, not absolute: {line}");
        }
    }
}

struct Region {
    origin: u32,
    length: u32,
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
