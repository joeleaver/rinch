//! Keyboard event interception.

use std::cell::RefCell;
use std::rc::Rc;

/// Keyboard event data for the global keyboard interceptor.
#[derive(Debug, Clone)]
pub struct KeyEventData {
    /// The logical key value (e.g., "a", "Enter", "Backspace")
    pub key: String,
    /// The physical key code (e.g., "KeyA", "Enter")
    pub code: String,
    /// Whether Ctrl/Cmd is pressed
    pub ctrl: bool,
    /// Whether Shift is pressed
    pub shift: bool,
    /// Whether Alt is pressed
    pub alt: bool,
    /// Whether Meta/Super is pressed
    pub meta: bool,
}

/// Type alias for the keyboard interceptor callback.
/// Returns true if the event was handled (should not propagate to the runtime).
pub type KeyboardInterceptor = Rc<dyn Fn(&KeyEventData) -> bool>;

thread_local! {
    /// The one interceptor slot for the whole thread.
    ///
    /// Deliberately **not** keyed by document, unlike the pointer-capture drag
    /// (issue #139): unscoped and last-wins is a real hazard here — two
    /// documents on one thread (a desktop app and its DevTools window, two
    /// embedded `RinchContext`s) share this slot, so the second
    /// [`set_keyboard_interceptor`] silently displaces the first and every
    /// document's keys then reach whichever registered last. It stays a single
    /// slot only because there is no in-tree registrant today, so it is an API
    /// hazard rather than a live bug, and because the fix belongs with the
    /// `(doc_key, node_id)` focus registry in issue #147 — keyboard routing is
    /// one decision, and giving this its own parallel map would have to be
    /// unpicked to land it.
    static KEYBOARD_INTERCEPTOR: RefCell<Option<KeyboardInterceptor>> = RefCell::new(None);
}

/// Set the global keyboard interceptor.
///
/// Only one interceptor can be active at a time, **per thread, not per
/// document**: a second call from another document on the same thread replaces
/// the first (issue #139; the per-document routing lands with issue #147).
pub fn set_keyboard_interceptor<F>(cb: F)
where
    F: Fn(&KeyEventData) -> bool + 'static,
{
    KEYBOARD_INTERCEPTOR.with(|i| {
        *i.borrow_mut() = Some(Rc::new(cb));
    });
}

/// Clear the global keyboard interceptor.
pub fn clear_keyboard_interceptor() {
    KEYBOARD_INTERCEPTOR.with(|i| {
        *i.borrow_mut() = None;
    });
}

/// Dispatch a keyboard event to the interceptor.
/// Returns true if the event was handled.
pub fn dispatch_keyboard_event(data: &KeyEventData) -> bool {
    KEYBOARD_INTERCEPTOR.with(|i| {
        if let Some(ref interceptor) = *i.borrow() {
            interceptor(data)
        } else {
            false
        }
    })
}
