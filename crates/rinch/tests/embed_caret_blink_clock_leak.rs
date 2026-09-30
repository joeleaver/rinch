//! The per-document caret blink clocks do not outlive their documents
//! (issue #1149, review of PR #1168).
//!
//! A clock is kept for each document that is blinking a focused editor. A
//! context dropped with its editor still focused releases it through the
//! editor's unregistration. The case that leaked: the focused editor unmounts
//! first. Its clock went with it, but the focus arbiter still names the
//! editor until the next key, so the next tick created a clock for an editor
//! that no longer exists — and dropping the context then stranded it for the
//! life of the thread, one per context (`doc_key`s are never reused).
//!
//! Requires the `embed` (or `gpu`) feature, and `desktop` for the editor:
//!     cargo test -p rinch --features embed,desktop --test embed_caret_blink_clock_leak
//!
//! One test in this binary: the clocks are thread-local, and `Signal::set`
//! needs the thread `RinchContext::new` registered as the main thread.

#![cfg(all(feature = "desktop", any(feature = "gpu", feature = "embed")))]

use std::cell::RefCell;
use std::rc::Rc;

use rinch::editor::{EditorHandle, blink_clock_count};
use rinch::embed::{RinchContext, RinchContextConfig};
use rinch::prelude::*;

/// A context whose one editor is mounted while `show` is true.
fn context(show: Signal<bool>) -> (RinchContext, EditorHandle) {
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
            let slot_in = slot_in.clone();
            rinch_core::show_dom(
                scope,
                &root,
                move || show.get(),
                move |scope: &mut RenderScope| {
                    let (container, handle) = rinch::editor::mount_editor(scope);
                    handle.load_html("<p>hello</p>");
                    container
                        .set_attribute("style", "width: 400px; font-size: 16px; line-height: 24px");
                    *slot_in.borrow_mut() = Some(handle);
                    container
                },
                None::<fn(&mut RenderScope) -> NodeHandle>,
            );
            root
        },
    );
    ctx.update(&[]);
    let handle = slot.borrow_mut().take().expect("mounted");
    (ctx, handle)
}

fn pump(ctx: &mut RinchContext) {
    for _ in 0..3 {
        ctx.update(&[]);
    }
}

#[test]
fn blink_clocks_do_not_outlive_their_documents() {
    assert_eq!(blink_clock_count(), 0, "precondition: no clock yet");

    // Dropped with its editor still focused.
    for _ in 0..3 {
        let show = Signal::new(true);
        let (mut ctx, handle) = context(show);
        handle.focus();
        pump(&mut ctx);
        assert!(ctx.app().has_focused_contenteditable(), "precondition");
        assert_eq!(
            blink_clock_count(),
            1,
            "control: a focused editor has a clock"
        );
        drop(ctx);
        assert_eq!(blink_clock_count(), 0, "a dropped context kept its clock");
    }

    // The focused editor unmounts, the context ticks on, then is dropped.
    for _ in 0..3 {
        let show = Signal::new(true);
        let (mut ctx, handle) = context(show);
        handle.focus();
        pump(&mut ctx);
        assert_eq!(
            blink_clock_count(),
            1,
            "control: a focused editor has a clock"
        );
        show.set(false);
        pump(&mut ctx);
        assert_eq!(ctx.next_wake(), None, "an unmounted editor blinks nothing");
        assert_eq!(
            blink_clock_count(),
            0,
            "a tick kept a clock for an editor that unmounted"
        );
        drop(ctx);
        assert_eq!(
            blink_clock_count(),
            0,
            "unmount-then-drop left a clock behind"
        );
    }
}
