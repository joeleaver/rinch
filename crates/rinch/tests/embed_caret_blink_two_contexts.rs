//! A context with no focused editor does not stop another context's caret
//! blinking (issue #331).
//!
//! The blink clock is one per thread — one caret has the keyboard — and every
//! context now ticks it from `update`. A context whose own editor is not
//! focused ticks it with no target; that used to *retarget* the clock to
//! nothing, restoring the other context's caret to solid and resetting the
//! phase, so with two contexts updated in turn the caret never went off.
//!
//! Requires the `embed` (or `gpu`) feature, and `desktop` for the editor:
//!     cargo test -p rinch --features embed,desktop --test embed_caret_blink_two_contexts
//!
//! One test in this binary: `RinchContext::new` registers its thread as the
//! main thread, and the blink clock is thread-local.

#![cfg(all(feature = "desktop", any(feature = "gpu", feature = "embed")))]

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use rinch::editor::EditorHandle;
use rinch::embed::{RinchContext, RinchContextConfig};
use rinch::platform::Instant;
use rinch::prelude::*;

/// Whether the editor's caret is currently blinked off. `None` when there is
/// no caret element at all.
fn caret_hidden(ctx: &RinchContext) -> Option<bool> {
    let doc = ctx.app().doc().expect("mounted").borrow();
    let id = doc.query_selector_all("[data-pm-caret]").into_iter().next()?;
    let style = doc.tree.nodes[id.0]
        .attributes
        .get("style")
        .cloned()
        .unwrap_or_default();
    Some(style.contains("visibility: hidden"))
}

fn editor_context() -> (RinchContext, EditorHandle) {
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
            handle.load_html("<p>hello</p>");
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
    (ctx, handle)
}

#[test]
fn a_context_without_a_focused_editor_leaves_the_other_ones_blink_alone() {
    let (mut with_editor, handle) = editor_context();
    let mut plain = RinchContext::new(
        RinchContextConfig {
            width: 400,
            height: 300,
            scale_factor: 1.0,
            theme: None,
            fonts: Vec::new(),
        },
        |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_attribute("style", "width: 100px; height: 100px");
            root
        },
    );
    handle.focus();
    for _ in 0..3 {
        with_editor.update(&[]);
        plain.update(&[]);
    }
    assert!(with_editor.app().has_focused_contenteditable(), "precondition");
    assert_eq!(plain.next_wake(), None, "nothing blinks in the plain context");

    // A host frame loop: both contexts updated every frame, in turn.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut went_off = false;
    while Instant::now() < deadline && !went_off {
        std::thread::sleep(Duration::from_millis(20));
        with_editor.update(&[]);
        plain.update(&[]);
        went_off = caret_hidden(&with_editor) == Some(true);
    }
    assert!(went_off, "the plain context's update kept the caret solid");
}
