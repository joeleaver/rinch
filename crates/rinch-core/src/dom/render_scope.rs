//! RenderScope, UpdateBatch, and DomUpdate types.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use crate::reactive::{Effect, Scope};

use super::traits::DomDocument;
use super::{NodeHandle, NodeId, next_reactive_id};

// ============================================================================
// Node ownership across scopes (issue #732)
// ============================================================================
//
// A branch helper hiding content it built discards it, recursively. A
// *captured* `NodeHandle` nested inside that markup must come out first, so the
// question asked of every descendant is "did this branch's render mint it" —
// directly, or through a scope its render created (a `for` row, a nested
// branch, a component re-render, a late patch). That is scope ancestry:
//
// - every `RenderScope` has a [`ScopeId`] and an optional parent `ScopeId`,
//   fixed at construction ([`RenderScope::with_parent`]);
// - every node a scope mints is recorded against that scope's id;
// - a node is owned by scope `S` when the chain from its minting scope up
//   through the recorded parents reaches `S`.
//
// The tables are **per document** (a `NodeId` is only unique within one,
// issue #134) and each entry outlives the scope it describes for as long as
// something still needs it: a scope's entry is kept while the scope is alive,
// while any node it minted is still recorded, and while any child scope's entry
// names it as parent. So a node minted by a throwaway scope (a late patch, a
// pooled spacer) still chains to that scope's parent after the scope is gone,
// and nothing is kept once nothing can be asked about. A node leaves the table
// when it is discarded or its parent's `set_inner_html` replaces it; the whole
// table goes when the last `RenderScope` of its document is dropped.

/// The identity of one [`RenderScope`] for ownership purposes (issue #732).
///
/// Read it with [`RenderScope::id`] and hand it to
/// [`RenderScope::with_parent`] when a scope you build mints nodes on behalf of
/// another one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ScopeId(u64);

/// A hasher for the dense integer keys of the ancestry tables. `NodeId` and
/// `ScopeId` are small integers the program mints, not attacker input, and the
/// table is hit on every node a scope mints.
#[derive(Default, Clone, Copy)]
struct IdHasher(u64);

impl std::hash::Hasher for IdHasher {
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(u64::from(b));
        }
    }
    fn write_u64(&mut self, n: u64) {
        self.0 = (self.0.rotate_left(5) ^ n).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    fn write_usize(&mut self, n: usize) {
        self.write_u64(n as u64);
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

type IdMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<IdHasher>>;

/// One scope's entry in its document's [`Ancestry`].
struct ScopeEntry {
    parent: Option<ScopeId>,
    /// Whether the `RenderScope` itself still exists.
    alive: bool,
    /// Recorded nodes it minted plus child entries naming it. While the scope
    /// is alive its own mints are counted on the scope (`RenderScope::minted`)
    /// and settled here when it drops, so this can dip below zero until then.
    refs: i64,
}

/// One document's ancestry tables (issue #732).
pub(crate) struct Ancestry {
    doc_key: u64,
    minted_by: IdMap<NodeId, ScopeId>,
    scopes: IdMap<ScopeId, ScopeEntry>,
}

type AncestryRef = Rc<RefCell<Ancestry>>;

thread_local! {
    static NEXT_SCOPE_ID: Cell<u64> = const { Cell::new(0) };

    /// Each document's tables, held weakly: the strong references live on the
    /// document's `RenderScope`s, and the slot is removed when the tables drop.
    static DOC_ANCESTRY: RefCell<HashMap<u64, Weak<RefCell<Ancestry>>>> =
        RefCell::new(HashMap::new());
}

impl Drop for Ancestry {
    fn drop(&mut self) {
        let key = self.doc_key;
        let _ = DOC_ANCESTRY.try_with(|slots| {
            if let Ok(mut slots) = slots.try_borrow_mut()
                && slots.get(&key).is_some_and(|w| w.strong_count() == 0)
            {
                slots.remove(&key);
            }
        });
    }
}

impl Ancestry {
    /// Drop `n` references to `scope`, removing its entry — and in turn
    /// releasing its parent — once it is dead and unreferenced.
    fn release(&mut self, scope: ScopeId, n: i64) {
        let mut next = Some((scope, n));
        while let Some((id, n)) = next.take() {
            let Some(entry) = self.scopes.get_mut(&id) else {
                return;
            };
            entry.refs -= n;
            if !entry.alive && entry.refs <= 0 {
                let parent = entry.parent;
                self.scopes.remove(&id);
                next = parent.map(|p| (p, 1));
            }
        }
    }

    /// Forget `node`'s minting record, if it has one.
    fn purge(&mut self, node: NodeId) {
        if let Some(scope) = self.minted_by.remove(&node) {
            self.release(scope, 1);
        }
    }

    /// Whether `node` was minted by `owner` or by a scope descended from it.
    fn owns(&self, owner: ScopeId, node: NodeId) -> bool {
        let mut current = self.minted_by.get(&node).copied();
        while let Some(scope) = current {
            if scope == owner {
                return true;
            }
            current = self.scopes.get(&scope).and_then(|e| e.parent);
        }
        false
    }
}

fn ancestry_for(doc_key: u64) -> Option<AncestryRef> {
    DOC_ANCESTRY.with(|slots| slots.borrow().get(&doc_key).and_then(Weak::upgrade))
}

fn ancestry_for_or_new(doc_key: u64) -> AncestryRef {
    DOC_ANCESTRY.with(|slots| {
        let mut slots = slots.borrow_mut();
        if let Some(existing) = slots.get(&doc_key).and_then(Weak::upgrade) {
            return existing;
        }
        let table = Rc::new(RefCell::new(Ancestry {
            doc_key,
            minted_by: IdMap::default(),
            scopes: IdMap::default(),
        }));
        slots.insert(doc_key, Rc::downgrade(&table));
        table
    })
}

/// Prepare `root`'s subtree for a discard, in one walk (issue #732).
///
/// Every node visited loses its minting record (the discard is about to
/// retire it). With `owner = Some(s)`, a descendant `s` does not own — one the
/// render was handed rather than built — is pushed to `captured` instead: its
/// record is kept and its subtree not entered, and the caller detaches it
/// before discarding `root`, so it can be shown again. With `None`, everything
/// under `root` is purged and nothing is captured.
///
/// Bounded by the subtree being discarded, minus what is captured.
pub(crate) fn sweep_for_discard(
    root: &NodeHandle,
    owner: Option<ScopeId>,
    captured: &mut Vec<NodeHandle>,
) {
    let Some(table) = ancestry_for(root.doc_key()) else {
        return;
    };
    let Some(doc) = root.accessed_doc() else {
        return;
    };
    let doc = doc.borrow();
    let mut table = table.borrow_mut();
    let mut stack = vec![root.node_id()];
    let mut is_root = true;
    while let Some(id) = stack.pop() {
        if !is_root
            && let Some(owner) = owner
            && !table.owns(owner, id)
        {
            captured.push(NodeHandle::new(id, root.doc.clone()));
            continue;
        }
        is_root = false;
        table.purge(id);
        stack.extend(doc.get_children(id));
    }
}

/// Forget the minting records of everything under `root`, not `root` itself —
/// what `set_inner_html` replaces.
pub(crate) fn purge_descendants(root: &NodeHandle) {
    let Some(table) = ancestry_for(root.doc_key()) else {
        return;
    };
    let Some(doc) = root.accessed_doc() else {
        return;
    };
    let doc = doc.borrow();
    let mut table = table.borrow_mut();
    let mut stack = doc.get_children(root.node_id());
    while let Some(id) = stack.pop() {
        table.purge(id);
        stack.extend(doc.get_children(id));
    }
}

/// **Test-only.** How many nodes the ancestry tables of every document with a
/// live `RenderScope` on this thread currently record a minting scope for.
#[doc(hidden)]
pub fn __minted_by_len() -> usize {
    DOC_ANCESTRY.with(|slots| {
        slots
            .borrow()
            .values()
            .filter_map(Weak::upgrade)
            .map(|t| t.borrow().minted_by.len())
            .sum()
    })
}

/// **Test-only.** How many scope entries those tables hold — live scopes, and
/// dropped ones something still refers to.
#[doc(hidden)]
pub fn __scope_parents_len() -> usize {
    DOC_ANCESTRY.with(|slots| {
        slots
            .borrow()
            .values()
            .filter_map(Weak::upgrade)
            .map(|t| t.borrow().scopes.len())
            .sum()
    })
}

/// **Test-only.** How many per-document slots the ancestry tables hold, live or
/// dead (issue #732): a dropped document must leave none behind.
#[doc(hidden)]
pub fn __doc_table_slots() -> usize {
    DOC_ANCESTRY.with(|slots| slots.borrow().len())
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
    /// This scope's identity in its document's ancestry tables (issue #732).
    id: ScopeId,
    /// Nodes this scope has recorded in the tables and not yet settled onto
    /// its entry — settled when the scope drops (see `ScopeEntry::refs`).
    minted: i64,
    /// The document's ancestry tables. Every scope of a document holds them,
    /// so they live exactly as long as some `RenderScope` of that document.
    ancestry: AncestryRef,
}

impl RenderScope {
    /// Create a render scope with **no ancestry parent** (issue #732).
    ///
    /// Right for a document's root scope, a mount, or a test. **Not** right
    /// for a scope that builds nodes on behalf of another scope's render —
    /// a list row, a branch's content, a patch a container applies to a late
    /// child: use [`with_parent`](Self::with_parent) there, naming the scope
    /// whose content this is. A branch that hides discards everything its
    /// render built, and a node minted by a parentless scope is not something
    /// its render built — it is only detached, so inside a branch it leaks on
    /// every hide.
    ///
    /// Deliberately not defaulted from whichever scope happens to be rendering:
    /// that answer is wrong for content a helper builds later from an effect
    /// (nothing of the outer render is on the stack then), and it would claim a
    /// cache that builds its own content during a render as the render's — and
    /// discard it from under the cache.
    pub fn new(doc: Rc<RefCell<dyn DomDocument>>, parent_id: NodeId) -> Self {
        Self::with_parent(doc, parent_id, None)
    }

    /// Create a render scope whose nodes belong to `parent`'s render
    /// (issue #732). `None` is [`new`](Self::new).
    ///
    /// Capture the parent's [`id`](Self::id) when you are handed its scope —
    /// a component's `render`, a helper's call — and pass that id to every
    /// scope you build for it, including from an effect or observer that runs
    /// much later: what matters is whose content this is, not what is on the
    /// stack when it is built. The scope may be dropped as soon as its nodes
    /// exist; they keep belonging to `parent`.
    pub fn with_parent(
        doc: Rc<RefCell<dyn DomDocument>>,
        parent_id: NodeId,
        parent: Option<ScopeId>,
    ) -> Self {
        let id = NEXT_SCOPE_ID.with(|c| {
            let id = c.get();
            c.set(id + 1);
            ScopeId(id)
        });
        let ancestry = ancestry_for_or_new(doc.borrow().doc_key());
        {
            let mut table = ancestry.borrow_mut();
            // A parent with no entry is gone and nothing refers to it, so
            // nothing alive can be asking whether it owns anything.
            let parent = parent.filter(|p| match table.scopes.get_mut(p) {
                Some(entry) => {
                    entry.refs += 1;
                    true
                }
                None => false,
            });
            table.scopes.insert(
                id,
                ScopeEntry {
                    parent,
                    alive: true,
                    refs: 0,
                },
            );
        }
        Self {
            doc: Rc::downgrade(&doc),
            parent_id,
            created: Vec::new(),
            effects: Vec::new(),
            children: Vec::new(),
            reactive_scope: Scope::new(),
            id,
            minted: 0,
            ancestry,
        }
    }

    /// This scope's [`ScopeId`] — the `parent` to pass to
    /// [`with_parent`](Self::with_parent) for a scope that builds content on
    /// this one's behalf.
    pub fn id(&self) -> ScopeId {
        self.id
    }

    /// Whether this scope minted `node` (issue #719).
    ///
    /// The question a reactive branch helper asks of its content root on a
    /// hide: **did my branch closure build this, or was it handed to me?** A
    /// root this scope created can never be shown again once the branch flips,
    /// so it is released ([`NodeHandle::discard`]); one it did not create is
    /// the caller's, may come back on the next show, and is only detached
    /// ([`NodeHandle::remove`]).
    ///
    /// Inside a root it built, the helper asks a wider question of every
    /// descendant — minted by this scope *or a scope descended from it* (a
    /// `for` row, a nested branch, a late patch; issue #732) — and detaches
    /// whatever is not, so a captured handle nested in branch-built markup
    /// (`if open { div { {panel} } }`) survives the discard. That walk is
    /// `sweep_for_discard`, run before the scope is disposed.
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

    /// Record a node this scope just minted. See [`created`](Self::created).
    fn own(&mut self, id: NodeId) -> NodeId {
        self.created.push(id);
        let previous = self.ancestry.borrow_mut().minted_by.insert(id, self.id);
        match previous {
            // Already ours (an id reused without a purge): counted once.
            Some(scope) if scope == self.id => {}
            Some(scope) => {
                self.minted += 1;
                self.ancestry.borrow_mut().release(scope, 1);
            }
            None => self.minted += 1,
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
    /// only caller is a test; prefer [`RenderScope::with_parent`] plus
    /// [`push_owner`](RenderScope::push_owner).
    #[doc(hidden)]
    pub fn child_scope(&mut self, parent: &NodeHandle) -> &mut RenderScope {
        let scope = RenderScope::with_parent(
            self.doc().expect("Document dropped"),
            parent.node_id,
            Some(self.id),
        );
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
        // The scope's ancestry entry outlives it while its nodes are still
        // recorded or a child entry names it (issue #732): settle its mints
        // onto the entry and let the count decide.
        let Ok(mut table) = self.ancestry.try_borrow_mut() else {
            debug_assert!(
                false,
                "a RenderScope dropped while its ancestry table was borrowed"
            );
            return;
        };
        if let Some(entry) = table.scopes.get_mut(&self.id) {
            entry.alive = false;
            entry.refs += self.minted;
        }
        table.release(self.id, 0);
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
