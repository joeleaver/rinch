//! Keyboard event interception.

use std::cell::RefCell;
use std::rc::Rc;

/// Which phase of a keystroke an event reports (issue #337).
///
/// A releases-aware consumer pairs a `Down` with its `Up` by comparing
/// [`KeyEventData::key`]. That pairing only works because press and release
/// are spelled by the same function from the same fields — see `hook_key_str`
/// in the desktop runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum KeyEventKind {
    /// The key went down.
    ///
    /// **OS auto-repeat also arrives as `Down`**, and there is currently no
    /// flag distinguishing it from a fresh press: the browser supplies one
    /// (`KeyboardEvent.repeat`) but `PlatformEvent::KeyDown` does not carry
    /// winit's, so a `repeat` field here would be truthful on web and silently
    /// `false` on desktop — the exact divergence a document-level hook exists
    /// to avoid. It arrives with the plumbing, in the issue that retires the
    /// runtime's own hand-rolled activation latch.
    #[default]
    Down,
    /// The key came up.
    Up,
}

/// Keyboard event data for the global keyboard interceptor.
///
/// `#[non_exhaustive]`: build one with [`KeyEventData::new`] and the `with_*`
/// setters rather than a struct literal, so a field added in a minor release
/// is not a breaking change. Reading fields is unaffected — and reading is
/// what an interceptor does, since these are delivered, not constructed, by
/// app code.
#[non_exhaustive]
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
    /// Press or release (issue #337). Before it, only presses were ever
    /// delivered, so a consumer had no way to see a key let go — and no way to
    /// ask, either, since every event was implicitly a press.
    pub kind: KeyEventKind,
}

impl KeyEventData {
    /// A press of `key` (spelled as `KeyboardEvent.key`) at physical `code`,
    /// with no modifiers held.
    ///
    /// Chain [`Self::with_modifiers`] and [`Self::with_kind`] for the rest.
    /// The constructor takes only the two fields that have no sensible
    /// default: every event has a key and a code, while "no modifiers, a
    /// press" is the common case and reads better as an absence than as four
    /// `false`s at every call site.
    pub fn new(key: impl Into<String>, code: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            code: code.into(),
            ctrl: false,
            shift: false,
            alt: false,
            meta: false,
            kind: KeyEventKind::Down,
        }
    }

    /// Set all four modifiers at once, in the order they are declared:
    /// **ctrl, shift, alt, meta**.
    ///
    /// One call rather than four setters because they are always known
    /// together — they come off a single `Modifiers` — and splitting them
    /// invites a site that sets three and forgets the fourth, which is exactly
    /// how `meta` came to be hardcoded `false` on two of three paths (#336).
    pub fn with_modifiers(mut self, ctrl: bool, shift: bool, alt: bool, meta: bool) -> Self {
        self.ctrl = ctrl;
        self.shift = shift;
        self.alt = alt;
        self.meta = meta;
        self
    }

    /// Mark this as a press or a release.
    pub fn with_kind(mut self, kind: KeyEventKind) -> Self {
        self.kind = kind;
        self
    }

    /// Whether this is a press — auto-repeat included.
    pub fn is_down(&self) -> bool {
        self.kind == KeyEventKind::Down
    }

    /// Whether this is a release.
    pub fn is_up(&self) -> bool {
        self.kind == KeyEventKind::Up
    }
}

/// Type alias for the keyboard interceptor callback.
/// Returns true if the event was handled (should not propagate to the runtime).
pub type KeyboardInterceptor = Rc<dyn Fn(&KeyEventData) -> bool>;

/// The per-document interceptor map (issue #340) — see
/// [`DocScopedSlotMap`](crate::reactive::DocScopedSlotMap).
type InterceptorSlots = crate::reactive::DocScopedSlotMap<dyn Fn(&KeyEventData) -> bool>;

thread_local! {
    /// One interceptor slot **per document**, plus the ownerless `None` entry
    /// for registrations made outside any dispatch (issue #340).
    ///
    /// This used to be a single slot for the whole thread, last-wins — the
    /// #134/#136 hazard: a thread can pump several documents' event streams (a
    /// desktop app and its DevTools window, two embedded `RinchContext`s), so
    /// the second document's [`set_keyboard_interceptor`] silently disabled
    /// the first's and whichever remained drove keyboard capture for both.
    /// Now a registration is keyed by
    /// [`current_dispatching_doc`](crate::context::current_dispatching_doc)
    /// and dispatch prefers the dispatching document's interceptor, falling
    /// back to the ownerless entry — so a hook installed from `main` or at
    /// mount still intercepts every document's keys, as it always has. The
    /// *lifetime* of each entry is issue #183 and is handled below.
    static KEYBOARD_INTERCEPTOR: RefCell<InterceptorSlots> =
        const { RefCell::new(InterceptorSlots::new()) };
}

/// Set the keyboard interceptor for the current document.
///
/// Only one interceptor can be active at a time **per document**: a second
/// call from the same document replaces the first, while another document's
/// registration is a separate slot (issue #340). "The current document" is the
/// one whose events are being dispatched right now; registering outside any
/// dispatch — from `main`, a timer, or at mount — fills the thread-global
/// fallback slot, which intercepts for every document that has no interceptor
/// of its own.
///
/// **Released on unmount.** Registering from inside a render ties the
/// interceptor to the ambient scope, so disposing that scope clears it — a
/// callback that captured a `Signal` cannot outlive the signal and read freed
/// state (issue #183). The cleanup only clears the slot if this interceptor is
/// *still* the one installed, so a later `set_keyboard_interceptor` is never
/// clobbered by an earlier component unmounting. Registering outside any
/// render — from `main`, a timer, a detached callback — has no owner and so
/// lives for the life of the app, as before.
pub fn set_keyboard_interceptor<F>(cb: F)
where
    F: Fn(&KeyEventData) -> bool + 'static,
{
    crate::reactive::install_doc_scoped_slot(&KEYBOARD_INTERCEPTOR, Rc::new(cb));
}

/// Clear the keyboard interceptor a dispatch would reach right now: the
/// current document's own if it has one, else the thread-global fallback.
pub fn clear_keyboard_interceptor() {
    crate::reactive::clear_doc_scoped_slot(&KEYBOARD_INTERCEPTOR);
}

/// Dispatch a keyboard event to the dispatching document's interceptor (or the
/// thread-global fallback). Returns true if the event was handled.
///
/// The `Rc` is cloned out before the call so the handler may re-enter (install a
/// different interceptor, for instance) without a double borrow.
pub fn dispatch_keyboard_event(data: &KeyEventData) -> bool {
    match crate::reactive::read_doc_scoped_slot(&KEYBOARD_INTERCEPTOR) {
        Some(cb) => cb(data),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    use crate::reactive::{Scope, Signal};

    fn key(name: &str) -> KeyEventData {
        KeyEventData::new(name.to_string(), name.to_string())
    }

    /// `with_modifiers` takes four positional bools — the shape whose failure
    /// mode is a silent transposition, which no symmetric-modifier test can
    /// see. Two asymmetric patterns pin every position to its field.
    #[test]
    fn with_modifiers_assigns_each_position_to_its_field() {
        let a = KeyEventData::new("a", "KeyA").with_modifiers(true, false, true, false);
        assert!(a.ctrl && !a.shift && a.alt && !a.meta, "{a:?}");
        let b = KeyEventData::new("a", "KeyA").with_modifiers(false, true, false, true);
        assert!(!b.ctrl && b.shift && !b.alt && b.meta, "{b:?}");
    }

    /// #183: a registry that outlives the component that filled it hands a
    /// disposed scope's state to the next event. Registering inside a render
    /// ties the interceptor to that scope.
    #[test]
    fn an_interceptor_registered_in_a_scope_is_released_when_the_scope_disposes() {
        clear_keyboard_interceptor();
        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        let scope = Scope::new();
        scope.run(move || {
            set_keyboard_interceptor(move |_| {
                flag.set(true);
                true
            })
        });
        assert!(
            dispatch_keyboard_event(&key("a")),
            "the interceptor is live while its scope is"
        );
        assert!(ran.get());
        ran.set(false);

        scope.dispose();
        assert!(
            !dispatch_keyboard_event(&key("a")),
            "disposing the owning scope must release the interceptor"
        );
        assert!(!ran.get(), "a released interceptor must not run");
    }

    /// An earlier component unmounting must not clear a *later* component's
    /// interceptor — the cleanup only takes the slot back if it still holds
    /// the one it installed.
    #[test]
    fn an_earlier_scopes_cleanup_does_not_clobber_a_later_interceptor() {
        clear_keyboard_interceptor();
        let first = Scope::new();
        first.run(|| set_keyboard_interceptor(|_| false));

        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        let second = Scope::new();
        second.run(move || {
            set_keyboard_interceptor(move |_| {
                flag.set(true);
                true
            })
        });

        first.dispose();
        assert!(
            dispatch_keyboard_event(&key("a")),
            "the second interceptor must survive the first scope's disposal"
        );
        assert!(ran.get(), "the surviving interceptor is the second one");

        second.dispose();
        assert!(!dispatch_keyboard_event(&key("a")));
    }

    /// Registering outside any render has no owner, so nothing releases it —
    /// the pre-existing app-lifetime behaviour.
    #[test]
    fn an_interceptor_registered_with_no_ambient_owner_lives_on() {
        clear_keyboard_interceptor();
        set_keyboard_interceptor(|_| true);
        Scope::new().dispose();
        assert!(
            dispatch_keyboard_event(&key("a")),
            "an ownerless interceptor keeps app lifetime"
        );
        clear_keyboard_interceptor();
    }

    /// The actual #183 shape: the interceptor captured a signal its component
    /// owns. Once the component is gone the signal's storage is freed, and a
    /// read of a freed signal panics — so the interceptor must never run again.
    #[test]
    fn a_released_interceptor_does_not_read_its_components_freed_signal() {
        clear_keyboard_interceptor();
        let scope = Scope::new();
        let count = scope.run(|| {
            let count = Signal::new(7);
            set_keyboard_interceptor(move |_| count.get() > 0);
            count
        });

        assert!(dispatch_keyboard_event(&key("a")));

        scope.dispose();
        assert!(!count.is_alive(), "disposal freed the component's signal");
        // Before the fix this dispatches into the stale interceptor, which
        // reads the freed signal and panics.
        assert!(
            !dispatch_keyboard_event(&key("a")),
            "a released interceptor must not read its component's freed state"
        );
    }

    // ── per-document routing (issue #340) ────────────────────────────────────

    /// Two documents on one thread each keep their own interceptor: B's
    /// registration must not destroy A's, and each document's keys reach only
    /// its own interceptor.
    ///
    /// The interceptors are distinguishable (each records its own tag) and the
    /// assertions name *which* one ran — under the old single slot, doc 2's
    /// registration displaced doc 1's, so doc 1's dispatch ran doc 2's
    /// interceptor and this test fails on the first `hits` assertion.
    #[test]
    fn two_documents_interceptors_coexist_and_each_receives_only_its_own_keys() {
        use crate::context::push_dispatching_doc;

        clear_keyboard_interceptor();
        let hits: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));

        {
            let _a = push_dispatching_doc(1);
            let h = hits.clone();
            set_keyboard_interceptor(move |_| {
                h.borrow_mut().push("doc1");
                true
            });
        }
        {
            let _b = push_dispatching_doc(2);
            let h = hits.clone();
            set_keyboard_interceptor(move |_| {
                h.borrow_mut().push("doc2");
                true
            });
        }

        {
            let _a = push_dispatching_doc(1);
            assert!(
                dispatch_keyboard_event(&key("x")),
                "doc 1 still has an interceptor — doc 2's registration must not displace it"
            );
        }
        assert_eq!(
            *hits.borrow(),
            vec!["doc1"],
            "doc 1's keys reach doc 1's interceptor, not doc 2's"
        );

        {
            let _b = push_dispatching_doc(2);
            assert!(dispatch_keyboard_event(&key("x")));
        }
        assert_eq!(
            *hits.borrow(),
            vec!["doc1", "doc2"],
            "doc 2's keys reach doc 2's interceptor"
        );

        {
            let _a = push_dispatching_doc(1);
            clear_keyboard_interceptor();
        }
        {
            let _b = push_dispatching_doc(2);
            clear_keyboard_interceptor();
        }
        assert!(!dispatch_keyboard_event(&key("x")));
    }

    /// An interceptor registered outside any dispatch is the thread-global
    /// fallback: a document with its own interceptor shadows it, every other
    /// document — and a dispatch outside any document — still reaches it.
    #[test]
    fn a_documents_interceptor_shadows_the_global_one_only_for_that_document() {
        use crate::context::push_dispatching_doc;

        clear_keyboard_interceptor();
        let hits: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));

        let h = hits.clone();
        set_keyboard_interceptor(move |_| {
            h.borrow_mut().push("global");
            true
        });

        {
            let _a = push_dispatching_doc(1);
            let h = hits.clone();
            set_keyboard_interceptor(move |_| {
                h.borrow_mut().push("doc1");
                true
            });
            assert!(dispatch_keyboard_event(&key("x")));
        }
        assert_eq!(
            hits.borrow().last(),
            Some(&"doc1"),
            "doc 1's own interceptor shadows the global one for doc 1's keys"
        );

        {
            let _b = push_dispatching_doc(2);
            assert!(
                dispatch_keyboard_event(&key("x")),
                "doc 2 falls back to the global interceptor"
            );
        }
        assert_eq!(
            hits.borrow().last(),
            Some(&"global"),
            "doc 1's registration must not have displaced the global slot doc 2 relies on"
        );

        assert!(
            dispatch_keyboard_event(&key("x")),
            "outside any dispatch, the global interceptor answers"
        );
        assert_eq!(hits.borrow().last(), Some(&"global"));

        {
            let _a = push_dispatching_doc(1);
            clear_keyboard_interceptor();
        }
        clear_keyboard_interceptor();
        assert!(!dispatch_keyboard_event(&key("x")));
    }

    /// Unmounting the component that registered a document's interceptor
    /// releases only that document's slot — the thread-global fallback the
    /// registration shadowed is in effect again, not destroyed.
    ///
    /// The scope is disposed with *no* marker current: the cleanup must
    /// remember the key it installed under, not re-read the ambient one.
    #[test]
    fn a_disposed_scopes_interceptor_releases_only_its_documents_slot() {
        use crate::context::push_dispatching_doc;

        clear_keyboard_interceptor();
        let hits: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));

        let h = hits.clone();
        set_keyboard_interceptor(move |_| {
            h.borrow_mut().push("global");
            true
        });

        let scope = Scope::new();
        {
            let _a = push_dispatching_doc(1);
            let h = hits.clone();
            scope.run(move || {
                set_keyboard_interceptor(move |_| {
                    h.borrow_mut().push("doc1");
                    true
                })
            });
            assert!(dispatch_keyboard_event(&key("x")));
        }
        assert_eq!(hits.borrow().last(), Some(&"doc1"));

        scope.dispose();
        {
            let _a = push_dispatching_doc(1);
            assert!(
                dispatch_keyboard_event(&key("x")),
                "doc 1 falls back to the global interceptor once its own is released"
            );
        }
        assert_eq!(hits.borrow().last(), Some(&"global"));

        clear_keyboard_interceptor();
        assert!(!dispatch_keyboard_event(&key("x")));
    }

    /// `clear_keyboard_interceptor` removes the interceptor a dispatch would
    /// reach right now: from inside a document's dispatch with no per-document
    /// interceptor installed, that is the thread-global one — so an app that
    /// registered at mount and clears from an event handler still clears it.
    #[test]
    fn clear_from_a_documents_dispatch_reaches_the_global_interceptor_it_falls_back_to() {
        use crate::context::push_dispatching_doc;

        clear_keyboard_interceptor();
        set_keyboard_interceptor(|_| true);
        {
            let _a = push_dispatching_doc(1);
            clear_keyboard_interceptor();
            assert!(
                !dispatch_keyboard_event(&key("x")),
                "cleared from inside doc 1's dispatch"
            );
        }
        assert!(
            !dispatch_keyboard_event(&key("x")),
            "the global slot the document fell back to is the one that was cleared"
        );
    }

    /// The other half of the clear-resolution rule: when the dispatching
    /// document HAS an interceptor of its own, `clear_keyboard_interceptor`
    /// removes exactly that one entry — the thread-global fallback it was
    /// shadowing survives for every other document.
    ///
    /// Kills: a clear that takes the fallback entry besides the document's
    /// own — which would silently destroy an app-wide interceptor the moment
    /// any component cleared its own (found open by review mutation on
    /// #498; the naive-exact-key sibling is pinned above).
    #[test]
    fn clearing_a_documents_own_interceptor_leaves_the_global_fallback_in_place() {
        use crate::context::push_dispatching_doc;

        clear_keyboard_interceptor();
        let hits: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));

        let h = hits.clone();
        set_keyboard_interceptor(move |_| {
            h.borrow_mut().push("global");
            true
        });
        {
            let _a = push_dispatching_doc(1);
            let h = hits.clone();
            set_keyboard_interceptor(move |_| {
                h.borrow_mut().push("doc1");
                true
            });
            // Removes doc 1's own entry — and nothing else.
            clear_keyboard_interceptor();
        }

        {
            let _b = push_dispatching_doc(2);
            assert!(
                dispatch_keyboard_event(&key("x")),
                "doc 2 still falls back to the global interceptor — clearing \
                 doc 1's own entry must not take the fallback with it"
            );
        }
        assert_eq!(hits.borrow().last(), Some(&"global"));

        clear_keyboard_interceptor();
        assert!(!dispatch_keyboard_event(&key("x")));
    }

    /// The dispatch must not hold the slot's borrow across user code: an
    /// interceptor is allowed to install its replacement from inside its own
    /// dispatch.
    #[test]
    fn an_interceptor_may_replace_itself_from_inside_dispatch() {
        clear_keyboard_interceptor();
        set_keyboard_interceptor(|_| {
            set_keyboard_interceptor(|_| false);
            true
        });

        assert!(dispatch_keyboard_event(&key("a")), "the first ran");
        assert!(
            !dispatch_keyboard_event(&key("a")),
            "and installed its replacement"
        );
        clear_keyboard_interceptor();
    }
}
