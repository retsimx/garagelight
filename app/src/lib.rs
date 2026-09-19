#![no_std]

pub mod ble;
pub mod blobs;
pub mod gatt;
pub mod logging;
pub mod net;
pub mod radio;
#[allow(dead_code)]
pub mod secrets;
pub mod update;

/// Integer version identity, parsed at compile time from the repo-root `VERSION`
/// file (injected by `app/build.rs` as `GARAGELIGHT_BUILD_VERSION`).
pub const VERSION: u32 = parse_u32(env!("GARAGELIGHT_BUILD_VERSION"));

/// Parse a bare ASCII integer. `build.rs` already rejected anything else, and
/// `str::parse` is not `const`, so a digit loop is enough here.
const fn parse_u32(s: &str) -> u32 {
    let bytes = s.as_bytes();
    let mut n: u32 = 0;
    let mut i = 0;
    while i < bytes.len() {
        n = n * 10 + (bytes[i] - b'0') as u32;
        i += 1;
    }
    n
}
