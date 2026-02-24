//! Document fragment for representing parts of documents.

use super::model::MarkData;
use std::collections::HashMap;

/// A fragment of document content (for clipboard, drag-drop, etc.).
#[derive(Clone, Debug)]
pub struct Fragment {
    /// The blocks in this fragment.
    pub blocks: Vec<FragmentBlock>,
}

impl Fragment {
    /// Create a new empty fragment.
    pub fn empty() -> Self {
        Self { blocks: Vec::new() }
    }

    /// Create a fragment from plain text. Each line becomes a paragraph block.
    pub fn from_text(text: &str) -> Self {
        let blocks = text
            .split('\n')
            .map(|line| FragmentBlock {
                block_type: "paragraph".to_string(),
                attrs: HashMap::new(),
                content: vec![FragmentInline::Text {
                    text: line.to_string(),
                    marks: Vec::new(),
                }],
            })
            .collect();
        Self { blocks }
    }

    /// Get the total character count of this fragment.
    pub fn size(&self) -> usize {
        let text_len: usize = self.blocks.iter().map(|b| b.text_length()).sum();
        let separators = self.blocks.len().saturating_sub(1);
        text_len + separators
    }

    /// Convert fragment to plain text.
    pub fn text(&self) -> String {
        self.blocks
            .iter()
            .map(|b| b.text())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Check if the fragment is empty.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty() || self.blocks.iter().all(|b| b.content.is_empty())
    }

    /// Convert fragment to HTML.
    pub fn to_html(&self) -> String {
        use super::model::serialization::{html_escape, wrap_mark};

        let mut html = String::new();
        for block in &self.blocks {
            let tag = block_type_to_tag(&block.block_type, &block.attrs);
            html.push_str(&format!("<{}>", tag));
            for inline in &block.content {
                match inline {
                    FragmentInline::Text { text, marks } => {
                        let mut result = html_escape(text);
                        for mark in marks {
                            result = wrap_mark(&result, mark);
                        }
                        html.push_str(&result);
                    }
                    FragmentInline::HardBreak => html.push_str("<br>"),
                }
            }
            html.push_str(&format!("</{}>", tag));
        }
        html
    }

    /// Convert fragment to a markdown string.
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();
        for block in &self.blocks {
            let content = block_inline_to_markdown(&block.content);
            match block.block_type.as_str() {
                "heading" => {
                    let level = block
                        .attrs
                        .get("level")
                        .and_then(|l| l.parse::<usize>().ok())
                        .unwrap_or(1);
                    let hashes = "#".repeat(level);
                    md.push_str(&format!("{} {}\n\n", hashes, content));
                }
                "blockquote" => {
                    md.push_str(&format!("> {}\n\n", content));
                }
                "code_block" => {
                    let lang = block.attrs.get("language").cloned().unwrap_or_default();
                    md.push_str(&format!("```{}\n{}\n```\n\n", lang, content));
                }
                "bullet_list" | "list_item" => {
                    md.push_str(&format!("- {}\n", content));
                }
                "ordered_list" => {
                    md.push_str(&format!("1. {}\n", content));
                }
                "horizontal_rule" => {
                    md.push_str("---\n\n");
                }
                _ => {
                    // paragraph and others
                    md.push_str(&content);
                    md.push_str("\n\n");
                }
            }
        }
        // Trim trailing whitespace
        md.trim_end().to_string()
    }

    /// Parse a markdown string into a Fragment.
    pub fn from_markdown(md: &str) -> Self {
        use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

        let mut options = Options::empty();
        options.insert(Options::ENABLE_STRIKETHROUGH);
        options.insert(Options::ENABLE_TABLES);

        let parser = Parser::new_ext(md, options);
        let mut blocks: Vec<FragmentBlock> = Vec::new();
        let mut current_content: Vec<FragmentInline> = Vec::new();
        let mut mark_stack: Vec<MarkData> = Vec::new();
        let mut in_block = false;

        // Stack of container block types/attrs. When we encounter a container
        // like BlockQuote that wraps a Paragraph, we push the container type
        // onto this stack. When the inner Paragraph ends, we use the outermost
        // container type instead of "paragraph".
        let mut container_stack: Vec<(String, HashMap<String, String>)> = Vec::new();

        for event in parser {
            match event {
                Event::Start(tag) => match tag {
                    Tag::Heading { level, .. } => {
                        container_stack.push(("heading".to_string(), {
                            let mut a = HashMap::new();
                            a.insert("level".to_string(), (level as usize).to_string());
                            a
                        }));
                        in_block = true;
                    }
                    Tag::Paragraph => {
                        // Only push paragraph if not inside a container that
                        // overrides (blockquote, list item). In those cases,
                        // the container is already on the stack.
                        if container_stack.is_empty() {
                            container_stack.push(("paragraph".to_string(), HashMap::new()));
                        }
                        in_block = true;
                    }
                    Tag::BlockQuote(_) => {
                        container_stack.push(("blockquote".to_string(), HashMap::new()));
                        // Don't set in_block yet — Paragraph inside will do that
                    }
                    Tag::CodeBlock(kind) => {
                        let mut attrs = HashMap::new();
                        if let CodeBlockKind::Fenced(lang) = kind
                            && !lang.is_empty()
                        {
                            attrs.insert("language".to_string(), lang.to_string());
                        }
                        container_stack.push(("code_block".to_string(), attrs));
                        in_block = true;
                    }
                    Tag::List(Some(_)) | Tag::List(None) => {
                        // List container — items will come as Tag::Item
                    }
                    Tag::Item => {
                        container_stack.push(("bullet_list".to_string(), HashMap::new()));
                        in_block = true;
                    }
                    Tag::Strong => mark_stack.push(MarkData::new("bold")),
                    Tag::Emphasis => mark_stack.push(MarkData::new("italic")),
                    Tag::Strikethrough => mark_stack.push(MarkData::new("strike")),
                    Tag::Link { dest_url, .. } => {
                        let mut attrs = HashMap::new();
                        attrs.insert("href".to_string(), dest_url.to_string());
                        mark_stack.push(MarkData::with_attrs("link", attrs));
                    }
                    _ => {}
                },
                Event::End(tag_end) => match tag_end {
                    TagEnd::Paragraph => {
                        if in_block {
                            // Emit a block. Use the outermost container type.
                            let (block_type, attrs) = container_stack
                                .first()
                                .cloned()
                                .unwrap_or_else(|| ("paragraph".to_string(), HashMap::new()));

                            // Trim trailing newlines from code block text
                            if block_type == "code_block" {
                                trim_trailing_newline(&mut current_content);
                            }

                            if current_content.is_empty() {
                                current_content.push(FragmentInline::Text {
                                    text: String::new(),
                                    marks: Vec::new(),
                                });
                            }
                            blocks.push(FragmentBlock {
                                block_type,
                                attrs,
                                content: std::mem::take(&mut current_content),
                            });
                            in_block = false;
                            // Pop only if the outermost container is paragraph
                            // (blockquote/item will be popped by their own End event)
                            if container_stack.last().map(|(t, _)| t.as_str()) == Some("paragraph")
                            {
                                container_stack.pop();
                            }
                        }
                    }
                    TagEnd::Heading(_) => {
                        if in_block {
                            if current_content.is_empty() {
                                current_content.push(FragmentInline::Text {
                                    text: String::new(),
                                    marks: Vec::new(),
                                });
                            }
                            let (block_type, attrs) = container_stack
                                .pop()
                                .unwrap_or_else(|| ("heading".to_string(), HashMap::new()));
                            blocks.push(FragmentBlock {
                                block_type,
                                attrs,
                                content: std::mem::take(&mut current_content),
                            });
                            in_block = false;
                        }
                    }
                    TagEnd::CodeBlock => {
                        if in_block {
                            // Trim trailing newlines from code block text
                            trim_trailing_newline(&mut current_content);

                            if current_content.is_empty() {
                                current_content.push(FragmentInline::Text {
                                    text: String::new(),
                                    marks: Vec::new(),
                                });
                            }
                            let (block_type, attrs) = container_stack
                                .pop()
                                .unwrap_or_else(|| ("code_block".to_string(), HashMap::new()));
                            blocks.push(FragmentBlock {
                                block_type,
                                attrs,
                                content: std::mem::take(&mut current_content),
                            });
                            in_block = false;
                        }
                    }
                    TagEnd::BlockQuote(_) => {
                        // Pop the blockquote container
                        if container_stack.last().map(|(t, _)| t.as_str()) == Some("blockquote") {
                            container_stack.pop();
                        }
                    }
                    TagEnd::Item => {
                        // If we still have content that wasn't flushed by a
                        // Paragraph end (some list items have no inner paragraph),
                        // emit the block now.
                        if in_block && !current_content.is_empty() {
                            let (block_type, attrs) = container_stack
                                .first()
                                .cloned()
                                .unwrap_or_else(|| ("bullet_list".to_string(), HashMap::new()));
                            blocks.push(FragmentBlock {
                                block_type,
                                attrs,
                                content: std::mem::take(&mut current_content),
                            });
                            in_block = false;
                        }
                        // Pop the item container
                        if container_stack.last().map(|(t, _)| t.as_str()) == Some("bullet_list") {
                            container_stack.pop();
                        }
                    }
                    TagEnd::Strong => {
                        mark_stack.retain(|m| m.mark_type != "bold");
                    }
                    TagEnd::Emphasis => {
                        mark_stack.retain(|m| m.mark_type != "italic");
                    }
                    TagEnd::Strikethrough => {
                        mark_stack.retain(|m| m.mark_type != "strike");
                    }
                    TagEnd::Link => {
                        mark_stack.retain(|m| m.mark_type != "link");
                    }
                    _ => {}
                },
                Event::Text(text) => {
                    if in_block {
                        current_content.push(FragmentInline::Text {
                            text: text.to_string(),
                            marks: mark_stack.clone(),
                        });
                    }
                }
                Event::Code(code) => {
                    if in_block {
                        current_content.push(FragmentInline::Text {
                            text: code.to_string(),
                            marks: vec![MarkData::new("code")],
                        });
                    }
                }
                Event::SoftBreak => {
                    if in_block {
                        current_content.push(FragmentInline::Text {
                            text: " ".to_string(),
                            marks: mark_stack.clone(),
                        });
                    }
                }
                Event::HardBreak => {
                    if in_block {
                        current_content.push(FragmentInline::HardBreak);
                    }
                }
                Event::Rule => {
                    blocks.push(FragmentBlock {
                        block_type: "horizontal_rule".to_string(),
                        attrs: HashMap::new(),
                        content: Vec::new(),
                    });
                }
                _ => {}
            }
        }

        if blocks.is_empty() {
            Self::from_text(md)
        } else {
            Self { blocks }
        }
    }

    /// Parse simple HTML into a Fragment.
    ///
    /// Handles common block tags (p, h1-h6, blockquote, pre) and inline
    /// formatting tags (strong/b, em/i, u, s/del, code, a, br).
    pub fn from_html(html: &str) -> Self {
        let blocks = parse_html_to_blocks(html);
        if blocks.is_empty() {
            // If no block-level elements found, treat entire content as a paragraph
            let content = parse_inline_html(html);
            Self {
                blocks: vec![FragmentBlock {
                    block_type: "paragraph".to_string(),
                    attrs: HashMap::new(),
                    content,
                }],
            }
        } else {
            Self { blocks }
        }
    }
}

/// A block within a fragment.
#[derive(Clone, Debug)]
pub struct FragmentBlock {
    /// Block type (e.g., "paragraph", "heading")
    pub block_type: String,
    /// Block attributes
    pub attrs: HashMap<String, String>,
    /// Inline content
    pub content: Vec<FragmentInline>,
}

impl FragmentBlock {
    /// Get text length of this block.
    pub fn text_length(&self) -> usize {
        self.content.iter().map(|i| i.text_length()).sum()
    }

    /// Get plain text of this block.
    pub fn text(&self) -> String {
        self.content.iter().map(|i| i.text()).collect()
    }
}

/// Inline content within a fragment block.
#[derive(Clone, Debug)]
pub enum FragmentInline {
    /// A text run with optional marks.
    Text { text: String, marks: Vec<MarkData> },
    /// A hard line break.
    HardBreak,
}

impl FragmentInline {
    /// Get the text length of this inline.
    pub fn text_length(&self) -> usize {
        match self {
            FragmentInline::Text { text, .. } => text.len(),
            FragmentInline::HardBreak => 1,
        }
    }

    /// Get the text of this inline.
    pub fn text(&self) -> String {
        match self {
            FragmentInline::Text { text, .. } => text.clone(),
            FragmentInline::HardBreak => "\n".to_string(),
        }
    }
}

/// Trim trailing newline from the last text inline (pulldown-cmark appends one to code blocks).
fn trim_trailing_newline(content: &mut [FragmentInline]) {
    if let Some(FragmentInline::Text { text, .. }) = content.last_mut()
        && text.ends_with('\n')
    {
        text.truncate(text.len() - 1);
    }
}

/// Convert inline content of a block to markdown.
fn block_inline_to_markdown(content: &[FragmentInline]) -> String {
    let mut result = String::new();
    for inline in content {
        match inline {
            FragmentInline::Text { text, marks } => {
                let mut s = text.clone();
                // Apply marks innermost-first
                for mark in marks.iter().rev() {
                    s = wrap_mark_markdown(&s, mark);
                }
                result.push_str(&s);
            }
            FragmentInline::HardBreak => {
                result.push_str("  \n"); // Two trailing spaces = hard break in markdown
            }
        }
    }
    result
}

/// Wrap text with markdown formatting for a mark.
fn wrap_mark_markdown(text: &str, mark: &MarkData) -> String {
    match mark.mark_type.as_str() {
        "bold" => format!("**{}**", text),
        "italic" => format!("*{}*", text),
        "strike" | "strikethrough" => format!("~~{}~~", text),
        "code" => format!("`{}`", text),
        "link" => {
            let href = mark.attrs.get("href").map(|s| s.as_str()).unwrap_or("#");
            format!("[{}]({})", text, href)
        }
        _ => text.to_string(), // underline, highlight etc have no markdown equivalent
    }
}

/// Convert a block type + attrs to an HTML tag name.
fn block_type_to_tag(block_type: &str, attrs: &HashMap<String, String>) -> &'static str {
    match block_type {
        "paragraph" => "p",
        "heading" => match attrs.get("level").map(|s| s.as_str()) {
            Some("1") => "h1",
            Some("2") => "h2",
            Some("3") => "h3",
            Some("4") => "h4",
            Some("5") => "h5",
            Some("6") => "h6",
            _ => "h1",
        },
        "blockquote" => "blockquote",
        "code_block" => "pre",
        "bullet_list" => "ul",
        "ordered_list" => "ol",
        "list_item" => "li",
        "horizontal_rule" => "hr",
        _ => "p",
    }
}

/// Convert an HTML tag name to a block type + attrs.
fn tag_to_block_type(tag: &str) -> (String, HashMap<String, String>) {
    let mut attrs = HashMap::new();
    let block_type = match tag {
        "p" | "div" => "paragraph",
        "h1" => {
            attrs.insert("level".to_string(), "1".to_string());
            "heading"
        }
        "h2" => {
            attrs.insert("level".to_string(), "2".to_string());
            "heading"
        }
        "h3" => {
            attrs.insert("level".to_string(), "3".to_string());
            "heading"
        }
        "h4" => {
            attrs.insert("level".to_string(), "4".to_string());
            "heading"
        }
        "h5" => {
            attrs.insert("level".to_string(), "5".to_string());
            "heading"
        }
        "h6" => {
            attrs.insert("level".to_string(), "6".to_string());
            "heading"
        }
        "blockquote" => "blockquote",
        "pre" => "code_block",
        "ul" => "bullet_list",
        "ol" => "ordered_list",
        "li" => "list_item",
        "hr" => "horizontal_rule",
        _ => "paragraph",
    };
    (block_type.to_string(), attrs)
}

/// Convert an HTML tag to a mark type (returns None if not a mark tag).
fn tag_to_mark(tag: &str) -> Option<String> {
    match tag {
        "strong" | "b" => Some("bold".to_string()),
        "em" | "i" => Some("italic".to_string()),
        "u" => Some("underline".to_string()),
        "s" | "del" | "strike" => Some("strike".to_string()),
        "code" => Some("code".to_string()),
        _ => None,
    }
}

/// Check if a tag is a block-level element.
fn is_block_tag(tag: &str) -> bool {
    matches!(
        tag,
        "p" | "div"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "blockquote"
            | "pre"
            | "ul"
            | "ol"
            | "li"
            | "hr"
    )
}

/// Simple HTML entity decoding.
fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

/// Parse HTML string into block-level fragments.
fn parse_html_to_blocks(html: &str) -> Vec<FragmentBlock> {
    let mut blocks = Vec::new();
    let mut pos = 0;
    let bytes = html.as_bytes();

    while pos < bytes.len() {
        // Skip whitespace between blocks
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }

        // Look for opening tag
        if bytes[pos] == b'<' {
            if let Some(tag_end) = html[pos..].find('>') {
                let tag_content = &html[pos + 1..pos + tag_end];
                let tag_name = tag_content
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_end_matches('/');

                if is_block_tag(tag_name) {
                    // Find closing tag
                    let close_tag = format!("</{}>", tag_name);
                    let inner_start = pos + tag_end + 1;

                    if tag_name == "hr" || tag_content.ends_with('/') {
                        // Self-closing
                        blocks.push(FragmentBlock {
                            block_type: "horizontal_rule".to_string(),
                            attrs: HashMap::new(),
                            content: Vec::new(),
                        });
                        pos = inner_start;
                    } else if let Some(close_pos) = html[inner_start..].find(&close_tag) {
                        let inner_html = &html[inner_start..inner_start + close_pos];
                        let (block_type, attrs) = tag_to_block_type(tag_name);
                        let content = parse_inline_html(inner_html);
                        blocks.push(FragmentBlock {
                            block_type,
                            attrs,
                            content,
                        });
                        pos = inner_start + close_pos + close_tag.len();
                    } else {
                        pos = inner_start;
                    }
                } else {
                    // Skip non-block tags at top level
                    pos = pos + tag_end + 1;
                }
            } else {
                pos += 1;
            }
        } else {
            // Text outside block tags - collect until next tag
            let text_end = html[pos..].find('<').unwrap_or(html.len() - pos);
            let text = decode_entities(html[pos..pos + text_end].trim());
            if !text.is_empty() {
                blocks.push(FragmentBlock {
                    block_type: "paragraph".to_string(),
                    attrs: HashMap::new(),
                    content: vec![FragmentInline::Text {
                        text,
                        marks: Vec::new(),
                    }],
                });
            }
            pos += text_end;
        }
    }

    blocks
}

/// Parse inline HTML content into FragmentInline nodes.
fn parse_inline_html(html: &str) -> Vec<FragmentInline> {
    let mut result = Vec::new();
    let mut mark_stack: Vec<String> = Vec::new();
    let mut pos = 0;

    while pos < html.len() {
        if html.as_bytes()[pos] == b'<' {
            if let Some(tag_end) = html[pos..].find('>') {
                let tag_content = &html[pos + 1..pos + tag_end];
                let is_closing = tag_content.starts_with('/');
                let tag_name = if is_closing {
                    tag_content[1..]
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .trim_end_matches('/')
                } else {
                    tag_content
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .trim_end_matches('/')
                };

                if tag_name == "br" {
                    result.push(FragmentInline::HardBreak);
                } else if is_closing {
                    // Pop matching mark from stack
                    if let Some(mark) = tag_to_mark(tag_name)
                        && let Some(idx) = mark_stack.iter().rposition(|m| *m == mark)
                    {
                        mark_stack.remove(idx);
                    }
                } else if let Some(mark) = tag_to_mark(tag_name) {
                    mark_stack.push(mark);
                }
                // Skip other tags (a, span, etc.) but continue parsing their content

                pos = pos + tag_end + 1;
            } else {
                pos += 1;
            }
        } else {
            // Text content
            let text_end = html[pos..].find('<').unwrap_or(html.len() - pos);
            let text = decode_entities(&html[pos..pos + text_end]);
            if !text.is_empty() {
                let marks: Vec<MarkData> = mark_stack.iter().map(MarkData::new).collect();
                result.push(FragmentInline::Text { text, marks });
            }
            pos += text_end;
        }
    }

    if result.is_empty() {
        result.push(FragmentInline::Text {
            text: String::new(),
            marks: Vec::new(),
        });
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_fragment() {
        let f = Fragment::empty();
        assert!(f.is_empty());
        assert_eq!(f.size(), 0);
        assert_eq!(f.text(), "");
    }

    #[test]
    fn from_text_single_line() {
        let f = Fragment::from_text("Hello");
        assert_eq!(f.blocks.len(), 1);
        assert_eq!(f.text(), "Hello");
        assert_eq!(f.size(), 5);
    }

    #[test]
    fn from_text_multiple_lines() {
        let f = Fragment::from_text("Hello\nWorld");
        assert_eq!(f.blocks.len(), 2);
        assert_eq!(f.text(), "Hello\nWorld");
        assert_eq!(f.size(), 11); // 5 + 1 (separator) + 5
    }

    #[test]
    fn fragment_block_type() {
        let f = Fragment::from_text("Hello");
        assert_eq!(f.blocks[0].block_type, "paragraph");
    }

    #[test]
    fn to_html_simple() {
        let f = Fragment::from_text("Hello");
        assert_eq!(f.to_html(), "<p>Hello</p>");
    }

    #[test]
    fn to_html_with_marks() {
        let f = Fragment {
            blocks: vec![FragmentBlock {
                block_type: "paragraph".to_string(),
                attrs: HashMap::new(),
                content: vec![
                    FragmentInline::Text {
                        text: "Hello".to_string(),
                        marks: vec![MarkData::new("bold")],
                    },
                    FragmentInline::Text {
                        text: " World".to_string(),
                        marks: Vec::new(),
                    },
                ],
            }],
        };
        assert_eq!(f.to_html(), "<p><strong>Hello</strong> World</p>");
    }

    #[test]
    fn from_html_simple_paragraph() {
        let f = Fragment::from_html("<p>Hello World</p>");
        assert_eq!(f.blocks.len(), 1);
        assert_eq!(f.blocks[0].block_type, "paragraph");
        assert_eq!(f.text(), "Hello World");
    }

    #[test]
    fn from_html_with_bold() {
        let f = Fragment::from_html("<p><strong>Hello</strong> World</p>");
        assert_eq!(f.blocks.len(), 1);
        assert_eq!(f.text(), "Hello World");
        // Should have 2 inline nodes: bold "Hello" and plain " World"
        assert_eq!(f.blocks[0].content.len(), 2);
        match &f.blocks[0].content[0] {
            FragmentInline::Text { text, marks } => {
                assert_eq!(text, "Hello");
                assert_eq!(marks.len(), 1);
                assert_eq!(marks[0].mark_type, "bold");
            }
            _ => panic!("Expected Text"),
        }
        match &f.blocks[0].content[1] {
            FragmentInline::Text { text, marks } => {
                assert_eq!(text, " World");
                assert!(marks.is_empty());
            }
            _ => panic!("Expected Text"),
        }
    }

    #[test]
    fn from_html_heading() {
        let f = Fragment::from_html("<h2>Title</h2>");
        assert_eq!(f.blocks.len(), 1);
        assert_eq!(f.blocks[0].block_type, "heading");
        assert_eq!(f.blocks[0].attrs.get("level"), Some(&"2".to_string()));
        assert_eq!(f.text(), "Title");
    }

    #[test]
    fn from_html_multiple_blocks() {
        let f = Fragment::from_html("<h1>Title</h1><p>Body text</p>");
        assert_eq!(f.blocks.len(), 2);
        assert_eq!(f.blocks[0].block_type, "heading");
        assert_eq!(f.blocks[1].block_type, "paragraph");
        assert_eq!(f.text(), "Title\nBody text");
    }

    #[test]
    fn html_roundtrip() {
        let html = "<p><strong>Hello</strong> World</p>";
        let f = Fragment::from_html(html);
        assert_eq!(f.to_html(), html);
    }

    #[test]
    fn to_markdown_simple() {
        let f = Fragment::from_text("Hello");
        assert_eq!(f.to_markdown(), "Hello");
    }

    #[test]
    fn to_markdown_heading() {
        let mut attrs = HashMap::new();
        attrs.insert("level".to_string(), "2".to_string());
        let f = Fragment {
            blocks: vec![FragmentBlock {
                block_type: "heading".to_string(),
                attrs,
                content: vec![FragmentInline::Text {
                    text: "Title".to_string(),
                    marks: Vec::new(),
                }],
            }],
        };
        assert_eq!(f.to_markdown(), "## Title");
    }

    #[test]
    fn to_markdown_bold() {
        let f = Fragment {
            blocks: vec![FragmentBlock {
                block_type: "paragraph".to_string(),
                attrs: HashMap::new(),
                content: vec![FragmentInline::Text {
                    text: "bold".to_string(),
                    marks: vec![MarkData::new("bold")],
                }],
            }],
        };
        assert_eq!(f.to_markdown(), "**bold**");
    }

    #[test]
    fn from_markdown_heading() {
        let f = Fragment::from_markdown("## Title");
        assert_eq!(f.blocks.len(), 1);
        assert_eq!(f.blocks[0].block_type, "heading");
        assert_eq!(f.blocks[0].attrs.get("level"), Some(&"2".to_string()));
        assert_eq!(f.text(), "Title");
    }

    #[test]
    fn from_markdown_bold() {
        let f = Fragment::from_markdown("**bold**");
        assert_eq!(f.blocks.len(), 1);
        assert_eq!(f.blocks[0].content.len(), 1);
        if let FragmentInline::Text { marks, .. } = &f.blocks[0].content[0] {
            assert!(marks.iter().any(|m| m.mark_type == "bold"));
        }
    }

    #[test]
    fn markdown_roundtrip() {
        let md = "# Hello\n\nThis is **bold** and *italic*.";
        let f = Fragment::from_markdown(md);
        let result = f.to_markdown();
        // Verify key elements survive roundtrip
        assert!(result.contains("# Hello"));
        assert!(result.contains("**bold**"));
        assert!(result.contains("*italic*"));
    }

    #[test]
    fn from_markdown_code_block() {
        let f = Fragment::from_markdown("```rust\nfn main() {}\n```");
        assert_eq!(f.blocks.len(), 1);
        assert_eq!(f.blocks[0].block_type, "code_block");
        assert_eq!(f.blocks[0].attrs.get("language"), Some(&"rust".to_string()));
        assert_eq!(f.text(), "fn main() {}");
    }

    #[test]
    fn from_markdown_blockquote() {
        let f = Fragment::from_markdown("> Quote text");
        assert_eq!(f.blocks.len(), 1);
        assert_eq!(f.blocks[0].block_type, "blockquote");
        assert_eq!(f.text(), "Quote text");
    }

    #[test]
    fn from_markdown_horizontal_rule() {
        let f = Fragment::from_markdown("---");
        assert_eq!(f.blocks.len(), 1);
        assert_eq!(f.blocks[0].block_type, "horizontal_rule");
    }

    #[test]
    fn from_markdown_link() {
        let f = Fragment::from_markdown("[click](https://example.com)");
        assert_eq!(f.blocks.len(), 1);
        if let FragmentInline::Text { marks, text } = &f.blocks[0].content[0] {
            assert_eq!(text, "click");
            assert!(marks.iter().any(|m| m.mark_type == "link"
                && m.attrs.get("href").map(|s| s.as_str()) == Some("https://example.com")));
        } else {
            panic!("Expected Text inline");
        }
    }

    #[test]
    fn from_markdown_bullet_list() {
        let f = Fragment::from_markdown("- Item 1\n- Item 2");
        assert_eq!(f.blocks.len(), 2);
        assert_eq!(f.blocks[0].block_type, "bullet_list");
        assert_eq!(f.blocks[1].block_type, "bullet_list");
        assert_eq!(f.blocks[0].text(), "Item 1");
        assert_eq!(f.blocks[1].text(), "Item 2");
    }
}
