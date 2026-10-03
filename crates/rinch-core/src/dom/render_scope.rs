//! RenderScope, UpdateBatch, and DomUpdate types.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use crate::reactive::{Effect, Scope};

use super::traits::DomDocument;
use super::{NodeHandle, NodeId, next_reactive_id};

// ============================================================================
// Scope ancestry (issue #732, round 3)
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
// This is real ownership: every `RenderScope` gets a globally unique
// [`ScopeId`], records its own **parent** scope id (the one whose render is
// synchronously on the call stack when this scope is constructed — see
// [`RenderScope::push_owner`]), and every node a scope mints is recorded
// against that scope's id. "Is `node` owned by `scope`" is then a literal
// walk up `node`'s minting scope's ancestor chain, looking for `scope` — not
// a numeric proxy for it.

/// A globally unique id for one `RenderScope` instance, for exactly as long
/// as that scope (or any node it minted) might still be asked about.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct ScopeId(u64);

thread_local! {
    /// Monotonic generator for [`ScopeId`] — one counter per thread, since
    /// `RenderScope`s (like `NodeId`s) are never compared across threads.
    static NEXT_SCOPE_ID: Cell<u64> = const { Cell::new(0) };

    /// The ambient "currently rendering" stack: the id of every `RenderScope`
    /// whose render call (`render_fn`/`view`, wrapped in
    /// [`RenderScope::push_owner`]) is synchronously on the call stack right
    /// now, innermost last. A scope constructed while this is non-empty
    /// records its top as its parent — exactly the nesting a `for` row, a
    /// re-rendered component's output, or a nested `if`/`match` branch is
    /// built through, and exactly what a node minted by *unrelated* code
    /// (nothing pushed here) does not get.
    static RENDER_SCOPE_STACK: RefCell<Vec<ScopeId>> = const { RefCell::new(Vec::new()) };

    /// Every live scope's parent, `None` for one built with nothing ambient
    /// (a document's root scope, or any scope built outside a render call).
    /// Removed when the scope itself is dropped — see `RenderScope`'s `Drop`
    /// impl — which is safe once nothing can walk through it any more: a
    /// walk only ever needs an *ancestor's* parent, read while collecting
    /// before the branch's own scope is disposed (every caller already must,
    /// since the watermark this replaced was read the same way), and a
    /// descendant scope (a row, a nested branch) outlives or is disposed no
    /// later than its own content, independently of this map.
    static SCOPE_PARENTS: RefCell<HashMap<ScopeId, Option<ScopeId>>> = RefCell::new(HashMap::new());

    /// Which scope minted each live node, keyed by `(doc_key, NodeId)` since
    /// `NodeId` is a per-document slab index and collides across documents on
    /// one thread (issue #134). Removed when the node is discarded —
    /// [`NodeHandle::discard`] purges its whole subtree before handing it to
    /// the backend — so this holds exactly the nodes some live `RenderScope`
    /// minted and nothing has yet discarded, not every node ever created.
    static MINTED_BY: RefCell<HashMap<(u64, NodeId), ScopeId>> = RefCell::new(HashMap::new());
}

fn next_scope_id() -> ScopeId {
    NEXT_SCOPE_ID.with(|c| {
        let id = c.get();
        c.set(id + 1);
        ScopeId(id)
    })
}

/// Remove every `(doc_key, id)` under `root`'s subtree from [`MINTED_BY`]
/// (issue #732, round 3) — called from [`NodeHandle::discard`] before the
/// backend retires the subtree, while `get_children` still answers.
///
/// Bounded by the discarded subtree, the same shape as
/// [`collect_captured_descendants`](super::collect_captured_descendants)'s
/// walk and no more expensive than the recursive retire the backend itself
/// performs: one extra `get_children` pass over exactly what is being thrown
/// away, never over the whole document. This is what keeps `MINTED_BY` from
/// growing without bound — every node any `RenderScope` ever minted is
/// recorded there, so without this every discard anywhere in the app (not
/// only the four reactive helpers) would leak one entry per node, forever.
pub(crate) fn purge_minted_by_subtree(root: &NodeHandle) {
    let doc_key = root.doc_key();
    fn walk(node: &NodeHandle, doc_key: u64) {
        MINTED_BY.with(|m| {
            m.borrow_mut().remove(&(doc_key, node.node_id()));
        });
        for child in node.children() {
            walk(&child, doc_key);
        }
    }
    walk(root, doc_key);
}

/// **Test-only.** How many nodes [`MINTED_BY`] currently holds a minting
/// scope for — every node some live `RenderScope` has minted and no discard
/// has yet purged (issue #732, round 3). What a growth fixture compares
/// across many toggles to prove this table does not leak.
#[doc(hidden)]
pub fn __minted_by_len() -> usize {
    MINTED_BY.with(|m| m.borrow().len())
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
    /// This scope's own identity in the ancestry tracking above (issue #732,
    /// round 3). Assigned once, at construction, and never reused.
    id: ScopeId,
}

/// Returned by [`RenderScope::push_owner`]. Pops this scope's id off
/// [`RENDER_SCOPE_STACK`] on drop, alongside disposing the reactive
/// ownership guard it wraps.
#[doc(hidden)]
pub struct RenderOwnerGuard {
    _reactive: crate::reactive::OwnerGuard,
    scope_id: ScopeId,
}

impl Drop for RenderOwnerGuard {
    fn drop(&mut self) {
        RENDER_SCOPE_STACK.with(|s| {
            let mut s = s.borrow_mut();
            debug_assert_eq!(
                s.last().copied(),
                Some(self.scope_id),
                "RenderOwnerGuard dropped out of order — the render-scope \
                 ambient stack must be LIFO, like the reactive owner stack it \
                 mirrors"
            );
            s.pop();
        });
    }
}

impl RenderScope {
    /// Create a new render scope rooted at the given node.
    ///
    /// Records this scope's **parent** as whatever `RenderScope`'s render is
    /// synchronously on the call stack right now (the top of
    /// [`RENDER_SCOPE_STACK`]) — `None` if nothing is, which is correct for a
    /// document's root scope and for any scope built outside a render call
    /// entirely. See the module-level doc above [`ScopeId`].
    pub fn new(doc: Rc<RefCell<dyn DomDocument>>, parent_id: NodeId) -> Self {
        let id = next_scope_id();
        let parent = RENDER_SCOPE_STACK.with(|s| s.borrow().last().copied());
        SCOPE_PARENTS.with(|m| {
            m.borrow_mut().insert(id, parent);
        });
        Self {
            doc: Rc::downgrade(&doc),
            parent_id,
            created: Vec::new(),
            effects: Vec::new(),
            children: Vec::new(),
            reactive_scope: Scope::new(),
            id,
        }
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
    /// transitively a child of this scope's render** (issue #732, round 3) —
    /// real ownership, not a numeric proxy for it. Walks `node`'s minting
    /// scope's recorded parent chain ([`SCOPE_PARENTS`]) looking for `self`'s
    /// own id; answers `false` once the chain runs out (reaches a scope with
    /// no parent, which is where an unrelated/outer scope — the thing round 2
    /// misclassified as owned — always ends up) or if `node` was never
    /// minted through any `RenderScope` at all (raw backend access — also
    /// correctly "not owned").
    ///
    /// Cost is one `HashMap` lookup in [`MINTED_BY`] plus one further lookup
    /// per level of scope nesting between `node`'s own scope and `self` — the
    /// "× scope depth" the walk in
    /// [`collect_captured_descendants`](super::collect_captured_descendants)
    /// pays per node, which is small and bounded (a document rarely nests
    /// reactive helpers more than a handful of levels deep), never a function
    /// of document size.
    pub(crate) fn owns_transitively(&self, doc_key: u64, node: NodeId) -> bool {
        let Some(mut current) = MINTED_BY.with(|m| m.borrow().get(&(doc_key, node)).copied())
        else {
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
        if let Some(doc) = self.doc() {
            let doc_key = doc.borrow().doc_key();
            MINTED_BY.with(|m| {
                m.borrow_mut().insert((doc_key, id), self.id);
            });
        }
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
    /// **Also** pushes this scope's id onto [`RENDER_SCOPE_STACK`] for the
    /// guard's lifetime (issue #732, round 3): this is the *only* place that
    /// stack is pushed, and it is exactly the window in which a
    /// `RenderScope::new(..)` a nested helper (a `for` row, a re-rendered
    /// component) constructs should record this scope as its parent. A
    /// scope built outside this window — before the branch's render even
    /// starts, or by code this render merely happens to run alongside rather
    /// than call into — gets no parent here, which is the fix for round 2's
    /// hole.
    ///
    /// Takes `&self` and returns a lifetime-free guard, so the `&mut` borrow of
    /// the same scope on the next line is still legal.
    #[doc(hidden)]
    pub fn push_owner(&self) -> RenderOwnerGuard {
        RENDER_SCOPE_STACK.with(|s| s.borrow_mut().push(self.id));
        RenderOwnerGuard {
            _reactive: self.reactive_scope.push_owner(),
            scope_id: self.id,
        }
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
