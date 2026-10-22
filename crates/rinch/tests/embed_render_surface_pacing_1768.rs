//! A `RenderSurface` in an embedded context knows where it is, and its render
//! callback is paced by `set_continuous` / `request_frame` (issue #1768).
//!
//!     cargo test -p rinch --features embed --test embed_render_surface_pacing_1768
//!
//! One test in this binary: `RinchContext::new` registers its thread as the
//! main thread, and the surface registry is thread-local.

#![cfg(any(feature = "gpu", feature = "embed"))]

use std::cell::Cell;
use std::rc::Rc;

use rinch::embed::{RinchContext, RinchContextConfig};
use rinch::platform::AppAction;
use rinch::prelude::*;
use rinch::render_surface::RenderSurfaceHandle;

#[test]
fn an_embedded_surface_reports_its_position_and_is_paced() {
    let surface = create_render_surface();
    let calls: Rc<Cell<u32>> = Rc::default();
    {
        let calls = calls.clone();
        surface.set_render_callback(move |_writer, _w, _h| calls.set(calls.get() + 1));
    }
    surface.set_continuous(false);
    let mounted = surface.clone();

    // Two more surfaces under a `display: none` wrapper: one continuous, one
    // on demand with a frame requested while hidden. Neither may keep the
    // host busy while it has no size.
    let counted = |s: &RenderSurfaceHandle| {
        let n: Rc<Cell<u32>> = Rc::default();
        let c = n.clone();
        s.set_render_callback(move |_writer, _w, _h| c.set(c.get() + 1));
        n
    };
    let hidden_cont = create_render_surface();
    let hidden_cont_calls = counted(&hidden_cont);
    hidden_cont.set_continuous(true);
    let hidden_req = create_render_surface();
    let hidden_req_calls = counted(&hidden_req);
    hidden_req.set_continuous(false);
    let hidden_wrap: Rc<std::cell::RefCell<Option<NodeHandle>>> = Rc::default();
    let (hc, hr, hw) = (hidden_cont.clone(), hidden_req.clone(), hidden_wrap.clone());
    let mut ctx = RinchContext::new(
        RinchContextConfig {
            width: 600,
            height: 450,
            scale_factor: 1.5,
            theme: None,
            fonts: Vec::new(),
        },
        move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            let wrap = scope.create_element("div");
            // Off the origin on both axes, and not the same on both.
            wrap.set_attribute(
                "style",
                "margin-left: 37px; margin-top: 21px; width: 100px; height: 40px;",
            );
            let rs = RenderSurface {
                surface: Some(mounted),
            }
            .render(scope, &[]);
            wrap.append_child(&rs);
            root.append_child(&wrap);
            let hide = scope.create_element("div");
            hide.set_attribute("style", "display: none; width: 50px; height: 20px;");
            for s in [hc.clone(), hr.clone()] {
                let rs = RenderSurface { surface: Some(s) }.render(scope, &[]);
                hide.append_child(&rs);
            }
            root.append_child(&hide);
            *hw.borrow_mut() = Some(hide);
            root
        },
    );

    ctx.update(&[]);
    let _ = ctx.scene();
    let (x, y, w, h) = surface.layout_rect();
    assert_eq!((w, h), (150, 60), "physical size");
    assert_eq!(
        (x, y),
        (37.0, 21.0),
        "the surface's logical top-left (it was (0, 0) before #1768)"
    );
    assert_eq!(calls.get(), 1, "on demand: the first size draws once");

    // Nothing asked: further scenes do not run the callback, and the context
    // does not say it needs one.
    let _ = ctx.scene();
    let _ = ctx.scene();
    assert_eq!(
        calls.get(),
        1,
        "an on-demand surface is not run by a quiet scene"
    );
    assert!(
        !ctx.needs_update(),
        "nothing wants a frame: not even the continuous surface under display: none"
    );
    hidden_req.request_frame();
    assert!(
        !ctx.needs_update(),
        "a request made while hidden waits for a size without keeping the host busy"
    );
    let _ = ctx.scene();
    assert_eq!(
        (hidden_cont_calls.get(), hidden_req_calls.get()),
        (0, 0),
        "hidden surfaces never ran"
    );

    // A request: the context says so, and the next scene draws it once.
    surface.request_frame();
    assert!(ctx.needs_update(), "a requested frame needs an update");
    let actions = ctx.update(&[]);
    assert!(
        actions.contains(&AppAction::RequestRedraw),
        "update asks the host for a frame: {actions:?}"
    );
    let _ = ctx.scene();
    assert_eq!(calls.get(), 2, "the requested frame drew");
    let _ = ctx.scene();
    assert_eq!(calls.get(), 2, "once");

    // Continuous: every scene draws, and the context keeps asking.
    surface.set_continuous(true);
    for i in 0..3 {
        let _ = ctx.scene();
        assert_eq!(calls.get(), 3 + i, "continuous scene {i} draws");
        assert!(
            ctx.needs_update(),
            "a continuous surface always wants a frame"
        );
    }

    // Shown: both draw (the continuous one every scene, the requested one once).
    hidden_wrap
        .borrow()
        .as_ref()
        .unwrap()
        .set_attribute("style", "width: 50px; height: 20px;");
    ctx.update(&[]);
    let _ = ctx.scene();
    let _ = ctx.scene();
    assert_eq!(
        hidden_cont_calls.get(),
        2,
        "continuous once shown: every scene"
    );
    assert_eq!(
        hidden_req_calls.get(),
        1,
        "the request made while hidden drew once"
    );
}
