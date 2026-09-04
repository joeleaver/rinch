//! Probe app for `tests/debug_event_latency.rs` (issue #153).
//!
//! Renders a full-window click target that records the interval between
//! consecutive clicks into a `.click-delta` text node. The companion ignored
//! test drives two back-to-back `click` commands through the raw rinch-debug
//! TCP protocol and asserts the app-observed delta stays far below one paint.
//!
//! Every click toggles the root background color, dirtying the whole window —
//! otherwise dirty-region caching would repaint only the tiny changed text and
//! the paint the second click stalls behind would be too cheap to observe. The
//! text-heavy filler rows make that full-window paint genuinely expensive in a
//! debug software build.
//!
//! Build with: `cargo build -p rinch --example debug_click_latency --features debug`

use rinch::prelude::*;
use std::time::Instant;

#[component]
fn app() -> NodeHandle {
    let last_click = Signal::new(None::<Instant>);
    let delta_ms = Signal::new(String::from("none"));
    let count = Signal::new(0u32);

    rsx! {
        div {
            style: {move || format!(
                "display: flex; flex-direction: column; width: 900px; height: 600px; \
                 padding: 20px; gap: 2px; background-color: {}",
                if count.get().is_multiple_of(2) { "#e3f2fd" } else { "#fff3e0" }
            )},
            onclick: move || {
                let now = Instant::now();
                if let Some(prev) = last_click.get() {
                    delta_ms.set(format!(
                        "{:.1}",
                        now.duration_since(prev).as_secs_f64() * 1000.0
                    ));
                }
                last_click.set(Some(now));
                count.update(|n| *n += 1);
            },
            div { class: "click-count", "Clicks: " {|| count.get().to_string()} }
            div { class: "click-delta", {|| delta_ms.get()} }
            for i in (0..40u32).collect::<Vec<_>>() {
                div { key: i, style: "font-size: 12px",
                    "Filler row " {i.to_string()} " with enough text to make a \
                     full-window software paint expensive in a debug build, so a \
                     debug command serialized behind one is clearly observable (#153)."
                }
            }
        }
    }
}

fn main() {
    App::new(app)
        .title("debug-click-latency-probe")
        .size(900, 600)
        .run();
}
