//! The public focus-target registry (issue #147).
//!
//! The runtime's focus arbiter ([`FocusTarget`](crate::app::FocusTarget)) is the
//! single authority for which widget owns keyboard input (design A10), and its
//! variant set is closed — a custom component cannot become one. What it *can*
//! do is register callbacks against a focusable DOM node it already owns: the
//! arbiter still holds the claim as `FocusTarget::Node`, and this registry is
//! the lookup that turns that claim into `on_focus_gained` / `on_focus_lost` /
//! `on_key` for the component behind it.
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
//! **Web parity.** This is desktop/Android/embed only — the browser backend
//! (`rinch-web`) has no arbiter, because the browser *is* one: put a real
//! `tabindex` on the node and use the DOM's own `focus`/`blur` events there.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::NodeHandle;
use rinch_core::events::KeyEventData;

/// A registered target's key handler. `true` consumes the key — the runtime's
/// own handling (Tab, Enter/Space activation, DevTools) does not run.
type FocusKeyHandler = Rc<dyn Fn(&KeyEventData) -> bool>;

/// A registered target's caret-rect provider: `(x, y, w, h)` in logical window
/// space, for IME candidate-box placement. See [`FocusEntry::caret_rect`].
type FocusCaretRect = Rc<dyn Fn() -> Option<(f32, f32, f32, f32)>>;

/// What a registered focus target wants to be told. Build with
/// [`FocusEntry::new`] and hand it to [`register_focus_target`]; every callback
/// is optional.
#[derive(Default)]
pub struct FocusEntry {
    on_focus_gained: Option<Rc<dyn Fn()>>,
    on_focus_lost: Option<Rc<dyn Fn()>>,
    on_key: Option<FocusKeyHandler>,
    /// The caret rect this target would place an IME candidate box at, in
    /// logical window space (`x, y, w, h`).
    ///
    /// **Nothing reads this yet** — it is the seam for issue #176, so enabling
    /// desktop IME for a registered target is later one branch in
    /// `RinchApp::ime_state` rather than a re-plumb of the registry. Deciding
    /// *when* a custom target wants IME enabled, and what its preedit contract
    /// is, is that issue's job.
    #[allow(dead_code)]
    caret_rect: Option<FocusCaretRect>,
}

impl std::fmt::Debug for FocusEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FocusEntry")
            .field("on_focus_gained", &self.on_focus_gained.is_some())
            .field("on_focus_lost", &self.on_focus_lost.is_some())
            .field("on_key", &self.on_key.is_some())
            .field("caret_rect", &self.caret_rect.is_some())
            .finish()
    }
}

impl FocusEntry {
    /// An entry with no callbacks. Registering one still makes the node a
    /// first-class focus target (mousedown claims it, Tab reaches it, the
    /// arbiter's liveness check trusts the registration) — it just says nothing
    /// back.
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

    /// Where this target would put an IME candidate box (logical window space).
    /// Accepted and stored, but **not read yet** — reserved for issue #176.
    pub fn caret_rect(mut self, f: impl Fn() -> Option<(f32, f32, f32, f32)> + 'static) -> Self {
        self.caret_rect = Some(Rc::new(f));
        self
    }
}

thread_local! {
    /// `(doc_key, node id, entry)` for every registered focus target.
    ///
    /// Keyed by `(doc_key, node_id)` exactly like the mounted-editor registry:
    /// node ids are per-document slab indices, so two documents on one thread
    /// (two embedded `RinchContext`s, two desktop windows) can both hold a
    /// target at the same node id (issue #134).
    static TARGETS: RefCell<Vec<(u64, usize, Rc<FocusEntry>)>> = const { RefCell::new(Vec::new()) };
}

/// Register `node` as a focus target, so the component behind it hears about
/// focus, blur and keys.
///
/// Replaces any prior registration for **this** node in **this** document;
/// another document's target at a colliding node id is left alone. The
/// registration is dropped when the ambient render scope is disposed — see the
/// module docs for why that is silent.
///
/// Called outside a render (no ambient owner — `main()`, a timer, a detached
/// callback) the entry is still registered, but nothing will ever deregister
/// it: prefer calling this from a component body.
pub fn register_focus_target(node: &NodeHandle, entry: FocusEntry) {
    let doc_key = node.doc_key();
    let node_id = node.node_id().0;
    TARGETS.with(|t| {
        let mut t = t.borrow_mut();
        t.retain(|(dk, id, _)| !(*dk == doc_key && *id == node_id));
        t.push((doc_key, node_id, Rc::new(entry)));
    });
    // Tie the registration to the component that made it. The *ambient owner*,
    // not `RenderScope::on_cleanup`: an `if`/`for` branch renders into a child
    // scope that is never installed as the thread-local render scope, but it
    // does push itself as the owner — so this is the hook that follows a
    // conditionally-mounted widget (issue #141 PR4).
    rinch_core::reactive::on_cleanup(move || unregister_focus_target(doc_key, node_id));
}

/// Forget the target registered at `(doc_key, node_id)`.
pub(crate) fn unregister_focus_target(doc_key: u64, node_id: usize) {
    TARGETS.with(|t| {
        t.borrow_mut()
            .retain(|(dk, id, _)| !(*dk == doc_key && *id == node_id))
    });
}

/// The entry registered at `(doc_key, node_id)`, if any. Cloned out of the
/// registry so the callback runs with no borrow held — it is user code and may
/// register or unregister targets itself.
fn entry_for(doc_key: u64, node_id: usize) -> Option<Rc<FocusEntry>> {
    TARGETS.with(|t| {
        t.borrow()
            .iter()
            .find(|(dk, id, _)| *dk == doc_key && *id == node_id)
            .map(|(_, _, e)| e.clone())
    })
}

/// Whether `(doc_key, node_id)` is a registered focus target.
///
/// This is the arbiter's **liveness authority** for a registered claim: an
/// unmount deregisters through the scope cleanup, which is a push notification
/// rather than the attribute probe `node_target_is_live` falls back to — so the
/// recycled-slab-slot window (#304) is closed for registered targets.
pub(crate) fn is_registered(doc_key: u64, node_id: usize) -> bool {
    TARGETS.with(|t| {
        t.borrow()
            .iter()
            .any(|(dk, id, _)| *dk == doc_key && *id == node_id)
    })
}

/// Fire `on_focus_gained` for a registered target. A no-op for an unregistered
/// node — every generic `tabindex` node takes `FocusTarget::Node`, only some of
/// them registered for the news.
pub(crate) fn notify_focus_gained(doc_key: u64, node_id: usize) {
    if let Some(entry) = entry_for(doc_key, node_id)
        && let Some(cb) = entry.on_focus_gained.clone()
    {
        cb();
    }
}

/// Fire `on_focus_lost` for a registered target. Callers must have completed
/// the focus transition first — this is user code (see [`FocusEntry::on_focus_lost`]).
pub(crate) fn notify_focus_lost(doc_key: u64, node_id: usize) {
    if let Some(entry) = entry_for(doc_key, node_id)
        && let Some(cb) = entry.on_focus_lost.clone()
    {
        cb();
    }
}

/// Offer `key` to the target focused at `(doc_key, node_id)`. `true` means the
/// target consumed it and the runtime must not handle it further.
pub(crate) fn offer_key(doc_key: u64, node_id: usize, key: &KeyEventData) -> bool {
    entry_for(doc_key, node_id)
        .and_then(|entry| entry.on_key.clone())
        .is_some_and(|cb| cb(key))
}
