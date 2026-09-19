//! GL-3 test-only DFU harness (`--features update-selftest`).
//!
//! Runs from the ACTIVE partition with core1 held busy, then streams a
//! candidate image out of the SPARE flash region and into the DFU partition
//! through the app-side [`Updater`]. The bench harness writes the control block
//! out-of-commit as little-endian u32 words at `SPARE_BASE`:
//!
//! ```text
//! [magic: u32 = 0x5433_4C47 ("GL3T"), mode: u32, len: u32][len image bytes]
//! ```
//!
//! `mode = 0` writes the image and halts without marking; `mode = 1` writes,
//! calls `mark_updated()` and resets so the bootloader swaps on the next boot.

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicU32, Ordering};

use cortex_m::peripheral::SCB;
use defmt_rtt as _;
use panic_probe as _;

use embassy_boot::State;
use embassy_executor::Spawner;
use embassy_rp::multicore::{spawn_core1, Stack};
use embassy_rp::uart;
use embassy_rp::watchdog::Watchdog;
use embedded_storage_async::nor_flash::NorFlash;
use garagelight_app::update::{self, Updater, WATCHDOG_TIMEOUT};
use garagelight_app::{logging, logln};
use garagelight_core::layout::{SPARE_BASE, SPARE_BYTES};

/// Control-block header: magic + mode + image length.
const CONTROL_BYTES: u32 = 12;
/// `"GL3T"` as a little-endian u32.
const CONTROL_MAGIC: u32 = 0x5433_4C47;
/// Mode 0: write then halt without marking.
const MODE_EXERCISE: u32 = 0;
/// Mode 1: write, `mark_updated()`, reset.
const MODE_SWAP: u32 = 1;
/// RAM copy chunk written to the DFU partition.
const CHUNK_BYTES: usize = 4096;

/// Heartbeat counter updated by the core1 workload.
static CORE1_TICKS: AtomicU32 = AtomicU32::new(0);
/// Stack for the core1 workload.
static mut CORE1_STACK: Stack<4096> = Stack::new();

fn read_u32(offset: u32) -> u32 {
    unsafe { core::ptr::read_volatile((SPARE_BASE + offset) as *const u32) }
}

fn read_image(dst: &mut [u8], offset: u32) {
    unsafe {
        core::ptr::copy_nonoverlapping(
            (SPARE_BASE + offset) as *const u8,
            dst.as_mut_ptr(),
            dst.len(),
        );
    }
}

/// Feed the watchdog forever so the device holds at a known point.
fn halt() -> ! {
    loop {
        update::feed(WATCHDOG_TIMEOUT);
        core::hint::spin_loop();
    }
}

fn state_name(state: &State) -> &'static str {
    match state {
        State::Boot => "Boot",
        State::Swap => "Swap",
        State::Revert => "Revert",
        State::DfuDetach => "DfuDetach",
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    let tx = uart::UartTx::new_blocking(p.UART0, p.PIN_0, uart::Config::default());
    logging::init(tx);

    let mut wd = Watchdog::new(p.WATCHDOG);
    wd.start(WATCHDOG_TIMEOUT);
    update::init_watchdog(wd);
    update::feed(WATCHDOG_TIMEOUT);

    let stack = unsafe { &mut *core::ptr::addr_of_mut!(CORE1_STACK) };
    spawn_core1(p.CORE1, stack, move || -> ! {
        let mut ticks: u32 = 0;
        loop {
            ticks = ticks.wrapping_add(1);
            CORE1_TICKS.store(ticks, Ordering::Relaxed);
            core::hint::spin_loop();
        }
    });
    logln!("update_selftest core1 active");

    let magic = read_u32(0);
    let mode = read_u32(4);
    let len = read_u32(8);
    logln!(
        "update_selftest control magic=0x{:x} mode={} len={} spare_base=0x{:x}",
        magic,
        mode,
        len,
        SPARE_BASE
    );

    if magic != CONTROL_MAGIC {
        logln!("update_selftest no valid control block (magic mismatch); halting");
        halt();
    }
    if len == 0 || len > SPARE_BYTES - CONTROL_BYTES {
        logln!("update_selftest invalid image length; halting");
        halt();
    }
    if mode != MODE_EXERCISE && mode != MODE_SWAP {
        logln!("update_selftest unknown mode; halting");
        halt();
    }

    let mut updater = Updater::new(p.FLASH);
    let state = match updater.get_state().await {
        Ok(state) => state,
        Err(_) => {
            logln!("update_selftest boot_state=error; halting");
            halt();
        }
    };
    logln!("update_selftest boot_state={}", state_name(&state));

    if !matches!(&state, State::Boot) {
        logln!("update_selftest post-revert/swap boot; halting without re-applying");
        halt();
    }

    let partition = match updater.prepare_update().await {
        Ok(partition) => partition,
        Err(_) => {
            logln!("update_selftest prepare_update failed; halting");
            halt();
        }
    };
    logln!("update_selftest dfu erased; writing {} bytes", len);

    let mut buf = [0u8; CHUNK_BYTES];
    let mut offset: u32 = 0;
    while offset < len {
        update::feed(WATCHDOG_TIMEOUT);
        let remaining = (len - offset) as usize;
        let chunk = remaining.min(CHUNK_BYTES);
        read_image(&mut buf[..chunk], CONTROL_BYTES + offset);
        if partition.write(offset, &buf[..chunk]).await.is_err() {
            logln!("update_selftest write failed offset={}; halting", offset);
            halt();
        }
        offset += chunk as u32;
        if offset.is_multiple_of(64 * 1024) {
            logln!("update_selftest written {} bytes", offset);
        }
    }
    logln!(
        "update_selftest write complete bytes={} core1_ticks={}",
        offset,
        CORE1_TICKS.load(Ordering::Relaxed)
    );
    update::feed(WATCHDOG_TIMEOUT);

    if mode == MODE_SWAP {
        match updater.mark_updated().await {
            Ok(()) => logln!("update_selftest mark_updated ok; resetting"),
            Err(_) => {
                logln!("update_selftest mark_updated failed; halting");
                halt();
            }
        }
        SCB::sys_reset();
    }

    logln!("update_selftest exercise mode complete; halting without marking");
    halt();
}
