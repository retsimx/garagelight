# Testing strategy

What is tested, where, and why. The rule of thumb: **policy is tested on the host in CI; hardware is
tested on the bench.** Anything that can be decided by a pure function must not require a
soldering iron to verify.

## The layers

| Layer | Scope | Where it runs | Tooling |
|---|---|---|---|
| **L0 — pure policy** | The state machines and parsers: fact → lamp, the fault pattern, the leash, the OTA decision, the HTTP response parser, value validation, payload formation, contract constants | Host, every push | `cargo test` (host target) |
| **L1 — deterministic time** | The same policy, driven through scripted timelines with a virtual clock | Host, every push | `embassy-time`'s `mock-driver` feature ("a MockDriver that can be manually advanced for testing purposes"), or the `std` driver |
| **L2 — on-target automated** | Things that need the real chip but not a human: flash swap/rollback, driver bring-up | Bench, on demand | `embedded-test` driven by `probe-rs` (replaces manual bench steps where practical) |
| **L3 — bench / end-to-end** | Radio, BLE stack behaviour, the real lamp wiring, the real OTA server, latency, soak | Bench, at milestones | Manual procedure, `btmon`, a logic trace, `mosquitto_sub` |
| **L4 — cross-repo contract** | The constants the two repositories share | Host, every push, in both repos | A vendored `contract.toml` + a test in each repo asserting its code matches |

## The architectural prerequisite

None of L0/L1 is possible unless **policy is pure and hardware sits behind ports**. Concretely, the
firmware is split into:

- **`core/`** — `no_std`, but compiles and tests on the host. Contains the fact → lamp state machine,
  the fault-pattern definition, the leash and keepalive rules, the OTA version/decision logic, the
  HTTP response parser, the value-validation rules, the telemetry payload formation, and the
  contract constants. It never touches a peripheral.
- **`app/`** — the hardware glue: cyw43, the BLE host stack, flash, GPIO, timers, the network
  runner. Thin adapters that call into `core`. Not host-testable, and deliberately kept boring.

This mirrors the sibling service on the Pi, which is testable today precisely because it was built
with trait boundaries.

## The tests that matter most

1. **The scripted lamp timeline (L1).** Drive the pure state machine through a sequence with a
   mock clock and assert the pin state at each step:

   | Step | Expect |
   |---|---|
   | boot, no write yet | fault pattern |
   | write `0x01` (broken) | lamp ON |
   | write `0x00` (intact) | lamp OFF |
   | write any other length/value | ignored, no state change |
   | link loss | fault pattern within one supervision timeout |
   | reconnect + write | fault clears, drive follows the fact |
   | connected, but no write for 30 s | fault pattern (the leash) |
   | keepalive-level writes resuming | fault clears |

   This is the safety requirement expressed as a test. It runs in milliseconds with no hardware, and
   it is the one behaviour a bench check can only confirm by luck.

2. **The OTA decision and parser (L0/L1, fixture-driven).** A valid response; a truncated one; a
   chunked response (rejected); a missing `Content-Length` (rejected); a `Content-Length` exceeding
   the ACTIVE slot (rejected **before** any flash write); the version comparison table
   (older / equal / newer, plus the 404-as-transient rule); and the streaming hash against a known
   vector.

3. **The self-test decision (L0).** Mandatory conditions pass ⇒ confirm; a mandatory condition fails
   ⇒ reset (revert); only the best-effort network signals failing ⇒ **still confirm**.

4. **Contract constants (L0 + L4).** The service and characteristic UUIDs, the byte value semantics,
   the initial `read` value, the connection parameters, the keepalive interval, the leash timeout
   (with the invariant leash > keepalive), and the MQTT topic. Asserted against a `contract.toml` that is vendored, byte-identical, in both repositories
   — so the two sides cannot silently drift apart.

5. **Partition arithmetic (compile time).** `const` assertions for alignment, for
   `DFU = ACTIVE + 1 page`, and for the STATE region being large enough. A bad table must fail the
   build, not the boot.

## Continuous integration

Every push and pull request:

- `cargo fmt --check`
- `cargo clippy -- -D warnings` (host, and the embedded target where clippy supports it)
- **`cargo test` on the host** — the `core` policy suite (this is the job that catches real bugs)
- `cargo build --release --target thumbv6m-none-eabi` (the real target)
- a **release build with dummy secrets**, taken from `secrets.example.rs`, so the production profile
  is proven to link before a deploy ever tried it
- the `cargo size` gate against the ACTIVE budget, with all four radio firmware blobs counted

## What is deliberately not tested, and why

- **The radio and the BLE stack** — vendor firmware and a third-party host stack. The isolation
  binary (`radio_smoke`) exists to attribute a failure to the radio rather than the application.
- **Flash hardware and core parking** — needs the chip; covered at L2 and on the bench.
- **Real timing and the real lamp** — the mock clock proves the *logic*; only hardware proves the
  microseconds and the wiring polarity.
- **The real OTA server and the broker** — external contracts, verified explicitly against the real
  systems before the dependent issues are accepted, and then left to the bench.
