//! GPIO28 lamp adapter: the single owner of the lamp pin (GL-6).
//!
//! The pure fact→lamp machine lives in `garagelight_core::lamp`; this module is
//! the thin hardware glue. A latest-state handoff replaces the lossy event
//! channel: a fact or link-loss event supersedes only stale work of the same
//! kind, while the high-priority lamp task remains the sole GPIO28 owner.

use core::cell::RefCell;
use core::sync::atomic::{AtomicBool, Ordering};

use embassy_executor::{SendSpawner, Spawner};
use embassy_futures::select::{select, Either};
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::PIN_28;
use embassy_rp::Peri;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer};
pub use garagelight_core::lamp::LampEvent;
use garagelight_core::lamp::{FaultReason, LampMachine, LAMP_ON_LEVEL};
use portable_atomic::{AtomicU32, AtomicU64, Ordering as PortableOrdering};

use crate::logln;

#[derive(Clone, Copy)]
struct StampedFact {
    fact: garagelight_core::contract::BeamFact,
    #[cfg(feature = "lamp-latency")]
    at_us: u64,
    generation: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LastControl {
    None,
    Fact,
    LinkDown,
}

struct LatestState {
    fact: Option<StampedFact>,
    link_down: bool,
    last_control: LastControl,
}

static LATEST: BlockingMutex<CriticalSectionRawMutex, RefCell<LatestState>> =
    BlockingMutex::new(RefCell::new(LatestState {
        fact: None,
        link_down: false,
        last_control: LastControl::None,
    }));
static STATE_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static NEXT_EVENT: AtomicU64 = AtomicU64::new(0);
static APPLIED_FACT: AtomicU64 = AtomicU64::new(0);
static APPLIED_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static FAULT_REASON: AtomicU32 = AtomicU32::new(1);

fn fault_reason_code(reason: FaultReason) -> u32 {
    match reason {
        FaultReason::Boot => 1,
        FaultReason::LinkDown => 2,
        FaultReason::Leash => 3,
        FaultReason::None => 0,
    }
}

/// Current lamp fault reason, mirrored for the MQTT diagnostics snapshot.
pub fn fault_reason() -> FaultReason {
    match FAULT_REASON.load(PortableOrdering::Relaxed) {
        1 => FaultReason::Boot,
        2 => FaultReason::LinkDown,
        3 => FaultReason::Leash,
        _ => FaultReason::None,
    }
}

/// Post a link-loss event or an already-validated fact to latest state.
pub fn post(event: LampEvent) {
    match event {
        LampEvent::Fact(fact) => {
            let _ = post_fact(fact);
        }
        LampEvent::LinkDown => {
            LATEST.lock(|cell| {
                let mut latest = cell.borrow_mut();
                latest.link_down = true;
                latest.last_control = LastControl::LinkDown;
            });
            STATE_SIGNAL.signal(());
        }
        LampEvent::Tick => {}
    }
}

/// Post a validated fact and return its exact handoff generation.
pub fn post_fact(fact: garagelight_core::contract::BeamFact) -> u64 {
    let generation = loop {
        let current = NEXT_EVENT.load(PortableOrdering::Relaxed);
        match garagelight_core::advance_lamp_generation(current) {
            Some(next) => match NEXT_EVENT.compare_exchange_weak(
                current,
                next,
                PortableOrdering::Relaxed,
                PortableOrdering::Relaxed,
            ) {
                Ok(_) => break next,
                Err(_) => continue,
            },
            None => {
                // u64::MAX is terminal. Clear the exact applied proof before
                // reposting at the terminal value, so no earlier u64::MAX
                // apply can satisfy a later request.
                APPLIED_FACT.store(0, PortableOrdering::Relaxed);
                break u64::MAX;
            }
        }
    };

    #[cfg(feature = "lamp-latency")]
    let at_us = Instant::now().as_micros();

    LATEST.lock(|cell| {
        let mut latest = cell.borrow_mut();
        latest.fact = Some(StampedFact {
            fact,
            #[cfg(feature = "lamp-latency")]
            at_us,
            generation,
        });
        latest.last_control = LastControl::Fact;
    });
    STATE_SIGNAL.signal(());
    generation
}

/// Wait until the exact fact generation has been applied and GPIO28 driven
/// before an ATT Write Response can be sent.
pub async fn wait_fact_applied(generation: u64) {
    loop {
        let applied = APPLIED_FACT.load(PortableOrdering::Relaxed);
        if garagelight_core::lamp_generation_applied(applied, generation) {
            return;
        }
        APPLIED_SIGNAL.wait().await;
    }
}

fn take_latest() -> (Option<StampedFact>, bool, LastControl) {
    LATEST.lock(|cell| {
        let mut latest = cell.borrow_mut();
        let fact = latest.fact.take();
        let link_down = latest.link_down;
        let last_control = latest.last_control;
        latest.link_down = false;
        latest.last_control = LastControl::None;
        (fact, link_down, last_control)
    })
}

/// Latched "the owner task has driven the pin at least once" (GL-11).
static READY: AtomicBool = AtomicBool::new(false);

/// Whether the lamp subsystem is up and applying the fact it holds.
pub fn ready() -> bool {
    READY.load(Ordering::Relaxed)
}

/// The single owner of GPIO28 and of the lamp state machine.
struct Lamp {
    output: Output<'static>,
    machine: LampMachine,
    /// Level last written to the pin; the task is the only owner, so this needs
    /// no locking.
    last: Level,
}

impl Lamp {
    /// Start dark so there is no startup glitch before the first `drive`.
    fn new(pin: Peri<'static, PIN_28>) -> Self {
        let dark = if LAMP_ON_LEVEL {
            Level::Low
        } else {
            Level::High
        };
        Self {
            output: Output::new(pin, dark),
            machine: LampMachine::new(),
            last: dark,
        }
    }

    /// Drive the pin to the level the machine reports for `now_ms`, writing only
    /// when the level changes.
    fn drive(&mut self, now_ms: u64) {
        let lit = self.machine.level(now_ms);
        let level = if lit == LAMP_ON_LEVEL {
            Level::High
        } else {
            Level::Low
        };
        if level != self.last {
            self.output.set_level(level);
            self.last = level;
        }
    }

    fn record_fault_transition(&self, before: FaultReason) {
        let after = self.machine.fault_reason();
        FAULT_REASON.store(fault_reason_code(after), PortableOrdering::Relaxed);
        if after != FaultReason::None && before != after {
            crate::health::record_fault_entry();
        }
    }

    fn apply_fact(&mut self, stamped: StampedFact, now_ms: u64) {
        let before = self.machine.fault_reason();
        self.machine.apply(LampEvent::Fact(stamped.fact), now_ms);
        self.drive(now_ms);
        self.record_fault_transition(before);
        crate::health::record_lamp_applied();

        APPLIED_FACT.store(stamped.generation, PortableOrdering::Relaxed);
        APPLIED_SIGNAL.signal(());

        #[cfg(feature = "lamp-latency")]
        {
            let applied_us = Instant::now().as_micros();
            defmt::info!("lamp_delta_us={}", applied_us.saturating_sub(stamped.at_us));
        }
    }

    fn apply_link_down(&mut self, now_ms: u64) {
        let before = self.machine.fault_reason();
        self.machine.apply(LampEvent::LinkDown, now_ms);
        self.drive(now_ms);
        self.record_fault_transition(before);
    }

    fn apply_tick(&mut self, now_ms: u64) {
        let before = self.machine.fault_reason();
        self.machine.apply(LampEvent::Tick, now_ms);
        self.drive(now_ms);
        self.record_fault_transition(before);
    }
}

/// The single lamp task. Runs on the high-priority executor, so it must not
/// block: no UART logging here. It consumes latest facts/link loss and owns the
/// fault tick itself, eliminating the separate lossy timer task.
#[embassy_executor::task]
async fn lamp_task(pin: Peri<'static, PIN_28>) {
    let mut lamp = Lamp::new(pin);
    FAULT_REASON.store(
        fault_reason_code(FaultReason::Boot),
        PortableOrdering::Relaxed,
    );
    let now = Instant::now().as_millis();
    lamp.drive(now);
    // One boot fault entry is present before the first fact or link event.
    crate::health::record_fault_entry();
    READY.store(true, Ordering::Relaxed);

    loop {
        let period = if lamp.machine.fault_reason() == FaultReason::None {
            Duration::from_secs(1)
        } else {
            Duration::from_millis(20)
        };

        match select(STATE_SIGNAL.wait(), Timer::after(period)).await {
            Either::First(()) => {
                let (fact, link_down, last_control) = take_latest();
                let now = Instant::now().as_millis();

                match (fact, link_down, last_control) {
                    (Some(fact), true, LastControl::LinkDown) => {
                        lamp.apply_link_down(now);
                        lamp.apply_fact(fact, now);
                    }
                    (Some(fact), true, _) => {
                        lamp.apply_fact(fact, now);
                        lamp.apply_link_down(now);
                    }
                    (Some(fact), false, _) => lamp.apply_fact(fact, now),
                    (None, true, _) => lamp.apply_link_down(now),
                    (None, false, _) => {}
                }
            }
            Either::Second(()) => lamp.apply_tick(Instant::now().as_millis()),
        }
    }
}

/// Spawn the lamp control path: the owner on the high-priority executor. Call
/// before radio/BLE init. The thread executor argument remains for call-site
/// compatibility; no separate timer task owns lamp work.
pub fn spawn(hi: SendSpawner, _spawner: Spawner, pin: Peri<'static, PIN_28>) {
    logln!("lamp control path started");
    hi.spawn(lamp_task(pin).unwrap());
}
