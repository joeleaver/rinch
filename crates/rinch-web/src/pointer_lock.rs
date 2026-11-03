//! The browser's Pointer Lock API behind `rinch::window`'s pointer lock
//! (issue #2092).
//!
//! Installed once per page, at the first mount: the [`PointerLockHost`] that
//! calls `requestPointerLock` / `exitPointerLock`, and three document
//! listeners — `pointerlockchange` and `pointerlockerror`, which tell rinch
//! what the browser granted or ended (a granted request, Escape, a tab switch,
//! the locked element's removal), and `mousemove`, whose `movementX`/`Y` are
//! the raw motion `rinch::window::on_pointer_motion` and a locked
//! `RenderSurface` hear.
//!
//! The browser decides, and decides later: `requestPointerLock` is honoured
//! only in a user gesture (transient activation), and the answer is an event.
//! So [`PointerLockHost::lock`] answers "pending", and the change listener
//! reports the outcome. Escape is the browser's: it exits the lock itself,
//! before the page can act on the key.

use rinch::window::{PointerLockError, PointerLockHost, PointerLockMode, PointerLockTarget};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

fn document() -> Option<web_sys::Document> {
    web_sys::window()?.document()
}

struct BrowserPointerLock;

impl PointerLockHost for BrowserPointerLock {
    fn lock(
        &mut self,
        target: &PointerLockTarget,
    ) -> Result<Option<PointerLockMode>, PointerLockError> {
        let document = document().ok_or(PointerLockError::Unsupported)?;
        let element = match target {
            PointerLockTarget::Window => document
                .document_element()
                .ok_or(PointerLockError::Unsupported)?,
            PointerLockTarget::Element(handle) => crate::web_document::element_of(handle)
                .filter(|e| e.is_connected())
                .ok_or(PointerLockError::NotConnected)?,
            _ => return Err(PointerLockError::Unsupported),
        };
        // Already locked to this element: the browser fires no change.
        if document.pointer_lock_element().as_ref() == Some(&element) {
            return Ok(Some(PointerLockMode::Locked));
        }
        request_pointer_lock(&element);
        Ok(None)
    }

    fn unlock(&mut self) {
        if let Some(document) = document()
            && document.pointer_lock_element().is_some()
        {
            document.exit_pointer_lock();
        }
    }
}

thread_local! {
    /// Swallows the rejection of the promise newer browsers return from
    /// `requestPointerLock`: the refusal also arrives as `pointerlockerror`,
    /// which is what reports it, and an unhandled rejection would only log.
    static IGNORE_REJECTION: Closure<dyn FnMut(JsValue)> = Closure::new(|_: JsValue| {});
}

/// `element.requestPointerLock()`, called through the property so that the
/// promise Chrome and Firefox return (older bindings declare `undefined`) can
/// be caught.
fn request_pointer_lock(element: &web_sys::Element) {
    let Ok(request) = js_sys::Reflect::get(element, &JsValue::from_str("requestPointerLock"))
    else {
        return;
    };
    let Some(request) = request.dyn_ref::<js_sys::Function>() else {
        return;
    };
    if let Ok(result) = request.call0(element)
        && let Some(promise) = result.dyn_ref::<js_sys::Promise>()
    {
        IGNORE_REJECTION.with(|ignore| {
            let _ = promise.catch(ignore);
        });
    }
}

/// What the browser holds now, for rinch.
fn report_lock_state() {
    let locked = document().and_then(|d| d.pointer_lock_element()).is_some();
    rinch::window::pointer_lock_changed(locked.then_some(PointerLockMode::Locked));
}

/// Install the host and the document listeners. Called once per page; the
/// listeners live as long as it.
pub(crate) fn install() {
    rinch::window::set_pointer_lock_host(Some(Box::new(BrowserPointerLock)));
    let Some(document) = document() else {
        return;
    };
    // A refusal (`pointerlockerror`) is reported as what the browser holds,
    // not as "unlocked": a request to move a held lock elsewhere can fail
    // while the first lock stays.
    for event in ["pointerlockchange", "pointerlockerror"] {
        let listener = Closure::<dyn Fn()>::new(report_lock_state);
        let _ = document.add_event_listener_with_callback(event, listener.as_ref().unchecked_ref());
        listener.forget();
    }
    let on_move = Closure::<dyn Fn(web_sys::Event)>::new(|event: web_sys::Event| {
        if let Some(mouse) = event.dyn_ref::<web_sys::MouseEvent>() {
            rinch::window::dispatch_pointer_motion(
                f64::from(mouse.movement_x()),
                f64::from(mouse.movement_y()),
            );
        }
    });
    // Capture phase, passive: before anything can stop it, and never in the
    // way of scrolling.
    let options = web_sys::AddEventListenerOptions::new();
    options.set_capture(true);
    options.set_passive(true);
    let _ = document.add_event_listener_with_callback_and_add_event_listener_options(
        "mousemove",
        on_move.as_ref().unchecked_ref(),
        &options,
    );
    on_move.forget();
}
