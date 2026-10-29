//! Drives `rinch_core::reactive::poll_signal` in the browser (issue #1773).
//!
//! The native hosts call [`drain_polls`] once per frame from their own loop.
//! The browser has no rinch frame loop — it repaints by itself — so nothing
//! sampled a poll here and `poll_signal` was native-only. This module keeps one
//! drain scheduled while any poll is registered: `requestAnimationFrame` while
//! some poll is [`PollRate::EveryFrame`](rinch_core::reactive::PollRate), else
//! `setTimeout` until the soonest timed poll is due, nothing once the registry
//! is empty (`drain_polls` reaps a poll whose signal was freed). A new
//! registration reschedules, since a new poll is due at once.
//!
//! While the tab is hidden the browser runs no animation frames, so a page
//! with an every-frame poll samples nothing until it is shown again.

use std::cell::{Cell, RefCell};

use rinch_core::reactive::{drain_polls, next_poll_due, polls_want_every_frame, set_poll_waker};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

#[derive(Clone, Copy)]
enum Pending {
    Frame(i32),
    Timeout(i32),
}

thread_local! {
    static PENDING: Cell<Option<Pending>> = const { Cell::new(None) };
    /// One closure for every wake, made on first use and kept for the page's life.
    static ON_WAKE: RefCell<Option<Closure<dyn FnMut()>>> = const { RefCell::new(None) };
}

/// Install the waker (from [`install_web_runtime`](super::install_web_runtime))
/// and schedule a drain for any poll registered before it.
pub(crate) fn install() {
    set_poll_waker(wake);
    wake();
}

/// A poll was registered: replace whatever drain is pending with one for the
/// registry as it is now.
fn wake() {
    if let Some(pending) = PENDING.with(|p| p.take())
        && let Some(window) = web_sys::window()
    {
        match pending {
            Pending::Frame(id) => {
                let _ = window.cancel_animation_frame(id);
            }
            Pending::Timeout(id) => window.clear_timeout_with_handle(id),
        }
    }
    schedule();
}

fn schedule() {
    if PENDING.with(|p| p.get()).is_some() {
        return;
    }
    let every_frame = polls_want_every_frame();
    let due = if every_frame { None } else { next_poll_due() };
    if !every_frame && due.is_none() {
        return;
    }
    let Some(window) = web_sys::window() else {
        return;
    };
    ON_WAKE.with(|cb| {
        let mut cb = cb.borrow_mut();
        let cb = cb.get_or_insert_with(|| Closure::new(on_wake));
        let f = cb.as_ref().unchecked_ref();
        let pending = if every_frame {
            window.request_animation_frame(f).ok().map(Pending::Frame)
        } else {
            let ms = due.map_or(0, |d| d.as_micros().div_ceil(1000));
            let ms = i32::try_from(ms).unwrap_or(i32::MAX);
            window
                .set_timeout_with_callback_and_timeout_and_arguments_0(f, ms)
                .ok()
                .map(Pending::Timeout)
        };
        PENDING.with(|p| p.set(pending));
    });
}

fn on_wake() {
    PENDING.with(|p| p.set(None));
    // One transaction for every poll that changed, as an event handler is.
    rinch_core::batch(drain_polls);
    schedule();
}
