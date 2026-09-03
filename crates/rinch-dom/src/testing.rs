//! Testing utilities for rinch-dom.
//!
//! Provides DOM tree serialization, query selectors, and text content extraction
//! for the rinch test harness.

use serde_json::{Value, json};

use crate::node::{NodeKind, NodeTree, RawNodeId};

/// Serialize the DOM tree starting from body as a JSON value (compact — no computed styles).
pub fn serialize_tree(tree: &NodeTree) -> Value {
    serialize_tree_with_options(tree, None, None)
}

/// Serialize the DOM tree with depth and root options.
/// `max_depth` limits recursion (default: unlimited). At the limit, children are
/// replaced with a count so the caller knows there's more to explore.
/// `root_id` starts serialization from a specific node instead of body.
pub fn serialize_tree_with_options(
    tree: &NodeTree,
    max_depth: Option<u32>,
    root_id: Option<RawNodeId>,
) -> Value {
    serialize_tree_full(tree, max_depth, root_id, false)
}

/// Serialize the DOM tree with depth and root options, optionally including
/// each node's computed styles.
///
/// `verbose` is what the visual-regression harness needs: rebuilding the screen
/// as HTML/CSS for a browser to render is only meaningful if the resolved style
/// of every node comes with it. [`serialize_tree_verbose`] is the unscoped
/// shorthand for `verbose = true`.
pub fn serialize_tree_full(
    tree: &NodeTree,
    max_depth: Option<u32>,
    root_id: Option<RawNodeId>,
    verbose: bool,
) -> Value {
    let root = root_id.unwrap_or(tree.body_id);
    serialize_node(tree, root, verbose, max_depth, 0)
}

/// Serialize the DOM tree with full computed styles on every node.
///
/// Delegates to [`serialize_tree_full`], which asks
/// [`crate::paint::painted_border_box`] for each node's on-screen rect — so a
/// `root_id`-scoped tree reports true screen coordinates rather than ones
/// relative to whatever it was scoped to.
pub fn serialize_tree_verbose(tree: &NodeTree) -> Value {
    serialize_tree_full(tree, None, None, true)
}

fn serialize_node(
    tree: &NodeTree,
    id: RawNodeId,
    verbose: bool,
    max_depth: Option<u32>,
    depth: u32,
) -> Value {
    let Some(node) = tree.get(id) else {
        return Value::Null;
    };

    let (node_type, tag, text) = match &node.kind {
        NodeKind::Document => ("document", None, None),
        NodeKind::Element(el) => ("element", Some(el.tag.as_str()), None),
        NodeKind::Text(t) => ("text", None, Some(t.content.as_str())),
        NodeKind::Comment(c) => ("comment", None, Some(c.as_str())),
    };

    let (layout, absolute) = geometry_json(tree, node, id);
    let mut obj = json!({
        "id": node.id,
        "type": node_type,
        "layout": layout,
        "absolute": absolute,
    });

    if node.scroll_offset != (0.0, 0.0) {
        obj["scroll_offset"] = json!({
            "x": node.scroll_offset.0,
            "y": node.scroll_offset.1,
        });
    }

    if let Some(tag) = tag {
        obj["tag"] = Value::String(tag.to_string());
    }
    if let Some(text) = text {
        obj["text"] = Value::String(text.to_string());
    }
    if !node.attributes.is_empty() {
        obj["attributes"] = json!(node.attributes);
    }
    if verbose {
        obj["computed_styles"] = json!(&node.computed_style);
    }

    // At the depth limit, show child count instead of expanding
    if let Some(max) = max_depth {
        if depth >= max && !node.children.is_empty() {
            obj["children_count"] = json!(node.children.len());
            obj["children_ids"] = json!(node.children);
            return obj;
        }
    }

    let children: Vec<Value> = node
        .children
        .iter()
        .map(|&child_id| serialize_node(tree, child_id, verbose, max_depth, depth + 1))
        .collect();

    if !children.is_empty() {
        obj["children"] = Value::Array(children);
    }

    obj
}

/// Find nodes matching a simple selector.
///
/// Supported selectors:
/// - `"div"` — match by tag name
/// - `"[data-rid]"` — match by attribute existence
/// - `"[data-rid=5]"` — match by attribute value
pub fn query_selector(tree: &NodeTree, selector: &str) -> Vec<RawNodeId> {
    let mut results = Vec::new();
    let matcher = parse_selector(selector);
    query_walk(tree, tree.body_id, &matcher, &mut results);
    results
}

enum Matcher {
    Tag(String),
    Class(String),
    AttrExists(String),
    AttrEquals(String, String),
}

fn parse_selector(selector: &str) -> Matcher {
    let s = selector.trim();
    if s.starts_with('[') && s.ends_with(']') {
        let inner = &s[1..s.len() - 1];
        if let Some(eq_pos) = inner.find('=') {
            Matcher::AttrEquals(
                inner[..eq_pos].to_string(),
                inner[eq_pos + 1..]
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string(),
            )
        } else {
            Matcher::AttrExists(inner.to_string())
        }
    } else if let Some(class) = s.strip_prefix('.') {
        Matcher::Class(class.to_string())
    } else {
        Matcher::Tag(s.to_lowercase())
    }
}

fn query_walk(tree: &NodeTree, id: RawNodeId, matcher: &Matcher, results: &mut Vec<RawNodeId>) {
    let Some(node) = tree.get(id) else { return };

    let matches = match matcher {
        Matcher::Tag(tag) => node
            .tag()
            .map(|t| t.to_lowercase() == *tag)
            .unwrap_or(false),
        Matcher::Class(cls) => node
            .attributes
            .get("class")
            .map(|c| c.split_whitespace().any(|w| w == cls))
            .unwrap_or(false),
        Matcher::AttrExists(attr) => node.attributes.contains_key(attr),
        Matcher::AttrEquals(attr, val) => {
            node.attributes.get(attr).map(|v| v == val).unwrap_or(false)
        }
    };

    if matches {
        results.push(id);
    }

    let children: Vec<_> = node.children.clone();
    for child_id in children {
        query_walk(tree, child_id, matcher, results);
    }
}

/// Get all text content in a subtree.
pub fn get_text_content(tree: &NodeTree, id: RawNodeId) -> String {
    let mut buf = String::new();
    collect_text(tree, id, &mut buf);
    buf
}

fn collect_text(tree: &NodeTree, id: RawNodeId, buf: &mut String) {
    let Some(node) = tree.get(id) else { return };
    if let NodeKind::Text(t) = &node.kind {
        buf.push_str(&t.content);
    }
    let children: Vec<_> = node.children.clone();
    for child_id in children {
        collect_text(tree, child_id, buf);
    }
}

/// Get compact summary for a node (no computed styles).
/// Used by query_selector to keep results small.
pub fn get_node_summary(tree: &NodeTree, id: RawNodeId) -> Option<Value> {
    let node = tree.get(id)?;

    let (node_type, tag, text) = match &node.kind {
        NodeKind::Document => ("document", None, None),
        NodeKind::Element(el) => ("element", Some(el.tag.as_str()), None),
        NodeKind::Text(t) => ("text", None, Some(t.content.as_str())),
        NodeKind::Comment(c) => ("comment", None, Some(c.as_str())),
    };

    let (layout, absolute) = geometry_json(tree, node, id);
    let mut obj = json!({
        "id": node.id,
        "type": node_type,
        "layout": layout,
        "absolute": absolute,
        "children": node.children,
        "parent": node.parent,
        "text_content": get_text_content(tree, id),
    });

    if let Some(tag) = tag {
        obj["tag"] = Value::String(tag.to_string());
    }
    if let Some(text) = text {
        obj["text"] = Value::String(text.to_string());
    }
    if !node.attributes.is_empty() {
        obj["attributes"] = json!(node.attributes);
    }

    Some(obj)
}

/// Get detailed info for a single node (includes computed styles).
pub fn get_node_detail(tree: &NodeTree, id: RawNodeId) -> Option<Value> {
    let node = tree.get(id)?;

    let (node_type, tag, text) = match &node.kind {
        NodeKind::Document => ("document", None, None),
        NodeKind::Element(el) => ("element", Some(el.tag.as_str()), None),
        NodeKind::Text(t) => ("text", None, Some(t.content.as_str())),
        NodeKind::Comment(c) => ("comment", None, Some(c.as_str())),
    };

    let (layout, absolute) = geometry_json(tree, node, id);
    let mut obj = json!({
        "id": node.id,
        "type": node_type,
        "layout": layout,
        "absolute": absolute,
        "attributes": node.attributes,
        "children": node.children,
        "parent": node.parent,
        "text_content": get_text_content(tree, id),
    });

    if node.scroll_offset != (0.0, 0.0) {
        obj["scroll_offset"] = json!({
            "x": node.scroll_offset.0,
            "y": node.scroll_offset.1,
        });
    }

    if let Some(tag) = tag {
        obj["tag"] = Value::String(tag.to_string());
    }
    if let Some(text) = text {
        obj["text"] = Value::String(text.to_string());
    }

    // Add computed styles
    obj["computed_styles"] = json!(&node.computed_style);
    // Add display mode
    obj["display_mode"] = Value::String(format!("{:?}", node.display_mode));

    Some(obj)
}

/// Build the two geometry objects reported for a node.
///
/// - `layout` mirrors the node's own `layout` box: x/y are **parent-relative**
///   (exactly `node.layout`, the internal truth — useful for verifying a child's
///   offset within its container).
/// - `absolute` is the on-screen box — the rect the node is *painted* in, from
///   [`crate::paint::painted_border_box`], which is the same walk paint itself
///   follows. **Pass `absolute.x`/`absolute.y` to `click()`** — the `layout` x/y
///   are NOT screen coordinates.
///
/// width/height are usually the same in both, and differ exactly where a CSS
/// transform scales the box: `layout` is the size Taffy gave it, `absolute` is
/// the size it covers on screen. Reporting the layout size there would make
/// "click the centre of `absolute`" miss inside any `scale()` container (#203).
///
/// # Units
///
/// Both boxes are in **logical (CSS) pixels** — `painted_border_box` is asked
/// at scale `1.0` — which is the space `click()`/`mouse_*` take and the space
/// the pointer arrives in on every host (#299), so the two agree at any DPI. A
/// `screenshot()` is in *physical* pixels, so on a HiDPI display a node sits at
/// `absolute * scale_factor` within the image.
fn geometry_json(tree: &NodeTree, node: &crate::node::Node, id: RawNodeId) -> (Value, Value) {
    let r = crate::paint::painted_border_box(tree, id, 1.0);
    (
        json!({
            "x": node.layout.x,
            "y": node.layout.y,
            "width": node.layout.width,
            "height": node.layout.height,
        }),
        json!({
            "x": r.x0 as f32,
            "y": r.y0 as f32,
            "width": r.width() as f32,
            "height": r.height() as f32,
        }),
    )
}

#[cfg(test)]
mod tests {
    use crate::RinchDocument;
    use rinch_core::dom::DomDocument;

    /// The MCP/debug serializers report each node's box twice: `layout` is
    /// parent-relative (== `node.layout`) and `absolute` is the on-screen box.
    /// A nested node offset from its parent must show the two differently, and
    /// a `root_id`-scoped tree must still report true on-screen `absolute`.
    #[test]
    fn layout_is_parent_relative_and_absolute_is_on_screen() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        // Container pushed right by a margin; child pushed in by padding.
        let container = doc.create_element("div");
        doc.set_attribute(
            container,
            "style",
            "margin-left: 50px; padding-left: 30px; width: 200px; height: 100px",
        );
        doc.append_child(body, container);
        let child = doc.create_element("div");
        doc.set_attribute(child, "style", "width: 50px; height: 20px");
        doc.append_child(container, child);

        doc.resolve_layout(800.0, 600.0);

        let summary = super::get_node_summary(&doc.tree, child.0).unwrap();
        // Parent-relative: child sits at the container's content-box left (padding).
        assert_eq!(summary["layout"]["x"], 30.0, "layout.x is parent-relative");
        // On-screen: container margin (50) + its padding (30).
        assert_eq!(summary["absolute"]["x"], 80.0, "absolute.x is on-screen");
        // width/height match in both.
        assert_eq!(summary["layout"]["width"], 50.0);
        assert_eq!(summary["absolute"]["width"], 50.0);

        // get_node_detail agrees.
        let detail = super::get_node_detail(&doc.tree, child.0).unwrap();
        assert_eq!(detail["layout"]["x"], 30.0);
        assert_eq!(detail["absolute"]["x"], 80.0);

        // A tree scoped to the child still reports true on-screen absolute,
        // not an origin-relative one.
        let scoped = super::serialize_tree_with_options(&doc.tree, Some(0), Some(child.0));
        assert_eq!(scoped["layout"]["x"], 30.0, "scoped layout stays relative");
        assert_eq!(
            scoped["absolute"]["x"], 80.0,
            "scoped absolute is still on-screen"
        );
    }

    /// CLAUDE.md tells the reader to click `absolute.x`/`absolute.y`, so the box
    /// has to be the one paint draws — a transform that moved and resized the
    /// node has to move and resize it too, or every documented MCP interaction
    /// inside a `scale()` container misses (#203).
    #[test]
    fn absolute_is_the_painted_box_under_a_transform() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let zoom = doc.create_element("div");
        doc.set_attribute(
            zoom,
            "style",
            "position: absolute; left: 100px; top: 50px; width: 300px; height: 200px; \
             transform: scale(2); transform-origin: 0 0",
        );
        doc.append_child(body, zoom);
        let button = doc.create_element("div");
        doc.set_attribute(
            button,
            "style",
            "position: absolute; left: 20px; top: 30px; width: 100px; height: 10px",
        );
        doc.append_child(zoom, button);

        doc.resolve_layout(800.0, 600.0);

        let summary = super::get_node_summary(&doc.tree, button.0).unwrap();
        // Parent-relative is untouched: still what Taffy stored.
        assert_eq!(summary["layout"]["x"], 20.0);
        assert_eq!(summary["layout"]["width"], 100.0);
        // Layout box (120,80)-(220,90); doubled about (100,50) → (140,110)-(340,130).
        assert_eq!(summary["absolute"]["x"], 140.0);
        assert_eq!(summary["absolute"]["y"], 110.0);
        assert_eq!(
            summary["absolute"]["width"], 200.0,
            "a scaled box covers twice its layout width on screen"
        );
        assert_ne!(
            summary["absolute"]["x"], 120.0,
            "the untransformed parent-chain sum is the box this must NOT report"
        );

        // The centre of `absolute` — what the MCP workflow clicks — is inside
        // the painted box.
        let cx = summary["absolute"]["x"].as_f64().unwrap()
            + summary["absolute"]["width"].as_f64().unwrap() / 2.0;
        assert_eq!(cx, 240.0);

        // The tree serializer agrees with the per-node summary.
        let scoped = super::serialize_tree_with_options(&doc.tree, Some(0), Some(button.0));
        assert_eq!(scoped["absolute"]["x"], 140.0);
        assert_eq!(scoped["absolute"]["width"], 200.0);
    }

    /// A `position: fixed` node reports its viewport box however far the
    /// container behind it has scrolled.
    #[test]
    fn absolute_is_the_viewport_box_for_a_fixed_node() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let page = doc.create_element("div");
        doc.set_attribute(
            page,
            "style",
            "position: absolute; left: 0; top: 0; width: 400px; height: 300px; overflow: auto",
        );
        doc.append_child(body, page);
        let tall = doc.create_element("div");
        doc.set_attribute(tall, "style", "width: 100%; height: 2000px");
        doc.append_child(page, tall);
        let overlay = doc.create_element("div");
        doc.set_attribute(
            overlay,
            "style",
            "position: fixed; left: 40px; top: 60px; width: 120px; height: 50px",
        );
        doc.append_child(page, overlay);
        doc.resolve_layout(800.0, 600.0);
        doc.tree.nodes[page.0].scroll_offset.1 = 150.0;

        let summary = super::get_node_summary(&doc.tree, overlay.0).unwrap();
        assert_eq!(summary["absolute"]["y"], 60.0);
        assert_ne!(
            summary["absolute"]["y"], -90.0,
            "the page's scroll must not be subtracted from a fixed box"
        );

        // And the whole-tree serializer — the `dom_tree` MCP tool — agrees. It
        // used to accumulate offsets top-down with no `position: fixed`
        // exception at all, so it and `get_node` disagreed about the same node.
        let tree = super::serialize_tree(&doc.tree);
        let found = find_by_id(&tree, overlay.0).expect("the overlay is in the tree");
        assert_eq!(found["absolute"]["y"], 60.0);
        assert_ne!(
            found["absolute"]["y"], -90.0,
            "the serialized tree must make the same exception"
        );
    }

    /// Depth-first search for a node's object in a serialized tree.
    fn find_by_id(node: &serde_json::Value, id: usize) -> Option<&serde_json::Value> {
        if node["id"] == id {
            return Some(node);
        }
        node["children"]
            .as_array()?
            .iter()
            .find_map(|c| find_by_id(c, id))
    }
}
