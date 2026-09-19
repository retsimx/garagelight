/// One validated DHT11 reading. Integer resolution matches the sensor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sample {
    pub temperature: i8,
    pub relative_humidity: u8,
}

/// Outcome of a single hardware read attempt.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Attempt {
    Ok(Sample),
    Checksum,
    Timeout,
    Pin,
    Stalled,
}

/// Maximum hardware read attempts before a sample is abandoned.
pub const MAX_ATTEMPTS: u8 = 3;

/// A read whose measured wall-clock duration reaches this bound is treated as
/// interrupted (e.g. by the flash driver parking both cores) and discarded.
pub const READ_STALL_US: u64 = 50_000;

/// Blocking settle between hardware read attempts. The DHT11 datasheet requires
/// a sampling interval of no less than 1 s, so back-to-back retries would
/// otherwise time out before the sensor is ready again.
pub const RETRY_SETTLE_MS: u64 = 1000;

/// Sampling period handed to the real-time scheduler.
pub const SAMPLE_INTERVAL_SECS: u64 = 15;

/// Gross-corruption sanity filter, not a datasheet gate.
/// Temperature -20..=60 C (covers the DHT11's 0-50 saturation ceiling and
/// module variants specified to 60 C); relative humidity 0..=100 %.
pub fn in_range(sample: Sample) -> bool {
    sample.temperature >= -20 && sample.temperature <= 60 && sample.relative_humidity <= 100
}

/// Run `read` up to `MAX_ATTEMPTS` times. Returns the first in-range `Ok`
/// sample; returns `None` on exhaustion. Never panics and always terminates.
pub fn read_with_retries<F: FnMut() -> Attempt>(mut read: F) -> Option<Sample> {
    let mut attempt: u8 = 0;
    loop {
        attempt += 1;
        if let Attempt::Ok(sample) = read() {
            if in_range(sample) {
                return Some(sample);
            }
        }
        if attempt >= MAX_ATTEMPTS {
            return None;
        }
    }
}
