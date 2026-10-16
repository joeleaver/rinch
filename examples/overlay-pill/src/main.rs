//! An overlay pill that never takes keyboard focus (issue #1519).
//!
//! Open a text editor, start typing, and run `cargo run -p overlay-pill`.
//! The pill appears on top without taking focus; keep typing and the text
//! still goes to the editor. Click "+1": the count goes up, and the next
//! keystroke still goes to the editor. "Click-through 5 s" lets the pointer
//! through to whatever is under the pill for five seconds, then turns it back.
//!
//! The pill sits at the status-bar level and on every workspace (#1549):
//! switch desktops (or macOS Spaces, fullscreen apps' included) and it stays.
//!
//! On native Wayland the focus half cannot be had (see the guide's
//! "Overlay windows" section); `prefer_x11(true)` runs the pill under
//! XWayland, where it can.

use std::time::Duration;

use rinch::prelude::*;
use rinch_core::element::{WindowLevel, WindowProps};

#[component]
fn pill() -> NodeHandle {
    let count = Signal::new(0);
    let through = Signal::new(false);
    rsx! {
        div {
            style: "display: flex; align-items: center; gap: 8px; height: 100%; \
                    padding: 0 14px; border-radius: 22px; box-sizing: border-box; \
                    background: rgba(24, 24, 32, 0.88); color: white; \
                    font-family: sans-serif; font-size: 14px;",
            span { style: "flex: 1;", "Clicks: " {|| count.get().to_string()} }
            button {
                onclick: move || count.update(|n| *n += 1),
                "+1"
            }
            button {
                onclick: move || {
                    through.set(true);
                    set_click_through_current_window(true);
                    // A click-through window cannot be clicked back: turn it
                    // off from elsewhere (here a thread; a hotkey or a tray
                    // item in a real app).
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_secs(5));
                        set_click_through_current_window(false);
                        through.send(false);
                    });
                },
                {|| if through.get() { "Passing clicks…" } else { "Click-through 5 s" }}
            }
            button { onclick: || { close_current_window(); }, "×" }
        }
    }
}

fn main() {
    App::new(pill)
        .window_props(WindowProps {
            title: "Overlay pill".into(),
            width: 360,
            height: 44,
            x: Some(80),
            y: Some(80),
            borderless: true,
            transparent: true,
            resizable: false,
            always_on_top: true,
            non_activating: true,
            skip_taskbar: true,
            // #1549: on every workspace / Space, over fullscreen apps on
            // macOS (Windows and Wayland: no effect).
            level: WindowLevel::StatusBar,
            visible_on_all_workspaces: true,
            ..Default::default()
        })
        .prefer_x11(true)
        .run();
}
