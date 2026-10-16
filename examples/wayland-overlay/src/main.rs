//! A native Wayland overlay: a `wlr-layer-shell` surface that never takes
//! keyboard focus (issue #1523).
//!
//! Run it on KWin, sway, Hyprland, niri or COSMIC with a text editor focused:
//! `cargo run -p wayland-overlay`. The bar appears above every window without
//! taking focus; keep typing and the text still goes to the editor. Drag it
//! by its grip (a `data-drag-window` region) and resize it from its edges
//! (`resize_inset`): a layer surface has no compositor move or resize, so
//! rinch follows the pointer and sets its margins and size. "Hide 2 s" hides
//! it and shows it again where it was. On GNOME (no layer shell) it is an
//! ordinary window and the label says so.
//!
//! `OVERLAY_OUTPUT=DP-1` puts it on that output; `OVERLAY_CENTRED=1` gives it
//! no position, so the compositor centres it; `OVERLAY_TOPLEVEL=1` makes it an
//! ordinary window (`wayland_layer_shell: Some(false)`); `OVERLAY_ABOVE_ALL=1`
//! puts it in the `overlay` layer. `OVERLAY_SCRIPT=1` runs the
//! geometry API through a fixed sequence (move, resize, hide, show) and
//! prints what `current_window_position` / `current_window_size` answer after
//! each step, then exits: what the live test of #1523 reads.

use rinch::prelude::*;
use rinch_core::element::{WindowLevel, WindowProps};

fn geometry() -> String {
    format!(
        "kind={:?} position={:?} size={:?}",
        current_window_surface_kind(),
        current_window_position(),
        current_window_size()
    )
}

/// The scripted steps: a name and what it does.
type Steps = std::rc::Rc<Vec<(&'static str, Box<dyn Fn()>)>>;

/// The scripted run: each step, then `OVERLAY_STEP_MS` (600 ms) later what
/// it gave, then as long again before the next.
fn script() {
    let step_ms: u32 = std::env::var("OVERLAY_STEP_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(600);
    let steps: Steps = std::rc::Rc::new(vec![
        ("start", Box::new(|| {})),
        (
            "move to (200, 150)",
            Box::new(|| set_current_window_position(200.0, 150.0)),
        ),
        (
            "resize to 420x60",
            Box::new(|| set_current_window_size(420.0, 60.0)),
        ),
        ("hide", Box::new(hide_current_window)),
        ("show", Box::new(show_current_window)),
        (
            "move to (37, 411)",
            Box::new(|| set_current_window_position(37.0, 411.0)),
        ),
    ]);
    fn run(steps: Steps, i: usize, step_ms: u32) {
        if i == steps.len() {
            println!("SCRIPT done");
            std::process::exit(0);
        }
        (steps[i].1)();
        set_timeout(step_ms, move || {
            println!("SCRIPT {}: {}", steps[i].0, geometry());
            set_timeout(step_ms, move || run(steps, i + 1, step_ms));
        });
    }
    set_timeout(800, move || run(steps, 0, step_ms));
}

#[component]
fn bar() -> NodeHandle {
    let shown = Signal::new(geometry());
    if std::env::var_os("OVERLAY_TIMER_DRAG").is_some() {
        set_timeout(3000, || {
            println!("TIMER start_window_drag");
            start_window_drag();
        });
    }
    if std::env::var_os("OVERLAY_SCRIPT").is_some() {
        script();
    }
    rsx! {
        div {
            style: "display: flex; align-items: center; gap: 8px; height: 100%; \
                    padding: 0 10px; border-radius: 10px; box-sizing: border-box; \
                    background: rgba(24, 24, 32, 0.9); color: white; \
                    font-family: sans-serif; font-size: 13px;",
            div {
                data-drag-window: "true",
                style: "cursor: move; padding: 4px 6px; border-radius: 4px; \
                        background: rgba(255, 255, 255, 0.15);",
                "⠿"
            }
            div {
                id: "apigrip",
                onmousedown: || start_window_drag(),
                style: "padding: 4px 6px; background: rgba(255, 0, 0, 0.5);",
                "API"
            }
            span { style: "flex: 1; white-space: nowrap; overflow: hidden;", {|| shown.get()} }
            button { onclick: move || shown.set(geometry()), "Where?" }
            button {
                onclick: || {
                    hide_current_window();
                    set_timeout(2000, show_current_window);
                },
                "Hide 2 s"
            }
            button { onclick: || { close_current_window(); }, "×" }
        }
    }
}

fn main() {
    let positioned = std::env::var_os("OVERLAY_CENTRED").is_none();
    App::new(bar)
        .window_props(WindowProps {
            title: "Wayland overlay".into(),
            width: 520,
            height: 44,
            // `OVERLAY_CENTRED=1`: no position, the compositor centres it.
            x: positioned.then_some(80),
            y: positioned.then_some(60),
            borderless: true,
            transparent: true,
            resize_inset: Some(6.0),
            non_activating: true,
            skip_taskbar: true,
            output: std::env::var("OVERLAY_OUTPUT").ok(),
            // `OVERLAY_ABOVE_ALL=1`: the `overlay` layer, above panels and
            // fullscreen windows (#1549's level).
            level: if std::env::var_os("OVERLAY_ABOVE_ALL").is_some() {
                WindowLevel::ScreenSaver
            } else {
                WindowLevel::Floating
            },
            // `OVERLAY_TOPLEVEL=1`: an ordinary xdg-toplevel, as before #1523.
            wayland_layer_shell: std::env::var_os("OVERLAY_TOPLEVEL").map(|_| false),
            ..Default::default()
        })
        .run();
}
