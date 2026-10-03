//! RenderScope, UpdateBatch, and DomUpdate types.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use crate::reactive::{Effect, Scope};

use super::traits::DomDocument;
use super::{NodeHandle, NodeId, next_reactive_id};

// ============================================================================
// Scope ancestry (issue #732, rounds 3 and 4)
// ============================================================================
//
// Rounds 1 and 2 of #732 both tried to answer "is this node owned by my
// render" without actually tracking ownership: round 1 asked `created`
// (exact — this scope and no one else, which misses nodes a *nested* child
// scope minted and leaks them); round 2 compared raw `NodeId` values against
// a per-scope watermark (an id-ordering PROXY for ownership, which a node
// minted by anything else running synchronously inside the same render —
// not a descendant scope at all — satisfies purely by chronological
// accident, so it gets swept into a discard it should have been excluded
// from). Both were caught by adversarial review before merge.
//
// Round 3 introduced real ancestry (a [`ScopeId`] per scope, a recorded
// parent, a `node -> minting scope` table) but established the parent link
// **dynamically**: a scope constructed while some outer scope's
// `push_owner()` guard happened to be on the call stack recorded that
// scope's id as its parent. That window is only the outer scope's own
// **initial, synchronous** render call — a `for` row inserted later by its
// own reconcile `Effect` (list growth), a nested branch flipping later, or a
// nested component re-rendering later all construct their new content's
// `RenderScope` from a later reactive flush, with nothing of the outer
// branch's on the stack any more, so the link was never made and the new
// content was misclassified as captured. Caught by review before merge.
//
// Round 4 makes the link **static** instead: each of the four reactive
// helpers (`show_dom`, `match_dom`, `for_each_dom_typed`,
// `reactive_component_dom`) captures the [`ScopeId`] of the scope it was
// itself called from — [`RenderScope::id`] of its `scope: &mut RenderScope`
// parameter — once, at the moment the helper is invoked, and threads that
// one fixed id into every `RenderScope` it ever constructs for this call's
// content from then on, via [`RenderScope::with_parent`] — at the initial
// render AND at every later reconcile/re-render the helper's own persistent
// `Effect` performs, however much later that runs. There is no more ambient
// stack to get the timing of wrong: the parent a scope is given is decided
// once, by the helper that will always be its logical parent, not by
// whatever else happens to be executing when it happens to be constructed.

/// A globally unique id for one `RenderScope` instance, for exactly as long
/// as that scope (or any node it minted) might still be asked about.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct ScopeId(u64);

/// One document's minting table — see [`DOC_MINTED_BY`].
type MintedByTable = Rc<RefCell<HashMap<NodeId, ScopeId>>>;
/// A non-owning reference to one document's minting table — what
/// [`DOC_MINTED_BY`] itself holds.
type WeakMintedByTable = Weak<RefCell<HashMap<NodeId, ScopeId>>>;

thread_local! {
    /// Monotonic generator for [`ScopeId`] — one counter per thread, since
    /// `RenderScope`s (like `NodeId`s) are never compared across threads.
    static NEXT_SCOPE_ID: Cell<u64> = const { Cell::new(0) };

    /// Every live scope's parent, `None` for one built with nothing
    /// explicitly supplied ([`RenderScope::new`] — a document's root scope,
    /// or any scope built outside a reactive helper entirely). Removed when
    /// the scope itself is dropped — see `RenderScope`'s `Drop` impl — which
    /// is safe once nothing can walk through it any more: a walk only ever
    /// needs an *ancestor's* parent, read while collecting before the
    /// branch's own scope is disposed, and a descendant scope (a row, a
    /// nested branch) outlives or is disposed no later than its own
    /// content, independently of this map.
    static SCOPE_PARENTS: RefCell<HashMap<ScopeId, Option<ScopeId>>> = RefCell::new(HashMap::new());

    /// One shared minting table **per document**, so it can be dropped with
    /// the document instead of living in a single flat thread-local forever
    /// (issue #732, round 4's second finding). Keyed by `doc_key` (issue
    /// #134: `NodeId` collides across documents on one thread), holding only
    /// a [`Weak`] reference — the [`Rc`] strong reference lives on every
    /// `RenderScope` for that document (`RenderScope::minted_by`), so the
    /// table deallocates itself, by ordinary `Rc` counting, the moment the
    /// last `RenderScope` referencing that document is dropped. That happens
    /// no later than the document itself stops being used — an embed
    /// `RinchContext` dropped directly, a window closed without an explicit
    /// node-by-node discard walk — which is exactly the teardown path round
    /// 3's single flat map had no hook for at all. A document that calls
    /// [`NodeHandle::discard`] on everything purges incrementally as it goes
    /// (see `purge_minted_by_subtree`) and this table is already near-empty
    /// by the time its last scope drops; one that doesn't still cannot leak
    /// past that point.
    static DOC_MINTED_BY: RefCell<HashMap<u64, WeakMintedByTable>> =
        RefCell::new(HashMap::new());
}

fn next_scope_id() -> ScopeId {
    NEXT_SCOPE_ID.with(|c| {
        let id = c.get();
        c.set(id + 1);
        ScopeId(id)
    })
}

/// The shared minting table for `doc_key`, creating it if this is the first
/// `RenderScope` on this thread to ask for it. Returns a **strong**
/// reference — see [`DOC_MINTED_BY`]'s own doc for why holding it is what
/// keeps the table alive for exactly as long as it is needed.
fn doc_minted_by_table(doc_key: u64) -> MintedByTable {
    DOC_MINTED_BY.with(|r| {
        let mut r = r.borrow_mut();
        if let Some(existing) = r.get(&doc_key).and_then(Weak::upgrade) {
            return existing;
        }
        let table = Rc::new(RefCell::new(HashMap::new()));
        r.insert(doc_key, Rc::downgrade(&table));
        table
    })
}

/// Remove every id under `root`'s subtree from its document's minting table
/// (issue #732) — called from [`NodeHandle::discard`] before the backend
/// retires the subtree, while `get_children` still answers.
///
/// Bounded by the discarded subtree, the same shape as
/// [`collect_captured_descendants`](super::collect_captured_descendants)'s
/// walk and no more expensive than the recursive retire the backend itself
/// performs: one extra `get_children` pass over exactly what is being thrown
/// away, never over the whole document. This is the **incremental** half of
/// keeping the minting table bounded; the other half, for a document torn
/// down WITHOUT ever calling `discard`, is `DOC_MINTED_BY`'s own `Rc`/`Weak`
/// structure (see its doc). If no `RenderScope` for this document is alive
/// any more, there is nothing to purge — `doc_minted_by_table` would only
/// mint a fresh, empty table, so this is a no-op rather than resurrecting one.
pub(crate) fn purge_minted_by_subtree(root: &NodeHandle) {
    let doc_key = root.doc_key();
    let Some(table) = DOC_MINTED_BY.with(|r| r.borrow().get(&doc_key).and_then(Weak::upgrade))
    else {
        return;
    };
    fn walk(node: &NodeHandle, table: &RefCell<HashMap<NodeId, ScopeId>>) {
        table.borrow_mut().remove(&node.node_id());
        for child in node.children() {
            walk(&child, table);
        }
    }
    walk(root, &table);
}

/// **Test-only.** How many nodes some document's minting table currently
/// holds a minting scope for, summed across every document that still has a
/// live `RenderScope` on this thread — every node some live `RenderScope`
/// has minted and no discard has yet purged, in a document that has not
/// (yet) been dropped out from under its scopes entirely (issue #732).
/// What a growth fixture compares across many toggles to prove this does
/// not leak, and what a document-teardown fixture compares before and after
/// dropping a whole document's scope tree to prove that path does not leak
/// either — a dropped document's table is gone from this sum the moment its
/// last `RenderScope` is dropped, with no further action needed.
#[doc(hidden)]
pub fn __minted_by_len() -> usize {
    DOC_MINTED_BY.with(|r| {
        r.borrow()
            .values()
            .filter_map(Weak::upgrade)
            .map(|t| t.borrow().len())
            .sum()
    })
}

/// **Test-only.** How many scopes [`SCOPE_PARENTS`] currently holds a parent
/// entry for — every `RenderScope` that exists right now, anywhere on this
/// thread. Same growth-fixture purpose as [`__minted_by_len`].
#[doc(hidden)]
pub fn __scope_parents_len() -> usize {
    SCOPE_PARENTS.with(|m| m.borrow().len())
}

/// Context for building DOM trees with automatic effect tracking.
///
/// RenderScope provides:
/// - DOM node creation methods that return [`NodeHandle`]s
/// - Effect registration for reactive bindings
/// - Cleanup tracking for proper disposal
///
/// # Lifecycle
///
/// When a RenderScope is dropped, all effects created within it are disposed,
/// and any cleanup functions are called.
pub struct RenderScope {
    /// Weak reference to the document.
    doc: Weak<RefCell<dyn DomDocument>>,
    /// The parent node for new children.
    parent_id: NodeId,
    /// Node ids this scope minted, in creation order (issue #719).
    ///
    /// This is **ownership of nodes**, the same rule #141 PR4 gave signals and
    /// effects: a node the scope created belongs to it, and a node handed to the
    /// scope from outside does not. It is what lets a branch helper tell a
    /// subtree it built — which nothing can ever show again once the branch
    /// flips — from a *captured* `NodeHandle` the caller may re-show, without
    /// guessing. See [`RenderScope::created`].
    created: Vec<NodeId>,
    /// Effects created within this scope (for future direct tracking).
    #[allow(dead_code)]
    effects: Vec<Effect>,
    /// Child scopes (for hierarchical cleanup).
    children: Vec<RenderScope>,
    /// The reactive scope for effect management **and cleanups** — see
    /// [`RenderScope::on_cleanup`]. Cleanups deliberately live here and not in a
    /// second list on `RenderScope`, so they run on drop as well as on dispose.
    reactive_scope: Scope,
    /// This scope's own identity in the ancestry tracking above (issue #732).
    /// Assigned once, at construction, and never reused.
    id: ScopeId,
    /// This scope's document's shared minting table — a strong reference,
    /// which is what keeps [`DOC_MINTED_BY`]'s entry for this document alive
    /// for exactly as long as any `RenderScope` for it still exists. See that
    /// static's own doc.
    minted_by: MintedByTable,
}

impl RenderScope {
    /// Create a new render scope rooted at the given node, with **no**
    /// parent in the ancestry tracking above — correct for a document's
    /// root scope, or any scope built outside a reactive helper entirely.
    ///
    /// A reactive helper (`show_dom`, `match_dom`, `for_each_dom_typed`,
    /// `reactive_component_dom`) must call
    /// [`with_parent`](Self::with_parent) instead, naming the scope it was
    /// itself invoked from — see the module-level doc above [`ScopeId`] for
    /// why that has to be a fixed id captured once rather than read from an
    /// ambient stack at construction time.
    pub fn new(doc: Rc<RefCell<dyn DomDocument>>, parent_id: NodeId) -> Self {
        Self::with_parent(doc, parent_id, None)
    }

    /// Create a new render scope rooted at the given node, recording
    /// `parent` as its ancestry parent (issue #732, round 4) — `None` is
    /// exactly [`new`](Self::new).
    ///
    /// Every reactive helper that constructs content on behalf of an outer
    /// scope — including from a later `Effect` re-run, not only its own
    /// initial render — must pass that outer scope's
    /// [`id`](Self::id), captured once when the helper itself was called, so
    /// that content a `for` row insert, a nested branch flip, or a nested
    /// component re-render builds **later** still chains back to the right
    /// ancestor; an ambient "what's on the call stack right now" answer is
    /// wrong exactly at this moment, since a later `Effect`'s flush is not
    /// nested inside anything the outer branch's own render call pushed.
    pub(crate) fn with_parent(
        doc: Rc<RefCell<dyn DomDocument>>,
        parent_id: NodeId,
        parent: Option<ScopeId>,
    ) -> Self {
        let id = next_scope_id();
        SCOPE_PARENTS.with(|m| {
            m.borrow_mut().insert(id, parent);
        });
        let doc_key = doc.borrow().doc_key();
        Self {
            doc: Rc::downgrade(&doc),
            parent_id,
            created: Vec::new(),
            effects: Vec::new(),
            children: Vec::new(),
            reactive_scope: Scope::new(),
            id,
            minted_by: doc_minted_by_table(doc_key),
        }
    }

    /// This scope's own [`ScopeId`] (issue #732, round 4) — what a reactive
    /// helper captures, once, at the moment it is called, to hand to every
    /// `RenderScope` it later constructs via
    /// [`with_parent`](Self::with_parent).
    pub(crate) fn id(&self) -> ScopeId {
        self.id
    }

    /// Whether this scope minted `node` (issue #719).
    ///
    /// The question a reactive branch helper has to answer on a hide: **did my
    /// branch closure build this, or was it handed to me?** A node this scope
    /// created can never be shown again once the branch flips, so its
    /// bookkeeping is released ([`NodeHandle::discard`]); one it did not create
    /// is the caller's, may come back on the next show, and is only detached
    /// ([`NodeHandle::remove`]).
    ///
    /// **Only the content root is asked here**, and that is deliberate:
    /// discarding is recursive, so a root this scope built takes its whole
    /// subtree with it — including nodes minted by *nested* scopes (a `for`'s
    /// rows, an inner branch), which is exactly right, since nothing outside
    /// can be holding them either.
    ///
    /// A captured handle nested **inside** branch-built markup
    /// (`if open { div { {panel} } }`) used to get this wrong: the `div` is
    /// this scope's, the recursion reached straight through to `panel`, and
    /// `panel` was retired with the wrapper (issue #732). It no longer does —
    /// every caller that discards a content root this scope created first
    /// calls [`collect_captured_descendants`](super::collect_captured_descendants)
    /// (or the convenience
    /// [`discard_owned_preserving_captured`](super::discard_owned_preserving_captured)
    /// that wraps it) over that root, which stops descending the moment a node
    /// answers [`owns_transitively`](Self::owns_transitively) as `false` and
    /// detaches it rather than letting the discard reach it — **not**
    /// `created`, which answers `false` for a node a nested child scope
    /// minted (a `for` row, a re-rendered component's output) just as it does
    /// for a genuine capture, and so cannot tell the two apart; the first
    /// round of this fix used `created` directly and leaked every nested
    /// `for`/component inside a branch. (A second round tried a per-scope id
    /// *watermark* instead — "no smaller than the first id this scope
    /// minted" — which fixed that leak but was itself only an id-ordering
    /// proxy for ownership: a node minted by anything else running
    /// synchronously in the same closure, not a descendant scope at all,
    /// satisfied it by chronological accident and was wrongly swept into the
    /// discard. Both rounds were caught by review before merge.) A caller
    /// must collect before disposing the scope, since `owns_transitively`
    /// reads the ancestry tables below, which the scope's own entry is
    /// removed from on drop.
    ///
    /// **The verb is chosen before the scope is disposed**, so a cleanup that
    /// re-parents a scope-built node while disposal runs cannot rescue it: the
    /// node was already recorded as the helper's and is released a moment later.
    /// That ordering is deliberate — disposal runs user code, and the ownership
    /// answer has to be the one that was true when the branch rendered — but it
    /// means "move this node somewhere safe in `on_cleanup`" is not a supported
    /// escape. Build it outside the closure and hand it in instead.
    ///
    /// And it answers for nodes **in a subtree**. A node this scope minted and
    /// attached to nothing is reached by no walk at all — the `<template>` an
    /// `rsx!` component site builds its children in is exactly that, which is
    /// why [`release_scratch_container`](super::release_scratch_container)
    /// exists.
    ///
    /// Linear over the ids the scope minted. A branch scope holds one render's
    /// worth, and the root is almost always its first — this is not a hot-path
    /// lookup, it runs once per node per branch flip.
    pub fn created(&self, node: NodeId) -> bool {
        self.created.contains(&node)
    }

    /// Whether `node` was minted by **this scope, or by any scope that is
    /// transitively a child of this scope's render** (issue #732) — real
    /// ownership, not a numeric proxy for it. Walks `node`'s minting scope's
    /// recorded parent chain ([`SCOPE_PARENTS`]) looking for `self`'s own
    /// id; answers `false` once the chain runs out (reaches a scope with no
    /// parent, which is where an unrelated/outer scope — the thing round 2
    /// misclassified as owned — always ends up) or if `node` was never
    /// minted through any `RenderScope` at all (raw backend access — also
    /// correctly "not owned"). `node` must belong to `self`'s own document —
    /// every real call site asks this only of a descendant inside a subtree
    /// it is already walking, which is always true.
    ///
    /// Cost is one `HashMap` lookup in `self`'s document's minting table
    /// plus one further lookup per level of scope nesting between `node`'s
    /// own scope and `self` — the "× scope depth" the walk in
    /// [`collect_captured_descendants`](super::collect_captured_descendants)
    /// pays per node, which is small and bounded (a document rarely nests
    /// reactive helpers more than a handful of levels deep), never a function
    /// of document size.
    pub(crate) fn owns_transitively(&self, node: NodeId) -> bool {
        let Some(mut current) = self.minted_by.borrow().get(&node).copied() else {
            return false;
        };
        if current == self.id {
            return true;
        }
        SCOPE_PARENTS.with(|p| {
            let p = p.borrow();
            loop {
                match p.get(&current).copied().flatten() {
                    Some(parent) => {
                        if parent == self.id {
                            return true;
                        }
                        current = parent;
                    }
                    None => return false,
                }
            }
        })
    }

    /// Record a node this scope just minted. See [`created`](Self::created)
    /// and, for the cross-scope question, [`owns_transitively`](Self::owns_transitively).
    fn own(&mut self, id: NodeId) -> NodeId {
        self.created.push(id);
        self.minted_by.borrow_mut().insert(id, self.id);
        id
    }

    /// Get the document reference.
    fn doc(&self) -> Option<Rc<RefCell<dyn DomDocument>>> {
        self.doc.upgrade()
    }

    /// Create a new element and return a handle to it.
    #[doc(hidden)]
    pub fn create_element(&mut self, tag: &str) -> NodeHandle {
        let doc = self.doc().expect("Document dropped");
        let node_id = doc.borrow_mut().create_element(tag);
        NodeHandle::new(self.own(node_id), self.doc.clone())
    }

    /// Create a new text node and return a handle to it.
    #[doc(hidden)]
    pub fn create_text(&mut self, text: &str) -> NodeHandle {
        let doc = self.doc().expect("Document dropped");
        let node_id = doc.borrow_mut().create_text(text);
        NodeHandle::new(self.own(node_id), self.doc.clone())
    }

    /// Create a reactive text node wrapped in a span with a tracking ID.
    ///
    /// Returns `(container_handle, reactive_id)` where:
    /// - `container_handle` is the span element that should be appended to the parent
    /// - `reactive_id` is the unique ID for tracking this reactive text node
    #[doc(hidden)]
    pub fn create_reactive_text(&mut self, initial_text: &str) -> (NodeHandle, usize) {
        let reactive_id = next_reactive_id();
        let doc = self.doc().expect("Document dropped");

        // Create a span wrapper with the reactive ID attribute
        let span_id = doc.borrow_mut().create_element("span");
        let span = NodeHandle::new(self.own(span_id), self.doc.clone());
        span.set_attribute("data-rid-reactive", &reactive_id.to_string());
        span.set_attribute("style", "display:contents"); // Invisible wrapper

        // Create the text node inside the span
        let text_id = doc.borrow_mut().create_text(initial_text);
        self.own(text_id);
        doc.borrow_mut().append_child(span_id, text_id);

        tracing::debug!(
            "Created reactive text: id={}, initial='{}', span_id={:?}",
            reactive_id,
            if initial_text.len() > 20 {
                &initial_text[..20]
            } else {
                initial_text
            },
            span_id
        );

        (span, reactive_id)
    }

    /// Create a comment node (useful as a placeholder/marker).
    #[doc(hidden)]
    pub fn create_comment(&mut self, text: &str) -> NodeHandle {
        let doc = self.doc().expect("Document dropped");
        let node_id = doc.borrow_mut().create_comment(text);
        NodeHandle::new(self.own(node_id), self.doc.clone())
    }

    /// Make this scope the ambient owner until the returned guard drops.
    ///
    /// Resources created while the guard is live — signals, memos, effects,
    /// event handlers — are attributed to this scope (issue #141). The render
    /// sites wrap exactly the user render call:
    ///
    /// ```ignore
    /// let node = { let _owner = child_scope.push_owner(); view(&item, &mut child_scope) };
    /// ```
    ///
    /// Issue #732, round 4: this guard no longer carries any scope-ancestry
    /// bookkeeping (round 3's `RENDER_SCOPE_STACK` push/pop is gone). The
    /// ancestry link a reactive helper's content needs is now a **fixed**
    /// id that helper captured once from [`RenderScope::id`] at the moment
    /// it was called, threaded explicitly into [`RenderScope::with_parent`]
    /// at every point it constructs content — including from a later
    /// `Effect` re-run — rather than inferred from whatever is ambient on a
    /// call stack at construction time, which round 3 got wrong for exactly
    /// that later-Effect case (a `for` row insert, a nested branch flip, a
    /// nested component re-render, all happening after the outer branch's
    /// own initial render had already returned).
    ///
    /// Takes `&self` and returns a lifetime-free guard, so the `&mut` borrow of
    /// the same scope on the next line is still legal.
    #[doc(hidden)]
    pub fn push_owner(&self) -> crate::reactive::OwnerGuard {
        self.reactive_scope.push_owner()
    }

    /// What this scope owns. See [`OwnedCounts`](crate::reactive::OwnedCounts).
    #[doc(hidden)]
    pub fn owned_counts(&self) -> crate::reactive::OwnedCounts {
        self.reactive_scope.owned_counts()
    }

    /// A non-owning reference to this scope's reactive scope, for comparison
    /// and diagnostics. See [`Owner`](crate::reactive::Owner).
    #[doc(hidden)]
    pub fn owner(&self) -> crate::reactive::Owner {
        self.reactive_scope.owner()
    }

    /// Create a child scope for nested rendering.
    ///
    /// Child scopes are cleaned up when the parent scope is disposed.
    ///
    /// # Warning
    ///
    /// Resources created through the returned `&mut` are attributed to the
    /// **ambient** owner — normally the parent — not to the child, because the
    /// returned reference outlives any guard this method could hand back. Its
    /// only caller is a test; prefer [`RenderScope::new`] plus
    /// [`push_owner`](RenderScope::push_owner).
    #[doc(hidden)]
    pub fn child_scope(&mut self, parent: &NodeHandle) -> &mut RenderScope {
        let scope = RenderScope::new(self.doc().expect("Document dropped"), parent.node_id);
        self.children.push(scope);
        self.children.last_mut().unwrap()
    }

    /// Create an effect that runs when its dependencies change.
    ///
    /// The effect is stored in this scope's reactive scope. When the scope
    /// is disposed, the effect will be disposed as well, preventing memory
    /// leaks and stale effects trying to update removed DOM nodes.
    pub fn create_effect<F: FnMut() + 'static>(&mut self, f: F) {
        let effect = Effect::new(f);
        // Store effect in this scope - will be disposed when scope is disposed
        self.reactive_scope.add_effect(effect);
    }

    /// Adopt an already-created effect into this scope.
    ///
    /// The effect will be disposed when this scope is disposed.
    pub fn create_effect_from(&mut self, effect: Effect) {
        self.reactive_scope.add_effect(effect);
    }

    /// Create a deferred effect that doesn't run immediately.
    pub fn create_effect_deferred<F: FnMut() + 'static>(&mut self, f: F) {
        let effect = Effect::new_deferred(f);
        self.reactive_scope.add_effect(effect);
    }

    /// Register a cleanup function to run when this scope is disposed.
    ///
    /// Delegates to the reactive [`Scope`], which runs its cleanups from its own
    /// `Drop` as well as from `dispose()`. Keeping a second list on `RenderScope`
    /// meant cleanups were lost on every drop-only teardown, because
    /// `Drop for RenderScope` does nothing and only the by-value `dispose()`
    /// drained that list (issue #141).
    pub fn on_cleanup<F: FnOnce() + 'static>(&mut self, f: F) {
        self.reactive_scope.on_cleanup(f);
    }

    /// Get a handle to the parent node.
    pub fn parent(&self) -> NodeHandle {
        NodeHandle::new(self.parent_id, self.doc.clone())
    }

    /// Get a weak reference to the document for creating NodeHandles.
    pub fn doc_weak(&self) -> Weak<RefCell<dyn DomDocument>> {
        self.doc.clone()
    }

    /// Get a handle to the document root element (html).
    ///
    /// Useful for resetting scroll position on the root element.
    pub fn root_handle(&self) -> NodeHandle {
        let doc = self.doc().expect("Document dropped");
        let root_id = doc.borrow().root();
        NodeHandle::new(root_id, self.doc.clone())
    }

    /// Get a handle to the body element.
    ///
    /// Useful for resetting scroll position on the body element.
    pub fn body_handle(&self) -> NodeHandle {
        let doc = self.doc().expect("Document dropped");
        let body_id = doc.borrow().body();
        NodeHandle::new(body_id, self.doc.clone())
    }

    /// Reset scroll position on root (html) and body elements to zero.
    ///
    /// This is useful for fixed-position overlays (drawers, modals) that need
    /// to appear at the top of the viewport regardless of current scroll state.
    /// Call this when opening such overlays.
    pub fn reset_document_scroll(&self) {
        self.root_handle().set_scroll_top(0.0);
        self.body_handle().set_scroll_top(0.0);
    }

    /// Register a click event handler and return a handler ID.
    ///
    /// The handler will be invoked when the element with the corresponding
    /// `data-rid` attribute is clicked.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let handler_id = scope.register_handler(|| {
    ///     println!("Button clicked!");
    /// });
    /// element.set_attribute("data-rid", &handler_id.to_string());
    /// ```
    #[doc(hidden)]
    pub fn register_handler<F: Fn() + 'static>(
        &mut self,
        callback: F,
    ) -> crate::events::EventHandlerId {
        crate::events::register_handler(std::rc::Rc::new(callback))
    }

    /// Register an input event handler and return a handler ID.
    ///
    /// The handler will be invoked when the element with the corresponding
    /// `data-oninput` attribute receives input, passing the new value.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let handler_id = scope.register_input_handler(|value| {
    ///     println!("Input value: {}", value);
    /// });
    /// element.set_attribute("data-oninput", &handler_id.to_string());
    /// ```
    #[doc(hidden)]
    pub fn register_input_handler<F: Fn(String) + 'static>(
        &mut self,
        callback: F,
    ) -> crate::events::EventHandlerId {
        crate::events::register_input_handler(crate::events::InputCallback::new(callback))
    }

    /// Register a file-drop event handler and return its ID.
    ///
    /// The handler receives the list of file paths dropped from the OS.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let handler_id = scope.register_file_drop_handler(|paths| {
    ///     println!("Dropped files: {:?}", paths);
    /// });
    /// element.set_attribute("data-onfiledrop", &handler_id.to_string());
    /// ```
    #[doc(hidden)]
    pub fn register_file_drop_handler<F: Fn(Vec<std::path::PathBuf>) + 'static>(
        &mut self,
        callback: F,
    ) -> crate::events::EventHandlerId {
        crate::events::register_file_drop_handler(crate::events::FileDropCallback::new(callback))
    }

    /// Register a scroll event handler and return its ID.
    ///
    /// The handler receives a [`ScrollEvent`](crate::events::ScrollEvent)
    /// carrying the container's offset on both axes.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let handler_id = scope.register_scroll_handler(|ev| {
    ///     println!("Scrolled to: {}, {}", ev.scroll_top, ev.scroll_left);
    /// });
    /// element.set_attribute("data-onscroll", &handler_id.to_string());
    /// ```
    #[doc(hidden)]
    pub fn register_scroll_handler<F: Fn(crate::events::ScrollEvent) + 'static>(
        &mut self,
        callback: F,
    ) -> crate::events::EventHandlerId {
        crate::events::register_scroll_handler(crate::events::ScrollCallback::new(callback))
    }

    /// Dispose of this scope and all child scopes.
    ///
    /// Equivalent to dropping it: cleanups and effects both live on
    /// `reactive_scope`, whose own `Drop` disposes it. This exists for callers
    /// that want teardown to happen at a definite point — which matters more
    /// than it looks, because disposal now *frees* the scope's signals, memos
    /// and event handlers rather than merely stopping its effects (issue #141),
    /// so doing it before the surrounding DOM is torn down rather than after is
    /// the difference between cleanups patching live nodes and patching a
    /// corpse.
    pub fn dispose(mut self) {
        self.dispose_in_place();
    }

    /// [`dispose`](RenderScope::dispose) for callers that hold only `&mut` —
    /// notably a `RenderScope` living inside a shared `Rc<RefCell<_>>`, where the
    /// by-value form is unreachable.
    ///
    /// Prefer the by-value form: it proves at the type level that nothing else
    /// can observe the scope while its cleanups run.
    pub fn dispose_in_place(&mut self) {
        // Emptied before any child is disposed, so a child's teardown cannot
        // observe (or re-enter) a half-drained list.
        for child in std::mem::take(&mut self.children) {
            child.dispose();
        }

        // Disposes effects and runs cleanups (see `on_cleanup`).
        self.reactive_scope.dispose();
    }
}

impl Drop for RenderScope {
    fn drop(&mut self) {
        // `children` are `RenderScope`s that drop recursively, and effects +
        // cleanups belong to `reactive_scope`, whose `Drop` calls its own
        // iterative `dispose()`. This is what makes `on_cleanup` fire on
        // drop-only teardown paths (issue #141).
        //
        // This scope's own ancestry entry goes too (issue #732, round 3): once
        // the scope itself is gone, nothing can ask "is `self.id` an ancestor"
        // any more through it, and every node it minted is either still live
        // (removed from `MINTED_BY` only on discard, independently of this) or
        // already discarded (and so already purged) — so dropping this entry
        // here, unconditionally, is what keeps `SCOPE_PARENTS` from growing by
        // one entry per branch flip for the life of the document.
        SCOPE_PARENTS.with(|m| {
            m.borrow_mut().remove(&self.id);
        });
    }
}

/// Batched DOM updates for efficiency.
///
/// Collects multiple DOM mutations and applies them in a single batch,
/// minimizing layout recalculations.
pub struct UpdateBatch {
    updates: Vec<DomUpdate>,
}

/// A single DOM update operation.
#[derive(Debug)]
pub enum DomUpdate {
    SetText {
        node: NodeId,
        text: String,
    },
    SetAttribute {
        node: NodeId,
        name: String,
        value: String,
    },
    RemoveAttribute {
        node: NodeId,
        name: String,
    },
    AppendChild {
        parent: NodeId,
        child: NodeId,
    },
    RemoveChild {
        parent: NodeId,
        child: NodeId,
    },
    InsertBefore {
        parent: NodeId,
        child: NodeId,
        reference: NodeId,
    },
    ReplaceNode {
        old: NodeId,
        new: NodeId,
    },
    SetStyle {
        node: NodeId,
        property: String,
        value: String,
    },
}

impl UpdateBatch {
    /// Create a new empty batch.
    pub fn new() -> Self {
        Self {
            updates: Vec::new(),
        }
    }

    /// Add an update to the batch.
    pub fn push(&mut self, update: DomUpdate) {
        self.updates.push(update);
    }

    /// Apply all updates to a document.
    ///
    /// **The four structural arms bypass the late-child registry** — a node
    /// appended, inserted, removed or replaced through this one tells no
    /// registered container that its children changed, so a `List`, `RadioGroup`
    /// or `Stepper` reached this way keeps the answers it last derived (issues
    /// #716, #745). Nothing in this workspace applies a structural arm, and it
    /// cannot be routed through [`NodeHandle`](super::NodeHandle)'s verbs as
    /// this signature stands: a notification needs the `Rc<RefCell<..>>` a
    /// `NodeHandle` holds a `Weak` of, and this is handed a `&mut dyn` borrowed
    /// for the whole loop. Tracked as **#756**. Reach for `NodeHandle`'s verbs
    /// instead when the nodes are inside a component library container.
    pub fn apply(self, doc: &mut dyn DomDocument) {
        for update in self.updates {
            match update {
                DomUpdate::SetText { node, text } => {
                    doc.set_text_content(node, &text);
                }
                DomUpdate::SetAttribute { node, name, value } => {
                    doc.set_attribute(node, &name, &value);
                }
                DomUpdate::RemoveAttribute { node, name } => {
                    doc.remove_attribute(node, &name);
                }
                DomUpdate::AppendChild { parent, child } => {
                    doc.append_child(parent, child);
                }
                DomUpdate::RemoveChild { parent, child } => {
                    doc.remove_child(parent, child);
                }
                DomUpdate::InsertBefore {
                    parent,
                    child,
                    reference,
                } => {
                    doc.insert_before(parent, child, reference);
                }
                DomUpdate::ReplaceNode { old, new } => {
                    doc.replace_node(old, new);
                }
                DomUpdate::SetStyle {
                    node,
                    property,
                    value,
                } => {
                    doc.set_style(node, &property, &value);
                }
            }
        }
    }

    /// Check if the batch is empty.
    pub fn is_empty(&self) -> bool {
        self.updates.is_empty()
    }

    /// Get the number of updates in the batch.
    pub fn len(&self) -> usize {
        self.updates.len()
    }
}

impl Default for UpdateBatch {
    fn default() -> Self {
        Self::new()
    }
}
