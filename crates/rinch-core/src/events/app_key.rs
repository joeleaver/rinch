//! App-wide key listeners (issue #1784).
//!
//! [`on_app_key`] registers a listener that hears every key press and release
//! the window (desktop) or page (rinch-web) receives — whatever has focus,
//! **including when nothing has** — that nothing ahead of it consumed. It is
//! what a browser app gets from `window.addEventListener("keydown", …)`, and
//! any number of parties can register one: an app's shortcuts beside an
//! embedded code editor's.
//!
//! # Where it sits
//!
//! Both backends offer a key, in order, to: menu chords → the focused editor's
//! / node view's `on_key` → the focused element's `onkeydown` and its
//! ancestors' → [`set_keyboard_interceptor`](super::set_keyboard_interceptor)
//! → open popups' [key entries](super::push_key_handler) → the
//! [dismiss stack](super::push_dismiss_handler) → a focused render surface's
//! own claim (`RenderSurfaceHandle::set_key_handler`) → **these listeners** →
//! the focused element's default (text editing, Tab, Enter/Space activation,
//! `onsubmit`). So everything that owns a key while it is open or focused — an
//! open `Select` list, a `Modal` closing on Escape, a code editor's
//! interceptor, a game canvas — wins, and a listener still runs before the
//! default action, as a `window` listener runs before a browser's.
//!
//! A key typed into a text field therefore reaches the listeners, as it reaches
//! a browser's `window` listener; a shortcut that should leave text fields
//! alone asks [`NodeHandle::active_element`](crate::dom::NodeHandle::active_element).
//! Not offered at all: a press while the rich-text editor holds the keyboard
//! (its keys are its own on both backends), while an input method composes,
//! and on desktop while the built-in text context menu or an open native
//! `<select>` list owns the keyboard.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use super::KeyEventData;
use crate::context::{current_dispatching_doc, doc_matches};
use crate::reactive::{Owner, current_owner};

type ListenerFn = Rc<dyn Fn(&KeyEventData) -> bool>;

struct Entry {
    id: u64,
    /// The document registering it, or `None` outside any (see
    /// [`doc_matches`]).
    doc: Option<u64>,
    owner: Option<Owner>,
    f: ListenerFn,
}

thread_local! {
    /// Registration order, oldest first.
    static LISTENERS: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
    /// Physical codes whose press a focused render surface claimed: their
    /// release is the surface's too ([`withhold_app_key_release`]).
    static WITHHELD: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

/// A registered app-wide key listener (see [`on_app_key`]).
///
/// **Dropping it does not unregister the listener.** The listener belongs to
/// the scope that was rendering when it was registered and is released when
/// that scope is disposed (a component unmounting); one registered outside any
/// render lives for the life of the app. Call [`remove`](Self::remove) to
/// release it earlier.
pub struct AppKeyListener(u64);

impl std::fmt::Debug for AppKeyListener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("AppKeyListener").field(&self.0).finish()
    }
}

impl AppKeyListener {
    /// Unregister the listener now. Harmless if it is already gone.
    pub fn remove(self) {
        remove(self.0);
    }
}

/// Register an app-wide key listener: `f` hears every key press and release
/// the window receives that nothing ahead of it consumed, whatever has focus —
/// including nothing (issue #1784). See the [module docs](self) for where it
/// sits among the other key handlers.
///
/// - **Order.** Listeners are offered a key in registration order. For a
///   **press**, the first to answer `true` consumes it: no later listener and
///   no default action (text insertion, Tab, activation, `onsubmit`) sees it,
///   and on rinch-web the event is `preventDefault`ed and its propagation
///   stopped. A **release** goes to every listener and the answer is ignored —
///   a release has no default to prevent, and a held-key tracker must see each
///   one. The one release withheld is that of a key a focused render surface
///   claimed: a surface's claimed keys are its own, press and release.
/// - **Document.** Registered while a document's code runs (its render, an
///   effect it owns, its event handler), a listener hears that document's keys;
///   registered outside any — from `main` — it hears every document's.
/// - **Lifetime.** Released when the scope rendering at registration is
///   disposed (and never called after that), else app lifetime;
///   [`AppKeyListener::remove`] releases it earlier. Dropping the handle
///   releases nothing, so `on_app_key(…);` in a component just works.
/// - The listener runs untracked, inside its owner scope, in the key's
///   transaction (one `batch` with the rest of the key's dispatch).
pub fn on_app_key(f: impl Fn(&KeyEventData) -> bool + 'static) -> AppKeyListener {
    let id = NEXT_ID.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    });
    let entry = Entry {
        id,
        doc: current_dispatching_doc(),
        owner: current_owner(),
        f: Rc::new(f),
    };
    LISTENERS.with(|l| l.borrow_mut().push(entry));
    // Released with the ambient scope, if there is one.
    crate::reactive::on_cleanup(move || remove(id));
    AppKeyListener(id)
}

/// Offer a key to the app-wide listeners ([`on_app_key`]). Returns whether a
/// listener consumed a **press**; a release always answers `false`.
///
/// Backends call this at the place the [module docs](self) name, after
/// [`dispatch_keyboard_event`](super::dispatch_keyboard_event) answered
/// `false` and a focused render surface did not claim the key. Listeners run
/// from a snapshot with no borrow held, so one may register or remove
/// listeners (its own included).
pub fn dispatch_app_key(data: &KeyEventData) -> bool {
    if data.is_up() {
        let withheld = WITHHELD.with(|w| w.borrow_mut().remove(&data.code));
        if withheld {
            return false;
        }
    } else {
        // A press that reached us: whatever an earlier release left marked for
        // this key is stale (that release never arrived — a window blur).
        WITHHELD.with(|w| w.borrow_mut().remove(&data.code));
    }
    crate::reactive::batch(|| dispatch_unbatched(data))
}

fn dispatch_unbatched(data: &KeyEventData) -> bool {
    let caller = current_dispatching_doc();
    let candidates: Vec<(u64, Option<u64>, Option<Owner>, ListenerFn)> = LISTENERS.with(|l| {
        l.borrow()
            .iter()
            .map(|e| (e.id, e.doc, e.owner.clone(), e.f.clone()))
            .collect()
    });
    let mut dead = Vec::new();
    let mut consumed = false;
    for (id, doc, owner, f) in candidates {
        // A disposed component's listener reads freed state — never run it.
        if owner.as_ref().is_some_and(|o| !o.is_alive()) {
            dead.push(id);
            continue;
        }
        if !doc_matches(doc, caller) {
            continue;
        }
        // Removed by a listener that ran earlier in this dispatch.
        if !LISTENERS.with(|l| l.borrow().iter().any(|e| e.id == id)) {
            continue;
        }
        let took = crate::reactive::untracked_handler(|| match &owner {
            Some(o) => o.run(|| f(data)),
            None => f(data),
        });
        if took && data.is_down() {
            consumed = true;
            break;
        }
    }
    for id in dead {
        remove(id);
    }
    consumed
}

/// Tell the listeners that a focused render surface claimed this **press**, so
/// its release is the surface's too and is not offered to them. Cleared by that
/// release, or by the next press of the same physical key that reaches
/// [`dispatch_app_key`].
pub fn withhold_app_key_release(code: &str) {
    WITHHELD.with(|w| {
        w.borrow_mut().insert(code.to_string());
    });
}

/// Remove the entry with `id`; it is dropped outside the borrow, since a
/// listener closes over user state whose `Drop` may re-enter this list.
fn remove(id: u64) {
    let reaped = LISTENERS.with(|l| {
        let Ok(mut list) = l.try_borrow_mut() else {
            return None;
        };
        list.iter().position(|e| e.id == id).map(|i| list.remove(i))
    });
    drop(reaped);
}

/// How many app-wide key listeners are registered on this thread. Diagnostics
/// and tests only.
#[doc(hidden)]
pub fn app_key_listener_count() -> usize {
    LISTENERS.with(|l| l.borrow().len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reactive::Scope;
    use std::cell::RefCell;

    fn down(code: &str) -> KeyEventData {
        KeyEventData::new(code, code)
    }
    fn up(code: &str) -> KeyEventData {
        down(code).with_kind(super::super::KeyEventKind::Up)
    }

    fn clear() {
        LISTENERS.with(|l| l.borrow_mut().clear());
        WITHHELD.with(|w| w.borrow_mut().clear());
    }

    /// Two parties both hear a key, in registration order, and a consuming
    /// press stops the later one; a release reaches both whatever they answer.
    #[test]
    fn listeners_run_in_order_and_a_consumed_press_stops_later_ones() {
        clear();
        let log = Rc::new(RefCell::new(Vec::new()));
        let l1 = log.clone();
        let _a = on_app_key(move |k| {
            l1.borrow_mut().push(format!("a:{}", k.key));
            k.key == "Delete"
        });
        let l2 = log.clone();
        let _b = on_app_key(move |k| {
            l2.borrow_mut().push(format!("b:{}", k.key));
            false
        });
        assert!(!dispatch_app_key(&down("KeyZ")));
        assert!(dispatch_app_key(&down("Delete")));
        assert!(
            !dispatch_app_key(&up("Delete")),
            "a release is never consumed"
        );
        assert_eq!(
            *log.borrow(),
            ["a:KeyZ", "b:KeyZ", "a:Delete", "a:Delete", "b:Delete"]
        );
        clear();
    }

    /// Disposing the scope that registered a listener releases it; a
    /// listener registered outside any scope stays; `remove` releases early.
    #[test]
    fn a_listener_goes_with_its_scope_and_remove_releases_it() {
        clear();
        let hits = Rc::new(Cell::new(0));
        let scope = Scope::new();
        let h = hits.clone();
        scope.run(|| {
            on_app_key(move |_| {
                h.set(h.get() + 1);
                false
            });
        });
        let h = hits.clone();
        let app = on_app_key(move |_| {
            h.set(h.get() + 10);
            false
        });
        dispatch_app_key(&down("KeyA"));
        assert_eq!(hits.get(), 11);
        scope.dispose();
        assert_eq!(app_key_listener_count(), 1, "the scope's entry is gone");
        dispatch_app_key(&down("KeyA"));
        assert_eq!(hits.get(), 21);
        app.remove();
        dispatch_app_key(&down("KeyA"));
        assert_eq!(hits.get(), 21);
        assert_eq!(app_key_listener_count(), 0);
        clear();
    }

    /// A press a surface claimed keeps its release from the listeners; the
    /// mark does not outlive a later delivered press of the same key.
    #[test]
    fn a_surface_claimed_press_withholds_only_its_own_release() {
        clear();
        let log = Rc::new(RefCell::new(Vec::new()));
        let l = log.clone();
        let _a = on_app_key(move |k| {
            l.borrow_mut().push(format!("{:?}:{}", k.kind, k.code));
            false
        });
        withhold_app_key_release("KeyW");
        dispatch_app_key(&up("KeyW"));
        dispatch_app_key(&up("KeyW"));
        withhold_app_key_release("KeyS");
        dispatch_app_key(&down("KeyS"));
        dispatch_app_key(&up("KeyS"));
        assert_eq!(*log.borrow(), ["Up:KeyW", "Down:KeyS", "Up:KeyS"]);
        clear();
    }

    /// A listener registered for one document does not hear another's keys;
    /// one registered outside any document hears both.
    #[test]
    fn a_documents_listener_hears_only_its_documents_keys() {
        clear();
        let log = Rc::new(RefCell::new(Vec::new()));
        let l = log.clone();
        {
            let _d = crate::context::push_dispatching_doc(7);
            on_app_key(move |k| {
                l.borrow_mut().push(format!("doc7:{}", k.key));
                false
            });
        }
        let l = log.clone();
        on_app_key(move |k| {
            l.borrow_mut().push(format!("any:{}", k.key));
            false
        });
        {
            let _d = crate::context::push_dispatching_doc(8);
            dispatch_app_key(&down("a"));
        }
        {
            let _d = crate::context::push_dispatching_doc(7);
            dispatch_app_key(&down("b"));
        }
        assert_eq!(*log.borrow(), ["any:a", "doc7:b", "any:b"]);
        clear();
    }

    /// A listener that removes a later one during dispatch stops it hearing
    /// that same key.
    #[test]
    fn a_listener_removed_mid_dispatch_is_not_called() {
        clear();
        let hits = Rc::new(Cell::new(0));
        let victim: Rc<RefCell<Option<AppKeyListener>>> = Rc::new(RefCell::new(None));
        let v = victim.clone();
        let _a = on_app_key(move |_| {
            if let Some(l) = v.borrow_mut().take() {
                l.remove();
            }
            false
        });
        let h = hits.clone();
        *victim.borrow_mut() = Some(on_app_key(move |_| {
            h.set(h.get() + 1);
            false
        }));
        dispatch_app_key(&down("KeyA"));
        assert_eq!(hits.get(), 0);
        clear();
    }
}
