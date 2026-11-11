//! Probe for issue #2464: what an app asks of a layer surface in one turn of
//! the event loop reaches the compositor in one commit, whatever order it
//! asked in.
//!
//! Driven by `tests/layer_one_commit_2464.rs` over stdin, one command per
//! line. Each change command makes its calls one after another inside one
//! `run_on_main_thread` closure, so they are one turn's requests:
//!
//! - `size-then-layer` / `layer-then-size`: the panel's move, a new height
//!   with a new edge and exclusive zone (`set_current_window_size` and
//!   `set_current_window_layer`), in that order;
//! - `size-then-position` / `position-then-size`: an overlay's move and
//!   resize (`set_current_window_size` and `set_current_window_position`);
//! - `quit`.
//!
//! Before and after a change it prints `PROBE-BEGIN <command>` and, a
//! second later, `PROBE-END <command>` to stderr, from the UI thread, so
//! the lines sit in order among `WAYLAND_DEBUG`'s.
//!
//! Environment: `PROBE_OVERLAY=1` starts it as a non-activating overlay
//! (a layer surface the app positions, #1523); otherwise it is a panel, the
//! app's own layer surface (`WindowProps::wayland_layer`, #2413).

use std::io::BufRead;

use rinch::prelude::*;
use rinch_core::element::WindowProps;

#[component]
fn app() -> NodeHandle {
    rsx! { div { style: "width: 100%; height: 100%; background: #36a;", "layer commit probe" } }
}

fn panel(edge: LayerAnchor, zone: u32) -> LayerSurface {
    LayerSurface {
        anchor: LayerAnchor {
            left: true,
            right: true,
            ..edge
        },
        exclusive_zone: LayerExclusiveZone::Reserve(zone),
        ..Default::default()
    }
}

/// One turn's requests, then the markers around them.
fn turn(name: String, calls: impl FnOnce() + Send + 'static) {
    rinch::run_on_main_thread(move || {
        eprintln!("PROBE-BEGIN {name}");
        calls();
        set_timeout(1000, move || eprintln!("PROBE-END {name}"));
    });
}

fn main() {
    let overlay = std::env::var("PROBE_OVERLAY").is_ok_and(|v| v == "1");
    std::thread::spawn(|| {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            let name = line.trim().to_string();
            match name.as_str() {
                "size-then-layer" => turn(name, || {
                    set_current_window_size(800.0, 60.0);
                    set_current_window_layer(panel(LayerAnchor::TOP, 60));
                }),
                "layer-then-size" => turn(name, || {
                    set_current_window_layer(panel(LayerAnchor::TOP, 60));
                    set_current_window_size(800.0, 60.0);
                }),
                "size-then-position" => turn(name, || {
                    set_current_window_size(300.0, 120.0);
                    set_current_window_position(200.0, 150.0);
                }),
                "position-then-size" => turn(name, || {
                    set_current_window_position(200.0, 150.0);
                    set_current_window_size(300.0, 120.0);
                }),
                "quit" => std::process::exit(0),
                _ => {}
            }
        }
    });

    let props = if overlay {
        WindowProps {
            title: "rinch-layer-commit-probe".into(),
            width: 240,
            height: 90,
            x: Some(40),
            y: Some(30),
            non_activating: true,
            skip_taskbar: true,
            ..Default::default()
        }
    } else {
        WindowProps {
            title: "rinch-layer-commit-probe".into(),
            width: 800,
            height: 32,
            wayland_layer: Some(panel(LayerAnchor::BOTTOM, 32)),
            ..Default::default()
        }
    };
    App::new(app).window_props(props).run();
}
