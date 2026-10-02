//! Issue #548: a mouse-clicked plain `<button>` must not make
//! `RinchContext::wants_keyboard()` answer `true`.
//!
//! `has_focused_node` (`FocusTarget::Node`, issue #228) is claimed by **any**
//! click on a `<button>`/`<a href>` — tags that are focusable with no
//! `tabindex` needed since issue #252 — not only by Tab. Before this fix,
//! `wants_keyboard()` OR'd in `has_focused_node()` unconditionally, so an
//! embed host following the documented contract ("route keyboard to rinch
//! while `wants_keyboard()`") stopped seeing its own Esc/hotkey presses the
//! instant the user clicked *any* button, with no visible keyboard focus or
//! Tab involved. The fix: `wants_keyboard()` now asks
//! `has_focused_key_consumer()`, which is only `true` for a generic node that
//! *registered* `FocusEntry::on_key` — a custom widget that actually reads
//! keys beyond Enter/Space.
//!
//! Same cross-test harness as `tests/multi_context.rs`: `RinchContext::new`
//! registers the creating thread as THE main thread process-wide.
//!
//! Requires the `embed` (or `gpu`) feature:
//!     cargo test -p rinch --features embed --test embed_wants_keyboard_node_focus

#![cfg(any(feature = "gpu", feature = "embed"))]

use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

use rinch::embed::{RinchContext, RinchContextConfig};
use rinch::platform::{MouseButton, PlatformEvent};
use rinch::prelude::*;

// ── harness ──────────────────────────────────────────────────────────────────

type Job = Box<dyn FnOnce() + Send>;

fn on_ui_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    static SENDER: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    let sender = SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("rinch-test-ui".into())
            .spawn(move || {
                for job in rx {
                    job();
                }
            })
            .expect("spawn ui worker");
        Mutex::new(tx)
    });

    let (result_tx, result_rx) = mpsc::channel();
    let job: Job = Box::new(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        let _ = result_tx.send(result);
    });
    sender
        .lock()
        .expect("ui sender lock")
        .send(job)
        .expect("ui worker alive");
    match result_rx.recv().expect("ui worker responded") {
        Ok(v) => v,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

fn cfg() -> RinchContextConfig {
    RinchContextConfig {
        width: 800,
        height: 600,
        scale_factor: 1.0,
        theme: None,
        fonts: Vec::new(),
    }
}

fn press(x: f32, y: f32) -> PlatformEvent {
    PlatformEvent::MouseDown {
        x,
        y,
        button: MouseButton::Left,
    }
}

fn release(x: f32, y: f32) -> PlatformEvent {
    PlatformEvent::MouseUp {
        x,
        y,
        button: MouseButton::Left,
    }
}

/// A full-window plain `<button>` with no `tabindex` and no registered focus
/// target — the exact shape the issue reports (a game's modal "OK" button).
fn plain_button_context() -> RinchContext {
    RinchContext::new(cfg(), |__scope: &mut RenderScope| {
        rsx! {
            button { style: "width: 800px; height: 600px;", "OK" }
        }
    })
}

/// A full-window `<input>` text field — the case that must keep claiming the
/// keyboard, as a positive control proving `wants_keyboard()` still answers
/// `true` for the targets it always should. The text engine only claims an
/// `<input>` that carries `data-oninput` (`click_handling.rs`), which the
/// `rsx!` macro writes for an `oninput`/`onchange`/`value_fn` prop — a bare
/// `input {}` with none of those is not a text target at all.
fn text_input_context() -> RinchContext {
    RinchContext::new(cfg(), |__scope: &mut RenderScope| {
        rsx! {
            input {
                style: "width: 800px; height: 600px;",
                oninput: move |_v: String| {},
            }
        }
    })
}

/// A full-window custom widget with an explicit `tabindex` that registers
/// `on_key` (e.g. a Tree view reading arrow keys) — the case that must keep
/// claiming the keyboard even though it is `FocusTarget::Node` exactly like
/// the plain button.
fn registered_key_widget_context() -> RinchContext {
    RinchContext::new(cfg(), |__scope: &mut RenderScope| {
        let div = __scope.create_element("div");
        div.set_attribute("tabindex", "0");
        div.set_attribute("style", "width: 800px; height: 600px;");
        register_focus_target(
            &div,
            FocusEntry::new().on_key(|_k| true), // consumes everything
        );
        div
    })
}

/// A registered node with no `on_key` at all (only `on_focus_gained`) — same
/// shape as a plain button for this question: it should NOT claim the
/// keyboard either.
fn registered_no_key_widget_context() -> RinchContext {
    RinchContext::new(cfg(), |__scope: &mut RenderScope| {
        let div = __scope.create_element("div");
        div.set_attribute("tabindex", "0");
        div.set_attribute("style", "width: 800px; height: 600px;");
        register_focus_target(&div, FocusEntry::new().on_focus_gained(|| {}));
        div
    })
}

// ── tests ────────────────────────────────────────────────────────────────────

/// The core repro: click the plain button, and `wants_keyboard()` must be
/// `false` — the whole window is the button, so any click lands on it.
#[test]
fn a_mouse_clicked_plain_button_does_not_want_the_keyboard() {
    let (node_focused, wants_kb) = on_ui_thread(|| {
        let mut ctx = plain_button_context();
        ctx.update(&[press(400.0, 300.0)]);
        (ctx.app().has_focused_node(), ctx.wants_keyboard())
    });
    // Positive control: the click really did claim the generic-node focus —
    // otherwise `wants_keyboard() == false` would be vacuous.
    assert!(
        node_focused,
        "the click must claim FocusTarget::Node on the button, or this test \
         proves nothing"
    );
    assert!(
        !wants_kb,
        "a plain <button> with no registered on_key must not make \
         wants_keyboard() true just because a mouse click focused it (#548)"
    );
}

/// A focused text `<input>` must still claim the keyboard (unaffected by the
/// #548 narrowing).
#[test]
fn a_focused_text_input_still_wants_the_keyboard() {
    let wants_kb = on_ui_thread(|| {
        let mut ctx = text_input_context();
        ctx.update(&[press(400.0, 300.0), release(400.0, 300.0)]);
        ctx.wants_keyboard()
    });
    assert!(
        wants_kb,
        "a focused text input must still want the keyboard"
    );
}

/// A custom widget that registered `on_key` (arrow-key navigation, etc.) must
/// still claim the keyboard when focused by a click — the runtime cannot know
/// which keys it wants without seeing them.
#[test]
fn a_registered_on_key_widget_still_wants_the_keyboard_when_clicked() {
    let (node_focused, wants_kb) = on_ui_thread(|| {
        let mut ctx = registered_key_widget_context();
        ctx.update(&[press(400.0, 300.0)]);
        (ctx.app().has_focused_node(), ctx.wants_keyboard())
    });
    assert!(node_focused, "the click must claim FocusTarget::Node");
    assert!(
        wants_kb,
        "a registered on_key target must keep wants_keyboard() == true"
    );
}

/// A registered node with no `on_key` (only focus/blur notifications) behaves
/// like the plain button: it does not need the keyboard.
#[test]
fn a_registered_node_with_no_on_key_does_not_want_the_keyboard() {
    let (node_focused, wants_kb) = on_ui_thread(|| {
        let mut ctx = registered_no_key_widget_context();
        ctx.update(&[press(400.0, 300.0)]);
        (ctx.app().has_focused_node(), ctx.wants_keyboard())
    });
    assert!(node_focused, "the click must claim FocusTarget::Node");
    assert!(
        !wants_kb,
        "a registered target with no on_key must not want the keyboard"
    );
}
