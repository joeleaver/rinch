//! Simple HTML parser for fragment parsing.
//!
//! This module provides a lightweight HTML parser that can parse HTML strings
//! into a tree of [`ParsedNode`] values. It's used by both the DOM adapter
//! and the window manager to handle `PatchElement::Html` patches.

use std::collections::HashMap;

/// A parsed HTML node.
#[derive(Debug, Clone)]
pub enum ParsedNode {
    /// An element node with tag, attributes, and children.
    Element {
        tag: String,
        attrs: HashMap<String, String>,
        children: Vec<ParsedNode>,
    },
    /// A text node.
    Text(String),
}

/// Parse an HTML string into a tree of nodes.
///
/// Returns `None` if the HTML is empty or couldn't be parsed.
pub fn parse_html_string(html: &str) -> Option<Vec<ParsedNode>> {
    let mut parser = HtmlParser::new(html);
    parse_children(&mut parser, None)
}

/// Simple HTML parser state.
struct HtmlParser<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> HtmlParser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    fn remaining(&self) -> &str {
        &self.input[self.pos..]
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.input.len() {
            let ch = self.input[self.pos..].chars().next().unwrap();
            if ch.is_whitespace() {
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }
    }

    fn peek_char(&self) -> Option<char> {
        self.remaining().chars().next()
    }

    fn consume_char(&mut self) -> Option<char> {
        let ch = self.remaining().chars().next()?;
        self.pos += ch.len_utf8();
        Some(ch)
    }

    fn consume_until(&mut self, target: &str) -> &str {
        let start = self.pos;
        if let Some(idx) = self.remaining().find(target) {
            self.pos += idx;
            &self.input[start..self.pos]
        } else {
            self.pos = self.input.len();
            &self.input[start..]
        }
    }

    fn consume_while<F: Fn(char) -> bool>(&mut self, predicate: F) -> &str {
        let start = self.pos;
        while let Some(ch) = self.peek_char() {
            if predicate(ch) {
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }
        &self.input[start..self.pos]
    }

    fn is_at_end(&self) -> bool {
        self.pos >= self.input.len()
    }
}

fn parse_children(parser: &mut HtmlParser, end_tag: Option<&str>) -> Option<Vec<ParsedNode>> {
    let mut children = Vec::new();

    while !parser.is_at_end() {
        // Check for closing tag
        if let Some(tag) = end_tag {
            let close_tag = format!("</{}", tag);
            if parser.remaining().starts_with(&close_tag) {
                // Consume the closing tag
                parser.pos += close_tag.len();
                // Consume until >
                parser.consume_until(">");
                if parser.peek_char() == Some('>') {
                    parser.consume_char();
                }
                return Some(children);
            }
        }

        if parser.remaining().starts_with("<!--") {
            // Skip comments
            parser.pos += 4;
            parser.consume_until("-->");
            if parser.remaining().starts_with("-->") {
                parser.pos += 3;
            }
        } else if parser.remaining().starts_with("</") {
            // Unexpected closing tag - stop parsing
            break;
        } else if parser.peek_char() == Some('<') {
            // Parse element
            if let Some(element) = parse_element(parser) {
                children.push(element);
            }
        } else {
            // Parse text
            let text = parser.consume_until("<");
            if !text.is_empty() {
                children.push(ParsedNode::Text(decode_html_entities(text)));
            }
        }
    }

    Some(children)
}

fn parse_element(parser: &mut HtmlParser) -> Option<ParsedNode> {
    // Consume <
    parser.consume_char()?;

    // Get tag name
    let tag = parser
        .consume_while(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == ':')
        .to_string();

    if tag.is_empty() {
        return None;
    }

    // Parse attributes
    let mut attrs = HashMap::new();
    loop {
        parser.skip_whitespace();

        if parser.peek_char() == Some('>') {
            parser.consume_char();
            break;
        }

        if parser.remaining().starts_with("/>") {
            parser.pos += 2;
            // Self-closing tag
            return Some(ParsedNode::Element {
                tag,
                attrs,
                children: Vec::new(),
            });
        }

        // Parse attribute name
        let attr_name = parser
            .consume_while(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == ':')
            .to_string();

        if attr_name.is_empty() {
            // Skip invalid character
            parser.consume_char();
            continue;
        }

        parser.skip_whitespace();

        let attr_value = if parser.peek_char() == Some('=') {
            parser.consume_char(); // =
            parser.skip_whitespace();

            if parser.peek_char() == Some('"') {
                parser.consume_char(); // "
                let value = parser.consume_until("\"").to_string();
                parser.consume_char(); // "
                decode_html_entities(&value)
            } else if parser.peek_char() == Some('\'') {
                parser.consume_char(); // '
                let value = parser.consume_until("'").to_string();
                parser.consume_char(); // '
                decode_html_entities(&value)
            } else {
                // Unquoted value
                parser
                    .consume_while(|c| !c.is_whitespace() && c != '>')
                    .to_string()
            }
        } else {
            // Boolean attribute
            String::new()
        };

        attrs.insert(attr_name, attr_value);
    }

    // Check for void elements
    if is_void_element(&tag) {
        return Some(ParsedNode::Element {
            tag,
            attrs,
            children: Vec::new(),
        });
    }

    // Parse children
    let children = parse_children(parser, Some(&tag)).unwrap_or_default();

    Some(ParsedNode::Element {
        tag,
        attrs,
        children,
    })
}

/// Check if a tag is a void element (self-closing, no end tag).
pub fn is_void_element(tag: &str) -> bool {
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

/// Decode common HTML entities.
pub fn decode_html_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", "\u{00A0}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_element() {
        let nodes = parse_html_string("<div>hello</div>").unwrap();
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            ParsedNode::Element { tag, children, .. } => {
                assert_eq!(tag, "div");
                assert_eq!(children.len(), 1);
                match &children[0] {
                    ParsedNode::Text(text) => assert_eq!(text, "hello"),
                    _ => panic!("Expected text node"),
                }
            }
            _ => panic!("Expected element"),
        }
    }

    #[test]
    fn test_parse_with_attributes() {
        let nodes = parse_html_string(r#"<div class="test" id="main">content</div>"#).unwrap();
        match &nodes[0] {
            ParsedNode::Element { tag, attrs, .. } => {
                assert_eq!(tag, "div");
                assert_eq!(attrs.get("class"), Some(&"test".to_string()));
                assert_eq!(attrs.get("id"), Some(&"main".to_string()));
            }
            _ => panic!("Expected element"),
        }
    }

    #[test]
    fn test_parse_void_element() {
        let nodes = parse_html_string("<br><hr>").unwrap();
        assert_eq!(nodes.len(), 2);
    }

    #[test]
    fn test_parse_nested() {
        let nodes = parse_html_string("<div><span>inner</span></div>").unwrap();
        match &nodes[0] {
            ParsedNode::Element { children, .. } => {
                assert_eq!(children.len(), 1);
                match &children[0] {
                    ParsedNode::Element { tag, .. } => assert_eq!(tag, "span"),
                    _ => panic!("Expected element"),
                }
            }
            _ => panic!("Expected element"),
        }
    }

    #[test]
    fn test_decode_entities() {
        assert_eq!(decode_html_entities("&lt;div&gt;"), "<div>");
        assert_eq!(decode_html_entities("&amp;"), "&");
    }
}
