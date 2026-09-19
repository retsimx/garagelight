//! App-side `embassy-boot` firmware-update wrapper (GL-3, retsimx/garagelight#4).
//!
//! This module owns the single hardware `WATCHDOG` so the periodic feed task in
//! `main` and the DFU flash adapter below feed the *same* watchdog. Every erase
//! and write reloads the watchdog, so a long update cannot trip a reset.
//!
//! The `embassy-rp` flash driver parks both cores itself; this module performs
//! no app-side core parking and leaves the driver's default XIP protection on.

use core::cell::RefCell;

use embassy_boot::{
    AlignedBuffer, FirmwareUpdater, FirmwareUpdaterConfig, FirmwareUpdaterError, State,
};
use embassy_embedded_hal::flash::partition::Partition;
use embassy_rp::flash::{Blocking, Flash};
use embassy_rp::peripherals::FLASH;
use embassy_rp::watchdog::Watchdog;
use embassy_rp::Peri;
use embassy_sync::blocking_mutex::raw::{CriticalSectionRawMutex, NoopRawMutex};
use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::Duration;
use embedded_storage::nor_flash::ErrorType;
use embedded_storage_async::nor_flash::{NorFlash, ReadNorFlash};
use static_cell::StaticCell;

use crate::logln;

/// RP2040 flash size handed to the driver (2 MiB).
pub const FLASH_SIZE: usize = 2 * 1024 * 1024;

/// Watchdog reload period. `main` arms the watchdog with the same value, and
/// every flash operation feeds it, so the two stay coupled.
pub const WATCHDOG_TIMEOUT: Duration = Duration::from_secs(8);

/// Async `NorFlash` write granularity: RP2040 flash writes byte-by-byte.
const DFU_WRITE_SIZE: usize = 1;

/// Async `NorFlash` read granularity: 4 bytes, matching the RP2040 flash
/// driver. The DFU/STATE partitions are erase-page aligned, so this only sizes
/// the embassy-boot magic buffer.
const DFU_READ_SIZE: usize = 4;

/// `AlignedBuffer` length required by embassy-boot:
/// `STATE::WRITE_SIZE.max(STATE::READ_SIZE)`.
const ALIGNED_LEN: usize = if DFU_READ_SIZE > DFU_WRITE_SIZE {
    DFU_READ_SIZE
} else {
    DFU_WRITE_SIZE
};

static WATCHDOG: BlockingMutex<CriticalSectionRawMutex, RefCell<Option<Watchdog>>> =
    BlockingMutex::new(RefCell::new(None));

/// Install the single `WATCHDOG` peripheral. Call once, early in `main`.
pub fn init_watchdog(wd: Watchdog) {
    WATCHDOG.lock(|cell| *cell.borrow_mut() = Some(wd));
}

/// Feed the shared watchdog if it has been installed.
pub fn feed(timeout: Duration) {
    WATCHDOG.lock(|cell| {
        if let Some(wd) = cell.borrow_mut().as_mut() {
            wd.feed(timeout);
        }
    });
}

/// Async flash adapter over the RP2040 `Flash` driver that feeds the shared
/// watchdog on every erase, write and read.
pub struct DfuFlash {
    flash: Flash<'static, FLASH, Blocking, FLASH_SIZE>,
}

impl DfuFlash {
    /// Create the adapter from the `FLASH` peripheral.
    pub fn new(flash: Peri<'static, FLASH>) -> Self {
        Self {
            flash: Flash::new_blocking(flash),
        }
    }
}

impl ErrorType for DfuFlash {
    type Error = embassy_rp::flash::Error;
}

impl ReadNorFlash for DfuFlash {
    const READ_SIZE: usize = DFU_READ_SIZE;

    async fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        feed(WATCHDOG_TIMEOUT);
        self.flash.blocking_read(offset, bytes)
    }

    fn capacity(&self) -> usize {
        self.flash.capacity()
    }
}

impl NorFlash for DfuFlash {
    const WRITE_SIZE: usize = DFU_WRITE_SIZE;
    const ERASE_SIZE: usize = embassy_rp::flash::ERASE_SIZE;

    async fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        let page = embassy_rp::flash::ERASE_SIZE as u32;
        let mut addr = from;
        while addr < to {
            feed(WATCHDOG_TIMEOUT);
            let end = core::cmp::min(addr + page, to);
            self.flash.blocking_erase(addr, end)?;
            addr = end;
        }
        Ok(())
    }

    async fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        feed(WATCHDOG_TIMEOUT);
        self.flash.blocking_write(offset, bytes)
    }
}

/// DFU partition produced by `FirmwareUpdaterConfig::from_linkerfile`. It
/// implements the async `NorFlash` trait, so callers stream chunks with
/// `.write(offset, data).await`.
pub type DfuPartition = Partition<'static, NoopRawMutex, DfuFlash>;

static DFU_FLASH: StaticCell<Mutex<NoopRawMutex, DfuFlash>> = StaticCell::new();
static ALIGNED: StaticCell<AlignedBuffer<ALIGNED_LEN>> = StaticCell::new();

/// Thin wrapper over `embassy_boot::FirmwareUpdater`.
pub struct Updater {
    inner: FirmwareUpdater<'static, DfuPartition, DfuPartition>,
}

impl Updater {
    /// Build the updater, taking ownership of the `FLASH` peripheral.
    ///
    /// # Panics
    ///
    /// Panics if called more than once: the flash mutex and the embassy-boot
    /// magic buffer are single `StaticCell`s (the peripheral is a singleton).
    pub fn new(flash: Peri<'static, FLASH>) -> Self {
        let flash: &'static Mutex<NoopRawMutex, DfuFlash> =
            DFU_FLASH.init(Mutex::new(DfuFlash::new(flash)));
        let config = FirmwareUpdaterConfig::from_linkerfile(flash, flash);
        let aligned = ALIGNED.init(AlignedBuffer([0; ALIGNED_LEN]));
        Self {
            inner: FirmwareUpdater::new(config, &mut aligned.0),
        }
    }

    /// Current boot state; check after a swap before calling
    /// [`Updater::mark_booted`].
    pub async fn get_state(&mut self) -> Result<State, FirmwareUpdaterError> {
        self.inner.get_state().await
    }

    /// Erase the whole DFU partition and return it for a chunked write.
    pub async fn prepare_update(&mut self) -> Result<&mut DfuPartition, FirmwareUpdaterError> {
        self.inner.prepare_update().await
    }

    /// Write a firmware chunk at `offset` within the DFU partition.
    pub async fn write_firmware(
        &mut self,
        offset: usize,
        data: &[u8],
    ) -> Result<(), FirmwareUpdaterError> {
        self.inner.write_firmware(offset, data).await
    }

    /// Schedule a swap on the next reset.
    pub async fn mark_updated(&mut self) -> Result<(), FirmwareUpdaterError> {
        self.inner.mark_updated().await
    }

    /// Confirm the running image and cancel rollback.
    pub async fn mark_booted(&mut self) -> Result<(), FirmwareUpdaterError> {
        self.inner.mark_booted().await
    }

    /// Request USB DFU mode on the next reset (documented recovery escape).
    pub async fn mark_dfu(&mut self) -> Result<(), FirmwareUpdaterError> {
        self.inner.mark_dfu().await
    }
}

/// Stub entry point for an OTA check. GL-10/GL-11 replace the body with
/// `ota::trigger()`; the `garagelight/reset` subscription calls this.
pub fn request_check() {
    logln!("ota_check_requested");
}
