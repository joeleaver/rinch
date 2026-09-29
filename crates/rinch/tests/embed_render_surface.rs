//! A `RenderSurface` inside a `RinchContext` is sized, driven and painted
//! (issue #331).
//!
//! The desktop runtime does three things for a `RenderSurface` on every paint:
//! it tells the surface its layout size, runs the surface's render callback,
//! and hands the frame to paint so the surface's box draws it inline. An
//! embedded context did none of them, so a surface there never learned its
//! size (a render callback, which skips an unmeasured surface, never ran), and
//! a frame submitted to it never reached the scene.
//!
//! Requires the `embed` (or `gpu`) feature:
//!     cargo test -p rinch --features embed --test embed_render_surface
//!
//! One test in this binary: `RinchContext::new` registers its thread as the
//! main thread, and the surface registry is thread-local.

#![cfg(any(feature = "gpu", feature = "embed"))]

use std::cell::Cell;
use std::rc::Rc;

use rinch::embed::{RinchContext, RinchContextConfig};
use rinch::platform::AppAction;
use rinch::prelude::*;

/// Late-bound resources the scene carries: an image draw is one of them, and
/// nothing else in this document (no text, no gradient) adds any.
fn scene_patches(ctx: &mut RinchContext) -> usize {
    ctx.scene().encoding().resources.patches.len()
}

#[test]
fn a_render_surface_in_an_embedded_context_is_sized_driven_and_painted() {
    let surface = create_render_surface();
    let calls: Rc<Cell<u32>> = Rc::default();
    let seen: Rc<Cell<(u32, u32)>> = Rc::default();
    {
        let calls = calls.clone();
        let seen = seen.clone();
        surface.set_render_callback(move |writer, w, h| {
            calls.set(calls.get() + 1);
            seen.set((w, h));
            // An opaque red frame at the surface's own size.
            let px = [255u8, 0, 0, 255].repeat((w * h) as usize);
            writer.submit_frame(&px, w, h);
        });
    }
    let mounted = surface.clone();
    let mut ctx = RinchContext::new(
        RinchContextConfig {
            // Scale 1.5, off the identity: the surface's size is PHYSICAL
            // pixels, so a size taken from the logical box would read 120x60.
            width: 600,
            height: 450,
            scale_factor: 1.5,
            theme: None,
            fonts: Vec::new(),
        },
        move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            let wrap = scope.create_element("div");
            wrap.set_attribute("style", "width: 120px; height: 60px;");
            let rs = RenderSurface {
                surface: Some(mounted),
            }
            .render(scope, &[]);
            wrap.append_child(&rs);
            root.append_child(&wrap);
            root
        },
    );

    ctx.update(&[]);
    let _ = ctx.scene();
    assert_eq!(
        surface.layout_size(),
        (180, 90),
        "the surface is told its physical layout size"
    );
    // The first scene measured the surface; its callback runs from then on.
    ctx.update(&[]);
    let before = calls.get();
    let patches = scene_patches(&mut ctx);
    assert!(
        calls.get() > before,
        "the render callback runs once the surface is measured"
    );
    assert_eq!(seen.get(), (180, 90), "the callback is handed that size");
    assert!(
        patches > 0,
        "the submitted frame is drawn into the scene (no image in it)"
    );

    // A render callback delivers a new frame into every scene, and each one is
    // drawn: nothing else changed, so a scene not told about the frame would be
    // served from the cache with the old one in it.
    use rinch_dom::perf::Counter;
    ctx.update(&[]);
    ctx.reset_perf();
    let _ = ctx.scene();
    let _ = ctx.scene();
    assert_eq!(
        ctx.perf_total().get(Counter::PaintCachedFrames),
        0,
        "a fresh frame rebuilds the scene"
    );

    // A frame submitted from outside the callback — another thread, a decoder —
    // is something to draw: the host is told, and the next update asks for a
    // redraw.
    ctx.update(&[]);
    let _ = ctx.scene();
    assert!(
        !ctx.needs_update(),
        "control: nothing pending after a scene"
    );
    let px = [0u8, 0, 255, 255].repeat(180 * 90);
    surface.writer().submit_frame(&px, 180, 90);
    assert!(ctx.needs_update(), "a submitted frame needs an update");
    let actions = ctx.update(&[]);
    assert!(
        actions.contains(&AppAction::RequestRedraw),
        "a submitted frame asks for a redraw: {actions:?}"
    );
}
