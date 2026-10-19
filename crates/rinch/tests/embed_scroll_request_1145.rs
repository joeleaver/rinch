//! #1145 through an embedded `RinchContext`: a `scroll_into_view` requested
//! with no other change (here from outside any handler, as a timer or an
//! effect elsewhere would) is applied by the next `update()`, and
//! `needs_update()` says so.
//!
//! Requires the `embed` (or `gpu`) feature:
//!     cargo test -p rinch --features embed --test embed_scroll_request_1145

#![cfg(any(feature = "gpu", feature = "embed"))]

use std::cell::RefCell;
use std::rc::Rc;

use rinch::embed::{RinchContext, RinchContextConfig};
use rinch::prelude::*;

#[test]
fn a_scroll_request_with_no_other_change_is_applied_by_the_next_update() {
    let held: Rc<RefCell<Option<(NodeHandle, NodeHandle)>>> = Rc::new(RefCell::new(None));
    let h = held.clone();
    let mut ctx = RinchContext::new(
        RinchContextConfig {
            width: 800,
            height: 600,
            scale_factor: 1.0,
            theme: None,
            fonts: Vec::new(),
        },
        move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_attribute("style", "height: 100px; overflow-y: auto; width: 200px");
            let tall = scope.create_element("div");
            tall.set_attribute("style", "height: 1000px");
            let target = scope.create_element("div");
            target.set_attribute("style", "height: 20px");
            root.append_child(&tall);
            root.append_child(&target);
            *h.borrow_mut() = Some((root.clone(), target));
            root
        },
    );
    ctx.update(&[]);
    ctx.update(&[]);
    assert!(!ctx.needs_update(), "idle before the request");

    let (root, target) = held.borrow().clone().unwrap();
    target.scroll_into_view();
    assert!(ctx.needs_update(), "a queued scroll is something to do");
    ctx.update(&[]);
    assert_eq!(root.scroll_top(), 920.0, "the next update applied it");
}
