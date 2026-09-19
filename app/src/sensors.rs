//! DHT11 sampling on core1 (GL-9, retsimx/garagelight#10).

use core::ptr::addr_of_mut;

use embassy_executor::Executor;
use embassy_rp::gpio::{Level, OutputOpenDrain};
use embassy_rp::multicore::{self, current_core, Stack};
use embassy_rp::peripherals::{CORE1, PIN_16};
use embassy_rp::Peri;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{block_for, Delay, Duration, Instant, Timer};
use static_cell::StaticCell;

use garagelight_core::sensor::{self, Attempt, Sample};

use crate::logln;

/// Latest valid sample. Single consumer: the telemetry path (GL-8).
pub static SAMPLE: Signal<CriticalSectionRawMutex, Sample> = Signal::new();

/// Await the next valid sample (GL-8 consumes this).
pub async fn wait_sample() -> Sample {
    SAMPLE.wait().await
}

/// Non-blocking check for a pending valid sample (GL-8 publishes only then).
pub fn sample_pending() -> bool {
    SAMPLE.signaled()
}

static mut CORE1_STACK: Stack<4096> = Stack::new();
static EXECUTOR1: StaticCell<Executor> = StaticCell::new();

/// Start the core1 executor and the DHT11 sampling task on GPIO16.
pub fn spawn_core1(core1: Peri<'static, CORE1>, pin: Peri<'static, PIN_16>) {
    multicore::spawn_core1(
        core1,
        unsafe { &mut *addr_of_mut!(CORE1_STACK) },
        move || {
            let executor = EXECUTOR1.init(Executor::new());
            executor.run(|spawner| {
                spawner.spawn(sampling_task(pin).unwrap());
            })
        },
    );
}

/// One blocking read attempt, classified for the pure retry policy. A read
/// reaching `READ_STALL_US` wall-clock is reported as `Stalled`.
fn read_once(delay: &mut Delay, sensor: &mut OutputOpenDrain<'static>) -> Attempt {
    let attempt_start = Instant::now();
    let result = dht_sensor::dht11::blocking::read(delay, sensor);
    let elapsed_us = attempt_start.elapsed().as_micros();
    match result {
        Ok(r) if elapsed_us < sensor::READ_STALL_US => Attempt::Ok(Sample {
            temperature: r.temperature,
            relative_humidity: r.relative_humidity,
        }),
        Ok(_) => Attempt::Stalled,
        Err(dht_sensor::DhtError::ChecksumMismatch) => Attempt::Checksum,
        Err(dht_sensor::DhtError::Timeout) => Attempt::Timeout,
        Err(dht_sensor::DhtError::PinError(_)) => Attempt::Pin,
    }
}

#[embassy_executor::task]
async fn sampling_task(pin: Peri<'static, PIN_16>) -> ! {
    let mut sensor = OutputOpenDrain::new(pin, Level::High);
    sensor.set_pullup(true);
    let mut delay = Delay;

    logln!("dht_start core={}", current_core() as u8);

    // DHT11 datasheet: let the sensor settle after power-up before the first
    // read.
    Timer::after(Duration::from_millis(1000)).await;

    loop {
        let start = Instant::now();

        let mut first_attempt = true;
        let sample = sensor::read_with_retries(|| {
            if !first_attempt {
                // Datasheet sampling interval: settle before a retry so it is
                // not rejected as a too-soon poll.
                block_for(Duration::from_millis(sensor::RETRY_SETTLE_MS));
            }
            first_attempt = false;
            read_once(&mut delay, &mut sensor)
        });

        match sample {
            Some(s) => {
                logln!(
                    "dht_sample core={} temp={} rh={}",
                    current_core() as u8,
                    s.temperature,
                    s.relative_humidity
                );
                SAMPLE.signal(s);
            }
            None => logln!("dht_sample_failed core={}", current_core() as u8),
        }

        // Absolute cadence: retries cannot drift the 15 s period.
        let next = start + Duration::from_secs(sensor::SAMPLE_INTERVAL_SECS);
        let now = Instant::now();
        if next > now {
            Timer::after(next - now).await;
        }
    }
}
