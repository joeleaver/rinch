//! A live check of `window_roles` and output ids (#2659, #2658): run inside a
//! nested compositor in an isolated session. A windowless app prints what it
//! knows before it opens any window:
//!
//! ```text
//! ROLES Some(WindowRoles { layer_shell: true })
//! OUTPUT id=… name=Some("HEADLESS-1") …
//! ```
//!
//! With `PANELS=1` and the layer shell offered, it then opens a layer-surface
//! panel on every output, keyed and placed by `Output::id` and as wide as its
//! output, prints `OPEN <id>` for each and then `PANEL <kind> <size>`, and
//! exits after two seconds. It never opens a window where
//! `layer_shell_available()` is false: that is the point of asking first.

use rinch::prelude::*;

#[component]
fn probe() -> NodeHandle {
    let roles = window_roles();
    println!("ROLES {roles:?}");
    for output in outputs() {
        println!(
            "OUTPUT id={} name={:?} size={:?} scale={}",
            output.id, output.name, output.size, output.scale
        );
    }
    let panels = std::env::var("PANELS").as_deref() == Ok("1") && layer_shell_available();
    if !panels {
        println!("NO WINDOW");
        exit();
        return __scope.create_element("div");
    }
    // Each panel's surface kind and size once the compositor has configured
    // it: a panel is as wide as the output it went on.
    rinch_core::reactive::unowned(|| {
        set_timeout(1500, || {
            for h in windows() {
                println!("PANEL {:?} {:?}", h.surface_kind(), h.size());
            }
        });
        set_timeout(2000, exit)
    });
    rsx! {
        div {
            for output in outputs() {
                Window {
                    key: output.id,
                    height: 24,
                    wayland_layer: Some(LayerSurface {
                        anchor: LayerAnchor::TOP | LayerAnchor::LEFT | LayerAnchor::RIGHT,
                        compositor_size: LayerAxes::WIDTH,
                        output_id: Some(output.id),
                        ..Default::default()
                    }),
                    on_open: {
                        let id = output.id;
                        move |_: WindowHandle| println!("OPEN {id}")
                    },
                    div { "panel" }
                }
            }
        }
    }
}

fn main() {
    App::windowless(probe).run();
}
