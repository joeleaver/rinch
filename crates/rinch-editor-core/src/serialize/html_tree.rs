//! The HTML reader's tree builder ([`super::html`]): a zero-dependency
//! tokenizer that turns a clipboard-sourced HTML fragment, however broken,
//! into a tree of [`ParsedNode`]s.
//!
//! It never fails and never stops early: markup it does not understand is
//! skipped where a browser shows nothing for it and kept as text where a
//! browser shows it. What it follows of the HTML parsing rules:
//!
//! - A tag name runs to the next whitespace, `/` or `>`, so Word's `<o:p>`
//!   and `<st1:place>` and a custom element are elements like any other.
//!   A `<` that starts no tag (`a < b`, `<3`) is text.
//! - Comments, conditional comments, `<!DOCTYPE>`, `<![if …]>` / `<![endif]>`
//!   and `<?xml …>` give nothing. Neither do `<script>`, `<style>`,
//!   `<title>`, `<iframe>`, `<noscript>`, `<noembed>`, `<noframes>` and
//!   `<template>` with their content, nor `<meta>` and `<link>`.
//! - `<html>` and `<body>` tags are skipped (their content stays), and a
//!   `<head>` ends at its end tag or at the first thing that is not head
//!   content.
//! - End tags are implied as a browser implies them: a `<p>` ends at the next
//!   block, an `<li>` at the next `<li>`, a cell at the next cell or row, a
//!   row at the next row. An end tag closes the nearest open element of its
//!   name, unless a table cell (or, for an inline element, a block) is in
//!   between; one that closes nothing is skipped.
//! - `<textarea>` holds text, not markup.
//! - At most [`MAX_DEPTH`] elements are open at once. Past that a start tag
//!   opens nothing and its content goes where it stands, which is what Chrome
//!   does at the same depth; everything that walks the tree may recurse.
//!
//! Not followed: a self-closing `<x/>` is an empty element whatever `x` is
//! (in HTML only a void element is), formatting elements are not reopened
//! across a block (the adoption agency algorithm), and content that has no
//! place in a table stays where it is written for the reader to move.

use super::html_entities::decode_entities;

/// The most elements open at once (Chrome's `kMaximumHTMLParserDOMTreeDepth`).
pub(super) const MAX_DEPTH: usize = 128;

thread_local! {
    static STEPS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The work the HTML reader has done on this thread: one for every tag,
/// text run and comment read, every open element looked at to place a tag,
/// and every node the reader visits to build content. A count, so that a
/// test can pin how reading scales without a clock.
#[doc(hidden)]
pub fn html_reader_steps() -> u64 {
    STEPS.with(std::cell::Cell::get)
}

pub(super) fn step() {
    STEPS.with(|c| c.set(c.get() + 1));
}

/// A parsed HTML node.
#[derive(Debug, Clone)]
pub(super) enum ParsedNode {
    Text(String),
    Element {
        tag: String,
        attributes: Vec<(String, String)>,
        children: Vec<ParsedNode>,
        /// Whether an element under this one is a block: set by the reader
        /// ([`super::html`]), which knows the schema.
        holds_block: bool,
    },
}

/// An element whose end tag has not been read.
struct Open {
    tag: String,
    attributes: Vec<(String, String)>,
    children: Vec<ParsedNode>,
}

pub(super) struct HtmlFragmentParser<'a> {
    input: &'a str,
    pos: usize,
    /// Keep every attribute rather than [`filter_attributes`]' set: for
    /// `dropped_table_attr`, which reports what the filter would drop.
    pub(super) all_attributes: bool,
    root: Vec<ParsedNode>,
    stack: Vec<Open>,
    /// Start tags that opened nothing because [`MAX_DEPTH`] elements were
    /// open; as many end tags close nothing.
    unopened: usize,
    /// Inside a `<head>`.
    in_head: bool,
}

impl<'a> HtmlFragmentParser<'a> {
    pub(super) fn new(input: &'a str) -> Self {
        Self {
            input,
            pos: 0,
            all_attributes: false,
            root: Vec::new(),
            stack: Vec::new(),
            unopened: 0,
            in_head: false,
        }
    }

    pub(super) fn parse(mut self) -> Vec<ParsedNode> {
        let bytes = self.input.as_bytes();
        while self.pos < bytes.len() {
            step();
            if bytes[self.pos] != b'<' {
                self.text();
                continue;
            }
            match bytes.get(self.pos + 1) {
                Some(b) if b.is_ascii_alphabetic() => self.start_tag(),
                Some(b'/') => self.end_tag(),
                Some(b'!') => self.comment(),
                Some(b'?') => self.skip_past(b'>'),
                // Not markup: `a < b`.
                _ => {
                    self.pos += 1;
                    self.append_text("<");
                }
            }
        }
        self.close_to(0);
        self.root
    }

    // ── Text ─────────────────────────────────────────────────────────────────

    fn text(&mut self) {
        let start = self.pos;
        let bytes = self.input.as_bytes();
        while self.pos < bytes.len() && bytes[self.pos] != b'<' {
            self.pos += 1;
        }
        let text = decode_entities(&self.input[start..self.pos]);
        self.append_text(&text);
    }

    fn append_text(&mut self, text: &str) {
        if self.in_head {
            if text.trim().is_empty() {
                return;
            }
            self.in_head = false;
        }
        if text.is_empty() {
            return;
        }
        let siblings = self.siblings();
        if let Some(ParsedNode::Text(last)) = siblings.last_mut() {
            last.push_str(text);
        } else {
            siblings.push(ParsedNode::Text(text.to_string()));
        }
    }

    fn siblings(&mut self) -> &mut Vec<ParsedNode> {
        match self.stack.last_mut() {
            Some(open) => &mut open.children,
            None => &mut self.root,
        }
    }

    // ── Comments ─────────────────────────────────────────────────────────────

    /// At `<!`: a comment, or a declaration (`<!DOCTYPE html>`,
    /// `<![endif]>`), which ends at the next `>`.
    fn comment(&mut self) {
        let bytes = self.input.as_bytes();
        if !bytes[self.pos..].starts_with(b"<!--") {
            self.skip_past(b'>');
            return;
        }
        self.pos += 4;
        // `<!-->` and `<!--->` are whole comments.
        if bytes[self.pos..].starts_with(b">") {
            self.pos += 1;
            return;
        }
        if bytes[self.pos..].starts_with(b"->") {
            self.pos += 2;
            return;
        }
        while self.pos < bytes.len() {
            if bytes[self.pos] == b'-' {
                let rest = &bytes[self.pos..];
                if rest.starts_with(b"-->") {
                    self.pos += 3;
                    return;
                }
                if rest.starts_with(b"--!>") {
                    self.pos += 4;
                    return;
                }
            }
            self.pos += 1;
        }
    }

    fn skip_past(&mut self, byte: u8) {
        let bytes = self.input.as_bytes();
        while self.pos < bytes.len() && bytes[self.pos] != byte {
            self.pos += 1;
        }
        if self.pos < bytes.len() {
            self.pos += 1;
        }
    }

    // ── Tags ─────────────────────────────────────────────────────────────────

    /// The tag name at `pos`: everything up to whitespace, `/` or `>`.
    fn tag_name(&mut self) -> String {
        let start = self.pos;
        let bytes = self.input.as_bytes();
        while self.pos < bytes.len()
            && !bytes[self.pos].is_ascii_whitespace()
            && bytes[self.pos] != b'/'
            && bytes[self.pos] != b'>'
        {
            self.pos += 1;
        }
        self.input[start..self.pos].to_ascii_lowercase()
    }

    /// The attributes of the tag being read, up to and past its `>`, and
    /// whether the tag ends in `/>`. `None` when the input ends first: a tag
    /// with no end is no tag.
    fn attributes(&mut self) -> Option<(Vec<(String, String)>, bool)> {
        let bytes = self.input.as_bytes();
        let mut attrs = Vec::new();
        let mut slash = false;
        loop {
            while self.pos < bytes.len() && bytes[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
            match bytes.get(self.pos) {
                None => return None,
                Some(b'>') => {
                    self.pos += 1;
                    return Some((attrs, slash));
                }
                Some(b'/') => {
                    self.pos += 1;
                    slash = true;
                    continue;
                }
                Some(_) => {}
            }
            slash = false;
            let name_start = self.pos;
            // A name may start with `=`.
            self.pos += 1;
            while self.pos < bytes.len()
                && !bytes[self.pos].is_ascii_whitespace()
                && !matches!(bytes[self.pos], b'=' | b'>' | b'/')
            {
                self.pos += 1;
            }
            // `pos` is at an ASCII byte or the end, and so was `name_start`.
            let name = self.input[name_start..self.pos].to_ascii_lowercase();
            while self.pos < bytes.len() && bytes[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
            if bytes.get(self.pos) != Some(&b'=') {
                attrs.push((name, String::new()));
                continue;
            }
            self.pos += 1;
            while self.pos < bytes.len() && bytes[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
            let value = match bytes.get(self.pos) {
                Some(&quote @ (b'"' | b'\'')) => {
                    self.pos += 1;
                    let start = self.pos;
                    while self.pos < bytes.len() && bytes[self.pos] != quote {
                        self.pos += 1;
                    }
                    let value = &self.input[start..self.pos];
                    if self.pos < bytes.len() {
                        self.pos += 1;
                    }
                    value
                }
                _ => {
                    let start = self.pos;
                    while self.pos < bytes.len()
                        && !bytes[self.pos].is_ascii_whitespace()
                        && bytes[self.pos] != b'>'
                    {
                        self.pos += 1;
                    }
                    &self.input[start..self.pos]
                }
            };
            attrs.push((name, decode_entities(value)));
        }
    }

    fn filter(&self, attributes: Vec<(String, String)>) -> Vec<(String, String)> {
        if self.all_attributes {
            attributes
        } else {
            filter_attributes(attributes)
        }
    }

    /// At `<` followed by a letter.
    fn start_tag(&mut self) {
        self.pos += 1;
        let tag = self.tag_name();
        let Some((attributes, self_closing)) = self.attributes() else {
            return;
        };
        if self.in_head && !is_head_content(&tag) {
            self.in_head = false;
        }
        match tag.as_str() {
            // Their content is the document's; the tags say nothing.
            "html" | "body" => return,
            "head" => {
                self.in_head = true;
                return;
            }
            "meta" | "link" | "base" | "basefont" | "bgsound" => return,
            "script" | "style" | "title" | "iframe" | "noscript" | "noembed" | "noframes"
            | "template" => {
                if !self_closing || tag != "template" {
                    self.raw_text(&tag);
                }
                return;
            }
            "textarea" => {
                let text = decode_entities(self.raw_text(&tag));
                let children = if text.is_empty() {
                    Vec::new()
                } else {
                    vec![ParsedNode::Text(text)]
                };
                let attributes = self.filter(attributes);
                self.siblings().push(ParsedNode::Element {
                    tag,
                    attributes,
                    children,
                    holds_block: false,
                });
                return;
            }
            "plaintext" => {
                // Everything after it is text.
                let text = self.input[self.pos..].to_string();
                self.pos = self.input.len();
                self.append_text(&text);
                return;
            }
            _ => {}
        }
        self.imply_end_tags(&tag);
        let attributes = self.filter(attributes);
        if self_closing || is_void_tag(&tag) {
            self.siblings().push(ParsedNode::Element {
                tag,
                attributes,
                children: Vec::new(),
                holds_block: false,
            });
        } else if self.stack.len() >= MAX_DEPTH {
            self.unopened += 1;
        } else {
            self.stack.push(Open {
                tag,
                attributes,
                children: Vec::new(),
            });
        }
    }

    /// The text up to the end tag of `tag` (or the end of the input), which
    /// is skipped too.
    fn raw_text(&mut self, tag: &str) -> &'a str {
        let bytes = self.input.as_bytes();
        let start = self.pos;
        let name = tag.as_bytes();
        while self.pos < bytes.len() {
            if bytes[self.pos] == b'<'
                && bytes.get(self.pos + 1) == Some(&b'/')
                && bytes[self.pos + 2..]
                    .get(..name.len())
                    .is_some_and(|b| b.eq_ignore_ascii_case(name))
                && bytes
                    .get(self.pos + 2 + name.len())
                    .is_none_or(|b| b.is_ascii_whitespace() || matches!(b, b'>' | b'/'))
            {
                // `pos` is at a `<`: a char boundary.
                let text = &self.input[start..self.pos];
                self.skip_past(b'>');
                return text;
            }
            self.pos += 1;
        }
        &self.input[start..]
    }

    /// At `</`.
    fn end_tag(&mut self) {
        let bytes = self.input.as_bytes();
        match bytes.get(self.pos + 2) {
            Some(b) if b.is_ascii_alphabetic() => {}
            // `</>` is nothing; `</ x>` is a comment.
            _ => {
                self.pos += 2;
                self.skip_past(b'>');
                return;
            }
        }
        self.pos += 2;
        let tag = self.tag_name();
        self.skip_past(b'>');
        match tag.as_str() {
            "html" | "body" => return,
            "head" => {
                self.in_head = false;
                return;
            }
            // `</br>` is a `<br>`.
            "br" => {
                self.siblings().push(ParsedNode::Element {
                    tag,
                    attributes: Vec::new(),
                    children: Vec::new(),
                    holds_block: false,
                });
                return;
            }
            _ => {}
        }
        if self.unopened > 0 {
            self.unopened -= 1;
            return;
        }
        let heading = is_heading(&tag);
        let table_part = is_table_part(&tag);
        let special = is_special(&tag);
        for i in (0..self.stack.len()).rev() {
            step();
            let open = self.stack[i].tag.as_str();
            if open == tag || (heading && is_heading(open)) {
                self.close_to(i);
                return;
            }
            // What an end tag does not reach across.
            let barrier = match open {
                "table" => tag != "table",
                // A row or a row group with no `<table>` around it stands
                // for its table.
                "td" | "th" | "caption" | "tr" | "tbody" | "thead" | "tfoot" => {
                    tag != "table" && !table_part
                }
                "ul" | "ol" => tag == "li" || !special,
                _ => !special && is_special(open),
            };
            if barrier {
                return;
            }
        }
    }

    /// Close every open element from index `to` up.
    fn close_to(&mut self, to: usize) {
        while self.stack.len() > to {
            let Some(open) = self.stack.pop() else { return };
            self.siblings().push(ParsedNode::Element {
                tag: open.tag,
                attributes: open.attributes,
                children: open.children,
                holds_block: false,
            });
        }
    }

    /// Close the nearest open element `closes` accepts, looking no further
    /// than the first one `stops` accepts.
    fn close_nearest(&mut self, closes: impl Fn(&str) -> bool, stops: impl Fn(&str) -> bool) {
        for i in (0..self.stack.len()).rev() {
            step();
            let open = self.stack[i].tag.as_str();
            if closes(open) {
                self.close_to(i);
                return;
            }
            if stops(open) {
                return;
            }
        }
    }

    /// Close the outermost open element `closes` accepts that is inside the
    /// nearest open `<table>` (or inside none).
    fn close_in_table(&mut self, closes: impl Fn(&str) -> bool) {
        let mut outermost = None;
        for i in (0..self.stack.len()).rev() {
            step();
            let open = self.stack[i].tag.as_str();
            if open == "table" {
                break;
            }
            if closes(open) {
                outermost = Some(i);
            }
        }
        if let Some(i) = outermost {
            self.close_to(i);
        }
    }

    /// The end tags a start tag of `tag` implies.
    fn imply_end_tags(&mut self, tag: &str) {
        if closes_p(tag) {
            self.close_nearest(
                |open| open == "p",
                |open| open == "button" || open == "object" || open == "table" || is_table_part(open),
            );
        }
        match tag {
            "li" => self.close_nearest(
                |open| open == "li",
                |open| is_special(open) && !matches!(open, "address" | "div" | "p"),
            ),
            "dd" | "dt" => self.close_nearest(
                |open| matches!(open, "dd" | "dt"),
                |open| is_special(open) && !matches!(open, "address" | "div" | "p"),
            ),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                if self.stack.last().is_some_and(|open| is_heading(&open.tag)) {
                    self.close_to(self.stack.len() - 1);
                }
            }
            "td" | "th" => self.close_nearest(
                |open| matches!(open, "td" | "th" | "caption" | "colgroup"),
                |open| matches!(open, "tr" | "tbody" | "thead" | "tfoot" | "table"),
            ),
            "tr" => self
                .close_in_table(|open| matches!(open, "tr" | "td" | "th" | "caption" | "colgroup")),
            "tbody" | "thead" | "tfoot" | "caption" | "colgroup" => self.close_in_table(is_table_part),
            _ => {}
        }
    }
}

/// Elements a `<head>` holds; anything else ends it.
fn is_head_content(tag: &str) -> bool {
    matches!(
        tag,
        "head"
            | "meta"
            | "link"
            | "style"
            | "script"
            | "title"
            | "base"
            | "basefont"
            | "bgsound"
            | "noscript"
            | "template"
    )
}

fn is_heading(tag: &str) -> bool {
    matches!(tag, "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
}

/// The parts of a table: elements with a place only inside one.
pub(super) fn is_table_part(tag: &str) -> bool {
    matches!(
        tag,
        "tr" | "td" | "th" | "tbody" | "thead" | "tfoot" | "caption" | "colgroup" | "col"
    )
}

/// Start tags that end an open `<p>`.
fn closes_p(tag: &str) -> bool {
    matches!(
        tag,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "center"
            | "details"
            | "dialog"
            | "dir"
            | "div"
            | "dl"
            | "fieldset"
            | "figcaption"
            | "figure"
            | "footer"
            | "header"
            | "hgroup"
            | "main"
            | "menu"
            | "nav"
            | "ol"
            | "p"
            | "search"
            | "section"
            | "summary"
            | "ul"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "pre"
            | "listing"
            | "form"
            | "table"
            | "hr"
            | "li"
            | "dd"
            | "dt"
            | "xmp"
    )
}

/// HTML's "special" elements: the ones an inline element's end tag does not
/// reach across.
fn is_special(tag: &str) -> bool {
    closes_p(tag)
        || is_table_part(tag)
        || matches!(
            tag,
            "applet"
                | "area"
                | "br"
                | "button"
                | "embed"
                | "frame"
                | "frameset"
                | "img"
                | "input"
                | "keygen"
                | "marquee"
                | "object"
                | "param"
                | "select"
                | "source"
                | "textarea"
                | "track"
                | "wbr"
        )
}

pub(super) fn is_void_tag(tag: &str) -> bool {
    matches!(
        tag,
        "br" | "hr"
            | "img"
            | "input"
            | "wbr"
            | "area"
            | "col"
            | "embed"
            | "source"
            | "track"
            | "param"
            | "keygen"
            | "frame"
    )
}

/// Keep only the safe, schema-relevant attributes — drops `on*` event handlers
/// and everything else.
fn filter_attributes(attrs: Vec<(String, String)>) -> Vec<(String, String)> {
    attrs
        .into_iter()
        .filter(|(name, _)| {
            matches!(
                name.as_str(),
                "href"
                    | "src"
                    | "alt"
                    | "title"
                    | "target"
                    | "start"
                    | "style"
                    | "class"
                    | "colspan"
                    | "rowspan"
                    | super::html::TASK_TYPE
                    | super::html::TASK_CHECKED
            )
        })
        .collect()
}
