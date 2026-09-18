# History of the design conversation

The chronological arc, including the pivots and the dead ends — which are usually the interesting
part of a write-up. Dates are 2026-09-18 unless noted.

## Act 0 — the accident that started it (2026-09-17, and the morning of the 18th)

While tuning an unrelated BLE bridge on the same Pi Zero W, a mystery appeared: **an unrelated BLE
device in the garage had become much slower.** The cause turned out to be a connection-interval
setting that had been written into the host's Bluetooth daemon configuration to keep a different
link in sync. It is not per-device — it applies to **every** LE connection the host makes — and it
had dragged the garage device to a 1000 ms interval. It was reverted and verified back at 7.5 ms.

The moral that shaped everything after: *Bluetooth host configuration is a shared, blunt instrument.*

Hours earlier, the same investigation had cleaned up **928 MB of unrotated log** produced by a
heartbeat that logged a line every two seconds on a single-core Pi.

Those two accidents are why anyone looked at this system at all.

## Act 1 — reading the code (architecture review)

With attention on the garage system, both repositories were read end to end and reviewed for
architecture rather than style. Findings (full list in [design.md](design.md)):

- the control channel **abuses the standard temperature characteristic** as a switch;
- the boot path **blocks on WiFi and resets the device** before BLE ever starts — the control path
  depends on the network;
- polling a sysfs GPIO every 10 ms instead of using kernel line events;
- discovery scans unfiltered and then **sleeps 10 s** before connecting;
- a 2 s heartbeat that exists only because the heartbeat *write* was the only transport;
- a global sysfs GPIO number dependent on a dynamically allocated chip base.

Verdict delivered at the time: *not fried, but three genuine architectural flaws and a lot of hobby
shortcuts.*

## Act 2 — the role question

**The owner's proposal**: the Pi Zero W should be the GATT peripheral and the Pico the central, with
the Pico subscribing to notifications — *"that's the correct approach, I assume?"*

This produced the principle that survived the whole project: **whoever owns the state owns the
characteristic; the actuator owns its own policy and hardware constants.** The beam sensor is wired
to the Pi, so the Pi owns the beam state; only the Pico knows the lamp polarity, so the Pico owns
fact → lamp.

But the *transport* conclusion inverted the instinct: a scheduled **connection** delivers lower,
more predictable latency than connectionless advertising, because a scanner must catch the right
channel while a connected peripheral simply transmits in its next connection event. The role
question then became an architecture-clarity choice rather than a latency one — and the cheaper
option (keep the GAP roles, fix the semantics) won.

## Act 3 — the OTA question, and a detour into the Rust ecosystem

**"If we rewrite the Pico firmware in native Rust, how do we handle OTA?"**

This opened a research thread that produced several of the project's most concrete findings:

- a dual-slot (A/B) bootloader with trial boot and rollback is the established safe pattern;
- the radio firmware is **four proprietary blobs**, not one, and the Bluetooth one is easy to miss —
  a reviewer caught that it was missing from the plan, which would have made BLE impossible;
- the host's Bluetooth stack had been rewritten in Rust, but the obvious crate versions **do not
  compile together**; the working combination is a specific upstream git revision. Another
  reviewer caught that;
- the flash budget must be *measured*, not assumed.

## Act 4 — the latency question, and the wire

**"What's the ultimate low-level way to minimise p50 and p99 latency?"**

The answer, ranked, was: **a wire.** Failing that: a scheduled connection, then UDP, then
advertising. The arithmetic showed the *hardware* dominates — a mechanical relay and the sensor's
response cost more than every software and radio term combined — and that a human eye cannot
perceive any of the difference.

Then the constraint arrived: **"I cannot change the hardware — software and protocol only."**

So the plan was rebuilt around what was actually achievable: kernel line events instead of polling,
the minimum connection interval, taking the blocking sensor read off the control core, and — the
line the owner liked most — **putting the annoying work on the second core**.

Which is also where the design got corrected: the BLE stack *cannot* be moved to that second core,
because WiFi and BLE are one chip behind one driver. Multicore buys removing the sensor read from
the control path. It does not buy a second radio.

## Act 5 — the process pivot

**"Load the epic-forge skill and let's nail down this design and create the epics and issues."**

From here the work stopped being a conversation and became an engineering artefact. The owner
imposed two rules that changed the outcome:

1. **"Establish all contracts here and now. There should be no discovery required in the epic."** —
   every interface, value, parameter and partition pinned inline, or explicitly marked unverified.
2. **"Run multiple blind review rounds… the reviewer must verify template correctness, whether the
   solution is correct, whether the design is actually grounded in reality."**

Between those, three things happened repeatedly: claims were **verified against upstream source**
rather than recalled; **shared configuration and other devices were treated as first-class
constraints**; and the *plan itself* was subjected to adversarial review.

Also in this act: the owner asked the question that produced the most useful self-assessment of the
whole project — *"is this overengineered for a simple light?"* — and then, when offered a five-issue
trim, chose the twenty-issue moon shot anyway, for the joy of it and for the public record.

## Act 6 — publication

The plan was published to the two repositories as two epics and twenty issues, with **31 native
dependency links** created through the forge's issue-dependency API (not comments), and every body
annotated in place so cross-references render as real links. One self-inflicted bug — an annotation
pass that rewrote its own section markers, briefly emptying twenty issue bodies — was caught by the
post-patch verification and fixed before it mattered.

## Act 7 — the testing gap

A question landed late and changed the plan again: *"Sure, we can't test the hardware locally or in
CI, but we definitely can and should be able to test the application logic, right?"*

Reading the artefacts back confirmed it: the test plans were almost entirely bench-and-eyeball, with
CI doing nothing but building. So the plan gained a testing strategy — a **pure `core` crate** for
policy, host tests in CI, a **mock-clock timeline test** for the lamp's entire safety behaviour,
fixture-driven OTA/parser tests, compile-time partition assertions, a `contract.toml` shared
byte-identically between the two repositories, and (on the Pi side) a CI pipeline and a
policy-extraction pass.

Seven review rounds had passed those test plans, because the rubric asked whether a test plan
*existed* — not whether it was any good.

## The shape of the whole thing

```
audit two repos  →  read the role question  →  research OTA/Rust  →  chase latency
      →  accept the hardware floor  →  design the contracts  →  review the design 7×
      →  publish 2 epics + 20 issues + 31 links
```

Five conversations about a lamp. Roughly two thirds of the output is not needed for the lamp to be
correct — see [retrospective.md](retrospective.md).
