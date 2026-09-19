//! GPIO28 lamp adapter: the single owner of the lamp pin (GL-6).
//!
//! The pure fact→lamp machine lives in `garagelight_core::lamp`; this module is
//! the thin hardware glue. One `Channel` carries facts, link loss and leash
//! ticks to the one task that owns the pin, which runs on the high-priority
//! executor so radio housekeeping cannot delay the apply. The fault timer is a
//! low-priority thread-mode task that only enqueues ticks — it never touches the
//! pin.

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

/// Ordered control events. Single consumer: the high-priority lamp task.
pub static EVENTS: Channel<CriticalSectionRawMutex, LampEvent, 8> = Channel::new();

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
    loop {
        let event = EVENTS.receive().await;
        let now = Instant::now().as_millis();
        lamp.machine.apply(event, now);
        lamp.drive(now);
    }
}

/// Low-priority leash/fault tick. Enqueues only; never touches the pin.
#[embassy_executor::task]
async fn fault_timer() {
    loop {
        Timer::after(Duration::from_millis(20)).await;
        let _ = EVENTS.sender().try_send(LampEvent::Tick);
    }
}

/// Spawn the lamp control path: the owner on the high-priority executor, the
/// fault timer on the thread executor. Call before radio/BLE init.
pub fn spawn(hi: SendSpawner, spawner: Spawner, pin: Peri<'static, PIN_28>) {
    logln!("lamp control path started");
    hi.spawn(lamp_task(pin).unwrap());
    spawner.spawn(fault_timer().unwrap());
}
