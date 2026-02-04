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
//! Widgets access the current scope via [`with_render_scope`] or [`try_with_render_scope`].
//!
//! ```ignore
//! // Runtime sets up the scope
//! set_render_scope(scope);
//!
//! // Widgets access it
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
//! fn counter(scope: &mut RenderScope) -> NodeHandle {
//!     let count = use_signal(|| 0);
//!
//!     // Create static structure once
//!     let div = scope.create_element("div");
//!     let text = scope.create_text("Count: ");
//!     let value = scope.create_text("0");
//!
//!     // Set up reactive binding - only updates this text node
//!     let value_handle = value.clone();
//!     scope.create_effect(move || {
//!         value_handle.set_text(&count.get().to_string());
//!     });
//!
//!     div.append_child(&text);
//!     div.append_child(&value);
//!     div
//! }
//! ```

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

use crate::reactive::{Effect, Scope};

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
    pub fn set_text(&self, text: &str) {
        if let Some(doc) = self.doc.upgrade() {
            tracing::debug!("NodeHandle::set_text(node={}, text_len={})", self.node_id.0, text.len());
            doc.borrow_mut().set_text_content(self.node_id, text);
        } else {
            tracing::warn!("NodeHandle::set_text FAILED - doc Weak reference is dead (node={})", self.node_id.0);
        }
    }

    /// Set an attribute on this element.
    ///
    /// # Panics
    /// May panic if called on a non-element node.
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
                    doc.borrow_mut().insert_before(parent_id, new_node.node_id, next_id);
                } else {
                    doc.borrow_mut().append_child(parent_id, new_node.node_id);
                }
            }
        }
    }

    /// Set a CSS style property.
    pub fn set_style(&self, property: &str, value: &str) {
        if let Some(doc) = self.doc.upgrade() {
            doc.borrow_mut().set_style(self.node_id, property, value);
        }
    }

    /// Set the class attribute.
    pub fn set_class(&self, class: &str) {
        self.set_attribute("class", class);
    }

    /// Add a class to the element's class list.
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

    /// Set the scroll position of this element.
    /// For elements with overflow: auto/scroll, this sets the vertical scroll offset.
    pub fn set_scroll_top(&self, scroll_top: f64) {
        if let Some(doc) = self.doc.upgrade() {
            doc.borrow_mut().set_scroll_top(self.node_id, scroll_top);
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
    /// This should be called before removing nodes to ensure blitz cleans up
    /// its internal animation state. Without this, blitz may crash when trying
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
            doc.borrow().query_selector_all(selector)
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
        doc.borrow().query_caret_position(self.node_id.0 as u64, byte_offset)
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
        doc.borrow().query_glyph_bounds(self.node_id.0 as u64, byte_offset)
    }

    /// Get the layout bounds of this node relative to its parent.
    ///
    /// Returns (x, y, width, height) if the node has been laid out.
    pub fn get_layout_bounds(&self) -> Option<(f32, f32, f32, f32)> {
        let doc = self.doc.upgrade()?;
        doc.borrow().query_node_layout(self.node_id.0 as u64)
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

/// Trait for converting values into DOM nodes.
///
/// This trait enables the rsx! macro to handle both NodeHandle returns
/// (from component functions) and text values in a uniform way.
pub trait IntoNode {
    /// Convert this value into a DOM node.
    fn into_node(self, scope: &mut RenderScope) -> NodeHandle;
}

impl IntoNode for NodeHandle {
    #[inline]
    fn into_node(self, _scope: &mut RenderScope) -> NodeHandle {
        self
    }
}

impl IntoNode for String {
    #[inline]
    fn into_node(self, scope: &mut RenderScope) -> NodeHandle {
        scope.create_text(&self)
    }
}

impl IntoNode for &str {
    #[inline]
    fn into_node(self, scope: &mut RenderScope) -> NodeHandle {
        scope.create_text(self)
    }
}

impl IntoNode for std::borrow::Cow<'_, str> {
    #[inline]
    fn into_node(self, scope: &mut RenderScope) -> NodeHandle {
        scope.create_text(self.as_ref())
    }
}

// Implement IntoNode for numeric types
macro_rules! impl_into_node_for_display {
    ($($ty:ty),*) => {
        $(
            impl IntoNode for $ty {
                #[inline]
                fn into_node(self, scope: &mut RenderScope) -> NodeHandle {
                    scope.create_text(&self.to_string())
                }
            }
        )*
    };
}

impl_into_node_for_display!(
    i8, i16, i32, i64, i128, isize,
    u8, u16, u32, u64, u128, usize,
    f32, f64,
    bool, char
);

/// Bounding box for a glyph cluster.
#[derive(Debug, Clone, Copy)]
pub struct GlyphBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Trait for DOM documents that support mutation operations.
///
/// This trait abstracts the DOM mutation API, allowing different
/// implementations (blitz-dom, web-sys, etc.) to be used.
///
/// # Required Operations
///
/// Implementors must provide:
/// - Node creation: [`create_element`](DomDocument::create_element), [`create_text`](DomDocument::create_text)
/// - Tree mutation: [`append_child`](DomDocument::append_child), [`remove_child`](DomDocument::remove_child), etc.
/// - Attribute mutation: [`set_attribute`](DomDocument::set_attribute), [`remove_attribute`](DomDocument::remove_attribute)
/// - Text mutation: [`set_text_content`](DomDocument::set_text_content)
/// - Dirty tracking: [`mark_dirty`](DomDocument::mark_dirty), [`take_dirty_nodes`](DomDocument::take_dirty_nodes)
pub trait DomDocument {
    /// Create a new element node with the given tag name.
    fn create_element(&mut self, tag: &str) -> NodeId;

    /// Create a new text node with the given content.
    fn create_text(&mut self, text: &str) -> NodeId;

    /// Create a comment node (useful for markers).
    fn create_comment(&mut self, text: &str) -> NodeId;

    /// Append a child node to a parent element.
    fn append_child(&mut self, parent: NodeId, child: NodeId);

    /// Remove a child node from a parent element.
    fn remove_child(&mut self, parent: NodeId, child: NodeId);

    /// Insert a child before a reference node.
    fn insert_before(&mut self, parent: NodeId, child: NodeId, reference: NodeId);

    /// Replace a node with another node.
    fn replace_node(&mut self, old: NodeId, new: NodeId);

    /// Remove a node from its parent.
    fn remove_node(&mut self, node: NodeId);

    /// Set the text content of a node.
    fn set_text_content(&mut self, node: NodeId, text: &str);

    /// Set an attribute on an element.
    fn set_attribute(&mut self, node: NodeId, name: &str, value: &str);

    /// Remove an attribute from an element.
    fn remove_attribute(&mut self, node: NodeId, name: &str);

    /// Get an attribute value.
    fn get_attribute(&self, node: NodeId, name: &str) -> Option<String>;

    /// Set a CSS style property on an element.
    fn set_style(&mut self, node: NodeId, property: &str, value: &str);

    /// Mark a node as dirty (needs re-layout).
    fn mark_dirty(&mut self, node: NodeId);

    /// Get and clear the set of dirty nodes.
    fn take_dirty_nodes(&mut self) -> Vec<NodeId>;

    /// Get the root node of the document.
    fn root(&self) -> NodeId;

    /// Get the body element of the document.
    fn body(&self) -> NodeId;

    /// Query selector to find a node.
    fn query_selector(&self, selector: &str) -> Option<NodeId>;

    /// Query selector to find all matching nodes.
    fn query_selector_all(&self, selector: &str) -> Vec<NodeId>;

    /// Get the children of a node.
    fn get_children(&self, node: NodeId) -> Vec<NodeId>;

    /// Insert a child at a specific index.
    fn insert_child(&mut self, parent: NodeId, child: NodeId, index: usize);

    /// Get the parent of a node.
    fn parent_node(&self, node: NodeId) -> Option<NodeId>;

    /// Get the next sibling of a node.
    fn next_sibling(&self, node: NodeId) -> Option<NodeId>;

    /// Parse HTML string and create DOM nodes, returning the root node ID.
    /// Returns None if the HTML couldn't be parsed.
    fn parse_html(&mut self, html: &str) -> Option<NodeId>;

    /// Set the scroll position of an element.
    /// For elements with overflow: auto/scroll, this sets the scroll offset.
    fn set_scroll_top(&mut self, node: NodeId, scroll_top: f64);

    /// Replace the children of an element by parsing an HTML string.
    ///
    /// All existing children are removed and replaced with the DOM tree
    /// produced by parsing `html`. This provides an atomic update path
    /// that avoids incremental mutations which can leave the document
    /// in an inconsistent state.
    fn set_inner_html(&mut self, node: NodeId, html: &str);

    /// Query the screen position of a text caret at the given byte offset.
    ///
    /// Returns the (x, y) coordinates where a text cursor would be rendered
    /// at the specified byte offset within the text node.
    ///
    /// # Arguments
    /// * `node_id` - The ID of the text node or element containing text
    /// * `byte_offset` - The UTF-8 byte offset within the text content
    ///
    /// # Returns
    /// Some((x, y)) if the node has text layout and the offset is valid, None otherwise
    fn query_caret_position(&self, node_id: u64, byte_offset: usize) -> Option<(f32, f32)>;

    /// Query the bounding box of a glyph cluster at the given byte offset.
    ///
    /// Returns the bounding box of the glyph cluster containing the specified
    /// byte offset within the text node.
    ///
    /// # Arguments
    /// * `node_id` - The ID of the text node or element containing text
    /// * `byte_offset` - The UTF-8 byte offset within the text content
    ///
    /// # Returns
    /// Some(GlyphBounds) if the node has text layout and the offset is valid, None otherwise
    fn query_glyph_bounds(&self, node_id: u64, byte_offset: usize) -> Option<GlyphBounds>;

    /// Focus a specific element programmatically.
    ///
    /// This sets the element as the currently focused element, allowing it to
    /// receive keyboard input. For input/textarea elements, this enables text input.
    ///
    /// # Arguments
    /// * `node_id` - The ID of the element to focus
    fn focus_element(&mut self, node_id: NodeId);

    /// Resolve layout for the document at the given viewport size.
    ///
    /// This computes Taffy layout and builds text layouts (IFC) for all dirty nodes.
    /// Must be called before querying caret positions or glyph bounds.
    ///
    /// # Arguments
    /// * `width` - Viewport width in pixels
    /// * `height` - Viewport height in pixels
    fn resolve_layout(&mut self, width: f32, height: f32);

    /// Query the layout bounds of a node relative to its parent.
    ///
    /// Returns the (x, y, width, height) of the node's layout box.
    fn query_node_layout(&self, node_id: u64) -> Option<(f32, f32, f32, f32)>;
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
    /// Effects created within this scope (for future direct tracking).
    #[allow(dead_code)]
    effects: Vec<Effect>,
    /// Child scopes (for hierarchical cleanup).
    children: Vec<RenderScope>,
    /// Cleanup functions to run on dispose.
    cleanups: Vec<Box<dyn FnOnce()>>,
    /// The reactive scope for effect management.
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
            cleanups: Vec::new(),
            reactive_scope: Scope::new(),
        }
    }

    /// Get the document reference.
    fn doc(&self) -> Option<Rc<RefCell<dyn DomDocument>>> {
        self.doc.upgrade()
    }

    /// Create a new element and return a handle to it.
    pub fn create_element(&mut self, tag: &str) -> NodeHandle {
        let doc = self.doc().expect("Document dropped");
        let node_id = doc.borrow_mut().create_element(tag);
        NodeHandle::new(node_id, self.doc.clone())
    }

    /// Create a new text node and return a handle to it.
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
            if initial_text.len() > 20 { &initial_text[..20] } else { initial_text },
            span_id
        );

        (span, reactive_id)
    }

    /// Create a comment node (useful as a placeholder/marker).
    pub fn create_comment(&mut self, text: &str) -> NodeHandle {
        let doc = self.doc().expect("Document dropped");
        let node_id = doc.borrow_mut().create_comment(text);
        NodeHandle::new(node_id, self.doc.clone())
    }

    /// Create a child scope for nested rendering.
    ///
    /// Child scopes are cleaned up when the parent scope is disposed.
    pub fn child_scope(&mut self, parent: &NodeHandle) -> &mut RenderScope {
        let scope = RenderScope::new(
            self.doc().expect("Document dropped"),
            parent.node_id,
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
    pub fn on_cleanup<F: FnOnce() + 'static>(&mut self, f: F) {
        self.cleanups.push(Box::new(f));
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
    pub fn register_handler<F: Fn() + 'static>(&mut self, callback: F) -> crate::events::EventHandlerId {
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
    pub fn register_input_handler<F: Fn(String) + 'static>(&mut self, callback: F) -> crate::events::EventHandlerId {
        crate::events::register_input_handler(crate::events::InputCallback::new(callback))
    }

    /// Dispose of this scope and all child scopes.
    pub fn dispose(mut self) {
        // Dispose child scopes first
        for child in self.children.drain(..) {
            child.dispose();
        }

        // Dispose effects
        self.reactive_scope.dispose();

        // Run cleanup functions
        for cleanup in self.cleanups.drain(..) {
            cleanup();
        }
    }
}

impl Drop for RenderScope {
    fn drop(&mut self) {
        // Effects and cleanups are handled by the Scope and cleanups vec
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
    SetText { node: NodeId, text: String },
    SetAttribute { node: NodeId, name: String, value: String },
    RemoveAttribute { node: NodeId, name: String },
    AppendChild { parent: NodeId, child: NodeId },
    RemoveChild { parent: NodeId, child: NodeId },
    InsertBefore { parent: NodeId, child: NodeId, reference: NodeId },
    ReplaceNode { old: NodeId, new: NodeId },
    SetStyle { node: NodeId, property: String, value: String },
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

/// A mock DOM document for testing.
#[cfg(test)]
pub struct MockDomDocument {
    next_id: usize,
    nodes: std::collections::HashMap<NodeId, MockNode>,
    dirty: Vec<NodeId>,
    root_id: NodeId,
    body_id: NodeId,
}

#[cfg(test)]
struct MockNode {
    #[allow(dead_code)]
    kind: MockNodeKind,
    text: String,
    attributes: std::collections::HashMap<String, String>,
    children: Vec<NodeId>,
    parent: Option<NodeId>,
}

#[cfg(test)]
#[allow(dead_code)]  // tag is stored for debugging but not read
enum MockNodeKind {
    Element(String),
    Text,
    Comment,
}

#[cfg(test)]
impl MockDomDocument {
    pub fn new() -> Self {
        let mut doc = Self {
            next_id: 0,
            nodes: std::collections::HashMap::new(),
            dirty: Vec::new(),
            root_id: NodeId(0),
            body_id: NodeId(0),
        };

        // Create root and body
        doc.root_id = doc.create_element("html");
        doc.body_id = doc.create_element("body");
        doc.append_child(doc.root_id, doc.body_id);

        doc
    }

    fn next_id(&mut self) -> NodeId {
        let id = NodeId(self.next_id);
        self.next_id += 1;
        id
    }
}

#[cfg(test)]
impl DomDocument for MockDomDocument {
    fn create_element(&mut self, tag: &str) -> NodeId {
        let id = self.next_id();
        self.nodes.insert(
            id,
            MockNode {
                kind: MockNodeKind::Element(tag.to_string()),
                text: String::new(),
                attributes: std::collections::HashMap::new(),
                children: Vec::new(),
                parent: None,
            },
        );
        id
    }

    fn create_text(&mut self, text: &str) -> NodeId {
        let id = self.next_id();
        self.nodes.insert(
            id,
            MockNode {
                kind: MockNodeKind::Text,
                text: text.to_string(),
                attributes: std::collections::HashMap::new(),
                children: Vec::new(),
                parent: None,
            },
        );
        id
    }

    fn create_comment(&mut self, text: &str) -> NodeId {
        let id = self.next_id();
        self.nodes.insert(
            id,
            MockNode {
                kind: MockNodeKind::Comment,
                text: text.to_string(),
                attributes: std::collections::HashMap::new(),
                children: Vec::new(),
                parent: None,
            },
        );
        id
    }

    fn append_child(&mut self, parent: NodeId, child: NodeId) {
        if let Some(node) = self.nodes.get_mut(&parent) {
            node.children.push(child);
        }
        if let Some(node) = self.nodes.get_mut(&child) {
            node.parent = Some(parent);
        }
        self.mark_dirty(parent);
    }

    fn remove_child(&mut self, parent: NodeId, child: NodeId) {
        if let Some(node) = self.nodes.get_mut(&parent) {
            node.children.retain(|&c| c != child);
        }
        if let Some(node) = self.nodes.get_mut(&child) {
            node.parent = None;
        }
        self.mark_dirty(parent);
    }

    fn insert_before(&mut self, parent: NodeId, child: NodeId, reference: NodeId) {
        if let Some(node) = self.nodes.get_mut(&parent) {
            if let Some(pos) = node.children.iter().position(|&c| c == reference) {
                node.children.insert(pos, child);
            }
        }
        if let Some(node) = self.nodes.get_mut(&child) {
            node.parent = Some(parent);
        }
        self.mark_dirty(parent);
    }

    fn replace_node(&mut self, old: NodeId, new: NodeId) {
        let parent = self.nodes.get(&old).and_then(|n| n.parent);
        if let Some(parent_id) = parent {
            if let Some(parent_node) = self.nodes.get_mut(&parent_id) {
                if let Some(pos) = parent_node.children.iter().position(|&c| c == old) {
                    parent_node.children[pos] = new;
                }
            }
            if let Some(node) = self.nodes.get_mut(&new) {
                node.parent = Some(parent_id);
            }
            self.mark_dirty(parent_id);
        }
    }

    fn remove_node(&mut self, node: NodeId) {
        let parent = self.nodes.get(&node).and_then(|n| n.parent);
        if let Some(parent_id) = parent {
            self.remove_child(parent_id, node);
        }
    }

    fn set_text_content(&mut self, node: NodeId, text: &str) {
        if let Some(n) = self.nodes.get_mut(&node) {
            n.text = text.to_string();
        }
        self.mark_dirty(node);
    }

    fn set_attribute(&mut self, node: NodeId, name: &str, value: &str) {
        if let Some(n) = self.nodes.get_mut(&node) {
            n.attributes.insert(name.to_string(), value.to_string());
        }
        self.mark_dirty(node);
    }

    fn remove_attribute(&mut self, node: NodeId, name: &str) {
        if let Some(n) = self.nodes.get_mut(&node) {
            n.attributes.remove(name);
        }
        self.mark_dirty(node);
    }

    fn get_attribute(&self, node: NodeId, name: &str) -> Option<String> {
        self.nodes.get(&node)?.attributes.get(name).cloned()
    }

    fn set_style(&mut self, node: NodeId, property: &str, value: &str) {
        if let Some(n) = self.nodes.get_mut(&node) {
            // Simple style handling - just append to style attribute
            let style = n.attributes.entry("style".to_string()).or_default();
            style.push_str(&format!("{}: {};", property, value));
        }
        self.mark_dirty(node);
    }

    fn mark_dirty(&mut self, node: NodeId) {
        if !self.dirty.contains(&node) {
            self.dirty.push(node);
        }
    }

    fn take_dirty_nodes(&mut self) -> Vec<NodeId> {
        std::mem::take(&mut self.dirty)
    }

    fn root(&self) -> NodeId {
        self.root_id
    }

    fn body(&self) -> NodeId {
        self.body_id
    }

    fn query_selector(&self, _selector: &str) -> Option<NodeId> {
        // Mock implementation - just return body
        Some(self.body_id)
    }

    fn query_selector_all(&self, _selector: &str) -> Vec<NodeId> {
        // Mock implementation - return empty vec
        Vec::new()
    }

    fn get_children(&self, node: NodeId) -> Vec<NodeId> {
        self.nodes
            .get(&node)
            .map(|n| n.children.clone())
            .unwrap_or_default()
    }

    fn insert_child(&mut self, parent: NodeId, child: NodeId, index: usize) {
        if let Some(parent_node) = self.nodes.get_mut(&parent) {
            let len = parent_node.children.len();
            let idx = index.min(len);
            parent_node.children.insert(idx, child);
        }
        if let Some(child_node) = self.nodes.get_mut(&child) {
            child_node.parent = Some(parent);
        }
        self.mark_dirty(parent);
    }

    fn parent_node(&self, node: NodeId) -> Option<NodeId> {
        self.nodes.get(&node)?.parent
    }

    fn next_sibling(&self, node: NodeId) -> Option<NodeId> {
        let parent_id = self.nodes.get(&node)?.parent?;
        let parent = self.nodes.get(&parent_id)?;
        let pos = parent.children.iter().position(|&c| c == node)?;
        parent.children.get(pos + 1).copied()
    }

    fn parse_html(&mut self, html: &str) -> Option<NodeId> {
        // Simple mock implementation - just create a text node with the content
        // Real implementations would parse the HTML properly
        let id = self.next_id();
        self.nodes.insert(
            id,
            MockNode {
                kind: MockNodeKind::Text,
                text: html.to_string(),
                attributes: std::collections::HashMap::new(),
                children: Vec::new(),
                parent: None,
            },
        );
        Some(id)
    }

    fn set_scroll_top(&mut self, _node: NodeId, _scroll_top: f64) {
        // Mock implementation - does nothing
    }

    fn set_inner_html(&mut self, _node: NodeId, _html: &str) {
        // Mock implementation - no-op for tests
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_handle_text() {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let mut scope = RenderScope::new(doc.clone(), doc.borrow().body());

        let text = scope.create_text("Hello");
        text.set_text("World");

        // Verify the text was updated
        assert!(doc.borrow_mut().take_dirty_nodes().len() > 0);
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
        assert!(doc.borrow_mut().take_dirty_nodes().len() > 0);
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

        assert_eq!(doc.get_attribute(node_id, "class"), Some("test".to_string()));
    }
}
