# Blog kit

Everything needed to write this up on `retsimx.github.io`. Target: `src/pages/projects/garagelight/`
— an `index.astro` plus numbered chapters, matching the existing series format.

## Titles

- **"A small step for man, a giant step for token wastage"** *(the author's own line — the strongest)*
- "Mission-critical rigour, applied indiscriminately"
- "I spec garage lights like moon landings"
- "Seven blind reviews of a light switch"
- "The most over-engineered garage beam light ever built, on purpose"

## Taglines / pull-quotes

- *"I spec garage lights like moon landings. Imagine what I'd do to something that matters."*
- *"Seven blind adversarial review rounds. On a light switch. The specs are public."*
- *"Mission-critical rigour. Applied indiscriminately."*
- *"Hire me in industry to build your mission-critical [work]"* — works as a closing line.

## Suggested chapters

1. **The bug that wasn't the bug** — a mystery slowdown in the garage turns out to be a host-wide
   Bluetooth setting affecting a completely different device. Plus: 928 MB of log from a two-second
   heartbeat. Establish that the interesting findings arrive sideways.
2. **Reading the code like an adult** — the audit of two small repositories. The control channel is
   a *temperature characteristic* pretending to be a switch; the lamp's control path depends on WiFi
   being up; the input is polled instead of using kernel events; the scan sleeps ten seconds.
3. **Who owns the truth?** — the role/ownership principle (state owner owns the characteristic,
   actuator owns its hardware constants), and why the "textbook" role inversion was *rejected* on
   latency grounds once the real numbers were in.
4. **Can a light be mission-critical?** — the safety reasoning: "off" is indistinguishable from
   "beam intact", so failure needs its own signal; a *connected but hung* peer is invisible to the
   supervision timeout; hence the double-dip fault pattern, the keepalive and the write-leash.
5. **The wire, and other humiliations** — the ranked answers to "what's the lowest-latency path?":
   a physical wire; then a scheduled connection; then UDP; then advertising. And the arithmetic
   showing a mechanical relay plus a human eye dwarf every software and radio term. Then: *"I cannot
   change the hardware — software and protocol only."*
6. **Two cores, one radio** — the enthusiasm for offloading work to the second core, and the
   correction: WiFi and BLE are one chip behind one driver, so the radio is indivisible. What
   multicore *does* buy (the sensor read off the control path), and the deadlock that review caught
   (the HAL already parks both cores).
7. **How to be wrong in public, safely** — the process: contracts pinned inline with the unknowns
   marked; **seven rounds of blind adversarial review** by reviewers with no access to the
   conversation; the convergence table; the missing firmware blob and the version conflict caught
   before a line of firmware existed.
8. **Two epics, twenty issues, thirty-one dependencies, one lamp** — the reveal, played straight:
   the scoreboard, the honest over-engineering audit, and the five-issue version that would have made
   the lamp correct on its own.

## The numbers to use

- **7 / 31 / 20 / 2** — review rounds / dependency edges / issues / epics.
- **Blocker line: 2 → 3 → 3 → 1 → 0 → 0 → 0** — the convergence table is the visual.
- **~150 findings, ~8 load-bearing** — the honest ratio.
- **1,000 ms → 7.5 ms** — the unrelated device dragging, and its fix.
- **928 MB** — the log.
- **~10–15 ms p50 / ~25–35 ms p99** — the software+radio latency target, against **~6–20 ms** of
  hardware floor (sensor + relay).
- **4** — proprietary firmware blobs the radio needs (one of which the plan initially forgot).
- **~3,600 lines** — artefact size, vs a few metres of wire.

## The technical beats worth their own paragraphs

- **A host-wide trap:** Bluetooth connection-interval settings in the daemon config apply to *every*
  LE connection the host makes, not to the link you intended. It silently slowed an unrelated device
  15×.
- **The four-blob radio:** the WiFi/BLE chip needs a WiFi firmware image, a Bluetooth firmware image,
  regulatory data and board config; miss one and the radio comes up half-alive.
- **A crate version deadlock:** the radio driver and the BLE host stack wanted incompatible versions
  of a shared trait crate; the working combination is a specific upstream git revision.
- **`pause_core1` is not idempotent:** the flash HAL already parks the other core, so adding your own
  park nests the call and deadlocks.
- **An API that sounds like it does what you want:** a "resolve by name" function that opens a device
  path rather than looking up the label.
- **Safety semantics as design:** "off" ≠ "unknown"; a periodic keepalive exists so that a hung peer
  is *detectable*; and the failure pattern must be unmistakable at a glance.
- **Review beats writing:** every blocking defect was found by review, not by writing.
- **The safety logic is a host test, not a bench ritual:** the whole fact → lamp state machine — boot,
  apply, link loss, the 30 s leash, an invalid byte, recovery — is driven through a **mock clock** in
  milliseconds in CI. The bench is reserved for what only hardware can prove: the radio, the wiring,
  the real timing.

## Honesty notes (keep these in)

- The two most valuable fixes were found **by accident**, not by the plan.
- Roughly **two thirds of the scope is not required for the lamp to work** — it is parity work
  carried by a language rewrite.
- The latency tuning is **imperceptible**; the document says so and specifies it anyway.
- The original test plans were **present but shallow** — bench-verified rather than host-tested. The
  gap only surfaced when it was asked directly: *"we can't test the hardware locally or in CI, but we
  definitely can and should be able to test the application logic, right?"*
- The design contains **explicit unverified assumptions** (lamp polarity, real latency, server
  behaviour, broker version). Publishing the plan means publishing the uncertainty, which is the
  point.
- The author asked "is this over-engineered?" and then chose the twenty-issue version **on purpose**,
  for the fun and the public record. That is the joke; do not pretend otherwise.

## Assets worth making

- A photo of the actual beam, lamp and Pico in the garage (the punchline is physical).
- The **convergence table** as an image (7 rounds, blockers falling).
- The **critical-path DAG** (the ASCII version in the epic renders fine, but a diagram is nicer).
- The **latency budget table**, with the hardware floor highlighted as unoptimisable.
- A side-by-side of the old control characteristic ("a thermometer that is a switch") and the new
  contract.
- The scoreboard ("2 epics / 20 issues / 31 edges / 1 lamp").

## Links for the post

- Epic, Pico W firmware: https://github.com/retsimx/garagelight/issues/1
- Epic, Pi Zero W side: https://github.com/retsimx/garagebeam/issues/1
- Full decision log, retrospective and design doc: [`docs/`](README.md) in this repository.
