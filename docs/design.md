# Design

## Purpose and scope

This document describes the **garage beam → lamp indicator**: what is deployed today, the failure
modes found while auditing it, and the target design with its normative contracts. The normative,
copy-pasteable issue-level contracts live in the public tracker (linked per section); this document
is the readable narrative version and the rationale.

Two repositories are in scope:

- **[`retsimx/garagelight`](https://github.com/retsimx/garagelight)** — the Pico W firmware (today
  MicroPython; target native Rust on embassy).
- **[`retsimx/garagebeam`](https://github.com/retsimx/garagebeam)** — the Pi Zero W service (Rust)
  that reads the beam and drives the lamp over BLE.

## The deployed system

```
  beam sensor (relay output)          DHT11                lamp (solid state)
        │                               │                        ▲
        ▼                               ▼                        │
  ┌───────────────────┐  BLE (1 value)  ┌───────────────────────────────┐
  │  Raspberry Pi     │ ──────────────► │  Raspberry Pi Pico W          │
  │  Zero W (central) │                 │  (peripheral + GATT server)   │
  │  beam service     │                 │  drives lamp; reads DHT11     │
  └───────────────────┘                 └───────────────┬───────────────┘
                                                        │ WiFi / MQTT
                                                        ▼
                                             broker ──► (time-series DB)
```

- The beam sensor's relay output lands on a GPIO of the Pi Zero W. The Pi polls it, and writes a state
  value over BLE to the Pico.
- The Pico W advertises as a BLE peripheral and exposes a **`write` characteristic**; the Pi is the
  BLE central and writes to it.
- The Pico drives the lamp from that value, and separately reads a DHT11 and publishes
  temperature/humidity over MQTT every 15 s.
- Over-the-air updates are file-based: a `boot.py` WiFi-connect and version check precede the
  application.

## Failure modes found while auditing the deployed system

These are the findings that motivated the rework. Each was observed or read from the deployed code,
not hypothesised.

1. **The control path depends on the network being up.** The boot path connects WiFi for up to 60 s
   and *resets the device* if the AP is absent — **before BLE is ever initialised**. A router reboot
   or AP outage therefore takes the lamp out of service, and repeated failure loops the device.
2. **A host-wide Bluetooth setting was crippling an unrelated device.** A connection-interval
   override written into the host's Bluetooth daemon configuration (`[LE] Min/MaxConnectionInterval`)
   is not per-device: it is applied to **every** LE connection the host makes. It had dragged the
   garage device in question — a *different* BLE peripheral — to a 1000 ms interval. Reverted and
   re-measured at 7.5 ms. This was found by accident, while investigating something else.
3. **928 MB of log.** The bridge logged a heartbeat line every 2 s in steady state. The log was
   unrotated. (The heartbeat *writes* were fine; the heartbeat *logging* was the bug.)
4. **The "temperature" that is a switch.** The Pico's control channel hijacks the standard
   Environmental Sensing / Temperature characteristic (0x181A / 0x2A6E) with a thermometer
   appearance, and treats a written integer as a light command. The GATT contract lies to any
   generic BLE client.
5. **Two seconds of avoidable jitter at the input.** The beam is read by polling sysfs every 10 ms,
   instead of using the kernel's GPIO line events.
6. **A ten-second scan on every reconnect.** Peripheral discovery used an unfiltered scan followed by
   an unconditional 10 s sleep, rather than a service-filtered scan driven by discovery events.
7. **Silent staleness.** Liveness relied on a periodic write: a *hung* peer that stays connected was
   indistinguishable from a peer with nothing to say.
8. **Fragile hardware addressing.** The beam input was addressed by a global sysfs GPIO number whose
   meaning depends on a dynamically allocated chip base.

## The target architecture

### Roles, and who owns what

The deployed GAP/GATT shape is **kept deliberately** (only the semantics are fixed). Two principles
drive it:

- **The state owner owns the characteristic.** The beam sensor is wired to the Pi, so the Pi is the
  source of truth and the Pico exposes it. The characteristic therefore carries the **beam fact**
  (`0x00` intact / `0x01` broken) — not a lamp command.
- **The actuator owns its own hardware detail.** Only the Pico knows how the lamp is wired, so the
  **fact → lamp mapping and the drive polarity live in the Pico's firmware** as a single constant.

That split means neither side needs to know the other's electrical reality.

### Contracts (summary)

Full normative detail: [`garagelight#6`](https://github.com/retsimx/garagelight/issues/6) (GATT),
[`#7`](https://github.com/retsimx/garagelight/issues/7) (lamp),
[`garagebeam#4`](https://github.com/retsimx/garagebeam/issues/4) (write path),
[`#5`](https://github.com/retsimx/garagebeam/issues/5) (interval),
[`garagelight#11`](https://github.com/retsimx/garagelight/issues/11)/[`#12`](https://github.com/retsimx/garagelight/issues/12) (OTA),
[`#9`](https://github.com/retsimx/garagelight/issues/9) (telemetry).

| Area | Contract |
|---|---|
| Service / characteristic | Custom 128-bit UUIDs; **read + write-without-response**; exactly 1 byte; other values ignored and logged |
| Value semantics | `0x00` beam intact, `0x01` beam broken — the **fact**, not a command. `read` returns the last applied fact, and `0x00` before the first write (documented as "no fact received yet") |
| Advertising | Connectable, 30 ms, service UUID in the payload, a generic appearance (the bogus thermometer is gone), no pairing |
| Connection | **Interval 7.5 ms** (the BLE minimum), slave latency 0, supervision timeout 1000 ms; requested by the central and re-applied if it drifts; the peripheral never counters |
| Writes | `WriteType::WithoutResponse`, on change, once after each (re)connect for resync, plus a **silent 10 s keepalive with no log line** |
| Lamp policy | broken → ON, intact → OFF; a `LAMP_ON_LEVEL` constant owned by the device that knows the wiring |
| Safe state | The **fault indication** (a distinctive double-dip, ~85 % lit, 1 flash/s) at boot, on link loss (≤ 1 s via the supervision timeout), and on staleness |
| Staleness | A peripheral-side **write-leash**: connected but no valid write for 30 s ⇒ fault indication. This is what covers a *hung* central, which the supervision timeout cannot see |
| Boot ordering | Radio + BLE + lamp **first**; WiFi/OTA/mesh afterwards and asynchronously; **never reset because WiFi is down** |
| Multicore | core0: radio, the BLE/GATT stack, the lamp apply, and the DFU writer. core1: the DHT11 read, telemetry formatting, logging. Cross-core via the chip's multicore-safe mutex |
| Flash | An A/B bootloader with ACTIVE/DFU/state partitions; the radio firmware blobs are embedded per image and fetched at build time (proprietary, never committed) |
| OTA | Version-gated, streamed, SHA-256 verified, written to the inactive slot, trial-booted, and **reverted on the next reset if the new image does not confirm** — bounded so a bad update cannot survive one reset |
| Telemetry | MQTT, topic and payload unchanged (parity with today), every 15 s, QoS 0 |

### Why the lamp gets a fault pattern, not just "off"

Because "off" is **indistinguishable from "beam intact"**. A driver glancing at a dark lamp would read
"all clear". So the failure state must be its own signal: a metronomic double-dip that cannot be
confused with either steady state, nor with the sparse irregular toggling of a car crossing the beam.
It is flash-rate limited (1/s, ≤ 3/s) rather than strobing.

## Testing

Policy is tested on the host, in CI, with no hardware; hardware is tested on the bench. Concretely:
the firmware is split into a **pure `core` crate** (the fact → lamp state machine, the fault pattern,
the leash, the OTA decision and parser, the value rules, the contract constants) and an `app` crate
that holds the hardware adapters. Time-dependent policy is driven by a **mock clock**, so the lamp's
whole safety behaviour is a millisecond-scale host test rather than a bench observation. The two
repositories share a vendored `contract.toml` asserted by a test in each, so the halves cannot drift.
Full detail: [testing.md](testing.md).

## The latency budget

The lamp is judged by a human eye, so the budget is dominated by things that are not the radio:

| Stage | Typical | Worst | Owner |
|---|---|---|---|
| Beam sensor response | 1–5 ms | 10–50 ms | **fixed hardware** |
| Relay operate | 5–15 ms | 20 ms + bounce | **fixed hardware** |
| Pi GPIO detection | ~0.5 ms | ~2 ms | kernel line events |
| Pi software → BLE write issued | ~1–3 ms | ~10 ms | BlueZ D-Bus; a raw L2CAP/ATT bypass exists as a documented optimisation |
| Next connection event | ≤ 7.5 ms | 7.5 ms | the 7.5 ms interval |
| Peripheral handling → pin | ~1–3 ms | ~10 ms | native Rust, on a high-priority path |
| **Software + radio total** | **~10–15 ms** | **~25–35 ms** | |

The hardware floor (sensor + relay, ~6–20 ms) is upstream of the GPIO edge and cannot be measured in
software; it is reported separately.

### Why a connection rather than advertising

Advertising was specified first and then rejected on latency grounds: an advertiser relies on a
scanner catching the right channel, which costs a stochastic 25–60 ms plus an unregister/re-register
gap whenever the payload changes. A **scheduled connection** transmits in the next connection event
(≤ 7.5 ms) and is immune to contention. Advertising would buy robustness to link loss, not speed; the
liveness problem it would have solved is instead handled by the keepalive plus the leash.

## What is deliberately *not* claimed

Honesty is part of the design record:

- **Lamp polarity** is unverified until the bench check; it is one constant, and the cutover issue
  exists to record it.
- **Real end-to-end latency** is a target, measured by a dedicated instrumentation issue, not an
  assertion.
- **Simultaneous WiFi + BLE on this board** is documented by the vendor but must be proven here.
- **The OTA server's HTTP behaviour** (`Content-Length`, no chunked encoding) is an external
  contract that must be verified against the real server.
- **The broker's MQTT v5 support** is a stated environment requirement, checked before the telemetry
  issue is accepted.
- **Host-only facts** (Bluetooth daemon configuration, GPIO chip identity, service coupling) are
  marked as such and are not re-asserted without evidence.
