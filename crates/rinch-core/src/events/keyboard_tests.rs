//! Tests for the keyboard interceptor and the popup-key dispatch (`keyboard.rs`).

use super::*;
use std::cell::{Cell, RefCell};

use crate::reactive::{Scope, Signal};

fn key(name: &str) -> KeyEventData {
    KeyEventData::new(name.to_string(), name.to_string())
}

/// `is_space` answers the spacebar on either field, and nothing else — not
/// a key merely named like it, not a key whose text is some other blank.
#[test]
fn is_space_answers_the_spacebar_by_key_or_code() {
    assert!(KeyEventData::new(" ", "Space").is_space(), "every backend");
    assert!(KeyEventData::new(" ", "").is_space(), "key alone");
    assert!(KeyEventData::new("", "Space").is_space(), "code alone");
    for (k, c) in [
        ("Enter", "Enter"),
        ("a", "KeyA"),
        ("\u{a0}", "KeyA"),
        ("\t", "Tab"),
        ("Space", "KeyS"),
    ] {
        assert!(!KeyEventData::new(k, c).is_space(), "{k:?}/{c:?}");
    }
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

/// **The interceptor wins over the dismiss stack** (#474).
///
/// `dispatch_keyboard_event`'s own doc says so, and nothing tested it:
/// removing the `if intercepted { return true; }` early return left both
/// suites green, because every other fixture uses an Escape no interceptor
/// is registered for. An app that consumes Escape in its document-level
/// hook means to own it, and an open modal must not take it anyway.
///
/// Both directions, because "the stack never runs" passes the first half
/// on its own.
#[test]
fn an_interceptor_that_consumes_escape_keeps_it_from_the_dismiss_stack() {
    clear_keyboard_interceptor();
    let reached = Rc::new(Cell::new(0u32));
    let r = reached.clone();
    let _entry = crate::events::push_dismiss_handler(0, move || {
        r.set(r.get() + 1);
        true
    });

    // Declining: the stack is consulted and consumes.
    set_keyboard_interceptor(|_| false);
    assert!(dispatch_keyboard_event(&key("Escape")));
    assert_eq!(reached.get(), 1, "a declining interceptor falls through");

    // Consuming: the stack is never reached.
    set_keyboard_interceptor(|_| true);
    assert!(dispatch_keyboard_event(&key("Escape")));
    assert_eq!(
        reached.get(),
        1,
        "an interceptor that consumes Escape must keep it from the stack"
    );

    clear_keyboard_interceptor();
}

/// And the stack only ever sees **Escape**: any other key an interceptor
/// declines falls through to the runtime untouched, as it always has.
#[test]
fn a_key_that_is_not_escape_never_reaches_the_dismiss_stack() {
    clear_keyboard_interceptor();
    let reached = Rc::new(Cell::new(0u32));
    let r = reached.clone();
    let _entry = crate::events::push_dismiss_handler(0, move || {
        r.set(r.get() + 1);
        true
    });
    assert!(!dispatch_keyboard_event(&key("a")));
    assert!(!dispatch_keyboard_event(&key("Enter")));
    assert_eq!(reached.get(), 0);
    // Positive control: the instrument fires for the key it is meant to.
    assert!(dispatch_keyboard_event(&key("Escape")));
    assert_eq!(reached.get(), 1);
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

/// The two halves a backend calls separately (#3062): the interceptor alone
/// never reaches the dismiss stack, and the popup keys never reach the
/// interceptor. Each half is asked with the other's party present and
/// willing, so a half that also ran the other would show.
#[test]
fn the_interceptor_and_the_popup_keys_are_asked_apart() {
    clear_keyboard_interceptor();
    let dismissed = Rc::new(Cell::new(0u32));
    let intercepted = Rc::new(Cell::new(0u32));
    let (d, i) = (dismissed.clone(), intercepted.clone());
    let _entry = crate::events::push_dismiss_handler(0, move || {
        d.set(d.get() + 1);
        true
    });
    set_keyboard_interceptor(move |_| {
        i.set(i.get() + 1);
        false
    });

    assert!(!dispatch_key_interceptor(&key("Escape")));
    assert_eq!((intercepted.get(), dismissed.get()), (1, 0));

    assert!(dispatch_popup_key(&key("Escape")));
    assert_eq!((intercepted.get(), dismissed.get()), (1, 1));

    // A release is no popup's: Escape's release dismisses nothing.
    assert!(!dispatch_popup_key(
        &key("Escape").with_kind(KeyEventKind::Up)
    ));
    assert_eq!(dismissed.get(), 1);

    // The whole of `dispatch_keyboard_event` is still both, in order.
    assert!(dispatch_keyboard_event(&key("Escape")));
    assert_eq!((intercepted.get(), dismissed.get()), (2, 2));
    clear_keyboard_interceptor();
}
