//! GPIO28 lamp adapter: the single owner of the lamp pin (GL-6).
//!
//! The pure fact→lamp machine lives in `garagelight_core::lamp`; this module is
//! the thin hardware glue. One `Channel` carries facts, link loss and leash
//! ticks to the one task that owns the pin, which runs on the high-priority
//! executor so radio housekeeping cannot delay the apply. The fault timer is a
//! low-priority thread-mode task that only enqueues ticks — it never touches the
//! pin.

use core::sync::atomic::{AtomicBool, Ordering};

use embassy_executor::{SendSpawner, Spawner};
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::PIN_28;
use embassy_rp::Peri;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Instant, Timer};
pub use garagelight_core::lamp::LampEvent;
use garagelight_core::lamp::{LampMachine, LAMP_ON_LEVEL};

use crate::logln;

/// A control event plus the microsecond instant it was posted (0 when the
/// lamp-latency feature is off).
#[derive(Clone, Copy)]
pub struct StampedEvent {
    pub event: LampEvent,
    pub at_us: u64,
}

/// Ordered control events. Single consumer: the high-priority lamp task.
static EVENTS: Channel<CriticalSectionRawMutex, StampedEvent, 8> = Channel::new();

/// Post a non-fact control event (link loss, tick).
pub fn post(event: LampEvent) {
    let _ = EVENTS.try_send(StampedEvent { event, at_us: 0 });
}

/// Post a validated beam fact, stamped when the lamp-latency feature is on.
#[cfg(feature = "lamp-latency")]
pub fn post_fact(fact: garagelight_core::contract::BeamFact) {
    let _ = EVENTS.try_send(StampedEvent {
        event: LampEvent::Fact(fact),
        at_us: Instant::now().as_micros(),
    });
}

/// Post a validated beam fact (unstamped when the lamp-latency feature is off).
#[cfg(not(feature = "lamp-latency"))]
pub fn post_fact(fact: garagelight_core::contract::BeamFact) {
    let _ = EVENTS.try_send(StampedEvent {
        event: LampEvent::Fact(fact),
        at_us: 0,
    });
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
}

/// The single lamp task. Runs on the high-priority executor, so it must not
/// block: no UART logging here.
#[embassy_executor::task]
async fn lamp_task(pin: Peri<'static, PIN_28>) {
    let mut lamp = Lamp::new(pin);
    lamp.drive(Instant::now().as_millis());
    READY.store(true, Ordering::Relaxed);
    loop {
        let msg = EVENTS.receive().await;
        let now = Instant::now().as_millis();
        lamp.machine.apply(msg.event, now);
        lamp.drive(now);
        #[cfg(feature = "lamp-latency")]
        if let LampEvent::Fact(_) = msg.event {
            let applied_us = Instant::now().as_micros();
            defmt::info!("lamp_delta_us={}", applied_us.saturating_sub(msg.at_us));
        }
    }
}

/// Low-priority leash/fault tick. Enqueues only; never touches the pin.
#[embassy_executor::task]
async fn fault_timer() {
    loop {
        Timer::after(Duration::from_millis(20)).await;
        post(LampEvent::Tick);
    }
}

/// Spawn the lamp control path: the owner on the high-priority executor, the
/// fault timer on the thread executor. Call before radio/BLE init.
pub fn spawn(hi: SendSpawner, spawner: Spawner, pin: Peri<'static, PIN_28>) {
    logln!("lamp control path started");
    hi.spawn(lamp_task(pin).unwrap());
    spawner.spawn(fault_timer().unwrap());
}
