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
//! Both backends offer a key press, in order, to (issue #3062; the whole order
//! is `docs/internals/focus-and-keyboard.md` "The key handler order"): the
//! app's menu chords and rinch's three keyboard owners (Escape ending a drag
//! or a pointer lock, an open native `<select>` list, the text context menu)
//! → [`set_keyboard_interceptor`](super::set_keyboard_interceptor) → the
//! focused editor's / node view's `on_key` → the focused element's `onkeydown`
//! and its ancestors' → open popups' [key entries](super::push_key_handler) →
//! the [dismiss stack](super::push_dismiss_handler) → a focused registered
//! widget's `FocusEntry::on_key` (desktop) → a focused render surface's own
//! claim (`RenderSurfaceHandle::set_key_handler`) → **these listeners** →
//! rinch's defaults: the rich-text editor's caret keys, Tab and keymap, a
//! field's text editing, Tab, Enter/Space activation, `onsubmit`. So
//! everything that owns a key while it is open or focused — an open `Select`
//! list, a `Modal` closing on Escape, a code editor's interceptor, a game
//! canvas — wins, and a listener still runs before the default action, as a
//! `window` listener runs before a browser's.
//!
//! A key typed into a text field or the rich-text editor therefore reaches the
//! listeners, as it reaches a browser's `window` listener, and a listener that
//! takes it takes it from the editor's keymap too (Ctrl+B, Enter, a typed
//! letter). A shortcut that should leave text alone asks
//! [`NodeHandle::active_element`](crate::dom::NodeHandle::active_element).
//! Not offered at all: a press while an input method composes, and on desktop
//! while the built-in text context menu or an open native `<select>` list owns
//! the keyboard.
//!
//! **Releases** follow their press: a release is offered to the listeners only
//! when its press was, so a key something ahead of them consumed, or an input
//! method composed with, is not theirs on the way up either. The element's own
//! `onkeyup` is not paired this way: it hears every release while its element
//! holds the keyboard, as a browser's `keyup` does. A window that loses the
//! keyboard forgets every press it offered ([`forget_app_key_presses`]): the
//! release of a key held across the blur goes elsewhere, and the next release
//! of that key here belongs to a press made after it.
//!
//! **Whose keys.** On desktop a listener registered while a document's code
//! runs hears that document's keys, and one registered from `main` every
//! document's on the thread — a DevTools window's included. On rinch-web a
//! listener hears every key the page's `document` sees, whichever root
//! registered it: every island's, and keys typed into page inputs outside any
//! rinch root (rinch-web offers the key as no document's, #2811).
//!
//! **Tab and repeats.** A listener that consumes Tab takes it before
//! `trap_focus` containment, which belongs to the default stage. A held key
//! arrives as repeated presses that [`KeyEventData`] does not mark (#797), so a
//! toggle shortcut fires on each repeat.

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

crate::doc_keyed! {
    /// Registration order, oldest first: a list, swept by its entries'
    /// documents (#2797), because the order is the dispatch order.
    static LISTENERS: crate::DocList<Entry> = crate::DocList::new(|e| e.doc);
}

thread_local! {
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
    /// Physical codes whose press a focused render surface claimed: their
    /// release is the surface's too ([`withhold_app_key_release`]).
    static WITHHELD: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    /// Physical codes whose press was offered to the listeners and whose
    /// release has not come yet: only those releases are offered (#3062). A
    /// press something earlier consumed (an interceptor, an element's
    /// `onkeydown`, the editor's `on_key`, a key entry, a surface) or that an
    /// input method composed with never reaches [`dispatch_app_key`], so its
    /// release does not either. Cleared on a window blur
    /// ([`forget_app_key_presses`]).
    static PRESSED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
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
/// including nothing (issue #1784). See the `module docs` for where it
/// sits among the other key handlers.
///
/// - **Order.** Listeners are offered a key in registration order. For a
///   **press**, the first to answer `true` consumes it: no later listener and
///   no default action (text insertion, Tab, activation, `onsubmit`) sees it,
///   and on rinch-web the event is `preventDefault`ed and its propagation
///   stopped. A **release** goes to every listener and the answer is ignored —
///   a release has no default to prevent, and a held-key tracker must see each
///   one. A release is offered only when its **press** was: a key whose press
///   something ahead of the listeners consumed (the interceptor, an element's
///   `onkeydown`, the editor's `on_key`, a key entry, a focused surface's
///   claim) or an input method composed with is not theirs, release included.
///   A listener later in the list than one that consumed the press still hears
///   the release, as every listener hears every release it was offered.
/// - **Document.** Registered while a document's code runs (its render, an
///   effect it owns, its event handler), a listener hears that document's keys;
///   registered outside any — from `main` — it hears every document's. On
///   rinch-web every listener hears every root's keys.
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
    LISTENERS.with_mut(|l| l.push(entry));
    // Released with the ambient scope, if there is one.
    crate::reactive::on_cleanup(move || remove(id));
    AppKeyListener(id)
}

/// Offer a key to the app-wide listeners ([`on_app_key`]). Returns whether a
/// listener consumed a **press**; a release always answers `false`.
///
/// A press is recorded by its physical `code`, and a release is offered only
/// when its press was (then the record is cleared): a backend calls this for
/// every press that reaches the listeners' stage and every release, and a
/// release whose press stopped earlier is dropped here.
///
/// Backends call this at the place the `module docs` name, after
/// [`dispatch_popup_key`](super::dispatch_popup_key) answered
/// `false` and a focused render surface did not claim the key. Listeners run
/// from a snapshot with no borrow held, so one may register or remove
/// listeners (its own included).
pub fn dispatch_app_key(data: &KeyEventData) -> bool {
    if data.is_up() {
        let withheld = WITHHELD.with(|w| w.borrow_mut().remove(&data.code));
        let pressed = PRESSED.with(|p| p.borrow_mut().remove(&data.code));
        if withheld || !pressed {
            return false;
        }
    } else {
        // A press that reached us: whatever an earlier release left marked for
        // this key is stale (that release never arrived — a window blur).
        WITHHELD.with(|w| w.borrow_mut().remove(&data.code));
        PRESSED.with(|p| p.borrow_mut().insert(data.code.clone()));
    }
    crate::reactive::batch(|| dispatch_unbatched(data))
}

fn dispatch_unbatched(data: &KeyEventData) -> bool {
    let caller = current_dispatching_doc();
    let candidates: Vec<(u64, Option<u64>, Option<Owner>, ListenerFn)> = LISTENERS.with(|l| {
        l.iter()
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
        if !LISTENERS.with(|l| l.iter().any(|e| e.id == id)) {
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

/// Forget every press offered to the listeners and every release a focused
/// render surface withheld: the window lost the keyboard. The release of a key
/// held across the blur goes to whatever took the keyboard, so without this a
/// later press of that key consumed ahead of the listeners (a field's
/// `onkeydown`) would have its release offered to them, paired with the stale
/// press. The desktop runtime calls it from `PlatformEvent::WindowFocus(false)`
/// and rinch-web from a `window` `blur`.
///
/// The records are the thread's, not a document's: any window's blur clears
/// them all, as a key cannot be held in a window without the keyboard.
pub fn forget_app_key_presses() {
    WITHHELD.with(|w| w.borrow_mut().clear());
    PRESSED.with(|p| p.borrow_mut().clear());
}

/// Whether the release of the physical key `code` will be withheld from the
/// listeners, because a focused render surface claimed its press
/// ([`withhold_app_key_release`]). Read it before [`dispatch_app_key`] is
/// offered the release, which clears the mark. rinch-web asks it so that it
/// stops the propagation of only those releases (issue #3062), as the desktop
/// withholds only those.
pub fn app_key_release_withheld(code: &str) -> bool {
    WITHHELD.with(|w| w.borrow().contains(code))
}

/// Remove the entry with `id`; it is dropped outside the borrow, since a
/// listener closes over user state whose `Drop` may re-enter this list.
fn remove(id: u64) {
    let reaped =
        LISTENERS.try_with_mut(|list| list.iter().position(|e| e.id == id).map(|i| list.remove(i)));
    drop(reaped);
}

/// How many app-wide key listeners are registered on this thread. Diagnostics
/// and tests only.
#[doc(hidden)]
pub fn app_key_listener_count() -> usize {
    LISTENERS.with(|l| l.len())
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
        let reaped = LISTENERS.with_mut(|l| std::mem::take(&mut **l));
        drop(reaped);
        WITHHELD.with(|w| w.borrow_mut().clear());
        PRESSED.with(|p| p.borrow_mut().clear());
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
        assert!(app_key_release_withheld("KeyW"));
        dispatch_app_key(&up("KeyW"));
        assert!(
            !app_key_release_withheld("KeyW"),
            "the release clears its own mark"
        );
        withhold_app_key_release("KeyS");
        dispatch_app_key(&down("KeyS"));
        dispatch_app_key(&up("KeyS"));
        assert_eq!(*log.borrow(), ["Down:KeyS", "Up:KeyS"]);
        clear();
    }

    /// A release reaches the listeners only when its press did (#3062): one
    /// whose press stopped ahead of them is not theirs. A delivered press's
    /// release is offered once, and a press a listener consumed still has its
    /// release offered to every listener.
    #[test]
    fn a_release_is_offered_only_after_its_press_was() {
        clear();
        let log = Rc::new(RefCell::new(Vec::new()));
        let l = log.clone();
        let _a = on_app_key(move |k| {
            l.borrow_mut().push(format!("a:{:?}:{}", k.kind, k.code));
            k.code == "KeyC"
        });
        let l = log.clone();
        let _b = on_app_key(move |k| {
            l.borrow_mut().push(format!("b:{:?}:{}", k.kind, k.code));
            false
        });
        // KeyX's press was consumed before the listeners' stage.
        dispatch_app_key(&up("KeyX"));
        dispatch_app_key(&down("KeyY"));
        dispatch_app_key(&down("KeyY"));
        dispatch_app_key(&up("KeyY"));
        dispatch_app_key(&up("KeyY"));
        assert!(dispatch_app_key(&down("KeyC")));
        dispatch_app_key(&up("KeyC"));
        assert_eq!(
            *log.borrow(),
            [
                "a:Down:KeyY",
                "b:Down:KeyY",
                "a:Down:KeyY",
                "b:Down:KeyY",
                "a:Up:KeyY",
                "b:Up:KeyY",
                "a:Down:KeyC",
                "a:Up:KeyC",
                "b:Up:KeyC",
            ]
        );
        clear();
    }

    /// A window blur forgets the presses the listeners were offered and the
    /// releases a surface withheld: the release of a key held across it is
    /// lost, and the next release of that key here is a later press's.
    #[test]
    fn a_window_blur_forgets_offered_presses_and_withheld_releases() {
        clear();
        let log = Rc::new(RefCell::new(Vec::new()));
        let l = log.clone();
        let _a = on_app_key(move |k| {
            l.borrow_mut().push(format!("{:?}:{}", k.kind, k.code));
            false
        });
        dispatch_app_key(&down("KeyS"));
        withhold_app_key_release("KeyW");
        forget_app_key_presses();
        assert!(!app_key_release_withheld("KeyW"));
        // A later S press stopped ahead of the listeners: its release is not
        // theirs.
        dispatch_app_key(&up("KeyS"));
        assert_eq!(*log.borrow(), ["Down:KeyS"]);
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
