//! A desktop wallpaper: a `wlr-layer-shell` surface in the `background`
//! layer, under every window, sized by the compositor to its whole output
//! (issue #2413).
//!
//! It is the app's own [`LayerSurface`]: anchored to all four edges with
//! both axes assigned by the compositor (`LayerAxes::BOTH`, the protocol's
//! size 0 x 0), an exclusive zone that ignores other surfaces' reserved
//! space (`LayerExclusiveZone::Ignore`, so it also lies under a panel), and
//! no keyboard. Its content lays out to the size the compositor gives, which
//! it prints in its corner.
//!
//! `cargo run --release -p wayland-wallpaper`, beside `wayland-bar` (the
//! examples run them as two processes; one process can also open both, see
//! `App::windowless` and `open_window`). `WALLPAPER_OUTPUT=DP-1` puts it on
//! that output. On GNOME (no
//! layer shell) it is an ordinary 960 x 540 window.

use rinch::prelude::*;
use rinch::{
    LayerAnchor, LayerAxes, LayerExclusiveZone, LayerShellLayer, LayerSurface, WindowProps,
};

#[component]
fn wallpaper() -> NodeHandle {
    rsx! {
        div {
            style: "position: relative; width: 100%; height: 100%; \
                    background: linear-gradient(135deg, #0f2027, #203a43 50%, #2c5364); \
                    font-family: sans-serif;",
            span {
                class: "size",
                style: "position: absolute; right: 16px; bottom: 56px; \
                        color: rgba(255, 255, 255, 0.55); font-size: 14px;",
                {|| {
                    let (w, h) = window_size();
                    format!("{w} x {h}")
                }}
            }
        }
    }
}

fn main() {
    App::new(wallpaper)
        .window_props(WindowProps {
            title: "Wayland wallpaper".into(),
            // The compositor's on a layer surface; the first frame's, and an
            // ordinary window's on GNOME.
            width: 960,
            height: 540,
            borderless: true,
            app_id: Some("rinch-wayland-wallpaper".into()),
            wayland_layer: Some(LayerSurface {
                layer: LayerShellLayer::Background,
                anchor: LayerAnchor::ALL,
                compositor_size: LayerAxes::BOTH,
                exclusive_zone: LayerExclusiveZone::Ignore,
                output: std::env::var("WALLPAPER_OUTPUT").ok(),
                ..Default::default()
            }),
            ..Default::default()
        })
        .run();
}
