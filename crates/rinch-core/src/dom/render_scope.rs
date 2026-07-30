//! RenderScope, UpdateBatch, and DomUpdate types.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use crate::reactive::{Effect, Scope};

use super::traits::DomDocument;
use super::{NodeHandle, NodeId, next_reactive_id};

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
    /// Effects created within this scope (for future direct tracking).
    #[allow(dead_code)]
    effects: Vec<Effect>,
    /// Child scopes (for hierarchical cleanup).
    children: Vec<RenderScope>,
    /// The reactive scope for effect management **and cleanups** — see
    /// [`RenderScope::on_cleanup`]. Cleanups deliberately live here and not in a
    /// second list on `RenderScope`, so they run on drop as well as on dispose.
    reactive_scope: Scope,
}

impl RenderScope {
    /// Create a new render scope rooted at the given node.
    pub fn new(doc: Rc<RefCell<dyn DomDocument>>, parent_id: NodeId) -> Self {
        Self {
            doc: Rc::downgrade(&doc),
            parent_id,
            effects: Vec::new(),
            children: Vec::new(),
            reactive_scope: Scope::new(),
        }
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
        NodeHandle::new(node_id, self.doc.clone())
    }

    /// Create a new text node and return a handle to it.
    #[doc(hidden)]
    pub fn create_text(&mut self, text: &str) -> NodeHandle {
        let doc = self.doc().expect("Document dropped");
        let node_id = doc.borrow_mut().create_text(text);
        NodeHandle::new(node_id, self.doc.clone())
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
        let span = NodeHandle::new(span_id, self.doc.clone());
        span.set_attribute("data-rid-reactive", &reactive_id.to_string());
        span.set_attribute("style", "display:contents"); // Invisible wrapper

        // Create the text node inside the span
        let text_id = doc.borrow_mut().create_text(initial_text);
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
        NodeHandle::new(node_id, self.doc.clone())
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
    /// The handler receives the current scroll offset (scroll_top) as `f64`.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let handler_id = scope.register_scroll_handler(|scroll_top| {
    ///     println!("Scrolled to: {}", scroll_top);
    /// });
    /// element.set_attribute("data-onscroll", &handler_id.to_string());
    /// ```
    #[doc(hidden)]
    pub fn register_scroll_handler<F: Fn(f64) + 'static>(
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
        // Nothing to do: `children` are `RenderScope`s that drop recursively,
        // and effects + cleanups belong to `reactive_scope`, whose `Drop` calls
        // its own iterative `dispose()`. This is what makes `on_cleanup` fire on
        // drop-only teardown paths (issue #141).
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
