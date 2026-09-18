# garagelight — design, process and retrospective

This directory is the **source material** for a future write-up on
[retsimx.github.io](https://retsimx.github.io) (`src/pages/projects/garagelight/`, following the
same chapters format as the `rshunterbtt` and `tlsr8266-firmware` series).

It exists because the design conversation, the contracts and the adversarial review trail would
otherwise be lost — and because the *process* turned out to be more interesting than the device.

## The system, in one paragraph

A photoelectric beam crosses the garage doorway. Its relay output is wired to a Raspberry Pi Zero W
standing in the garage. The Pi publishes that beam state over Bluetooth Low Energy to a Raspberry Pi
Pico W, which drives a lamp that tells the driver whether the car has cleared the door. The Pico also
reads a DHT11 and publishes temperature/humidity over MQTT. The lamp is not decoration: it is the
input to a human decision about lowering a door onto a car.

## The story, in one paragraph

What began as "how do we handle OTA updates if we rewrite the Pico firmware in Rust?" became a
two-week design exercise that produced **two epics, twenty issues, thirty-one dependency edges**, a
two-core firmware architecture, and **seven rounds of blind adversarial review**. Along the way it
found and fixed three real defects in the *existing* system (a host-wide Bluetooth setting that was
slowing an unrelated device by 15×, a 928 MB unrotated log, and a control path that silently
depended on WiFi being up). It also, by any sane standard, over-engineered a light.

## Read in this order

| Doc | What it covers |
|---|---|
| [design.md](design.md) | The system as deployed, the failure modes found, and the target architecture with its contracts and the latency budget. |
| [decisions.md](decisions.md) | Every material design decision, the options rejected, and why. The "why" narrative. |
| [history.md](history.md) | The chronology of the design conversation: the arc, the pivots, and the dead ends. |
| [retrospective.md](retrospective.md) | What worked, what didn't, and a candid over-engineering audit. |
| [testing.md](testing.md) | The testing strategy: pure policy tested in CI, hardware on the bench, the mock-clock timeline test, and the CI jobs. |
| [reviews.md](reviews.md) | The blind adversarial review loop: seven rounds, the convergence, and the findings that mattered. |
| [blog-source.md](blog-source.md) | The blog kit: hooks, beats, numbers, quotable lines and taglines. |

## Provenance

- **Design conversation**: a long, single-session dialogue between the repository owner and an AI
  assistant, working from the deployed repositories and the real hardware.
- **Blind reviews**: independent reviewer agents with **no access to the design conversation**,
  given only the draft artefacts and the project's engineering rules, tasked with independently
  verifying claims against upstream source. Their verdicts drove seven revisions.
- **The plan itself**: the epics and issues are public and natively dependency-linked:
  - Epic [`retsimx/garagelight#1`](https://github.com/retsimx/garagelight/issues/1) — the Pico W firmware.
  - Epic [`retsimx/garagebeam#1`](https://github.com/retsimx/garagebeam/issues/1) — the Pi Zero W side.
- **Raw review reports** are deliberately **not** published here: they quote real device addresses
  and host addresses that are already committed in this repository's history, and the plan
  (`GL-14`) exists specifically to purge or explicitly accept those. Publishing the reports would
  spread them further. The review summary in [reviews.md](reviews.md) is sanitised.

## The honesty clause

Every claim in this set is either verified against upstream source (crate source, vendor
documentation, the real repositories) or explicitly marked as an unverified assumption. The
retrospective section exists partly to keep the author honest about which parts were engineering
and which parts were theatre.
