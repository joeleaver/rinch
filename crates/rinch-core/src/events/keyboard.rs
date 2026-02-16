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
    static KEYBOARD_INTERCEPTOR: RefCell<Option<KeyboardInterceptor>> = RefCell::new(None);
}

/// Set the global keyboard interceptor.
/// Only one interceptor can be active at a time.
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
