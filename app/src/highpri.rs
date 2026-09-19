//! High-priority control executor for the lamp apply (GL-6).
//!
//! The lamp apply must preempt radio housekeeping, so it runs on a Cortex-M
//! `InterruptExecutor` at `Priority::P2` (above thread mode). The executor
//! reuses `embassy-executor`'s existing `CortexMPender`; `embassy-rp`'s own
//! executor features are off, so there is no `__pender` conflict.
//!
//! `SWI_IRQ_0` is a free software interrupt: the radio binds `PIO0_IRQ_0` and
//! `DMA_IRQ_0`, and the time driver owns its own.

use embassy_executor::{InterruptExecutor, SendSpawner};
use embassy_rp::interrupt;
use embassy_rp::interrupt::InterruptExt;

static EXECUTOR: InterruptExecutor = InterruptExecutor::new();

#[interrupt]
unsafe fn SWI_IRQ_0() {
    unsafe { EXECUTOR.on_interrupt() }
}

/// Start the high-priority control executor above thread mode.
///
/// Call once at boot, before the radio, and spawn the lamp apply on the
/// returned `SendSpawner` so it preempts the radio runner.
pub fn start() -> SendSpawner {
    interrupt::SWI_IRQ_0.set_priority(interrupt::Priority::P2);
    EXECUTOR.start(interrupt::SWI_IRQ_0)
}
