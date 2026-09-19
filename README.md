# garagelight — Pico W firmware (native Rust / embassy)

## Overview

The garage beam → lamp system: a beam breaks, the fact is published over BLE and
telemetry, and the lamp reacts. This repository is the **Pico W firmware**, written in
native Rust on `embassy-rp` for the RP2040. GL-1 stands up the buildable, flashable
scaffold; GL-2 brings up the CYW43439 radio (WiFi + BLE coexistence); GL-3 installs the
`embassy-boot-rp` A/B bootloader and the pinned flash partition table. DHT sampling (GL-9)
and MQTT telemetry (GL-8) are implemented; OTA behaviour arrives in a later issue.

- Epic: [retsimx/garagelight#1](https://github.com/retsimx/garagelight/issues/1)
- Bootstrap (GL-1): [retsimx/garagelight#2](https://github.com/retsimx/garagelight/issues/2)
- Radio bring-up (GL-2): [retsimx/garagelight#3](https://github.com/retsimx/garagelight/issues/3)
- Flash layout / A/B boot (GL-3): [retsimx/garagelight#4](https://github.com/retsimx/garagelight/issues/4)

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
core/        pure host-testable policy: contract constants, value rules, budget math, layout
app/         firmware: no_std lib + production, bench and update test binaries
bootloader/  embassy-boot-rp A/B bootloader; owns boot2 and the pinned partition table
```

- `core/` (`garagelight-core`) is `#![cfg_attr(not(test), no_std)]` and depends on no
  hardware-facing crates, so its policy tests run on the host. It also holds the
  partition-layout constants (`core/src/layout.rs`).
- `app/` (`garagelight-app`) is a `#![no_std]` library (`app/src/lib.rs`: `blobs`,
  `logging`, `radio`, `secrets`, `update`) plus four binaries: the production
  `app/src/main.rs` (`garagelight-app`), the feature-gated bench binary
  `app/src/bin/radio_smoke.rs` (`radio_smoke`, enabled by `--features radio-smoke`), and the
  feature-gated update test binaries `app/src/bin/update_selftest.rs` (`update_selftest`) and
  `app/src/bin/update_image.rs` (`update_image`, enabled by `--features update-selftest`).
  Hardware dependencies are gated under
  `[target.'cfg(target_arch = "arm")'.dependencies]`. It links
  `link.x` only (never `link-rp.x`) and carries no boot2.
- `bootloader/` (`garagelight-bootloader`) is the GL-3 `embassy-boot-rp` A/B bootloader. It
  links the stock `link-rp.x`, owns boot2, and swaps a pending DFU image into ACTIVE before
  booting ACTIVE. It is provisioned once over USB/SWD and is never updated OTA.

## Flash layout / partition table (GL-3)

The GL-3 bootloader owns the pinned 2 MiB partition table. Both `bootloader/memory.x` and
`app/memory.x` carry the same table; `core/src/layout.rs` holds the constants with `const`
assertions, and host tests parse both linker scripts to catch drift.

| Region | Start | Size |
|---|---|---|
| BOOTLOADER | 0x10000000 | 24 KiB |
| ACTIVE | 0x10006000 | 780 KiB |
| DFU | 0x100C9000 | 784 KiB |
| STATE | 0x1018D000 | 4 KiB |
| spare | 0x1018E000 | ~456 KiB |

- **BOOTLOADER** — boot2 plus the 24 KiB bootloader window; USB/SWD-only to change.
- **ACTIVE** — the running application image (`garagelight-app`), linked here at 780 KiB.
- **DFU** — the staging slot, one erase page larger than ACTIVE (`DFU = ACTIVE + 1 page`).
- **STATE** — boot state and the swap-progress log. `embassy-boot` requires
  `2 + 4 × (ACTIVE pages)` write-size units (782 for 195 ACTIVE pages), well inside 4 KiB.
- **spare** — unused flash between STATE and the end of the 2 MiB part.

The `__bootloader_*` linker symbols are **flash-relative** (`ORIGIN(ACTIVE) - ORIGIN(BOOT2)`),
matching `embassy_rp::Flash` offsets from `FLASH_BASE = 0x1000_0000`; using the absolute
addresses from the table above would produce out-of-range partitions.

## Build

In a clean checkout, materialise the (gitignored) secrets file first — CI does the same:

```sh
cp app/secrets.example.rs app/src/secrets.rs
cargo build --release --target thumbv6m-none-eabi
```

The firmware version is the single bare integer in the repo-root `VERSION` file, which
`app/build.rs` reads and injects as `GARAGELIGHT_BUILD_VERSION`; the running firmware reports
it in the boot log. The build fails if `VERSION` is not a bare integer, or if
`app/src/secrets.rs` is absent (the error names the `cp` above).

Measure the image against the 780 KiB (798,720 B) budget:

```sh
cargo size --release --target thumbv6m-none-eabi -p garagelight-app
```

Current measured size (all four blobs included):

```
   text    data     bss     dec     hex filename
 360108      68   32208  392384   5fcc0 garagelight-app
```

**text 360,108 + data 68 = 360,176 B** against a budget of **798,720 B = 780 KiB**.

## Test

The host policy suite (contract ↔ `contract.toml`, byte-value rules, budget invariants, and
partition-layout ↔ `memory.x` drift):

```sh
cargo test -p garagelight-core --target x86_64-unknown-linux-gnu
```

The host target must be explicit because `.cargo/config.toml` sets a global
`[build] target = thumbv6m-none-eabi`; without the override the `toml`/`serde` dev-deps try
to compile for the bare-metal target and fail.

Firmware and radio behaviour have no host test; they are covered by the thumbv6m build and
the bench runbook below.

## Provision and flash (USB / SWD)

The GL-3 `embassy-boot-rp` bootloader replaces the temporary GL-1/GL-2 boot2 shim: it owns
boot2 and is what runs from the reset vector, so the shim is no longer needed once the
bootloader is provisioned. Flash the bootloader at `0x10000000`, then the ACTIVE application
at `0x10006000`; every later reset boots through the bootloader:

```sh
cargo build --release --target thumbv6m-none-eabi
probe-rs download --chip RP2040 target/thumbv6m-none-eabi/release/garagelight-bootloader
probe-rs download --chip RP2040 target/thumbv6m-none-eabi/release/garagelight-app
probe-rs reset --chip RP2040
```

Once the bootloader is provisioned, `probe-rs run` works for day-to-day app flashing when the
probe's reset line is wired: it downloads the app and resets the chip, the reset vector runs
the bootloader, and the bootloader loads ACTIVE. On a probe without the reset line wired, run
`probe-rs reset` and then a fresh `probe-rs attach` to stream RTT. Reflashing the app erases
only the sectors from `0x10006000` up, so the bootloader survives.

```sh
probe-rs run --chip RP2040 target/thumbv6m-none-eabi/release/garagelight-app
```

`.cargo/config.toml` sets `runner = "probe-rs run --chip RP2040"` for the bare-metal
target, so the equivalent shorthand works:

```sh
cargo run --release -p garagelight-app
```

The app logs over **UART0 (GP0 TX) at 115200** and defmt/RTT: version, reset reason and
the four blob sizes. The `radio_smoke` bench binary runs the same way.

### A/B update lifecycle

`app/src/update.rs` wraps `embassy_boot::FirmwareUpdater` with an app-side flash adapter that
feeds the shared 8 s watchdog on every erase, write and read (`main.rs` arms that same
watchdog as its first action). The lifecycle:

1. `prepare_update()` / `write_firmware()` stream the new image into **DFU**.
2. `mark_updated()` schedules the swap; on the **next reset** the bootloader copies DFU into
   ACTIVE and runs the new image.
3. The first boot of that image must call `mark_booted()` to confirm it. If it never does,
   the bootloader reverts to the previous image on the **next reset**.
4. `mark_dfu()` requests DFU mode on the next reset instead of booting: the bootloader resets
   into the ROM USB bootloader (BOOTSEL) (see Recovery).

Do **not** add application-side `pause_core1`/`resume_core1`: the `embassy-rp` flash driver
already parks both cores around each flash operation, and the app must not enable
`run-from-ram`.

### OTA protocol (GL-10)

`app/src/ota/` owns the DNS/TCP/TLS transport and the update task; the pure decision,
parsers and streaming session live in `core/src/ota/`. The device fetches three URLs, each a
`GET` with `Connection: close`, rooted at `{OTA_URL}/{OTA_PROJECT}/`:

| Request | Response body |
|---|---|
| `version` | the published version as a bare integer |
| `{version}.bin.sha256` | lowercase hex SHA-256 of the image (64 chars, optional trailing newline) |
| `{version}.bin` | the raw image for the ACTIVE slot |

Fetch order is `version` → `{version}.bin.sha256` → `{version}.bin`. An update is attempted
when the published version **differs** from the running `garagelight_app::VERSION`; a `404` on
any of the three is **transient** (logged, no retry loop — the next boot or `garagelight/reset`
trigger re-checks).

- **Transport** — `https` uses TLS 1.3 (`embedded-tls`). Certificate verification is
  **disabled** (`UnsecureProvider`): the session is encrypted but the server is not
  authenticated. `http://` is supported as a bench subset. `OTA_URL` may name a host (resolved
  by DNS) or an IPv4 literal (which skips DNS).
- **Auth** — HTTP Basic from `OTA_USER`/`OTA_PASSWORD`; the `Authorization` header is omitted
  when both are empty. Credentials are never logged.
- **Framing** — the server **must** send exactly one `Content-Length`. Chunked
  `Transfer-Encoding` is rejected, as are a missing or duplicate `Content-Length`.
- **Streaming bound** — the body is streamed in 4 KiB chunks and never buffered whole.
  `Content-Length` must be ≤ the 780 KiB ACTIVE slot and is checked **before any flash write**,
  so an oversize image touches no flash.
- **Verify and roll back** — the stream is hashed with SHA-256 as it is written to DFU; a
  mismatch aborts and nothing is marked. On success `mark_updated()` schedules the swap and the
  device resets, with the bootloader reverting if the new image does not confirm. SHA-256
  detects **accidental corruption only** — it is fetched from the same unauthenticated server,
  so it is not tamper detection.

## Recovery (BOOTSEL / UF2 and DFU)

**BOOTSEL / UF2 mass-storage recovery.** The RP2040 ROM bootloader is always available, even
when the flash contents are unusable:

1. Hold **BOOTSEL** while plugging in USB.
2. A mass-storage device named **`RPI-RP2`** appears.
3. Copy a `.uf2` image onto it; the board reboots.

Because the bootloader owns boot2 and lives at `0x10000000`, BOOTSEL recovers a board whose
ACTIVE image is bad.

**USB DFU escape hatch.** When OTA is unavailable but the application still runs, it can ask
the bootloader for USB DFU on the next reset via `app/src/update.rs`:

- `Updater::mark_dfu()` — request DFU mode on the next reset; the bootloader resets into the
  ROM USB bootloader (BOOTSEL, mounting as `RPI-RP2`) instead of booting ACTIVE.

This is deliberately kept: a device mounted out of easy reach needs a USB escape hatch that
does not depend on the OTA stack.

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

## Radio bring-up (GL-2)

`app/src/radio.rs` initialises the on-board CYW43439 combo radio once per boot and returns
the WiFi net driver, the `cyw43` control handle and the BLE controller:

- **PIO/SPI wiring** — power `PIN_23`, CS `PIN_25`, SPI data `PIN_24`, SPI clock `PIN_29`,
  `PIO0`, `DMA_CH0` + `DMA_CH1`. `cyw43-pio`'s `PioSpi` drives the bus through PIO0.
- **Four blobs** — WLAN, Bluetooth and NVRAM are passed to `cyw43::new_with_bluetooth`
  (the four aligned blobs in `app/src/blobs.rs`); CLM is loaded afterwards through
  `control.init(clm)`.
- **Power management off** — `PowerManagementMode::None`, logged as
  `radio power_management=none` (no DTIM-bounded sleep latency).
- **LED** — driven through the cyw43 GPIO API (`control.gpio_set(0, …)`), not `embassy-rp`
  GPIO.
- **BLE** — the Bluetooth driver is wrapped in
  `trouble_host::prelude::ExternalController` (the `BleController` type in
  `app/src/radio.rs`); GL-5 builds the peripheral and GATT server on top of it (below).

Production `main.rs` initialises the radio, starts the BLE peripheral and GATT server via
`ble::spawn` (app/src/main.rs:65), brings up the onboard-LED control path
(app/src/main.rs:68-76), and **then** calls `net::spawn` (app/src/main.rs:80). `net::spawn`
builds the `embassy_net::Stack` over the net driver, spawns the runner and the
never-returning reconnect supervisor (app/src/net.rs:42-50). GL-7 joins the AP and supervises
the link; MQTT telemetry (GL-8) is layered on top, and OTA remains a later issue.

### BLE peripheral / GATT (GL-5)

GL-5 brings up the trouble-host BLE peripheral and GATT server. `main.rs` calls `ble::spawn`
immediately after `radio::init` and before the WiFi/net stack, so the control path is
independent of WiFi.

- **GATT contract** — `app/src/gatt.rs` defines the beam service from the shared contract:
  service `6a4c0001-b5a3-4f1e-9c2d-7e8f9a0b1c2d`, characteristic
  `6a4c0002-b5a3-4f1e-9c2d-7e8f9a0b1c2d` (`read` + `write_without_response`, 1 byte:
  `0x00` intact / `0x01` broken, initial `0x00`). The UUIDs and value rules come from
  `contract.toml` via `core/src/contract.rs`.
- **Advertising** — `app/src/ble.rs` advertises as `glgt` (connectable scannable
  undirected), interval 30 ms, with the 128-bit service UUID in the advertising data and
  appearance `GENERIC_POWER_DEVICE`. After a disconnect it re-advertises automatically.
- **Write handling** — a valid write is enqueued to the control path as a
  `LampEvent::Fact` on the `EVENTS` channel in `app/src/lamp.rs`; every connection-end path
  enqueues `LampEvent::LinkDown`. Invalid length/value writes and ATT Write Requests are
  ignored and logged.
- **Not done** — the peripheral never initiates a connection-parameter update, and there is
  no pairing, encryption or bonding.

### Lamp control path (GL-6)

GL-6 drives the lamp on GPIO28 from the validated beam fact and owns the "state unknown"
fault indication.

- **Mapping** — `0x01` (broken) lights the lamp, `0x00` (intact) darkens it.
- **Fault pattern** — a 2 s cycle, ~85 % lit (`1500` on / `150` off / `200` on / `150` off
  ms, 1 flash/s), shown at boot, on link loss (≤ 1 s supervision timeout) and on a 30 s
  write-leash while connected. `app/src/ble.rs` enqueues `LampEvent::Fact`/`LinkDown`, and a
  20 ms `Tick` keeps the leash current.
- **Single owner** — one state machine (`core/src/lamp.rs`, pure, host-tested with an
  injected clock) owns GPIO28 and consumes every event in `app/src/lamp.rs`.
- **High-priority apply** — the owner runs on an `InterruptExecutor` (`SWI_IRQ_0`,
  `Priority::P2`) so the apply preempts the radio runner; the fault tick is a separate
  lower-priority core0 task that never touches the pin.
- **Polarity unverified** — `LAMP_ON_LEVEL` (`true`, active-high) is a single constant; no
  lamp is attached to the prototype, so the wiring polarity is confirmed at cutover.

### WiFi station and DNS (GL-7, GL-10)

GL-7 adds the station join and reconnect supervisor. It built `embassy-net` **without the
`dns` feature**, so at that point the configured `MQTT_BROKER` and `OTA_URL` had to be **IP
literals**, not hostnames. **GL-10** enables `dns` and resolves `OTA_URL` when it names a host;
an IPv4 literal is still accepted and skips DNS. `MQTT_BROKER` is unchanged by GL-10.

### MQTT telemetry (GL-8)

GL-8 publishes the DHT11 sample to the configured broker and subscribes to a reset topic.
`app/src/telemetry.rs` runs one task on core0, spawned after `net::spawn`
(app/src/main.rs:90), so telemetry is best-effort and never resets the device.

- **Session** — client id `garagelight` (`core/src/telemetry.rs:11`), keepalive 60 s
  (`core/src/telemetry.rs:13`), no last-will, no auth and no TLS.
- **Broker** — the `MQTT_BROKER` secret is a compile-time constant (fixed at startup) parsed
  on each connect attempt as an IPv4 literal with an optional `mqtt://` scheme and optional
  `:port` defaulting to 1883 (`core/src/telemetry.rs:72-81`). It accepts
  `mqtt://IP:1883`, `IP:1883`, or a plain `IP`. Hostnames are rejected because the firmware
  has no DNS (GL-7); the example value is `mqtt://CHANGE_ME:1883`.
- **Publish** — topic `garage/temperature` (from `contract.toml` via
  `core/src/contract.rs:19`), payload `{"temp": <int>, "humidity": <int>}`
  (`core/src/telemetry.rs:58-67`), **QoS 0**, published once per fresh DHT11 sample (every
  15 s; `core/src/sensor.rs:31`). A failed sensor read signals nothing and is not published.
- **Subscribe** — `garagelight/reset` (`core/src/telemetry.rs:14`); any message on that
  exact topic logs `mqtt_reset_received` and calls the `update::request_check()` OTA-check
  entry point, currently a log-only stub (`app/src/update.rs:192`).
- **Reconnect** — application-supervised: each attempt opens a fresh `TcpSocket` and session
  handshake, then backs off 1 s doubling to a 60 s ceiling (`app/src/telemetry.rs:53-54`). A
  fresh `Connected` broker session re-subscribes; a resumed `Reconnected` session keeps the
  broker-side subscription. A broker restart resumes with no device reboot.
- **Buffers** — `rx = 256` holds the largest inbound packet (a `garagelight/reset` publish);
  `tx = 512` holds the CONNECT workspace plus the retained SUBSCRIBE plus a QoS-0 encode and
  reconnect headroom; TCP rx/tx are 512 each (`app/src/telemetry.rs:39-42`). `minimq` is
  pinned to `=0.13.3` (`Cargo.toml:22`) and no dependency was added: `embassy-net`'s
  `TcpSocket` already implements the `embedded-io-async` traits.

**Two deliberate deviations from the legacy MicroPython firmware:**

- QoS 1 → **QoS 0** — the sample is periodic and loss-tolerant.
- `minimq` is an **MQTT v5** client, so the broker must speak MQTT v5
  (**mosquitto ≥ 1.6**); a v4-only broker will refuse the connect.

**Bench verification (GL-8).** With the device joined and `MQTT_BROKER` pointing at
`<broker>`:

```sh
# Observe the ~15 s cadence and payload.
mosquitto_sub -h <broker> -t garage/temperature -v -V mqttv5

# Trigger the reset path; the device log then shows
# mqtt_reset_received and ota_check_requested.
mosquitto_pub -h <broker> -t garagelight/reset -m reset
```

Restart mosquitto and confirm the device reconnects, re-subscribes and resumes publishing
with no reboot.

The MQTT v5 connect, the live ~15 s cadence, and the broker-restart recovery are **bench-only
gates not covered by CI**.

### Dependency note — one embassy source

The workspace `[patch.crates-io]` pins **`embassy-net-driver` only** to the same embassy
revision as `cyw43`; `embassy-net` itself stays on the crates.io release (`0.9.1`, smoltcp).
Without the driver patch, `cyw43::NetDriver` implements the git `embassy-net-driver` trait
while crates.io `embassy-net` expects its own registry copy, so `embassy_net::Stack` fails
to type-check with `E0277` ("multiple different versions of crate `embassy_net_driver`").
Patching only the driver crate unifies both halves on one driver trait, which is the
smallest change that compiles. Patching `embassy-net` too would needlessly swap the whole
network backend to the git rev's `xarxa` stack.

The BLE stack versions are pinned by `Cargo.lock`: **`trouble-host 0.8.0`**,
**`bt-hci 0.10.1`** and **`btuuid 0.1.1`**, resolved from the workspace
`trouble-host = "0.8"` / `bt-hci = "0.10"` requirements in `Cargo.toml`.

## Bench / hardware verification runbook (GL-2)

The radio path is hardware-only; verification is an L3 bench run. A spare Pico W is flashed
with the Raspberry Pi `debugprobe` firmware and wired as the SWD probe for the target.

**Flash the probe**

1. Download `debugprobe_on_pico.uf2` from
   <https://github.com/raspberrypi/debugprobe/releases> — the releases have no Pico W
   build; the Pico build works unchanged on a Pico W.
2. Hold **BOOTSEL**, plug the probe Pico W into USB; a `RPI-RP2` volume appears.
3. Drag `debugprobe_on_pico.uf2` onto the `RPI-RP2` volume.

The probe enumerates as `2e8a:000c  Raspberry Pi Debugprobe on Pico (CMSIS-DAP)` with a
`ttyACM0` UART bridge. On a Pico W its onboard LED stays dark — the Pico build drives GP25,
which is the wireless CS on a W, and the W LED sits behind the wireless chip. That is
expected, not a fault.

**Wiring (probe → target)**

| Probe (debugprobe) | Target (Pico W) |
|---|---|
| GP2 | SWCLK |
| GP3 | SWDIO |
| GND | GND |
| GP4 (UART TX) | GP1 (UART RX) — optional UART bridge |
| GP5 (UART RX) | GP0 (UART TX) — optional UART bridge |

Power the target from its own USB and share GND with the probe.

**Install probe-rs**

```sh
cargo install probe-rs-tools --locked
```

On Linux, non-root probe access may need udev rules (see the `probe-rs` docs; `udevadm`
confirms the device is visible).

**Build and run**

Provision the GL-3 bootloader once with the *Provision and flash* commands above, then run
the bench binary — a normal reset boots ACTIVE through the bootloader. If the probe's reset
line is not wired, use `probe-rs reset` then a fresh `probe-rs attach` to stream RTT:

```sh
cargo build --release --target thumbv6m-none-eabi --features radio-smoke --bin radio_smoke
probe-rs run --chip RP2040 target/thumbv6m-none-eabi/release/radio_smoke
```

The GL-1/GL-2 `scripts/boot2_shim.py` workaround is superseded by the real bootloader and is
no longer used. If the bench board is blank, flash the bootloader and the ACTIVE app first as
in *Provision and flash*.

(`cargo run --release -p garagelight-app --features radio-smoke --bin radio_smoke` is the
shorthand — the `.cargo/config.toml` runner already passes `--chip RP2040`.)

**Expected pass evidence** (`radio_smoke`, on defmt/RTT and UART0 GP0/GP1 at 115200):

- `led on` / `led off` — the cyw43 GPIO LED toggles.
- `wifi_ssid=…` lines and `wifi_scan ssids=N` with **N ≥ 1** — the WiFi driver scanned
  through the shared PIO/SPI bus.
- `ble_hci_ok company_identifier=… lmp_subversion=… hci_version=…` — the Bluetooth blob
  loaded and the controller answered HCI.
- `coexistence wifi=up ble=up` — WiFi and BLE are both live in the one image.

A radio-silent bench returns `wifi_scan ssids=0` and `coexistence wifi=down ble=up`; repeat
the run near an AP to obtain the N ≥ 1 pass signal. L3 pass is recorded only from captured
output of this run.

## CI

`.github/workflows/ci.yml` runs on push and pull request:

- **host** — `cargo fmt --all --check`; `cargo clippy -p garagelight-core --all-targets
  --target x86_64-unknown-linux-gnu -- -D warnings`; `cargo test -p garagelight-core
  --target x86_64-unknown-linux-gnu` (the explicit host target is required — see Test).
- **firmware** — copies `app/secrets.example.rs` to `app/src/secrets.rs`; `cargo clippy
  --workspace --target thumbv6m-none-eabi -- -D warnings`; `cargo build --release --target
  thumbv6m-none-eabi --workspace`; clippies and release-builds the feature-gated
  `radio_smoke` bench (`--features radio-smoke --bin radio_smoke`) so it cannot rot;
  asserts exactly one `bt-hci`; and runs the `cargo size` gate on `garagelight-app`,
  failing above 798,720 B.
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
