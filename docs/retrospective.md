# Retrospective

An honest look at how this was designed, what it produced, and how much of it was necessary.

## The scoreboard

| Metric | Value |
|---|---|
| Function delivered | A lamp that says "the car has cleared the door" |
| Repositories | 2 |
| Epics | 2 |
| Issues | 20 |
| Native dependency edges | 31 |
| Blind adversarial review rounds | 7 |
| Findings raised by those rounds | ~150 |
| Findings that changed the design | ~8 |
| Artefact size | ~3,600 lines |
| Hardware cost of the obvious alternative | A few metres of two-core bell wire |

## What genuinely worked

1. **The accidents.** The two highest-value outputs of the entire effort were the morning's
   discoveries: a host-wide Bluetooth setting was slowing an unrelated garage device by 15×, and a
   heartbeat was writing 928 MB of log. Both were found while looking at something else, both were
   fixed in minutes, and neither required a plan, an epic, or a reviewer. Worth remembering when
   the plan feels like the work.

2. **Owning the state.** The rule "whoever owns the state owns the characteristic; the actuator owns
   its own hardware constants" resolved a genuinely confused interface, and it immediately exposed
   the fact that the existing control channel was a *temperature characteristic* pretending to be a
   switch.

3. **Taking the safety state seriously.** The best decision in the whole design was recognising that
   "lamp off" and "beam intact" look identical, so the failure state needs to be its own visible
   signal — and then that a *connected but hung* peer is invisible to a supervision timeout, which
   is why a keepalive and a write-leash exist at all. Neither came from the review process. Both came
   from asking what the lamp is actually *for*.

4. **Independent review, on the claims that mattered.** Reviewers with no access to the conversation
   caught, before any firmware existed:
   - a **missing Bluetooth firmware blob** — BLE could never have initialised, and the flash budget
     was computed against the wrong inventory;
   - a **version conflict** between the radio driver and the BLE host crates that would have stalled
     the first build;
   - a **core-assignment contradiction** that was unimplementable as written — and, on checking
     further, that the driver already parks the other core, so the hand-rolled version would have
     **deadlocked the device**;
   - an **outright wrong API instruction written by the author** (`Chip::from_name` does not resolve
     device labels; it opens `/dev/<name>`);
   - a **miscalculated critical path** that mis-prioritised four issues;
   - a **liveness hole** created by an earlier "simplification" in the same document.

   Every one of those is a real defect that review, not writing, prevented.

5. **Contracts instead of discovery.** Pinning every value, byte, interval and partition inline — and
   marking what could not be verified as unverified — turned the plan into something a stranger (or
   the author, in six months) can execute without re-deriving it.

## What was theatre

1. **Dual-core, high-priority, microsecond-jitter engineering on a signal whose dominant terms are a
   mechanical relay (5–15 ms) and a human eye (~100 ms perceptibility).** The document itself
   contains the latency analysis proving the optimisation is imperceptible. It was specified anyway.
   The only *warranted* part of multicore is getting the blocking sensor read off the control path.

2. **A seven-round review loop.** Diminishing returns set in around round four; the last three rounds
   were mostly the plan contradicting itself because the author's own edits had not propagated. That
   is a tooling problem (single source of truth, generated references) masquerading as diligence.

3. **Breadth that parity demanded, not correctness.** Two thirds of the issues exist to keep
   *existing* behaviour — telemetry, OTA, the file-deployment flow — after a language rewrite. They
   are real work, but they are the rewrite's tax bill, not the lamp's requirements.

4. **Compliance-flavoured detail.** A flash-rate ceiling justified with reference to photosensitivity
   guidance, for a lamp in a garage, is technically defensible and socially hilarious.

5. **Twenty issues and thirty-one edges for `read a relay → write one byte → drive a pin`.**

## What I would do differently

1. **Write the safety-critical core first, then stop.** Define the fact→lamp mapping, the failure
   state, the staleness rule, and the availability rule. That is the whole of the lamp's correctness.
2. **Treat the plan as a hypothesis and ship a slice.** The Pi-side input and interval work does not
   depend on the firmware rewrite at all; it could have landed in a day and been measured, and the
   rewrite scoped afterwards with real numbers.
3. **Generate references, don't hand-maintain them.** Most late-round findings were one file
   disagreeing with another after an edit. Dependency lists and priority tables should be derived
   from a single source, not repeated in four places.
4. **Verify the two or three load-bearing external facts *before* writing anything else.** The blob
   inventory and the crate version matrix were the only blocking unknowns; both were cheap to check
   and expensive to get wrong.
5. **Cap the review loop.** Two rounds, then a decision. A third round only if the second finds a
   blocker.
6. **Ask "what can we test without hardware?" before writing the test plans, not after.** The first
   drafts verified almost everything on the bench — which is slow, manual, and only catches the case
   you happen to exercise. Most of the interesting logic (the lamp state machine, the OTA decision,
   the parser, the value rules) is pure and could have been host-tested from the start. Seven review
   rounds missed it, because the rubric asked whether each issue *had* a test plan, and every issue
   did. Depth is not presence.

## The one-sentence version

The lamp's correctness needed about five issues and a careful hour; the plan needed twenty issues
and seven adversarial reviews; the *fun* needed three thousand six hundred lines. All three were
delivered.
