//! Review fixture for PR #1311 (issue #548): does the `wants_keyboard()`
//! narrowing also answer `false` for a plain button that reached
//! `FocusTarget::Node` via **Tab** (keyboard navigation), not a mouse click?
//!
//! The documented embed host pattern (`docs/src/guide/game-engine.md`,
//! "Input Routing") is:
//!
//! ```ignore
//! if ctx.wants_keyboard() {
//!     ctx.update(&[key_event]);
//! } else {
//!     game.handle_key(key);
//! }
//! ```
//!
//! If Tab-focus on a plain button also answers `wants_keyboard() == false`,
//! then a host following that pattern would never forward the *next* key
//! event (another Tab, or Enter to activate) to rinch once a plain button
//! holds the keyboard — Tab navigation and Enter/Space activation become
//! unreachable in embed for any `FocusTarget::Node` that isn't a registered
//! `on_key` widget, regardless of how it got focus.

#![cfg(any(feature = "gpu", feature = "embed"))]

use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

use rinch::embed::{RinchContext, RinchContextConfig};
use rinch::platform::{KeyCode, KeyRepeat, Modifiers, PlatformEvent};
use rinch::prelude::*;

type Job = Box<dyn FnOnce() + Send>;

fn on_ui_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    static SENDER: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    let sender = SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("rinch-test-ui-548r".into())
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

fn tab_down() -> PlatformEvent {
    PlatformEvent::KeyDown {
        key: KeyCode::Tab,
        logical_key: Some("Tab".to_string()),
        text: None,
        modifiers: Modifiers::default(),
        repeat: KeyRepeat::Unknown,
    }
}

/// Two plain buttons, no tabindex, no registered on_key — an ordinary HUD
/// dialog with two actions, the shape issue #548 itself describes.
fn two_plain_buttons_context() -> RinchContext {
    RinchContext::new(cfg(), |__scope: &mut RenderScope| {
        rsx! {
            div { style: "width: 800px; height: 600px;",
                button { style: "width: 100px; height: 40px;", "OK" }
                button { style: "width: 100px; height: 40px;", "Cancel" }
            }
        }
    })
}

/// Tab onto the first plain button (keyboard navigation, no mouse at all)
/// must answer `wants_keyboard() == true` — unlike the mouse-click case the
/// PR's own fixtures cover. The review of #1311 found the first cut of the
/// fix (registered-`on_key`-only) answered `false` here too, which would
/// have broken Tab navigation and Enter-activation for exactly the UI the
/// documented host pattern is meant to serve. The follow-up fix gates on
/// `is_focus_visible() || on_key registered`: Tab (and `NodeHandle::focus()`)
/// set the focus-visible ring, a mouse press claiming the node does not —
/// the same click-vs-keyboard-focus split a browser's `:focus-visible` makes.
#[test]
fn tab_focus_on_a_plain_button_still_wants_the_keyboard() {
    let (node_focused, wants_kb) = on_ui_thread(|| {
        let mut ctx = two_plain_buttons_context();
        // Nothing focused yet; Tab should move focus to the first button,
        // the same way `handle_tab`'s "nothing focused -> focus the first
        // tabbable" rule works elsewhere in the suite (CLAUDE.md's Keyboard
        // Focus section; `tab_order_tests.rs`).
        ctx.update(&[tab_down()]);
        (ctx.app().has_focused_node(), ctx.wants_keyboard())
    });
    assert!(
        node_focused,
        "Tab must claim FocusTarget::Node on the first button, or this test \
         proves nothing about keyboard-driven focus"
    );
    assert!(
        wants_kb,
        "a Tab-focused plain button must keep wants_keyboard() == true — \
         is_focus_visible() should be set by Tab even though nothing is \
         registered and no mouse was involved, so Tab navigation and \
         Enter-activation keep reaching rinch under the documented host \
         pattern"
    );
}
