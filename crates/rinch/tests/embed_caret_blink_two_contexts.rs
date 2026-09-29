//! Two embedded contexts on one thread, each with a focused editor, both blink
//! (issue #1149).
//!
//! The blink clock was one per thread: one anchor, one target. Every document
//! on the thread ticks it — a desktop window and each `RinchContext` alike —
//! so two documents that each had a focused editor retargeted it on every
//! tick, which restored the other caret to solid and restarted the phase.
//! Neither caret ever went off. The clock is kept per document now
//! (`DomDocument::doc_key`, the #134 rule).
//!
//! Requires the `embed` (or `gpu`) feature, and `desktop` for the editor:
//!     cargo test -p rinch --features embed,desktop --test embed_caret_blink_two_contexts
//!
//! One test in this binary: the blink clock is thread-local.

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
    let id = doc
        .query_selector_all("[data-pm-caret]")
        .into_iter()
        .next()?;
    let style = doc.tree.nodes[id.0]
        .attributes
        .get("style")
        .cloned()
        .unwrap_or_default();
    Some(style.contains("visibility: hidden"))
}

fn editor_context(text: &str) -> (RinchContext, EditorHandle) {
    let slot: Rc<RefCell<Option<EditorHandle>>> = Rc::default();
    let slot_in = slot.clone();
    let html = format!("<p>{text}</p>");
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
            handle.load_html(&html);
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
fn two_contexts_with_focused_editors_both_blink() {
    let (mut a, ha) = editor_context("alpha");
    let (mut b, hb) = editor_context("beta");
    // One focus request at a time: each lands in its own context's update.
    ha.focus();
    a.update(&[]);
    hb.focus();
    b.update(&[]);
    for _ in 0..3 {
        a.update(&[]);
        b.update(&[]);
    }
    assert!(a.app().has_focused_contenteditable(), "precondition: a");
    assert!(b.app().has_focused_contenteditable(), "precondition: b");
    assert_eq!(caret_hidden(&a), Some(false), "a's caret starts solid");
    assert_eq!(caret_hidden(&b), Some(false), "b's caret starts solid");

    // Tick both the way a host pumping two contexts does: in turn, every
    // 20 ms. Each caret must go off and come back on.
    let deadline = Instant::now() + Duration::from_secs(5);
    let (mut a_off, mut a_back) = (false, false);
    let (mut b_off, mut b_back) = (false, false);
    while Instant::now() < deadline && !(a_back && b_back) {
        std::thread::sleep(Duration::from_millis(20));
        a.update(&[]);
        b.update(&[]);
        match caret_hidden(&a) {
            Some(true) => a_off = true,
            Some(false) => a_back |= a_off,
            None => panic!("a's caret went away"),
        }
        match caret_hidden(&b) {
            Some(true) => b_off = true,
            Some(false) => b_back |= b_off,
            None => panic!("b's caret went away"),
        }
    }
    assert!(
        a_off && a_back,
        "a's caret never blinked (off {a_off}, back {a_back})"
    );
    assert!(
        b_off && b_back,
        "b's caret never blinked (off {b_off}, back {b_back})"
    );

    // Each context arms its own wake while its caret blinks.
    assert!(a.next_wake().is_some(), "a arms a wake");
    assert!(b.next_wake().is_some(), "b arms a wake");

    // Blurring one host window stops that blink, with its caret solid, and
    // leaves the other blinking. Blur `a` while its caret is OFF, so
    // "restored to solid" is distinguishable from "left alone".
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && caret_hidden(&a) != Some(true) {
        std::thread::sleep(Duration::from_millis(20));
        a.update(&[]);
        b.update(&[]);
    }
    assert_eq!(caret_hidden(&a), Some(true), "control: a's caret is off");
    a.update(&[rinch::platform::PlatformEvent::WindowFocus(false)]);
    assert_eq!(a.next_wake(), None, "a blurred window arms no wake");
    assert_eq!(caret_hidden(&a), Some(false), "and leaves its caret solid");

    let deadline = Instant::now() + Duration::from_secs(5);
    let (mut b_off, mut b_back) = (false, false);
    while Instant::now() < deadline && !b_back {
        std::thread::sleep(Duration::from_millis(20));
        a.update(&[]);
        b.update(&[]);
        match caret_hidden(&b) {
            Some(true) => b_off = true,
            Some(false) => b_back |= b_off,
            None => panic!("b's caret went away"),
        }
        assert_eq!(caret_hidden(&a), Some(false), "a stays solid while blurred");
    }
    assert!(b_off && b_back, "b stopped blinking when a was blurred");
    assert!(b.next_wake().is_some(), "b still arms a wake");
}
