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
//! A second button moves the bar to the other edge (top or bottom) at run
//! time: a new anchor through `set_current_window_layer`, and the reserved
//! area moves with it.
//!
//! - `BAR_EDGE=top` starts it along the top edge instead.
//! - `BAR_GAP=6` floats it that many logical pixels off its edge (a margin
//!   on the anchored edge), so a move between edges changes the anchors and
//!   the margins together.
//! - `BAR_OUTPUT=DP-1` puts it on that output (default: the compositor's
//!   choice).
//! - `BAR_SCRIPT=1` changes the thickness every `BAR_STEP_MS` (1500) ms, four
//!   times, printing the size the compositor gave after each, then exits;
//!   `BAR_SCRIPT=edges` moves it to the other edge instead, four times. What
//!   the live checks of #2413 read.
//! - `BAR_SCRIPT=outputs` moves it to the next connected output
//!   (`rinch::outputs`, #2414) every step, eight times, through
//!   `set_current_window_layer`, printing the outputs and the size the
//!   compositor gave the bar at each step; an output unplugged under it is
//!   left to the compositor (rinch re-creates the surface where the
//!   compositor puts it). What the live check of #2414 reads.
//!
//! A wallpaper is a second process (`examples/wayland-wallpaper`): rinch
//! shows one window per process.

use rinch::prelude::*;
use rinch::{LayerAnchor, LayerAxes, LayerExclusiveZone, LayerMargins, LayerSurface, WindowProps};

/// The bar's thickness, logical pixels: thin, and taller.
const THIN: u32 = 32;
const THICK: u32 = 48;

/// Where the bar is and how thick: what its layer surface follows.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Placement {
    top: bool,
    thick: bool,
}

impl Placement {
    fn thickness(self) -> u32 {
        if self.thick { THICK } else { THIN }
    }
}

/// The layer surface for a bar: along the top or bottom edge, as wide as the
/// compositor makes it, reserving its thickness.
fn bar_surface(placement: Placement) -> LayerSurface {
    let edge = if placement.top {
        LayerAnchor::TOP
    } else {
        LayerAnchor::BOTTOM
    };
    let thickness = placement.thickness();
    let gap: i32 = std::env::var("BAR_GAP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    LayerSurface {
        anchor: edge | LayerAnchor::LEFT | LayerAnchor::RIGHT,
        margins: if placement.top {
            LayerMargins {
                top: gap,
                ..Default::default()
            }
        } else {
            LayerMargins {
                bottom: gap,
                ..Default::default()
            }
        },
        exclusive_zone: LayerExclusiveZone::Reserve(thickness),
        compositor_size: LayerAxes::WIDTH,
        output: std::env::var("BAR_OUTPUT").ok(),
        ..Default::default()
    }
}

/// Move and size the live bar to `placement`: a new thickness first, so the
/// zone never reserves more than the bar covers, then the anchors, margins
/// and zone in one change. The width is the compositor's whatever is asked
/// here.
fn place(placement: Placement) {
    let thickness = f64::from(placement.thickness());
    if let Some((width, height)) = current_window_size()
        && height != thickness
    {
        set_current_window_size(width, thickness);
    }
    set_current_window_layer(bar_surface(placement));
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

/// `BAR_SCRIPT=1`: thicken and thin the bar; `BAR_SCRIPT=edges`: move it
/// between the edges. Each step reports where the bar is and what size the
/// compositor gave it.
fn script(placement: Signal<Placement>, edges: bool) {
    let step: u32 = std::env::var("BAR_STEP_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1500);
    fn run(placement: Signal<Placement>, edges: bool, left: u32, step: u32) {
        let p = placement.get();
        println!(
            "BAR edge={} thickness={} size={:?}",
            if p.top { "top" } else { "bottom" },
            p.thickness(),
            current_window_size()
        );
        if left == 0 {
            println!("BAR done");
            std::process::exit(0);
        }
        let next = if edges {
            Placement { top: !p.top, ..p }
        } else {
            Placement {
                thick: !p.thick,
                ..p
            }
        };
        placement.set(next);
        place(next);
        set_timeout(step, move || run(placement, edges, left - 1, step));
    }
    set_timeout(step, move || run(placement, edges, 4, step));
}

/// `BAR_SCRIPT=outputs`: move the bar to the next output every step. Each
/// step first reports the outputs and the bar's size (its width is its
/// output's logical width), then moves it.
fn tour_outputs(placement: Signal<Placement>) {
    let step: u32 = std::env::var("BAR_STEP_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1500);
    fn run(placement: Signal<Placement>, next: usize, left: u32, step: u32) {
        let list = outputs();
        let names: Vec<String> = list
            .iter()
            .map(|o| {
                format!(
                    "{} {}x{}",
                    o.name.as_deref().unwrap_or("?"),
                    o.size.0,
                    o.size.1
                )
            })
            .collect();
        println!(
            "BAR outputs=[{}] size={:?}",
            names.join(", "),
            current_window_size()
        );
        if left == 0 {
            println!("BAR done");
            std::process::exit(0);
        }
        if let Some(output) = list.get(next % list.len().max(1)) {
            println!("BAR move to {}", output.name.as_deref().unwrap_or("?"));
            set_current_window_layer(LayerSurface {
                output: output.name.clone(),
                ..bar_surface(placement.get())
            });
        }
        set_timeout(step, move || run(placement, next + 1, left - 1, step));
    }
    set_timeout(step, move || run(placement, 1, 8, step));
}

/// Where the bar starts: `BAR_EDGE=top`, else the bottom; thin.
fn initial_placement() -> Placement {
    Placement {
        top: std::env::var("BAR_EDGE").as_deref() == Ok("top"),
        thick: false,
    }
}

#[component]
fn bar() -> NodeHandle {
    let now = Signal::new(clock_text());
    let placement = Signal::new(initial_placement());
    let workspace = Signal::new(1u32);

    // On the second, then every second: the interval keeps its deadlines,
    // so it stays on the second.
    set_timeout(ms_to_next_second(), move || {
        now.set(clock_text());
        let _ = set_interval(1000, move || now.set(clock_text()));
    });
    match std::env::var("BAR_SCRIPT").as_deref() {
        Ok("edges") => script(placement, true),
        Ok("outputs") => tour_outputs(placement),
        Ok(_) => script(placement, false),
        Err(_) => {}
    }

    rsx! {
        div {
            // The edge line faces the windows: on top along the bottom
            // edge, underneath along the top one.
            style: {move || format!(
                "display: flex; align-items: center; gap: 6px; height: 100%; \
                 padding: 0 8px; box-sizing: border-box; \
                 background: #1d1f27; color: #e6e8ef; \
                 border-{}: 1px solid #343846; \
                 font-family: sans-serif; font-size: 13px;",
                if placement.get().top { "bottom" } else { "top" }
            )},
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
                class: "edge",
                onclick: move || {
                    let next = Placement { top: !placement.get().top, ..placement.get() };
                    placement.set(next);
                    place(next);
                },
                style: "padding: 3px 8px; border: none; border-radius: 6px; \
                        background: #2c3040; color: #e6e8ef;",
                {|| if placement.get().top { "To bottom" } else { "To top" }}
            }
            button {
                class: "thickness",
                onclick: move || {
                    let next = Placement { thick: !placement.get().thick, ..placement.get() };
                    placement.set(next);
                    place(next);
                },
                style: "padding: 3px 8px; border: none; border-radius: 6px; \
                        background: #2c3040; color: #e6e8ef;",
                {|| format!("⇕ {}", placement.get().thickness())}
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
            wayland_layer: Some(bar_surface(initial_placement())),
            ..Default::default()
        })
        .run();
}
