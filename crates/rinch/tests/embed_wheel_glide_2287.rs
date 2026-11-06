//! A host that draws only when `RinchContext::needs_update()` says so must
//! hear that a wheel glide (#2287) is running, or the glide stalls after its
//! first frame (review of PR #2290).
//!
//!     cargo test -p rinch --features embed --test embed_wheel_glide_2287

#![cfg(any(feature = "gpu", feature = "embed"))]

use rinch::embed::{RinchContext, RinchContextConfig};
use rinch::prelude::*;
use rinch_platform::PlatformEvent;

#[test]
fn needs_update_is_true_while_a_wheel_glide_runs() {
    let mut ctx = RinchContext::new(
        RinchContextConfig {
            width: 800,
            height: 600,
            scale_factor: 1.0,
            theme: None,
            fonts: Vec::new(),
        },
        |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            let scroller = scope.create_element("div");
            scroller.set_attribute(
                "style",
                "position: absolute; left: 0px; top: 0px; width: 200px; height: 300px; \
                 overflow-y: auto",
            );
            let inner = scope.create_element("div");
            inner.set_attribute("style", "height: 3000px");
            scroller.append_child(&inner);
            root.append_child(&scroller);
            root
        },
    );
    ctx.update(&[]);
    ctx.update(&[]);
    assert!(!ctx.needs_update(), "control: idle before the wheel");
    ctx.update(&[
        PlatformEvent::MouseMove { x: 100.0, y: 150.0 },
        PlatformEvent::MouseWheelLines {
            x: 100.0,
            y: 150.0,
            lines_x: 0.0,
            lines_y: -1.0,
        },
    ]);
    assert!(
        ctx.app_mut().wheel_scroll_animating(),
        "positive control: the notch glides"
    );
    assert!(ctx.needs_update(), "a glide is running and needs frames");
}
