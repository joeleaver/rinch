//! Two contexts on one thread each draw their own `RenderSurface`'s new frame
//! (issue #331, review of PR #1148).
//!
//! The surface registry is thread-global. A context's `scene()` used to run
//! every surface's render callback and collect — clearing the new-frame flag
//! of — every surface on the thread, so when context A's `scene()` ran first
//! in a frame it took B's fresh frame, and B's `scene()` was served from its
//! cache with the old one. A context now drives only the surfaces in its own
//! document.
//!
//! Requires the `embed` (or `gpu`) feature:
//!     cargo test -p rinch --features embed --test embed_render_surface_two_contexts
//!
//! One test in this binary: `RinchContext::new` registers its thread as the
//! main thread, and the surface registry is thread-local.

#![cfg(any(feature = "gpu", feature = "embed"))]

use std::cell::Cell;
use std::rc::Rc;

use rinch::embed::{RinchContext, RinchContextConfig};
use rinch::prelude::*;
use rinch_dom::perf::Counter;

fn surface_ctx(surface: RenderSurfaceHandle, box_css: &'static str) -> RinchContext {
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
            let wrap = scope.create_element("div");
            wrap.set_attribute("style", box_css);
            let rs = RenderSurface {
                surface: Some(surface.clone()),
            }
            .render(scope, &[]);
            wrap.append_child(&rs);
            root.append_child(&wrap);
            root
        },
    )
}

#[test]
fn each_context_draws_its_own_surfaces_new_frame() {
    let sa = create_render_surface();
    let sb = create_render_surface();
    let b_calls: Rc<Cell<u32>> = Rc::default();
    {
        let b_calls = b_calls.clone();
        sb.set_render_callback(move |_, _, _| b_calls.set(b_calls.get() + 1));
    }
    let mut a = surface_ctx(sa.clone(), "width: 50px; height: 40px;");
    let mut b = surface_ctx(sb.clone(), "width: 70px; height: 30px;");
    for _ in 0..3 {
        a.update(&[]);
        let _ = a.scene();
        b.update(&[]);
        let _ = b.scene();
    }
    assert_eq!(sa.layout_size(), (50, 40), "control: A's surface is sized");
    assert_eq!(sb.layout_size(), (70, 30), "control: B's surface is sized");

    // A frame arrives for B from outside its callback (another thread, a
    // decoder). The host's frame updates and draws A first, then B.
    let px = [0u8, 0, 255, 255].repeat(70 * 30);
    sb.writer().submit_frame(&px, 70, 30);
    let calls = b_calls.get();
    a.update(&[]);
    let _ = a.scene();
    assert_eq!(b_calls.get(), calls, "A's scene does not run B's callback");
    b.update(&[]);
    b.reset_perf();
    let _ = b.scene();
    assert_eq!(b_calls.get(), calls + 1, "B's scene runs B's callback once");
    assert_eq!(
        b.perf_total().get(Counter::PaintCachedFrames),
        0,
        "B's scene is rebuilt with its new frame, not served from the cache"
    );
}
