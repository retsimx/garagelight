# Design — GL-1: Bootstrap embassy-rp project, toolchain, CI build and size budget gate

- **Issue**: [retsimx/garagelight#2](https://github.com/retsimx/garagelight/issues/2) (GL-1)
- **Epic**: [#1](https://github.com/retsimx/garagelight/issues/1)
- **Session**: `gl2-bootstrap`
- **Status**: Awaiting user approval

## 1. Intent and constraints

Stand up a buildable, flashable native-Rust embassy project for the Raspberry Pi Pico W in
`retsimx/garagelight`, replacing the current MicroPython firmware, with:

- a pinned `rust-toolchain.toml` and a two-crate workspace (`app/`, `bootloader/`);
- a minimal application that initialises **UART0 (GP0/GP1) at 115200** and the watchdog, and logs the
  **reset reason** and **running version**;
- the full CYW43 dependency set declared, with all **four** proprietary blobs embedded so the size
  gate measures the real image;
- GitHub Actions CI running fmt, clippy, tests, and a **hard 780 KiB flash-size gate**;
- `README.md` build/flash/recovery docs with the toolchain version and blob provenance/licence;
- a documented review of the pre-existing `master`-branch Rust attempt (reuse vs supersede).

Out of scope (per issue): any radio/BLE/WiFi/DHT/OTA behaviour, and real bootloader provisioning.

### Normative facts fixed by the issue/epic (not open for redesign)

| Fact | Value |
|---|---|
| Target | `thumbv6m-none-eabi`, Rust **stable**, pinned |
| ACTIVE partition | **780 KiB at `0x10006000`** |
| App linker rule | must **not** link stock `link-rp.x` (no boot2 in the app) |
| Blobs (4, gitignored, build-time fetched) | WLAN FW `43439A0.bin` (231,077 B), BT FW `43439A0_btfw.bin` (6,164 B), NVRAM `nvram_rp2040.bin` (742 B), CLM `43439A0_clm.bin` (984 B) — total 238,967 B ≈ 233.4 KiB |
| Blob loading API | WLAN+BT+NVRAM → `cyw43::new_with_bluetooth(...)`; CLM → `control.init(clm)` |
| BLE bootstrap pin | `[patch.crates-io]` of `cyw43`/`cyw43-pio`/`embassy-*` to git rev `3cd51e6d8eb6aff8b0d64d9e56a75a538bcfc65a` |
| `bt-hci` uniqueness | `cargo tree -d` must show exactly one `bt-hci` |
| CI | fmt `--check`, clippy `-D warnings`, tests, `cargo size` vs 780 KiB |

## 2. Review of the pre-existing `master`-branch Rust attempt

`master` holds a single-crate embassy attempt (`Cargo.toml`, `build.rs`, `src/main.rs`, `src/wifi.rs`,
`memory.x`, `build.sh`) pinned to `nightly-2023-07-17` with git-`main` embassy 0.1/0.2 deps, plus two
committed blobs (`firmware/43439A0.bin`, `firmware/43439A0_clm.bin`).

**Reused (adapted, not copied):**

- The `build.rs` "copy `memory.x` to `OUT_DIR` + `rustc-link-search`" pattern.
- The `probe-rs run --chip RP2040` runner and `DEFMT_LOG` env in `.cargo/config.toml`.
- The watchdog feed task shape (`feed()` every 100 ms) and the panic→watchdog-reset intent.
- CYW43 Pico W wiring/pin assignment, recorded for GL-2: power `PIN_23`, CS `PIN_25`, SPI data
  `PIN_24`, SPI clock `PIN_29`, `PIO0`, `DMA_CH0`+`DMA_CH1`.
- RP2040 memory map knowledge (BOOT2/FLASH/RAM) and `opt-level=z`/LTO profile intent.

**Superseded (and why):**

| Old | New | Why |
|---|---|---|
| Single crate | `app/` + `bootloader/` workspace | Issue requires separate binaries with different linker scripts |
| nightly-2023 + git-`main` deps | pinned stable + `[patch]` to a **frozen rev** | Reproducibility; `trouble-host`/`cyw43` need a known-good BLE set |
| App links `-Tlink-rp.x` | App links `link.x` + `defmt.x` only | App must not embed boot2; bootloader owns boot2 (GL-3) |
| `include_bytes!` of 2 committed blobs | 4 blobs, build-time fetched, gitignored | Licence compliance + complete blob set (BT FW and NVRAM were missing) |
| `env!("WIFI_*")` secrets | `app/src/secrets.rs` (gitignored), GL-2 owns WiFi | Issue requires `secrets.rs` gitignored |
| No UART, no reset reason, no version | UART0 115200 + reset reason + version | Field-observability requirement |
| FLASH at `0x10000100`, full 2 MiB | ACTIVE `0x10006000`, 780 KiB | Epic partition contract |

## 3. Approaches considered

| # | Approach | Tradeoffs | Verdict |
|---|---|---|---|
| **A** | **Spec-literal scaffold with a host-testable core.** Two members. Embassy deps target-gated to `cfg(target_arch="arm")`; a tiny `app/src/lib.rs` holds the ACTIVE-budget math + version, unit-tested on the host. Blobs fetched by `build.rs` (UREQ). CI size gate hard-fails. Bootloader is a minimal `cortex-m-rt` stub bin with a placeholder `memory.x`. | Small extra `lib.rs`/target-gating; gives a *real*, meaningful test suite (the size math) and keeps exactly two members. | **Recommended** |
| **B** | Same as A, but no `lib.rs`/target-gating; CI "tests" are compile-only (`cargo test --no-run`) and the bootloader is a lib (path-override) with no `memory.x`. | Simplest tree; but the "test suite" is vacuous and GL-3 must restructure the crate. | Rejected — weak gate, more churn |
| **C** | Same as A, but defer all blobs/cyw43/BLE to GL-2 and only scaffold+CI now. | Violates GL-1 acceptance (`cargo tree -d` one `bt-hci`; four blobs counted). | Rejected — out of contract |

## 4. Detailed design (Approach A)

### 4.1 Workspace layout

```
garagelight/
├── Cargo.toml                 # virtual workspace, [workspace.dependencies], [patch.crates-io], profiles
├── Cargo.lock                 # committed
├── rust-toolchain.toml        # channel = "1.98.1", components, target
├── .cargo/config.toml         # build.target, probe-rs runner, DEFMT_LOG
├── .github/workflows/ci.yml   # fmt, clippy, test, build, size gate
├── .gitignore                 # target/, cyw43-firmware/, app/src/secrets.rs
├── README.md                  # build/flash/test/recovery, toolchain, blob provenance+licence
├── app/
│   ├── Cargo.toml             # bin garagelight-app + lib garagelight_app; arm-gated deps
│   ├── build.rs               # memory.x copy + 4-blob fetch + link flags (NO link-rp.x)
│   ├── memory.x               # FLASH @0x10006000 LENGTH 780K, RAM 264K
│   └── src/
│       ├── main.rs            # embassy main: UART, watchdog, reset reason, version, blobs
│       ├── logging.rs         # UART0 115200 writer + dual defmt/UART macro
│       ├── blobs.rs           # four aligned_bytes! statics
│       └── lib.rs             # ACTIVE budget math + VERSION (host-testable, no deps)
└── bootloader/
    ├── Cargo.toml             # bin garagelight-bootloader
    ├── build.rs               # memory.x copy + link.x/link-rp.x/defmt.x
    ├── memory.x               # PLACEHOLDER (BOOT2, 24K FLASH) — GL-3 owns the real one
    └── src/main.rs            # placeholder #[entry] stub — GL-3 replaces entirely
```

### 4.2 Toolchain

`rust-toolchain.toml`:

```toml
[toolchain]
channel = "1.98.1"            # pinned stable (current at authoring; effective floor ≥1.85)
components = ["rustfmt", "clippy", "llvm-tools-preview"]
targets = ["thumbv6m-none-eabi"]
```

README records **1.98.1** and the reasoning: edition-2024 needs ≥1.85, `trouble-host` floor is 1.80,
so the effective floor is 1.85 and the pinned toolchain is 1.98.1 (rustup installs it on first use).

### 4.3 Dependency strategy (`[workspace.dependencies]` + `[patch.crates-io]`)

Source of truth = the TrouBLE `examples/rp-pico-w` known-good set:

- `[patch.crates-io]`: `cyw43`, `cyw43-pio`, `embassy-rp`, `embassy-sync`, `embassy-executor`,
  `embassy-time` → `git = "https://github.com/embassy-rs/embassy.git", rev = "3cd51e6d…"`.
- Version deps (crates.io unless patched): `embassy-rp 0.10` (`critical-section-impl`, `time-driver`,
  `rp2040`, `defmt`, `unstable-pac`), `embassy-executor 0.10` (`platform-cortex-m`, `executor-thread`,
  `defmt`, `executor-interrupt`), `embassy-time 0.5` (`defmt`, `defmt-timestamp-uptime`),
  `embassy-sync 0.8`, `embassy-net 0.9`, `cyw43 0.7` (`bluetooth`, `defmt`, `firmware-logs`),
  `cyw43-pio 0.10`, `bt-hci 0.10`, `trouble-host 0.8` (`peripheral`, `gatt`), `embassy-boot-rp 0.10`,
  `static_cell 2`, `sha2 0.11`, `minimq 0.13`, an `embedded-hal` DHT11 driver, `defmt 1`, `defmt-rtt 1`,
  `panic-probe 1` (`print-defmt`), `cortex-m`, `cortex-m-rt`.
- `cargo tree -d` gate: exactly one `bt-hci` (0.10.x) — `trouble-host` and `cyw43`'s
  `bt-hci-transport` both resolve to it.

**Target-gating:** embassy/`cyw43`/`trouble-host`/etc. live under
`[target.'cfg(target_arch = "arm")'.dependencies]` in `app/Cargo.toml`, so `lib.rs` (pure logic) can
be unit-tested on the host while the binary builds only for `thumbv6m`. `bootloader` likewise.

> The DHT11 driver crate is pinned in `[workspace.dependencies]` and referenced by `app`; it is never
> called in GL-1 (sensors are GL-9). If a chosen driver does not compile for `thumbv6m`, GL-1 records
> the substitution — the issue explicitly permits incremental dependency proof.

### 4.4 Application

`app/memory.x` (active partition; no BOOT2 region because the app must not embed boot2):

```
MEMORY {
    FLASH : ORIGIN = 0x10006000, LENGTH = 780K
    RAM   : ORIGIN = 0x20000000, LENGTH = 264K
}
```

`app/build.rs`:

1. copy `memory.x` into `OUT_DIR`, add `rustc-link-search`, `rerun-if-changed=memory.x`;
2. fetch the four blobs (plus the Infineon licence text) into `app/cyw43-firmware/` if absent, from
   the **pinned rev** `https://github.com/embassy-rs/embassy/raw/3cd51e6d…/cyw43-firmware/<file>`
   (build-dependency `ureq`, pure-Rust TLS — no OpenSSL);
3. link flags: `--nmagic`, `-Tlink.x`, `-Tdefmt.x`. **`-Tlink-rp.x` is deliberately absent.**

`app/src/blobs.rs` uses `cyw43::aligned_bytes!` for the four statics (WLAN, BT, NVRAM, CLM) so the
image is link-compatible with the eventual `new_with_bluetooth` call, and `main` logs their sizes —
which both proves inclusion and keeps them from being garbage-collected by `--gc-sections`.

`app/src/main.rs` (`#![no_std] #![no_main]`):

```text
embassy_rp::init  →  UART0 logging (GP0/GP1, 115200)
                  →  Read Watchdog::reset_reason() → Forced | TimedOut | None(power-on/debugger)
                  →  log "version=… reset_reason=… blobs[wifi,bt,nvram,clm]=…"
                  →  Watchdog::start(8s); spawn watchdog-feed task
                  →  idle loop
```

`app/src/logging.rs`: a `core::fmt::Write` blocking UART TX writer plus `log!`/`logln!` macros that
emit to **both** UART0 and `defmt`. No float formatting anywhere (RP2040 has no FPU).

`app/src/lib.rs` (host-testable, zero deps):

```rust
pub const ACTIVE_BASE: u32 = 0x1000_6000;
pub const ACTIVE_FLASH_BYTES: u32 = 780 * 1024;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub fn fits(used_bytes: u32) -> bool { used_bytes <= ACTIVE_FLASH_BYTES }
```

with `#[cfg(test)]` tests asserting `fits(ACTIVE_FLASH_BYTES)`, `!fits(ACTIVE_FLASH_BYTES + 1)`, the
blob total `238_967` fits, and `VERSION` is non-empty. This makes the size invariant testable.

### 4.5 Bootloader placeholder

`bootloader` is a minimal real binary so the workspace builds both final-shaped artifacts:
`#![no_std] #![no_main]`, a `#[cortex_m_rt::entry]` stub that loops (+ panic handler), a
`build.rs` linking `link.x`/`link-rp.x`/`defmt.x`, and a clearly-labelled **placeholder** `memory.x`
(BOOT2 + 24 KiB FLASH). The issue assigns the bootloader's *content, memory.x and partition table* to
GL-3; GL-1 keeps only enough to compile and link, and the file header says so.

### 4.6 CI (`.github/workflows/ci.yml`)

Trigger: `push` and `pull_request`. Single Linux job, `dtolnay/rust-toolchain` + cache, then:

1. `cargo fmt --all --check`
2. `cargo clippy --workspace --all-targets --target thumbv6m-none-eabi -- -D warnings`
3. `cargo test -p garagelight-app --lib` (host unit tests for the size math)
4. `cargo build --release --target thumbv6m-none-eabi --workspace`
5. `cargo tree -d` asserted to contain exactly one `bt-hci`
6. **size gate**: `cargo size --release --target thumbv6m-none-eabi -p garagelight-app` and assert
   `text + data ≤ 780*1024` (798,720 B). **Hard fail** the job on breach. `cargo-binutils` installed
   via `cargo install cargo-binutils --locked` (cached).

Size is expected to be ≈240–260 KiB (blobs 233 KiB + minimal code), comfortably inside the budget;
no blob-relocation fallback is needed at GL-1.

### 4.7 Docs and ignore

`README.md`: build, flash (SWD `probe-rs`, BOOTSEL/UF2), test, recovery; the toolchain version and
effective floor; the four blob filenames/sizes, the pinned source URL, the Infineon Permissive Binary
Licence; and the exact `cyw43` calls (`new_with_bluetooth`, `control.init`) recorded for GL-2.

`.gitignore`: `target/`, `app/cyw43-firmware/`, `app/src/secrets.rs` (plus the pre-existing `.idea/`).
The four blobs are never committed.

## 5. Complexity check (proportional)

Material decisions, mapped to the grug shortlist:

- **Reuse over invention (19, 25):** the workspace reuses the upstream embassy-boot app linker shape
  and the TrouBLE `rp-pico-w` dependency/patch/fetch pattern rather than inventing a build system.
- **Smallest solution (2, 20):** only the hooks the acceptance criteria require are added (UART,
  watchdog, reset reason, version, four blob statics). No radio/BLE/MQTT/sensor wiring — those are
  later issues.
- **Coherence (21):** the only additions beyond the issue's file list are `app/src/lib.rs` +
  `logging.rs` + `blobs.rs` and the bootloader's placeholder `build.rs`/`memory.x`, each with a
  stated requirement it serves (host-testable size math; dual UART logging; blob inclusion;
  a linkable both-member build).
- **Types/testing (8, 11):** one small pure module carries the size invariant and its regression
  tests; no elaborate type machinery is introduced.

No material complexity concerns beyond the deliberate (necessary) BLE dependency set.

## 6. Assumptions and risks

- **Effective MSRV** is recorded as pinned 1.98.1 with floor ≥1.85 (edition 2024); the actual floor
  is confirmed during implementation and recorded in README.
- **Cold boot** from reset is out of scope: the ACTIVE app boots over SWD via `probe-rs run`
  (debugger sets PC); true reset-boot arrives with GL-3's bootloader. (User-confirmed.)
- **Blob host** is pinned to the frozen embassy rev, not a moving branch.
- If any contract-listed optional crate (DHT11/MQTT) fails to build for `thumbv6m`, GL-1 documents
  the deferral (issue permits incremental dependency proof).

## 7. Acceptance mapping

| Issue criterion | Design element |
|---|---|
| Build both members, clean checkout | §4.1, §4.5 |
| `Cargo.lock` committed, one `bt-hci` | §4.3 + CI step 5 |
| CI fmt+clippy+tests+`cargo size` | §4.6 |
| Image ≤780 KiB incl. 4 blobs | §4.4 blobs + §4.6 size gate |
| `master` attempt reviewed | §2 |
| Flash over SWD, UART0 115200, reset reason + version | §4.4 |
| README build/flash/recovery | §4.7 |
