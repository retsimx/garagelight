//! PLACEHOLDER bootloader binary.
//!
//! GL-3 (retsimx/garagelight#4) owns the real bootloader: its content, real
//! `memory.x` and the partition table. This file exists only so that the
//! three-crate workspace compiles and links during GL-1; GL-3 replaces this
//! file entirely.

#![no_std]
#![no_main]

use cortex_m_rt::entry;

// Force the rp-pac crate (and therefore its `__INTERRUPTS` vector table) onto
// the link line. Under a whole-workspace build, Cargo feature unification
// enables embassy-rp's `rt` feature (via cyw43-pio), which turns on
// cortex-m-rt's `device` feature. cortex-m-rt's `link.x` then requires the
// `__INTERRUPTS` symbol that rp-pac provides. Referencing the re-export keeps
// this placeholder linkable without starting any hardware.
use embassy_rp::pac as _;

#[entry]
fn main() -> ! {
    loop {
        cortex_m::asm::wfi();
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    cortex_m::asm::udf()
}
