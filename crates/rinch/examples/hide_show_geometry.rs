//! Probe for issue #1546: a window hidden and shown again comes back where it
//! was and at the size it had.
//!
//! Driven by `tests/hide_show_geometry.rs` over stdin, one command per line:
//! `hide`, `show`, `move X Y`, `size W H`, `geom` (prints
//! `geom <x> <y> <w> <h>` from `current_window_position`/`_size`, `none` for
//! an absent value) and `quit`. The test moves and resizes the window from
//! outside (`xdotool`), as a user dragging it would.
//!
//! Environment: `PROBE_NON_ACTIVATING=1` makes it a non-activating,
//! skip-taskbar window (#1519); `PROBE_HIDDEN=1` starts it hidden (#1520).

use std::io::{BufRead, Write};

use rinch::prelude::*;
use rinch_core::element::WindowProps;

#[component]
fn app() -> NodeHandle {
    rsx! { div { style: "width: 100%; height: 100%; background: #3a6;", "geometry probe" } }
}

fn flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| v == "1")
}

fn main() {
    std::thread::spawn(|| {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            let words: Vec<&str> = line.split_whitespace().collect();
            let num = |i: usize| words.get(i).and_then(|w| w.parse::<f64>().ok());
            match words.first().copied() {
                Some("hide") => hide_current_window(),
                Some("show") => show_current_window(),
                Some("max") => toggle_maximize_current_window(),
                Some("min") => minimize_current_window(),
                Some("move") => {
                    if let (Some(x), Some(y)) = (num(1), num(2)) {
                        set_current_window_position(x, y);
                    }
                }
                Some("size") => {
                    if let (Some(w), Some(h)) = (num(1), num(2)) {
                        set_current_window_size(w, h);
                    }
                }
                Some("geom") => {
                    let pos = current_window_position()
                        .map_or("none none".to_string(), |(x, y)| format!("{x} {y}"));
                    let size = current_window_size()
                        .map_or("none none".to_string(), |(w, h)| format!("{w} {h}"));
                    println!("geom {pos} {size}");
                    let _ = std::io::stdout().flush();
                }
                Some("quit") => std::process::exit(0),
                _ => {}
            }
        }
    });

    let props = WindowProps {
        title: "rinch-geom-probe".into(),
        width: 560,
        height: 280,
        x: Some(220),
        y: Some(224),
        visible: !flag("PROBE_HIDDEN"),
        non_activating: flag("PROBE_NON_ACTIVATING"),
        skip_taskbar: flag("PROBE_NON_ACTIVATING"),
        ..Default::default()
    };
    App::new(app).window_props(props).run();
}
