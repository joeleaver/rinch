//! The focused editor's caret blinks in an embedded context (issue #331).
//!
//! The desktop runtime ticks the blink clock after every event-loop turn and
//! arms a timed wake for the next toggle. `RinchContext::update` never ticked
//! it, so an embedded editor's caret was drawn once and never toggled. Now
//! `update` ticks it, and `next_wake` hands the host the instant of the next
//! toggle — what desktop's `ControlFlow::WaitUntil` is armed with — so an
//! event-driven host knows when to call `update` again with no input.
//!
//! Requires the `embed` (or `gpu`) feature, and `desktop` for the editor:
//!     cargo test -p rinch --features embed,desktop --test embed_caret_blink
//!
//! One test in this binary: `RinchContext::new` registers its thread as the
//! main thread, and the blink clock is thread-local.

#![cfg(all(feature = "desktop", any(feature = "gpu", feature = "embed")))]

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use rinch::editor::EditorHandle;
use rinch::embed::{RinchContext, RinchContextConfig};
use rinch::platform::{AppAction, Instant, PlatformEvent};
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
fn the_focused_editors_caret_blinks_in_an_embedded_context() {
    let (mut ctx, handle) = editor_context();
    assert_eq!(ctx.next_wake(), None, "control: nothing blinks unfocused");

    handle.focus();
    for _ in 0..3 {
        ctx.update(&[]);
    }
    assert!(ctx.app().has_focused_contenteditable(), "precondition");
    assert_eq!(caret_hidden(&ctx), Some(false), "the caret starts solid");
    let wake = ctx.next_wake().expect("a blinking caret arms a wake");
    assert!(
        wake <= Instant::now() + Duration::from_millis(530),
        "the wake is at most one half-period away"
    );
    assert!(
        wake > Instant::now() + Duration::from_millis(200),
        "and a real half-period, not already due"
    );
    assert!(
        !ctx.needs_update(),
        "nothing is due right after the blink restarts"
    );

    // Call `update` the way an event-driven host would: only when the wake it
    // was handed comes due. The caret must go off, then on again.
    let deadline = Instant::now() + Duration::from_secs(5);
    let (mut went_off, mut came_back) = (false, false);
    while Instant::now() < deadline && !came_back {
        let wake = ctx.next_wake().expect("still blinking");
        let now = Instant::now();
        if wake > now {
            std::thread::sleep(wake - now);
        }
        assert!(ctx.needs_update(), "a due wake is an update to run");
        let actions = ctx.update(&[]);
        match caret_hidden(&ctx) {
            Some(true) => {
                if !went_off {
                    assert!(
                        actions.contains(&AppAction::RequestRedraw),
                        "a toggle asks for a redraw: {actions:?}"
                    );
                }
                went_off = true;
            }
            Some(false) => came_back = went_off,
            None => panic!("the caret went away"),
        }
    }
    assert!(went_off, "the caret never blinked off");
    assert!(came_back, "the caret never blinked back on");

    // A blurred host window stops the blink with the caret solid, and arms no
    // wake — desktop's #316 rule. Blurred while the caret is OFF: blurring
    // while it is on cannot tell "restored to solid" from "left alone".
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && caret_hidden(&ctx) != Some(true) {
        std::thread::sleep(Duration::from_millis(20));
        ctx.update(&[]);
    }
    assert_eq!(caret_hidden(&ctx), Some(true), "control: the caret is off");
    ctx.update(&[PlatformEvent::WindowFocus(false)]);
    assert_eq!(ctx.next_wake(), None, "a blurred window arms no wake");
    assert_eq!(
        caret_hidden(&ctx),
        Some(false),
        "and leaves the caret solid"
    );
}
