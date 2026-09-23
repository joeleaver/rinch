//! A release delivered in the same `update()` as a drag move is judged
//! against the box where that move put the dragged element.
//!
//! The `MouseMove` drag arm defers its layout to the frame (update-path audit
//! F2.2), and `RinchContext::update` lays out only after the whole event list.
//! So `update(&[move, up])` used to hit-test the release against the layout
//! from *before* the move — the panel the pointer was dragging was still at
//! its old place, and its `onmouseup` never fired. `RinchApp::handle_event`
//! settles the owed layout before any non-move event (review of #881, D2).
//!
//! Requires the `embed` (or `gpu`) feature:
//!     cargo test -p rinch --features embed --test embed_drag_release

#![cfg(any(feature = "gpu", feature = "embed"))]

use std::cell::Cell;
use std::rc::Rc;

use rinch::embed::{RinchContext, RinchContextConfig};
use rinch::prelude::*;
use rinch_platform::{MouseButton, PlatformEvent};

#[test]
fn a_release_in_the_same_update_as_a_drag_move_sees_the_moved_box() {
    // One test in this binary, so its thread is the only one to register as
    // the main thread; no shared-worker harness is needed.
    let ups = Rc::new(Cell::new(0u32));
    let ups2 = ups.clone();
    let x = Signal::new(0.0f32);
    let mut ctx = RinchContext::new(
        RinchContextConfig {
            width: 800,
            height: 600,
            scale_factor: 1.0,
            theme: None,
            fonts: Vec::new(),
        },
        move |__scope: &mut RenderScope| {
            let ups = ups2.clone();
            rsx! {
                div {
                    div {
                        style: {move || format!(
                            "position: absolute; top: 0px; width: 50px; height: 50px; left: {}px",
                            x.get()
                        )},
                        onmouseup: move || ups.set(ups.get() + 1),
                    }
                }
            }
        },
    );
    ctx.update(&[]);

    rinch_core::Drag::absolute()
        .on_move(move |px, _| x.set(px - 25.0))
        .start();
    ctx.update(&[
        PlatformEvent::MouseMove { x: 400.0, y: 25.0 },
        PlatformEvent::MouseUp {
            x: 400.0,
            y: 25.0,
            button: MouseButton::Left,
        },
    ]);
    rinch_core::Drag::cancel();

    assert_eq!(
        ups.get(),
        1,
        "the release over the dragged panel reached its onmouseup"
    );
}
