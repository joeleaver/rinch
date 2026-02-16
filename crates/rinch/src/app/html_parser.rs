// ── HTML escape helpers ─────────────────────────────────────────────────────

/// Escape HTML special characters in text content.
pub(super) fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Escape characters for use inside an HTML attribute value (double-quoted).
pub(super) fn html_escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ── Simple HTML Fragment Parser ─────────────────────────────────────────────
//
// Parses well-formed HTML fragments from clipboard data into a tree of
// ParsedNode values that can be inserted into the rinch DOM.
// Only used when clipboard feature is enabled.

/// A parsed HTML node from clipboard content.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(super) enum ParsedNode {
    /// Plain text content.
    Text(String),
    /// An HTML element with tag, attributes, and children.
    Element {
        tag: String,
        attributes: Vec<(String, String)>,
        children: Vec<ParsedNode>,
    },
}

/// Parse an HTML fragment string into a list of parsed nodes.
///
/// Handles common clipboard HTML:
/// - Strips `<html>`, `<head>`, `<body>` wrappers (extracts body content)
/// - Strips `<meta>`, `<style>`, `<script>` tags entirely
/// - Preserves formatting elements (strong, em, b, i, u, s, span, etc.)
/// - Preserves block elements (p, div, h1-h6, ul, ol, li, blockquote, etc.)
/// - Preserves style and class attributes, strips everything else except href
/// - Decodes basic HTML entities (&amp; &lt; &gt; &quot; &nbsp;)
#[allow(dead_code)]
pub(super) fn parse_html_fragment(html: &str) -> Vec<ParsedNode> {
    let mut parser = HtmlFragmentParser::new(html);
    let nodes = parser.parse();

    // If the result is a single <html> or <body> wrapper, unwrap it
    unwrap_body(nodes)
}

/// Unwrap `<html>` / `<body>` wrappers, returning their inner children.
#[allow(dead_code)]
fn unwrap_body(nodes: Vec<ParsedNode>) -> Vec<ParsedNode> {
    if nodes.len() == 1
        && let ParsedNode::Element {
            ref tag,
            ref children,
            ..
        } = nodes[0]
        && (tag == "html" || tag == "body")
    {
        return unwrap_body(children.clone());
    }
    // Also check for html > head + body pattern
    let mut result = Vec::new();
    let mut found_wrapper = false;
    for node in &nodes {
        if let ParsedNode::Element { tag, children, .. } = node {
            if tag == "html" || tag == "body" {
                result.extend(unwrap_body(children.clone()));
                found_wrapper = true;
            } else if tag == "head" || tag == "meta" || tag == "style" || tag == "script" {
                found_wrapper = true;
                // Skip these entirely
            } else {
                result.push(node.clone());
            }
        } else {
            result.push(node.clone());
        }
    }
    if found_wrapper && !result.is_empty() {
        return result;
    }
    nodes
}

#[allow(dead_code)]
struct HtmlFragmentParser<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> HtmlFragmentParser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    fn parse(&mut self) -> Vec<ParsedNode> {
        self.parse_nodes(None)
    }

    fn parse_nodes(&mut self, end_tag: Option<&str>) -> Vec<ParsedNode> {
        let mut nodes = Vec::new();

        while self.pos < self.input.len() {
            // Check for end tag
            if let Some(end) = end_tag
                && self.looking_at_close_tag(end)
            {
                self.consume_close_tag();
                break;
            }

            if self.peek() == Some('<') {
                // Check for comment
                if self.input[self.pos..].starts_with("<!--") {
                    self.skip_comment();
                    continue;
                }
                // Check for closing tag (unexpected — belongs to parent)
                if self.input[self.pos..].starts_with("</") {
                    if end_tag.is_none() {
                        // Unexpected close tag — skip it
                        self.skip_to('>');
                    }
                    break;
                }
                // Opening tag
                if let Some(element) = self.parse_element() {
                    nodes.push(element);
                }
            } else {
                // Text content
                let text = self.parse_text();
                if !text.is_empty() {
                    nodes.push(ParsedNode::Text(text));
                }
            }
        }

        nodes
    }

    fn parse_element(&mut self) -> Option<ParsedNode> {
        // Consume '<'
        self.advance();

        // Parse tag name
        let tag = self.parse_tag_name()?.to_ascii_lowercase();

        // Skip dangerous/unwanted tags entirely
        if matches!(
            tag.as_str(),
            "script" | "style" | "meta" | "link" | "head" | "title"
        ) {
            self.skip_until_close_tag(&tag);
            return None;
        }

        // Parse attributes
        let attributes = self.parse_attributes();

        // Check for self-closing or void element
        self.skip_whitespace();
        let self_closing = self.peek() == Some('/');
        if self_closing {
            self.advance(); // skip '/'
        }

        // Consume '>'
        if self.peek() == Some('>') {
            self.advance();
        }

        let is_void = is_void_tag(&tag);
        if self_closing || is_void {
            return Some(ParsedNode::Element {
                tag,
                attributes: filter_attributes(attributes),
                children: Vec::new(),
            });
        }

        // Parse children until matching close tag
        let children = self.parse_nodes(Some(&tag));

        Some(ParsedNode::Element {
            tag,
            attributes: filter_attributes(attributes),
            children,
        })
    }

    fn parse_tag_name(&mut self) -> Option<String> {
        let start = self.pos;
        while self.pos < self.input.len() {
            let ch = self.input.as_bytes()[self.pos];
            if ch.is_ascii_alphanumeric() || ch == b'-' || ch == b'_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos > start {
            Some(self.input[start..self.pos].to_string())
        } else {
            // Skip malformed tag
            self.skip_to('>');
            None
        }
    }

    fn parse_attributes(&mut self) -> Vec<(String, String)> {
        let mut attrs = Vec::new();

        loop {
            self.skip_whitespace();
            let ch = self.peek();
            if ch == Some('>') || ch == Some('/') || ch.is_none() {
                break;
            }

            // Parse attribute name
            let name_start = self.pos;
            while self.pos < self.input.len() {
                let ch = self.input.as_bytes()[self.pos];
                if ch == b'=' || ch == b'>' || ch == b'/' || ch.is_ascii_whitespace() {
                    break;
                }
                self.pos += 1;
            }
            let name = self.input[name_start..self.pos].to_ascii_lowercase();
            if name.is_empty() {
                self.advance(); // skip invalid char
                continue;
            }

            self.skip_whitespace();

            // Check for '='
            if self.peek() == Some('=') {
                self.advance(); // skip '='
                self.skip_whitespace();

                // Parse attribute value
                let value = if self.peek() == Some('"') {
                    self.advance(); // skip opening quote
                    let v = self.consume_until('"');
                    self.advance(); // skip closing quote
                    decode_entities(&v)
                } else if self.peek() == Some('\'') {
                    self.advance();
                    let v = self.consume_until('\'');
                    self.advance();
                    decode_entities(&v)
                } else {
                    // Unquoted value
                    let v_start = self.pos;
                    while self.pos < self.input.len() {
                        let ch = self.input.as_bytes()[self.pos];
                        if ch.is_ascii_whitespace() || ch == b'>' || ch == b'/' {
                            break;
                        }
                        self.pos += 1;
                    }
                    decode_entities(&self.input[v_start..self.pos])
                };
                attrs.push((name, value));
            } else {
                // Boolean attribute (no value)
                attrs.push((name, String::new()));
            }
        }

        attrs
    }

    fn parse_text(&mut self) -> String {
        let start = self.pos;
        while self.pos < self.input.len() && self.input.as_bytes()[self.pos] != b'<' {
            self.pos += 1;
        }
        let raw = &self.input[start..self.pos];
        decode_entities(raw)
    }

    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn advance(&mut self) {
        if self.pos < self.input.len() {
            self.pos += self.input[self.pos..]
                .chars()
                .next()
                .map_or(1, |c| c.len_utf8());
        }
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.input.len() && self.input.as_bytes()[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn skip_to(&mut self, ch: char) {
        while self.pos < self.input.len() && self.input.as_bytes()[self.pos] != ch as u8 {
            self.pos += 1;
        }
        if self.pos < self.input.len() {
            self.pos += 1;
        }
    }

    fn consume_until(&mut self, ch: char) -> String {
        let start = self.pos;
        while self.pos < self.input.len() && self.input.as_bytes()[self.pos] != ch as u8 {
            self.pos += 1;
        }
        self.input[start..self.pos].to_string()
    }

    fn looking_at_close_tag(&self, tag: &str) -> bool {
        let remaining = &self.input[self.pos..];
        if !remaining.starts_with("</") {
            return false;
        }
        let after = &remaining[2..];
        if after.len() < tag.len() {
            return false;
        }
        after[..tag.len()].eq_ignore_ascii_case(tag)
            && after
                .as_bytes()
                .get(tag.len())
                .is_none_or(|&b| b == b'>' || b.is_ascii_whitespace())
    }

    fn consume_close_tag(&mut self) {
        self.skip_to('>');
    }

    fn skip_comment(&mut self) {
        self.pos += 4; // skip "<!--"
        while self.pos + 2 < self.input.len() {
            if &self.input[self.pos..self.pos + 3] == "-->" {
                self.pos += 3;
                return;
            }
            self.pos += 1;
        }
        self.pos = self.input.len();
    }

    fn skip_until_close_tag(&mut self, tag: &str) {
        let close = format!("</{}", tag);
        while self.pos < self.input.len() {
            if self.input[self.pos..]
                .to_ascii_lowercase()
                .starts_with(&close)
            {
                self.skip_to('>');
                return;
            }
            self.pos += 1;
        }
    }
}

/// Check if an HTML tag is a void element (no closing tag).
#[allow(dead_code)]
fn is_void_tag(tag: &str) -> bool {
    matches!(
        tag,
        "br" | "hr"
            | "img"
            | "input"
            | "meta"
            | "link"
            | "wbr"
            | "area"
            | "base"
            | "col"
            | "embed"
            | "source"
            | "track"
    )
}

/// Filter attributes to only keep safe ones (style, class, href).
#[allow(dead_code)]
fn filter_attributes(attrs: Vec<(String, String)>) -> Vec<(String, String)> {
    attrs
        .into_iter()
        .filter(|(name, _)| matches!(name.as_str(), "style" | "class" | "href"))
        .collect()
}

/// Decode basic HTML entities.
#[allow(dead_code)]
fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&nbsp;", "\u{00A0}")
        .replace("&#39;", "'")
}
