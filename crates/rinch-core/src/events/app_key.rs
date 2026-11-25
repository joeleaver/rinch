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
//! # Releases follow their press, per listener
//!
//! **A release reaches exactly the listeners that heard its press** (#3062,
//! #3082), whichever window or document it arrives in. Each listener keeps the
//! physical codes of the presses it was offered; a release goes to the
//! listeners holding its code, in registration order, and clears their record.
//! So a key something ahead of the listeners consumed, or an input method
//! composed with, is not theirs on the way up either; a listener after one
//! that consumed a press did not hear it and does not hear its release; and a
//! key held in one window and released in another (a click into a second
//! window, a menu or overlay window taking its parent's keyboard, #2552)
//! reaches the listener that heard it go down — a `main`-registered one and
//! the first window's own alike — and no listener of the second window that
//! never saw it pressed. The element's own `onkeyup` is not paired this way:
//! it hears every release while its element holds the keyboard, as a
//! browser's `keyup` does.
//!
//! A record whose release never arrives (the key let go in another app) is
//! dropped by the next **fresh** press of that key, wherever it lands: the
//! backend reports one with [`note_fresh_key_press`] before the key's dispatch
//! starts, and the listeners that hear that press record it again. A window
//! blur clears nothing: a key held across it may well be released in another
//! of the app's windows, to the listener that heard it. A stale record never
//! blocks a key; at worst, under a host that does not tell a fresh press from
//! an auto-repeat, it lets one more release through to a listener that heard
//! an earlier press of that key.
//!
//! **Whose keys.** On desktop a listener registered while a document's code
//! runs hears that document's presses, and one registered from `main` every
//! document's on the thread — a DevTools window's included. On rinch-web a
//! listener hears every key the page's `document` sees, whichever root
//! registered it: every island's, and keys typed into page inputs outside any
//! rinch root (rinch-web offers the key as no document's, #2811).
//!
//! **Tab and repeats.** A listener that consumes Tab takes it before
//! `trap_focus` containment, which belongs to the default stage. A held key
//! arrives as repeated presses that [`KeyEventData`] does not mark (#797), so a
//! toggle shortcut fires on each repeat.

use std::cell::Cell;
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
    /// Physical codes of the presses this listener heard whose release has
    /// not come yet: only those releases are its (#3062). One entry per code,
    /// so it is bounded by the keyboard, and it goes with the entry.
    held: Vec<String>,
}

crate::doc_keyed! {
    /// Registration order, oldest first: a list, swept by its entries'
    /// documents (#2797), because the order is the dispatch order.
    static LISTENERS: crate::DocList<Entry> = crate::DocList::new(|e| e.doc);
}

thread_local! {
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
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
///   stopped. A **release** goes to every listener that heard its press and
///   the answer is ignored — a release has no default to prevent, and a
///   held-key tracker must see each one.
/// - **Pairing.** A release reaches exactly the listeners that heard its
///   press, whichever window or document it arrives in, and no other: not
///   one whose press something ahead of the listeners consumed (the
///   interceptor, an element's `onkeydown`, the editor's `on_key`, a key
///   entry, a focused surface's claim) or an input method composed with, nor
///   a listener after the one that consumed the press. So a listener sees
///   every key it saw go down come up, even when the key is let go in another
///   of the app's windows.
/// - **Document.** Registered while a document's code runs (its render, an
///   effect it owns, its event handler), a listener hears that document's
///   presses; registered outside any — from `main` — it hears every
///   document's. On rinch-web every listener hears every root's keys.
/// - **Lifetime.** Released when the scope rendering at registration is
///   disposed (and never called after that), else app lifetime;
///   [`AppKeyListener::remove`] releases it earlier, with the presses it
///   holds. Dropping the handle releases nothing, so `on_app_key(…);` in a
///   component just works.
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
        held: Vec::new(),
    };
    LISTENERS.with_mut(|l| l.push(entry));
    // Released with the ambient scope, if there is one.
    crate::reactive::on_cleanup(move || remove(id));
    AppKeyListener(id)
}

/// Offer a key to the app-wide listeners ([`on_app_key`]). Returns whether a
/// listener consumed a **press**; a release always answers `false`.
///
/// A press goes to the listeners of the dispatching document in registration
/// order, until one consumes it, and each listener it reaches records its
/// physical `code`. A release goes to the listeners holding its `code`,
/// whatever the dispatching document, and clears their record. A backend calls
/// this for every press that reaches the listeners' stage and for every
/// release; a release no listener heard the press of is dropped here.
///
/// Backends call this at the place the `module docs` name, after
/// [`dispatch_popup_key`](super::dispatch_popup_key) answered
/// `false` and a focused render surface did not claim the key. Listeners run
/// from a snapshot with no borrow held, so one may register or remove
/// listeners (its own included).
pub fn dispatch_app_key(data: &KeyEventData) -> bool {
    crate::reactive::batch(|| {
        if data.is_up() {
            offer_release(data);
            false
        } else {
            offer_press(data)
        }
    })
}

/// A fresh physical press of `code` arrived — the backend says it is not an
/// auto-repeat — so any press of that key a listener still holds is an earlier
/// one whose release never came (the key was let go in another app). Those
/// records are dropped, for every listener on the thread: the release of this
/// press is offered only to the listeners this press reaches.
///
/// A backend calls it at the start of a fresh press's dispatch, before
/// anything can consume the press: desktop from `handle_event`'s `KeyDown`
/// (and the shell's menu chords, which run before it) for
/// `KeyRepeat::Fresh`, rinch-web from its `window` capture `keydown` for an
/// event whose `repeat` is false. A press the backend cannot classify
/// (`KeyRepeat::Unknown`) is not reported: dropping a held key's record on
/// an auto-repeat would keep its release from a listener that heard it go
/// down, which is worse than the one extra release a stale record can let
/// through.
pub fn note_fresh_key_press(code: &str) {
    LISTENERS.with_mut(|l| {
        for e in l.iter_mut() {
            e.held.retain(|c| c != code);
        }
    });
}

type Candidate = (u64, Option<Owner>, ListenerFn);

fn offer_press(data: &KeyEventData) -> bool {
    let caller = current_dispatching_doc();
    let candidates: Vec<(Option<u64>, Candidate)> = LISTENERS.with(|l| {
        l.iter()
            .map(|e| (e.doc, (e.id, e.owner.clone(), e.f.clone())))
            .collect()
    });
    let mut dead = Vec::new();
    let mut consumed = false;
    for (doc, (id, owner, f)) in candidates {
        // A disposed component's listener reads freed state — never run it.
        if owner.as_ref().is_some_and(|o| !o.is_alive()) {
            dead.push(id);
            continue;
        }
        if !doc_matches(doc, caller) {
            continue;
        }
        // Recorded before it runs: it hears this press whatever it answers,
        // so it hears the release too. False when it is gone (removed by a
        // listener that ran earlier in this dispatch).
        let registered = LISTENERS.with_mut(|l| {
            l.iter_mut().find(|e| e.id == id).map(|e| {
                if !e.held.contains(&data.code) {
                    e.held.push(data.code.clone());
                }
            })
        });
        if registered.is_none() {
            continue;
        }
        if run(owner.as_ref(), &f, data) {
            consumed = true;
            break;
        }
    }
    for id in dead {
        remove(id);
    }
    consumed
}

fn offer_release(data: &KeyEventData) {
    // Exactly the listeners holding this key's press, wherever they were
    // registered; their records are cleared as they are chosen.
    let holders: Vec<Candidate> = LISTENERS.with_mut(|l| {
        l.iter_mut()
            .filter_map(|e| {
                let at = e.held.iter().position(|c| *c == data.code)?;
                e.held.swap_remove(at);
                Some((e.id, e.owner.clone(), e.f.clone()))
            })
            .collect()
    });
    let mut dead = Vec::new();
    for (id, owner, f) in holders {
        if owner.as_ref().is_some_and(|o| !o.is_alive()) {
            dead.push(id);
            continue;
        }
        // Removed by a listener that ran earlier in this dispatch.
        if !LISTENERS.with(|l| l.iter().any(|e| e.id == id)) {
            continue;
        }
        run(owner.as_ref(), &f, data);
    }
    for id in dead {
        remove(id);
    }
}

/// Run one listener untracked, inside its owner scope when it has one.
fn run(owner: Option<&Owner>, f: &ListenerFn, data: &KeyEventData) -> bool {
    crate::reactive::untracked_handler(|| match owner {
        Some(o) => o.run(|| f(data)),
        None => f(data),
    })
}

/// Remove the entry with `id`, and the presses it holds; it is dropped
/// outside the borrow, since a listener closes over user state whose `Drop`
/// may re-enter this list.
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
#[path = "app_key_tests.rs"]
mod tests;
