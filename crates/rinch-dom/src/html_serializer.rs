//! Serialize a rinch DOM tree to HTML.
//!
//! Walks the node tree and produces HTML that can be rendered in a browser
//! for visual comparison testing. The output preserves the exact element
//! structure, attributes, and text content that rinch uses internally.

use crate::node::{NodeKind, NodeTree, RawNodeId};

/// Serialize the DOM tree rooted at `body_id` to an HTML string.
///
/// Returns a complete HTML document with the provided CSS injected as a
/// `<style>` block. The viewport is set to the given dimensions.
pub fn serialize_to_html(tree: &NodeTree, css: &str, width: u32, height: u32) -> String {
    let mut html = String::with_capacity(8192);

    html.push_str("<!DOCTYPE html>\n<html>\n<head>\n");
    html.push_str("<meta charset=\"utf-8\">\n");
    html.push_str(&format!(
        "<meta name=\"viewport\" content=\"width={}, initial-scale=1\">\n",
        width
    ));
    html.push_str("<style>\n");
    // Reset browser defaults to match rinch's rendering
    html.push_str("* { margin: 0; padding: 0; box-sizing: border-box; }\n");
    html.push_str("html, body { width: 100%; height: 100%; }\n");
    // Inject the rinch CSS (theme + components)
    html.push_str(css);
    html.push_str("\n</style>\n");
    html.push_str("</head>\n");
    html.push_str(&format!(
        "<body style=\"width: {}px; height: {}px; overflow: hidden;\">\n",
        width, height
    ));

    // Serialize the body's children (not the body node itself, since we emitted it above)
    let body = &tree.nodes[tree.body_id];
    for &child_id in &body.children {
        serialize_node(tree, child_id, &mut html, 0);
    }

    html.push_str("</body>\n</html>\n");
    html
}

/// Serialize a single node and its subtree.
fn serialize_node(tree: &NodeTree, node_id: RawNodeId, html: &mut String, depth: usize) {
    let node = &tree.nodes[node_id];

    // Skip anonymous block boxes and pseudo-elements — they're internal layout artifacts
    if node.is_anonymous_block_box || node.is_pseudo_element {
        return;
    }

    match &node.kind {
        NodeKind::Element(elem) => {
            let tag = &elem.tag;

            // Indent for readability
            let indent = "  ".repeat(depth);
            html.push_str(&indent);
            html.push('<');
            html.push_str(tag);

            // Emit attributes in a stable order: class, style, then others sorted
            let mut attr_keys: Vec<&String> = node.attributes.keys().collect();
            attr_keys.sort();

            // Put class first, style second, rest alphabetically
            let mut ordered: Vec<&String> = Vec::with_capacity(attr_keys.len());
            if let Some(pos) = attr_keys.iter().position(|k| k.as_str() == "class") {
                ordered.push(attr_keys.remove(pos));
            }
            if let Some(pos) = attr_keys.iter().position(|k| k.as_str() == "style") {
                ordered.push(attr_keys.remove(pos));
            }
            ordered.extend(attr_keys);

            for key in ordered {
                let value = &node.attributes[key];
                // Skip internal rinch attributes that don't affect rendering
                if key.starts_with("data-rid")
                    || key.starts_with("data-on")
                    || key == "data-viewport"
                {
                    continue;
                }
                html.push(' ');
                html.push_str(key);
                html.push_str("=\"");
                html.push_str(&escape_attr(value));
                html.push('"');
            }

            html.push_str(">\n");

            // Recurse into children
            for &child_id in &node.children {
                serialize_node(tree, child_id, html, depth + 1);
            }

            // Void elements don't get closing tags
            if !is_void_element(tag) {
                html.push_str(&indent);
                html.push_str("</");
                html.push_str(tag);
                html.push_str(">\n");
            }
        }
        NodeKind::Text(text) => {
            if !text.content.is_empty() {
                let indent = "  ".repeat(depth);
                html.push_str(&indent);
                html.push_str(&escape_html(&text.content));
                html.push('\n');
            }
        }
        NodeKind::Comment(content) => {
            let indent = "  ".repeat(depth);
            html.push_str(&indent);
            html.push_str("<!-- ");
            html.push_str(content);
            html.push_str(" -->\n");
        }
        NodeKind::Document => {
            // Shouldn't happen in normal traversal, but recurse children if it does
            for &child_id in &node.children {
                serialize_node(tree, child_id, html, depth);
            }
        }
    }
}

/// HTML-escape text content.
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Escape attribute values.
fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Check if an HTML tag is a void element (self-closing, no end tag).
fn is_void_element(tag: &str) -> bool {
    matches!(
        tag,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}
