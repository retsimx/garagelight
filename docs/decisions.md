# Decision log

Every material decision from the design conversation, with the alternatives that were rejected. The
rationale is the interesting part; the conclusions are what the tracker enforces.

---

### 1. Keep the GAP/GATT shape, fix the semantics

- **Question**: should the roles be inverted so the *state owner* (the Pi) is the BLE peripheral and
  the Pico becomes the central? That is the textbook mapping.
- **Chosen**: keep the deployed shape — **Pico = peripheral + GATT server, Pi = central** — and
  replace the abused temperature characteristic with an honest custom one carrying the **beam fact**.
- **Rejected**: inverting the GAP roles. It is more "correct" on ownership, but it means the Pi
  becomes an advertiser (and gains advertising's lifecycle problems) while the Pico gains
  scan/connect/reconnect logic — i.e. more change, on the less robust platform, for the same
  latency.
- **Why it matters**: the latency is set by the **connection interval** and the handling path, not by
  the direction of the GATT roles. Once that was established, the role question became a pure
  architecture-clarity choice, and the cheaper option won.

### 2. A connection, not advertising

- **Question**: broadcast the beam state in advertisements (connectionless, robust to link loss) or
  hold a connection and push writes?
- **Chosen**: **connection at the 7.5 ms minimum interval**.
- **Rejected**: advertising. A scanner must catch the right channel, costing a stochastic 25–60 ms,
  plus an unregister/re-register gap on every payload change. Advertising buys robustness, not speed.
- **Consequence**: the liveness problem that advertising would have solved for free must be handled
  explicitly (see decision 8).

### 3. BLE for control, WiFi only for telemetry and OTA

- **Question**: the Pico already has WiFi. Why not control over MQTT/WiFi (or raw UDP)?
- **Chosen**: **BLE for the control path**; WiFi carries telemetry and OTA.
- **Rejected**: MQTT for control (a broker and an AP in the critical path, and a worse latency tail
  under contention); raw WiFi UDP (better median, but the tail depends on whatever the 2.4 GHz band
  is doing).
- **Why**: a scheduled BLE connection is immune to medium contention; the lamp must not depend on
  the LAN.

### 4. WiFi stays on the Pico (the "pure BLE device" is rejected)

- **Question**: if the Pi owns telemetry anyway, could the Pico drop WiFi entirely — no network
  stack, no MQTT, smaller firmware?
- **Chosen**: **WiFi stays**.
- **Rejected**: a pure-BLE Pico with updates delivered over the BLE link.
- **Why**: OTA must be **out-of-band** relative to the control link. Pushing firmware over the same
  link you might be trying to fix shares fate with the failure; a broken BLE stack would leave no way
  to ship the fix. WiFi also already exists, so the marginal cost is the integration, not the
  hardware.

### 5. Fix it in software; change no hardware

- **Question**: much of the latency is upstream of software — the sensor's response and a mechanical
  relay (~6–20 ms combined).
- **Chosen**: **software and protocol only**. The relay and the sensor stay.
- **Consequence**: an irreducible hardware floor that is reported separately from the measurable
  software/radio budget, and the honest admission that the largest single term cannot be optimised
  here.

### 6. The Pico firmware is rewritten in Rust

- **Question**: keep MicroPython, or rewrite on embassy?
- **Chosen**: **native Rust**, in scope.
- **Accepted costs**: the CYW43 radio firmware blob set (proprietary, four parts), a BLE host stack
  that is younger than the C stack, and a flash budget that must be measured rather than assumed.
- **Why**: deterministic handling (no interpreter jitter), real types, and a testable structure. The
  rewrite is also what makes the fail-safe update machinery (decision 9) possible.

### 7. Two cores, with the radio indivisible

- **Question**: use the chip's second core. Can the BLE stack be offloaded to it to shield the
  control path?
- **Chosen**: **core0 = radio + control + the DFU writer; core1 = DHT11 + logging.** The reason the
  control path gets a high-priority execution context is preemption, not core count.
- **Rejected**: "offload the BLE stack to core1" — impossible. WiFi and BLE are **one chip, one
  driver, one bus**; the stack cannot be split from the radio it drives.
- **Grounded constraint**: the flash driver **already pauses both cores** around an erase/program, so
  hand-rolling a core park would deadlock. That was a blocking review finding.

### 8. The heartbeat: remove the log, keep a silent keepalive

- **Question**: the periodic write generated 928 MB of log. Delete it?
- **Chosen**: delete the **logged** heartbeat; keep a **silent 10 s keepalive**, and add a
  peripheral-side **30 s write-leash**.
- **Why**: the log was the bug, not the traffic. And with only disconnect-driven liveness, a central
  that *hangs* while staying connected is invisible — the supervision timeout never fires, and the
  lamp holds a stale fact indefinitely. A stale driver-facing indicator is the failure this whole
  design exists to prevent.

### 9. The lamp's failure state is its own signal

- **Question**: what should the lamp do when it does not know the truth?
- **Chosen**: a **distinctive double-dip** (~85 % lit, 1 flash/s).
- **Rejected**: driving it off (indistinguishable from "beam intact" — the dangerous ambiguity);
  driving it on (reads as a genuine "car present" alarm); a plain 1 Hz blink (less distinguishable
  from a car crossing).
- **Why**: "unknown" must be visibly different from both known states, and must not strobe
  (photosensitivity).

### 10. Updates: dual-slot, verified, with rollback

- **Question**: the deployed OTA copies files with no integrity check and no rollback.
- **Chosen**: an **A/B image with a trial boot**: the new image lands in the inactive slot, is
  SHA-256 verified, boots once, self-tests, and confirms itself — otherwise the bootloader reverts it.
- **Why**: the failure that matters is an image that boots but is functionally broken (no radio, no
  lamp). Rollback is the cheap insurance that makes an unattended device recoverable.
- **Explicitly not claimed**: SHA-256 detects accidental corruption, not tampering (the hash comes
  from the same unauthenticated server).

### 11. Telemetry: keep MQTT, accept QoS 0

- **Question**: the Pico publishes temperature/humidity every 15 s. Keep MQTT, or write to the
  time-series database directly?
- **Chosen**: **MQTT, unchanged topic and payload** — parity with the deployed system.
- **Deviation**: **QoS 0** instead of today's 1. A periodic level sample is loss-tolerant, and the
  downstream chain already re-samples.
- **Also pinned**: the client library speaks MQTT **v5**, so the broker version is an environment
  requirement, not an assumption.

### 12. Open BLE, no pairing

- **Question**: should the control link be bonded/encrypted?
- **Chosen**: **open, no pairing**, documented. The characteristic is write-only in effect and
  carries no secret; the threat model is a neighbour with a phone.
- **Left open**: bonding is a supported upgrade path in the chosen stack, if that judgement changes.

### 13. Meta-decision: a full plan, and adversarial review of it

- **Question**: a light. A tracker, a dependency graph, and blind reviewers?
- **Chosen**: **yes**, deliberately.
- **Why**: the design had real traps in it — a missing firmware blob, a version conflict that would
  have stalled the first BLE build, a core-assignment contradiction, a liveness hole, and an
  outright wrong API instruction written by the author. Independent review caught all of them before
  a line of firmware was written, and the plan is now resumable by anyone.
- **The honest counterweight**: see [retrospective.md](retrospective.md). Roughly two thirds of the
  delivered scope — and the majority of the review findings — are not required to make the lamp
  correct.
