//! Testing utilities for rinch-dom.
//!
//! Provides DOM tree serialization, query selectors, and text content extraction
//! for the rinch test harness.

use serde_json::{Value, json};

use crate::node::{NodeKind, NodeTree, RawNodeId};

/// Serialize the DOM tree starting from body as a JSON value (compact — no computed styles).
pub fn serialize_tree(tree: &NodeTree) -> Value {
    serialize_node(tree, tree.body_id, 0.0, 0.0, false)
}

/// Serialize the DOM tree with full computed styles on every node.
pub fn serialize_tree_verbose(tree: &NodeTree) -> Value {
    serialize_node(tree, tree.body_id, 0.0, 0.0, true)
}

fn serialize_node(
    tree: &NodeTree,
    id: RawNodeId,
    offset_x: f32,
    offset_y: f32,
    verbose: bool,
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

    let children: Vec<Value> = node
        .children
        .iter()
        .map(|&child_id| serialize_node(tree, child_id, abs_x - sx, abs_y - sy, verbose))
        .collect();

    let mut obj = json!({
        "id": node.id,
        "type": node_type,
        "layout": {
            "x": abs_x,
            "y": abs_y,
            "width": node.layout.width,
            "height": node.layout.height,
        },
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

    let mut obj = json!({
        "id": node.id,
        "type": node_type,
        "layout": {
            "x": abs.0,
            "y": abs.1,
            "width": node.layout.width,
            "height": node.layout.height,
        },
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

    let mut obj = json!({
        "id": node.id,
        "type": node_type,
        "layout": {
            "x": abs.0,
            "y": abs.1,
            "width": node.layout.width,
            "height": node.layout.height,
        },
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
