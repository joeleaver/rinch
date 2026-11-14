//! Probe for issue #2652: the scale a window's first frame is painted at on
//! Wayland.
//!
//! A compositor tells a surface its scale only once the surface is mapped,
//! which takes a first buffer. rinch starts the surface at the scale it will
//! most likely get (the output's), so on a scaled output the first frame is
//! painted at that scale, and there is no second frame at another.
//!
//! The window (240x120 logical) is an xdg-toplevel, or with `PROBE_LAYER=1`
//! a layer surface anchored to the top edge. With `PROBE_HIDDEN=1` it starts
//! hidden and is shown after `PROBE_WAIT_MS` (default 500), shown and hidden
//! `PROBE_SHOWS - 1` times first (default 1 show). The probe exits
//! `PROBE_SETTLE_MS` (default 1500) after its last show. Run it with
//! `RINCH_PERF=1` and read the `[PERF] frame N` lines: each painted frame's
//! `surface_px` is its buffer's area, `240 * 120 * scale^2`. `PROBE scale S`
//! lines give the document's scale ([`rinch::window::scale_factor`]) once
//! each show has settled.
//!
//! `PROBE_OUTPUT` names the output the window asks for
//! (`WindowProps::output`), and `PROBE_WHILE_HIDDEN` is a shell command run
//! while it is hidden between shows (a test changes the output's scale with
//! it).
//!
//! The live test is `tests/first_frame_scale_2652.rs`.

use rinch::prelude::*;
use rinch_core::element::WindowProps;

#[component]
fn app() -> NodeHandle {
    rsx! {
        div { style: "width: 100%; height: 100%; background: #36a; color: white; font: 14px/20px sans-serif;",
            "first frame scale probe"
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

/// Print the document's scale, read on the main thread, and wait for it.
fn report_scale() {
    let (tx, rx) = std::sync::mpsc::channel();
    rinch::run_on_main_thread(move || {
        eprintln!("PROBE scale {}", rinch::window::scale_factor());
        let _ = tx.send(());
    });
    let _ = rx.recv();
}

fn main() {
    let hidden = flag("PROBE_HIDDEN");
    let wait = std::time::Duration::from_millis(number("PROBE_WAIT_MS", 500));
    let settle = std::time::Duration::from_millis(number("PROBE_SETTLE_MS", 1500));
    let shows = number("PROBE_SHOWS", 1).max(1);
    std::thread::spawn(move || {
        if hidden {
            for show in 1..=shows {
                std::thread::sleep(wait);
                rinch::run_on_main_thread(move || {
                    eprintln!("PROBE show {show}");
                    show_current_window();
                });
                std::thread::sleep(settle);
                report_scale();
                if show < shows {
                    rinch::run_on_main_thread(hide_current_window);
                    if let Ok(command) = std::env::var("PROBE_WHILE_HIDDEN") {
                        let status = std::process::Command::new("sh")
                            .arg("-c")
                            .arg(&command)
                            .status();
                        eprintln!("PROBE while hidden: {status:?}");
                    }
                }
            }
        } else {
            std::thread::sleep(settle);
            report_scale();
        }
        rinch::run_on_main_thread(|| std::process::exit(0));
    });
    let wayland_layer = flag("PROBE_LAYER").then(|| LayerSurface {
        anchor: LayerAnchor::TOP,
        ..Default::default()
    });
    App::new(app)
        .window_props(WindowProps {
            title: "rinch-first-frame-scale-probe".into(),
            width: 240,
            height: 120,
            visible: !hidden,
            wayland_layer,
            output: std::env::var("PROBE_OUTPUT").ok(),
            ..Default::default()
        })
        .run();
}
