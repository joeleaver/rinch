//! Paste notification (issue #150).
//!
//! On the web, `ClipboardEvent.clipboardData` is the **only** synchronous channel
//! to content copied outside the app — `navigator.clipboard.readText()` is a
//! promise, and the keydown that "is" Ctrl+V fires *before* the browser has any
//! clipboard data to give. So an app that intercepts Ctrl+V through
//! [`set_keyboard_interceptor`](super::set_keyboard_interceptor) and then reads
//! the clipboard can only ever see its own copies.
//!
//! The fix is to hang paste logic off the paste itself rather than off the key
//! that triggered it: rinch-web's document-level `paste` listener fills the
//! `rinch-clipboard` buffers from `clipboardData` and *then* calls
//! [`dispatch_paste_event`], so an interceptor runs with fresh content already in
//! place and `paste_text()` answers correctly.
//!
//! It also means `prevent_default()` on the consumed Ctrl+V keydown stays safe:
//! the app's paste path no longer depends on that keydown reaching the browser.
//!
//! Desktop has no OS-level paste event — the runtime reads the clipboard directly
//! when Ctrl+V arrives — so nothing dispatches this there today. The hook lives in
//! `rinch-core` so app code that registers it compiles for every target.

use std::cell::RefCell;
use std::rc::Rc;

/// The content a paste carried, as the platform delivered it.
///
/// Both fields are `None` when the platform offered neither flavour (the buffers
/// are then unchanged, and a handler should fall back to reading the clipboard).
#[derive(Debug, Clone, Default)]
pub struct PasteEventData {
    /// The `text/plain` flavour.
    pub text: Option<String>,
    /// The `text/html` flavour, when the source offered rich content.
    pub html: Option<String>,
}

/// Type alias for the paste interceptor callback.
/// Returns true if the event was handled (the platform's default paste, e.g. the
/// browser inserting into a focused control, should be suppressed).
pub type PasteInterceptor = Rc<dyn Fn(&PasteEventData) -> bool>;

thread_local! {
    /// The one interceptor slot for the whole thread — the same single-slot,
    /// last-wins caveat as [`KEYBOARD_INTERCEPTOR`](super::set_keyboard_interceptor):
    /// two documents on one thread share it.
    static PASTE_INTERCEPTOR: RefCell<Option<PasteInterceptor>> = RefCell::new(None);
}

/// Set the global paste interceptor.
///
/// Runs **after** the platform's clipboard content has been made readable, so a
/// handler may call `rinch::clipboard::paste_text()` / `paste_html()` inside it
/// and get the just-pasted content. Only one interceptor can be active at a time,
/// per thread, not per document: a second call replaces the first.
///
/// **Released on unmount.** Registering from inside a render ties the
/// interceptor to the ambient scope, so disposing that scope clears it — a
/// callback that captured a `Signal` cannot outlive the signal and read freed
/// state (issue #183; the standing rule is the one
/// [`register_focus_target`](https://github.com/joeleaver/rinch/issues/147) follows).
/// The cleanup only clears the slot if this interceptor is *still* the one
/// installed, so a later `set_paste_interceptor` is never clobbered by an
/// earlier component unmounting. Registering outside any render — from `main`,
/// a timer, a detached callback — has no owner and so lives for the life of the
/// app, as before.
pub fn set_paste_interceptor<F>(cb: F)
where
    F: Fn(&PasteEventData) -> bool + 'static,
{
    let cb: PasteInterceptor = Rc::new(cb);
    let mine = Rc::downgrade(&cb);
    // The displaced interceptor is dropped *after* the borrow ends: its `Drop`
    // is user code and may re-enter this module (clearing, or registering a
    // replacement), which inside the `borrow_mut` would panic.
    let _previous = PASTE_INTERCEPTOR.with(|i| i.borrow_mut().replace(cb));
    crate::reactive::on_cleanup(move || {
        let Some(ours) = mine.upgrade() else {
            // Already replaced by a later registration, which owns the slot now.
            return;
        };
        let _displaced = PASTE_INTERCEPTOR.with(|i| {
            let mut slot = i.borrow_mut();
            if slot
                .as_ref()
                .is_some_and(|current| Rc::ptr_eq(current, &ours))
            {
                slot.take()
            } else {
                None
            }
        });
    });
}

/// Clear the global paste interceptor.
pub fn clear_paste_interceptor() {
    // Dropped outside the borrow — see `set_paste_interceptor`.
    let _previous = PASTE_INTERCEPTOR.with(|i| i.borrow_mut().take());
}

/// Whether a paste interceptor is registered.
///
/// Lets a backend skip the work of reading `clipboardData` when nothing would
/// consume it.
pub fn has_paste_interceptor() -> bool {
    PASTE_INTERCEPTOR.with(|i| i.borrow().is_some())
}

/// Dispatch a paste to the interceptor. Returns true if it was handled.
///
/// The `Rc` is cloned out before the call so the handler may re-enter (register a
/// different interceptor, for instance) without a double borrow.
pub fn dispatch_paste_event(data: &PasteEventData) -> bool {
    let interceptor = PASTE_INTERCEPTOR.with(|i| i.borrow().clone());
    match interceptor {
        Some(cb) => cb(data),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    use crate::reactive::Scope;

    /// #183: a registry that outlives the component that filled it hands a
    /// disposed scope's state to the next event. Registering inside a render
    /// ties the interceptor to that scope.
    #[test]
    fn an_interceptor_registered_in_a_scope_is_released_when_the_scope_disposes() {
        clear_paste_interceptor();
        let scope = Scope::new();
        scope.run(|| set_paste_interceptor(|_| true));
        assert!(
            has_paste_interceptor(),
            "the interceptor is live while its scope is"
        );

        scope.dispose();
        assert!(
            !has_paste_interceptor(),
            "disposing the owning scope must release the interceptor"
        );
        assert!(
            !dispatch_paste_event(&PasteEventData::default()),
            "a released interceptor must not run"
        );
    }

    /// An earlier component unmounting must not clear a *later* component's
    /// interceptor — the cleanup only takes the slot back if it still holds
    /// the one it installed.
    #[test]
    fn an_earlier_scopes_cleanup_does_not_clobber_a_later_interceptor() {
        clear_paste_interceptor();
        let first = Scope::new();
        first.run(|| set_paste_interceptor(|_| false));

        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        let second = Scope::new();
        second.run(move || {
            set_paste_interceptor(move |_| {
                flag.set(true);
                true
            })
        });

        first.dispose();
        assert!(
            has_paste_interceptor(),
            "the second interceptor must survive the first scope's disposal"
        );
        assert!(dispatch_paste_event(&PasteEventData::default()));
        assert!(ran.get(), "the surviving interceptor is the second one");

        second.dispose();
        assert!(!has_paste_interceptor());
    }

    /// Registering outside any render has no owner, so nothing releases it —
    /// the pre-existing app-lifetime behaviour.
    #[test]
    fn an_interceptor_registered_with_no_ambient_owner_lives_on() {
        clear_paste_interceptor();
        set_paste_interceptor(|_| true);
        Scope::new().dispose();
        assert!(has_paste_interceptor());
        clear_paste_interceptor();
    }

    #[test]
    fn an_interceptor_sees_both_flavours_and_can_consume_the_paste() {
        clear_paste_interceptor();
        let seen = Rc::new(RefCell::new(None::<PasteEventData>));
        let s = seen.clone();
        set_paste_interceptor(move |data| {
            *s.borrow_mut() = Some(data.clone());
            true
        });

        let data = PasteEventData {
            text: Some("plain".into()),
            html: Some("<b>rich</b>".into()),
        };
        assert!(dispatch_paste_event(&data));
        let got = seen.borrow().clone().expect("the interceptor ran");
        assert_eq!(got.text.as_deref(), Some("plain"));
        assert_eq!(got.html.as_deref(), Some("<b>rich</b>"));

        clear_paste_interceptor();
        assert!(
            !dispatch_paste_event(&data),
            "a cleared interceptor leaves the paste to the platform"
        );
    }

    #[test]
    fn an_unconsumed_paste_reports_false_and_the_slot_is_last_wins() {
        clear_paste_interceptor();
        assert!(!has_paste_interceptor());
        assert!(!dispatch_paste_event(&PasteEventData::default()));

        set_paste_interceptor(|_| false);
        assert!(has_paste_interceptor());
        assert!(!dispatch_paste_event(&PasteEventData::default()));

        // Last-wins, exactly like the keyboard interceptor.
        let ran = Rc::new(Cell::new(false));
        let r = ran.clone();
        set_paste_interceptor(move |_| {
            r.set(true);
            true
        });
        assert!(dispatch_paste_event(&PasteEventData::default()));
        assert!(ran.get());
        clear_paste_interceptor();
    }
}
