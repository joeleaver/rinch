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
/// of every node comes with it. [`serialize_tree_verbose`] emits those styles
/// but ignores `max_depth`/`root_id`, so it cannot answer a scoped request.
pub fn serialize_tree_full(
    tree: &NodeTree,
    max_depth: Option<u32>,
    root_id: Option<RawNodeId>,
    verbose: bool,
) -> Value {
    let root = root_id.unwrap_or(tree.body_id);
    // Seed the accumulator with the root's own screen position (minus its own
    // layout, which serialize_node re-adds) so `absolute` is true on-screen
    // coordinates even when scoped to a subtree via `root_id`.
    let (ox, oy) = subtree_offset(tree, root);
    serialize_node(tree, root, ox, oy, verbose, max_depth, 0)
}

/// The offset to seed `serialize_node` with so a subtree root's reported
/// `absolute` equals its real on-screen position: `abs(root) - root.layout`.
fn subtree_offset(tree: &NodeTree, root: RawNodeId) -> (f32, f32) {
    let (ax, ay) = compute_absolute_position(tree, root);
    let (lx, ly) = tree
        .get(root)
        .map(|n| (n.layout.x, n.layout.y))
        .unwrap_or((0.0, 0.0));
    (ax - lx, ay - ly)
}

/// Serialize the DOM tree with full computed styles on every node.
pub fn serialize_tree_verbose(tree: &NodeTree) -> Value {
    serialize_node(tree, tree.body_id, 0.0, 0.0, true, None, 0)
}

fn serialize_node(
    tree: &NodeTree,
    id: RawNodeId,
    offset_x: f32,
    offset_y: f32,
    verbose: bool,
    max_depth: Option<u32>,
    depth: u32,
) -> Value {
    let Some(node) = tree.get(id) else {
        return Value::Null;
    };

    let abs_x = offset_x + node.layout.x;
    let abs_y = offset_y + node.layout.y;

    let (node_type, tag, text) = match &node.kind {
        NodeKind::Document => ("document", None, None),
        NodeKind::Element(el) => ("element", Some(el.tag.as_str()), None),
        NodeKind::Text(t) => ("text", None, Some(t.content.as_str())),
        NodeKind::Comment(c) => ("comment", None, Some(c.as_str())),
    };

    let sx = node.scroll_offset.0 as f32;
    let sy = node.scroll_offset.1 as f32;

    let (layout, absolute) = geometry_json(node, abs_x, abs_y);
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
        .map(|&child_id| {
            serialize_node(
                tree,
                child_id,
                abs_x - sx,
                abs_y - sy,
                verbose,
                max_depth,
                depth + 1,
            )
        })
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

    let abs = compute_absolute_position(tree, id);

    let (node_type, tag, text) = match &node.kind {
        NodeKind::Document => ("document", None, None),
        NodeKind::Element(el) => ("element", Some(el.tag.as_str()), None),
        NodeKind::Text(t) => ("text", None, Some(t.content.as_str())),
        NodeKind::Comment(c) => ("comment", None, Some(c.as_str())),
    };

    let (layout, absolute) = geometry_json(node, abs.0, abs.1);
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

    let abs = compute_absolute_position(tree, id);

    let (node_type, tag, text) = match &node.kind {
        NodeKind::Document => ("document", None, None),
        NodeKind::Element(el) => ("element", Some(el.tag.as_str()), None),
        NodeKind::Text(t) => ("text", None, Some(t.content.as_str())),
        NodeKind::Comment(c) => ("comment", None, Some(c.as_str())),
    };

    let (layout, absolute) = geometry_json(node, abs.0, abs.1);
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
/// - `absolute` is the on-screen box: x/y are accumulated down the ancestor
///   chain (minus scroll). **Pass `absolute.x`/`absolute.y` to `click()`** — the
///   `layout` x/y are NOT screen coordinates.
///
/// width/height are the same in both (a box has one size).
fn geometry_json(node: &crate::node::Node, abs_x: f32, abs_y: f32) -> (Value, Value) {
    let w = node.layout.width;
    let h = node.layout.height;
    (
        json!({ "x": node.layout.x, "y": node.layout.y, "width": w, "height": h }),
        json!({ "x": abs_x, "y": abs_y, "width": w, "height": h }),
    )
}

/// Compute absolute position by walking up the tree.
fn compute_absolute_position(tree: &NodeTree, id: RawNodeId) -> (f32, f32) {
    let mut x = 0.0f32;
    let mut y = 0.0f32;
    let mut current = Some(id);
    while let Some(nid) = current {
        if let Some(node) = tree.get(nid) {
            x += node.layout.x;
            y += node.layout.y;
            // position: fixed — viewport-relative, stop accumulating parent offsets
            if node.computed_style.position == crate::computed_style::PositionValue::Fixed {
                break;
            }
            // Subtract parent's scroll offset (same as hit_test)
            if let Some(parent_id) = node.parent {
                if let Some(parent) = tree.get(parent_id) {
                    x -= parent.scroll_offset.0 as f32;
                    y -= parent.scroll_offset.1 as f32;
                }
            }
            current = node.parent;
        } else {
            break;
        }
    }
    (x, y)
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
}
