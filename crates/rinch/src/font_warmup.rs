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
//! no frame (a windowless one) never warms up then: `FontWarmup::AtStart`
//! starts it as the app starts instead, window or not.
//!
//! Under `RINCH_PERF` it says when it is done:
//!
//! ```text
//! [PERF] font warm-up: 249 families in 512.3 ms (look-ups 248.0 ms, longest 6.1 ms, thread CPU 241.5 ms)
//! ```
//!
//! The wall time includes a 1 ms pause after each look-up; `longest` is about
//! the longest a document's own look-up waited for the lock meanwhile.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use crate::font::FontWarmup;

static STARTED: AtomicBool = AtomicBool::new(false);
/// The app's [`FontWarmup`]: 0 after the first frame, 1 at start, 2 off.
static WHEN: AtomicU8 = AtomicU8::new(0);

/// The app is starting (`App::run`, after its font defaults are set): with
/// [`FontWarmup::AtStart`] the warm-up starts now, window or not; otherwise
/// the choice is kept for [`window_presented`].
pub(crate) fn app_started(when: FontWarmup) -> Option<std::thread::JoinHandle<()>> {
    WHEN.store(
        match when {
            FontWarmup::AfterFirstFrame => 0,
            FontWarmup::AtStart => 1,
            FontWarmup::Off => 2,
        },
        Ordering::Relaxed,
    );
    (when == FontWarmup::AtStart).then(start).flatten()
}

/// A window presented a frame: the first time, with
/// [`FontWarmup::AfterFirstFrame`], start the warm-up.
pub(crate) fn window_presented() {
    if WHEN.load(Ordering::Relaxed) == 0 {
        let _ = start();
    }
}

/// Start the warm-up once per process, with the app's font defaults (this
/// thread's, which the documents use). At start no document may have made
/// the font scan yet: the thread makes it then, as the first document would.
fn start() -> Option<std::thread::JoinHandle<()>> {
    if STARTED.swap(true, Ordering::Relaxed) {
        return None;
    }
    let defaults = rinch_dom::fonts::font_defaults();
    let spawned = std::thread::Builder::new()
        .name("rinch-font-warmup".into())
        .spawn(move || {
            let started = std::time::Instant::now();
            let cpu = thread_cpu();
            if rinch_dom::fonts::system_font_scans() == 0 {
                drop(rinch_dom::fonts::new_font_context());
            }
            let done = rinch_dom::fonts::warm_generic_families(&defaults);
            let line = warm_up_line(&done, started.elapsed(), thread_cpu().zip(cpu));
            if rinch_dom::perf::perf_env_enabled() {
                eprintln!("{line}");
            }
            #[cfg(test)]
            {
                *LAST.lock().unwrap() = Some(done);
            }
            tracing::debug!("rinch: {line}");
        });
    match spawned {
        Ok(handle) => Some(handle),
        Err(e) => {
            tracing::debug!("rinch: no font warm-up thread ({e})");
            None
        }
    }
}

/// The last warm-up's result, for the tests.
#[cfg(test)]
static LAST: std::sync::Mutex<Option<rinch_dom::fonts::FontWarmUp>> = std::sync::Mutex::new(None);

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

    /// #2552: with `FontWarmup::AtStart` the warm-up runs as the app starts,
    /// with no window and no frame presented (a windowless app, a hidden
    /// launcher), makes the font scan itself, and looks the families up;
    /// once per process, so a later first frame starts none.
    #[test]
    fn at_start_the_warm_up_runs_with_no_window_2552() {
        STARTED.store(false, Ordering::Relaxed);
        let handle = app_started(FontWarmup::AtStart).expect("started at once");
        handle.join().unwrap();
        WHEN.store(0, Ordering::Relaxed);
        let done = LAST.lock().unwrap().expect("it finished");
        assert!(done.families > 0, "{done:?}");
        assert!(rinch_dom::fonts::system_font_scans() >= 1);
        window_presented();
        assert!(app_started(FontWarmup::AtStart).is_none(), "once");
    }

    /// #3016: a process with the warm-up thread runs under Valgrind. This
    /// test binary runs [`at_start_the_warm_up_runs_with_no_window_2552`]
    /// again under Callgrind and under Memcheck, which must pass it.
    ///
    /// The thread's first `thread_cpu` was rustix's first `clock_gettime`,
    /// which looks the vDSO up in the aux vector. rustix read the vector
    /// with `prctl(PR_GET_AUXV)`, which Valgrind passes to the kernel, and
    /// the kernel's vector names the host's vDSO, which the guest has not
    /// mapped: SIGSEGV in `check_elf_base` (deskrinch's Callgrind run
    /// failed an assertion in its signal code instead). rinch's rustix now
    /// has `use-libc-auxv`, so it reads the vector Valgrind gave the
    /// process through `getauxval`.
    ///
    /// Valgrind is `$RINCH_VALGRIND` or `valgrind` on `PATH`. Without it the
    /// test says so and passes, unless `RINCH_REQUIRE_VALGRIND` is set and
    /// not empty (CI's `test (unit)` shard sets it and installs valgrind),
    /// when it fails.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_warm_up_runs_under_valgrind_3016() {
        use std::process::Command;
        let valgrind = std::env::var_os("RINCH_VALGRIND").unwrap_or_else(|| "valgrind".into());
        let found = Command::new(&valgrind)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success());
        if !found {
            assert!(
                std::env::var_os("RINCH_REQUIRE_VALGRIND").is_none_or(|v| v.is_empty()),
                "RINCH_REQUIRE_VALGRIND is set and there is no {valgrind:?}"
            );
            eprintln!("the_warm_up_runs_under_valgrind_3016: no {valgrind:?}, not run");
            return;
        }
        let exe = std::env::current_exe().unwrap();
        let test = "font_warmup::tests::at_start_the_warm_up_runs_with_no_window_2552";
        for tool in [
            ["--tool=callgrind", "--callgrind-out-file=/dev/null"],
            ["--tool=memcheck", "--leak-check=no"],
        ] {
            // A crash leaves a `vgcore.<pid>` in the working directory.
            let out = Command::new(&valgrind)
                .current_dir(std::env::temp_dir())
                .args(tool)
                .args(["--num-callers=30", "--quiet", "--"])
                .arg(&exe)
                .args([test, "--exact", "--test-threads=1"])
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            // Valgrind's report ends with every thread's stack: its head
            // names the fault, so keep the first lines.
            let stderr: String = stderr.lines().take(60).collect::<Vec<_>>().join("\n");
            assert!(
                out.status.success() && stdout.contains("test result: ok. 1 passed"),
                "{}: {}\n--- stdout\n{stdout}\n--- stderr\n{stderr}",
                tool[0],
                out.status
            );
        }
    }
}
