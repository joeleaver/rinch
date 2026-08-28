//! HTML serializer - converts rinch DOM JSON to standalone HTML.

use crate::css_export::computed_style_to_css;
use serde_json::Value;

/// Configuration for HTML serialization.
#[derive(Debug, Clone)]
pub struct HtmlConfig {
    /// Viewport width in pixels.
    pub viewport_width: u32,
    /// Viewport height in pixels.
    pub viewport_height: u32,
    /// Background color for the document body.
    pub background_color: String,
}

impl Default for HtmlConfig {
    fn default() -> Self {
        Self {
            viewport_width: 800,
            viewport_height: 600,
            background_color: "#1a1a1a".to_string(),
        }
    }
}

/// Serialize a rinch DOM tree (JSON) to a standalone HTML document.
pub fn serialize_to_html(dom: &Value, config: &HtmlConfig) -> String {
    let mut html = String::new();

    // HTML doctype and head
    html.push_str("<!DOCTYPE html>\n");
    html.push_str("<html>\n");
    html.push_str("<head>\n");
    html.push_str("  <meta charset=\"utf-8\">\n");
    html.push_str(&format!(
        "  <meta name=\"viewport\" content=\"width={}, height={}\">\n",
        config.viewport_width, config.viewport_height
    ));
    html.push_str("  <style>\n");
    html.push_str("    * { margin: 0; padding: 0; box-sizing: border-box; }\n");
    html.push_str(&format!(
        "    html, body {{ width: {}px; height: {}px; overflow: hidden; }}\n",
        config.viewport_width, config.viewport_height
    ));
    html.push_str("  </style>\n");
    html.push_str("</head>\n");

    // Body with background color
    html.push_str(&format!(
        "<body style=\"background-color: {};\">\n",
        config.background_color
    ));

    // Serialize DOM tree
    serialize_node(&mut html, dom, 1);

    html.push_str("</body>\n");
    html.push_str("</html>\n");

    html
}

/// Remove CSS property declarations that contain unresolved CSS variables.
/// The computed_styles already have resolved values, so we don't need the var() references.
fn strip_css_variables(style: &str) -> String {
    style
        .split(';')
        .filter(|decl| !decl.contains("var(--"))
        .collect::<Vec<_>>()
        .join(";")
}

/// Recursively serialize a DOM node.
fn serialize_node(html: &mut String, node: &Value, indent: usize) {
    let node_type = node.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match node_type {
        "element" => serialize_element(html, node, indent),
        "text" => serialize_text(html, node, indent),
        "document" => {
            // Document node - just serialize children
            if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
                for child in children {
                    serialize_node(html, child, indent);
                }
            }
        }
        _ => {} // Skip unknown node types
    }
}

/// Serialize an element node.
fn serialize_element(html: &mut String, node: &Value, indent: usize) {
    let indent_str = "  ".repeat(indent);
    let tag = node.get("tag").and_then(|v| v.as_str()).unwrap_or("div");

    // Skip certain internal elements
    if tag == "body" || tag == "html" || tag == "head" {
        // Just serialize children for these
        if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
            for child in children {
                serialize_node(html, child, indent);
            }
        }
        return;
    }

    // Skip style/script tags entirely - they contain CSS/JS, not visual content
    // We have our own style reset in the document head
    if tag == "style" || tag == "script" {
        return;
    }

    // Build inline style from computed_styles
    let mut style = String::new();
    if let Some(computed_styles) = node.get("computed_styles") {
        style = computed_style_to_css(computed_styles);
    }

    // Get attributes
    let mut attrs = String::new();
    if let Some(attributes) = node.get("attributes").and_then(|v| v.as_object()) {
        for (key, value) in attributes {
            if key == "style" {
                // Merge with computed style, but strip CSS variables since
                // computed_styles already has resolved values
                if let Some(inline_style) = value.as_str() {
                    let filtered = strip_css_variables(inline_style);
                    if !filtered.trim().is_empty() {
                        if !style.is_empty() {
                            style.push(' ');
                        }
                        style.push_str(&filtered);
                    }
                }
            } else if key != "class" && key != "data-rid" {
                // Include other attributes (skip class and internal rinch attributes)
                if let Some(v) = value.as_str() {
                    attrs.push_str(&format!(
                        " {}=\"{}\"",
                        html_escape_attr(key),
                        html_escape_attr(v)
                    ));
                }
            }
        }
    }

    // Build the opening tag
    html.push_str(&indent_str);
    html.push('<');
    html.push_str(tag);
    if !style.is_empty() {
        html.push_str(&format!(" style=\"{}\"", html_escape_attr(&style)));
    }
    html.push_str(&attrs);

    // Check for void elements
    if is_void_element(tag) {
        html.push_str(" />\n");
        return;
    }

    html.push('>');

    // Serialize children
    let children = node.get("children").and_then(|v| v.as_array());
    let has_children = children.map(|c| !c.is_empty()).unwrap_or(false);
    let has_only_text = children
        .map(|c| c.len() == 1 && c[0].get("type").and_then(|v| v.as_str()) == Some("text"))
        .unwrap_or(false);

    if has_only_text {
        // Inline text content
        if let Some(children) = children {
            if let Some(text) = children[0].get("text").and_then(|v| v.as_str()) {
                html.push_str(&html_escape(text));
            }
        }
    } else if has_children {
        html.push('\n');
        if let Some(children) = children {
            for child in children {
                serialize_node(html, child, indent + 1);
            }
        }
        html.push_str(&indent_str);
    }

    // Closing tag
    html.push_str("</");
    html.push_str(tag);
    html.push_str(">\n");
}

/// Serialize a text node.
fn serialize_text(html: &mut String, node: &Value, indent: usize) {
    let indent_str = "  ".repeat(indent);
    if let Some(text) = node.get("text").and_then(|v| v.as_str()) {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            html.push_str(&indent_str);
            html.push_str(&html_escape(trimmed));
            html.push('\n');
        }
    }
}

/// Check if a tag is a void element (no closing tag).
fn is_void_element(tag: &str) -> bool {
    matches!(
        tag.to_lowercase().as_str(),
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

/// Escape HTML special characters.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Escape HTML attribute special characters.
fn html_escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_simple_element() {
        let dom = json!({
            "type": "element",
            "tag": "div",
            "computed_styles": {
                "display": "Flex",
                "padding_top": {"Length": 16.0}
            },
            "children": []
        });

        let config = HtmlConfig::default();
        let html = serialize_to_html(&dom, &config);

        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("<div"));
        assert!(html.contains("display: flex"));
        assert!(html.contains("padding-top: 16px"));
    }

    #[test]
    fn test_text_node() {
        let dom = json!({
            "type": "element",
            "tag": "p",
            "computed_styles": {},
            "children": [
                {
                    "type": "text",
                    "text": "Hello, World!"
                }
            ]
        });

        let config = HtmlConfig::default();
        let html = serialize_to_html(&dom, &config);

        assert!(html.contains("<p"));
        assert!(html.contains("Hello, World!"));
        assert!(html.contains("</p>"));
    }

    #[test]
    fn test_nested_elements() {
        let dom = json!({
            "type": "element",
            "tag": "div",
            "computed_styles": {"display": "Flex"},
            "children": [
                {
                    "type": "element",
                    "tag": "span",
                    "computed_styles": {"color": "#ffffff"},
                    "children": [
                        {"type": "text", "text": "Nested"}
                    ]
                }
            ]
        });

        let config = HtmlConfig::default();
        let html = serialize_to_html(&dom, &config);

        assert!(html.contains("<div"));
        assert!(html.contains("<span"));
        assert!(html.contains("color: #ffffff"));
        assert!(html.contains("Nested"));
    }

    #[test]
    fn test_html_escaping() {
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
        assert_eq!(html_escape("a & b"), "a &amp; b");
        assert_eq!(html_escape_attr("value=\"test\""), "value=&quot;test&quot;");
    }

    #[test]
    fn test_void_elements() {
        assert!(is_void_element("img"));
        assert!(is_void_element("br"));
        assert!(is_void_element("input"));
        assert!(!is_void_element("div"));
        assert!(!is_void_element("span"));
    }

    #[test]
    fn test_style_script_tags_skipped() {
        let dom = json!({
            "type": "element",
            "tag": "div",
            "computed_styles": {"display": "Flex"},
            "children": [
                {
                    "type": "element",
                    "tag": "style",
                    "computed_styles": {"display": "Flex"},
                    "children": [
                        {"type": "text", "text": ".my-class { color: red; }"}
                    ]
                },
                {
                    "type": "element",
                    "tag": "script",
                    "children": [
                        {"type": "text", "text": "console.log('test');"}
                    ]
                },
                {
                    "type": "element",
                    "tag": "p",
                    "children": [
                        {"type": "text", "text": "Visible content"}
                    ]
                }
            ]
        });

        let config = HtmlConfig::default();
        let html = serialize_to_html(&dom, &config);

        // The CSS content from rinch's <style> tag should not appear in body
        assert!(!html.contains(".my-class { color: red; }"));
        // The JS content should not appear
        assert!(!html.contains("console.log('test');"));

        // But the visible content should still be present
        assert!(html.contains("<p"));
        assert!(html.contains("Visible content"));
    }
}
