//! Browser-native DOM implementation of the `DomDocument` trait.
//!
//! Instead of painting to a canvas, this implementation creates real browser DOM
//! elements via `web_sys`. The browser handles layout, CSS, text rendering, and
//! painting natively. The reactive system (Signal/Effect) and all components work
//! through NodeHandle -> DomDocument, so everything works automatically.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use rinch_core::dom::{DomDocument, GlyphBounds, NodeId};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

/// Process-global NodeId counter. Shared across all `WebDocument`s so that
/// multiple coexisting roots (islands) never reuse a NodeId — keeping each
/// node's `__nid` unambiguous even when several documents share the page.
static NEXT_NODE_ID: AtomicUsize = AtomicUsize::new(0);

/// A `DomDocument` backed by real browser DOM elements.
///
/// Each rinch `NodeId` maps to a `web_sys::Node` stored in a HashMap.
/// Reverse lookups (browser node -> NodeId) use a `__nid` JS property
/// set on each node via `Reflect::set`.
pub struct WebDocument {
    /// Process-unique document identity (see [`DomDocument::doc_key`]).
    doc_key: u64,
    /// The browser's document object.
    browser_doc: web_sys::Document,
    /// Map from rinch NodeId to browser DOM node.
    nodes: HashMap<usize, web_sys::Node>,
    /// The root wrapper element (`<div id="rinch-root">`), or the adopted host
    /// element for an island mount (see [`WebDocument::new_into`]).
    root_id: NodeId,
    /// The body wrapper element (`<div id="rinch-body">`).
    body_id: NodeId,
}

thread_local! {
    /// Reverse map (`NodeId.0` → browser node) populated for every node `set_nid`
    /// tags. Lets non-`DomDocument` code — the editor input glue — resolve a rinch
    /// node id back to its DOM node for caret/selection geometry. Entries are not
    /// eagerly pruned (node *creation* is structural, so growth is bounded);
    /// [`node_by_nid`] returns `None` for a node detached from the document.
    static NODE_REGISTRY: std::cell::RefCell<HashMap<usize, web_sys::Node>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Resolve a rinch `NodeId.0` to its live browser node — the editor glue uses this to
/// read caret/selection geometry for a model position. `None` if unknown or detached.
pub(crate) fn node_by_nid(nid: usize) -> Option<web_sys::Node> {
    NODE_REGISTRY
        .with(|m| m.borrow().get(&nid).cloned())
        .filter(|n| n.is_connected())
}

/// Set the `__nid` JS property on a browser node for reverse lookups.
fn set_nid(node: &web_sys::Node, id: NodeId) {
    let _ = js_sys::Reflect::set(node, &"__nid".into(), &JsValue::from(id.0 as u32));
    NODE_REGISTRY.with(|m| m.borrow_mut().insert(id.0, node.clone()));
}

/// Coerces a scroll offset to whatever `Element::set_scroll_{top,left}` expects.
///
/// web-sys types these setters as `i32` on stable, but as `f64` under
/// `--cfg=web_sys_unstable_apis` (which an OPFS storage backend needs for
/// `FileSystemSyncAccessHandle`). Converting here keeps the call sites clean and
/// preserves sub-pixel scroll offsets on the unstable path.
#[cfg(web_sys_unstable_apis)]
fn scroll_px(v: f64) -> f64 {
    v
}

#[cfg(not(web_sys_unstable_apis))]
fn scroll_px(v: f64) -> i32 {
    v as i32
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

/// Mirror an attribute onto the live DOM *property* for the form-control
/// attributes that browsers reflect asymmetrically (`value`, `checked`,
/// `selected`, `indeterminate`).
///
/// For `<input>`/`<textarea>`/`<select>`/`<option>` the attribute only seeds the
/// control's *default* value. Once the user edits the control it becomes "dirty"
/// and the browser no longer mirrors the attribute into the property, so a
/// reactive `value:`/`checked:` binding that writes the attribute would silently
/// stop updating the displayed value. Writing the property keeps the control in
/// sync with the signal even after it has been typed into / toggled (issue #100).
///
/// The property is only written when it actually differs from the requested
/// value. For `value` that avoids resetting the caret to the end on the common
/// echo path (user types → `oninput` → signal → effect re-writes the same
/// string); for the boolean properties it just skips redundant DOM writes.
fn sync_reflected_property(node: &web_sys::Node, name: &str, value: &str) {
    match name {
        "value" => {
            // Each control type exposes value()/set_value(); write only when it
            // differs (the `!= value` guard avoids a caret reset on the echo path).
            if let Some(input) = node.dyn_ref::<web_sys::HtmlInputElement>() {
                // A file input's value setter throws `InvalidStateError` for any
                // non-empty string (the browser forbids setting a file path
                // programmatically), and web-sys' `set_value` is not a `catch`
                // binding — the exception would unwind uncaught across the wasm
                // boundary. Only `""` is permitted (it clears the selection), so
                // skip any other value on a file input.
                let settable = input.type_() != "file" || value.is_empty();
                if settable && input.value() != value {
                    input.set_value(value);
                }
            } else if let Some(textarea) = node.dyn_ref::<web_sys::HtmlTextAreaElement>()
                && textarea.value() != value
            {
                textarea.set_value(value);
            } else if let Some(select) = node.dyn_ref::<web_sys::HtmlSelectElement>()
                && select.value() != value
            {
                select.set_value(value);
            }
        }
        "checked" => {
            if let Some(input) = node.dyn_ref::<web_sys::HtmlInputElement>() {
                let on = attr_is_truthy(value);
                if input.checked() != on {
                    input.set_checked(on);
                }
            }
        }
        "indeterminate" => {
            if let Some(input) = node.dyn_ref::<web_sys::HtmlInputElement>() {
                let on = attr_is_truthy(value);
                if input.indeterminate() != on {
                    input.set_indeterminate(on);
                }
            }
        }
        "selected" => {
            if let Some(option) = node.dyn_ref::<web_sys::HtmlOptionElement>() {
                let on = attr_is_truthy(value);
                if option.selected() != on {
                    option.set_selected(on);
                }
            }
        }
        _ => {}
    }
}

/// Truthiness for HTML boolean-ish attributes as rinch emits them.
///
/// rinch writes these via two conventions: the `rsx!` macro stringifies a `bool`
/// closure to `"true"`/`"false"`, while components set the bare presence form
/// (`""`, matching HTML where a present boolean attribute is true regardless of
/// value). Treat everything as "on" except the explicit falsey strings so both
/// conventions round-trip — in particular an empty string means *present* (true).
fn attr_is_truthy(value: &str) -> bool {
    !matches!(value, "false" | "0")
}

/// Get the `__nid` JS property from a browser node.
pub(crate) fn get_nid(node: &web_sys::Node) -> Option<NodeId> {
    js_sys::Reflect::get(node, &"__nid".into())
        .ok()
        .and_then(|v| v.as_f64())
        .map(|n| NodeId(n as usize))
}

/// Update (or inject) the page-global theme `<style data-rinch-theme>` in the
/// document head, independent of any one `WebDocument`.
///
/// Theme CSS is page-global (one `<style>` shared by every root), so the
/// signal-change hook uses this to keep it current regardless of which root
/// triggered the change. Idempotent: updates the existing element if present.
pub fn update_theme_style_global(css: &str) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    if let Ok(Some(el)) = doc.query_selector("[data-rinch-theme]") {
        el.set_text_content(Some(css));
    } else if let Ok(style) = doc.create_element("style") {
        style.set_attribute("data-rinch-theme", "true").ok();
        style.set_text_content(Some(css));
        if let Some(head) = doc.head() {
            head.append_child(&style).ok();
        }
    }
}

impl WebDocument {
    /// Create a new WebDocument backed by the browser's document.
    ///
    /// Creates a `<div id="rinch-root">` as root and `<div id="rinch-body">`
    /// as body, appending root to `document.body()`.
    pub fn new(browser_doc: web_sys::Document) -> Self {
        let mut doc = Self {
            doc_key: rinch_core::dom::next_doc_key(),
            browser_doc,
            nodes: HashMap::new(),
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

    /// Create a `WebDocument` that mounts into an **existing** browser element
    /// (an "island" host), instead of creating `#rinch-root`/`#rinch-body` and
    /// appending to `document.body`.
    ///
    /// The `host` element is adopted as both the document root and body, so the
    /// component tree is appended directly inside it. No fixed ids are set, so
    /// any number of islands can coexist on one page without id collisions.
    pub fn new_into(browser_doc: web_sys::Document, host: web_sys::Element) -> Self {
        let mut doc = Self {
            doc_key: rinch_core::dom::next_doc_key(),
            browser_doc,
            nodes: HashMap::new(),
            root_id: NodeId(0),
            body_id: NodeId(0),
        };

        let host_node: web_sys::Node = host.into();
        let id = doc.alloc_id();
        set_nid(&host_node, id);
        doc.nodes.insert(id.0, host_node);
        // The host is both root and body: content mounts directly inside it.
        doc.root_id = id;
        doc.body_id = id;

        doc
    }

    /// Allocate the next (process-globally unique) NodeId.
    fn alloc_id(&mut self) -> NodeId {
        NodeId(NEXT_NODE_ID.fetch_add(1, Ordering::Relaxed))
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
pub(crate) fn find_text_node_at_byte_offset(
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
            && let Some(result) = find_text_node_recursive(&child, remaining)
        {
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
    fn doc_key(&self) -> u64 {
        self.doc_key
    }

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
        if let (Some(old_node), Some(new_node)) = (self.nodes.get(&old.0), self.nodes.get(&new.0))
            && let Some(parent) = old_node.parent_node()
        {
            parent.replace_child(new_node, old_node).ok();
        }
    }

    fn remove_node(&mut self, node: NodeId) {
        if let Some(n) = self.nodes.get(&node.0)
            && let Some(parent) = n.parent_node()
        {
            parent.remove_child(n).ok();
        }
    }

    fn set_text_content(&mut self, node: NodeId, text: &str) {
        if let Some(n) = self.nodes.get(&node.0) {
            n.set_text_content(Some(text));
            // A <textarea>'s child text is its *default value*, which the browser
            // stops mirroring onto the live `value` property once the control is
            // user-edited — the same dirty-flag asymmetry `set_attribute("value")`
            // handles for other controls (issue #100). So when a textarea's text
            // changes — set directly on it, or (the reactive `textarea { {|| … } }`
            // case) on one of its child text nodes — re-sync its value property.
            let textarea = if let Some(ta) = n.dyn_ref::<web_sys::HtmlTextAreaElement>() {
                Some(ta.clone())
            } else if let Some(parent) = n.parent_node() {
                parent.dyn_into::<web_sys::HtmlTextAreaElement>().ok()
            } else {
                None
            };
            if let Some(ta) = textarea {
                let content = ta.text_content().unwrap_or_default();
                if ta.value() != content {
                    ta.set_value(&content);
                }
            }
        }
    }

    fn set_attribute(&mut self, node: NodeId, name: &str, value: &str) {
        if let Some(n) = self.nodes.get(&node.0) {
            if let Ok(el) = n.clone().dyn_into::<web_sys::Element>() {
                match name {
                    // Boolean content attributes follow HTML *presence* semantics:
                    // a present attribute is true regardless of its string value.
                    // The rsx macro stringifies a `bool` closure to `"true"`/
                    // `"false"`, so writing the literal string would leave
                    // `checked="false"` *present* — meaning `defaultChecked` stays
                    // true, `[checked]` selectors match, and `form.reset()` would
                    // re-check it, all contradicting the property synced below.
                    // Mirror truthiness onto presence instead.
                    "checked" | "selected" => {
                        if attr_is_truthy(value) {
                            el.set_attribute(name, "").ok();
                        } else {
                            el.remove_attribute(name).ok();
                        }
                    }
                    // `indeterminate` is a property-only flag with no HTML content
                    // attribute; don't materialize a bogus one (the property is
                    // synced below).
                    "indeterminate" => {}
                    _ => {
                        el.set_attribute(name, value).ok();
                    }
                }
            }
            // A handful of attributes are reflected *asymmetrically* onto a live
            // DOM property on form controls (`value`, `checked`, `selected`,
            // `indeterminate`). Writing the attribute alone only sets the
            // control's *default*: the moment the user edits the control it goes
            // "dirty" and the browser stops mirroring the attribute into the
            // property. A reactive `value:`/`checked:` binding would then silently
            // stop updating what is displayed. Mirror the property too so
            // signal-driven updates keep working after the control has been typed
            // into / toggled (issue #100).
            sync_reflected_property(n, name, value);
        }
    }

    fn remove_attribute(&mut self, node: NodeId, name: &str) {
        if let Some(n) = self.nodes.get(&node.0) {
            if let Ok(el) = n.clone().dyn_into::<web_sys::Element>() {
                el.remove_attribute(name).ok();
            }
            // Keep the reflected property in sync when the attribute is removed,
            // otherwise a dirtied control keeps showing the stale property (#100).
            match name {
                "value" => sync_reflected_property(n, "value", ""),
                "checked" | "selected" | "indeterminate" => {
                    sync_reflected_property(n, name, "false")
                }
                _ => {}
            }
        }
    }

    fn get_attribute(&self, node: NodeId, name: &str) -> Option<String> {
        let n = self.nodes.get(&node.0)?;
        let el: web_sys::Element = n.clone().dyn_into().ok()?;
        el.get_attribute(name)
    }

    fn set_style(&mut self, node: NodeId, property: &str, value: &str) {
        if let Some(n) = self.nodes.get(&node.0)
            && let Ok(el) = n.clone().dyn_into::<web_sys::HtmlElement>()
        {
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
                    && let Some(nid) = get_nid(&node)
                {
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
                    && let Some(nid) = get_nid(&child)
                {
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
            && let Ok(el) = n.clone().dyn_into::<web_sys::Element>()
        {
            el.set_scroll_top(scroll_px(scroll_top));
        }
    }

    fn scroll_top(&self, node: NodeId) -> f64 {
        self.nodes
            .get(&node.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
            .map(|el| el.scroll_top() as f64)
            .unwrap_or(0.0)
    }

    fn scroll_left(&self, node: NodeId) -> f64 {
        self.nodes
            .get(&node.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
            .map(|el| el.scroll_left() as f64)
            .unwrap_or(0.0)
    }

    fn set_scroll_left(&mut self, node: NodeId, scroll_left: f64) {
        if let Some(n) = self.nodes.get(&node.0)
            && let Ok(el) = n.clone().dyn_into::<web_sys::Element>()
        {
            el.set_scroll_left(scroll_px(scroll_left));
        }
    }

    fn scroll_height(&self, node: NodeId) -> f64 {
        self.nodes
            .get(&node.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
            .map(|el| el.scroll_height() as f64)
            .unwrap_or(0.0)
    }

    fn scroll_width(&self, node: NodeId) -> f64 {
        self.nodes
            .get(&node.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
            .map(|el| el.scroll_width() as f64)
            .unwrap_or(0.0)
    }

    fn client_height(&self, node: NodeId) -> f64 {
        self.nodes
            .get(&node.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
            .map(|el| el.client_height() as f64)
            .unwrap_or(0.0)
    }

    fn client_width(&self, node: NodeId) -> f64 {
        self.nodes
            .get(&node.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
            .map(|el| el.client_width() as f64)
            .unwrap_or(0.0)
    }

    fn set_inner_html(&mut self, node: NodeId, html: &str) {
        if let Some(n) = self.nodes.get(&node.0)
            && let Ok(el) = n.clone().dyn_into::<web_sys::Element>()
        {
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
            && let Ok(el) = n.clone().dyn_into::<web_sys::HtmlElement>()
        {
            el.focus().ok();
        }
    }

    fn resolve_layout(&mut self, _width: f32, _height: f32) {
        // No-op: browser handles layout natively.
    }

    fn query_node_layout(&self, node_id: u64) -> Option<(f32, f32, f32, f32)> {
        let n = self.nodes.get(&(node_id as usize))?;
        let el: web_sys::Element = n.clone().dyn_into().ok()?;
        let rect = el.get_bounding_client_rect();
        // The trait contract is **immediate-parent-relative** `(x, y)` (the desktop
        // Taffy backend returns this, and the editor's `block_offset_in_container`
        // sums it up the ancestor chain). `offsetLeft/offsetTop` would be
        // *offsetParent*-relative, which double-counts for a nested block (a
        // paragraph inside a blockquote/list, whose offsetParent is the editor
        // container, not its immediate parent). Use border-box differences instead.
        let (px, py) = match el.parent_element() {
            Some(parent) => {
                let pr = parent.get_bounding_client_rect();
                (pr.x(), pr.y())
            }
            None => (0.0, 0.0),
        };
        Some((
            (rect.x() - px) as f32,
            (rect.y() - py) as f32,
            rect.width() as f32,
            rect.height() as f32,
        ))
    }

    fn content_origin_inset(&self, node_id: u64) -> (f32, f32) {
        // clientLeft/clientTop = the node's left/top border width. The editor's overlay
        // container is position:relative with a border, so an absolutely-positioned
        // overlay anchors to its padding box; the block offset (computed from border-box
        // getBoundingClientRect differences) must subtract this to land on the glyphs.
        let Some(n) = self.nodes.get(&(node_id as usize)) else {
            return (0.0, 0.0);
        };
        match n.clone().dyn_into::<web_sys::Element>() {
            Ok(el) => (el.client_left() as f32, el.client_top() as f32),
            Err(_) => (0.0, 0.0),
        }
    }

    fn node_font(&self, node_id: u64) -> Option<rinch_core::dom::NodeFont> {
        // `getComputedStyle` so the IME preedit overlay (an absolutely-positioned span
        // in the editor container, which inherits only the container's default font)
        // can match the block it composes into — e.g. a 32px heading, not 16px body.
        let el: web_sys::Element = self
            .nodes
            .get(&(node_id as usize))?
            .clone()
            .dyn_into()
            .ok()?;
        let cs = web_sys::window()?.get_computed_style(&el).ok().flatten()?;
        let get = |p: &str| cs.get_property_value(p).unwrap_or_default();
        Some(rinch_core::dom::NodeFont {
            family: get("font-family"),
            size: get("font-size"),
            weight: get("font-weight"),
            style: get("font-style"),
        })
    }

    fn query_selection_rects(
        &self,
        node_id: u64,
        byte_a: usize,
        byte_b: usize,
    ) -> Vec<(f32, f32, f32, f32)> {
        // Per-line selection rectangles for the flat UTF-8 byte range `[a, b)` within
        // the block's concatenated inline text, returned **block-local** (relative to
        // the block element's top-left) — the editor view adds the block's container
        // offset itself. Mirrors `query_caret_position`'s coordinate convention.
        if byte_a >= byte_b {
            return Vec::new();
        }
        let Some(n) = self.nodes.get(&(node_id as usize)) else {
            return Vec::new();
        };
        let resolve = || -> Option<web_sys::DomRectList> {
            let (start_node, start_off) = find_text_node_at_byte_offset(n, byte_a)?;
            let (end_node, end_off) = find_text_node_at_byte_offset(n, byte_b)?;
            let range = self.browser_doc.create_range().ok()?;
            range.set_start(&start_node, start_off).ok()?;
            range.set_end(&end_node, end_off).ok()?;
            range.get_client_rects()
        };
        let Some(rects) = resolve() else {
            return Vec::new();
        };
        let Ok(el) = n.clone().dyn_into::<web_sys::Element>() else {
            return Vec::new();
        };
        let block = el.get_bounding_client_rect();
        let (bx, by) = (block.x(), block.y());
        let mut out = Vec::with_capacity(rects.length() as usize);
        for i in 0..rects.length() {
            if let Some(r) = rects.get(i) {
                // Skip zero-area rects (collapsed line fragments) the browser can emit.
                if r.width() <= 0.0 || r.height() <= 0.0 {
                    continue;
                }
                out.push((
                    (r.x() - bx) as f32,
                    (r.y() - by) as f32,
                    r.width() as f32,
                    r.height() as f32,
                ));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attr_truthiness_covers_both_emit_conventions() {
        // The rsx! macro stringifies bool closures.
        assert!(attr_is_truthy("true"));
        assert!(!attr_is_truthy("false"));
        // Components use the HTML presence form: an empty string is *present*,
        // hence true (e.g. `input.set_attribute("checked", "")`).
        assert!(attr_is_truthy(""));
        // Other falsey spellings.
        assert!(!attr_is_truthy("0"));
        // Anything else (a stray value on a boolean attribute) is still "on",
        // matching HTML's "present attribute = true" rule.
        assert!(attr_is_truthy("on"));
        assert!(attr_is_truthy("checked"));
    }
}
