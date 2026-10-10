//! The public focus-target registry (issue #147).
//!
//! The runtime's focus arbiter ([`FocusTarget`](crate::app::FocusTarget)) is the
//! single authority for which widget owns keyboard input (design A10), and its
//! variant set is closed — a custom component cannot become one. What it *can*
//! do is register callbacks against a focusable DOM node it already owns: the
//! arbiter still holds the claim as `FocusTarget::Node`, and this registry is
//! the lookup that turns that claim into `on_focus_gained` / `on_focus_lost` /
//! `on_key` — and, for a target that says it is a text component,
//! `on_ime` + `caret_rect` (issue #176) — for the component behind it.
//!
//! ```ignore
//! use rinch::prelude::*;
//!
//! #[component]
//! fn code_editor() -> NodeHandle {
//!     let focused = Signal::new(false);
//!     let node = rsx! { div { tabindex: "0", "…" } };
//!     register_focus_target(
//!         &node,
//!         FocusEntry::new()
//!             .on_focus_gained(move || focused.set(true))
//!             .on_focus_lost(move || focused.set(false))
//!             .on_key(move |k| k.key == "ArrowDown" /* consumed */),
//!     );
//!     node
//! }
//! ```
//!
//! **Lifetime.** Registration is tied to the ambient reactive owner — the scope
//! of the component (or the `if`/`for` branch) that registered it, the same
//! `on_cleanup` hook the mounted-editor registry uses. Unmounting the component
//! deregisters it **silently**: `on_focus_lost` does *not* fire, even if the
//! node held focus. Firing it would run user code against a scope whose signals
//! were just freed, which panics (issue #141 PR4). The arbiter notices the
//! vanished target on its next key dispatch and releases the claim.
//!
//! **IME.** A target that registers [`FocusEntry::on_ime`] is a *text* target:
//! while it holds the claim the runtime enables the platform IME on the window
//! and routes every composition event to it, exactly as it does for the
//! rich-text editor and a built-in `<input>` — IME is one shared runtime
//! service riding the arbiter, not a per-widget path (issue #176).
//! [`FocusEntry::caret_rect`] places the OS candidate box.
//!
//! **Web parity.** This is desktop/Android/embed only — the browser backend
//! (`rinch-web`) has no arbiter, because the browser *is* one: put a real
//! `tabindex` on the node and use the DOM's own `focus`/`blur` events there.

use std::cell::RefCell;
use std::rc::Rc;

use std::collections::{HashMap, HashSet};

use rinch_core::dom::{NodeHandle, NodeId};
use rinch_core::events::KeyEventData;
use rinch_core::reactive::{Owner, current_owner};
use rinch_platform::ImeEvent;

/// A registered target's key handler. `true` consumes the key — the runtime's
/// own handling (Tab, Enter/Space activation, DevTools) does not run.
type FocusKeyHandler = Rc<dyn Fn(&KeyEventData) -> bool>;

/// A registered target's caret-rect provider: `(x, y, w, h)` in logical window
/// space, for IME candidate-box placement. See [`FocusEntry::caret_rect`].
type FocusCaretRect = Rc<dyn Fn() -> Option<(f32, f32, f32, f32)>>;

/// A registered target's IME consumer. See [`FocusEntry::on_ime`].
type FocusImeHandler = Rc<dyn Fn(&ImeEvent)>;

/// What a registered focus target wants to be told. Build with
/// [`FocusEntry::new`] and hand it to [`register_focus_target`]; every callback
/// is optional.
#[derive(Default)]
pub struct FocusEntry {
    on_focus_gained: Option<Rc<dyn Fn()>>,
    on_focus_lost: Option<Rc<dyn Fn()>>,
    on_key: Option<FocusKeyHandler>,
    /// The composition consumer. Its presence is what makes this target a
    /// *text* target: `RinchApp::ime_state` enables the platform IME while it
    /// holds the claim (issue #176).
    on_ime: Option<FocusImeHandler>,
    /// The caret rect this target would place an IME candidate box at, in
    /// logical window space (`x, y, w, h`).
    caret_rect: Option<FocusCaretRect>,
}

impl std::fmt::Debug for FocusEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FocusEntry")
            .field("on_focus_gained", &self.on_focus_gained.is_some())
            .field("on_focus_lost", &self.on_focus_lost.is_some())
            .field("on_key", &self.on_key.is_some())
            .field("on_ime", &self.on_ime.is_some())
            .field("caret_rect", &self.caret_rect.is_some())
            .finish()
    }
}

impl FocusEntry {
    /// An entry with no callbacks — a registration that says nothing back.
    ///
    /// Registering does **not** make a node focusable: a mousedown claims, and
    /// Tab reaches, only a node carrying its own `tabindex`. What the
    /// registration buys is the lifecycle (the callbacks below) plus the
    /// arbiter's liveness check trusting it over an attribute probe. Register a
    /// node with no `tabindex` and nothing will ever focus it.
    pub fn new() -> Self {
        Self::default()
    }

    /// Called once this target owns the keyboard, **after** the arbiter and the
    /// DOM focus state are fully installed — so the callback may re-enter the
    /// runtime (move focus again, mutate the DOM) freely.
    ///
    /// Fires for Tab, a mousedown on the node, `NodeHandle::focus()` /
    /// `request_focus`, and when the window regains OS focus while this target
    /// still holds the claim.
    pub fn on_focus_gained(mut self, f: impl Fn() + 'static) -> Self {
        self.on_focus_gained = Some(Rc::new(f));
        self
    }

    /// Called once this target has lost the keyboard — after the transition
    /// completes, never mid-teardown (the arbiter defers it exactly like a
    /// blurred input's `data-onchange` commit, issue #226).
    ///
    /// Fires when another target claims focus (an `<input>`, a `<select>`, the
    /// rich-text editor, a render surface, another registered node), when a
    /// press lands outside, and when the window loses OS focus — the claim is
    /// **kept** across window blur, browser-style, so the pair is
    /// blur → `on_focus_lost`, refocus → `on_focus_gained` with no key routing
    /// in between.
    ///
    /// It does **not** fire when the component unmounts; see the module docs.
    pub fn on_focus_lost(mut self, f: impl Fn() + 'static) -> Self {
        self.on_focus_lost = Some(Rc::new(f));
        self
    }

    /// Offered every `KeyDown` while this target holds focus, **before** the
    /// runtime's own handling. Return `true` to consume the key; `false` lets
    /// it fall through to Tab navigation, Enter/Space activation, DevTools and
    /// the rest.
    ///
    /// This runs after the document-level
    /// [`set_keyboard_interceptor`](rinch_core::events::set_keyboard_interceptor)
    /// hook, which is a capture-phase hook for the whole document and a
    /// different job.
    pub fn on_key(mut self, f: impl Fn(&KeyEventData) -> bool + 'static) -> Self {
        self.on_key = Some(Rc::new(f));
        self
    }

    /// Consume IME composition while this target holds the keyboard (issue
    /// #176). Registering this is what declares the target a **text** target:
    /// the runtime then enables the platform IME on the window for it, places
    /// the candidate box at [`caret_rect`](Self::caret_rect), and routes every
    /// [`ImeEvent`] here — the same five portable variants the rich-text
    /// editor and a built-in `<input>` consume. A target without it drives no
    /// IME at all, so a focusable card or a toolbar button never turns the
    /// OS input method on.
    ///
    /// Runs after the transition that gave this target focus, like the other
    /// callbacks, so it may re-enter the runtime freely. An unmounted target
    /// receives nothing: its scope disposal deregistered it.
    ///
    /// **The runtime never fabricates an event.** It owns no preedit on your
    /// behalf — [`ImeEvent::Preedit`] is a transient overlay *you* render and
    /// *you* discard. In particular a focus change is not an
    /// [`ImeEvent::Disabled`]: when another target claims the keyboard the
    /// window's IME may stay enabled throughout, so nothing ends your
    /// composition but [`on_focus_lost`](Self::on_focus_lost). Drop the
    /// preedit there.
    ///
    /// ```ignore
    /// FocusEntry::new()
    ///     .caret_rect(move || Some(caret.get()))
    ///     .on_ime(move |e| match e {
    ///         ImeEvent::Preedit { text, cursor } => model.set_preedit(text, *cursor),
    ///         ImeEvent::Commit(text) => model.insert(text),
    ///         ImeEvent::Disabled => model.clear_preedit(),
    ///         _ => {}
    ///     })
    /// ```
    pub fn on_ime(mut self, f: impl Fn(&ImeEvent) + 'static) -> Self {
        self.on_ime = Some(Rc::new(f));
        self
    }

    /// Where this target would put an IME candidate box: `(x, y, w, h)` in
    /// **logical window space** — CSS pixels from the window's top-left, the
    /// same space `NodeHandle` layout bounds are reported in, which the shell
    /// hands winit as a `LogicalPosition`/`LogicalSize` and the platform scales
    /// by the window's DPI factor. Do **not** pre-multiply by the scale factor.
    ///
    /// Polled by the runtime whenever it reconciles the window's IME state
    /// (once per event-loop iteration), so the box follows the caret with no
    /// notification needed — but keep it cheap, and do not mutate the DOM from
    /// it. `None` (or no provider at all) leaves placement to the platform.
    ///
    /// Read only for a target that also registers [`on_ime`](Self::on_ime);
    /// on its own it turns nothing on.
    pub fn caret_rect(mut self, f: impl Fn() -> Option<(f32, f32, f32, f32)> + 'static) -> Self {
        self.caret_rect = Some(Rc::new(f));
        self
    }
}

/// One registered focus target.
struct Registered {
    doc_key: u64,
    node_id: usize,
    /// Names the `register_focus_target` call that made this entry
    /// (issue #1490).
    token: u64,
    /// The ambient owner at registration (`None`: app lifetime). Read only
    /// for a target freed while it held the claim (#1509).
    owner: Option<Owner>,
    entry: Rc<FocusEntry>,
}

/// A registered target `set_inner_html` freed while it held the arbiter's
/// claim (#1509): its registration is gone, but its component may be alive
/// and still owed `on_focus_lost` when the arbiter releases the claim.
pub(crate) struct FreedFocusLost {
    owner: Option<Owner>,
    on_focus_lost: Rc<dyn Fn()>,
}

impl FreedFocusLost {
    /// Run the callback — unless its component has been disposed since, which
    /// stays silent like any unmount (#141 PR4).
    pub(crate) fn fire(self) {
        if self.owner.as_ref().is_none_or(Owner::is_alive) {
            let cb = self.on_focus_lost;
            rinch_core::batch(|| cb());
        }
    }
}

impl Registered {
    fn is_at(&self, doc_key: u64, node_id: usize) -> bool {
        self.doc_key == doc_key && self.node_id == node_id
    }
}

thread_local! {
    /// Every registered focus target.
    ///
    /// Keyed by `(doc_key, node_id)` exactly like the mounted-editor registry:
    /// node ids are per-document slab indices, so two documents on one thread
    /// (two embedded `RinchContext`s, two desktop windows) can both hold a
    /// target at the same node id (issue #134).
    ///
    /// The token names the `register_focus_target` call that made the entry
    /// (issue #1490): that call's scope cleanup releases the entry only while
    /// it is still the one registered, since `rinch-dom` re-issues a freed node
    /// id and a cleanup that forgot by id would drop another component's
    /// target.
    static TARGETS: RefCell<Vec<Registered>> = const { RefCell::new(Vec::new()) };

    /// The node each document's arbiter claim is on, as `FocusTarget::Node`
    /// (kept by [`note_claim`]). Lets [`forget_freed_nodes`] tell the one
    /// freed target still owed an `on_focus_lost` from the rest (#1509).
    static CLAIMED: RefCell<HashMap<u64, usize>> = RefCell::new(HashMap::new());

    /// Targets freed while they held their document's claim, until the arbiter
    /// releases it (at most one per document).
    static FREED: RefCell<Vec<(u64, usize, FreedFocusLost)>> = const { RefCell::new(Vec::new()) };

    /// The next registration token. Never reused.
    static NEXT_TOKEN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Register `node` as a focus target, so the component behind it hears about
/// focus, blur and keys.
///
/// Replaces any prior registration for **this** node in **this** document;
/// another document's target at a colliding node id is left alone. The
/// registration is dropped when the ambient render scope is disposed — see the
/// module docs for why that is silent. That disposal drops **this**
/// registration only: one that has replaced it since (a second call for the
/// node, or another component's on a node id the document re-issued) stays
/// (issue #1490).
///
/// Called outside a render (no ambient owner — `main()`, a timer, a detached
/// callback) the entry is still registered, but nothing will ever deregister
/// it: prefer calling this from a component body.
pub fn register_focus_target(node: &NodeHandle, entry: FocusEntry) {
    let doc_key = node.doc_key();
    let node_id = node.node_id().0;
    let token = NEXT_TOKEN.with(|next| {
        let token = next.get();
        next.set(token + 1);
        token
    });
    // A node `set_inner_html` frees takes its entry with it (#1509).
    rinch_core::dom::on_nodes_freed(rinch_core::dom::FreedNodesListener {
        wants: has_targets,
        forget: forget_freed_nodes,
    });
    TARGETS.with(|t| {
        let mut t = t.borrow_mut();
        t.retain(|r| !r.is_at(doc_key, node_id));
        t.push(Registered {
            doc_key,
            node_id,
            token,
            owner: current_owner(),
            entry: Rc::new(entry),
        });
    });
    // Tie the registration to the component that made it. The *ambient owner*,
    // not `RenderScope::on_cleanup`: an `if`/`for` branch renders into a child
    // scope that is never installed as the thread-local render scope, but it
    // does push itself as the owner — so this is the hook that follows a
    // conditionally-mounted widget (issue #141 PR4).
    rinch_core::reactive::on_cleanup(move || unregister_focus_target(doc_key, node_id, token));
}

/// Forget the target `token` registered at `(doc_key, node_id)`, if it is
/// still the one registered there (issue #1490).
fn unregister_focus_target(doc_key: u64, node_id: usize, token: u64) {
    TARGETS.with(|t| {
        t.borrow_mut()
            .retain(|r| !(r.is_at(doc_key, node_id) && r.token == token))
    });
}

/// Whether any focus target is registered on this thread.
fn has_targets() -> bool {
    TARGETS.with(|t| !t.borrow().is_empty())
}

/// Forget every target registered on one of `ids` in `doc_key`: nodes
/// `NodeHandle::set_inner_html` is about to free, whose ids `rinch-dom` hands
/// to the next nodes it mints (#1509). The registering scope's own cleanup
/// later finds nothing and does nothing.
///
/// A target that holds its document's arbiter claim is still owed
/// `on_focus_lost`: its component may be alive. It is parked (see
/// [`was_freed`], [`take_freed`]) until the arbiter releases the claim,
/// which it does at its next key or IME dispatch through the ordinary
/// transition, so the callback runs deferred like any focus work — or not at
/// all if the component is disposed by then. Every other freed entry is
/// dropped silently. Linear in `ids` plus the registered targets.
fn forget_freed_nodes(doc_key: u64, ids: &[NodeId]) {
    let freed: HashSet<usize> = ids.iter().map(|id| id.0).collect();
    let claimed = CLAIMED.with(|c| c.borrow().get(&doc_key).copied());
    let removed: Vec<Registered> = TARGETS.with(|t| {
        let mut t = t.borrow_mut();
        let (gone, kept) = std::mem::take(&mut *t)
            .into_iter()
            .partition(|r| r.doc_key == doc_key && freed.contains(&r.node_id));
        *t = kept;
        gone
    });
    // Out of the `TARGETS` borrow: dropping an entry drops its captures,
    // which are app code.
    for r in removed {
        if Some(r.node_id) == claimed
            && let Some(cb) = r.entry.on_focus_lost.clone()
        {
            let lost = FreedFocusLost {
                owner: r.owner.clone(),
                on_focus_lost: cb,
            };
            FREED.with(|f| f.borrow_mut().push((doc_key, r.node_id, lost)));
        }
    }
}

/// Record the node `doc_key`'s arbiter claim is on now (`None`: not a
/// `FocusTarget::Node`). Called on every arbiter transition, after the
/// previous owner's teardown has taken what it was owed, so a parked freed
/// target that is no longer the claim is dropped here.
pub(crate) fn note_claim(doc_key: u64, node_id: Option<usize>) {
    CLAIMED.with(|c| {
        let mut c = c.borrow_mut();
        match node_id {
            Some(id) => c.insert(doc_key, id),
            None => c.remove(&doc_key),
        }
    });
    let stale: Vec<_> = FREED.with(|f| {
        let mut f = f.borrow_mut();
        let (stale, kept) = std::mem::take(&mut *f)
            .into_iter()
            .partition(|(d, id, _)| *d == doc_key && Some(*id) != node_id);
        *f = kept;
        stale
    });
    drop(stale);
}

/// Whether the claim on `(doc_key, node_id)` is a target `set_inner_html`
/// freed (#1509): the arbiter must release it, whatever node now has the id.
pub(crate) fn was_freed(doc_key: u64, node_id: usize) -> bool {
    FREED.with(|f| {
        f.borrow()
            .iter()
            .any(|(d, id, _)| *d == doc_key && *id == node_id)
    })
}

/// Take the `on_focus_lost` owed to a target freed while it held the claim
/// on `(doc_key, node_id)`, for the arbiter's teardown of that claim.
pub(crate) fn take_freed(doc_key: u64, node_id: usize) -> Option<FreedFocusLost> {
    FREED.with(|f| {
        let mut f = f.borrow_mut();
        let i = f
            .iter()
            .position(|(d, id, _)| *d == doc_key && *id == node_id)?;
        Some(f.remove(i).2)
    })
}

/// The entry registered at `(doc_key, node_id)`, if any. Cloned out of the
/// registry so the callback runs with no borrow held — it is user code and may
/// register or unregister targets itself.
fn entry_for(doc_key: u64, node_id: usize) -> Option<Rc<FocusEntry>> {
    TARGETS.with(|t| {
        t.borrow()
            .iter()
            .find(|r| r.is_at(doc_key, node_id))
            .map(|r| r.entry.clone())
    })
}

/// Whether `(doc_key, node_id)` is a registered focus target.
///
/// This is the arbiter's **liveness authority** for a registered claim: an
/// unmount deregisters through the scope cleanup, which is a push notification
/// rather than the attribute probe `node_target_is_live` falls back to. That
/// closes the recycled-slab-slot window (#304) for registered nodes: an
/// unmount releases the entry by its token (#1490), and a node freed while the
/// registering scope is still alive (`set_inner_html` over it, the one
/// `NodeHandle` verb that frees on `rinch-dom`) takes its entry with it
/// before `rinch-dom` can mint another node on its id (#1509). A freed target
/// that held the claim is still owed `on_focus_lost` — see
/// [`forget_freed_nodes`].
pub(crate) fn is_registered(doc_key: u64, node_id: usize) -> bool {
    TARGETS.with(|t| t.borrow().iter().any(|r| r.is_at(doc_key, node_id)))
}

/// Whether the target at `(doc_key, node_id)` registered [`FocusEntry::on_key`]
/// — i.e. is a custom control that reads keys beyond Enter/Space/Tab (arrow-key
/// navigation, a shortcut of its own).
///
/// This is the question an embed host's keyboard-routing decision should ask
/// (issue #548, [`crate::app::RinchApp::has_focused_key_consumer`]): a generic
/// focusable node (`FocusTarget::Node`) is claimed by **any** mousedown on a
/// plain `<button>`/`<a href>` just as much as by Tab onto a registered custom
/// widget (issue #252 widened the tag-implied `tabindex` set), but a plain
/// button with no registered `on_key` only ever consumes Enter/Space through
/// the runtime's own activation path. `false` for an unregistered node (the
/// plain-button case) and for a registered one that only asked for
/// focus/blur/IME notifications, not keys.
pub(crate) fn wants_key_routing(doc_key: u64, node_id: usize) -> bool {
    entry_for(doc_key, node_id).is_some_and(|entry| entry.on_key.is_some())
}

/// Fire `on_focus_gained` for a registered target. A no-op for an unregistered
/// node — every generic `tabindex` node takes `FocusTarget::Node`, only some of
/// them registered for the news.
pub(crate) fn notify_focus_gained(doc_key: u64, node_id: usize) {
    if let Some(entry) = entry_for(doc_key, node_id)
        && let Some(cb) = entry.on_focus_gained.clone()
    {
        // One transaction per callback, like every event handler.
        rinch_core::batch(|| cb());
    }
}

/// Fire `on_focus_lost` for a registered target. Callers must have completed
/// the focus transition first — this is user code (see [`FocusEntry::on_focus_lost`]).
pub(crate) fn notify_focus_lost(doc_key: u64, node_id: usize) {
    if let Some(entry) = entry_for(doc_key, node_id)
        && let Some(cb) = entry.on_focus_lost.clone()
    {
        rinch_core::batch(|| cb());
    }
}

/// Offer `key` to the target focused at `(doc_key, node_id)`. `true` means the
/// target consumed it and the runtime must not handle it further.
pub(crate) fn offer_key(doc_key: u64, node_id: usize, key: &KeyEventData) -> bool {
    entry_for(doc_key, node_id)
        .and_then(|entry| entry.on_key.clone())
        .is_some_and(|cb| rinch_core::batch(|| cb(key)))
}

/// Whether the target at `(doc_key, node_id)` consumes IME composition — i.e.
/// registered [`FocusEntry::on_ime`] (issue #176).
///
/// This is the arbiter's enablement predicate: `false` for an unregistered
/// node, and for a registered one that is focusable but not *text* (a card, a
/// toolbar button), so focusing those does not switch the OS input method on.
/// An unmounted target answers `false` for free — its registration is gone.
#[cfg(any(feature = "desktop", test))] // read only by `RinchApp::ime_state`
pub(crate) fn wants_ime(doc_key: u64, node_id: usize) -> bool {
    entry_for(doc_key, node_id).is_some_and(|entry| entry.on_ime.is_some())
}

/// The target's IME candidate-box rect in logical window space, freshly read
/// from its [`FocusEntry::caret_rect`] provider. `None` when it has none, or
/// when it has no caret right now.
#[cfg(any(feature = "desktop", test))] // read only by `RinchApp::ime_state`
pub(crate) fn caret_rect_of(doc_key: u64, node_id: usize) -> Option<(f32, f32, f32, f32)> {
    entry_for(doc_key, node_id)
        .and_then(|entry| entry.caret_rect.clone())
        .and_then(|cb| cb())
}

/// Deliver `ime` to the target focused at `(doc_key, node_id)`. Returns whether
/// anything consumed it — `false` for an unregistered, unmounted, or non-text
/// target, which is the same silence rule the focus callbacks obey.
pub(crate) fn offer_ime(doc_key: u64, node_id: usize, ime: &ImeEvent) -> bool {
    match entry_for(doc_key, node_id).and_then(|entry| entry.on_ime.clone()) {
        Some(cb) => {
            rinch_core::batch(|| cb(ime));
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rinch_core::dom::DomDocument;
    use rinch_core::dom::mock::MockDomDocument;
    use rinch_core::reactive::Scope;
    use std::cell::RefCell;
    use std::rc::Weak;

    /// Issue #1490: a registration's scope cleanup releases that registration,
    /// not whatever is registered at `(doc_key, node_id)` when it runs. On
    /// `rinch-dom` a freed node id is handed to the next node minted, so the
    /// "second" registration there is another component's; a second
    /// registration on the same node reaches the same cleanup without needing
    /// the id re-issued.
    #[test]
    fn a_replaced_target_survives_the_first_scopes_cleanup() {
        let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(MockDomDocument::new()));
        let weak: Weak<RefCell<dyn DomDocument>> = Rc::downgrade(&doc);
        let id = doc.borrow_mut().create_element("div");
        let node = NodeHandle::new(id, weak);
        let (doc_key, node_id) = (node.doc_key(), node.node_id().0);

        let first = Scope::new();
        first.run(|| register_focus_target(&node, FocusEntry::new()));
        assert!(
            !wants_key_routing(doc_key, node_id),
            "control: no on_key yet"
        );
        let second = Scope::new();
        second.run(|| register_focus_target(&node, FocusEntry::new().on_key(|_| true)));
        assert!(
            wants_key_routing(doc_key, node_id),
            "control: the second registration replaced the first"
        );

        first.dispose();
        assert!(
            wants_key_routing(doc_key, node_id),
            "the first scope's cleanup must not take the registration that \
             replaced its own"
        );

        second.dispose();
        assert!(
            !is_registered(doc_key, node_id),
            "the second scope's own cleanup does release it"
        );
    }
}
