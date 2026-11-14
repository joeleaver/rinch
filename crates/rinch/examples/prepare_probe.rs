//! Probe for issue #2633: what the first show of a window started hidden
//! costs, with and without `prepare_current_window`.
//!
//! The window is a launcher of 640x420 (a field and five rows of an icon and
//! a name, the first highlighted) started hidden. With `PROBE_PREPARE=1` it
//! calls `prepare_current_window()` once mounted. After `PROBE_WAIT_MS`
//! (default 1500) it is shown `PROBE_SHOWS` times (default 1; each show but
//! the last is followed by a hide), and the last show is measured: from the
//! main-thread turn that asks for it to the turn after its first presented
//! frame on the GPU (`on_frame_composited`), or until `PROBE_SETTLE_MS`
//! (default 1500) later in software, which takes in its first presented
//! frame (with `PROBE_SETTLE_ONLY=1` on the GPU too: on Wayland the first
//! frame of a show was painted at the window's provisional scale and the next
//! at the output's, until #2652 started the surface at the output's scale).
//! Then it exits. Under Valgrind a cold GPU preparation takes
//! minutes of real time: give it a `PROBE_WAIT_MS` long enough, or the
//! preparation's work is counted in the show.
//!
//! Instructions, under Callgrind, all threads:
//!
//! ```text
//! valgrind --tool=callgrind --instr-atstart=no \
//!     target/release/examples/prepare_probe
//! ```
//!
//! The probe turns Callgrind's instrumentation on at the start of the
//! measured show and off at its end (client requests, x86_64; a no-op
//! outside Valgrind). It prints its resident set size (`VmRSS`) before the
//! measured show: with the software adapter (lavapipe) a GPU renderer's
//! memory is in it.

use std::sync::atomic::{AtomicBool, Ordering};

use rinch::prelude::*;
use rinch_core::element::WindowProps;

const ICON: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAFElEQVR4nGP4z8DQwMDw/wQDjAEAOAQGjfQ5xpIAAAAASUVORK5CYII=";

const CSS: &str = "
    body { margin: 0; font-family: sans-serif; font-size: 14px; line-height: 20px; }
    .field { height: 32px; margin: 8px; border: 1px solid #888; }
    .row { height: 40px; display: flex; align-items: center; padding: 0 8px; }
    .row.on { background: linear-gradient(rgb(40, 80, 200), rgb(20, 40, 120)); color: white; }
    .row img { width: 32px; height: 32px; margin-right: 8px; }
";

const ROWS: [&str; 5] = [
    "Files",
    "Terminal",
    "Web Browser",
    "Mail",
    "System Settings",
];

/// The measured show is under way: its first presented frame ends it.
static MEASURING: AtomicBool = AtomicBool::new(false);

#[component]
fn launcher() -> NodeHandle {
    if flag("PROBE_PREPARE") {
        prepare_current_window();
    }
    // On the GPU the measured show ends at the turn after its first
    // presented frame; the software renderer has no such hook, and ends
    // `PROBE_SETTLE_MS` after the show.
    #[cfg(feature = "gpu")]
    let _ = rinch::window::on_frame_composited(|_| {
        if !flag("PROBE_SETTLE_ONLY") && MEASURING.swap(false, Ordering::SeqCst) {
            on_main(stop);
        }
    });
    rsx! {
        div {
            style { {CSS} }
            div { class: "field", "Search applications" }
            for (i, name) in ROWS.into_iter().enumerate() {
                div { key: i, class: if i == 0 { "row on" } else { "row" },
                    img { src: ICON }
                    {name.to_string()}
                }
            }
        }
    }
}

fn flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| v == "1")
}

fn number(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// A Callgrind client request (`valgrind/callgrind.h`); a no-op outside
/// Valgrind.
#[cfg(target_arch = "x86_64")]
fn callgrind(request: u64) {
    let args: [u64; 6] = [request, 0, 0, 0, 0, 0];
    // SAFETY: Valgrind's "special instruction" preamble: four rotations of
    // rdi by 128 bits in all (no change) and `xchg rbx, rbx` (no change).
    unsafe {
        std::arch::asm!(
            "rol rdi, 3", "rol rdi, 13", "rol rdi, 61", "rol rdi, 51",
            "xchg rbx, rbx",
            inout("rdx") 0u64 => _,
            inout("rdi") 0u64 => _,
            in("rax") args.as_ptr(),
            options(nostack),
        );
    }
}
#[cfg(not(target_arch = "x86_64"))]
fn callgrind(_request: u64) {}

/// `('C' << 24) | ('T' << 16)`: Callgrind's requests.
const START_INSTRUMENTATION: u64 = 0x4354_0004;
const STOP_INSTRUMENTATION: u64 = 0x4354_0005;

fn rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
        })
        .unwrap_or(0)
}

fn on_main(f: impl FnOnce() + Send + 'static) {
    run_on_main_thread(f);
}

/// End the measurement and the probe.
fn stop() {
    callgrind(STOP_INSTRUMENTATION);
    eprintln!("probe: rss after the measured show: {} KiB", rss_kib());
    std::process::exit(0);
}

fn main() {
    let wait = std::time::Duration::from_millis(number("PROBE_WAIT_MS", 1500));
    let settle = std::time::Duration::from_millis(number("PROBE_SETTLE_MS", 1500));
    let shows = number("PROBE_SHOWS", 1).max(1);
    std::thread::spawn(move || {
        std::thread::sleep(wait);
        for _ in 1..shows {
            on_main(show_current_window);
            std::thread::sleep(settle);
            on_main(hide_current_window);
            std::thread::sleep(wait);
        }
        on_main(|| {
            eprintln!("probe: rss before the measured show: {} KiB", rss_kib());
            MEASURING.store(true, Ordering::SeqCst);
            callgrind(START_INSTRUMENTATION);
            show_current_window();
        });
        std::thread::sleep(settle);
        on_main(stop);
    });
    App::new(launcher)
        .window_props(WindowProps {
            title: "rinch-prepare-probe".into(),
            width: 640,
            height: 420,
            visible: false,
            ..Default::default()
        })
        .run();
}
