//! GL-3 A/B bootloader (retsimx/garagelight#4).
//!
//! Boring and minimal: it inspects the `embassy-boot` state partition, swaps
//! the DFU image into ACTIVE when a swap is pending, and boots the ACTIVE
//! partition. It is provisioned once over USB/SWD and is never updated OTA.
//!
//! Mirrors the stock upstream RP example (`examples/boot/bootloader/rp`).

#![no_std]
#![no_main]

use core::alloc::{GlobalAlloc, Layout};
use core::cell::RefCell;

use cortex_m_rt::{entry, exception};
use defmt_rtt as _;
use embassy_boot_rp::{BootLoader, BootLoaderConfig, State, WatchdogFlash};
use embassy_sync::blocking_mutex::Mutex;
use embassy_time::Duration;

const FLASH_SIZE: usize = 2 * 1024 * 1024;

/// Fail-fast global allocator.
///
/// The app enables `embedded-tls`'s `rsa` feature, and the `rsa` crate
/// unconditionally turns on the `alloc` feature of `digest` and `signature`.
/// `embassy-boot` — shared by the app and this bootloader — depends on both
/// crates, so Cargo's workspace feature unification compiles them here with
/// `alloc`, and rustc therefore requires a global allocator for this binary.
///
/// The bootloader performs no heap allocation (`embassy-boot`'s signature
/// verification is compiled out; `_verify` is off). This allocator exists only
/// to satisfy the linker and to fault loudly if that assumption is ever broken.
struct NoAlloc;

unsafe impl GlobalAlloc for NoAlloc {
    unsafe fn alloc(&self, _layout: Layout) -> *mut u8 {
        core::ptr::null_mut()
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
}

#[global_allocator]
static NO_ALLOC: NoAlloc = NoAlloc;

#[entry]
fn main() -> ! {
    let p = embassy_rp::init(Default::default());

    let flash = WatchdogFlash::<FLASH_SIZE>::start(p.FLASH, p.WATCHDOG, Duration::from_secs(8));
    let flash = Mutex::new(RefCell::new(flash));

    let config = BootLoaderConfig::from_linkerfile_blocking(&flash, &flash, &flash);
    let active_offset = config.active.offset();
    let bl: BootLoader = BootLoader::prepare(config);

    // State::DfuDetach is the escape hatch an operator triggers via
    // `mark_dfu()`: reset into the ROM USB bootloader (BOOTSEL) instead of
    // booting a possibly-bad ACTIVE image. This path depends only on the ROM,
    // not the OTA stack, so recovery works even when the app cannot run.
    if bl.state == State::DfuDetach {
        defmt::info!("bootloader: DFU detach requested, resetting to USB boot");
        embassy_rp::rom_data::reset_to_usb_boot(0, 0);
    }

    defmt::info!(
        "bootloader: loading active image at offset 0x{:x}",
        active_offset
    );

    unsafe { bl.load(embassy_rp::flash::FLASH_BASE as u32 + active_offset) }
}

#[unsafe(no_mangle)]
#[cfg_attr(target_os = "none", unsafe(link_section = ".HardFault.user"))]
unsafe extern "C" fn HardFault() {
    cortex_m::peripheral::SCB::sys_reset();
}

#[exception]
unsafe fn DefaultHandler(_: i16) -> ! {
    const SCB_ICSR: *const u32 = 0xE000_ED04 as *const u32;
    let irqn = unsafe { core::ptr::read_volatile(SCB_ICSR) } as u8 as i16 - 16;

    panic!("DefaultHandler #{:?}", irqn);
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    cortex_m::asm::udf()
}
