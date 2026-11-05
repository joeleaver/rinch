//! A document's nesting, cut short before anything recursive reads it.
//!
//! css-inline serializes recursively: ~16k nested elements (80 KB) overflow
//! an 8 MB stack and abort the process, and rinch lays out recursively too.
//! Mail is a few dozen levels deep, so [`cap`] leaves out every element
//! nested deeper than [`MAX_DEPTH`].
//!
//! The depth is the real one: the document is parsed with html5ever, as a
//! browser parses it (so `<div/>` opens a div, an end tag past a cell
//! boundary is ignored, and unclosed formatting elements are reopened), into
//! a flat arena ([`Arena`]), and written back with html5ever's serializer
//! from an explicit stack. Nothing here recurses on the document's depth.
//!
//! html5ever's tree builder itself slows with depth (it walks its stack of
//! open elements: 320 KB of unclosed `<div>`s took 6 s), so a linear pass
//! first ([`open_bound`]) counts how many elements could be open at each
//! tag, generously, and cuts the input where that passes [`MAX_OPEN`].

use std::borrow::Cow;
use std::cell::{Ref, RefCell};

use html5ever::serialize::{Serialize, SerializeOpts, Serializer, TraversalScope};
use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::{Attribute, QualName, parse_document};

/// The deepest nesting kept, in elements below the document.
pub const MAX_DEPTH: usize = 512;

/// Where the input is cut before it is parsed: past this many elements that
/// could be open at once ([`open_bound`]).
pub const MAX_OPEN: usize = 1024;

/// `document` with every element nested deeper than [`MAX_DEPTH`] left out,
/// borrowed when nothing is. A document that is cut comes back as html5ever
/// writes a parsed document (`<html><head>…<body>…`).
pub(crate) fn cap(document: &str) -> Cow<'_, str> {
    // A document cannot nest deeper than it has `<`s.
    if document.bytes().filter(|&b| b == b'<').count() <= MAX_DEPTH {
        return Cow::Borrowed(document);
    }
    let cut = open_bound(document);
    let input = &document[..cut.unwrap_or(document.len())];
    let arena = parse_document(Arena::default(), Default::default()).one(input);
    if cut.is_none() && arena.depth() <= MAX_DEPTH {
        return Cow::Borrowed(document);
    }
    let mut out = Vec::with_capacity(document.len());
    let opts = SerializeOpts {
        traversal_scope: TraversalScope::ChildrenOnly(None),
        ..Default::default()
    };
    if html5ever::serialize(&mut out, &arena, opts).is_err() {
        return Cow::Owned(String::new());
    }
    Cow::Owned(String::from_utf8(out).unwrap_or_default())
}

/// Where in `document` more than [`MAX_OPEN`] elements could be open at
/// once, if anywhere. Linear, and generous: every start tag of an element
/// that is not void opens one (`<div/>` too, as the parser has it), the
/// common implied closes close (a `<p>` the next `<p>`, a cell the next
/// cell), and an end tag closes only an element of its name it can reach:
/// one of the top 64, with no table, cell, caption, template or object in
/// between (where the parser would ignore it). Misnested mail counts high,
/// never low.
pub(crate) fn open_bound(document: &str) -> Option<usize> {
    const REACH: usize = 64;
    let bytes = document.as_bytes();
    let mut stack: Vec<String> = Vec::new();
    let mut i = 0;
    while let Some(lt) = memchr(b'<', bytes, i) {
        if bytes[lt..].starts_with(b"<!--") {
            i = find_ascii(bytes, lt + 4, b"-->").map_or(bytes.len(), |p| p + 3);
            continue;
        }
        let Some(gt) = memchr(b'>', bytes, lt) else {
            break;
        };
        let tag = &document[lt + 1..gt];
        i = gt + 1;
        if let Some(name) = tag.strip_prefix('/') {
            let name = tag_name(name);
            let reach = stack
                .iter()
                .rev()
                .take(REACH)
                .position(|n| *n == name || (n != &name && is_scope_boundary(n)));
            if let Some(from_top) = reach
                && stack[stack.len() - 1 - from_top] == name
            {
                let at = stack.len() - 1 - from_top;
                // Formatting elements closed out from under a formatting
                // element are reopened and stay open (`<b><i></b>`): they
                // count again. Under a block they are only rebuilt inside
                // what follows it.
                let reopened: Vec<String> = if is_formatting(&name) {
                    stack[at + 1..]
                        .iter()
                        .filter(|n| is_formatting(n))
                        .cloned()
                        .collect()
                } else {
                    Vec::new()
                };
                stack.truncate(at);
                stack.extend(reopened);
                if stack.len() > MAX_OPEN {
                    return Some(lt);
                }
            }
            continue;
        }
        if !bytes.get(lt + 1).is_some_and(u8::is_ascii_alphabetic) {
            continue;
        }
        let name = tag_name(tag);
        if is_raw_text(&name) {
            let mut close = b"</".to_vec();
            close.extend_from_slice(name.as_bytes());
            i = find_ascii(bytes, i, &close).map_or(bytes.len(), |p| p + close.len());
            continue;
        }
        if is_void(&name) {
            continue;
        }
        close_implied(&mut stack, &name);
        stack.push(name);
        if stack.len() > MAX_OPEN {
            return Some(lt);
        }
    }
    None
}

fn memchr(needle: u8, bytes: &[u8], from: usize) -> Option<usize> {
    bytes
        .get(from..)?
        .iter()
        .position(|&b| b == needle)
        .map(|p| from + p)
}

/// Where `needle` (ASCII) starts in `bytes` from `from`, any case.
fn find_ascii(bytes: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    bytes
        .get(from..)?
        .windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle))
        .map(|p| from + p)
}

fn tag_name(tag: &str) -> String {
    tag.split(|c: char| c.is_whitespace() || c == '/' || c == '>')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn is_raw_text(name: &str) -> bool {
    matches!(
        name,
        "script" | "style" | "textarea" | "title" | "xmp" | "iframe" | "noembed" | "noframes"
    )
}

/// The formatting elements HTML's parser reopens when they are misnested.
fn is_formatting(name: &str) -> bool {
    matches!(
        name,
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

fn is_scope_boundary(name: &str) -> bool {
    matches!(
        name,
        "table" | "td" | "th" | "caption" | "template" | "object" | "applet" | "marquee" | "html"
    )
}

fn is_void(name: &str) -> bool {
    matches!(
        name,
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
            | "source"
            | "track"
            | "wbr"
    )
}

/// Close what opening `name` closes in the parser: the nearest open element
/// of the kinds it ends, with everything above it, unless one of the
/// elements that bound it comes first (a `<td>` ends the open cell, not one
/// past its table; an `<li>` ends the open item, not one past its list).
fn close_implied(stack: &mut Vec<String>, name: &str) {
    const REACH: usize = 64;
    let (ends, bounds): (&[&str], &[&str]) = match name {
        "td" | "th" => (&["td", "th"], &["table"]),
        "tr" => (&["tr"], &["table"]),
        "tbody" | "thead" | "tfoot" => (&["tbody", "thead", "tfoot"], &["table"]),
        "li" => (&["li"], &["ul", "ol", "table", "td", "th"]),
        "dt" | "dd" => (&["dt", "dd"], &["dl", "table", "td", "th"]),
        "option" => (&["option"], &["select"]),
        "p" | "div" | "table" | "ul" | "ol" | "dl" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
        | "blockquote" | "pre" | "center" | "section" | "header" | "footer" | "form" => (
            &["p"],
            &["table", "td", "th", "button", "caption", "template", "html"],
        ),
        _ => return,
    };
    let found = stack
        .iter()
        .rev()
        .take(REACH)
        .position(|n| ends.contains(&n.as_str()) || bounds.contains(&n.as_str()));
    if let Some(from_top) = found {
        let at = stack.len() - 1 - from_top;
        if ends.contains(&stack[at].as_str()) {
            stack.truncate(at);
        }
    }
}

type Id = usize;

enum Kind {
    Document,
    Doctype(String),
    Element {
        name: QualName,
        attrs: Vec<Attribute>,
        /// A `<template>`'s contents, a fragment of its own.
        contents: Option<Id>,
    },
    Text(StrTendril),
    Comment(StrTendril),
    Pi(StrTendril, StrTendril),
}

struct Node {
    kind: Kind,
    parent: Option<Id>,
    children: Vec<Id>,
}

/// A parsed document as html5ever builds it, in one vector.
pub(crate) struct Arena {
    nodes: RefCell<Vec<Node>>,
}

impl Default for Arena {
    fn default() -> Self {
        Self {
            nodes: RefCell::new(vec![Node {
                kind: Kind::Document,
                parent: None,
                children: Vec::new(),
            }]),
        }
    }
}

impl Arena {
    fn push(&self, kind: Kind) -> Id {
        let mut nodes = self.nodes.borrow_mut();
        nodes.push(Node {
            kind,
            parent: None,
            children: Vec::new(),
        });
        nodes.len() - 1
    }

    /// Put `child` under `parent` at `at` (the end when `None`), joining text
    /// to a text node just before it, as the parser expects.
    fn insert(&self, parent: Id, at: Option<usize>, child: NodeOrText<Id>) {
        let id = match child {
            NodeOrText::AppendNode(id) => id,
            NodeOrText::AppendText(text) => {
                let before = {
                    let nodes = self.nodes.borrow();
                    match at {
                        Some(i) => i.checked_sub(1).map(|i| nodes[parent].children[i]),
                        None => nodes[parent].children.last().copied(),
                    }
                };
                if let Some(b) = before
                    && let Kind::Text(t) = &mut self.nodes.borrow_mut()[b].kind
                {
                    t.push_tendril(&text);
                    return;
                }
                self.push(Kind::Text(text))
            }
        };
        self.detach(id);
        let mut nodes = self.nodes.borrow_mut();
        nodes[id].parent = Some(parent);
        let children = &mut nodes[parent].children;
        match at {
            Some(i) => children.insert(i, id),
            None => children.push(id),
        }
    }

    fn detach(&self, id: Id) {
        let mut nodes = self.nodes.borrow_mut();
        if let Some(p) = nodes[id].parent.take() {
            nodes[p].children.retain(|&c| c != id);
        }
    }

    /// The deepest element, counted from the document.
    fn depth(&self) -> usize {
        let nodes = self.nodes.borrow();
        let mut deepest = 0;
        let mut stack = vec![(0usize, 0usize)];
        while let Some((id, depth)) = stack.pop() {
            deepest = deepest.max(depth);
            let contents = match &nodes[id].kind {
                Kind::Element { contents, .. } => *contents,
                _ => None,
            };
            let below = depth + usize::from(matches!(nodes[id].kind, Kind::Element { .. }));
            stack.extend(nodes[id].children.iter().map(|&c| (c, below)));
            stack.extend(contents.map(|c| (c, below)));
        }
        deepest
    }
}

impl TreeSink for Arena {
    type Handle = Id;
    type Output = Self;
    type ElemName<'a> = Ref<'a, QualName>;

    fn finish(self) -> Self {
        self
    }

    fn parse_error(&self, _msg: Cow<'static, str>) {}

    fn get_document(&self) -> Id {
        0
    }

    fn elem_name<'a>(&'a self, target: &'a Id) -> Ref<'a, QualName> {
        Ref::map(self.nodes.borrow(), |n| match &n[*target].kind {
            Kind::Element { name, .. } => name,
            _ => unreachable!("html5ever asks only elements their name"),
        })
    }

    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> Id {
        let contents = flags.template.then(|| self.push(Kind::Document));
        self.push(Kind::Element {
            name,
            attrs,
            contents,
        })
    }

    fn create_comment(&self, text: StrTendril) -> Id {
        self.push(Kind::Comment(text))
    }

    fn create_pi(&self, target: StrTendril, data: StrTendril) -> Id {
        self.push(Kind::Pi(target, data))
    }

    fn append(&self, parent: &Id, child: NodeOrText<Id>) {
        self.insert(*parent, None, child);
    }

    fn append_based_on_parent_node(&self, element: &Id, prev: &Id, child: NodeOrText<Id>) {
        if self.nodes.borrow()[*element].parent.is_some() {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev, child);
        }
    }

    fn append_doctype_to_document(
        &self,
        name: StrTendril,
        _public: StrTendril,
        _system: StrTendril,
    ) {
        let id = self.push(Kind::Doctype(name.to_string()));
        self.insert(0, None, NodeOrText::AppendNode(id));
    }

    fn get_template_contents(&self, target: &Id) -> Id {
        match &self.nodes.borrow()[*target].kind {
            Kind::Element {
                contents: Some(c), ..
            } => *c,
            _ => *target,
        }
    }

    fn same_node(&self, x: &Id, y: &Id) -> bool {
        x == y
    }

    fn set_quirks_mode(&self, _mode: QuirksMode) {}

    fn append_before_sibling(&self, sibling: &Id, child: NodeOrText<Id>) {
        let Some(parent) = self.nodes.borrow()[*sibling].parent else {
            return;
        };
        let at = self.nodes.borrow()[parent]
            .children
            .iter()
            .position(|c| c == sibling);
        self.insert(parent, at, child);
    }

    fn add_attrs_if_missing(&self, target: &Id, attrs: Vec<Attribute>) {
        if let Kind::Element { attrs: have, .. } = &mut self.nodes.borrow_mut()[*target].kind {
            for a in attrs {
                if !have.iter().any(|h| h.name == a.name) {
                    have.push(a);
                }
            }
        }
    }

    fn remove_from_parent(&self, target: &Id) {
        self.detach(*target);
    }

    fn reparent_children(&self, node: &Id, new_parent: &Id) {
        let children = std::mem::take(&mut self.nodes.borrow_mut()[*node].children);
        for c in children {
            self.nodes.borrow_mut()[c].parent = None;
            self.insert(*new_parent, None, NodeOrText::AppendNode(c));
        }
    }
}

impl Serialize for Arena {
    /// The document's children, leaving out each element deeper than
    /// [`MAX_DEPTH`] (and what is in it).
    fn serialize<S: Serializer>(&self, s: &mut S, _scope: TraversalScope) -> std::io::Result<()> {
        enum Step {
            Open(Id, usize),
            Close(Id),
        }
        let nodes = self.nodes.borrow();
        let mut stack: Vec<Step> = nodes[0]
            .children
            .iter()
            .rev()
            .map(|&c| Step::Open(c, 1))
            .collect();
        while let Some(step) = stack.pop() {
            match step {
                Step::Close(id) => {
                    if let Kind::Element { name, .. } = &nodes[id].kind {
                        s.end_elem(name.clone())?;
                    }
                }
                Step::Open(id, depth) => match &nodes[id].kind {
                    Kind::Element {
                        name,
                        attrs,
                        contents,
                    } => {
                        if depth > MAX_DEPTH {
                            continue;
                        }
                        s.start_elem(
                            name.clone(),
                            attrs.iter().map(|a| (&a.name, a.value.as_ref())),
                        )?;
                        stack.push(Step::Close(id));
                        let inside = contents.map_or(&nodes[id].children, |c| &nodes[c].children);
                        stack.extend(inside.iter().rev().map(|&c| Step::Open(c, depth + 1)));
                    }
                    Kind::Text(t) => s.write_text(t)?,
                    Kind::Comment(t) => s.write_comment(t)?,
                    Kind::Doctype(name) => s.write_doctype(name)?,
                    Kind::Pi(target, data) => s.write_processing_instruction(target, data)?,
                    Kind::Document => {}
                },
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Arena, MAX_DEPTH, cap};
    use html5ever::tendril::TendrilSink;

    #[test]
    fn ordinary_documents_are_untouched() {
        let doc = "<table><tr><td><p>one<p>two<ul><li>a<li>b</ul></td><td>x</table><!-- <div> -->\
                   <style>div > p { }</style><img src=x><br>";
        assert!(matches!(cap(doc), std::borrow::Cow::Borrowed(_)));
        let mut many_ps = "<p>x".repeat(5_000);
        many_ps.push_str("<div>end</div>");
        assert!(matches!(cap(&many_ps), std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn real_mails_unclosed_cells_and_paragraphs_do_not_count_as_depth() {
        let rows = "<tr><td><p>one<td><p>two<li>x<li>y".repeat(5_000);
        let sloppy = "<div><font>x</div><p><b>y<i>z</b></i>".repeat(3_000);
        let doc = format!("<table>{rows}</table><font><b>bold</font></b>{sloppy}");
        assert_eq!(super::open_bound(&doc), None);
    }

    #[test]
    fn what_is_nested_too_deep_is_left_out() {
        let depth = MAX_DEPTH + 100;
        let doc = format!(
            "<style>p {{ color: red }}</style><p>before</p>{}deep{}<p>after</p>",
            "<div>".repeat(depth),
            "</div>".repeat(depth)
        );
        let out = cap(&doc);
        assert!(!out.contains("deep"), "the deep text is gone");
        assert!(out.contains("before") && out.contains("after"));
        assert!(
            out.contains("<style>p { color: red }</style>"),
            "raw text kept"
        );
        // `<html>` and `<body>` are two of the levels.
        assert_eq!(out.matches("<div>").count(), MAX_DEPTH - 2);
    }

    #[cfg_attr(debug_assertions, ignore = "large inputs, timed: run with --release")]
    #[test]
    fn the_depth_is_the_parsers_not_the_tags() {
        // The second review of #2213: each of these nests deeper than a scan
        // of the tags said, and overflowed css-inline's stack.
        for (unit, n) in [
            ("<div/>", 16_000),
            ("<span><table><tbody><tr><td></span>", 2_000),
            ("<div><table><tr><td></div>", 2_000),
            ("<b><i></b>", 16_000),
        ] {
            let doc = unit.repeat(n);
            let out = cap(&doc);
            let arena =
                html5ever::parse_document(Arena::default(), Default::default()).one(out.as_ref());
            assert!(arena.depth() <= MAX_DEPTH + 1, "{unit}: {}", arena.depth());
        }
    }
}
