//! Pure fact→lamp policy: the beam mapping, the fault pattern and the write leash.
//!
//! No hardware and no clock live here. The caller injects `now_ms`, which keeps the
//! whole machine a pure, host-testable function of its inputs.

use crate::contract::{BeamFact, LEASH_TIMEOUT_MS};

/// Electrical level that lights the lamp (`true` = drive high, active-high).
///
/// **Unverified**: the prototype has no lamp attached; the bench cutover owns
/// confirming this against the real wiring.
pub const LAMP_ON_LEVEL: bool = true;

/// Total length of one fault-pattern cycle.
pub const FAULT_CYCLE_MS: u64 = 2000;
/// Lit phase at the start of the cycle.
pub const FAULT_ON1_MS: u64 = 1500;
/// First dark phase.
pub const FAULT_OFF1_MS: u64 = 150;
/// Second (short) lit phase.
pub const FAULT_ON2_MS: u64 = 200;
/// Second dark phase, closing the cycle.
pub const FAULT_OFF2_MS: u64 = 150;

const _: () =
    assert!(FAULT_ON1_MS + FAULT_OFF1_MS + FAULT_ON2_MS + FAULT_OFF2_MS == FAULT_CYCLE_MS);

/// Lit (`true`) or dark (`false`) at `elapsed_ms` into the fault pattern.
///
/// Within each 2000 ms cycle: `[0,1500)` on, `[1500,1650)` off, `[1650,1850)` on,
/// `[1850,2000)` off; the pattern repeats via modulo.
pub fn fault_level(elapsed_ms: u64) -> bool {
    let t = elapsed_ms % FAULT_CYCLE_MS;
    let first_dark_end = FAULT_ON1_MS + FAULT_OFF1_MS;
    let second_lit_end = first_dark_end + FAULT_ON2_MS;
    t < FAULT_ON1_MS || (first_dark_end..second_lit_end).contains(&t)
}

/// Everything the single lamp owner consumes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LampEvent {
    /// A validated beam fact; applying it clears any fault atomically.
    Fact(BeamFact),
    /// The BLE link ended; the safe fault state.
    LinkDown,
    /// A periodic fault-timer evaluation.
    Tick,
}

/// Why the lamp is currently in fault mode.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FaultReason {
    /// Faulted before the machine received its first fact.
    Boot,
    /// The BLE link supervision ended.
    LinkDown,
    /// A live link stopped receiving facts for the write leash.
    Leash,
    /// No fault is active.
    None,
}

impl FaultReason {
    /// Compact identifier used in diagnostics.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Boot => "boot",
            Self::LinkDown => "link_down",
            Self::Leash => "leash",
            Self::None => "none",
        }
    }
}

/// Advance a lamp-fact handshake generation without a rollover.
///
/// `u64::MAX` is a terminal generation: the app keeps that proof value and
/// clears the applied proof before posting each later fact, so a stale applied
/// value can never satisfy a new post.
pub const fn advance_lamp_generation(current: u64) -> Option<u64> {
    current.checked_add(1)
}

/// Exact-match rule for the post-apply ATT Write Response handshake.
///
/// The comparison deliberately is not a `>=` range comparison: only the exact
/// latest fact generation may complete its request.
pub const fn lamp_generation_applied(applied: u64, requested: u64) -> bool {
    applied == requested
}

/// The single-owner fact→lamp machine. `fault` means "state unknown" and drives the
/// distinctive pattern; otherwise the last applied [`BeamFact`] drives the lamp.
pub struct LampMachine {
    fault: bool,
    fact: BeamFact,
    live: bool,
    last_write_ms: u64,
    fault_since_ms: u64,
    /// Observable reason for the current fault; `None` after a valid fact.
    pub fault_reason: FaultReason,
}

impl LampMachine {
    /// Boot state: fault active from the first instant, no fact applied, not live.
    pub const fn new() -> Self {
        Self {
            fault: true,
            fact: BeamFact::Intact,
            live: false,
            last_write_ms: 0,
            fault_since_ms: 0,
            fault_reason: FaultReason::Boot,
        }
    }

    /// Apply a fact, clearing any fault and marking the link live, atomically.
    pub fn on_fact(&mut self, fact: BeamFact, now_ms: u64) {
        self.fact = fact;
        self.fault = false;
        self.live = true;
        self.last_write_ms = now_ms;
        self.fault_reason = FaultReason::None;
    }

    /// The link ended: no live fact, show the fault pattern from `now_ms`.
    pub fn on_link_down(&mut self, now_ms: u64) {
        self.live = false;
        self.fault = true;
        self.fault_since_ms = now_ms;
        self.fault_reason = FaultReason::LinkDown;
    }

    /// A leash tick: fault if a live link has gone quiet for too long.
    pub fn on_tick(&mut self, now_ms: u64) {
        if !self.fault
            && self.live
            && now_ms.saturating_sub(self.last_write_ms) >= u64::from(LEASH_TIMEOUT_MS)
        {
            self.fault = true;
            self.fault_since_ms = now_ms;
            self.fault_reason = FaultReason::Leash;
        }
    }

    /// Return the reason for the current fault state.
    pub const fn fault_reason(&self) -> FaultReason {
        self.fault_reason
    }

    /// The lamp level at `now_ms`: the fault pattern while faulted, else the fact.
    pub fn level(&self, now_ms: u64) -> bool {
        if self.fault {
            fault_level(now_ms.saturating_sub(self.fault_since_ms))
        } else {
            matches!(self.fact, BeamFact::Broken)
        }
    }

    /// Single entry point: dispatch an event to its handler.
    pub fn apply(&mut self, event: LampEvent, now_ms: u64) {
        match event {
            LampEvent::Fact(fact) => self.on_fact(fact, now_ms),
            LampEvent::LinkDown => self.on_link_down(now_ms),
            LampEvent::Tick => self.on_tick(now_ms),
        }
    }
}

impl Default for LampMachine {
    fn default() -> Self {
        Self::new()
    }
}
