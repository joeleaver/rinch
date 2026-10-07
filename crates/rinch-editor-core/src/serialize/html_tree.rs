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
//! - `<html>` and `<body>` tags are skipped (their content stays) with the
//!   whitespace after them and the line end before their end tags, and a
//!   `<head>` ends at its end tag or at the first thing that is not head
//!   content.
//! - End tags are implied as a browser implies them: a `<p>` ends at the next
//!   block, an `<li>` at the next `<li>`, a cell at the next cell or row, a
//!   row at the next row. An end tag closes the nearest open element of its
//!   name, unless a table cell (or, for an inline element, a block) is in
//!   between; one that closes nothing is skipped.
//! - The end tag of a formatting element (`<b>`, `<i>`, `<a>`, …: the HTML
//!   parser's list) that has a block open between it and its start tag ends
//!   the element there, as a browser's adoption agency algorithm does: what
//!   the open elements in between hold so far keeps the formatting, what is
//!   read after it does not. With more than [`MAX_ADOPT_BLOCKS`] blocks open
//!   in between (a browser's limit), or more than [`MAX_ADOPT`] elements of
//!   any kind (this reader's), the end tag is skipped.
//! - A `</p>` with no `<p>` to close ends the line (in a browser it is an
//!   empty `<p>`).
//! - `<textarea>` holds text, not markup.
//! - The tree is at most [`MAX_DEPTH`] elements deep (two more for a block
//!   read past it and an element with no content in that), because everything that walks it recurses. An element
//!   opened deeper is still open, so its end tag is its own, but it is
//!   flattened into the element at the limit: an inline one gives its
//!   content and no element (its mark is lost), a block is a child of the
//!   element at the limit, split around the blocks inside it, and what the
//!   reader drops whole (an `<svg>`) is dropped. So no depth changes the text or
//!   where a line ends; what is lost past the limit is nesting.
//! - An open `<svg>` or `<math>` ends at the first HTML start tag the HTML
//!   parser ends foreign content at (`<p>`, `<div>`, `<span>`, `<b>`, …),
//!   unless a `<foreignObject>`, `<desc>` or `<annotation-xml>` is open
//!   inside it; and a start tag inside one implies no end tag outside it.
//! - A tag finds the element it closes however many are open: it looks
//!   through the innermost [`MAX_SCAN`], and past those asks an index of
//!   where each name is open, so no tag costs more than a fixed amount.
//!
//! Not followed: a self-closing `<x/>` is an empty element whatever `x` is
//! (in HTML only a void element is), a formatting element that a block's end
//! closes is not opened again for what follows (a browser reconstructs its
//! active formatting elements: `<p><b>a</p>b` is two bold lines there), and
//! content that has no place in a table stays where it is written for the
//! reader to move.

use super::html_entities::{decode_attribute, decode_entities};

/// How deep the tree is built. A valid document is far shallower (a list
/// costs three elements a level, a quote one), and Chrome's own limit
/// (`kMaximumHTMLParserDOMTreeDepth`) is 512, past which it too keeps every
/// element and stops nesting them. This one is lower because of what a level
/// costs here: the reader, the fitter and everything else that walks a
/// document recurse once or more per level, an unoptimized build spends
/// about 6.4 KB of stack a level reading nested quotes or lists (1.1 KB
/// optimized; measured on x86-64), so this depth reads in 1.25 MB where a
/// spawned thread has 2 MB, and 512 would not. Pinned by
/// `the_depth_limit_reads_on_a_small_stack`.
pub(super) const MAX_DEPTH: usize = 192;

/// The most open elements a tag looks through one by one for the one it
/// closes. A well nested document closes the innermost. Past these the
/// answer comes from `open_at` — the same answer, at a fixed cost — so
/// thousands of open elements make a tag no dearer and change nothing it
/// closes.
pub(super) const MAX_SCAN: usize = MAX_DEPTH;

/// The most blocks that may be open above a formatting element for its
/// misnested end tag to end it ([`HtmlFragmentParser::end_under`]). Chrome
/// 153's number, measured: with 7 `<div>` open between `<b>` and `</b>` the
/// bold ends at the end tag, with 8 it runs on (its adoption agency loop
/// runs 8 times, one a block and one to finish).
const MAX_ADOPT_BLOCKS: usize = 7;

/// The most elements of any kind that may be open above it. Each is
/// visited, and each that holds content gets a copy of the element around
/// it, so this bounds what one end tag costs and how many copies of one
/// element's attributes are made. A browser has no such limit.
const MAX_ADOPT: usize = 32;

/// Every name [`is_special`] or [`is_foreign`] accepts: the only elements
/// that an implied end tag closes, that stop one, or that an end tag does
/// not reach across. What the index is asked about.
const SCOPE_NAMES: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "center",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "header",
    "hgroup",
    "main",
    "menu",
    "nav",
    "ol",
    "p",
    "search",
    "section",
    "summary",
    "ul",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "pre",
    "listing",
    "form",
    "table",
    "hr",
    "li",
    "dd",
    "dt",
    "xmp",
    "tr",
    "td",
    "th",
    "tbody",
    "thead",
    "tfoot",
    "caption",
    "colgroup",
    "col",
    "applet",
    "area",
    "br",
    "button",
    "embed",
    "frame",
    "frameset",
    "img",
    "input",
    "keygen",
    "marquee",
    "object",
    "param",
    "select",
    "source",
    "textarea",
    "track",
    "wbr",
    "svg",
    "math",
    "foreignobject",
    "desc",
    "annotation-xml",
];

thread_local! {
    static STEPS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// [`MAX_DEPTH`], for tests.
#[doc(hidden)]
pub fn html_reader_max_depth() -> usize {
    MAX_DEPTH
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
    steps(1);
}

fn steps(n: u64) {
    STEPS.with(|c| c.set(c.get() + n));
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
    /// Empty for an element that was ended while elements above it were
    /// open ([`HtmlFragmentParser::end_under`]): what closes under it from
    /// then on is its parent's content.
    tag: String,
    attributes: Vec<(String, String)>,
    /// For an element past [`MAX_DEPTH`] that is flattened as a block: the
    /// part of it being read. Empty for one flattened as inline; for one
    /// the reader drops, everything in it, which goes nowhere.
    children: Vec<ParsedNode>,
    /// Past [`MAX_DEPTH`]: a part of this block has been handed over, or a
    /// block inside it has, so an empty rest is nothing.
    split: bool,
}

pub(super) struct HtmlFragmentParser<'a> {
    input: &'a str,
    pos: usize,
    /// Keep every attribute rather than [`filter_attributes`]' set: for
    /// `dropped_table_attr`, which reports what the filter would drop.
    pub(super) all_attributes: bool,
    root: Vec<ParsedNode>,
    stack: Vec<Open>,
    /// The open elements past [`MAX_DEPTH`] that are flattened as blocks,
    /// by index in `stack`, outermost first.
    flat_blocks: Vec<usize>,
    /// The index of an open element past [`MAX_DEPTH`] that the reader drops
    /// with its content: what is read inside it is not kept.
    flat_dropped: Option<usize>,
    /// How many `<svg>` and `<math>` elements are open.
    foreign: usize,
    /// Where each tag name is open: indices into `stack`, ascending.
    open_at: std::collections::HashMap<String, Vec<usize>>,
    /// Inside a `<head>`.
    in_head: bool,
    /// An `<html>`, `<head>` or `<body>` tag was the last tag read: the
    /// whitespace after it is the source's layout, not content.
    after_document_tag: bool,
}

impl<'a> HtmlFragmentParser<'a> {
    pub(super) fn new(input: &'a str) -> Self {
        Self {
            input,
            pos: 0,
            all_attributes: false,
            root: Vec::new(),
            stack: Vec::new(),
            flat_blocks: Vec::new(),
            flat_dropped: None,
            foreign: 0,
            open_at: std::collections::HashMap::new(),
            in_head: false,
            after_document_tag: false,
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
        let mut raw = &self.input[start..self.pos];
        if self.after_document_tag {
            raw = raw.trim_start_matches(|c: char| c.is_ascii_whitespace());
            if raw.is_empty() {
                return;
            }
            self.after_document_tag = false;
        }
        let text = decode_entities(raw);
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

    /// Before `</body>` or `</html>`: the line end the source puts in front
    /// of the tag (with the whitespace around it) is its layout, not text.
    fn trim_line_end(&mut self) {
        let siblings = self.siblings();
        let Some(ParsedNode::Text(last)) = siblings.last_mut() else {
            return;
        };
        let kept = last
            .trim_end_matches(|c: char| c.is_ascii_whitespace())
            .len();
        if last[kept..].contains('\n') {
            last.truncate(kept);
            if last.is_empty() {
                siblings.pop();
            }
        }
    }

    /// Where content read now goes: the innermost open element, or past
    /// [`MAX_DEPTH`] the innermost one flattened as a block, or the element
    /// at the limit.
    fn siblings(&mut self) -> &mut Vec<ParsedNode> {
        let len = self.stack.len();
        if len == 0 {
            return &mut self.root;
        }
        let at = if len <= MAX_DEPTH {
            len - 1
        } else if let Some(dropped) = self.flat_dropped {
            dropped
        } else {
            self.flat_blocks.last().copied().unwrap_or(MAX_DEPTH - 1)
        };
        &mut self.stack[at].children
    }

    /// Open an element.
    fn open(&mut self, tag: String, attributes: Vec<(String, String)>) {
        if matches!(tag.as_str(), "svg" | "math") {
            self.foreign += 1;
        }
        let at = self.stack.len();
        if at >= MAX_DEPTH && self.flat_dropped.is_none() {
            if super::html::is_dropped(&tag) {
                self.flat_dropped = Some(at);
            } else if is_flat_block(&tag) {
                self.end_flat_part();
                self.flat_blocks.push(at);
            }
        }
        debug_assert!(
            !(is_special(&tag) || is_foreign(&tag)) || SCOPE_NAMES.contains(&tag.as_str()),
            "<{tag}> is missing from SCOPE_NAMES"
        );
        match self.open_at.get_mut(&tag) {
            Some(indices) => indices.push(at),
            None => {
                self.open_at.insert(tag.clone(), vec![at]);
            }
        }
        self.stack.push(Open {
            tag,
            attributes,
            children: Vec::new(),
            split: false,
        });
    }

    /// The innermost open element named `name`.
    fn last_open(&self, name: &str) -> Option<usize> {
        step();
        self.open_at.get(name)?.last().copied()
    }

    /// The innermost open element whose name `accepts` accepts, among the
    /// names that matter to implied and misnested end tags
    /// ([`SCOPE_NAMES`]).
    fn nearest_scope(&self, accepts: impl Fn(&str) -> bool) -> Option<usize> {
        SCOPE_NAMES
            .iter()
            .filter(|name| accepts(name))
            .filter_map(|name| self.last_open(name))
            .max()
    }

    /// Add an element with no end tag to read.
    fn push_closed(&mut self, tag: String, attributes: Vec<(String, String)>, text: String) {
        let children = if text.is_empty() {
            Vec::new()
        } else {
            vec![ParsedNode::Text(text)]
        };
        self.siblings().push(ParsedNode::Element {
            tag,
            attributes,
            children,
            holds_block: false,
        });
    }

    /// A block starts inside the innermost flattened block: what that one
    /// has read so far is a part of its own, in front of the new block.
    fn end_flat_part(&mut self) {
        let Some(&at) = self.flat_blocks.last() else {
            return;
        };
        let open = &mut self.stack[at];
        open.split = true;
        if open.children.is_empty() {
            return;
        }
        let children = std::mem::take(&mut open.children);
        // The block's attributes go with its first part that holds anything,
        // and are moved there: a copy for every part would cost the
        // attributes once for every block inside the block.
        let whitespace =
            matches!(children.as_slice(), [ParsedNode::Text(text)] if text.trim().is_empty());
        let attributes = if whitespace {
            Vec::new()
        } else {
            std::mem::take(&mut open.attributes)
        };
        let tag = open.tag.clone();
        self.push_flat(tag, attributes, children, true);
    }

    /// Hand a flattened block (or a part of one) to the element at the limit.
    /// Nothing for an empty part; whitespace between two blocks stays that.
    fn push_flat(
        &mut self,
        tag: String,
        attributes: Vec<(String, String)>,
        mut children: Vec<ParsedNode>,
        part: bool,
    ) {
        let into = &mut self.stack[MAX_DEPTH - 1].children;
        if part {
            match children.as_mut_slice() {
                [] => return,
                [ParsedNode::Text(text)] if text.trim().is_empty() => {
                    if let Some(ParsedNode::Text(last)) = into.last_mut() {
                        last.push_str(text);
                    } else {
                        into.append(&mut children);
                    }
                    return;
                }
                _ => {}
            }
        }
        let bytes: usize = attributes.iter().map(|(k, v)| k.len() + v.len()).sum();
        steps((bytes / 64) as u64);
        into.push(ParsedNode::Element {
            tag,
            attributes,
            children,
            holds_block: false,
        });
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
            attrs.push((name, decode_attribute(value)));
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
        self.after_document_tag = matches!(tag.as_str(), "html" | "head" | "body");
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
                let attributes = self.filter(attributes);
                self.push_closed(tag, attributes, text);
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
        if self.foreign > 0 && ends_foreign_content(&tag) {
            self.end_foreign_content();
        }
        self.imply_end_tags(&tag);
        let attributes = self.filter(attributes);
        if self_closing || is_void_tag(&tag) {
            self.push_closed(tag, attributes, String::new());
        } else {
            self.open(tag, attributes);
        }
    }

    /// An HTML start tag inside an `<svg>` or `<math>`: the element ends
    /// there, unless the tag is inside a part of it that holds HTML.
    fn end_foreign_content(&mut self) {
        for i in (0..self.stack.len()).rev().take(MAX_SCAN) {
            step();
            match self.stack[i].tag.as_str() {
                "svg" | "math" => {
                    self.close_to(i);
                    return;
                }
                open if is_foreign(open) => return,
                _ => {}
            }
        }
        if self.stack.len() > MAX_SCAN
            && let Some(i) = self.nearest_scope(is_foreign)
            && matches!(self.stack[i].tag.as_str(), "svg" | "math")
        {
            self.close_to(i);
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
        self.after_document_tag = matches!(tag.as_str(), "html" | "head" | "body");
        match tag.as_str() {
            "html" | "body" => {
                self.trim_line_end();
                return;
            }
            "head" => {
                self.in_head = false;
                return;
            }
            // `</br>` is a `<br>`.
            "br" => {
                self.push_closed(tag, Vec::new(), String::new());
                return;
            }
            _ => {}
        }
        let heading = is_heading(&tag);
        let table_part = is_table_part(&tag);
        let special = is_special(&tag);
        let formatting = is_formatting(&tag);
        // The part of an `<svg>` or `<math>` that holds HTML is a scope too.
        // Across one, with no block open, the end tag still closes its
        // element and the drawing with it, as it always has: what follows
        // would be read into the drawing otherwise, and dropped with it.
        let drawing_scope = |open: &str| {
            self.foreign > 0 && matches!(open, "foreignobject" | "desc" | "annotation-xml")
        };
        // What an end tag does not reach across.
        let barrier = |open: &str| match open {
            "table" => tag != "table",
            // A row or a row group with no `<table>` around it stands for
            // its table.
            "td" | "th" | "caption" | "tr" | "tbody" | "thead" | "tfoot" => {
                tag != "table" && !table_part
            }
            // A formatting element's end tag reaches across a block
            // (`end_under`), and not out of what HTML calls a scope.
            _ if formatting => matches!(open, "applet" | "marquee" | "object"),
            "ul" | "ol" => tag == "li" || !special,
            _ => !special && is_special(open),
        };
        // The element the tag ends, and whether a block is open above it.
        let mut found = None;
        let mut across_block = false;
        let mut across_scope = false;
        let mut scanned = false;
        for i in (0..self.stack.len()).rev().take(MAX_SCAN) {
            step();
            let open = self.stack[i].tag.as_str();
            if open == tag || (heading && is_heading(open)) {
                found = Some(i);
                scanned = true;
                break;
            }
            if barrier(open) {
                scanned = true;
                break;
            }
            across_block |= is_special(open);
            across_scope |= drawing_scope(open);
        }
        if !scanned && self.stack.len() > MAX_SCAN {
            // Further up than the scan looks: the same rule, from the index.
            let nearest = if heading {
                self.nearest_scope(is_heading)
            } else {
                self.last_open(&tag)
            };
            if let Some(nearest) = nearest
                && self.nearest_scope(barrier).is_none_or(|b| b <= nearest)
            {
                found = Some(nearest);
                across_block = self.nearest_scope(is_special).is_some_and(|b| b > nearest);
                across_scope = self
                    .nearest_scope(drawing_scope)
                    .is_some_and(|b| b > nearest);
            }
        }
        match found {
            // Out of a drawing's HTML part a browser skips the end tag.
            Some(_) if formatting && across_block && across_scope => {}
            Some(found) if formatting && across_block => self.end_under(found),
            Some(found) => self.close_to(found),
            // In a browser a `</p>` with no `<p>` to close is an empty
            // `<p>`: the line ends there. An empty block that is no
            // paragraph says that and no more (an empty paragraph would be
            // a blank line of its own).
            None if tag == "p" => self.push_closed("div".to_string(), Vec::new(), String::new()),
            None => {}
        }
    }

    /// The end tag of the formatting element at `at`, read while a block is
    /// open above it (`<b>a<div>b</b>c</div>d`). As in a browser, the
    /// element ends there and what it held stays formatted: every element
    /// open above it gets a copy of it around what that element holds so
    /// far, and what is read from here on is outside it. An element open
    /// above the innermost block that is no formatting element ends too.
    ///
    /// The tree gets no deeper: the element leaves every path it was on,
    /// and a copy joins each such path once.
    fn end_under(&mut self, at: usize) {
        if self.stack.len() - 1 - at > MAX_ADOPT {
            return;
        }
        let blocks = self.stack[at + 1..]
            .iter()
            .filter(|open| is_special(&open.tag))
            .count();
        steps(blocks as u64);
        if blocks > MAX_ADOPT_BLOCKS {
            return;
        }
        // Above the innermost block: a browser pops these, and opens the
        // formatting elements among them again for what follows.
        let mut i = self.stack.len() - 1;
        while i > at {
            step();
            let open = self.stack[i].tag.as_str();
            if is_special(open)
                || is_foreign(open)
                || is_flat_block(open)
                || super::html::is_dropped(open)
            {
                break;
            }
            if !open.is_empty() && !is_formatting(open) {
                self.end_one_under(i);
            }
            // Closing the innermost element takes the ended ones under it
            // along; the one at `at` is not ended yet.
            i = (i - 1).min(self.stack.len() - 1);
        }
        self.end_one_under(at);
    }

    /// End the element at `at` and leave what is open above it open.
    fn end_one_under(&mut self, at: usize) {
        if at + 1 == self.stack.len() {
            self.close_to(at);
            return;
        }
        let tag = std::mem::take(&mut self.stack[at].tag);
        let attributes = std::mem::take(&mut self.stack[at].attributes);
        // It is the innermost open element of its name.
        let unlisted = self.open_at.get_mut(&tag).and_then(Vec::pop);
        debug_assert_eq!(unlisted, Some(at));
        // Past the depth limit an inline element is no element: it has
        // nothing to keep.
        if at >= MAX_DEPTH {
            return;
        }
        let bytes: usize = attributes.iter().map(|(k, v)| k.len() + v.len()).sum();
        for open in &mut self.stack[at..] {
            step();
            if open.children.is_empty() {
                continue;
            }
            steps((bytes / 64) as u64);
            open.children = vec![ParsedNode::Element {
                tag: tag.clone(),
                attributes: attributes.clone(),
                children: std::mem::take(&mut open.children),
                holds_block: false,
            }];
        }
    }

    /// Close every open element from index `to` up.
    fn close_to(&mut self, to: usize) {
        while self.stack.len() > to {
            self.pop_open();
        }
        // An element ended under the ones just closed (`end_under`) has
        // nothing open above it now: it is no longer in the way.
        while self.stack.last().is_some_and(|open| open.tag.is_empty()) {
            self.pop_open();
        }
    }

    /// Close the innermost open element.
    fn pop_open(&mut self) {
        let Some(mut open) = self.stack.pop() else {
            return;
        };
        if matches!(open.tag.as_str(), "svg" | "math") {
            self.foreign -= 1;
        }
        if let Some(indices) = self.open_at.get_mut(&open.tag) {
            indices.pop();
        }
        let at = self.stack.len();
        if open.tag.is_empty() {
            // Ended earlier: what it holds is its parent's.
            if at < MAX_DEPTH && !open.children.is_empty() {
                steps((open.children.len() / 8) as u64);
                // (It is closed as soon as the last element open above it
                // is, so these are the elements that closed under it.)
                self.siblings().append(&mut open.children);
            }
            return;
        }
        if at < MAX_DEPTH {
            self.siblings().push(ParsedNode::Element {
                tag: open.tag,
                attributes: open.attributes,
                children: open.children,
                holds_block: false,
            });
            return;
        }
        // Flattened: an inline element gave its content where it stood,
        // and a dropped one kept its own to itself.
        if self.flat_dropped == Some(at) {
            self.flat_dropped = None;
        } else if self.flat_blocks.last() == Some(&at) {
            self.flat_blocks.pop();
            self.push_flat(open.tag, open.attributes, open.children, open.split);
        }
    }

    /// Close the nearest open element `closes` accepts, looking no further
    /// than the first one `stops` accepts.
    fn close_nearest(&mut self, closes: impl Fn(&str) -> bool, stops: impl Fn(&str) -> bool) {
        for i in (0..self.stack.len()).rev().take(MAX_SCAN) {
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
        if self.stack.len() > MAX_SCAN
            && let Some(found) = self.nearest_scope(&closes)
            && self.nearest_scope(&stops).is_none_or(|stop| stop <= found)
        {
            self.close_to(found);
        }
    }

    /// Close the outermost open element `closes` accepts that is inside the
    /// nearest open `<table>` (or inside none).
    fn close_in_table(&mut self, closes: impl Fn(&str) -> bool) {
        if self.stack.len() > MAX_SCAN {
            let inside = self.last_open("table").map_or(0, |table| table + 1);
            let outermost = SCOPE_NAMES
                .iter()
                .filter(|name| closes(name))
                .filter_map(|name| {
                    step();
                    let indices = self.open_at.get(*name)?;
                    indices
                        .get(indices.partition_point(|i| *i < inside))
                        .copied()
                })
                .min();
            if let Some(i) = outermost {
                self.close_to(i);
            }
            return;
        }
        let mut outermost = None;
        for i in (0..self.stack.len()).rev().take(MAX_SCAN) {
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
                |open| {
                    open == "button"
                        || open == "object"
                        || open == "table"
                        || is_table_part(open)
                        || is_foreign(open)
                },
            );
        }
        match tag {
            "li" => self.close_nearest(
                |open| open == "li",
                |open| {
                    is_foreign(open) || is_special(open) && !matches!(open, "address" | "div" | "p")
                },
            ),
            "dd" | "dt" => self.close_nearest(
                |open| matches!(open, "dd" | "dt"),
                |open| {
                    is_foreign(open) || is_special(open) && !matches!(open, "address" | "div" | "p")
                },
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
            "tbody" | "thead" | "tfoot" | "caption" | "colgroup" => {
                self.close_in_table(is_table_part)
            }
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

/// Past [`MAX_DEPTH`]: an element kept as an element, beside the blocks
/// inside it, because it is a line of its own.
fn is_flat_block(tag: &str) -> bool {
    closes_p(tag) || super::html::is_block_level(tag)
}

/// The HTML start tags that end an open `<svg>` or `<math>` (the HTML
/// parser's list for foreign content).
fn ends_foreign_content(tag: &str) -> bool {
    matches!(
        tag,
        "b" | "big"
            | "blockquote"
            | "br"
            | "center"
            | "code"
            | "dd"
            | "div"
            | "dl"
            | "dt"
            | "em"
            | "embed"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "hr"
            | "i"
            | "img"
            | "li"
            | "listing"
            | "menu"
            | "nobr"
            | "ol"
            | "p"
            | "pre"
            | "ruby"
            | "s"
            | "small"
            | "span"
            | "strong"
            | "strike"
            | "sub"
            | "sup"
            | "table"
            | "tt"
            | "u"
            | "ul"
            | "var"
    )
}

/// `<svg>`, `<math>` and the elements inside them that hold HTML: an HTML
/// start tag inside one implies no end tag outside it.
fn is_foreign(tag: &str) -> bool {
    matches!(
        tag,
        "svg" | "math" | "foreignobject" | "desc" | "annotation-xml"
    )
}

/// HTML's formatting elements: the ones whose end tag a browser lets end
/// the element across an open block (the adoption agency algorithm).
fn is_formatting(tag: &str) -> bool {
    matches!(
        tag,
        "a" | "b"
            | "big"
            | "code"
            | "em"
            | "font"
            | "i"
            | "nobr"
            | "s"
            | "small"
            | "strike"
            | "strong"
            | "tt"
            | "u"
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

#[cfg(test)]
mod tests {
    use super::*;

    /// [`SCOPE_NAMES`] is exactly the names [`is_special`] and
    /// [`is_foreign`] accept: a name missing from it would be invisible to
    /// a tag that looks past [`MAX_SCAN`] open elements.
    #[test]
    fn scope_names_are_the_special_and_foreign_elements() {
        for name in SCOPE_NAMES {
            assert!(is_special(name) || is_foreign(name), "{name}");
        }
        // Every HTML element name, and the foreign ones: opening one that is
        // special and not listed trips the assertion in `open`.
        let all = "a abbr acronym address applet area article aside audio b base basefont bdi bdo \
                   bgsound big blink blockquote body br button canvas caption center cite code col \
                   colgroup data datalist dd del details dfn dialog dir div dl dt em embed fieldset \
                   figcaption figure font footer form frame frameset h1 h2 h3 h4 h5 h6 head header \
                   hgroup hr html i iframe img input ins kbd keygen label legend li link listing \
                   main map mark marquee menu meta meter nav nobr noembed noframes noscript object \
                   ol optgroup option output p param picture plaintext pre progress q rb rp rt rtc \
                   ruby s samp script search section select slot small source span strike strong \
                   style sub summary sup table tbody td template textarea tfoot th thead time title \
                   tr track tt u ul var video wbr xmp svg math foreignobject desc annotation-xml";
        let mut listed = 0;
        for name in all.split_whitespace() {
            if is_special(name) || is_foreign(name) {
                assert!(SCOPE_NAMES.contains(&name), "{name}");
                listed += 1;
            }
            HtmlFragmentParser::new(&format!("<{name}>x")).parse();
        }
        assert_eq!(listed, SCOPE_NAMES.len());
    }
}
