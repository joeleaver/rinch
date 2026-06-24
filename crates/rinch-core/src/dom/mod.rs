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

/// A headless [`DomDocument`](traits::DomDocument) implementation for tests.
/// Available to downstream test code via the `test-util` feature.
#[cfg(any(test, feature = "test-util"))]
pub mod mock;
mod render_scope;
pub mod traits;

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

    /// Get the underlying node ID.
    pub fn node_id(&self) -> NodeId {
        self.node_id
    }

    /// Check if this handle still points to a valid node.
    pub fn is_valid(&self) -> bool {
        self.doc.upgrade().is_some()
    }

    /// Set the text content of this node.
    ///
    /// For text nodes, this updates the text directly.
    /// For element nodes, this replaces all children with a single text node.
    #[doc(hidden)]
    pub fn set_text(&self, text: &str) {
        if let Some(doc) = self.doc.upgrade() {
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
        if let Some(doc) = self.doc.upgrade() {
            doc.borrow_mut().set_attribute(self.node_id, name, value);
        }
    }

    /// Remove an attribute from this element.
    pub fn remove_attribute(&self, name: &str) {
        if let Some(doc) = self.doc.upgrade() {
            doc.borrow_mut().remove_attribute(self.node_id, name);
        }
    }

    /// Get an attribute value from this element.
    pub fn get_attribute(&self, name: &str) -> Option<String> {
        let doc = self.doc.upgrade()?;
        doc.borrow().get_attribute(self.node_id, name)
    }

    /// Append a child node to this element.
    #[doc(hidden)]
    pub fn append_child(&self, child: &NodeHandle) {
        if let Some(doc) = self.doc.upgrade() {
            doc.borrow_mut().append_child(self.node_id, child.node_id);
        }
    }

    /// Remove a child node from this element.
    pub fn remove_child(&self, child: &NodeHandle) {
        if let Some(doc) = self.doc.upgrade() {
            doc.borrow_mut().remove_child(self.node_id, child.node_id);
        }
    }

    /// Insert a child before a reference node.
    pub fn insert_before(&self, child: &NodeHandle, reference: &NodeHandle) {
        if let Some(doc) = self.doc.upgrade() {
            doc.borrow_mut()
                .insert_before(self.node_id, child.node_id, reference.node_id);
        }
    }

    /// Replace this node with another node.
    pub fn replace_with(&self, replacement: &NodeHandle) {
        if let Some(doc) = self.doc.upgrade() {
            doc.borrow_mut()
                .replace_node(self.node_id, replacement.node_id);
        }
    }

    /// Remove this node from its parent.
    pub fn remove(&self) {
        if let Some(doc) = self.doc.upgrade() {
            doc.borrow_mut().remove_node(self.node_id);
        }
    }

    /// Focus this element programmatically.
    ///
    /// This sets the element as the currently focused element, allowing it to
    /// receive keyboard input. For input/textarea elements, this enables text input.
    pub fn focus(&self) {
        if let Some(doc) = self.doc.upgrade() {
            doc.borrow_mut().focus_element(self.node_id);
        }
    }

    /// Get the children of this node as NodeHandles.
    pub fn children(&self) -> Vec<NodeHandle> {
        if let Some(doc) = self.doc.upgrade() {
            let child_ids = doc.borrow().get_children(self.node_id);
            child_ids
                .into_iter()
                .map(|id| NodeHandle::new(id, self.doc.clone()))
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get the parent node.
    pub fn parent_node(&self) -> Option<NodeHandle> {
        let doc = self.doc.upgrade()?;
        let parent_id = doc.borrow().parent_node(self.node_id)?;
        Some(NodeHandle::new(parent_id, self.doc.clone()))
    }

    /// Get the next sibling node.
    pub fn next_sibling(&self) -> Option<NodeHandle> {
        let doc = self.doc.upgrade()?;
        let sibling_id = doc.borrow().next_sibling(self.node_id)?;
        Some(NodeHandle::new(sibling_id, self.doc.clone()))
    }

    /// Insert a node after this node (as next sibling).
    pub fn insert_after(&self, new_node: &NodeHandle) {
        if let Some(doc) = self.doc.upgrade() {
            let parent_id = doc.borrow().parent_node(self.node_id);
            if let Some(parent_id) = parent_id {
                let next = doc.borrow().next_sibling(self.node_id);
                if let Some(next_id) = next {
                    doc.borrow_mut()
                        .insert_before(parent_id, new_node.node_id, next_id);
                } else {
                    doc.borrow_mut().append_child(parent_id, new_node.node_id);
                }
            }
        }
    }

    /// Set a CSS style property.
    #[doc(hidden)]
    pub fn set_style(&self, property: &str, value: &str) {
        if let Some(doc) = self.doc.upgrade() {
            doc.borrow_mut().set_style(self.node_id, property, value);
        }
    }

    /// Set multiple CSS style properties in a single operation.
    /// More efficient than calling `set_style` multiple times because it only
    /// parses the style string once.
    pub fn set_styles(&self, properties: &[(&str, &str)]) {
        if let Some(doc) = self.doc.upgrade() {
            doc.borrow_mut().set_styles(self.node_id, properties);
        }
    }

    /// Set the class attribute.
    pub fn set_class(&self, class: &str) {
        self.set_attribute("class", class);
    }

    /// Add a class to the element's class list.
    #[doc(hidden)]
    pub fn add_class(&self, class: &str) {
        if let Some(doc) = self.doc.upgrade() {
            // Get current class attribute (borrow ends here)
            let current = doc.borrow().get_attribute(self.node_id, "class");
            let new_class = match current {
                Some(existing) if !existing.is_empty() => format!("{} {}", existing, class),
                _ => class.to_string(),
            };
            // Now safe to borrow_mut
            doc.borrow_mut()
                .set_attribute(self.node_id, "class", &new_class);
        }
    }

    /// Remove a class from the element's class list.
    #[doc(hidden)]
    pub fn remove_class(&self, class: &str) {
        if let Some(doc) = self.doc.upgrade() {
            // Get current class attribute (borrow ends here)
            let existing = doc.borrow().get_attribute(self.node_id, "class");
            if let Some(existing) = existing {
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
        if let Some(doc) = self.doc.upgrade() {
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
        if let Some(doc) = self.doc.upgrade() {
            doc.borrow_mut().set_scroll_top(self.node_id, scroll_top);
        }
    }

    /// Get the vertical scroll position of this element.
    /// Equivalent to `element.scrollTop` in the web DOM.
    pub fn scroll_top(&self) -> f64 {
        self.doc
            .upgrade()
            .map(|doc| doc.borrow().scroll_top(self.node_id))
            .unwrap_or(0.0)
    }

    /// Get the horizontal scroll position of this element.
    /// Equivalent to `element.scrollLeft` in the web DOM.
    pub fn scroll_left(&self) -> f64 {
        self.doc
            .upgrade()
            .map(|doc| doc.borrow().scroll_left(self.node_id))
            .unwrap_or(0.0)
    }

    /// Set the horizontal scroll position of this element.
    /// Equivalent to `element.scrollLeft = value` in the web DOM.
    pub fn set_scroll_left(&self, scroll_left: f64) {
        if let Some(doc) = self.doc.upgrade() {
            doc.borrow_mut().set_scroll_left(self.node_id, scroll_left);
        }
    }

    /// Get the total scrollable content height.
    /// Equivalent to `element.scrollHeight` in the web DOM.
    pub fn scroll_height(&self) -> f64 {
        self.doc
            .upgrade()
            .map(|doc| doc.borrow().scroll_height(self.node_id))
            .unwrap_or(0.0)
    }

    /// Get the total scrollable content width.
    /// Equivalent to `element.scrollWidth` in the web DOM.
    pub fn scroll_width(&self) -> f64 {
        self.doc
            .upgrade()
            .map(|doc| doc.borrow().scroll_width(self.node_id))
            .unwrap_or(0.0)
    }

    /// Get the visible content area height (layout height minus padding and border).
    /// Equivalent to `element.clientHeight` in the web DOM.
    pub fn client_height(&self) -> f64 {
        self.doc
            .upgrade()
            .map(|doc| doc.borrow().client_height(self.node_id))
            .unwrap_or(0.0)
    }

    /// Get the visible content area width (layout width minus padding and border).
    /// Equivalent to `element.clientWidth` in the web DOM.
    pub fn client_width(&self) -> f64 {
        self.doc
            .upgrade()
            .map(|doc| doc.borrow().client_width(self.node_id))
            .unwrap_or(0.0)
    }

    /// Scroll this element so its bottom content is visible.
    /// Convenience method: sets `scroll_top` to `scroll_height - client_height`.
    pub fn scroll_to_bottom(&self) {
        if let Some(doc) = self.doc.upgrade() {
            let sh = doc.borrow().scroll_height(self.node_id);
            let ch = doc.borrow().client_height(self.node_id);
            let max = (sh - ch).max(0.0);
            doc.borrow_mut().set_scroll_top(self.node_id, max);
        }
    }

    /// Request that this element be scrolled into view.
    ///
    /// The scroll is deferred until after the next layout pass, since the
    /// element's position must be known relative to its scroll container.
    pub fn scroll_into_view(&self) {
        if let Some(doc) = self.doc.upgrade() {
            doc.borrow_mut().request_scroll_into_view(self.node_id);
        }
    }

    /// Replace this element's children by parsing an HTML string.
    ///
    /// This atomically removes all existing children and replaces them with
    /// the DOM tree produced by parsing `html`. The underlying document
    /// implementation handles parsing and insertion.
    pub fn set_inner_html(&self, html: &str) {
        if let Some(doc) = self.doc.upgrade() {
            doc.borrow_mut().set_inner_html(self.node_id, html);
        }
    }

    /// Clear CSS animations and transitions on this node and all descendants.
    ///
    /// This should be called before removing nodes to ensure rinch-dom cleans up
    /// its internal animation state. Without this, rinch-dom may crash when trying
    /// to access deleted nodes during the next resolve() call.
    pub fn clear_animations(&self) {
        if let Some(doc) = self.doc.upgrade() {
            // Clear on this node
            {
                let mut doc_ref = doc.borrow_mut();
                doc_ref.set_style(self.node_id, "transition", "none");
                doc_ref.set_style(self.node_id, "animation", "none");
            }
            // Recursively clear on children
            for child in self.children() {
                child.clear_animations();
            }
        }
    }

    /// Query a single child node matching the selector.
    pub fn query_selector(&self, selector: &str) -> Option<NodeHandle> {
        let doc = self.doc.upgrade()?;
        let node_id = doc.borrow().query_selector(selector)?;
        Some(NodeHandle::new(node_id, self.doc.clone()))
    }

    /// Query all child nodes matching the selector.
    pub fn query_selector_all(&self, selector: &str) -> Vec<NodeHandle> {
        if let Some(doc) = self.doc.upgrade() {
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
        let doc = self.doc.upgrade()?;
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
        let doc = self.doc.upgrade()?;
        doc.borrow()
            .query_glyph_bounds(self.node_id.0 as u64, byte_offset)
    }

    /// Get the layout bounds of this node relative to its parent.
    ///
    /// Returns (x, y, width, height) if the node has been laid out.
    pub fn get_layout_bounds(&self) -> Option<(f32, f32, f32, f32)> {
        let doc = self.doc.upgrade()?;
        doc.borrow().query_node_layout(self.node_id.0 as u64)
    }

    /// Reactive bounds signal for this element, refreshed by the runtime after
    /// each layout pass.
    ///
    /// The signal carries absolute viewport-relative pixel bounds — the same
    /// frame [`crate::events::ClickContext::element_x`] uses, not the
    /// parent-relative values from [`Self::get_layout_bounds`]. Subscribers
    /// only re-run when the rect changes (uses `set_if_changed` internally).
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
        crate::reactive::register_bounds_signal(self.node_id.0 as u64)
    }

    /// Get the tag name of this node (if it's an element).
    pub fn tag_name(&self) -> Option<String> {
        let doc = self.doc.upgrade()?;
        doc.borrow().tag_name(self.node_id)
    }

    /// Get the node type (1 = element, 3 = text, 8 = comment).
    pub fn node_type(&self) -> Option<u16> {
        let doc = self.doc.upgrade()?;
        doc.borrow().node_type(self.node_id)
    }

    /// Get the text content of this node.
    ///
    /// For text nodes: returns the text. For elements: returns concatenated descendant text.
    pub fn text_content(&self) -> Option<String> {
        let doc = self.doc.upgrade()?;
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
        // Dispose old scope (cleans up nested effects)
        if let Some(old) = cs.borrow_mut().take() {
            old.dispose();
        }
        // Remove old nodes
        for node in cc.borrow_mut().drain(..) {
            node.clear_animations();
            node.remove();
        }
        // Render fresh
        if let Some(doc) = doc_weak.upgrade() {
            let mut child_scope = RenderScope::new(doc, parent_id);
            let node = render_fn(&mut child_scope);
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
}
