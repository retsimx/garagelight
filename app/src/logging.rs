//! Dual-output logging: every message goes to defmt (RTT) and to UART0
//! (GP0 TX / GP1 RX) at 115200 baud.
//!
//! Only integer/string formatting is used; RP2040 has no FPU, so no float
//! formatting appears anywhere in the log path.

use core::cell::RefCell;
use core::fmt::{self, Write};

use embassy_rp::uart::{Blocking, UartTx};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::blocking_mutex::Mutex;

/// Blocking UART0 sink behind a `core::fmt::Write` implementation.
pub struct Logger {
    tx: UartTx<'static, Blocking>,
}

impl Logger {
    fn new(tx: UartTx<'static, Blocking>) -> Self {
        Self { tx }
    }
}

impl Write for Logger {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.tx.blocking_write(s.as_bytes()).map_err(|_| fmt::Error)
    }
}

static LOGGER: Mutex<CriticalSectionRawMutex, RefCell<Option<Logger>>> =
    Mutex::new(RefCell::new(None));

/// Install the UART0 writer produced by `main`.
pub fn init(tx: UartTx<'static, Blocking>) {
    LOGGER.lock(|cell| *cell.borrow_mut() = Some(Logger::new(tx)));
}

/// Write a formatted message to UART0 (no trailing newline).
pub fn emit(args: fmt::Arguments<'_>) {
    LOGGER.lock(|cell| {
        if let Some(logger) = cell.borrow_mut().as_mut() {
            let _ = logger.write_fmt(args);
        }
    });
}

/// Log to defmt and UART0 (no trailing newline on UART).
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {{
        ::defmt::info!($($arg)*);
        $crate::logging::emit(format_args!($($arg)*));
    }};
}

/// Log to defmt and UART0, terminating the UART line with CRLF.
#[macro_export]
macro_rules! logln {
    ($($arg:tt)*) => {{
        ::defmt::info!($($arg)*);
        $crate::logging::emit(format_args!($($arg)*));
        $crate::logging::emit(format_args!("\r\n"));
    }};
}
