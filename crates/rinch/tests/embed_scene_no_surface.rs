//! With no `RenderSurface` on the thread, `scene()` runs no surface pass
//! (issue #331, review of PR #1148).
//!
//! The surface pass walks the whole document to find this context's
//! surfaces, and it ran ahead of the scene cache: a `scene()` on an unchanged
//! 5000-node document went from about 8 ns to about 60 µs, on every frame of
//! every embedded host. With nothing registered every step of the pass is a
//! no-op, so it is skipped. Pinned structurally (the pass count), not by a
//! timing.
//!
//! Requires the `embed` (or `gpu`) feature:
//!     cargo test -p rinch --features embed --test embed_scene_no_surface
//!
//! One test in this binary: `RinchContext::new` registers its thread as the
//! main thread, and the surface registry is thread-local.

#![cfg(any(feature = "gpu", feature = "embed"))]

use rinch::embed::{RinchContext, RinchContextConfig};
use rinch::prelude::*;

fn context(with_surface: Option<RenderSurfaceHandle>) -> RinchContext {
    RinchContext::new(
        RinchContextConfig {
            width: 400,
            height: 300,
            scale_factor: 1.0,
            theme: None,
            fonts: Vec::new(),
        },
        move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            for _ in 0..50 {
                root.append_child(&scope.create_element("div"));
            }
            if let Some(surface) = with_surface {
                let rs = RenderSurface {
                    surface: Some(surface),
                }
                .render(scope, &[]);
                root.append_child(&rs);
            }
            root
        },
    )
}

#[test]
fn scene_runs_the_surface_pass_only_while_a_surface_is_registered() {
    let mut plain = context(None);
    plain.update(&[]);
    for _ in 0..3 {
        let _ = plain.scene();
    }
    assert_eq!(
        plain.surface_pass_count(),
        0,
        "no surface registered, no pass"
    );

    let mut with = context(Some(create_render_surface()));
    with.update(&[]);
    let _ = with.scene();
    let _ = with.scene();
    assert_eq!(
        with.surface_pass_count(),
        2,
        "control: every scene drives a registered surface"
    );
}
