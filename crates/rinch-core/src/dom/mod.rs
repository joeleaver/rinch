//! DOM abstractions for fine-grained reactive rendering.
//!
//! This module provides the core primitives for surgical DOM updates:
//!
//! - [`NodeHandle`] - A stable reference to a DOM node for targeted updates
//! - [`RenderScope`] - Context for building DOM trees with automatic effect tracking
//! - [`DomDocument`] - Trait abstracting DOM mutation operations
//!
//! # Architecture
//!
//! Fine-grained rendering works by:
//! 1. Components render once, creating DOM nodes directly
//! 2. Reactive expressions become Effects that update specific nodes
//! 3. Signal changes trigger only the affected Effects, not full re-renders
//!
//! ```text
//! Signal.set() → Effect runs → NodeHandle.set_text() → Minimal re-layout
//! ```
//!
//! # Thread-Local Context
//!
//! The render scope is managed via thread-local storage, similar to hooks.
//! Components access the current scope via [`with_render_scope`] or [`try_with_render_scope`].
//!
//! ```ignore
//! // Runtime sets up the scope
//! set_render_scope(scope);
//!
//! // Components access it
//! with_render_scope(|scope| {
//!     let div = scope.create_element("div");
//!     // ...
//! });
//!
//! // Runtime clears it
//! clear_render_scope();
//! ```
//!
//! # Example
//!
//! ```ignore
//! // Using #[component] and rsx! is the idiomatic approach:
//! #[component]
//! fn counter() -> NodeHandle {
//!     let count = Signal::new(0);
//!     rsx! {
//!         div { "Count: " {|| count.get().to_string()} }
//!     }
//! }
//!
//! // The lower-level RenderScope API (used internally by rsx!):
//! fn counter_manual(__scope: &mut RenderScope) -> NodeHandle {
//!     let count = Signal::new(0);
//!
//!     // Create static structure once
//!     let div = __scope.create_element("div");
//!     let text = __scope.create_text("Count: ");
//!     let value = __scope.create_text("0");
//!
//!     // Set up reactive binding - only updates this text node
//!     let value_handle = value.clone();
//!     __scope.create_effect(move || {
//!         value_handle.set_text(&count.get().to_string());
//!     });
//!
//!     div.append_child(&text);
//!     div.append_child(&value);
//!     div
//! }
//! ```

/// The HTML boolean-attribute set and the truthiness rule for it (issue #551).
mod bool_attr;

/// Declaration-level arithmetic on an inline `style` attribute (issue #647).
mod inline_style;
mod late_child;

/// A headless [`DomDocument`](traits::DomDocument) implementation for tests.
/// Available to downstream test code via the `test-util` feature.
#[cfg(any(test, feature = "test-util"))]
pub mod mock;
mod render_scope;
pub mod traits;

pub use bool_attr::{
    attr_is_truthy, data_attr_is_on, is_boolean_attribute, is_presence_reflected_attribute,
};
pub use inline_style::{
    StyleProp, normalize_property_name, serialize_declarations, split_declarations,
};
pub use late_child::{on_child_inserted, on_child_removed};
pub use render_scope::*;
pub use traits::*;

use std::cell::RefCell;
use std::rc::{Rc, Weak};

// ============================================================================
// Thread-Local Render Scope Context
// ============================================================================

thread_local! {
    /// The current render scope, set by the runtime during rendering.
    static RENDER_SCOPE: RefCell<Option<Rc<RefCell<RenderScope>>>> = const { RefCell::new(None) };
    /// Counter for generating unique reactive IDs across all RenderScopes.
    static NEXT_REACTIVE_ID: RefCell<usize> = const { RefCell::new(1) };
}

/// Get the next unique reactive ID.
fn next_reactive_id() -> usize {
    NEXT_REACTIVE_ID.with(|id| {
        let current = *id.borrow();
        *id.borrow_mut() = current + 1;
        current
    })
}

/// Reset the reactive ID counter (for testing or app restart).
pub fn reset_reactive_id_counter() {
    NEXT_REACTIVE_ID.with(|id| {
        *id.borrow_mut() = 1;
    });
}

/// Set the current render scope for thread-local access.
///
/// This should be called by the runtime before rendering components.
/// The scope is wrapped in `Rc<RefCell<_>>` to allow interior mutability.
pub fn set_render_scope(scope: Rc<RefCell<RenderScope>>) {
    RENDER_SCOPE.with(|s| {
        *s.borrow_mut() = Some(scope);
    });
}

/// Clear the current render scope.
///
/// This should be called by the runtime after rendering is complete.
pub fn clear_render_scope() {
    RENDER_SCOPE.with(|s| {
        *s.borrow_mut() = None;
    });
}

/// Execute a closure with mutable access to the current render scope.
///
/// # Panics
///
/// Panics if called outside of a render context (when no scope is set).
///
/// # Example
///
/// ```ignore
/// with_render_scope(|scope| {
///     let div = scope.create_element("div");
///     div.set_attribute("class", "container");
///     div
/// })
/// ```
pub fn with_render_scope<F, R>(f: F) -> R
where
    F: FnOnce(&mut RenderScope) -> R,
{
    RENDER_SCOPE.with(|s| {
        let scope_opt = s.borrow();
        let scope_rc = scope_opt.as_ref().expect(
            "\n\n\x1b[1;31mrinch render error: No render scope available!\x1b[0m\n\
            DOM operations can only be performed during component rendering.\n\
            Make sure you're not calling render functions in:\n\
            - Event handlers\n\
            - Async callbacks\n\
            - Static initializers\n",
        );
        let mut scope = scope_rc.borrow_mut();
        f(&mut scope)
    })
}

/// Try to execute a closure with the current render scope.
///
/// Returns `None` if no render scope is currently set.
/// This is useful for optional integrations that shouldn't panic.
pub fn try_with_render_scope<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut RenderScope) -> R,
{
    RENDER_SCOPE.with(|s| {
        let scope_opt = s.borrow();
        scope_opt.as_ref().map(|scope_rc| {
            let mut scope = scope_rc.borrow_mut();
            f(&mut scope)
        })
    })
}

/// Check if a render scope is currently available.
pub fn has_render_scope() -> bool {
    RENDER_SCOPE.with(|s| s.borrow().is_some())
}

// ============================================================================
// NodeId and NodeHandle
// ============================================================================

/// Unique identifier for a DOM node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(pub usize);

/// A stable handle to a DOM node for surgical updates.
///
/// NodeHandles are lightweight (Rc-based) and can be cloned freely.
/// They reference nodes through weak pointers, allowing the DOM to
/// be cleaned up when no longer needed.
///
/// # Operations
///
/// - Text content: [`set_text`](NodeHandle::set_text)
/// - Attributes: [`set_attribute`](NodeHandle::set_attribute), [`remove_attribute`](NodeHandle::remove_attribute)
/// - Tree structure: [`append_child`](NodeHandle::append_child), [`remove_child`](NodeHandle::remove_child), [`remove`](NodeHandle::remove)
/// - Queries: [`node_id`](NodeHandle::node_id), [`is_valid`](NodeHandle::is_valid)
#[derive(Clone)]
pub struct NodeHandle {
    node_id: NodeId,
    doc: Weak<RefCell<dyn DomDocument>>,
}

impl NodeHandle {
    /// Create a new NodeHandle.
    pub fn new(node_id: NodeId, doc: Weak<RefCell<dyn DomDocument>>) -> Self {
        Self { node_id, doc }
    }

    /// The document, for an operation on it — after running any effects the
    /// current batch has queued.
    ///
    /// **Program order across a batch.** An event handler runs inside a
    /// [`batch`](crate::reactive::batch), which defers effects to the end of
    /// the handler. A handler that writes a signal and then touches the DOM
    /// itself — `open.set(true); field.focus()`, `rows.update(..);
    /// list.scroll_to_bottom()`, `cls.set(..); node.get_attribute("class")` —
    /// wrote that code assuming the write had already reached the DOM, as it
    /// always had. So every `NodeHandle` operation first runs the effects the
    /// batch has queued so far ([`flush_pending_effects`]), the way a browser
    /// flushes pending style before answering a layout query. A handler that
    /// only writes signals still flushes once, at the end; one that interleaves
    /// writes and DOM calls flushes at each DOM call that has something
    /// pending, which is exactly the order it would have seen unbatched.
    ///
    /// Outside a batch, and inside an effect or memo computation (where the
    /// queue is the flush's own business), this is a no-op.
    ///
    /// [`flush_pending_effects`]: crate::reactive::flush_pending_effects
    fn accessed_doc(&self) -> Option<Rc<RefCell<dyn DomDocument>>> {
        crate::reactive::flush_pending_effects();
        self.doc.upgrade()
    }

    /// Get the underlying node ID.
    pub fn node_id(&self) -> NodeId {
        self.node_id
    }

    /// Check if this handle still points to a valid node.
    pub fn is_valid(&self) -> bool {
        self.doc.upgrade().is_some()
    }

    /// This node's document identity (see [`DomDocument::doc_key`]), or `0` if
    /// the document is already gone.
    ///
    /// Node ids are per-document slab indices, so anything that keys a registry
    /// by node id has to pair it with this or two documents on one thread will
    /// collide (issue #134). Two such registries live outside this crate — the
    /// mounted-editor registry and the focus-target registry (issue #147) — and
    /// both are handed a `NodeHandle`, which is why this is public.
    pub fn doc_key(&self) -> u64 {
        self.doc
            .upgrade()
            .map(|doc| doc.borrow().doc_key())
            .unwrap_or(0)
    }

    /// Set the text content of this node.
    ///
    /// For text nodes, this updates the text directly.
    /// For element nodes, this replaces all children with a single text node.
    #[doc(hidden)]
    pub fn set_text(&self, text: &str) {
        if let Some(doc) = self.accessed_doc() {
            tracing::debug!(
                "NodeHandle::set_text(node={}, text_len={})",
                self.node_id.0,
                text.len()
            );
            doc.borrow_mut().set_text_content(self.node_id, text);
        } else {
            tracing::warn!(
                "NodeHandle::set_text FAILED - doc Weak reference is dead (node={})",
                self.node_id.0
            );
        }
    }

    /// Set an attribute on this element.
    ///
    /// # Panics
    /// May panic if called on a non-element node.
    #[doc(hidden)]
    pub fn set_attribute(&self, name: &str, value: &str) {
        if let Some(doc) = self.accessed_doc() {
            doc.borrow_mut().set_attribute(self.node_id, name, value);
        }
    }

    /// Write an attribute the way markup means it — the entry point `rsx!`
    /// generates, for both a literal and a reactive binding.
    ///
    /// For an **HTML boolean attribute** ([`is_boolean_attribute`]) the value is
    /// a *shape*, not a string: a truthy value writes the bare presence form and
    /// a falsey one removes the attribute. Writing the string `"false"` there
    /// would leave the attribute *present*, which HTML reads as true — a
    /// reactive `disabled`/`checked`/`readonly` that could only ever turn on
    /// (issue #551). Every other attribute is written verbatim.
    ///
    /// [`Self::set_attribute`] stays the literal primitive: it writes exactly
    /// what it is given, and the fixtures that probe the `"false"` escape
    /// (`node_is_disabled`, `data-nofocus`) depend on that. That holds on **both**
    /// backends since issue #622 — the web backend used to presence-map `checked`
    /// and `selected` there, so one call meant two things. Reach for this one
    /// from anything that renders a *value* into an attribute; reach for
    /// `set_attribute` when you have already decided the markup, and for
    /// [`Self::remove_attribute`] when you mean off.
    pub fn write_attribute(&self, name: &str, value: &str) {
        if !is_boolean_attribute(name) {
            self.set_attribute(name, value);
            return;
        }
        if attr_is_truthy(value) {
            // The presence form, not the incoming string: `checked="true"` and
            // `checked=""` must be one state, or `[checked]`-style selectors and
            // a browser's attribute/property mirroring disagree with each other.
            self.set_attribute(name, "");
        } else if is_presence_reflected_attribute(name) || self.get_attribute(name).is_some() {
            // Guarded so the overwhelmingly common case — a static `false`, or an
            // effect re-firing while already off — costs no style invalidation.
            //
            // The guard asks "is the attribute already absent" as a proxy for
            // "is the control already off", and for `checked` / `selected` that
            // proxy is false on the web: a user toggle sets the browser's dirty
            // checkedness flag and moves the live property alone, so the
            // attribute reads absent while the box is checked, the write is
            // skipped, and the binding never recovers (issue #687). Those two
            // go to the backend unconditionally, which is the only layer that
            // can see the property — `WebDocument::remove_attribute` mirrors the
            // removal onto it. Desktop pays nothing for the extra call:
            // `RinchDocument::remove_attribute` returns early for an attribute
            // the node does not carry.
            self.remove_attribute(name);
        }
    }

    /// Remove an attribute from this element.
    pub fn remove_attribute(&self, name: &str) {
        if let Some(doc) = self.accessed_doc() {
            doc.borrow_mut().remove_attribute(self.node_id, name);
        }
    }

    /// Get an attribute value from this element.
    pub fn get_attribute(&self, name: &str) -> Option<String> {
        let doc = self.accessed_doc()?;
        doc.borrow().get_attribute(self.node_id, name)
    }

    /// The text this form control holds **right now** (issue #238) — the live
    /// text the user sees and edits, which on the web is the element's `.value`
    /// property and not the `value` attribute
    /// [`get_attribute`](Self::get_attribute) reads.
    ///
    /// Use it, not `get_attribute("value")`, whenever the question is "what
    /// does the field say?": the two agree on desktop and part ways in a
    /// browser as soon as the user types. See [`DomDocument::live_value`] for
    /// each backend's answer and what `None` means.
    pub fn live_value(&self) -> Option<String> {
        let doc = self.accessed_doc()?;
        doc.borrow().live_value(self.node_id)
    }

    /// Append a child node to this element.
    #[doc(hidden)]
    pub fn append_child(&self, child: &NodeHandle) {
        // A child that already has a parent is *moved*, and the parent it came
        // from has lost a child (issue #745). Read before the document changes,
        // and `None` unless something on the thread is watching for removals.
        let vacated = late_child::vacated_parent(child);
        if let Some(doc) = self.accessed_doc() {
            doc.borrow_mut().append_child(self.node_id, child.node_id);
        }
        late_child::notify_vacated(vacated.as_ref(), self);
        late_child::notify_inserted(self, child);
    }

    /// Remove a child node from this element.
    pub fn remove_child(&self, child: &NodeHandle) {
        if let Some(doc) = self.accessed_doc() {
            doc.borrow_mut().remove_child(self.node_id, child.node_id);
        }
        // This node *is* the parent that lost a child, so there is nothing to
        // capture beforehand (issue #745).
        late_child::notify_removed(self);
    }

    /// Insert a child before a reference node.
    pub fn insert_before(&self, child: &NodeHandle, reference: &NodeHandle) {
        let vacated = late_child::vacated_parent(child);
        if let Some(doc) = self.accessed_doc() {
            doc.borrow_mut()
                .insert_before(self.node_id, child.node_id, reference.node_id);
        }
        // A keyed `for` reorder relocates a row with this verb, and the row's
        // parent does not change; `notify_vacated` declines that case, so a
        // reorder still fires one notification and not two (issue #745).
        late_child::notify_vacated(vacated.as_ref(), self);
        late_child::notify_inserted(self, child);
    }

    /// Replace this node with another node.
    pub fn replace_with(&self, replacement: &NodeHandle) {
        let parent = self.parent_node();
        let vacated = late_child::vacated_parent(replacement);
        if let Some(doc) = self.accessed_doc() {
            doc.borrow_mut()
                .replace_node(self.node_id, replacement.node_id);
        }
        if let Some(parent) = parent {
            // Two nodes move: this one is displaced out of `parent`, and the
            // replacement may have come from somewhere else (issue #745).
            late_child::notify_removed(&parent);
            late_child::notify_vacated(vacated.as_ref(), &parent);
            late_child::notify_inserted(&parent, replacement);
        }
    }

    /// Remove this node from its parent, **leaving it re-insertable**.
    ///
    /// The subtree keeps its identity on every backend: append this handle again
    /// and the whole thing comes back (issue #719). Use it when the node may be
    /// shown again — a branch that toggles a captured handle.
    ///
    /// When you are finished with the subtree for good, call
    /// [`discard`](Self::discard) instead, or the backend keeps it alive for the
    /// life of the document. See [`DomDocument::remove_node`] and
    /// [`DomDocument::discard_node`] for what each backend reclaims.
    pub fn remove(&self) {
        let vacated = late_child::vacated_parent(self);
        if let Some(doc) = self.accessed_doc() {
            doc.borrow_mut().remove_node(self.node_id);
        }
        if let Some(vacated) = vacated {
            late_child::notify_removed(&vacated);
        }
    }

    /// Remove this node and **release the backend's bookkeeping** for it and
    /// every descendant — you are finished with the subtree for good.
    ///
    /// Treat this handle, and every handle into the subtree, as **dead**: do not
    /// re-attach it, build a fresh node instead. A backend that retires makes
    /// every operation on it a silent no-op — `rinch-web` and
    /// [`MockDomDocument`](mock::MockDomDocument) both do, so the mistake fails
    /// `cargo test` as well as a browser.
    ///
    /// What a discard actually reclaims differs by backend and is **not**
    /// guaranteed: on `rinch-dom` today it is exactly [`remove`](Self::remove)
    /// and the node still re-inserts (issue #723). See
    /// [`DomDocument::discard_node`] for the full contract.
    pub fn discard(&self) {
        // Before the backend lets go: a discarded id may be handed to the next
        // node the document mints, and an observer left behind under it would
        // then fire for a container that no longer exists (issue #716). Read the
        // parent here too, for the same reason — afterwards this node is
        // detached and may be retired (issue #745).
        late_child::forget_node(self);
        let vacated = late_child::vacated_parent(self);
        if let Some(doc) = self.accessed_doc() {
            doc.borrow_mut().discard_node(self.node_id);
        }
        if let Some(vacated) = vacated {
            late_child::notify_removed(&vacated);
        }
    }

    /// Focus this element programmatically.
    ///
    /// This sets the element as the currently focused element, allowing it to
    /// receive keyboard input. For input/textarea elements, this enables text input.
    pub fn focus(&self) {
        if let Some(doc) = self.accessed_doc() {
            doc.borrow_mut().focus_element(self.node_id);
        }
    }

    /// The element currently holding keyboard focus **in this node's document**
    /// (issue #695).
    ///
    /// A document-level question reached through a node handle, because a
    /// [`RenderScope`] is not available inside the effect that needs to ask it —
    /// the same shape as [`set_scroll_locked`](Self::set_scroll_locked), which
    /// is a document-level action spelled on the node it acts *for*.
    ///
    /// `None` means *unknown or outside this document*, not *nothing is
    /// focused*: on the web an element rinch did not create (and so carries no
    /// `__nid`) cannot be named here. See [`DomDocument::active_element`].
    pub fn active_element(&self) -> Option<NodeHandle> {
        let doc = self.accessed_doc()?;
        let id = doc.borrow().active_element()?;
        Some(NodeHandle::new(id, self.doc.clone()))
    }

    /// Move keyboard focus into this subtree the way a browser's `showModal()`
    /// does (issue #695): the `autofocus` descendant if there is one, else —
    /// under [`FocusIntoPolicy::FirstFocusable`] — the first focusable one.
    ///
    /// See [`DomDocument::focus_into`], including why desktop applies it after
    /// the next layout rather than immediately.
    pub fn focus_into(&self, policy: FocusIntoPolicy) {
        if let Some(doc) = self.accessed_doc() {
            doc.borrow_mut().focus_into(self.node_id, policy);
        }
    }

    /// Give the keyboard back now that this overlay has closed, or let it go
    /// (issue #695) — with **this node as the closing overlay's root**.
    ///
    /// The backend decides which, because neither answer is knowable at the
    /// moment an effect calls this: see [`DomDocument::restore_focus`] for the
    /// three questions and why a `blur()` verb would not have been enough.
    pub fn restore_focus(&self, opener: Option<&NodeHandle>) {
        if let Some(doc) = self.accessed_doc() {
            doc.borrow_mut()
                .restore_focus(opener.map(|o| o.node_id), self.node_id);
        }
    }

    /// Lock or unlock document-level scrolling, with **this node as the locking
    /// overlay's root** (issue #474).
    ///
    /// A `Modal`/`Drawer` with `lock_scroll` calls this on its own root when it
    /// opens and again when it closes or unmounts. The node matters: desktop
    /// keeps this subtree scrollable while rejecting a gesture anywhere else, so
    /// the dialog's own `overflow: auto` body still works. See
    /// [`DomDocument::set_scroll_locked`] for what each backend does and why the
    /// count is the backend's job.
    ///
    /// Every lock must be paired with an unlock. Do **not** reach for
    /// [`RenderScope::body_handle`](super::RenderScope::body_handle) instead:
    /// on the web that is `<div id="rinch-body">`, not the page.
    pub fn set_scroll_locked(&self, locked: bool) {
        if let Some(doc) = self.accessed_doc() {
            doc.borrow_mut().set_scroll_locked(locked, self.node_id);
        }
    }

    /// Get the children of this node as NodeHandles.
    pub fn children(&self) -> Vec<NodeHandle> {
        if let Some(doc) = self.accessed_doc() {
            let child_ids = doc.borrow().get_children(self.node_id);
            child_ids
                .into_iter()
                .map(|id| NodeHandle::new(id, self.doc.clone()))
                .collect()
        } else {
            Vec::new()
        }
    }

    /// The document this handle points into, if it is still alive.
    ///
    /// For code that walks several nodes at once and would otherwise upgrade the
    /// same `Weak` once per step — [`late_child::notify_inserted`] does, on every
    /// insertion.
    pub(super) fn doc_upgrade(&self) -> Option<Rc<RefCell<dyn DomDocument>>> {
        self.doc.upgrade()
    }

    /// Get the parent node.
    pub fn parent_node(&self) -> Option<NodeHandle> {
        let doc = self.accessed_doc()?;
        let parent_id = doc.borrow().parent_node(self.node_id)?;
        Some(NodeHandle::new(parent_id, self.doc.clone()))
    }

    /// Get the next sibling node.
    pub fn next_sibling(&self) -> Option<NodeHandle> {
        let doc = self.accessed_doc()?;
        let sibling_id = doc.borrow().next_sibling(self.node_id)?;
        Some(NodeHandle::new(sibling_id, self.doc.clone()))
    }

    /// Insert a node after this node (as next sibling).
    pub fn insert_after(&self, new_node: &NodeHandle) {
        let vacated = late_child::vacated_parent(new_node);
        let mut inserted_into = None;
        if let Some(doc) = self.accessed_doc() {
            let parent_id = doc.borrow().parent_node(self.node_id);
            if let Some(parent_id) = parent_id {
                let next = doc.borrow().next_sibling(self.node_id);
                if let Some(next_id) = next {
                    doc.borrow_mut()
                        .insert_before(parent_id, new_node.node_id, next_id);
                } else {
                    doc.borrow_mut().append_child(parent_id, new_node.node_id);
                }
                inserted_into = Some(parent_id);
            }
        }
        if let Some(parent_id) = inserted_into {
            let parent = NodeHandle::new(parent_id, self.doc.clone());
            late_child::notify_vacated(vacated.as_ref(), &parent);
            late_child::notify_inserted(&parent, new_node);
        }
    }

    /// Set a CSS style property.
    #[doc(hidden)]
    pub fn set_style(&self, property: &str, value: &str) {
        if let Some(doc) = self.accessed_doc() {
            doc.borrow_mut().set_style(self.node_id, property, value);
        }
    }

    /// Set multiple CSS style properties in a single operation.
    /// More efficient than calling `set_style` multiple times because it only
    /// parses the style string once.
    pub fn set_styles(&self, properties: &[(&str, &str)]) {
        if let Some(doc) = self.accessed_doc() {
            doc.borrow_mut().set_styles(self.node_id, properties);
        }
    }

    /// Lay an author-written inline style string **over** this node's own,
    /// last-wins per property (issue #647).
    ///
    /// This is what a component-level `style:` prop means: "applied to the
    /// component's root", not "instead of whatever the root already has". A
    /// component publishes its props through inline declarations on that root
    /// — every overlay's `z_index` is a custom property written there — and
    /// `set_attribute("style", …)` would erase all of them, silently and with
    /// the prop still compiling.
    ///
    /// Stateless, so a caller that writes repeatedly to the **same** node wants
    /// [`StyleProp`] instead: it takes its own previous declarations off before
    /// laying the new ones on, which is what stops a reactive binding from
    /// accumulating properties it no longer declares.
    pub fn merge_style(&self, css: &str) {
        StyleProp::default().apply(self, css);
    }

    /// Set the class attribute.
    pub fn set_class(&self, class: &str) {
        self.set_attribute("class", class);
    }

    /// Add a class to the element's class list.
    ///
    /// **Idempotent**, as `DOMTokenList.add` is: a class already on the element
    /// is not added a second time, and the attribute is left untouched. That
    /// matters because a reactive effect that owns one modifier class (issue
    /// #717) re-runs whenever anything it read changes, not only when its own
    /// answer flips — `Signal::set` notifies on every write, equal value or not
    /// (`set_if_changed` is the other method) — so a non-idempotent `add_class`
    /// grew the attribute by one word per write and healed only when the class
    /// came off again.
    #[doc(hidden)]
    pub fn add_class(&self, class: &str) {
        if let Some(doc) = self.accessed_doc() {
            // Get current class attribute (borrow ends here)
            let current = doc.borrow().get_attribute(self.node_id, "class");
            let new_class = match current {
                Some(existing) if existing.split_whitespace().any(|c| c == class) => return,
                Some(existing) if !existing.is_empty() => format!("{} {}", existing, class),
                _ => class.to_string(),
            };
            // Now safe to borrow_mut
            doc.borrow_mut()
                .set_attribute(self.node_id, "class", &new_class);
        }
    }

    /// Remove a class from the element's class list.
    ///
    /// Symmetric with [`add_class`](Self::add_class): a class that is **not** on
    /// the element is not removed a second time, and the attribute is left
    /// untouched (issue #730). #717 made that the common path rather than a rare
    /// one — ten components' effects call this on every run whose answer is
    /// `false`, and an effect re-runs whenever anything it read changes, not only
    /// when its own answer flips.
    ///
    /// The early return also means an elided removal no longer *tidies* the
    /// attribute: this rebuilds the class list by splitting on whitespace and
    /// joining with single spaces, so padding and double spaces used to be
    /// normalised away by a removal that took nothing off, and now survive.
    /// Everything that *matches* a class splits on whitespace — Stylo's
    /// selector matching, `query_selector`, `rinch-components`' list walk, the
    /// menu bar's — so the spelling reaches neither styling nor hit testing.
    /// It does reach `html_serializer`, which writes every attribute through
    /// verbatim: a padded `class` now serialises padded.
    #[doc(hidden)]
    pub fn remove_class(&self, class: &str) {
        if let Some(doc) = self.accessed_doc() {
            // Get current class attribute (borrow ends here)
            let existing = doc.borrow().get_attribute(self.node_id, "class");
            if let Some(existing) = existing {
                if !existing.split_whitespace().any(|c| c == class) {
                    return;
                }
                let new_class: String = existing
                    .split_whitespace()
                    .filter(|c| *c != class)
                    .collect::<Vec<_>>()
                    .join(" ");
                // Now safe to borrow_mut
                doc.borrow_mut()
                    .set_attribute(self.node_id, "class", &new_class);
            }
        }
    }

    /// Toggle a class on the element.
    pub fn toggle_class(&self, class: &str) {
        if let Some(doc) = self.accessed_doc() {
            // Get current class attribute (borrow ends here)
            let has_class = doc
                .borrow()
                .get_attribute(self.node_id, "class")
                .map(|c| c.split_whitespace().any(|c| c == class))
                .unwrap_or(false);

            // Now safe to call methods that borrow
            if has_class {
                self.remove_class(class);
            } else {
                self.add_class(class);
            }
        }
    }

    /// Set the vertical scroll position of this element.
    /// For elements with overflow: auto/scroll, this sets the vertical scroll offset.
    pub fn set_scroll_top(&self, scroll_top: f64) {
        if let Some(doc) = self.accessed_doc() {
            doc.borrow_mut().set_scroll_top(self.node_id, scroll_top);
        }
    }

    /// Get the vertical scroll position of this element.
    /// Equivalent to `element.scrollTop` in the web DOM.
    pub fn scroll_top(&self) -> f64 {
        self.accessed_doc()
            .map(|doc| doc.borrow().scroll_top(self.node_id))
            .unwrap_or(0.0)
    }

    /// Get the horizontal scroll position of this element.
    /// Equivalent to `element.scrollLeft` in the web DOM.
    pub fn scroll_left(&self) -> f64 {
        self.accessed_doc()
            .map(|doc| doc.borrow().scroll_left(self.node_id))
            .unwrap_or(0.0)
    }

    /// Set the horizontal scroll position of this element.
    /// Equivalent to `element.scrollLeft = value` in the web DOM.
    pub fn set_scroll_left(&self, scroll_left: f64) {
        if let Some(doc) = self.accessed_doc() {
            doc.borrow_mut().set_scroll_left(self.node_id, scroll_left);
        }
    }

    /// Get the total scrollable content height.
    /// Equivalent to `element.scrollHeight` in the web DOM.
    pub fn scroll_height(&self) -> f64 {
        self.accessed_doc()
            .map(|doc| doc.borrow().scroll_height(self.node_id))
            .unwrap_or(0.0)
    }

    /// Get the total scrollable content width.
    /// Equivalent to `element.scrollWidth` in the web DOM.
    pub fn scroll_width(&self) -> f64 {
        self.accessed_doc()
            .map(|doc| doc.borrow().scroll_width(self.node_id))
            .unwrap_or(0.0)
    }

    /// Get the visible content area height (layout height minus padding and border).
    /// Equivalent to `element.clientHeight` in the web DOM.
    pub fn client_height(&self) -> f64 {
        self.accessed_doc()
            .map(|doc| doc.borrow().client_height(self.node_id))
            .unwrap_or(0.0)
    }

    /// Get the visible content area width (layout width minus padding and border).
    /// Equivalent to `element.clientWidth` in the web DOM.
    pub fn client_width(&self) -> f64 {
        self.accessed_doc()
            .map(|doc| doc.borrow().client_width(self.node_id))
            .unwrap_or(0.0)
    }

    /// Scroll this element so its bottom content is visible.
    /// Convenience method: sets `scroll_top` to `scroll_height - client_height`.
    pub fn scroll_to_bottom(&self) {
        if let Some(doc) = self.accessed_doc() {
            let sh = doc.borrow().scroll_height(self.node_id);
            let ch = doc.borrow().client_height(self.node_id);
            let max = (sh - ch).max(0.0);
            doc.borrow_mut().set_scroll_top(self.node_id, max);
        }
    }

    /// Request that this element be scrolled into view — the minimal
    /// ("nearest") scroll: an element already in view does not move.
    ///
    /// The backends differ in when and how far:
    /// - **desktop** (`rinch-dom`) defers the scroll until after the next layout
    ///   pass, since the element's position must be known relative to its scroll
    ///   container, and scrolls only the **nearest** scroll container (issue #842).
    /// - **web** (`rinch-web`) scrolls immediately, with the browser's
    ///   `scrollIntoView({block: "nearest", inline: "nearest"})`, which scrolls
    ///   **every** scrollable ancestor, the page included. Until the editor's
    ///   caret scroll (#837) it was a silent no-op there.
    /// - the test `MockDomDocument` only queues the request.
    pub fn scroll_into_view(&self) {
        if let Some(doc) = self.accessed_doc() {
            doc.borrow_mut().request_scroll_into_view(self.node_id);
        }
    }

    /// Request that this element be scrolled to a set place in its scroll
    /// container: its top `fraction` of the way down the container's visible
    /// height (clamped to `0.0..=1.0`), kept at least `margin` px inside either
    /// edge, and the scroll clamped to what the content allows — so an element
    /// near the top of the content stays near the top. It moves even an element
    /// already in view.
    ///
    /// - **desktop** (`rinch-dom`) defers it until after the next layout pass
    ///   and moves the **nearest** scroll container only, vertically, as
    ///   [`Self::scroll_into_view`] does.
    /// - **web** (`rinch-web`) sets the nearest scroll container's `scrollTop`
    ///   at once (the page's, when no ancestor scrolls), then brings the element
    ///   into the page's view the way [`Self::scroll_into_view`] does.
    /// - the test `MockDomDocument` only queues the request.
    pub fn scroll_to_fraction(&self, fraction: f32, margin: f32) {
        let fraction = if fraction.is_nan() {
            0.0
        } else {
            fraction.clamp(0.0, 1.0)
        };
        if let Some(doc) = self.accessed_doc() {
            doc.borrow_mut()
                .request_scroll_to_fraction(self.node_id, fraction, margin.max(0.0));
        }
    }

    /// Replace this element's children by parsing an HTML string.
    ///
    /// This atomically removes all existing children and replaces them with
    /// the DOM tree produced by parsing `html`. The underlying document
    /// implementation handles parsing and insertion.
    pub fn set_inner_html(&self, html: &str) {
        if let Some(doc) = self.accessed_doc() {
            doc.borrow_mut().set_inner_html(self.node_id, html);
        }
    }

    /// Query a single child node matching the selector.
    pub fn query_selector(&self, selector: &str) -> Option<NodeHandle> {
        let doc = self.accessed_doc()?;
        let node_id = doc.borrow().query_selector(selector)?;
        Some(NodeHandle::new(node_id, self.doc.clone()))
    }

    /// Query all child nodes matching the selector.
    pub fn query_selector_all(&self, selector: &str) -> Vec<NodeHandle> {
        if let Some(doc) = self.accessed_doc() {
            doc.borrow()
                .query_selector_all(selector)
                .into_iter()
                .map(|id| NodeHandle::new(id, self.doc.clone()))
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Query the screen position of a text caret at the given byte offset.
    ///
    /// Returns the (x, y) coordinates where a text cursor would be rendered
    /// at the specified byte offset within this node's text content.
    ///
    /// # Returns
    /// Some((x, y)) if the node has text layout and the offset is valid, None otherwise
    pub fn query_caret_position(&self, byte_offset: usize) -> Option<(f32, f32)> {
        let doc = self.accessed_doc()?;
        doc.borrow()
            .query_caret_position(self.node_id.0 as u64, byte_offset)
    }

    /// Query the bounding box of a glyph cluster at the given byte offset.
    ///
    /// Returns the bounding box of the glyph cluster containing the specified
    /// byte offset within this node's text content.
    ///
    /// # Returns
    /// Some(GlyphBounds) if the node has text layout and the offset is valid, None otherwise
    pub fn query_glyph_bounds(&self, byte_offset: usize) -> Option<GlyphBounds> {
        let doc = self.accessed_doc()?;
        doc.borrow()
            .query_glyph_bounds(self.node_id.0 as u64, byte_offset)
    }

    /// Get the layout bounds of this node relative to its parent.
    ///
    /// Returns (x, y, width, height) if the node has been laid out.
    pub fn get_layout_bounds(&self) -> Option<(f32, f32, f32, f32)> {
        let doc = self.accessed_doc()?;
        doc.borrow().query_node_layout(self.node_id.0 as u64)
    }

    /// Reactive bounds signal for this element, refreshed by the runtime after
    /// each layout pass — including pure window resizes (#145).
    ///
    /// The signal carries absolute viewport-relative pixel bounds — the same
    /// frame [`crate::events::ClickContext::element_x`] uses, not the
    /// parent-relative values from [`Self::get_layout_bounds`]. Subscribers
    /// only re-run when the rect changes (uses `set_if_changed` internally).
    ///
    /// "The same frame" means the box the element is **painted** in, so a CSS
    /// transform on the element or any ancestor moves and resizes it, and a
    /// `position: fixed` element reports its viewport box (#203).
    ///
    /// There is no separate resize event: the root element's `bounds_signal()`
    /// is the supported way to observe viewport size changes — a full-size
    /// element's signal reports the new geometry as soon as the resize's
    /// layout pass completes.
    ///
    /// Initial value is `ElementBounds::default()` (zero rect). The first real
    /// bounds arrive after the next layout pass.
    ///
    /// Typical use: derive zoom / scroll / domain-coordinate math from a
    /// strip's measured pixel width without hand-rolling a polling thread.
    ///
    /// ```ignore
    /// let strip = __scope.create_element("div");
    /// // ... attach strip to DOM, etc.
    /// let strip_bounds = strip.bounds_signal();
    /// rsx! {
    ///     // child positioned at `bar_index / total_bars` of strip width,
    ///     // automatically updating when the strip resizes.
    ///     div {
    ///         style: {move || format!("left: {}px", bar_index as f32 / total_bars as f32 * strip_bounds.get().width)},
    ///     }
    /// }
    /// ```
    pub fn bounds_signal(&self) -> crate::reactive::Signal<crate::reactive::ElementBounds> {
        // Scope the registration to this node's document — node ids collide
        // across documents (issue #134). A handle whose document is already
        // gone registers under key 0 (matches no live document), yielding a
        // signal that simply keeps its zero rect.
        crate::reactive::register_bounds_signal(self.doc_key(), self.node_id.0 as u64)
    }

    /// Get the tag name of this node (if it's an element).
    pub fn tag_name(&self) -> Option<String> {
        let doc = self.accessed_doc()?;
        doc.borrow().tag_name(self.node_id)
    }

    /// Get the node type (1 = element, 3 = text, 8 = comment).
    pub fn node_type(&self) -> Option<u16> {
        let doc = self.accessed_doc()?;
        doc.borrow().node_type(self.node_id)
    }

    /// Get the text content of this node.
    ///
    /// For text nodes: returns the text. For elements: returns concatenated descendant text.
    pub fn text_content(&self) -> Option<String> {
        let doc = self.accessed_doc()?;
        doc.borrow().text_content(self.node_id)
    }
}

impl std::fmt::Debug for NodeHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeHandle")
            .field("node_id", &self.node_id)
            .field("valid", &self.is_valid())
            .finish()
    }
}

// ============================================================================
// reactive_component_dom
// ============================================================================

/// Re-render a component whenever signals read inside `render_fn` change.
///
/// This uses the same marker + Effect + DOM swap pattern as `show_dom`.
/// The entire component is reconstructed on each signal change — suitable for
/// component props that are closures (e.g., `variant: {|| if active.get() { "filled" } else { "light" }}`).
///
/// Unlike `show_dom`/`match_dom`/`for_each_dom`, `render_fn` runs **tracked**:
/// its signal reads are the only thing that can schedule a re-render, and the
/// runtime cannot tell a prop read from a subtree read — only the caller knows
/// where that line is. So a caller whose `render_fn` renders a user subtree
/// must draw it itself: read the driving values in the tracked region and wrap
/// the subtree render in [`crate::reactive::untracked`], or an incidental read
/// deep in the subtree rebuilds the whole component — and resets its
/// component-local state — on every change (issue #390). The `rsx!` macro's
/// generated `render_fn` does exactly that: prop closures tracked, children +
/// `Component::render` untracked.
///
/// Release the scratch container an `rsx!` component site builds its children
/// in (issue #719).
///
/// A component site mints a `<template>`, renders the site's children into it,
/// reads them back out with [`NodeHandle::children`] and hands them to
/// [`Component::render`], which re-parents the ones it adopts into its own tree.
/// The container is then dead — and it is dead in the one way scope ownership
/// cannot see: **it is in no subtree**, never having been attached to anything,
/// so the recursive discard of a branch's content root never reaches it. One
/// orphan per component render, measured at `+100` over 200 toggles of
/// `if open { Card {} }` on `rinch-web`.
///
/// Anything still under the container was **not** adopted, so it leaves with it —
/// by the same ownership rule the reactive helpers use, applied one level down:
/// a leftover the site built is discarded, a leftover the site was *handed*
/// (`Card { {captured.clone()} }` where `Card` ignores its children) is only
/// detached, so a caller's subtree is never retired out from under it.
///
/// Call it **after** `Component::render`, so the children it adopted have
/// already been re-parented out.
pub fn release_scratch_container(scope: &RenderScope, container: &NodeHandle) {
    for leftover in container.children() {
        if scope.created(leftover.node_id()) {
            leftover.discard();
        } else {
            leftover.remove();
        }
    }
    container.discard();
}

/// A `render_fn` that **memoises** — one that hands back a subtree it built
/// once, rather than building afresh — is supported on both backends, and is the
/// #654 shape. The previous output leaves by whichever verb its ownership says
/// (issue #719): built through this call's scope, it is discarded and the
/// backend reclaims it; handed in from outside, it is only detached and comes
/// back on the next run.
///
/// Returns the marker comment node. The caller should NOT append it again.
pub fn reactive_component_dom<R>(
    scope: &mut RenderScope,
    parent: &NodeHandle,
    render_fn: R,
) -> NodeHandle
where
    R: Fn(&mut RenderScope) -> NodeHandle + 'static,
{
    let marker = scope.create_comment("component");
    parent.append_child(&marker);

    let current_content: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(Vec::new()));
    let current_scope: Rc<RefCell<Option<RenderScope>>> = Rc::new(RefCell::new(None));
    let doc_weak = scope.doc_weak();
    let parent_id = parent.node_id();

    let cc = current_content.clone();
    let cs = current_scope.clone();
    let m = marker.clone();

    let effect = crate::reactive::Effect::new(move || {
        // Dispose old scope (cleans up nested effects).
        //
        // `take()` on its own line so the `RefMut` is not held across the
        // dispose — see the matching note in `show_dom` (issue #141).
        let old = cs.borrow_mut().take();

        // Ownership decides the verb, exactly as in `show_dom` (issue #719).
        // A `render_fn` that *builds* its output — what `rsx!` generates —
        // creates it through this scope, so the previous output is `discard`ed
        // and the backend lets go of it. A `render_fn` that memoises and hands
        // back a subtree it built once is supported too: that node is not this
        // scope's, so it is only detached and the next run re-inserts it.
        let doomed: Vec<(NodeHandle, bool)> = cc
            .borrow_mut()
            .drain(..)
            .map(|node| {
                let owned = old.as_ref().is_some_and(|s| s.created(node.node_id()));
                (node, owned)
            })
            .collect();

        if let Some(old) = old {
            old.dispose();
        }
        // Removal of either kind cancels the subtree's transitions and
        // animations in the document implementation (#699); stamping inline
        // `transition: none` here disarmed it permanently (#704).
        for (node, owned) in doomed {
            if owned {
                node.discard();
            } else {
                node.remove();
            }
        }
        // Render fresh
        if let Some(doc) = doc_weak.upgrade() {
            let mut child_scope = RenderScope::new(doc, parent_id);
            // The component's own resources belong to its own scope, not to the
            // effect that re-renders it (issue #141). This is the deepest reach
            // of the ambient owner: `render_fn` runs arbitrary user
            // `Component::render` code.
            let node = {
                let _owner = child_scope.push_owner();
                render_fn(&mut child_scope)
            };
            m.insert_after(&node);
            cc.borrow_mut().push(node);
            *cs.borrow_mut() = Some(child_scope);
        }
    });
    scope.create_effect_from(effect);
    marker
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use mock::MockDomDocument;

    /// A falsey write of `checked` / `selected` reaches the backend even when
    /// the content attribute is already absent (issue #687).
    ///
    /// `write_attribute`'s removal is otherwise guarded on the attribute being
    /// present, which reads as "already off" — true for every attribute whose
    /// whole state is the attribute, and false for these two on the web, where
    /// a user toggle moves the live IDL property and leaves the attribute
    /// behind. Only the backend can see that property, so the writer must call
    /// it; whether the call is worth making is not the writer's question.
    ///
    /// The mock records every mutation as a dirty node, so "did the call reach
    /// the backend" is observable here without a browser. Three mutants die:
    /// restoring the guard for the pair (row 1 and row 2), listing only
    /// `checked` in it (row 2), and dropping the guard for *everything* (row 3,
    /// which would restyle a node on every falsey write of any boolean
    /// attribute).
    #[test]
    fn a_falsey_write_of_the_presence_pair_always_reaches_the_backend() {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);

        let input = scope.create_element("input");
        let option = scope.create_element("option");
        let dirtied = |doc: &Rc<RefCell<MockDomDocument>>, n: &NodeHandle| {
            doc.borrow_mut().take_dirty_nodes().contains(&n.node_id())
        };
        doc.borrow_mut().take_dirty_nodes(); // drop the creation noise

        // Neither attribute is present, so the guard would skip both writes.
        input.write_attribute("checked", "false");
        assert!(
            dirtied(&doc, &input),
            "a falsey `checked` must reach the backend even with no attribute              to remove — on the web that call is the only thing that can clear              a user-toggled control (#687)"
        );

        option.write_attribute("selected", "false");
        assert!(
            dirtied(&doc, &option),
            "`selected` is the other half of the pair — an option's selectedness              goes dirty the same way"
        );

        // The name is ASCII case-insensitive, like every HTML attribute name
        // (#688). `is_boolean_attribute` folds, so an uppercase spelling reaches
        // the falsey branch; the pair check has to fold with it or `CHECKED:
        // {|| false}` keeps the bug. Sampled off the fixed point on purpose —
        // with the attribute *present* the guard finds it either way, because
        // `get_attribute` folds too.
        doc.borrow_mut().take_dirty_nodes();
        input.write_attribute("CHECKED", "false");
        assert!(
            dirtied(&doc, &input),
            "`CHECKED` is the same attribute as `checked`, and must reach the \
             backend the same way"
        );

        // The counter-case: every other boolean attribute keeps the guard, so a
        // falsey write with nothing to remove costs no invalidation.
        input.write_attribute("disabled", "false");
        assert!(
            !dirtied(&doc, &input),
            "an ordinary boolean attribute must keep its guard: its state is the              attribute, so an absent one is already off"
        );

        // Positive control for that instrument — the same call, with something
        // to remove, does reach the backend.
        input.write_attribute("disabled", "true");
        doc.borrow_mut().take_dirty_nodes();
        input.write_attribute("disabled", "false");
        assert!(
            dirtied(&doc, &input),
            "a falsey write that does erase an attribute must invalidate"
        );
        assert_eq!(input.get_attribute("disabled"), None);
    }

    /// A component's own resources are attributed to the component's child
    /// scope, not to the effect that re-renders it (issue #141).
    ///
    /// This is the deepest reach of the ambient owner: `render_fn` runs
    /// arbitrary user `Component::render` code, and it is always inside
    /// `run_effect`, so the item guard has to nest under the effect's push.
    #[test]
    fn a_component_body_is_attributed_to_its_own_child_scope() {
        use crate::reactive::{Owner, Signal, current_owner};

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let parent = scope.parent();

        let version = Signal::new(0);
        let seen: Rc<RefCell<Vec<Option<Owner>>>> = Rc::new(RefCell::new(Vec::new()));

        let log = seen.clone();
        let marker = reactive_component_dom(&mut scope, &parent, move |s: &mut RenderScope| {
            version.get();
            log.borrow_mut().push(current_owner());
            Signal::new(0);
            s.create_element("div")
        });
        let _ = marker;

        version.set(1); // re-render: disposes the old child scope, builds a new one

        let seen = seen.borrow();
        assert_eq!(seen.len(), 2, "initial render plus one re-render");

        let first = seen[0].clone().expect("the component runs under an owner");
        let second = seen[1].clone().expect("the component runs under an owner");
        assert_ne!(first, second, "each re-render gets a fresh child scope");
        assert_ne!(
            second,
            scope.owner(),
            "the component is not attributed to the scope that hosts it"
        );
        assert!(!first.is_alive(), "the previous child scope was disposed");
        assert_eq!(
            second.owned_counts().map(|c| c.signals),
            Some(1),
            "the component owns the signal its body created"
        );
        assert_eq!(scope.owned_counts().signals, 0);
    }

    #[test]
    fn test_node_handle_text() {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let mut scope = RenderScope::new(doc.clone(), doc.borrow().body());

        let text = scope.create_text("Hello");
        text.set_text("World");

        // Verify the text was updated
        assert!(!doc.borrow_mut().take_dirty_nodes().is_empty());
    }

    #[test]
    fn test_node_handle_attributes() {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let mut scope = RenderScope::new(doc.clone(), doc.borrow().body());

        let div = scope.create_element("div");
        div.set_attribute("id", "test");
        div.set_class("foo bar");

        assert_eq!(
            doc.borrow().get_attribute(div.node_id(), "id"),
            Some("test".to_string())
        );
        assert_eq!(
            doc.borrow().get_attribute(div.node_id(), "class"),
            Some("foo bar".to_string())
        );
    }

    #[test]
    fn test_node_handle_class_manipulation() {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let mut scope = RenderScope::new(doc.clone(), doc.borrow().body());

        let div = scope.create_element("div");
        div.set_class("foo");
        div.add_class("bar");

        assert_eq!(
            doc.borrow().get_attribute(div.node_id(), "class"),
            Some("foo bar".to_string())
        );

        // Idempotent, as `DOMTokenList.add` is (issue #717): a reactive effect
        // that owns one modifier class calls this on every run, and `Signal::set`
        // notifies on every write, not only on a change.
        div.add_class("bar");
        div.add_class("bar");
        assert_eq!(
            doc.borrow().get_attribute(div.node_id(), "class"),
            Some("foo bar".to_string()),
            "a class already present must not be added again"
        );

        div.remove_class("foo");
        assert_eq!(
            doc.borrow().get_attribute(div.node_id(), "class"),
            Some("bar".to_string())
        );

        // A prefix of an existing class is a different class, so the
        // whitespace-word comparison above is not a `contains`.
        div.add_class("ba");
        assert_eq!(
            doc.borrow().get_attribute(div.node_id(), "class"),
            Some("bar ba".to_string())
        );
    }

    /// `remove_class` of a class that is **not** there must not write the
    /// attribute at all (issue #730).
    ///
    /// The assertion is about the *write*, not about the resulting string: a
    /// string-only check cannot tell an elided write from a rewritten-identical
    /// one, which is the fixed point this class of test hides in. `set_attribute`
    /// on [`MockDomDocument`] marks the node dirty, so a drained dirty set that
    /// stays empty is the evidence — and the positive control below proves the
    /// instrument fires.
    #[test]
    fn remove_class_of_an_absent_class_writes_nothing() {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let mut scope = RenderScope::new(doc.clone(), doc.borrow().body());

        let div = scope.create_element("div");
        div.set_class("foo bar");

        // Positive control: removing a class that IS present writes, so the
        // dirty set is a live instrument and not a constant `empty`.
        doc.borrow_mut().take_dirty_nodes();
        div.remove_class("bar");
        assert_eq!(
            doc.borrow_mut().take_dirty_nodes(),
            vec![div.node_id()],
            "removing a class that is present must write the attribute"
        );
        assert_eq!(
            doc.borrow().get_attribute(div.node_id(), "class"),
            Some("foo".to_string())
        );

        // The case #730 is about. #717 made this the common path: ten
        // components' effects call `remove_class` on every run whose answer is
        // `false`, and an effect re-runs whenever anything it read changes, not
        // only when its own answer flips.
        div.remove_class("bar");
        assert!(
            doc.borrow_mut().take_dirty_nodes().is_empty(),
            "removing a class that is absent must not write the attribute"
        );

        // A prefix of a present class is a different class, so the elision is a
        // whitespace-word comparison and not a `contains` — `fo` is absent even
        // though `foo` is there.
        div.remove_class("fo");
        assert!(
            doc.borrow_mut().take_dirty_nodes().is_empty(),
            "a prefix of a present class is absent, so its removal writes nothing"
        );
        assert_eq!(
            doc.borrow().get_attribute(div.node_id(), "class"),
            Some("foo".to_string())
        );

        // A node with no `class` attribute at all was already write-free, and
        // stays that way.
        let bare = scope.create_element("div");
        doc.borrow_mut().take_dirty_nodes();
        bare.remove_class("anything");
        assert!(
            doc.borrow_mut().take_dirty_nodes().is_empty(),
            "a node with no class attribute must not gain one"
        );
        assert_eq!(doc.borrow().get_attribute(bare.node_id(), "class"), None);
    }

    /// Eliding the write of an absent class must not weaken the removal of a
    /// present one: a doubled class loses **every** copy.
    ///
    /// Measured by #726's reviewer on the pre-#730 code and kept deliberately —
    /// the early return is a guard on `any`, so it must not become a guard that
    /// stops after the first match.
    #[test]
    fn remove_class_removes_every_copy_of_a_doubled_class() {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let mut scope = RenderScope::new(doc.clone(), doc.borrow().body());

        let div = scope.create_element("div");
        div.set_class("a  a");
        div.remove_class("a");
        assert_eq!(
            doc.borrow().get_attribute(div.node_id(), "class"),
            Some(String::new()),
            "every whitespace-delimited copy of the class comes off"
        );

        let other = scope.create_element("div");
        other.set_class("keep dup keep2 dup");
        other.remove_class("dup");
        assert_eq!(
            doc.borrow().get_attribute(other.node_id(), "class"),
            Some("keep keep2".to_string())
        );
    }

    /// The elided write keeps the attribute's **spelling**, which the rewriting
    /// one did not (issue #730's own note).
    ///
    /// `remove_class` normalises whitespace as a side effect of its split/join,
    /// so a node whose `class` carries padding or double spaces used to be tidied
    /// by a removal that took nothing off. It is no longer. Nothing that
    /// *matches* a class reads it byte-wise — both backends split on
    /// whitespace — so this pins the new spelling rather than defending a
    /// consumer. `html_serializer` does write it through verbatim, so a padded
    /// `class` serialises padded; the workspace suite is green with that.
    #[test]
    fn an_elided_removal_leaves_the_attributes_spelling_alone() {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let mut scope = RenderScope::new(doc.clone(), doc.borrow().body());

        let div = scope.create_element("div");
        div.set_class("  foo   bar ");
        div.remove_class("baz");
        assert_eq!(
            doc.borrow().get_attribute(div.node_id(), "class"),
            Some("  foo   bar ".to_string()),
            "an elided removal writes nothing, so it tidies nothing either"
        );

        // A removal that does take a word off still normalises, as it always has.
        div.remove_class("foo");
        assert_eq!(
            doc.borrow().get_attribute(div.node_id(), "class"),
            Some("bar".to_string())
        );
    }

    #[test]
    fn test_render_scope_hierarchy() {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let mut scope = RenderScope::new(doc.clone(), doc.borrow().body());

        let container = scope.create_element("div");
        let child_scope = scope.child_scope(&container);

        let inner = child_scope.create_element("span");
        container.append_child(&inner);

        // Verify the hierarchy was created
        assert!(!doc.borrow_mut().take_dirty_nodes().is_empty());
    }

    #[test]
    fn test_update_batch() {
        let mut doc = MockDomDocument::new();
        let node_id = doc.create_element("div");

        let mut batch = UpdateBatch::new();
        batch.push(DomUpdate::SetText {
            node: node_id,
            text: "Hello".to_string(),
        });
        batch.push(DomUpdate::SetAttribute {
            node: node_id,
            name: "class".to_string(),
            value: "test".to_string(),
        });

        assert_eq!(batch.len(), 2);
        batch.apply(&mut doc);

        assert_eq!(
            doc.get_attribute(node_id, "class"),
            Some("test".to_string())
        );
    }

    /// `on_cleanup` must fire when a `RenderScope` is merely dropped, not only
    /// when the by-value `dispose()` is called (issue #141).
    ///
    /// This is the shape every real teardown path takes: `Drop for RenderScope`
    /// does nothing, so while cleanups lived in a second list on `RenderScope`
    /// they were silently discarded — which is why `unregister_editor` and
    /// `unregister_render_surface` never ran on a dropped context or a closed
    /// DevTools window.
    #[test]
    fn on_cleanup_runs_when_the_render_scope_is_dropped() {
        use std::cell::Cell;
        use std::rc::Rc;

        let fired = Rc::new(Cell::new(false));

        {
            let doc = Rc::new(RefCell::new(MockDomDocument::new()));
            let mut scope = RenderScope::new(doc.clone(), doc.borrow().body());
            let flag = fired.clone();
            scope.on_cleanup(move || flag.set(true));
            assert!(!fired.get(), "cleanup must not run before teardown");
            // Dropped here — NOT `dispose()`d.
        }

        assert!(
            fired.get(),
            "on_cleanup must run on drop, not only on the by-value dispose()"
        );
    }

    /// The explicit `dispose()` path keeps working after the delegation.
    #[test]
    fn on_cleanup_runs_on_explicit_dispose() {
        use std::cell::Cell;
        use std::rc::Rc;

        let fired = Rc::new(Cell::new(false));
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let mut scope = RenderScope::new(doc.clone(), doc.borrow().body());
        let flag = fired.clone();
        scope.on_cleanup(move || flag.set(true));

        scope.dispose();

        assert!(fired.get(), "on_cleanup must still run on dispose()");
    }

    /// A component cleanup that reads a signal must not subscribe the re-render
    /// effect (issue #494).
    ///
    /// A re-render disposes the previous component scope from inside
    /// `reactive_component_dom`'s effect, so an `on_cleanup` registered by the
    /// component body runs with that effect as the current observer. A tracked
    /// read there would re-render the whole component — resetting its
    /// component-local state — on every later write to that signal. Counts
    /// renders, since the DOM is identical either way.
    #[test]
    fn a_component_cleanup_that_reads_a_signal_does_not_subscribe_the_rerender_effect() {
        use crate::reactive::{Signal, on_cleanup};
        use std::cell::Cell;

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let parent = scope.parent();

        let rev = Signal::new(0u32);
        let probe = Signal::new(0u32);
        let renders = Rc::new(Cell::new(0usize));

        let count = renders.clone();
        let _marker = reactive_component_dom(&mut scope, &parent, move |s| {
            count.set(count.get() + 1);
            let _ = rev.get(); // the tracked "prop" read driving re-renders
            on_cleanup(move || {
                let _ = probe.get();
            });
            s.create_element("div")
        });

        rev.set(1); // re-render: disposes the previous scope, runs its cleanup
        let renders_before = renders.get();

        probe.set(1);
        assert_eq!(
            renders.get(),
            renders_before,
            "a write to a signal only the cleanup read must not re-render the component"
        );

        // Positive control: a write the re-render effect legitimately tracks.
        rev.set(2);
        assert_eq!(renders.get(), renders_before + 1);
    }
}
