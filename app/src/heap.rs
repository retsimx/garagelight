//! Static heap for the OTA TLS path.
//!
//! `embedded-tls`'s `rsa` feature (required to verify the RSA handshake
//! signature from the real endpoint's Let's Encrypt certificate) enables its
//! `alloc` feature, so the firmware must provide a `#[global_allocator]`.
//! There is no dynamic OS heap on the RP2040; this is a fixed 48 KiB region in
//! `.bss`, handed to `embedded-alloc`'s linked-list first-fit allocator.
//!
//! The heap is only ever touched by the OTA/TLS path. It must be initialised
//! once, before any allocation, from `main` — see [`init`].

use core::mem::MaybeUninit;
use core::ptr::addr_of_mut;

use embedded_alloc::LlffHeap as Heap;

/// Size of the static heap.
const HEAP_SIZE: usize = 48 * 1024;

/// The global allocator. `empty()` is uninitialised until [`init`] runs.
#[global_allocator]
static HEAP: Heap = Heap::empty();

/// Backing store for [`HEAP`]. Kept as `MaybeUninit` so it lands in `.bss`
/// rather than being materialised in `.data`.
static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];

/// Initialise the global allocator from the static backing store.
///
/// # Safety / preconditions
///
/// Must be called exactly once, before any allocation. `main` calls it first
/// thing, before `embassy_rp::init` and before the OTA task can run.
pub fn init() {
    // SAFETY: `HEAP_MEM` is a private static, and this function is documented
    // to run exactly once before any allocation. `addr_of_mut!` avoids forming
    // a reference to the `static mut`.
    unsafe {
        let start = addr_of_mut!(HEAP_MEM) as usize;
        HEAP.init(start, HEAP_SIZE);
    }
}
