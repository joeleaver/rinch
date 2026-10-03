//! Review fixture for PR #1311 (issue #548): an open native `<select>` popup
//! must make `wants_keyboard()` answer `true`, in a pure `embed` build (no
//! `desktop` feature). `FocusTarget::Select` is covered by neither
//! `has_focused_input` nor `has_focused_contenteditable`, and the original
//! #1311 fix's `has_focused_key_consumer()` fell through its `_` arm to
//! `false` for it too — a pre-existing gap (unchanged by #1311's narrowing;
//! `has_focused_node()` never matched `Select` either) that the review
//! caught and folded into the same follow-up fix: `has_focused_key_consumer`
//! now has an explicit `FocusTarget::Select(_) => true` arm, since an open
//! popup's own arrow/Enter/Escape handling needs every key routed to it.

#![cfg(any(feature = "gpu", feature = "embed"))]

use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

use rinch::embed::{RinchContext, RinchContextConfig};
use rinch::platform::{MouseButton, PlatformEvent};
use rinch::prelude::*;

type Job = Box<dyn FnOnce() + Send>;

fn on_ui_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    static SENDER: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    let sender = SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("rinch-test-ui-548sel".into())
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

fn select_context() -> RinchContext {
    RinchContext::new(cfg(), |__scope: &mut RenderScope| {
        rsx! {
            select { style: "width: 800px; height: 600px;",
                option { value: "a", "A" }
                option { value: "b", "B" }
            }
        }
    })
}

#[test]
fn an_open_select_popup_in_embed_wants_the_keyboard() {
    let (popup_open, wants_kb) = on_ui_thread(|| {
        let mut ctx = select_context();
        ctx.update(&[press(400.0, 300.0), release(400.0, 300.0)]);
        let popup_open = {
            let doc = ctx.app().doc().expect("mounted").borrow();
            !doc.query_selector_all(".rinch-nsel-panel").is_empty()
        };
        (popup_open, ctx.wants_keyboard())
    });
    // Positive control: the click really did open the popup — otherwise
    // `wants_keyboard() == true` would be vacuous.
    assert!(
        popup_open,
        "clicking the closed <select> must open its popup (.rinch-nsel-panel), \
         or this test proves nothing"
    );
    assert!(
        wants_kb,
        "an open native <select> popup must make wants_keyboard() == true — \
         its own arrow/Enter/Escape handling needs every key, and before \
         this fix FocusTarget::Select fell through has_focused_key_consumer's \
         `_` arm to false (a pre-existing gap, not caused by #1311's \
         narrowing, folded into the same follow-up fix)"
    );
}
