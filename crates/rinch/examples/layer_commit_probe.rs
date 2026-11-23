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
//! - `rename-and-zone`, `rename-again`: the panel given another namespace
//!   (`desktop`, then `notification`) with another exclusive zone, which a
//!   live surface keeps and takes (#2821);
//! - `rename-and-output`: another namespace (`tooltip`) with another output,
//!   which makes the surface again;
//! - `hide-rename-show`: the panel hidden, given the namespace `desktop`,
//!   and shown;
//! - `content-margin`: the content-sized panel (`PROBE_CONTENT=1`) given a
//!   350 px left margin, which narrows its content and so changes its size
//!   (#2829);
//! - `quit`.
//!
//! Before and after a change it prints `PROBE-BEGIN <command>` and, a
//! second later, `PROBE-END <command>` to stderr, from the UI thread, so
//! the lines sit in order among `WAYLAND_DEBUG`'s.
//!
//! Environment: `PROBE_OVERLAY=1` starts it as a non-activating overlay
//! (a layer surface the app positions, #1523); otherwise it is a panel, the
//! app's own layer surface (`WindowProps::wayland_layer`, #2413), named
//! `PROBE_NAMESPACE` when that is set (#2821). `PROBE_APP_ID` sets
//! `WindowProps::app_id`. `PROBE_CONTENT=1` starts it as a panel sized by
//! its content (#2552) at the top-left corner: a wrapping row of 24 boxes,
//! 100 x 20 each, as many to a row as the output's width less the left
//! margin holds.

use std::io::BufRead;

use rinch::prelude::*;
use rinch_core::element::{ContentSizing, WindowProps};

#[component]
fn app() -> NodeHandle {
    rsx! { div { style: "width: 100%; height: 100%; background: #36a;", "layer commit probe" } }
}

/// The content-sized panel's content: 24 boxes of 100 x 20 in a wrapping
/// row.
#[component]
fn boxes() -> NodeHandle {
    rsx! {
        div { style: "display: flex; flex-wrap: wrap; background: #36a;",
            for i in 0..24u32 {
                div { key: i, style: "width: 100px; height: 20px; flex: none;" }
            }
        }
    }
}

/// The content-sized panel: top-left, `left` px from the left edge.
fn corner(left: i32) -> LayerSurface {
    LayerSurface {
        anchor: LayerAnchor {
            top: true,
            left: true,
            ..LayerAnchor::NONE
        },
        margins: LayerMargins {
            left,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// The panel at `edge`, reserving `zone`, named `PROBE_NAMESPACE`.
fn panel(edge: LayerAnchor, zone: u32) -> LayerSurface {
    LayerSurface {
        anchor: LayerAnchor {
            left: true,
            right: true,
            ..edge
        },
        exclusive_zone: LayerExclusiveZone::Reserve(zone),
        namespace: std::env::var("PROBE_NAMESPACE").ok(),
        ..Default::default()
    }
}

/// The panel at the bottom, reserving `zone`, named `namespace`.
fn renamed(namespace: &str, zone: u32) -> LayerSurface {
    LayerSurface {
        namespace: Some(namespace.into()),
        ..panel(LayerAnchor::BOTTOM, zone)
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
                "rename-and-zone" => turn(name, || {
                    set_current_window_layer(renamed("desktop", 48));
                }),
                "rename-again" => turn(name, || {
                    set_current_window_layer(renamed("notification", 56));
                }),
                "rename-and-output" => turn(name, || {
                    set_current_window_layer(LayerSurface {
                        output: Some("PROBE-NOWHERE".into()),
                        ..renamed("tooltip", 56)
                    });
                }),
                "hide-rename-show" => turn(name, || {
                    hide_current_window();
                    set_current_window_layer(renamed("desktop", 32));
                    show_current_window();
                }),
                "content-margin" => turn(name, || set_current_window_layer(corner(350))),
                "quit" => std::process::exit(0),
                _ => {}
            }
        }
    });

    if std::env::var("PROBE_CONTENT").is_ok_and(|v| v == "1") {
        let props = WindowProps {
            title: "rinch-layer-commit-probe".into(),
            size_to_content: Some(ContentSizing::new()),
            wayland_layer: Some(corner(0)),
            ..Default::default()
        };
        App::new(boxes).window_props(props).run();
        return;
    }
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
            app_id: std::env::var("PROBE_APP_ID").ok(),
            ..Default::default()
        }
    };
    App::new(app).window_props(props).run();
}
