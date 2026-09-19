//! OTA transport and task glue (GL-10, retsimx/garagelight#11).
//!
//! Pure policy (URL/version/hash parsing, the byte-fed HTTP head parser and the
//! streaming verified update session) lives in `garagelight_core::ota`. This
//! module owns only the hardware/network glue, split for cohesion:
//!
//! - [`transport`] — DNS, TCP (`embassy-net`), TLS 1.3 (`embedded-tls`, no-op
//!   verifier), the HTTP/1.0 GET request/response and the body adapter,
//! - [`task`] — the `ota_task` and the `check_and_update` orchestration.
//!
//! The public surface is [`TRIGGER`], [`trigger`] and [`spawn`]. `main` (or,
//! later, GL-8's reset topic) fires the trigger once; it is not a periodic poll.
//! The control path is never blocked: every network wait is a timeout-wrapped
//! await, and flash is touched only after a response head and a hash are
//! accepted.

mod task;
mod transport;

use embassy_executor::Spawner;
use embassy_net::Stack;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;

use crate::beacon::SharedControl;
use crate::update;

use task::ota_task;

/// Boot/reset trigger. `main` (or, later, GL-8's reset topic) fires it once; it
/// is not a periodic poll.
pub static TRIGGER: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Request one OTA check on the running `ota_task`.
pub fn trigger() {
    TRIGGER.signal(());
}

/// Spawn the OTA task. The task owns `updater` for the life of the program and
/// only touches flash after a response head and a hash have been accepted. It
/// shares the cyw43 control channel so a failed check can blink its stage on
/// the onboard LED when no probe or device log is available.
pub fn spawn(
    spawner: Spawner,
    stack: Stack<'static>,
    updater: update::Updater,
    control: &'static SharedControl,
) {
    spawner.spawn(ota_task(stack, updater, control).unwrap());
}

/// A fixed-capacity `core::fmt::Write` sink for requests and short names.
struct TextBuf<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> TextBuf<N> {
    fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
        }
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }

    fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

impl<const N: usize> core::fmt::Write for TextBuf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let end = self.len + s.len();
        if end > N {
            return Err(core::fmt::Error);
        }
        self.buf[self.len..end].copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}

/// Transport-level failure categories. The underlying `Error` types are
/// intentionally dropped: logging them defensively keeps secrets out and lets
/// one error type serve both plain and TLS connections.
#[derive(Clone, Copy)]
enum FetchError {
    Timeout,
    Write,
    Flush,
    Read,
    Closed,
    Head(&'static str),
    Leftover,
    BodyTooLarge,
}

impl FetchError {
    fn code(self) -> &'static str {
        match self {
            FetchError::Timeout => "timeout",
            FetchError::Write => "write",
            FetchError::Flush => "flush",
            FetchError::Read => "read",
            FetchError::Closed => "closed",
            FetchError::Head(code) => code,
            FetchError::Leftover => "leftover",
            FetchError::BodyTooLarge => "body_too_large",
        }
    }
}

/// Coarse reason an OTA check did not complete; logged as `code=`.
enum OtaError {
    Url,
    Dns,
    Connect,
    Tls,
    Timeout,
    Request,
    Auth,
    Head,
    Version,
    Sha,
    Transport(FetchError),
    Update(&'static str),
}

impl OtaError {
    fn code(&self) -> &'static str {
        match self {
            OtaError::Url => "url",
            OtaError::Dns => "dns",
            OtaError::Connect => "connect",
            OtaError::Tls => "tls",
            OtaError::Timeout => "timeout",
            OtaError::Request => "request",
            OtaError::Auth => "auth",
            OtaError::Head => "head",
            OtaError::Version => "version",
            OtaError::Sha => "sha",
            OtaError::Transport(error) => error.code(),
            OtaError::Update(code) => code,
        }
    }

    /// Number of LED blinks that report where an OTA check failed. Grouped by
    /// stage: 1 resolver, 2 reach, 3 handshake, 4 deadline, 5 request/response,
    /// 6 hash, 7 apply/flash. A human reports the count when no probe is attached.
    pub(super) fn blink_code(&self) -> u32 {
        match self {
            OtaError::Dns => 1,
            OtaError::Connect => 2,
            OtaError::Tls => 3,
            OtaError::Timeout => 4,
            OtaError::Url
            | OtaError::Request
            | OtaError::Auth
            | OtaError::Head
            | OtaError::Version => 5,
            OtaError::Sha => 6,
            OtaError::Update(code) => match *code {
                "hash_mismatch" => 7,
                "too_short" => 8,
                "too_long" => 9,
                "flash" => 10,
                "read" => 11,
                "oversize" => 12,
                _ => 7,
            },
            OtaError::Transport(error) => match error {
                FetchError::Timeout => 14,
                FetchError::Write => 15,
                FetchError::Flush => 16,
                FetchError::Read => 17,
                FetchError::Closed => 18,
                FetchError::Leftover => 19,
                FetchError::BodyTooLarge => 20,
                FetchError::Head(_) => 21,
            },
        }
    }
}
