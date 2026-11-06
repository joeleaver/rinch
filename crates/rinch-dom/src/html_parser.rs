//! Simple HTML parser for fragment parsing.
//!
//! This module provides a lightweight HTML parser that can parse HTML strings
//! into a tree of nodes that can be inserted into the DOM.

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
    parse_children(&mut parser, None, false, Ns::Html)
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

/// The namespace content is parsed in: what decides whether an element's
/// text is raw (#2239). Tracked as the HTML standard's tree builder does,
/// as far as that decides it: `<svg>` and `<math>` open foreign content, each
/// language's integration points hold HTML again, and an HTML-only start tag
/// breaks out of it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ns {
    Html,
    Svg,
    MathMl,
}

/// The children of `end_tag` (the top level with `None`), parsed in `ns`.
/// `raw`: the element holds raw text, whose character references are not
/// decoded.
fn parse_children(
    parser: &mut HtmlParser,
    end_tag: Option<&str>,
    raw: bool,
    ns: Ns,
) -> Option<Vec<ParsedNode>> {
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
            if let Some(element) = parse_element(parser, ns) {
                children.push(element);
            }
        } else {
            // Parse text
            let text = parser.consume_until("<");
            if !text.is_empty() {
                // `<style>`, `<script>` and the other raw text elements hold
                // text whose references are not decoded (the HTML standard's
                // RAWTEXT and script data states), in HTML only.
                children.push(ParsedNode::Text(if raw {
                    text.to_string()
                } else {
                    decode_html_entities(text)
                }));
            }
        }
    }

    Some(children)
}

fn parse_element(parser: &mut HtmlParser, ns: Ns) -> Option<ParsedNode> {
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
                decode_attribute_value(&value)
            } else if parser.peek_char() == Some('\'') {
                parser.consume_char(); // '
                let value = parser.consume_until("'").to_string();
                parser.consume_char(); // '
                decode_attribute_value(&value)
            } else {
                // Unquoted value
                decode_attribute_value(parser.consume_while(|c| !c.is_whitespace() && c != '>'))
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
    let is = |names: &[&str]| names.iter().any(|t| tag.eq_ignore_ascii_case(t));
    // The element's own namespace: an HTML-only start tag in foreign content
    // is HTML (the standard pops out of the foreign elements there).
    let breaks_out = is(BREAKOUT_TAGS)
        || (tag.eq_ignore_ascii_case("font")
            && ["color", "face", "size"]
                .iter()
                .any(|a| attrs.keys().any(|k| k.eq_ignore_ascii_case(a))));
    let own = match ns {
        _ if is(&["svg"]) => Ns::Svg,
        Ns::Html if is(&["math"]) => Ns::MathMl,
        Ns::Svg | Ns::MathMl if breaks_out => Ns::Html,
        ns => ns,
    };
    // What its children are parsed in: each language's integration points
    // hold HTML (SVG `<foreignObject>`, `<desc>`, `<title>`; MathML's text
    // integration points and an `<annotation-xml>` of HTML).
    let html_encoding = || {
        attrs.iter().any(|(k, v)| {
            k.eq_ignore_ascii_case("encoding")
                && (v.eq_ignore_ascii_case("text/html")
                    || v.eq_ignore_ascii_case("application/xhtml+xml"))
        })
    };
    let child_ns = match own {
        Ns::Svg if is(&["foreignObject", "desc", "title"]) => Ns::Html,
        Ns::MathMl if is(&["mi", "mo", "mn", "ms", "mtext"]) => Ns::Html,
        Ns::MathMl if is(&["annotation-xml"]) && html_encoding() => Ns::Html,
        own => own,
    };
    let raw = own == Ns::Html && is_raw_text_element(&tag);
    let children = parse_children(parser, Some(&tag), raw, child_ns).unwrap_or_default();

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

/// Whether an HTML element `tag`'s content is raw text, in which character
/// references are not decoded: the standard's RAWTEXT and script data
/// elements, and `<noscript>` (scripting is on in a browser). Not in SVG or
/// MathML, where a `<style>` or `<script>` is decoded.
/// The start tags that break out of foreign content (`<font>` too, with a
/// `color`, `face` or `size` attribute).
const BREAKOUT_TAGS: &[&str] = &[
    "b",
    "big",
    "blockquote",
    "body",
    "br",
    "center",
    "code",
    "dd",
    "div",
    "dl",
    "dt",
    "em",
    "embed",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "hr",
    "i",
    "img",
    "li",
    "listing",
    "menu",
    "meta",
    "nobr",
    "ol",
    "p",
    "pre",
    "ruby",
    "s",
    "small",
    "span",
    "strong",
    "strike",
    "sub",
    "sup",
    "table",
    "tt",
    "u",
    "ul",
    "var",
];

fn is_raw_text_element(tag: &str) -> bool {
    [
        "style", "script", "xmp", "iframe", "noembed", "noframes", "noscript",
    ]
    .iter()
    .any(|t| tag.eq_ignore_ascii_case(t))
}

/// Decode the character references of an element's text as the HTML
/// standard does (#2239): every named reference, decimal and hexadecimal
/// ones (`&#8203;`, `&#x200B;`), in one pass
/// ([`rinch_core::html_entities::decode_text`]).
pub fn decode_html_entities(s: &str) -> String {
    rinch_core::html_entities::decode_text(s)
}

/// Decode the character references of an attribute's value
/// ([`rinch_core::html_entities::decode_attribute`]): as in text, except that
/// a legacy name with no `;` before `=`, a letter or a digit is left alone
/// (`?a=1&copy=2`).
pub fn decode_attribute_value(s: &str) -> String {
    rinch_core::html_entities::decode_attribute(s)
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
        // #2239: numeric references, every named one, one pass.
        assert_eq!(
            decode_html_entities("a&#8203;b&#x41;&#X42;&#67&hellip;&amp;lt;"),
            "a\u{200b}bABC\u{2026}&lt;"
        );
        assert_eq!(
            decode_attribute_value("?a=1&copy=2&#8203;"),
            "?a=1&copy=2\u{200b}"
        );
    }
}
