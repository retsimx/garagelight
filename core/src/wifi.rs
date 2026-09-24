//! Wi-Fi recovery policy: join timeouts, no-IP watchdog, and rejoin counter.
//!
//! Pure and host-testable (L0). The radio glue and supervisor task live in
//! `app`; nothing here touches a peripheral.

use core::time::Duration;

/// Upper bound for a single CYW43 join/leave call. A call that exceeds this
/// is treated as a normal failed attempt instead of wedging the supervisor
/// task (the hardware watchdog only sees a stalled executor, not a stalled
/// task).
pub const JOIN_TIMEOUT: Duration = Duration::from_secs(30);

/// Total time without an IP lease before a bounded MCU reset is due. This is
/// the single deliberate exception to "network loss never resets": the device
/// cannot reach the network in this state anyway, so resetting is safe.
pub const NO_IP_RESET: Duration = Duration::from_secs(180);

/// Consecutive MQTT connect failures before a Wi-Fi rejoin is requested,
/// covering the "associated but no traffic" case that link state alone never
/// reports.
pub const REJOIN_AFTER_FAILURES: u32 = 3;

/// Tracks consecutive time without an IP lease and reports when a bounded
/// reset is due. Pure: the caller feeds elapsed time; this type decides.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NoIpWatchdog {
    elapsed: Duration,
}

impl NoIpWatchdog {
    /// A freshly disarmed watchdog with no accumulated no-IP time.
    pub const fn new() -> Self {
        NoIpWatchdog {
            elapsed: Duration::ZERO,
        }
    }

    /// Accumulate time since the last successful association. The caller
    /// passes the elapsed time since the last IP lease; this type decides
    /// whether a reset is due.
    pub fn record(&mut self, elapsed: Duration) {
        self.elapsed = self.elapsed.saturating_add(elapsed);
    }

    /// Whether the accumulated no-IP time has reached the reset threshold.
    pub fn is_reset_due(&self) -> bool {
        self.elapsed >= NO_IP_RESET
    }

    /// Re-arm the watchdog after a reset or a new association.
    pub fn reset(&mut self) {
        self.elapsed = Duration::ZERO;
    }
}

impl Default for NoIpWatchdog {
    fn default() -> Self {
        Self::new()
    }
}

/// Counts consecutive MQTT connect failures and signals when a Wi-Fi rejoin
/// should be requested.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RejoinCounter {
    failures: u32,
}

impl RejoinCounter {
    /// A counter with no recorded failures.
    pub const fn new() -> Self {
        RejoinCounter { failures: 0 }
    }

    /// Record one MQTT connect failure. Returns `true` when a rejoin is now
    /// due (the failure count has reached the threshold).
    pub fn record_failure(&mut self) -> bool {
        self.failures = self.failures.saturating_add(1);
        self.failures >= REJOIN_AFTER_FAILURES
    }

    /// Reset the counter after a clean connect or after a rejoin has been
    /// requested.
    pub fn reset(&mut self) {
        self.failures = 0;
    }
}

impl Default for RejoinCounter {
    fn default() -> Self {
        Self::new()
    }
}
