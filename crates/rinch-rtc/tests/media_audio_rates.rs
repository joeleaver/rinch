//! An app-fed audio source at a rate that is not a multiple of 50 keeps up
//! (#1947's review, adopted): 11025 Hz took 220 frames per 20 ms tick
//! instead of 220.5 and fell further behind every second. 1049 Hz is the
//! extreme case (20.98 frames a tick: 47 ms behind per second without the
//! carry); 11025 and 44100 are the rates an app would use.
#![cfg(all(not(target_arch = "wasm32"), feature = "media"))]

mod common;

use std::time::{Duration, Instant};

use common::{init, pump_until};
use rinch_rtc::media::{AudioSourceOptions, LocalTrack};

#[test]
fn odd_rates_do_not_build_up_latency() {
    init();
    for rate in [48_000u32, 11_025, 44_100, 1_049] {
        let (t, w) = LocalTrack::audio_source(AudioSourceOptions {
            sample_rate: rate,
            channels: 1,
        })
        .unwrap();
        let w2 = w.clone();
        let secs = 6u64;
        let feeder = std::thread::spawn(move || {
            let start = Instant::now();
            let mut pushed = 0u64;
            for k in 1..=(secs * 100) {
                let due = start + Duration::from_millis(10 * k);
                if let Some(d) = due.checked_duration_since(Instant::now()) {
                    std::thread::sleep(d);
                }
                let target = u64::from(rate) * k / 100;
                w2.push(&vec![0.1f32; (target - pushed) as usize]);
                pushed = target;
            }
            pushed
        });
        let _ = pump_until(|| feeder.is_finished(), Duration::from_secs(secs + 5));
        let pushed = feeder.join().unwrap();
        let taken = rinch_rtc::testing::audio_samples_taken(&w);
        let backlog_ms = (pushed - taken) as f64 * 1000.0 / f64::from(rate);
        assert!(
            backlog_ms < 60.0,
            "{rate} Hz: {backlog_ms:.0} ms behind after {secs} s"
        );
        drop(t);
    }
}
