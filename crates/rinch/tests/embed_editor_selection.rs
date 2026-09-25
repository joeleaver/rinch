//! An embedded editor's selection set by app code is drawn on the next
//! `update()` (#1001): `needs_update()` says there is something to do, and the
//! update runs the overlay pass that draws the highlight. A selection-only
//! transaction dirties no DOM, so without the owed pass nothing did.
//!
//! Requires the `embed` (or `gpu`) feature, and `desktop` for the editor:
//!     cargo test -p rinch --features embed,desktop --test embed_editor_selection

#![cfg(all(feature = "desktop", any(feature = "gpu", feature = "embed")))]

use std::cell::RefCell;
use std::rc::Rc;

use rinch::editor::EditorHandle;
use rinch::embed::{RinchContext, RinchContextConfig};
use rinch::prelude::*;
use rinch_editor_core::{Pos, Selection};

fn highlights(ctx: &RinchContext) -> usize {
    let doc = ctx.app().doc().expect("mounted").borrow();
    doc.query_selector_all("[data-pm-selection]")
        .into_iter()
        .filter(|id| {
            let l = doc.tree.nodes[id.0].layout;
            doc.tree.nodes[id.0].computed_style.display
                != rinch_dom::computed_style::DisplayValue::None
                && l.width > 0.0
                && l.height > 0.0
        })
        .count()
}

#[test]
fn a_selection_set_from_app_code_is_drawn_on_the_next_update() {
    // One test in this binary, so its thread is the only one to register as
    // the main thread.
    let slot: Rc<RefCell<Option<EditorHandle>>> = Rc::default();
    let slot_in = slot.clone();
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
            let (container, handle) = rinch::editor::mount_editor(scope);
            handle.load_html("<p>line 000</p><p>line 001</p><p>line 002</p>");
            container.set_attribute(
                "style",
                "width: 400px; font-size: 16px; line-height: 24px; font-family: sans-serif",
            );
            root.append_child(&container);
            *slot_in.borrow_mut() = Some(handle);
            root
        },
    );
    ctx.update(&[]);
    let handle = slot.borrow_mut().take().expect("mounted");
    handle.focus();
    for _ in 0..3 {
        ctx.update(&[]);
    }
    assert!(
        ctx.app().has_focused_contenteditable(),
        "precondition: focused"
    );
    assert_eq!(highlights(&ctx), 0, "control: no range yet");
    assert!(!ctx.needs_update(), "control: the context is idle");

    // The second paragraph, spanning 11..19.
    handle.set_selection(Selection::text(Pos(11), Pos(19)));
    assert!(
        ctx.needs_update(),
        "the selection change asks for an update"
    );
    ctx.update(&[]);
    assert_eq!(highlights(&ctx), 1, "the highlight is drawn");
    assert!(!ctx.needs_update(), "and the context is idle again");
}
