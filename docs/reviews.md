# The blind adversarial review loop

The most transferable part of this project was not the design — it was how the design was attacked.

## Protocol

Each round used a **fresh reviewer with no access to the design conversation**. It was given only:

- the draft artefacts (two epic bodies, the issue bodies, a manifest, a machine-readable state file);
- the project's engineering rules and the artefact templates;
- a directive to **verify claims independently rather than take them on trust**, and to report each
  claim as `VERIFIED` / `WRONG` / `UNVERIFIABLE` **with the source used**;
- instructions to judge decomposition adequacy, template compliance, grug-style minimalism, whether
  the solution actually works, cross-repo consistency, and public-repository hygiene;
- an explicit instruction: *"if there are no blockers and no majors, say APPROVE; do not invent
  findings to appear thorough."*

Each report was written to disk, the findings applied, and the next round run against a freshly-blind
reviewer. The loop continued until the verdict was a clean approval.

## Convergence

| Round | Verdict | Blockers | Majors |
|---|---|---|---|
| 1 | CHANGES REQUIRED | 2 | 9 |
| 2 | CHANGES REQUIRED | 3 | 9 |
| 3 | CHANGES REQUIRED | 3 | 7 |
| 4 | CHANGES REQUIRED | 1 | 4 |
| 5 | CHANGES REQUIRED | 0 | 2 |
| 6 | CHANGES REQUIRED | 0 | 3 |
| **7** | **APPROVE** | **0** | **0** |

The blocker line is the story: **2 → 3 → 3 → 1 → 0 → 0 → 0.** Rounds 2–3 found *more* problems than
round 1, because by then the artefacts were detailed enough to be wrong in specific, checkable ways.
Round 6's majors were not design flaws at all — they were the author's own revision-6 edits failing to
propagate into the manifest and the acceptance criteria.

## What kinds of things were found

Findings clustered into five categories, in rough order of value:

1. **Grounding failures — the design asserted something false about the world.**
   - A required firmware blob was **missing from the entire plan** (BLE could never have come up, and
     the flash budget was computed against the wrong inventory).
   - Two crates in the BLE path required **incompatible versions of a shared trait crate**; the
     "obvious" versions do not compile together.
   - An instruction the author wrote from memory was simply wrong: a library function that appears to
     resolve a device by label in fact opens a differently-named path.
   - Firmware blobs needed a specific 4-byte-aligned wrapper type, not a raw byte include.
   - A partition-size rule was stated with the wrong formula (the conclusion survived; the reasoning
     did not).

2. **Internal contradictions — two parts of the plan could not both be true.**
   - The flash write path was assigned to core1 while the same document required parking core1 around
     flash writes — unimplementable. Worse, follow-up verification showed the HAL **already parks both
     cores**, so the hand-rolled version would have deadlocked the device.
   - A critical path was drawn with a dependency that did not exist, while four issues genuinely on
     the longest chain were prioritised below it.
   - Removing the heartbeat silently removed the only detection of a *connected but hung* peer,
     leaving the fail-safe indicator able to lie indefinitely.

3. **Propagation failures — one file disagreed with another.** Priorities, dependency edges and
   "Blocks" lists drifted between the epic tables, the issue bodies, the manifest and the state file.

4. **Decomposition problems.** Issues that could not be completed independently; a bundle of
   unrelated work in one issue; a traceability gap (an epic acceptance criterion owned by nobody).

5. **Hygiene.** Placeholder discipline, proprietary blobs already committed in a public repository's
   history, and an initial "the host's config is X" assertion that no committed evidence supported.

## What the reviewers were good at, and where they were not

**Good at:** checking arithmetic, reading vendored crate source instead of trusting docs, catching
version conflicts, noticing when a sentence contradicted another sentence three files away, and
refusing to approve work that could not be executed without further research.

**Not as good at:** knowing the deployed host's live state (they could not see it) and knowing
intent (they had no access to the conversation). Both of those produced findings that were
*technically* wrong but *usefully* so — an assertion that had no committed evidence deserved to be
challenged, regardless of whether it happened to be true.

## Honest limits of this record

- The **raw review reports are deliberately not published** in this repository: they quote real
  device and host addresses that are themselves already committed in this repository's history, and
  a decommission issue exists specifically to purge or explicitly accept them. Publishing the reports
  would spread them further.
- The approval covers the **seventh** revision. A final revision applied the approving reviewer's own
  *non-blocking* findings and was **not** independently re-reviewed. That is stated rather than
  glossed.
