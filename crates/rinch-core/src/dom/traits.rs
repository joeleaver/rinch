//! DOM traits: IntoNode, DomDocument, GlyphBounds.

use super::{NodeHandle, NodeId, RenderScope};

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

impl IntoNode for &String {
    #[inline]
    fn into_node(self, scope: &mut RenderScope) -> NodeHandle {
        scope.create_text(self.as_str())
    }
}

impl IntoNode for std::borrow::Cow<'_, str> {
    #[inline]
    fn into_node(self, scope: &mut RenderScope) -> NodeHandle {
        scope.create_text(self.as_ref())
    }
}

impl IntoNode for Vec<NodeHandle> {
    /// Append all NodeHandles as children of a transparent wrapper.
    ///
    /// This enables the `.iter().map(|x| rsx!{...}).collect::<Vec<_>>()` pattern in RSX.
    /// For reactive lists that update when signals change, prefer `for` loops with keyed
    /// reconciliation instead.
    #[inline]
    fn into_node(self, scope: &mut RenderScope) -> NodeHandle {
        let container = scope.create_element("div");
        container.set_attribute("style", "display:contents");
        for child in &self {
            container.append_child(child);
        }
        container
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
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64, bool, char
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
/// implementations (rinch-dom, web-sys, etc.) to be used.
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

    /// Set multiple CSS style properties on an element in a single operation.
    /// More efficient than calling `set_style` multiple times because it only
    /// parses the style string once.
    fn set_styles(&mut self, node: NodeId, properties: &[(&str, &str)]) {
        // Default implementation falls back to calling set_style for each property.
        for &(property, value) in properties {
            self.set_style(node, property, value);
        }
    }

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

    /// Per-line selection rectangles `(x, y, width, height)`, layout-local to the
    /// node's inline layout, covering the byte range `[a, b)`. Used to render a
    /// text selection's highlight. Default: empty (no inline layout / mock).
    fn query_selection_rects(
        &self,
        _node_id: u64,
        _byte_a: usize,
        _byte_b: usize,
    ) -> Vec<(f32, f32, f32, f32)> {
        Vec::new()
    }

    /// Get the tag name of an element node.
    ///
    /// Returns `Some("div")`, `Some("p")`, etc. for elements, `None` for text/comment nodes.
    fn tag_name(&self, _node: NodeId) -> Option<String> {
        None
    }

    /// Get the node type (W3C-style).
    ///
    /// Returns 1 for elements, 3 for text nodes, 8 for comments. Returns `None` if unknown.
    fn node_type(&self, _node: NodeId) -> Option<u16> {
        None
    }

    /// Get the text content of a node.
    ///
    /// For text nodes, returns the text. For elements, returns concatenated descendant text.
    fn text_content(&self, _node: NodeId) -> Option<String> {
        None
    }

    // ── Scroll query API ─────────────────────────────────────────────────

    /// Get the vertical scroll position of an element.
    /// Equivalent to `element.scrollTop` in the web DOM.
    fn scroll_top(&self, _node: NodeId) -> f64 {
        0.0
    }

    /// Get the horizontal scroll position of an element.
    /// Equivalent to `element.scrollLeft` in the web DOM.
    fn scroll_left(&self, _node: NodeId) -> f64 {
        0.0
    }

    /// Set the horizontal scroll position of an element.
    /// Equivalent to `element.scrollLeft = value` in the web DOM.
    fn set_scroll_left(&mut self, _node: NodeId, _scroll_left: f64) {}

    /// Get the total scrollable content height of an element.
    /// Equivalent to `element.scrollHeight` in the web DOM.
    fn scroll_height(&self, _node: NodeId) -> f64 {
        0.0
    }

    /// Get the total scrollable content width of an element.
    /// Equivalent to `element.scrollWidth` in the web DOM.
    fn scroll_width(&self, _node: NodeId) -> f64 {
        0.0
    }

    /// Get the visible content area height (layout height minus padding and border).
    /// Equivalent to `element.clientHeight` in the web DOM.
    fn client_height(&self, _node: NodeId) -> f64 {
        0.0
    }

    /// Get the visible content area width (layout width minus padding and border).
    /// Equivalent to `element.clientWidth` in the web DOM.
    fn client_width(&self, _node: NodeId) -> f64 {
        0.0
    }

    /// Request that the given node be scrolled into view after the next layout.
    ///
    /// The scroll is deferred because layout must be resolved first to know
    /// the element's position relative to its scroll container.
    fn request_scroll_into_view(&mut self, _node: NodeId) {}

    /// Drain all pending scroll-into-view requests.
    ///
    /// Called by the runtime after `resolve_layout()` to apply deferred scrolls.
    fn drain_scroll_into_view_requests(&mut self) -> Vec<NodeId> {
        Vec::new()
    }
}
