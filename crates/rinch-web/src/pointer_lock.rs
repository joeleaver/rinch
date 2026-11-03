//! The browser's Pointer Lock API behind `rinch::window`'s pointer lock
//! (issue #2092).
//!
//! Installed once per page, at the first mount: the [`PointerLockHost`] that
//! calls `requestPointerLock` / `exitPointerLock`, and three document
//! listeners — `pointerlockchange` and `pointerlockerror`, which tell rinch
//! what the browser granted or ended (a granted request, Escape, a tab switch,
//! the locked element's removal) and give back any lock nobody here holds (a
//! grant that arrives after `exit_pointer_lock` withdrew its request, or on an
//! element other than the one asked for), and `mousemove`, whose `movementX`/`Y` are
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
            set_requested(Some(Request {
                element,
                granted: true,
            }));
            return Ok(Some(PointerLockMode::Locked));
        }
        set_requested(Some(Request {
            element: element.clone(),
            granted: false,
        }));
        request_pointer_lock(&element);
        Ok(None)
    }

    fn unlock(&mut self) {
        // A request still pending is withdrawn: its late grant finds nothing
        // requested, and is given back (`on_change`).
        set_requested(None);
        if let Some(document) = document()
            && document.pointer_lock_element().is_some()
        {
            document.exit_pointer_lock();
        }
    }
}

/// What rinch asked the browser to lock, and whether it has.
struct Request {
    element: web_sys::Element,
    granted: bool,
}

thread_local! {
    static REQUESTED: std::cell::RefCell<Option<Request>> = const { std::cell::RefCell::new(None) };
}

fn set_requested(request: Option<Request>) {
    REQUESTED.with(|r| *r.borrow_mut() = request);
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

/// Give back a lock nobody here holds: one granted after its request was
/// withdrawn, or on an element other than the one asked for (a refused move
/// from A to B leaves A locked while rinch waits for B).
fn give_back(document: &web_sys::Document) {
    set_requested(None);
    document.exit_pointer_lock();
    rinch::window::pointer_lock_changed(None);
}

/// `pointerlockchange`: the browser granted, moved or ended a lock.
fn on_change() {
    let Some(document) = document() else {
        return;
    };
    let locked = document.pointer_lock_element();
    let asked = REQUESTED.with(|r| {
        r.borrow()
            .as_ref()
            .map(|req| (req.element.clone(), req.granted))
    });
    match (locked, asked) {
        // The request granted.
        (Some(el), Some((req, _))) if el == req => {
            REQUESTED.with(|r| {
                if let Some(req) = r.borrow_mut().as_mut() {
                    req.granted = true;
                }
            });
            if !rinch::window::pointer_lock_changed(Some(PointerLockMode::Locked)) {
                give_back(&document);
            }
        }
        // Locked, but not what rinch asked for, or after it stopped asking.
        (Some(_), _) => give_back(&document),
        // A granted lock ended (Escape, a tab switch, the element removed).
        (None, Some((_, true))) => {
            set_requested(None);
            rinch::window::pointer_lock_changed(None);
        }
        // Unlocked while a request is pending (the browser released an
        // earlier lock on its way to the new one): keep waiting.
        (None, Some((_, false))) => {}
        (None, None) => {
            rinch::window::pointer_lock_changed(None);
        }
    }
}

/// `pointerlockerror`: a request was refused. A pending one ends; whatever the
/// browser still holds for an earlier request is given back, since rinch no
/// longer has it on record.
fn on_error() {
    let Some(document) = document() else {
        return;
    };
    let pending = REQUESTED.with(|r| r.borrow().as_ref().is_some_and(|req| !req.granted));
    if !pending {
        return;
    }
    if document.pointer_lock_element().is_some() {
        give_back(&document);
    } else {
        set_requested(None);
        rinch::window::pointer_lock_changed(None);
    }
}

/// Install the host and the document listeners. Called once per page; the
/// listeners live as long as it.
pub(crate) fn install() {
    rinch::window::set_pointer_lock_host(Some(Box::new(BrowserPointerLock)));
    let Some(document) = document() else {
        return;
    };
    for (event, handler) in [
        ("pointerlockchange", on_change as fn()),
        ("pointerlockerror", on_error as fn()),
    ] {
        let listener = Closure::<dyn Fn()>::new(handler);
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
