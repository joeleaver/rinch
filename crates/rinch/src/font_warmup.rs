//! The generic families resolved on a thread of their own once the app has
//! shown its first frame (#2552): see
//! [`rinch_dom::fonts::warm_generic_families`].
//!
//! Without it, the first text of a process with a glyph its font stack's
//! first families lack (a menu's `✓` or `▸`) resolved the generic list's
//! families on the UI thread, one `FcFontSort` each: 80–560 ms before a
//! panel's first menu showed. Started after the first presented frame, so
//! it neither competes with the start-up frame nor holds the shared font
//! lock while that frame needs it; once per process. An app that presents
//! no frame (a windowless one) never warms up.
//!
//! Under `RINCH_PERF` it says when it is done:
//!
//! ```text
//! [PERF] font warm-up: 249 families in 512.3 ms (look-ups 248.0 ms, longest 6.1 ms, thread CPU 241.5 ms)
//! ```
//!
//! The wall time includes a 1 ms pause after each look-up; `longest` is about
//! the longest a document's own look-up waited for the lock meanwhile.

use std::sync::atomic::{AtomicBool, Ordering};

static STARTED: AtomicBool = AtomicBool::new(false);
static DISABLED: AtomicBool = AtomicBool::new(false);

/// Whether the warm-up runs (on unless `App::font_warmup` turned it off).
pub(crate) fn set_enabled(on: bool) {
    DISABLED.store(!on, Ordering::Relaxed);
}

/// A window presented a frame: the first time, start the warm-up, with the
/// app's font defaults (this thread's, which the documents use).
pub(crate) fn window_presented() {
    if DISABLED.load(Ordering::Relaxed) || STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    let defaults = rinch_dom::fonts::font_defaults();
    let spawned = std::thread::Builder::new()
        .name("rinch-font-warmup".into())
        .spawn(move || {
            let started = std::time::Instant::now();
            let cpu = thread_cpu();
            let done = rinch_dom::fonts::warm_generic_families(&defaults);
            let line = warm_up_line(&done, started.elapsed(), thread_cpu().zip(cpu));
            if rinch_dom::perf::perf_env_enabled() {
                eprintln!("{line}");
            }
            tracing::debug!("rinch: {line}");
        });
    if let Err(e) = spawned {
        tracing::debug!("rinch: no font warm-up thread ({e})");
    }
}

/// This thread's CPU time, where the platform says it.
fn thread_cpu() -> Option<std::time::Duration> {
    #[cfg(target_os = "linux")]
    {
        let t = rustix::time::clock_gettime(rustix::time::ClockId::ThreadCPUTime);
        Some(std::time::Duration::new(t.tv_sec as u64, t.tv_nsec as u32))
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// `[PERF] font warm-up: …`; `cpu` is the thread's CPU time after and
/// before, where known.
fn warm_up_line(
    done: &rinch_dom::fonts::FontWarmUp,
    wall: std::time::Duration,
    cpu: Option<(std::time::Duration, std::time::Duration)>,
) -> String {
    let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
    let cpu = cpu.map_or(String::new(), |(after, before)| {
        format!(", thread CPU {:.1} ms", ms(after.saturating_sub(before)))
    });
    format!(
        "[PERF] font warm-up: {} families in {:.1} ms (look-ups {:.1} ms, longest {:.1} ms{cpu})",
        done.families,
        ms(wall),
        ms(done.lookups),
        ms(done.longest)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn the_warm_up_line_gives_the_families_and_the_times() {
        let done = rinch_dom::fonts::FontWarmUp {
            families: 3,
            lookups: Duration::from_micros(4_200),
            longest: Duration::from_micros(2_100),
        };
        assert_eq!(
            warm_up_line(&done, Duration::from_micros(7_300), None),
            "[PERF] font warm-up: 3 families in 7.3 ms (look-ups 4.2 ms, longest 2.1 ms)"
        );
        assert_eq!(
            warm_up_line(
                &done,
                Duration::from_micros(7_300),
                Some((Duration::from_micros(9_000), Duration::from_micros(5_000)))
            ),
            "[PERF] font warm-up: 3 families in 7.3 ms (look-ups 4.2 ms, longest 2.1 ms, thread CPU 4.0 ms)"
        );
    }
}
