//! A desktop panel: a full-width bar along the bottom of its output that
//! reserves its height, so maximised windows stop at its top edge (issue
//! #2413). It is a `wlr-layer-shell` surface with the app's own
//! configuration ([`WindowProps::wayland_layer`]):
//!
//! - anchored to the bottom, left and right edges, with its **width assigned
//!   by the compositor** (`compositor_size: LayerAxes::WIDTH`, the protocol's
//!   width 0): the output's width, and again when the output's mode changes;
//! - an **exclusive zone** of its height (`LayerExclusiveZone::Reserve`);
//! - keyboard none: clicking it never takes the focus from the window you
//!   are typing in.
//!
//! What it shows is what a real panel shows, so its repaint cost is a
//! panel's: a launcher button and four workspace buttons on the left, a
//! title in the middle, and on the right a button that makes the bar taller
//! or thinner (changing the exclusive zone at run time with
//! `set_current_window_layer`) and a clock ticking once a second, on the
//! second.
//!
//! Run it on KWin, sway, Hyprland, niri or COSMIC: `cargo run --release -p
//! wayland-bar`. On GNOME (no layer shell) it is an ordinary 800 x 32 window.
//!
//! - `BAR_EDGE=top` puts it along the top edge instead.
//! - `BAR_OUTPUT=DP-1` puts it on that output (default: the compositor's
//!   choice).
//! - `BAR_SCRIPT=1` changes the thickness every `BAR_STEP_MS` (1500) ms, four
//!   times, printing the size the compositor gave after each, then exits:
//!   what the live check of #2413 reads.
//!
//! A wallpaper is a second process (`examples/wayland-wallpaper`): rinch
//! shows one window per process.

use rinch::prelude::*;
use rinch::{LayerAnchor, LayerAxes, LayerExclusiveZone, LayerSurface, WindowProps};

/// The bar's thickness, logical pixels: thin, and taller.
const THIN: u32 = 32;
const THICK: u32 = 48;

/// The layer surface for a bar of `thickness`: along the bottom (or top)
/// edge, as wide as the compositor makes it, reserving its thickness.
fn bar_surface(thickness: u32) -> LayerSurface {
    let edge = if std::env::var("BAR_EDGE").as_deref() == Ok("top") {
        LayerAnchor::TOP
    } else {
        LayerAnchor::BOTTOM
    };
    LayerSurface {
        anchor: edge | LayerAnchor::LEFT | LayerAnchor::RIGHT,
        exclusive_zone: LayerExclusiveZone::Reserve(thickness),
        compositor_size: LayerAxes::WIDTH,
        output: std::env::var("BAR_OUTPUT").ok(),
        ..Default::default()
    }
}

/// Make the bar `thickness` tall and reserve that much: the size first, so
/// the zone never reserves more than the bar covers. The width is the
/// compositor's whatever is asked here.
fn set_thickness(thickness: u32) {
    let width = current_window_size().map_or(1.0, |(w, _)| w);
    set_current_window_size(width, f64::from(thickness));
    set_current_window_layer(bar_surface(thickness));
}

/// The clock's text: local time to the second.
fn clock_text() -> String {
    chrono::Local::now()
        .format("%a %e %b  %H:%M:%S")
        .to_string()
}

/// Milliseconds to the next second, plus a little so the tick lands after it.
fn ms_to_next_second() -> u32 {
    let ms = chrono::Local::now().timestamp_subsec_millis();
    1000 - ms.min(999) + 5
}

/// `BAR_SCRIPT=1`: thicken and thin the bar, reporting what the compositor
/// gave each time.
fn script(thick: Signal<bool>) {
    let step: u32 = std::env::var("BAR_STEP_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1500);
    fn run(thick: Signal<bool>, left: u32, step: u32) {
        println!(
            "BAR thickness={} size={:?}",
            if thick.get() { THICK } else { THIN },
            current_window_size()
        );
        if left == 0 {
            println!("BAR done");
            std::process::exit(0);
        }
        let next = !thick.get();
        thick.set(next);
        set_thickness(if next { THICK } else { THIN });
        set_timeout(step, move || run(thick, left - 1, step));
    }
    set_timeout(step, move || run(thick, 4, step));
}

#[component]
fn bar() -> NodeHandle {
    let now = Signal::new(clock_text());
    let thick = Signal::new(false);
    let workspace = Signal::new(1u32);

    // On the second, then every second: the interval keeps its deadlines,
    // so it stays on the second.
    set_timeout(ms_to_next_second(), move || {
        now.set(clock_text());
        let _ = set_interval(1000, move || now.set(clock_text()));
    });
    if std::env::var_os("BAR_SCRIPT").is_some() {
        script(thick);
    }

    rsx! {
        div {
            style: "display: flex; align-items: center; gap: 6px; height: 100%; \
                    padding: 0 8px; box-sizing: border-box; \
                    background: #1d1f27; color: #e6e8ef; \
                    border-top: 1px solid #343846; \
                    font-family: sans-serif; font-size: 13px;",
            button {
                class: "launcher",
                style: "padding: 3px 10px; border: none; border-radius: 6px; \
                        background: #3b5bdb; color: white;",
                "Apps"
            }
            for n in vec![1u32, 2, 3, 4] {
                button {
                    key: n,
                    class: "workspace",
                    onclick: move || workspace.set(n),
                    style: {move || format!(
                        "width: 24px; padding: 3px 0; border: none; border-radius: 6px; \
                         color: #e6e8ef; background: {};",
                        if workspace.get() == n { "#4c5266" } else { "transparent" }
                    )},
                    {n.to_string()}
                }
            }
            span {
                style: "flex: 1; text-align: center; color: #a4a9b8; \
                        white-space: nowrap; overflow: hidden;",
                {|| format!("Workspace {}", workspace.get())}
            }
            button {
                class: "thickness",
                onclick: move || {
                    let next = !thick.get();
                    thick.set(next);
                    set_thickness(if next { THICK } else { THIN });
                },
                style: "padding: 3px 8px; border: none; border-radius: 6px; \
                        background: #2c3040; color: #e6e8ef;",
                {|| if thick.get() { format!("⇕ {THICK}") } else { format!("⇕ {THIN}") }}
            }
            span {
                class: "clock",
                style: "padding: 0 4px; white-space: nowrap;",
                {|| now.get()}
            }
        }
    }
}

fn main() {
    App::new(bar)
        .window_props(WindowProps {
            title: "Wayland bar".into(),
            // The width is the compositor's on a layer surface; 800 is the
            // first frame's, and an ordinary window's on GNOME.
            width: 800,
            height: THIN,
            borderless: true,
            app_id: Some("rinch-wayland-bar".into()),
            wayland_layer: Some(bar_surface(THIN)),
            ..Default::default()
        })
        .run();
}
