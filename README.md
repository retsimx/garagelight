# garagelight — Pico W firmware (native Rust / embassy)

## Overview

The garage beam → lamp system: a beam breaks, the fact is published over BLE and
telemetry, and the lamp reacts. This repository is the **Pico W firmware**, written in
native Rust on `embassy-rp` for the RP2040. GL-1 stands up the buildable, flashable
scaffold; radio/BLE/WiFi/DHT/OTA behaviour arrives in later issues.

- Epic: [retsimx/garagelight#1](https://github.com/retsimx/garagelight/issues/1)
- This issue (GL-1): [retsimx/garagelight#2](https://github.com/retsimx/garagelight/issues/2)

The legacy MicroPython firmware (`main.py`, `boot.py`, `ble.py`, `secrets.py`, …) is the
previous generation and is not the current build.

## Toolchain

`rust-toolchain.toml` pins:

```toml
[toolchain]
channel = "1.98.1"
components = ["rustfmt", "clippy", "llvm-tools-preview"]
targets = ["thumbv6m-none-eabi"]
```

`rustup show` installs the pinned toolchain automatically on first use. If a bare-metal
target is not yet present, add it explicitly:

```sh
rustup target add thumbv6m-none-eabi
```

**Effective floor.** The pinned channel is **1.98.1** (stable). Edition-2024 crates in the
dependency set require Rust ≥ 1.85; the design recorded a `trouble-host` floor of 1.80, so
the effective floor was recorded as 1.85. On-disk verification of the resolved tree shows
the binding constraint is actually higher — `bt-hci 0.10.1` declares
`rust-version = "1.87"` and `minimq 0.13.3` declares `1.88` — so the true effective floor is
1.88, which the pinned 1.98.1 clears. The three workspace crates themselves are edition
2021.

## Layout

```
core/        pure host-testable policy: contract constants, value rules, budget math
app/         firmware: UART logging, watchdog, reset reason, version, CYW43 blobs
bootloader/  placeholder; GL-3 replaces it with the real bootloader (GL-3)
```

- `core/` (`garagelight-core`) is `#![cfg_attr(not(test), no_std)]` and depends on no
  hardware-facing crates, so its policy tests run on the host.
- `app/` (`garagelight-app`) is the firmware binary. Hardware dependencies are gated under
  `[target.'cfg(target_arch = "arm")'.dependencies]`.
- `bootloader/` (`garagelight-bootloader`) compiles and links, but its `memory.x` and entry
  stub are explicitly a placeholder owned by GL-3.

## Build

In a clean checkout, materialise the (gitignored) secrets file first — CI does the same:

```sh
cp app/secrets.example.rs app/src/secrets.rs
cargo build --release --target thumbv6m-none-eabi
```

Measure the image against the 780 KiB (798,720 B) budget:

```sh
cargo size --release --target thumbv6m-none-eabi -p garagelight-app
```

Current measured size (all four blobs included):

```
   text    data     bss     dec     hex filename
 249396      68    1460  250924   3d42c garagelight-app
```

**text 249,396 + data 68 = 249,464 B** against a budget of **798,720 B = 780 KiB**.

## Test

The host policy suite (contract ↔ `contract.toml`, byte-value rules, budget invariants):

```sh
cargo test -p garagelight-core --target x86_64-unknown-linux-gnu
```

The host target must be explicit because `.cargo/config.toml` sets a global
`[build] target = thumbv6m-none-eabi`; without the override the `toml`/`serde` dev-deps try
to compile for the bare-metal target and fail.

## Flash (SWD, development)

```sh
probe-rs run --chip RP2040 target/thumbv6m-none-eabi/release/garagelight-app
```

`.cargo/config.toml` sets `runner = "probe-rs run --chip RP2040"` for the bare-metal
target, so the equivalent shorthand works:

```sh
cargo run --release -p garagelight-app
```

The app logs over **UART0 (GP0 TX) at 115200** and defmt/RTT: version, reset reason and
the four blob sizes.

## Recovery / first provisioning (BOOTSEL / UF2)

There is no self-contained reset-to-bootloader path in GL-1. To recover a bricked board:

1. Hold **BOOTSEL** while plugging in USB.
2. A mass-storage device named **`RPI-RP2`** appears.
3. Copy a `.uf2` image onto it; the board reboots.

Note that GL-1's `garagelight-app` is an **ACTIVE-partition image** linked at
`0x10006000` (780 KiB). The real bootloader, partition table and UF2 flow arrive with
**GL-3**; until then the app is normally run over SWD (above), which sets the program
counter directly.

## CYW43 firmware blobs

Four proprietary Infineon/CYW43 files are needed. They are **never committed**; `app/build.rs`
fetches any missing file (and the licence) at build time into the gitignored
`app/cyw43-firmware/`.

| File | Size | Role |
|---|---:|---|
| `43439A0.bin` | 231,077 B | WLAN firmware |
| `43439A0_btfw.bin` | 6,164 B | Bluetooth firmware |
| `nvram_rp2040.bin` | 742 B | Board NVRAM |
| `43439A0_clm.bin` | 984 B | CLM |
| **Total** | **238,967 B** | |

Pinned source base URL (frozen embassy revision `3cd51e6d8eb6aff8b0d64d9e56a75a538bcfc65a`,
the same rev as the `[patch.crates-io]` pins):

```
https://github.com/embassy-rs/embassy/raw/3cd51e6d8eb6aff8b0d64d9e56a75a538bcfc65a/cyw43-firmware/
```

Licence: **Infineon Permissive Binary License**, fetched alongside the blobs as
`LICENSE-permissive-binary-license-1.0.txt`.

Recorded for GL-2 (the blobs are shaped so these calls work unchanged):

```rust
cyw43::new_with_bluetooth(state, pwr, spi, WIFI_FW, BT_FW, NVRAM)
control.init(CLM)
```

WLAN + BT + NVRAM go through `new_with_bluetooth`; **CLM is loaded via the
`control.init(clm)` path**, not the constructor.

## CI

`.github/workflows/ci.yml` runs on push and pull request:

- **host** — `cargo fmt --all --check`; `cargo clippy -p garagelight-core --all-targets
  --target x86_64-unknown-linux-gnu -- -D warnings`; `cargo test -p garagelight-core
  --target x86_64-unknown-linux-gnu` (the explicit host target is required — see Test).
- **firmware** — copies `app/secrets.example.rs` to `app/src/secrets.rs`; `cargo clippy
  --workspace --target thumbv6m-none-eabi -- -D warnings`; `cargo build --release --target
  thumbv6m-none-eabi --workspace`; asserts exactly one `bt-hci`; and runs the
  `cargo size` gate, failing above 798,720 B.
- **contract** — fetches
  `https://raw.githubusercontent.com/retsimx/garagebeam/main/contract.toml` and compares it
  byte-for-byte with the local `contract.toml`, failing on any difference. While the sibling
  mirror (GB-7) does not exist the raw fetch 404s, so the job emits a warning and skips; it
  hard-fails automatically once the file lands.

## `master`-branch review

The earlier single-crate embassy attempt on `master` was reviewed before this scaffold.

**Reused (adapted):**

- The `build.rs` "copy `memory.x` to `OUT_DIR` + `rustc-link-search` + `rerun-if-changed`"
  pattern.
- The `probe-rs run --chip RP2040` runner and `DEFMT_LOG` env in `.cargo/config.toml`.
- The watchdog-feed task shape (`start` + periodic `feed`).
- The CYW43 Pico W pin wiring recorded for GL-2: power `PIN_23`, CS `PIN_25`, SPI data
  `PIN_24`, SPI clock `PIN_29`, `PIO0`, `DMA_CH0`/`DMA_CH1`.

**Superseded:**

| Old (`master`) | New | Why |
|---|---|---|
| Single crate | `core/` + `app/` + `bootloader/` | pure host-testable policy must be split from hardware; separate bootloader binary |
| `nightly-2023-07-17` + git-`main` embassy deps | pinned stable **1.98.1** + `[patch.crates-io]` to frozen rev `3cd51e6d…` | reproducibility and a known-good `trouble-host`/`cyw43` BLE set |
| App linked `-Tlink-rp.x` | app links `link.x` + `defmt.x` only | the app must not embed boot2; the bootloader owns it (GL-3) |
| Two committed blobs (`43439A0.bin`, `43439A0_clm.bin`) | four blobs, build-time fetched, gitignored | licence compliance + complete set (BT firmware and NVRAM were missing) |
| `env!("WIFI_*")` secrets | `app/src/secrets.rs` from `secrets.example.rs` (gitignored) | one-time copy in a clean checkout; GL-2 owns WiFi |
| No UART, reset reason or version | UART0 115200 + reset reason + version | field observability |

## Contract

`contract.toml` is the shared BLE GATT / connection / telemetry contract. It is vendored
byte-identically in the sibling `retsimx/garagebeam` repository (GB-7). The `core` crate
holds the constants and a host test asserts each of them against the file; CI additionally
checks cross-repo byte equality.
