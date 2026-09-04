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

/// The per-document interceptor map (issue #478) — see
/// [`DocScopedSlotMap`](crate::reactive::DocScopedSlotMap).
type InterceptorSlots = crate::reactive::DocScopedSlotMap<dyn Fn(&PasteEventData) -> bool>;

thread_local! {
    /// One interceptor slot **per document**, plus the ownerless `None` entry —
    /// keyed exactly like [`KEYBOARD_INTERCEPTOR`](super::set_keyboard_interceptor)
    /// (issues #340, #478): two documents on one thread no longer clobber each
    /// other's registration, and dispatch prefers the dispatching document's
    /// interceptor over the thread-global fallback.
    static PASTE_INTERCEPTOR: RefCell<InterceptorSlots> =
        const { RefCell::new(InterceptorSlots::new()) };
}

/// Set the paste interceptor for the current document.
///
/// Runs **after** the platform's clipboard content has been made readable, so a
/// handler may call `rinch::clipboard::paste_text()` / `paste_html()` inside it
/// and get the just-pasted content. Only one interceptor can be active at a time
/// **per document**: a second call from the same document replaces the first
/// (issue #478). Registering outside any dispatch — from `main`, a timer, or at
/// mount, and everywhere on rinch-web, which never marks dispatch — fills the
/// thread-global fallback slot, which serves every document that has no
/// interceptor of its own.
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
/// app, as before. That discipline lives in
/// [`install_scoped_slot`](crate::reactive::install_scoped_slot), shared with
/// the keyboard and selection registries.
pub fn set_paste_interceptor<F>(cb: F)
where
    F: Fn(&PasteEventData) -> bool + 'static,
{
    crate::reactive::install_doc_scoped_slot(&PASTE_INTERCEPTOR, Rc::new(cb));
}

/// Clear the paste interceptor a dispatch would reach right now: the current
/// document's own if it has one, else the thread-global fallback.
pub fn clear_paste_interceptor() {
    crate::reactive::clear_doc_scoped_slot(&PASTE_INTERCEPTOR);
}

/// Whether a paste dispatched by the current document would reach an
/// interceptor.
///
/// Lets a backend skip the work of reading `clipboardData` when nothing would
/// consume it.
pub fn has_paste_interceptor() -> bool {
    crate::reactive::read_doc_scoped_slot(&PASTE_INTERCEPTOR).is_some()
}

/// Dispatch a paste to the dispatching document's interceptor (or the
/// thread-global fallback). Returns true if it was handled.
///
/// The `Rc` is cloned out before the call so the handler may re-enter (register a
/// different interceptor, for instance) without a double borrow.
pub fn dispatch_paste_event(data: &PasteEventData) -> bool {
    match crate::reactive::read_doc_scoped_slot(&PASTE_INTERCEPTOR) {
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

    // ── per-document routing (issue #478) ────────────────────────────────────

    /// Two documents on one thread each keep their own paste interceptor, and
    /// `has_paste_interceptor` answers for the *dispatching* document — a
    /// backend must not read `clipboardData` for a document whose paste
    /// nothing would consume.
    ///
    /// Under the old single slot, doc 2's registration displaced doc 1's (so
    /// doc 1's paste ran doc 2's interceptor) and `has_paste_interceptor`
    /// answered `true` for every document on the thread.
    #[test]
    fn two_documents_paste_interceptors_coexist_and_route_by_dispatching_document() {
        use crate::context::push_dispatching_doc;

        clear_paste_interceptor();
        let hits: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));

        {
            let _a = push_dispatching_doc(1);
            let h = hits.clone();
            set_paste_interceptor(move |_| {
                h.borrow_mut().push("doc1");
                true
            });
        }
        {
            let _b = push_dispatching_doc(2);
            let h = hits.clone();
            set_paste_interceptor(move |_| {
                h.borrow_mut().push("doc2");
                true
            });
        }

        {
            let _a = push_dispatching_doc(1);
            assert!(has_paste_interceptor());
            assert!(
                dispatch_paste_event(&PasteEventData::default()),
                "doc 1 still has an interceptor — doc 2's registration must not displace it"
            );
        }
        assert_eq!(
            *hits.borrow(),
            vec!["doc1"],
            "doc 1's paste reaches doc 1's interceptor, not doc 2's"
        );

        {
            let _b = push_dispatching_doc(2);
            assert!(dispatch_paste_event(&PasteEventData::default()));
        }
        assert_eq!(*hits.borrow(), vec!["doc1", "doc2"]);

        {
            let _c = push_dispatching_doc(3);
            assert!(
                !has_paste_interceptor(),
                "a third document with no interceptor of its own and no global \
                 fallback has nothing a paste would reach"
            );
            assert!(!dispatch_paste_event(&PasteEventData::default()));
        }
        assert_eq!(*hits.borrow(), vec!["doc1", "doc2"]);

        {
            let _a = push_dispatching_doc(1);
            clear_paste_interceptor();
        }
        {
            let _b = push_dispatching_doc(2);
            clear_paste_interceptor();
        }
        assert!(!has_paste_interceptor());
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
