//! Browser-native DOM implementation of the `DomDocument` trait.
//!
//! Instead of painting to a canvas, this implementation creates real browser DOM
//! elements via `web_sys`. The browser handles layout, CSS, text rendering, and
//! painting natively. The reactive system (Signal/Effect) and all components work
//! through NodeHandle -> DomDocument, so everything works automatically.

use std::collections::HashMap;

use rinch_core::dom::{DomDocument, GlyphBounds, NodeId};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

/// A `DomDocument` backed by real browser DOM elements.
///
/// Each rinch `NodeId` maps to a `web_sys::Node` stored in a HashMap.
/// Reverse lookups (browser node -> NodeId) use a `__nid` JS property
/// set on each node via `Reflect::set`.
pub struct WebDocument {
    /// The browser's document object.
    browser_doc: web_sys::Document,
    /// Map from rinch NodeId to browser DOM node.
    nodes: HashMap<usize, web_sys::Node>,
    /// Next available node ID.
    next_id: usize,
    /// The root wrapper element (`<div id="rinch-root">`).
    root_id: NodeId,
    /// The body wrapper element (`<div id="rinch-body">`).
    body_id: NodeId,
}

/// Set the `__nid` JS property on a browser node for reverse lookups.
fn set_nid(node: &web_sys::Node, id: NodeId) {
    let _ = js_sys::Reflect::set(node, &"__nid".into(), &JsValue::from(id.0 as u32));
}

/// Returns true if the tag name is an SVG element.
///
/// SVG elements must be created with `createElementNS` using the SVG namespace,
/// otherwise the browser treats them as unknown HTML elements and won't render them.
fn is_svg_tag(tag: &str) -> bool {
    matches!(
        tag,
        "svg"
            | "path"
            | "circle"
            | "ellipse"
            | "line"
            | "polyline"
            | "polygon"
            | "rect"
            | "g"
            | "defs"
            | "use"
            | "text"
            | "tspan"
            | "clipPath"
            | "mask"
            | "pattern"
            | "image"
            | "foreignObject"
            | "animate"
            | "animateTransform"
            | "set"
            | "stop"
            | "linearGradient"
            | "radialGradient"
            | "filter"
            | "feGaussianBlur"
            | "feOffset"
            | "feMerge"
            | "feMergeNode"
            | "feFlood"
            | "feComposite"
            | "feBlend"
            | "symbol"
            | "marker"
            | "title"
            | "desc"
    )
}

/// Get the `__nid` JS property from a browser node.
fn get_nid(node: &web_sys::Node) -> Option<NodeId> {
    js_sys::Reflect::get(node, &"__nid".into())
        .ok()
        .and_then(|v| v.as_f64())
        .map(|n| NodeId(n as usize))
}

impl WebDocument {
    /// Create a new WebDocument backed by the browser's document.
    ///
    /// Creates a `<div id="rinch-root">` as root and `<div id="rinch-body">`
    /// as body, appending root to `document.body()`.
    pub fn new(browser_doc: web_sys::Document) -> Self {
        let mut doc = Self {
            browser_doc,
            nodes: HashMap::new(),
            next_id: 0,
            root_id: NodeId(0),
            body_id: NodeId(0),
        };

        // Create root element
        let root_el = doc.browser_doc.create_element("div").unwrap();
        root_el.set_id("rinch-root");
        let root_node: web_sys::Node = root_el.into();
        let root_id = doc.alloc_id();
        set_nid(&root_node, root_id);
        doc.nodes.insert(root_id.0, root_node.clone());
        doc.root_id = root_id;

        // Create body element
        let body_el = doc.browser_doc.create_element("div").unwrap();
        body_el.set_id("rinch-body");
        let body_node: web_sys::Node = body_el.into();
        let body_id = doc.alloc_id();
        set_nid(&body_node, body_id);
        doc.nodes.insert(body_id.0, body_node.clone());
        doc.body_id = body_id;

        // Append body to root
        root_node.append_child(&body_node).ok();

        // Append root to the real document.body
        if let Some(real_body) = doc.browser_doc.body() {
            real_body.append_child(&root_node).ok();
        }

        doc
    }

    /// Allocate the next NodeId.
    fn alloc_id(&mut self) -> NodeId {
        let id = NodeId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Get a reference to the browser document.
    pub fn browser_document(&self) -> &web_sys::Document {
        &self.browser_doc
    }

    /// Inject CSS as a `<style>` element in `<head>`.
    pub fn inject_style(&self, css: &str) {
        if let Ok(style) = self.browser_doc.create_element("style") {
            style.set_attribute("data-rinch-theme", "true").ok();
            style.set_text_content(Some(css));
            if let Some(head) = self.browser_doc.head() {
                head.append_child(&style).ok();
            }
        }
    }

    /// Update the theme `<style>` element, or inject one if it doesn't exist.
    pub fn update_theme_style(&self, css: &str) {
        if let Ok(Some(el)) = self.browser_doc.query_selector("[data-rinch-theme]") {
            el.set_text_content(Some(css));
        } else {
            self.inject_style(css);
        }
    }

    /// Recursively walk a DOM subtree and assign `__nid` + register in HashMap.
    /// Used after `set_inner_html` which creates new child nodes without IDs.
    fn register_subtree(&mut self, node: &web_sys::Node) {
        // Assign nid to this node if it doesn't have one
        if get_nid(node).is_none() {
            let id = self.alloc_id();
            set_nid(node, id);
            self.nodes.insert(id.0, node.clone());
        }
        // Recurse into children
        let children = node.child_nodes();
        for i in 0..children.length() {
            if let Some(child) = children.item(i) {
                self.register_subtree(&child);
            }
        }
    }
}

/// Walk a DOM subtree depth-first to find the text node containing the given UTF-8 byte offset.
/// Returns `(text_node, utf16_offset_within_node)`.
fn find_text_node_at_byte_offset(
    node: &web_sys::Node,
    byte_offset: usize,
) -> Option<(web_sys::Node, u32)> {
    let mut remaining = byte_offset;
    find_text_node_recursive(node, &mut remaining)
}

fn find_text_node_recursive(
    node: &web_sys::Node,
    remaining: &mut usize,
) -> Option<(web_sys::Node, u32)> {
    // If this is a text node, check if the offset falls within it
    if node.node_type() == web_sys::Node::TEXT_NODE {
        let text = node.text_content().unwrap_or_default();
        let byte_len = text.len();
        if *remaining <= byte_len {
            // Convert remaining UTF-8 byte offset to UTF-16 code unit offset
            let utf16_offset = utf8_byte_to_utf16_offset(&text, *remaining);
            return Some((node.clone(), utf16_offset as u32));
        }
        *remaining -= byte_len;
        return None;
    }

    // Recurse into children
    let children = node.child_nodes();
    for i in 0..children.length() {
        if let Some(child) = children.item(i)
            && let Some(result) = find_text_node_recursive(&child, remaining) {
                return Some(result);
            }
    }
    None
}

/// Convert a UTF-8 byte offset to a UTF-16 code unit offset within a string.
fn utf8_byte_to_utf16_offset(text: &str, byte_offset: usize) -> usize {
    let mut utf16_offset = 0;
    for (i, ch) in text.char_indices() {
        if i >= byte_offset {
            break;
        }
        utf16_offset += ch.len_utf16();
    }
    utf16_offset
}

impl DomDocument for WebDocument {
    fn create_element(&mut self, tag: &str) -> NodeId {
        let el = if is_svg_tag(tag) {
            self.browser_doc
                .create_element_ns(Some("http://www.w3.org/2000/svg"), tag)
                .unwrap()
        } else {
            self.browser_doc.create_element(tag).unwrap()
        };
        let node: web_sys::Node = el.into();
        let id = self.alloc_id();
        set_nid(&node, id);
        self.nodes.insert(id.0, node);
        id
    }

    fn create_text(&mut self, text: &str) -> NodeId {
        let text_node = self.browser_doc.create_text_node(text);
        let node: web_sys::Node = text_node.into();
        let id = self.alloc_id();
        set_nid(&node, id);
        self.nodes.insert(id.0, node);
        id
    }

    fn create_comment(&mut self, text: &str) -> NodeId {
        let comment = self.browser_doc.create_comment(text);
        let node: web_sys::Node = comment.into();
        let id = self.alloc_id();
        set_nid(&node, id);
        self.nodes.insert(id.0, node);
        id
    }

    fn append_child(&mut self, parent: NodeId, child: NodeId) {
        if let (Some(p), Some(c)) = (self.nodes.get(&parent.0), self.nodes.get(&child.0)) {
            p.append_child(c).ok();
        }
    }

    fn remove_child(&mut self, parent: NodeId, child: NodeId) {
        if let (Some(p), Some(c)) = (self.nodes.get(&parent.0), self.nodes.get(&child.0)) {
            p.remove_child(c).ok();
        }
    }

    fn insert_before(&mut self, parent: NodeId, child: NodeId, reference: NodeId) {
        if let (Some(p), Some(c), Some(r)) = (
            self.nodes.get(&parent.0),
            self.nodes.get(&child.0),
            self.nodes.get(&reference.0),
        ) {
            p.insert_before(c, Some(r)).ok();
        }
    }

    fn replace_node(&mut self, old: NodeId, new: NodeId) {
        if let (Some(old_node), Some(new_node)) =
            (self.nodes.get(&old.0), self.nodes.get(&new.0))
            && let Some(parent) = old_node.parent_node() {
                parent.replace_child(new_node, old_node).ok();
            }
    }

    fn remove_node(&mut self, node: NodeId) {
        if let Some(n) = self.nodes.get(&node.0)
            && let Some(parent) = n.parent_node() {
                parent.remove_child(n).ok();
            }
    }

    fn set_text_content(&mut self, node: NodeId, text: &str) {
        if let Some(n) = self.nodes.get(&node.0) {
            n.set_text_content(Some(text));
        }
    }

    fn set_attribute(&mut self, node: NodeId, name: &str, value: &str) {
        if let Some(n) = self.nodes.get(&node.0)
            && let Ok(el) = n.clone().dyn_into::<web_sys::Element>() {
                el.set_attribute(name, value).ok();
            }
    }

    fn remove_attribute(&mut self, node: NodeId, name: &str) {
        if let Some(n) = self.nodes.get(&node.0)
            && let Ok(el) = n.clone().dyn_into::<web_sys::Element>() {
                el.remove_attribute(name).ok();
            }
    }

    fn get_attribute(&self, node: NodeId, name: &str) -> Option<String> {
        let n = self.nodes.get(&node.0)?;
        let el: web_sys::Element = n.clone().dyn_into().ok()?;
        el.get_attribute(name)
    }

    fn set_style(&mut self, node: NodeId, property: &str, value: &str) {
        if let Some(n) = self.nodes.get(&node.0)
            && let Ok(el) = n.clone().dyn_into::<web_sys::HtmlElement>() {
                el.style().set_property(property, value).ok();
            }
    }

    fn mark_dirty(&mut self, _node: NodeId) {
        // No-op: browser handles reflow automatically.
    }

    fn take_dirty_nodes(&mut self) -> Vec<NodeId> {
        // No-op: browser handles reflow automatically.
        Vec::new()
    }

    fn root(&self) -> NodeId {
        self.root_id
    }

    fn body(&self) -> NodeId {
        self.body_id
    }

    fn query_selector(&self, selector: &str) -> Option<NodeId> {
        let el = self.browser_doc.query_selector(selector).ok()??;
        let node: web_sys::Node = el.into();
        get_nid(&node)
    }

    fn query_selector_all(&self, selector: &str) -> Vec<NodeId> {
        let mut result = Vec::new();
        if let Ok(node_list) = self.browser_doc.query_selector_all(selector) {
            for i in 0..node_list.length() {
                if let Some(node) = node_list.item(i)
                    && let Some(nid) = get_nid(&node) {
                        result.push(nid);
                    }
            }
        }
        result
    }

    fn get_children(&self, node: NodeId) -> Vec<NodeId> {
        let mut result = Vec::new();
        if let Some(n) = self.nodes.get(&node.0) {
            let children = n.child_nodes();
            for i in 0..children.length() {
                if let Some(child) = children.item(i)
                    && let Some(nid) = get_nid(&child) {
                        result.push(nid);
                    }
            }
        }
        result
    }

    fn insert_child(&mut self, parent: NodeId, child: NodeId, index: usize) {
        if let (Some(p), Some(c)) = (self.nodes.get(&parent.0), self.nodes.get(&child.0)) {
            let children = p.child_nodes();
            if index < children.length() as usize {
                if let Some(ref_node) = children.item(index as u32) {
                    p.insert_before(c, Some(&ref_node)).ok();
                } else {
                    p.append_child(c).ok();
                }
            } else {
                p.append_child(c).ok();
            }
        }
    }

    fn parent_node(&self, node: NodeId) -> Option<NodeId> {
        let n = self.nodes.get(&node.0)?;
        let parent = n.parent_node()?;
        get_nid(&parent)
    }

    fn next_sibling(&self, node: NodeId) -> Option<NodeId> {
        let n = self.nodes.get(&node.0)?;
        let sibling = n.next_sibling()?;
        get_nid(&sibling)
    }

    fn parse_html(&mut self, html: &str) -> Option<NodeId> {
        let temp = self.browser_doc.create_element("div").ok()?;
        temp.set_inner_html(html);
        let first_child = temp.first_child()?;
        // Register the subtree
        self.register_subtree(&first_child);
        get_nid(&first_child)
    }

    fn set_scroll_top(&mut self, node: NodeId, scroll_top: f64) {
        if let Some(n) = self.nodes.get(&node.0)
            && let Ok(el) = n.clone().dyn_into::<web_sys::Element>() {
                el.set_scroll_top(scroll_top as i32);
            }
    }

    fn set_inner_html(&mut self, node: NodeId, html: &str) {
        if let Some(n) = self.nodes.get(&node.0)
            && let Ok(el) = n.clone().dyn_into::<web_sys::Element>() {
                el.set_inner_html(html);
                // Walk all new child nodes and register them
                let children = el.child_nodes();
                for i in 0..children.length() {
                    if let Some(child) = children.item(i) {
                        self.register_subtree(&child);
                    }
                }
            }
    }

    fn query_caret_position(&self, node_id: u64, byte_offset: usize) -> Option<(f32, f32)> {
        let n = self.nodes.get(&(node_id as usize))?;
        let (text_node, utf16_offset) = find_text_node_at_byte_offset(n, byte_offset)?;
        let range = self.browser_doc.create_range().ok()?;
        range.set_start(&text_node, utf16_offset).ok()?;
        range.set_end(&text_node, utf16_offset).ok()?;
        let rect = range.get_bounding_client_rect();
        // Get the block element's rect to compute relative coordinates
        let el: web_sys::Element = n.clone().dyn_into().ok()?;
        let block_rect = el.get_bounding_client_rect();
        Some((
            (rect.x() - block_rect.x()) as f32,
            (rect.y() - block_rect.y()) as f32,
        ))
    }

    fn query_glyph_bounds(&self, node_id: u64, byte_offset: usize) -> Option<GlyphBounds> {
        let n = self.nodes.get(&(node_id as usize))?;
        let (text_node, utf16_offset) = find_text_node_at_byte_offset(n, byte_offset)?;
        let text_content = text_node.text_content().unwrap_or_default();
        let text_utf16_len: usize = text_content.encode_utf16().count();

        let range = self.browser_doc.create_range().ok()?;
        // If we're at the end of text, use the last character's bounds
        if utf16_offset as usize >= text_utf16_len {
            if text_utf16_len == 0 {
                return None;
            }
            range
                .set_start(&text_node, (text_utf16_len - 1) as u32)
                .ok()?;
            range.set_end(&text_node, text_utf16_len as u32).ok()?;
        } else {
            range.set_start(&text_node, utf16_offset).ok()?;
            range.set_end(&text_node, utf16_offset + 1).ok()?;
        }

        let rect = range.get_bounding_client_rect();
        let el: web_sys::Element = n.clone().dyn_into().ok()?;
        let block_rect = el.get_bounding_client_rect();

        Some(GlyphBounds {
            x: (rect.x() - block_rect.x()) as f32,
            y: (rect.y() - block_rect.y()) as f32,
            width: rect.width() as f32,
            height: rect.height() as f32,
        })
    }

    fn focus_element(&mut self, node_id: NodeId) {
        if let Some(n) = self.nodes.get(&node_id.0)
            && let Ok(el) = n.clone().dyn_into::<web_sys::HtmlElement>() {
                el.focus().ok();
            }
    }

    fn resolve_layout(&mut self, _width: f32, _height: f32) {
        // No-op: browser handles layout natively.
    }

    fn query_node_layout(&self, node_id: u64) -> Option<(f32, f32, f32, f32)> {
        let n = self.nodes.get(&(node_id as usize))?;
        // Try HtmlElement.offset* for parent-relative coordinates (needed by editor)
        if let Ok(el) = n.clone().dyn_into::<web_sys::HtmlElement>() {
            Some((
                el.offset_left() as f32,
                el.offset_top() as f32,
                el.offset_width() as f32,
                el.offset_height() as f32,
            ))
        } else {
            // Fallback for non-HTML elements (SVG, etc.)
            let el: web_sys::Element = n.clone().dyn_into().ok()?;
            let rect = el.get_bounding_client_rect();
            Some((
                rect.x() as f32,
                rect.y() as f32,
                rect.width() as f32,
                rect.height() as f32,
            ))
        }
    }
}
