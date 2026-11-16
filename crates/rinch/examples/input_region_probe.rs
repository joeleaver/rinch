//! Probe for issue #2741 (window input regions), driven by
//! `tests/input_region_2741.rs` on a headless sway in an isolated session.
//!
//! - `MODE=under`: an ordinary window (title `under`, 800x600) that prints
//!   `UNDER CLICK <x> <y>` for every press it gets.
//! - `MODE=overlay`: a transparent, content-sized layer surface 100px from
//!   the output's top-left holding a column of 200x60 popups with a 20px gap
//!   between them and rounded corners, the deskrinch notification column. It
//!   prints `POPUP CLICK <n>` for a press on popup `n` and `OVERLAY CLICK` for
//!   one on the column itself. `REGION=all` (the default) or `painted` picks
//!   its `InputRegion`. Over stdin: `grow` shows a third popup (the column
//!   grows by 80px), `through` turns click-through on (no frame follows),
//!   `quit` exits. Each command is bracketed by
//!   `PROBE-BEGIN <cmd>` / `PROBE-END <cmd>` on stderr, a second apart, so
//!   the lines sit among `WAYLAND_DEBUG`'s.
//!
//! Both print `READY` on stderr once they have been up for a second.

use std::io::BufRead;

use rinch::prelude::*;
use rinch_core::element::{InputRegion, WindowProps};

#[component]
fn under() -> NodeHandle {
    set_timeout(1000, || eprintln!("READY"));
    rsx! {
        div {
            style: "width: 100%; height: 100%; background: #ddd",
            onmousedown: move || println!("UNDER CLICK"),
            "under"
        }
    }
}

#[component]
fn overlay() -> NodeHandle {
    let popups = Signal::new(2usize);
    set_timeout(1000, || eprintln!("READY"));
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            let cmd = line.trim().to_string();
            rinch::run_on_main_thread(move || {
                eprintln!("PROBE-BEGIN {cmd}");
                match cmd.as_str() {
                    "grow" => popups.set(3),
                    "through" => set_click_through_current_window(true),
                    "quit" => exit(),
                    _ => {}
                }
                set_timeout(1000, move || eprintln!("PROBE-END {cmd}"));
            });
        }
    });
    rsx! {
        div {
            style: "display: flex; flex-direction: column; gap: 20px; width: 200px; margin: 0",
            onmousedown: move || println!("OVERLAY CLICK"),
            for n in (0..3usize).collect::<Vec<_>>() {
                div {
                    key: n,
                    style: {move || format!(
                        "height: 60px; border-radius: 10px; background: #345; display: {}",
                        if n < popups.get() { "block" } else { "none" }
                    )},
                    onmousedown: move || println!("POPUP CLICK {n}"),
                }
            }
        }
    }
}

fn main() {
    match std::env::var("MODE").as_deref() {
        Ok("under") => App::new(under)
            .window_props(WindowProps {
                title: "under".into(),
                width: 800,
                height: 600,
                ..Default::default()
            })
            .run(),
        _ => {
            let region = match std::env::var("REGION").as_deref() {
                Ok("painted") => InputRegion::Painted,
                _ => InputRegion::All,
            };
            App::new(overlay)
                .window_props(WindowProps {
                    title: "overlay".into(),
                    transparent: true,
                    input_region: region,
                    size_to_content: Some(ContentSizing::new()),
                    wayland_layer: Some(LayerSurface {
                        anchor: LayerAnchor::TOP | LayerAnchor::LEFT,
                        margins: LayerMargins {
                            top: 100,
                            left: 100,
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .run()
        }
    }
}
