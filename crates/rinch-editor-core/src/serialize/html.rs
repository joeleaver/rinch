//! Schema-driven HTML serialization (copy-out) and parsing (paste-in).
//!
//! Both directions are driven by the schema's `parse_html_tags` (on `NodeSpec` and
//! `MarkSpec`) — one table, used both ways — unifying the old engine's three
//! divergent hardcoded tag maps (`wrap_mark`, `block_type_to_tag`,
//! `mark_type_to_tag`). A few genuinely attr-dependent cases (heading level,
//! ordered-list `start`, `<span style>` → `text_color`/`highlight`, a task list's
//! `<ul data-type="taskList">` and its items' `data-checked`) are handled
//! explicitly, mirroring ProseMirror's per-type `toDOM`/`parseDOM`.
//!
//! HTML is a **lossy clipboard interchange** — the durable, total format is
//! [`super::doc_json`]. The parse direction is a **whitelist** of what a
//! document can hold: an element the schema has no node or mark for gives
//! none, unknown attributes are dropped, `<script>`/`<iframe>`/`<object>`/`<embed>`
//! never materialize, and `javascript:`/`vbscript:` URLs are stripped. This
//! kills the audit's raw-DOM-paste hole.
//!
//! It is **total about text** (#1397): whatever the markup, every character
//! of text a browser would show for it is in what [`slice_from_html`]
//! returns, and what a browser hides (comments, `<style>`, `<head>`) is not.
//! An element the reader has no node or mark for is read through, for its
//! content. Not read: what is not text content (an `<input>`'s value, a
//! `<select>`'s options, an image's `alt` when the image is refused), the
//! fallback content of embedded content (`<object>`, `<video>`, `<canvas>`),
//! and `<svg>` and `<math>`.
//!
//! The tree builder is [`super::html_tree`], zero-dependency.

use super::html_integer::{parse_html_clamped_non_negative_integer, parse_html_integer};
use super::html_tree::{HtmlFragmentParser, ParsedNode, is_table_part, step};
use crate::EditorError;
use crate::model::{AttrValue, Attrs, Fragment, Mark, MarkType, Node, NodeType, Slice};
use crate::pos::Pos;
use crate::schema::Schema;
use crate::tables;
use std::collections::HashMap;

// ─── Copy-out: model → HTML ──────────────────────────────────────────────────

/// Serialize a node (and its subtree) to HTML. A `doc` node emits its block
/// children with no wrapper; any other node emits its own element.
pub fn node_to_html(node: &Node) -> String {
    let mut out = String::new();
    if node.type_name() == "doc" {
        for child in node.content().children() {
            write_node(child, &mut out);
        }
    } else {
        write_node(node, &mut out);
    }
    out
}

/// Serialize a [`Slice`]'s content to an HTML fragment — the clipboard copy-out.
///
/// The slice's open depths are **not** encoded in the markup; they are re-derived
/// on paste by [`slice_from_html`]. So a within-block copy
/// (whose `content` is the bare inline runs) round-trips back into a textblock, and
/// a multi-block copy emits its `<p>`/`<h1>`/… children directly. Content that
/// is nothing without the node it was in — list items, table rows — should be
/// cut with [`clipboard_slice`], which keeps that node.
pub fn slice_to_html(slice: &Slice) -> String {
    let mut out = String::new();
    for child in slice.content.children() {
        write_node(child, &mut out);
    }
    out
}

/// The content of `doc` between `from` and `to` as it goes on the clipboard:
/// [`Node::slice`], and when that is content the document's top node does not
/// take — the items of a list, the rows or cells of a table — the nodes it
/// was in, as far out as it takes for the content to stand alone. A selection
/// inside a list is then that kind of list (`<ol start="3">`, a task list
/// with its checkboxes) wherever it is pasted.
///
/// Inline content (a selection inside one textblock) and blocks any container
/// takes (two paragraphs of a quote) are returned as they are. ProseMirror
/// keeps every ancestor and records them beside the markup; this keeps only
/// the ones the markup cannot do without, so that what another application
/// reads is not wrapped in structure the selection did not show.
pub fn clipboard_slice(doc: &Node, from: usize, to: usize) -> Result<Slice, EditorError> {
    let mut slice = doc.slice(from, to)?;
    if from == to {
        return Ok(slice);
    }
    let r_from = doc.resolve(Pos(from))?;
    let mut depth = r_from.shared_depth(Pos(to));
    while depth > 0
        && let Some(first) = slice.content.children().first()
        && !first.is_inline()
        && !doc.node_type().content_match().accepts(first.type_name())
    {
        let parent = r_from.node(depth).copy_with_content(slice.content.clone());
        slice = Slice::new(
            Fragment::from_node(parent),
            slice.open_start + 1,
            slice.open_end + 1,
        );
        depth -= 1;
    }
    Ok(slice)
}

fn write_node(node: &Node, out: &mut String) {
    // Text node: escaped text wrapped in its marks (innermost = marks[0]).
    if let Some(text) = node.text() {
        let mut s = escape_text(text);
        for mark in node.marks() {
            s = wrap_mark(mark, &s);
        }
        out.push_str(&s);
        return;
    }
    // Leaf non-text node (atom: hr / image / hard_break) → void element.
    if node.is_leaf() {
        let mut s = void_element(node);
        for mark in node.marks() {
            s = wrap_mark(mark, &s);
        }
        out.push_str(&s);
        return;
    }
    // Container element.
    let (open, close) = block_tags(node);
    out.push_str(&open);
    for child in node.content().children() {
        write_node(child, out);
    }
    out.push_str(&close);
}

/// The open/close tag pair for a container node — schema-derived, with the
/// attr-dependent specials handled explicitly.
fn block_tags(node: &Node) -> (String, String) {
    match node.type_name() {
        "heading" => {
            let level = node.attrs().get_int("level").unwrap_or(1).clamp(1, 6);
            let align = align_style_attr(node);
            (format!("<h{level}{align}>"), format!("</h{level}>"))
        }
        "paragraph" => {
            let align = align_style_attr(node);
            (format!("<p{align}>"), "</p>".to_string())
        }
        "ordered_list" => {
            let start = node.attrs().get_int("start").unwrap_or(1);
            if start != 1 {
                (format!("<ol start=\"{start}\">"), "</ol>".to_string())
            } else {
                ("<ol>".to_string(), "</ol>".to_string())
            }
        }
        "code_block" => ("<pre>".to_string(), "</pre>".to_string()),
        // TipTap's markup for a task list, which the import reads back.
        "task_list" => (
            format!("<ul {TASK_TYPE}=\"{TASK_LIST}\">"),
            "</ul>".to_string(),
        ),
        "task_item" => {
            let checked = node.attrs().get_bool("checked").unwrap_or(false);
            (
                format!("<li {TASK_TYPE}=\"{TASK_ITEM}\" {TASK_CHECKED}=\"{checked}\">"),
                "</li>".to_string(),
            )
        }
        "table_cell" | "table_header_cell" => {
            let tag = primary_tag(node.node_type());
            let mut open = format!("<{tag}");
            // Clamped to what the import reads (Chrome's limits): a larger
            // span would read back clamped anyway.
            for (name, attr, max) in [("colspan", "colspan", 1000), ("rowspan", "rowspan", 65534)] {
                let n = node.attrs().get_int(attr).unwrap_or(1).min(max);
                if n > 1 {
                    open.push_str(&format!(" {name}=\"{n}\""));
                }
            }
            open.push('>');
            (open, format!("</{tag}>"))
        }
        _ => {
            let tag = primary_tag(node.node_type());
            (format!("<{tag}>"), format!("</{tag}>"))
        }
    }
}

/// The attribute naming a task list (`<ul>`) or a task item (`<li>`), and
/// its values: TipTap's markup, which copy-out writes and paste-in reads.
pub(super) const TASK_TYPE: &str = "data-type";
const TASK_LIST: &str = "taskList";
const TASK_ITEM: &str = "taskItem";
/// A task item's `checked` attribute: `"true"` or `"false"`.
pub(super) const TASK_CHECKED: &str = "data-checked";

/// The ` style="text-align:…"` fragment for a textblock's `text_align` attribute,
/// or an empty string for the default (`left`) or an unrecognized value. Whitelisted
/// to the three non-default alignments so a hostile `text_align` from an untrusted
/// `DocNode` can never inject arbitrary CSS into the exported markup.
fn align_style_attr(node: &Node) -> String {
    match node.attrs().get_str("text_align") {
        Some(a @ ("center" | "right" | "justify")) => format!(" style=\"text-align:{a}\""),
        _ => String::new(),
    }
}

/// A void/self-closing element (hr, img, br) with its declared string attrs.
fn void_element(node: &Node) -> String {
    let tag = primary_tag(node.node_type());
    let mut s = format!("<{tag}");
    for (k, v) in node.attrs().iter() {
        if let Some(val) = v.as_str()
            && !val.is_empty()
        {
            s.push_str(&format!(" {k}=\"{}\"", escape_attr(val)));
        }
    }
    s.push('>');
    s
}

/// The DOM element tag name a **non-text** node renders to — the schema's
/// `parse_html_tags`, with the attr-dependent specials (`heading` → `h{level}`,
/// `code_block` → `pre`). The desktop view (M5) uses this for its incremental
/// node→element projection, so the host tag is the *same* schema-derived table
/// copy-out HTML uses. Other attrs (`ordered_list.start`, `image.src/alt`) are set
/// by the view as element attributes, not encoded in the tag.
pub fn node_dom_tag(node: &Node) -> String {
    match node.type_name() {
        "heading" => {
            let level = node.attrs().get_int("level").unwrap_or(1).clamp(1, 6);
            format!("h{level}")
        }
        "code_block" => "pre".to_string(),
        _ => primary_tag(node.node_type()),
    }
}

/// The DOM element tag name a mark wraps its content in — the schema's
/// `parse_html_tags`, with the attr-bearing specials (`link` → `a`, `text_color`
/// → `span`, `highlight` → `mark`). `None` for a mark type with no HTML tag (it
/// renders without a wrapper — lossy by design, matching copy-out HTML). The
/// desktop view (M5) uses this for its incremental inline mark wrappers; the
/// attr-bearing marks set their own attributes (`href`, inline `style`).
pub fn mark_dom_tag(mark: &Mark) -> Option<String> {
    match mark.type_name() {
        "link" => Some("a".to_string()),
        "text_color" => Some("span".to_string()),
        "highlight" => Some("mark".to_string()),
        _ => mark.typ.spec().parse_html_tags.first().cloned(),
    }
}

/// The HTML tag a node type serializes to: its first `parse_html_tags` entry, or
/// a group-based fallback (`div` for blocks, `span` otherwise) for types with no
/// declared tags (e.g. `task_list`, whose copy-out [`block_tags`] writes as a
/// `<ul>` itself). HTML is lossy here by design; DocNode is not.
fn primary_tag(nt: &NodeType) -> String {
    if let Some(tag) = nt.spec().parse_html_tags.first() {
        return tag.clone();
    }
    if nt.is_block() {
        "div".to_string()
    } else {
        "span".to_string()
    }
}

/// Wrap `inner` in the HTML for `mark`. Schema-derived for simple marks; explicit
/// for the attr-bearing ones (`link`, `text_color`, `highlight`).
fn wrap_mark(mark: &Mark, inner: &str) -> String {
    match mark.type_name() {
        "link" => {
            let href = mark.attrs.get_str("href").unwrap_or("");
            let mut s = format!("<a href=\"{}\"", escape_attr(href));
            if let Some(t) = mark.attrs.get_str("title")
                && !t.is_empty()
            {
                s.push_str(&format!(" title=\"{}\"", escape_attr(t)));
            }
            if let Some(tg) = mark.attrs.get_str("target")
                && !tg.is_empty()
            {
                s.push_str(&format!(" target=\"{}\"", escape_attr(tg)));
                // Guard against reverse-tabnabbing on HTML export.
                if tg != "_self" {
                    s.push_str(" rel=\"noopener noreferrer\"");
                }
            }
            s.push_str(&format!(">{inner}</a>"));
            s
        }
        // A colour is written only when `is_safe_css_color` accepts it, the
        // check the HTML import applies: the attribute can arrive unchecked (a
        // `DocNode`, a collaboration peer), and escaping keeps it inside the
        // attribute but not out of the CSS (`red;background:url(…)`).
        "text_color" => {
            let color = mark.attrs.get_str("color").unwrap_or("").trim();
            if !is_safe_css_color(color) {
                return inner.to_string();
            }
            format!(
                "<span style=\"color:{}\">{}</span>",
                escape_attr(color),
                inner
            )
        }
        "highlight" => {
            let color = mark.attrs.get_str("color").unwrap_or("").trim();
            if !is_safe_css_color(color) {
                format!("<mark>{inner}</mark>")
            } else {
                format!(
                    "<mark style=\"background-color:{}\">{}</mark>",
                    escape_attr(color),
                    inner
                )
            }
        }
        _ => match mark.typ.spec().parse_html_tags.first() {
            Some(tag) => format!("<{tag}>{inner}</{tag}>"),
            None => inner.to_string(),
        },
    }
}

/// Escape text content (`&`, `<`, `>`).
fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Escape a double-quoted attribute value.
fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ─── Paste-in: HTML → Slice (whitelist) ──────────────────────────────────────

/// Parse an HTML fragment into a sanitized [`Slice`] against `schema`.
///
/// The result is structurally valid content, **open at each end down to the
/// textblock there** ([`open_depth`]): `<p>a</p><p>b</p>` is open through its
/// paragraphs, `<ul><li>a</li></ul>` through the list, its item and the
/// item's paragraph. Markup carries no open depths, so this is the reading
/// that suits a paste ([`Transaction::replace_selection`]): the first
/// textblock's content continues the line it is pasted on and the last one
/// takes the text after the caret, while everything between keeps its
/// structure. An edge that reaches no textblock (a rule, a table) is closed.
///
/// It does not fail on markup, and the content is always valid: what the
/// reader has no node for is read through for its text (an element that holds
/// a block is a container of blocks, any other is inline), table parts with
/// no `<table>` around them are a table, and what has no place in a table
/// goes in front of it. Unknown marks and attributes are dropped; dangerous
/// elements never materialize. The only errors are a schema with no
/// paragraph or list item type, and attributes the schema refuses.
///
/// [`Transaction::replace_selection`]: crate::state::Transaction::replace_selection
pub fn slice_from_html(schema: &Schema, html: &str) -> Result<Slice, EditorError> {
    let mut forest = HtmlFragmentParser::new(html).parse();
    let parser = HtmlParser::new(schema)?;
    parser.mark_blocks(&mut forest);
    let blocks = parser.parse_blocks(&forest)?;
    if blocks.is_empty() {
        return Ok(Slice::empty());
    }
    let open_start = open_depth(blocks.first());
    let open_end = open_depth_end(blocks.last());
    Ok(Slice::new(
        Fragment::from_children(blocks),
        open_start,
        open_end,
    ))
}

/// How many nodes are open at the start of content whose first node is
/// `node`: every node down to and including the first textblock, when the
/// way there crosses no leaf and no isolating node (a table cell); else none.
fn open_depth(node: Option<&Node>) -> usize {
    edge_depth(node, |n| n.content().children().first())
}

/// [`open_depth`] for the end of content whose last node is `node`.
fn open_depth_end(node: Option<&Node>) -> usize {
    edge_depth(node, |n| n.content().children().last())
}

fn edge_depth(mut node: Option<&Node>, next: impl Fn(&Node) -> Option<&Node>) -> usize {
    let mut depth = 0;
    while let Some(n) = node {
        if n.is_textblock() {
            return depth + 1;
        }
        if n.is_leaf() || n.node_type().is_isolating() {
            return 0;
        }
        depth += 1;
        node = next(n);
    }
    0
}

/// The schema's table node types, when it has all four.
struct TableTypes<'a> {
    table: &'a NodeType,
    row: &'a NodeType,
    cell: &'a NodeType,
    header: &'a NodeType,
}

/// Holds the schema and the reverse tag→type lookup tables, built once per parse.
struct HtmlParser<'a> {
    schema: &'a Schema,
    /// Block-level HTML tag → node type (`p`→paragraph, `h1`→heading, …).
    block: HashMap<&'a str, &'a NodeType>,
    /// Inline-leaf HTML tag → node type (`img`→image, `br`→hard_break).
    inline_leaf: HashMap<&'a str, &'a NodeType>,
    /// Inline HTML tag → mark type (`strong`/`b`→bold, `a`→link, `mark`→highlight, …).
    marks: HashMap<&'a str, &'a MarkType>,
    paragraph: &'a NodeType,
    list_item: &'a NodeType,
    tables: Option<TableTypes<'a>>,
    code_block: Option<&'a NodeType>,
    hard_break: Option<&'a NodeType>,
}

/// The textblock that inline content between blocks becomes: a paragraph
/// (`None`), or the heading (say) whose element holds the blocks.
type LooseAs<'w> = Option<(&'w NodeType, &'w Attrs)>;

/// Inline content being gathered.
#[derive(Default)]
struct Inline {
    nodes: Vec<Node>,
    /// Whitespace met between two inline elements at block level; it is
    /// content only if more inline content follows it.
    space: Option<String>,
}

/// The blocks [`HtmlParser::parse_block_refs`] is making of some nodes.
#[derive(Default)]
struct Building {
    blocks: Vec<Node>,
    /// Inline content since the last block.
    loose: Inline,
    /// See [`Parsed::blank`].
    blank: Vec<Node>,
}

/// What [`HtmlParser::parse_block_refs`] made of some nodes.
struct Parsed {
    blocks: Vec<Node>,
    /// When there are no blocks: the whitespace the nodes hold directly,
    /// which is the content of the line they are (`<div>&nbsp;</div>`).
    blank: Vec<Node>,
}

impl<'a> HtmlParser<'a> {
    fn new(schema: &'a Schema) -> Result<Self, EditorError> {
        let mut block = HashMap::new();
        let mut inline_leaf = HashMap::new();
        for (name, spec) in &schema.nodes {
            let nt = schema.node_type(name).expect("compiled node type");
            for tag in &spec.parse_html_tags {
                if nt.is_inline() && nt.is_leaf() {
                    inline_leaf.insert(tag.as_str(), nt);
                } else if nt.is_block() {
                    block.insert(tag.as_str(), nt);
                }
            }
        }
        let mut marks = HashMap::new();
        for (name, spec) in &schema.marks {
            let mt = schema.mark_type(name).expect("compiled mark type");
            for tag in &spec.parse_html_tags {
                marks.insert(tag.as_str(), mt);
            }
        }
        let paragraph = schema
            .node_type("paragraph")
            .ok_or_else(|| EditorError::HtmlParse("schema has no 'paragraph' type".into()))?;
        let list_item = schema
            .node_type("list_item")
            .ok_or_else(|| EditorError::HtmlParse("schema has no 'list_item' type".into()))?;
        let tables = match (
            schema.node_type("table"),
            schema.node_type("table_row"),
            schema.node_type("table_cell"),
            schema.node_type("table_header_cell"),
        ) {
            (Some(table), Some(row), Some(cell), Some(header)) => Some(TableTypes {
                table,
                row,
                cell,
                header,
            }),
            _ => None,
        };
        let hard_break = schema
            .node_type("hard_break")
            .filter(|nt| nt.is_inline() && nt.is_leaf());
        Ok(Self {
            schema,
            block,
            inline_leaf,
            marks,
            paragraph,
            list_item,
            tables,
            code_block: schema.node_type("code_block"),
            hard_break,
        })
    }

    /// Whether `tag` is a block: one of the schema's, or an HTML element that
    /// is a line (or more) of its own.
    fn is_block_tag(&self, tag: &str) -> bool {
        self.block.contains_key(tag) || is_block_level(tag)
    }

    /// Set every element's `holds_block` under `nodes`. Returns whether any
    /// of `nodes` is, or holds, a block.
    fn mark_blocks(&self, nodes: &mut [ParsedNode]) -> bool {
        let mut any = false;
        for n in nodes {
            if let ParsedNode::Element {
                tag,
                children,
                holds_block,
                ..
            } = n
            {
                if is_dropped(tag) {
                    continue;
                }
                *holds_block = self.mark_blocks(children);
                any |= *holds_block || self.is_block_tag(tag);
            }
        }
        any
    }

    fn parse_blocks(&self, nodes: &[ParsedNode]) -> Result<Vec<Node>, EditorError> {
        let refs: Vec<&ParsedNode> = nodes.iter().collect();
        Ok(self.parse_block_refs(&refs, None)?.blocks)
    }

    /// Read sibling nodes as blocks. Inline content between blocks is a
    /// paragraph; a run of `<li>` with no list around it is a list, and a run
    /// of table parts with no `<table>` around it a table.
    ///
    /// This and the functions it calls back through are kept small on
    /// purpose: they recurse once per level of nesting, and an unoptimized
    /// build gives every temporary of a function its own stack slot.
    fn parse_block_refs(
        &self,
        nodes: &[&ParsedNode],
        loose_as: LooseAs<'_>,
    ) -> Result<Parsed, EditorError> {
        let mut b = Building::default();
        let mut i = 0;
        while i < nodes.len() {
            step();
            i = self.read_block(nodes, i, loose_as, &mut b)?;
        }
        self.flush_loose(&mut b.loose, &mut b.blocks, loose_as)?;
        if !b.blocks.is_empty() {
            b.blank.clear();
        }
        Ok(Parsed {
            blocks: b.blocks,
            blank: b.blank,
        })
    }

    /// Read `nodes[i]`, and the run of items or table parts it starts.
    /// Returns the index of the node after what was read.
    fn read_block(
        &self,
        nodes: &[&ParsedNode],
        i: usize,
        loose_as: LooseAs<'_>,
        b: &mut Building,
    ) -> Result<usize, EditorError> {
        let n = nodes[i];
        match n {
            ParsedNode::Text(t) => self.read_loose_text(n, t, b)?,
            ParsedNode::Element {
                tag, attributes, ..
            } => {
                if is_dropped(tag) {
                    return Ok(i + 1);
                }
                if tag == "li" {
                    return self.read_bare_items(nodes, i, is_task_item(attributes), loose_as, b);
                }
                if self.tables.is_some() && is_table_part(tag) {
                    return self.read_bare_table_parts(nodes, i, loose_as, b);
                }
                self.read_element(n, loose_as, b)?;
            }
        }
        Ok(i + 1)
    }

    /// Text between blocks.
    fn read_loose_text(
        &self,
        n: &ParsedNode,
        t: &str,
        b: &mut Building,
    ) -> Result<(), EditorError> {
        if !t.trim().is_empty() {
            self.parse_inline(std::slice::from_ref(n), &[], &mut b.loose)?;
        } else if !b.loose.nodes.is_empty() {
            // Between two inline elements a browser shows this as one
            // space; before a block, as nothing.
            b.loose.space = Some(if t.contains('\u{a0}') {
                t.to_string()
            } else {
                " ".to_string()
            });
        } else if b.blocks.is_empty() && !t.is_empty() {
            b.blank.push(self.schema.text(t)?);
        }
        Ok(())
    }

    /// A list's items alone are what some sources put on the clipboard for a
    /// selection inside one list: a run of them is a bullet list, or a task
    /// list for task items.
    fn read_bare_items(
        &self,
        nodes: &[&ParsedNode],
        start: usize,
        tasks: bool,
        loose_as: LooseAs<'_>,
        b: &mut Building,
    ) -> Result<usize, EditorError> {
        let mut end = start + 1;
        while nodes.get(end).is_some_and(|next| match next {
            ParsedNode::Text(t) => t.trim().is_empty(),
            ParsedNode::Element {
                tag, attributes, ..
            } => tag == "li" && is_task_item(attributes) == tasks,
        }) {
            end += 1;
        }
        self.flush_loose(&mut b.loose, &mut b.blocks, loose_as)?;
        match self.block.get("ul") {
            Some(list) => {
                let list = self.build_list(list, &[], tasks, &nodes[start..end])?;
                b.blocks.push(list);
            }
            // A schema with no bullet list: the items' content.
            None => {
                for item in &nodes[start..end] {
                    if let ParsedNode::Element { children, .. } = item {
                        b.blocks.extend(self.parse_blocks(children)?);
                    }
                }
            }
        }
        Ok(end)
    }

    /// A run of table parts with no `<table>` around them.
    fn read_bare_table_parts(
        &self,
        nodes: &[&ParsedNode],
        start: usize,
        loose_as: LooseAs<'_>,
        b: &mut Building,
    ) -> Result<usize, EditorError> {
        let mut end = start + 1;
        while nodes.get(end).is_some_and(|next| match next {
            ParsedNode::Text(t) => t.trim().is_empty(),
            ParsedNode::Element { tag, .. } => is_table_part(tag) || is_dropped(tag),
        }) {
            end += 1;
        }
        self.flush_loose(&mut b.loose, &mut b.blocks, loose_as)?;
        if let Some(tables) = &self.tables {
            b.blocks
                .extend(self.build_table(tables, &nodes[start..end], false)?);
        }
        Ok(end)
    }

    /// Read one element that is neither dropped, an `<li>` nor a table part.
    fn read_element(
        &self,
        n: &ParsedNode,
        loose_as: LooseAs<'_>,
        b: &mut Building,
    ) -> Result<(), EditorError> {
        let ParsedNode::Element {
            tag,
            attributes,
            children,
            holds_block,
        } = n
        else {
            return Ok(());
        };
        match self.block.get(tag.as_str()) {
            Some(nt) if nt.name() == "table" => {
                if let Some(tables) = &self.tables {
                    self.flush_loose(&mut b.loose, &mut b.blocks, loose_as)?;
                    let parts: Vec<&ParsedNode> = children.iter().collect();
                    b.blocks.extend(self.build_table(tables, &parts, true)?);
                    return Ok(());
                }
            }
            Some(nt) if *holds_block && nt.is_textblock() && Some(*nt) != self.code_block => {
                return self
                    .read_textblock_around_blocks(nt, tag, attributes, children, loose_as, b);
            }
            Some(nt) if !is_table_part(tag) => {
                self.flush_loose(&mut b.loose, &mut b.blocks, loose_as)?;
                b.blocks
                    .push(self.build_block(nt, tag, attributes, children)?);
                return Ok(());
            }
            _ => {}
        }
        if let Some(code) = self.code_block
            && is_preformatted(tag, attributes)
        {
            // What VS Code copies: a `<div style="white-space: pre">` of
            // one `<div>` per line. A code block keeps its lines and
            // their indentation, which paragraphs would not.
            self.flush_loose(&mut b.loose, &mut b.blocks, loose_as)?;
            b.blocks.push(self.build_code_block(code, children)?);
            Ok(())
        } else if !holds_block && !is_block_level(tag) {
            // Inline as far as anyone can tell: a known inline element,
            // or one the reader does not know (`<o:p>`, `<st1:place>`,
            // a custom element) that holds no block.
            self.parse_inline(std::slice::from_ref(n), &[], &mut b.loose)
        } else if *holds_block
            && self.marks.contains_key(tag.as_str())
            && let Some(mark) = self.mark_for(tag, attributes)?
        {
            self.read_marked_blocks(&mark, children, loose_as, b)
        } else {
            self.read_container(children, loose_as, b)
        }
    }

    /// A heading (say) around blocks. Its blocks stay blocks, in the order
    /// they are written, and the inline content between them is headings.
    fn read_textblock_around_blocks(
        &self,
        nt: &NodeType,
        tag: &str,
        attributes: &[(String, String)],
        children: &[ParsedNode],
        loose_as: LooseAs<'_>,
        b: &mut Building,
    ) -> Result<(), EditorError> {
        self.flush_loose(&mut b.loose, &mut b.blocks, loose_as)?;
        let attrs = textblock_attrs(nt, tag, attributes);
        let refs: Vec<&ParsedNode> = children.iter().collect();
        let inner = self.parse_block_refs(&refs, Some((nt, &attrs)))?;
        if inner.blocks.is_empty() {
            b.blocks
                .push(self.make_node(nt, attrs, inline_fragment(inner.blank))?);
        } else {
            b.blocks.extend(inner.blocks);
        }
        Ok(())
    }

    /// A mark around blocks (`<a href><h3>…</h3><p>…</p></a>`, a card that
    /// is one link): the blocks stay blocks, and the mark goes on the inline
    /// content inside them.
    fn read_marked_blocks(
        &self,
        mark: &Mark,
        children: &[ParsedNode],
        loose_as: LooseAs<'_>,
        b: &mut Building,
    ) -> Result<(), EditorError> {
        self.flush_loose(&mut b.loose, &mut b.blocks, loose_as)?;
        let refs: Vec<&ParsedNode> = children.iter().collect();
        for block in self.parse_block_refs(&refs, loose_as)?.blocks {
            b.blocks.push(with_mark_inside(&block, mark));
        }
        Ok(())
    }

    /// A container (div, section, …), or any other element around blocks:
    /// the `<b style="font-weight:normal">` Google Docs wraps everything it
    /// copies in, Word's `<w:sdt>`, Sheets' `<google-sheets-html-origin>`.
    fn read_container(
        &self,
        children: &[ParsedNode],
        loose_as: LooseAs<'_>,
        b: &mut Building,
    ) -> Result<(), EditorError> {
        let refs: Vec<&ParsedNode> = children.iter().collect();
        let inner = self.parse_block_refs(&refs, loose_as)?;
        if inner.blocks.is_empty() {
            for node in inner.blank {
                self.push_inline(&mut b.loose, node)?;
            }
        } else {
            self.flush_loose(&mut b.loose, &mut b.blocks, loose_as)?;
            b.blocks.extend(inner.blocks);
        }
        Ok(())
    }

    /// Flush accumulated inline content as a block: a paragraph, or what
    /// `loose_as` says.
    fn flush_loose(
        &self,
        loose: &mut Inline,
        blocks: &mut Vec<Node>,
        loose_as: LooseAs<'_>,
    ) -> Result<(), EditorError> {
        let loose = std::mem::take(loose);
        if loose.nodes.is_empty() {
            return Ok(());
        }
        let content = inline_fragment(loose.nodes);
        let (nt, attrs) = match loose_as {
            Some((nt, attrs)) => (nt, attrs.clone()),
            None => (self.paragraph, Attrs::new()),
        };
        blocks.push(self.make_node(nt, attrs, content)?);
        Ok(())
    }

    fn build_block(
        &self,
        nt: &NodeType,
        tag: &str,
        attributes: &[(String, String)],
        children: &[ParsedNode],
    ) -> Result<Node, EditorError> {
        match nt.name() {
            "code_block" => self.build_code_block(nt, children),
            "bullet_list" | "ordered_list" => {
                let items: Vec<&ParsedNode> = children.iter().collect();
                self.build_list(nt, attributes, is_task_list(attributes), &items)
            }
            "horizontal_rule" => self.make_node(nt, Attrs::new(), Fragment::empty()),
            _ if nt.is_textblock() => {
                let content = self.parse_inline_children(children)?;
                self.make_node(nt, textblock_attrs(nt, tag, attributes), content)
            }
            // A blockquote, a list item, any other block of blocks.
            _ => {
                let inner = self.ensure_block_plus(self.parse_blocks(children)?)?;
                self.make_node(nt, Attrs::new(), Fragment::from_children(inner))
            }
        }
    }

    /// A code block of the text under `children`, one line per block-level
    /// element and per `<br>`.
    fn build_code_block(
        &self,
        nt: &NodeType,
        children: &[ParsedNode],
    ) -> Result<Node, EditorError> {
        let mut text = String::new();
        let mut line_break = false;
        self.collect_text(children, &mut text, &mut line_break);
        let content = if text.is_empty() {
            Fragment::empty()
        } else {
            Fragment::from_node(self.schema.text(&text)?)
        };
        self.make_node(nt, Attrs::new(), content)
    }

    /// Gather the text under `nodes`, tags aside, for a code block.
    /// `line_break`: a line ended since the last text.
    fn collect_text(&self, nodes: &[ParsedNode], out: &mut String, line_break: &mut bool) {
        for n in nodes {
            step();
            match n {
                ParsedNode::Text(t) => {
                    if std::mem::take(line_break) && !out.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push_str(t);
                }
                ParsedNode::Element { tag, children, .. } => {
                    if is_dropped(tag) {
                        continue;
                    }
                    if self.hard_break.is_some()
                        && self.inline_leaf.get(tag.as_str()).copied() == self.hard_break
                    {
                        if std::mem::take(line_break) && !out.ends_with('\n') {
                            out.push('\n');
                        }
                        out.push('\n');
                        continue;
                    }
                    let block = self.is_block_tag(tag);
                    if block {
                        *line_break = !out.is_empty();
                    }
                    self.collect_text(children, out, line_break);
                    if block {
                        *line_break = !out.is_empty();
                    }
                }
            }
        }
    }

    /// A list of type `nt` (a task list when `tasks` and the schema has one)
    /// from the children of a `<ul>` / `<ol>`, or from a run of bare `<li>`.
    fn build_list(
        &self,
        nt: &NodeType,
        attributes: &[(String, String)],
        tasks: bool,
        children: &[&ParsedNode],
    ) -> Result<Node, EditorError> {
        if tasks
            && nt.name() == "bullet_list"
            && let (Some(list), Some(item)) = (
                self.schema.node_type("task_list"),
                self.schema.node_type("task_item"),
            )
        {
            let mut items = Vec::new();
            for (inner, checked) in self.list_item_contents(children)? {
                let attrs = Attrs::from_iter([("checked", AttrValue::Bool(checked))]);
                items.push(self.make_node(item, attrs, Fragment::from_children(inner))?);
            }
            if items.is_empty() {
                // Unchecked, said as an item that is read says it, so that
                // the list reads back equal to itself.
                let para = self.make_node(self.paragraph, Attrs::new(), Fragment::empty())?;
                let attrs = Attrs::from_iter([("checked", AttrValue::Bool(false))]);
                items.push(self.make_node(item, attrs, Fragment::from_node(para))?);
            }
            return self.make_node(list, Attrs::new(), Fragment::from_children(items));
        }
        let mut items = Vec::new();
        for (inner, _) in self.list_item_contents(children)? {
            items.push(self.make_node(
                self.list_item,
                Attrs::new(),
                Fragment::from_children(inner),
            )?);
        }
        if items.is_empty() {
            items.push(self.empty_list_item()?);
        }
        let attrs = if nt.name() == "ordered_list" {
            // HTML's integer rules, as `ol.start` reads it (#1164).
            let start = attr(attributes, "start")
                .and_then(parse_html_integer)
                .map_or(1, i64::from);
            Attrs::from_iter([("start", AttrValue::Int(start))])
        } else {
            Attrs::new()
        };
        self.make_node(nt, attrs, Fragment::from_children(items))
    }

    /// The content of each item of a list whose element children are
    /// `children`, with whether the item says `data-checked="true"`.
    ///
    /// An `<li>` is an item. A list directly inside the list — which is how
    /// Google Docs writes a nested list, `<ul><li>a</li><ul><li>b</li></ul></ul>`,
    /// and what browsers build from it — goes under the item before it
    /// (ProseMirror's list normalisation), or in an item of its own when it
    /// comes first. Any other child with content gets an item of its own:
    /// nothing in a list is dropped for not being an `<li>`.
    fn list_item_contents(
        &self,
        children: &[&ParsedNode],
    ) -> Result<Vec<(Vec<Node>, bool)>, EditorError> {
        let mut items: Vec<(Vec<Node>, bool)> = Vec::new();
        let mut i = 0;
        while i < children.len() {
            step();
            let child = children[i];
            i += 1;
            match child {
                ParsedNode::Element {
                    tag,
                    attributes,
                    children,
                    ..
                } if tag == "li" => {
                    let checked =
                        attr(attributes, TASK_CHECKED).is_some_and(|v| v.trim() == "true");
                    items.push((
                        self.ensure_block_plus(self.parse_blocks(children)?)?,
                        checked,
                    ));
                }
                ParsedNode::Text(t) if t.trim().is_empty() => {}
                stray => {
                    // Table parts side by side are one table, as anywhere.
                    let start = i - 1;
                    if matches!(stray, ParsedNode::Element { tag, .. } if is_table_part(tag)) {
                        while children.get(i).is_some_and(|next| match next {
                            ParsedNode::Text(t) => t.trim().is_empty(),
                            ParsedNode::Element { tag, .. } => is_table_part(tag),
                        }) {
                            i += 1;
                        }
                    }
                    let blocks = self.parse_block_refs(&children[start..i], None)?.blocks;
                    if blocks.is_empty() {
                        continue;
                    }
                    let nested_list = matches!(stray, ParsedNode::Element { tag, .. } if tag == "ul" || tag == "ol");
                    match items.last_mut() {
                        Some((last, _)) if nested_list => last.extend(blocks),
                        _ => items.push((blocks, false)),
                    }
                }
            }
        }
        Ok(items)
    }

    /// Ensure a `block+` content list is non-empty (insert an empty paragraph).
    fn ensure_block_plus(&self, mut blocks: Vec<Node>) -> Result<Vec<Node>, EditorError> {
        if blocks.is_empty() {
            blocks.push(self.make_node(self.paragraph, Attrs::new(), Fragment::empty())?);
        }
        Ok(blocks)
    }

    fn empty_list_item(&self) -> Result<Node, EditorError> {
        let para = self.make_node(self.paragraph, Attrs::new(), Fragment::empty())?;
        self.make_node(self.list_item, Attrs::new(), Fragment::from_node(para))
    }

    // ── Tables ───────────────────────────────────────────────────────────────

    /// A table from `parts`: the children of a `<table>` (`whole`), or a run
    /// of table parts with no `<table>` around them — cells are one row, rows
    /// the rows of one table — which is how a browser reads them where a
    /// table's content is expected, and what another editor puts on the
    /// clipboard for part of a table.
    ///
    /// The table comes last. Before it come the blocks of everything in
    /// `parts` that has no place in a table (text and elements directly in
    /// the table, a row group or a row), which a browser moves in front of
    /// the table too, and then its captions. With no row at all there is no
    /// table, unless it is a `<table>` with nothing in it, which reads as one
    /// empty cell.
    fn build_table(
        &self,
        types: &TableTypes<'a>,
        parts: &[&ParsedNode],
        whole: bool,
    ) -> Result<Vec<Node>, EditorError> {
        let mut outside: Vec<&ParsedNode> = Vec::new();
        let mut captions: Vec<&ParsedNode> = Vec::new();
        let mut rows = self.parse_table_rows(types, parts, &mut outside, &mut captions)?;
        let mut blocks = self.parse_block_refs(&outside, None)?.blocks;
        for caption in captions {
            if let ParsedNode::Element { children, .. } = caption {
                blocks.extend(self.parse_blocks(children)?);
            }
        }
        if rows.is_empty() {
            if !whole || !blocks.is_empty() {
                return Ok(blocks);
            }
            // `table > table_row+` must be non-empty.
            rows.push(self.empty_table_row(types)?);
        }
        blocks.push(self.make_node(types.table, Attrs::new(), Fragment::from_children(rows))?);
        Ok(blocks)
    }

    /// Parse table parts into `table_row` nodes, transparently descending
    /// through `<thead>`/`<tbody>`/`<tfoot>` wrappers. A run of cells with no
    /// `<tr>` is a row. Rows with no cells are dropped. What is neither a
    /// part nor in a cell goes to `outside`, and `<caption>`s to `captions`.
    ///
    /// Each wrapper, and each run of bare `<tr>`s (which the HTML parser wraps
    /// in an implicit `<tbody>`), is a **row group**: a `rowspan="0"` spans to
    /// the end of its group, so a group's rows are collected before any is
    /// built (#1164).
    fn parse_table_rows<'n>(
        &self,
        types: &TableTypes<'a>,
        parts: &[&'n ParsedNode],
        outside: &mut Vec<&'n ParsedNode>,
        captions: &mut Vec<&'n ParsedNode>,
    ) -> Result<Vec<Node>, EditorError> {
        let mut rows = Vec::new();
        let mut group: Vec<Vec<PendingCell<'a>>> = Vec::new();
        // Cells directly in the table: a row with no `<tr>`.
        let mut loose_cells: Vec<&'n ParsedNode> = Vec::new();
        for (index, part) in parts.iter().copied().enumerate() {
            step();
            let ParsedNode::Element { tag, children, .. } = part else {
                if matches!(part, ParsedNode::Text(t) if !t.trim().is_empty()) {
                    outside.push(part);
                }
                continue;
            };
            if is_dropped(tag) {
                continue;
            }
            match tag.as_str() {
                "td" | "th" => loose_cells.push(part),
                "caption" => captions.push(part),
                "col" => {}
                "colgroup" => outside.extend(children.iter().filter(|c| {
                    !matches!(c, ParsedNode::Element { tag, .. } if tag == "col")
                        && !matches!(c, ParsedNode::Text(t) if t.trim().is_empty())
                })),
                "tr" => {
                    let cell_parts: Vec<&ParsedNode> = children.iter().collect();
                    let cells = self.parse_table_cells(types, &cell_parts, outside)?;
                    if !cells.is_empty() {
                        group.push(cells);
                    }
                }
                "thead" | "tbody" | "tfoot" => {
                    rows.extend(self.build_row_group(types, std::mem::take(&mut group))?);
                    let group_parts: Vec<&ParsedNode> = children.iter().collect();
                    rows.extend(self.parse_table_rows(types, &group_parts, outside, captions)?);
                }
                _ => outside.push(part),
            }
            // A run of bare cells ends at whatever is not a cell.
            let run_goes_on = matches!(tag.as_str(), "td" | "th")
                && parts[index + 1..]
                    .iter()
                    .find(|next| !matches!(next, ParsedNode::Text(t) if t.trim().is_empty()))
                    .is_some_and(
                        |next| matches!(next, ParsedNode::Element { tag, .. } if tag == "td" || tag == "th"),
                    );
            if !loose_cells.is_empty() && !run_goes_on {
                let cells = self.parse_table_cells(types, &loose_cells, outside)?;
                loose_cells.clear();
                if !cells.is_empty() {
                    group.push(cells);
                }
            }
        }
        rows.extend(self.build_row_group(types, group)?);
        Ok(rows)
    }

    /// Build one row group's `table_row`s, cutting each cell to the grid
    /// Chrome 153 lays out (#1164, #1176):
    ///
    /// - A `rowspan` reaches at most the group's last row, and `rowspan="0"`
    ///   is exactly that (the model has no "to the end" span; every span is
    ///   at least 1). Chrome cuts a span at the end of its row group, so a
    ///   `rowspan="3"` in a one-row `<tbody>` spans one row there too.
    /// - A row's spans reach at most [`tables::MAX_IMPORTED_ROW_WIDTH`]
    ///   columns, counting the columns carried into it by rowspans from the
    ///   rows above: a `colspan` is cut to what is left, never below 1, and a
    ///   cell that starts in a row already that wide spans that row alone, so
    ///   it carries nothing down. Each such cell adds one column, so a row is
    ///   at most 1000 columns plus one per cell that found it full. That limit
    ///   is rinch's, not Chrome's: a row's width used to grow with every
    ///   rowspan above it, and a 57 KB paste made a 1,000,000-column table.
    fn build_row_group(
        &self,
        types: &TableTypes<'a>,
        group: Vec<Vec<PendingCell<'a>>>,
    ) -> Result<Vec<Node>, EditorError> {
        let len = group.len();
        let mut rows = Vec::with_capacity(len);
        // `ends[r]`: the columns that stop being carried down at row `r`.
        let mut ends = vec![0usize; len + 1];
        let mut carried = 0usize;
        for (r, cells) in group.into_iter().enumerate() {
            carried -= ends[r];
            let rows_left = len - r;
            let mut row_width = carried;
            let mut starts = 0usize;
            let cells = cells
                .into_iter()
                .map(|cell| {
                    // A cell that finds its row full spans that row alone,
                    // so it carries nothing down: otherwise every cell past
                    // the 1000th column would widen every row below it.
                    let full = row_width >= tables::MAX_IMPORTED_ROW_WIDTH;
                    let rowspan = match cell.rowspan as usize {
                        _ if full => 1,
                        0 => rows_left,
                        n => n.min(rows_left),
                    };
                    let room = tables::MAX_IMPORTED_ROW_WIDTH
                        .saturating_sub(row_width)
                        .max(1);
                    let colspan = (cell.colspan as usize).min(room);
                    row_width += colspan;
                    if rowspan > 1 {
                        starts += colspan;
                        ends[r + rowspan] += colspan;
                    }
                    let attrs = Attrs::from_iter([
                        ("colspan", AttrValue::Int(colspan as i64)),
                        ("rowspan", AttrValue::Int(rowspan as i64)),
                    ]);
                    self.make_node(cell.node_type, attrs, cell.content)
                })
                .collect::<Result<Vec<_>, _>>()?;
            carried += starts;
            rows.push(self.make_node(types.row, Attrs::new(), Fragment::from_children(cells))?);
        }
        Ok(rows)
    }

    /// Parse a row's `<td>`/`<th>` children into pending cells (block content,
    /// with `colspan`/`rowspan` carried through). Anything else with content
    /// goes to `outside`.
    fn parse_table_cells<'n>(
        &self,
        types: &TableTypes<'a>,
        children: &[&'n ParsedNode],
        outside: &mut Vec<&'n ParsedNode>,
    ) -> Result<Vec<PendingCell<'a>>, EditorError> {
        let mut cells = Vec::new();
        for child in children.iter().copied() {
            step();
            let ParsedNode::Element {
                tag,
                children: cell_children,
                attributes,
                ..
            } = child
            else {
                if matches!(child, ParsedNode::Text(t) if !t.trim().is_empty()) {
                    outside.push(child);
                }
                continue;
            };
            let nt = match tag.as_str() {
                "td" => types.cell,
                "th" => types.header,
                _ => {
                    if !is_dropped(tag) {
                        outside.push(child);
                    }
                    continue;
                }
            };
            let inner = self.ensure_block_plus(self.parse_blocks(cell_children)?)?;
            // `colspan`/`rowspan` — the only table attributes that survive
            // paste — read as Chrome 153's `td.colSpan` / `td.rowSpan` read
            // them: clamped to 1..=1000 and 0..=65534, an overflow the maximum,
            // an error or a negative value the default 1 (#1164).
            let span = |name: &str, min: u32, max: u32| {
                attr(attributes, name)
                    .and_then(|v| parse_html_clamped_non_negative_integer(v, min, max))
                    .unwrap_or(1)
            };
            cells.push(PendingCell {
                node_type: nt,
                colspan: span("colspan", 1, 1000),
                rowspan: span("rowspan", 0, 65534),
                content: Fragment::from_children(inner),
            });
        }
        Ok(cells)
    }

    /// A `table_row` holding one empty (single-paragraph) `table_cell` — the
    /// fallback for a `<table>` with nothing in it.
    fn empty_table_row(&self, types: &TableTypes<'a>) -> Result<Node, EditorError> {
        let para = self.make_node(self.paragraph, Attrs::new(), Fragment::empty())?;
        let cell = self.make_node(
            types.cell,
            Attrs::from_iter([
                ("colspan", AttrValue::Int(1)),
                ("rowspan", AttrValue::Int(1)),
            ]),
            Fragment::from_node(para),
        )?;
        self.make_node(types.row, Attrs::new(), Fragment::from_node(cell))
    }

    // ── Inline content ───────────────────────────────────────────────────────

    fn parse_inline_children(&self, children: &[ParsedNode]) -> Result<Fragment, EditorError> {
        let mut out = Inline::default();
        self.parse_inline(children, &[], &mut out)?;
        Ok(inline_fragment(out.nodes))
    }

    /// Add `node` to `out`, after the space owed before it.
    fn push_inline(&self, out: &mut Inline, node: Node) -> Result<(), EditorError> {
        if let Some(space) = out.space.take()
            && !out.nodes.is_empty()
        {
            out.nodes.push(self.schema.text(&space)?);
        }
        out.nodes.push(node);
        Ok(())
    }

    /// Read `nodes` as inline content. Every element is read through: one
    /// that is a mark, a leaf or a styled `<span>` as that, any other for
    /// its content alone.
    fn parse_inline(
        &self,
        nodes: &[ParsedNode],
        active: &[Mark],
        out: &mut Inline,
    ) -> Result<(), EditorError> {
        for n in nodes {
            step();
            match n {
                ParsedNode::Text(t) => self.push_text(t, active, out)?,
                ParsedNode::Element {
                    tag,
                    attributes,
                    children,
                    ..
                } => {
                    if is_dropped(tag) {
                        continue;
                    }
                    match self.inline_marks(tag, attributes, active, out)? {
                        Some(next) => self.parse_inline(children, &next, out)?,
                        None => self.parse_inline(children, active, out)?,
                    }
                }
            }
        }
        Ok(())
    }

    fn push_text(&self, t: &str, active: &[Mark], out: &mut Inline) -> Result<(), EditorError> {
        if t.is_empty() {
            return Ok(());
        }
        self.push_inline(out, self.schema.text_with_marks(t, active.to_vec())?)
    }

    /// The marks on the content of an inline `tag` element, when they are
    /// not `active`. A leaf (br, img) is added to `out` here: it has no
    /// content.
    fn inline_marks(
        &self,
        tag: &str,
        attributes: &[(String, String)],
        active: &[Mark],
        out: &mut Inline,
    ) -> Result<Option<Vec<Mark>>, EditorError> {
        // Inline leaves (br, img).
        if let Some(nt) = self.inline_leaf.get(tag) {
            if let Some(leaf) = self.build_inline_leaf(nt, attributes, active)? {
                self.push_inline(out, leaf)?;
            }
            return Ok(None);
        }
        // Mark-bearing tags (strong, em, a, mark, …). One that makes no mark
        // (an <a> with no or an unsafe href) is transparent: its text is kept.
        if self.marks.contains_key(tag) {
            return Ok(self
                .mark_for(tag, attributes)?
                .map(|mark| with_mark(active, mark)));
        }
        // <span style>: style-derived marks (color → text_color, bg → highlight).
        if tag == "span" {
            return self.span_marks(attributes, active);
        }
        // Any other element → transparent.
        Ok(None)
    }

    fn build_inline_leaf(
        &self,
        nt: &NodeType,
        attributes: &[(String, String)],
        active: &[Mark],
    ) -> Result<Option<Node>, EditorError> {
        match nt.name() {
            "hard_break" => Ok(Some(self.make_node(nt, Attrs::new(), Fragment::empty())?)),
            "image" => {
                let Some(src) = attr(attributes, "src") else {
                    return Ok(None);
                };
                if !is_safe_url(src, true) {
                    return Ok(None);
                }
                let mut pairs: Vec<(&str, AttrValue)> = vec![("src", AttrValue::from(src))];
                // An empty `alt` or `title` is none: the writer writes
                // neither, so this reads back the same.
                if let Some(alt) = attr(attributes, "alt").filter(|v| !v.is_empty()) {
                    pairs.push(("alt", AttrValue::from(alt)));
                }
                if let Some(title) = attr(attributes, "title").filter(|v| !v.is_empty()) {
                    pairs.push(("title", AttrValue::from(title)));
                }
                let img = self.make_node(nt, Attrs::from_iter(pairs), Fragment::empty())?;
                Ok(Some(if active.is_empty() {
                    img
                } else {
                    img.with_marks(active.to_vec())
                }))
            }
            _ => Ok(None),
        }
    }

    /// Resolve a mark-bearing tag to a `Mark`, or `None` if it cannot be formed
    /// (e.g. a link with no safe href → transparent passthrough).
    fn mark_for(
        &self,
        tag: &str,
        attributes: &[(String, String)],
    ) -> Result<Option<Mark>, EditorError> {
        let Some(mt) = self.marks.get(tag).copied() else {
            return Ok(None);
        };
        match mt.name() {
            "link" => {
                let Some(href) = attr(attributes, "href") else {
                    return Ok(None);
                };
                if !is_safe_url(href, false) {
                    return Ok(None);
                }
                let mut pairs: Vec<(&str, AttrValue)> = vec![("href", AttrValue::from(href))];
                if let Some(title) = attr(attributes, "title")
                    && !title.is_empty()
                {
                    pairs.push(("title", AttrValue::from(title)));
                }
                if let Some(target) = attr(attributes, "target")
                    && !target.is_empty()
                {
                    pairs.push(("target", AttrValue::from(target)));
                }
                let attrs = mt.compute_attrs(&Attrs::from_iter(pairs))?;
                Ok(Some(Mark::new(mt.clone(), attrs)))
            }
            "highlight" => {
                let mut pairs: Vec<(&str, AttrValue)> = Vec::new();
                if let Some(style) = attr(attributes, "style")
                    && let Some(bg) =
                        parse_style(style, "background-color").filter(|c| is_safe_css_color(c))
                {
                    pairs.push(("color", AttrValue::from(bg)));
                }
                let attrs = mt.compute_attrs(&Attrs::from_iter(pairs))?;
                Ok(Some(Mark::new(mt.clone(), attrs)))
            }
            "bold" if weight_is_normal(attributes) => Ok(None),
            _ => {
                let attrs = mt.compute_attrs(&Attrs::new())?;
                Ok(Some(Mark::new(mt.clone(), attrs)))
            }
        }
    }

    /// Marks contributed by a `<span style>`: `color` → `text_color`,
    /// `background-color` → `highlight`. Returns the new active mark set, or
    /// `None` for a span with no style.
    fn span_marks(
        &self,
        attributes: &[(String, String)],
        active: &[Mark],
    ) -> Result<Option<Vec<Mark>>, EditorError> {
        let Some(style) = attr(attributes, "style") else {
            return Ok(None);
        };
        let mut next = active.to_vec();
        if let (Some(color), Some(mt)) = (
            parse_style(style, "color").filter(|c| is_safe_css_color(c)),
            self.schema.mark_type("text_color"),
        ) {
            let attrs = mt.compute_attrs(&Attrs::from_iter([("color", AttrValue::from(color))]))?;
            next = with_mark(&next, Mark::new(mt.clone(), attrs));
        }
        // `transparent` is no highlight: Google Docs says it on every span.
        if let (Some(bg), Some(mt)) = (
            parse_style(style, "background-color")
                .filter(|c| is_safe_css_color(c) && !c.trim().eq_ignore_ascii_case("transparent")),
            self.schema.mark_type("highlight"),
        ) {
            let attrs = mt.compute_attrs(&Attrs::from_iter([("color", AttrValue::from(bg))]))?;
            next = with_mark(&next, Mark::new(mt.clone(), attrs));
        }
        Ok(Some(next))
    }

    fn make_node(
        &self,
        nt: &NodeType,
        attrs: Attrs,
        content: Fragment,
    ) -> Result<Node, EditorError> {
        let attrs = nt.compute_attrs(&attrs)?;
        Ok(Node::new_branch(nt.clone(), attrs, content))
    }
}

/// `active` with `mark` in place of any mark of its type: of two links, or
/// two colours, one inside the other, the inner one is what the text has.
fn with_mark(active: &[Mark], mark: Mark) -> Vec<Mark> {
    if active.iter().any(|m| m.type_name() == mark.type_name()) {
        let others: Vec<Mark> = active
            .iter()
            .filter(|m| m.type_name() != mark.type_name())
            .cloned()
            .collect();
        mark.add_to_set(&others)
    } else {
        mark.add_to_set(active)
    }
}

/// `nodes` as inline content: neighbouring text with the same marks is one
/// text node, as it is everywhere else in a document.
fn inline_fragment(nodes: Vec<Node>) -> Fragment {
    let mut out: Vec<Node> = Vec::with_capacity(nodes.len());
    // The text gathered onto the last node of `out`, when more than its own.
    let mut joined: Option<String> = None;
    fn settle(out: &mut [Node], joined: &mut Option<String>) {
        if let (Some(text), Some(last)) = (joined.take(), out.last_mut()) {
            *last = last.with_text(text.into());
        }
    }
    for node in nodes {
        match (out.last(), node.text()) {
            (Some(last), Some(text)) if last.is_text() && last.same_markup(&node) => {
                joined
                    .get_or_insert_with(|| last.text().unwrap_or_default().to_string())
                    .push_str(text);
            }
            _ => {
                settle(&mut out, &mut joined);
                out.push(node);
            }
        }
    }
    settle(&mut out, &mut joined);
    Fragment::from_children(out)
}

/// The attrs of a textblock of type `nt` read from a `tag` element: a
/// heading's level, and the alignment of a heading or a paragraph (the two
/// types whose schema declares `text_align`).
fn textblock_attrs(nt: &NodeType, tag: &str, attributes: &[(String, String)]) -> Attrs {
    match nt.name() {
        "heading" => align_attrs(attributes).with("level", AttrValue::Int(heading_level(tag))),
        "paragraph" => align_attrs(attributes),
        _ => Attrs::new(),
    }
}

/// HTML elements that are a line, or more, of their own, whatever the schema
/// makes of them: where one starts or ends, a line ends.
fn is_block_level(tag: &str) -> bool {
    is_table_part(tag)
        || matches!(
            tag,
            "address"
                | "article"
                | "aside"
                | "blockquote"
                | "center"
                | "dd"
                | "details"
                | "dialog"
                | "dir"
                | "div"
                | "dl"
                | "dt"
                | "fieldset"
                | "figcaption"
                | "figure"
                | "footer"
                | "form"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "header"
                | "hgroup"
                | "hr"
                | "legend"
                | "li"
                | "listing"
                | "main"
                | "menu"
                | "nav"
                | "ol"
                | "p"
                | "pre"
                | "search"
                | "section"
                | "summary"
                | "table"
                | "ul"
                | "xmp"
        )
}

/// A `<div>` (or other container) whose own style says `white-space: pre`:
/// preformatted text.
fn is_preformatted(tag: &str, attributes: &[(String, String)]) -> bool {
    tag == "div"
        && attr(attributes, "style")
            .and_then(|style| parse_style(style, "white-space"))
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("pre"))
}

/// An `<li>` whose `data-type` is `taskItem`.
fn is_task_item(attributes: &[(String, String)]) -> bool {
    attr(attributes, TASK_TYPE).is_some_and(|v| v.trim() == TASK_ITEM)
}

/// Elements dropped with everything in them: what a browser shows no text
/// for (scripts, styles, embedded content and its fallback, the options of
/// a `<select>`), and what the reader will not read (`<svg>` and `<math>`,
/// which on a clipboard are icons and the hidden copy of a rendered
/// formula). The tree builder never hands over the first few
/// ([`super::html_tree`]); they are listed for a tree from anywhere else.
fn is_dropped(tag: &str) -> bool {
    matches!(
        tag,
        "script"
            | "style"
            | "meta"
            | "link"
            | "head"
            | "title"
            | "template"
            | "iframe"
            | "noscript"
            | "noembed"
            | "noframes"
            | "base"
            | "object"
            | "embed"
            | "applet"
            | "audio"
            | "video"
            | "canvas"
            | "input"
            | "select"
            | "datalist"
            | "svg"
            | "math"
            | "frame"
            | "frameset"
    )
}

/// A table cell parsed but not yet built: its `rowspan` can be `0`, "to the end
/// of the row group", which only the whole group resolves
/// ([`HtmlParser::build_row_group`]).
struct PendingCell<'a> {
    node_type: &'a NodeType,
    colspan: u32,
    /// `0` spans to the end of the row group.
    rowspan: u32,
    content: Fragment,
}

/// The heading level encoded in an `h1`..`h6` tag (defaults to 1).
fn heading_level(tag: &str) -> i64 {
    tag.strip_prefix('h')
        .and_then(|d| d.parse::<i64>().ok())
        .filter(|l| (1..=6).contains(l))
        .unwrap_or(1)
}

/// The `text_align` attrs implied by an element's inline `style="text-align:…"`,
/// or empty attrs when there is no recognized alignment.
///
/// The inverse of [`align_style_attr`]: it lets a textblock survive an HTML
/// round-trip (copy/paste, `load_html` of previously exported markup) with its
/// alignment intact. Whitelisted to the same three non-default values, so a
/// hostile `style` cannot smuggle an arbitrary value into the attribute — and
/// `left` is skipped because it is the schema default.
fn align_attrs(attributes: &[(String, String)]) -> Attrs {
    let align = attr(attributes, "style")
        .and_then(|s| parse_style(s, "text-align"))
        .map(|a| a.trim().to_ascii_lowercase())
        .filter(|a| matches!(a.as_str(), "center" | "right" | "justify"));
    match align {
        Some(a) => Attrs::from_iter([("text_align", AttrValue::Str(a.into()))]),
        None => Attrs::new(),
    }
}

/// `node` with `mark` added to every inline node inside it that the inline
/// reader would have put it on, had the mark's element been inside the
/// block: text and images, where the parent allows the mark and the node
/// carries no mark of that type already (an inner link keeps its own href).
/// A hard break carries no mark, there as here (#1401), so
/// `<strong><p>a<br>b</p></strong>` and `<p><strong>a<br>b</strong></p>` are
/// one document.
fn with_mark_inside(node: &Node, mark: &Mark) -> Node {
    if node.is_leaf() || node.is_text() {
        return node.clone();
    }
    let allowed = node.node_type().spec().marks.allows(mark.type_name());
    let children: Vec<Node> = node
        .content()
        .children()
        .iter()
        .map(|child| {
            if !child.is_inline() {
                with_mark_inside(child, mark)
            } else if allowed
                && child.type_name() != "hard_break"
                && !child
                    .marks()
                    .iter()
                    .any(|m| m.type_name() == mark.type_name())
            {
                child.with_marks(mark.add_to_set(child.marks()))
            } else {
                child.clone()
            }
        })
        .collect();
    if node.is_textblock() {
        // Text that differed only by this mark is one node now.
        node.copy_with_content(inline_fragment(children))
    } else {
        node.copy_with_content(Fragment::from_children(children))
    }
}

/// Whether a `<b>` / `<strong>` says in its own style that it is not bold:
/// Google Docs wraps everything it copies in `<b style="font-weight:normal">`.
fn weight_is_normal(attributes: &[(String, String)]) -> bool {
    attr(attributes, "style")
        .and_then(|style| parse_style(style, "font-weight"))
        .is_some_and(|w| matches!(w.trim().to_ascii_lowercase().as_str(), "normal" | "400"))
}

/// A `<ul>` whose `data-type` is `taskList`.
fn is_task_list(attributes: &[(String, String)]) -> bool {
    attr(attributes, TASK_TYPE).is_some_and(|v| v.trim() == TASK_LIST)
}

/// Find an attribute value (case-insensitive name) in a parsed attribute list.
fn attr<'b>(attributes: &'b [(String, String)], name: &str) -> Option<&'b str> {
    attributes
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// Extract a single CSS property value from an inline `style` attribute.
///
/// Quote- and bracket-aware (issue #705): a `;` or `:` inside `url(...)` or a
/// quoted string is part of the value, not a declaration boundary — see
/// [`super::style_scan`].
fn parse_style(style: &str, property: &str) -> Option<String> {
    for (name, value) in super::style_scan::style_declarations(style) {
        if name.eq_ignore_ascii_case(property) && !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

/// Reject dangerous URL schemes. `javascript:`/`vbscript:` are always rejected;
/// `data:` is rejected for links and allowed only for **raster** image data URIs
/// (`data:image/svg+xml` is excluded — SVG can carry `<script>`). Whitespace
/// (including embedded newlines/tabs) is stripped before the scheme check to defeat
/// `java\nscript:`-style obfuscation. Shared with the markdown ingress path.
pub(crate) fn is_safe_url(url: &str, allow_data_image: bool) -> bool {
    let normalized: String = url
        .trim()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    if normalized.starts_with("javascript:") || normalized.starts_with("vbscript:") {
        return false;
    }
    if normalized.starts_with("data:") {
        if !allow_data_image {
            return false;
        }
        return [
            "data:image/png",
            "data:image/jpeg",
            "data:image/jpg",
            "data:image/gif",
            "data:image/webp",
            "data:image/avif",
            "data:image/bmp",
        ]
        .iter()
        .any(|prefix| normalized.starts_with(prefix));
    }
    true
}

/// Whitelist for CSS color values accepted from pasted `style` attributes. Allows
/// only named colors (letters), hex (`#rgb`/`#rgba`/`#rrggbb`/`#rrggbbaa`), and the
/// numeric color functions `rgb()/rgba()/hsl()/hsla()`. Everything else — notably
/// `expression(...)`, `url(...)`, and anything carrying `;`/`@` — is rejected, so a
/// pasted color can never smuggle arbitrary CSS into the model or HTML copy-out.
pub(crate) fn is_safe_css_color(value: &str) -> bool {
    let v = value.trim();
    if v.is_empty() || v.len() > 64 {
        return false;
    }
    if let Some(hex) = v.strip_prefix('#') {
        return matches!(hex.len(), 3 | 4 | 6 | 8) && hex.bytes().all(|b| b.is_ascii_hexdigit());
    }
    if let Some(open) = v.find('(') {
        let name = v[..open].trim().to_ascii_lowercase();
        if !matches!(name.as_str(), "rgb" | "rgba" | "hsl" | "hsla") || !v.ends_with(')') {
            return false;
        }
        let args = &v[open + 1..v.len() - 1];
        return args
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b',' | b'%' | b' ' | b'/'));
    }
    v.bytes().all(|b| b.is_ascii_alphabetic())
}

/// An attribute the HTML import would drop or degrade, as strict Markdown
/// reading reports it for an HTML table block.
#[cfg(feature = "markdown")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DroppedAttr {
    /// An `<a>` whose `href` [`is_safe_url`] refuses.
    UnsafeLink,
    /// An `<img>` whose `src` [`is_safe_url`] refuses.
    UnsafeImage,
    /// Any other attribute, or value, the import does not carry into the model.
    Other,
}

/// The first attribute in `html` that [`slice_from_html`] would not carry into
/// the document, for the tags a table block may hold: `href`/`title`/`target`
/// (`rel` is the writer's own) on `<a>`, `src`/`alt`/`title` on `<img>`,
/// `colspan`/`rowspan` in range on a cell, `style` holding only `text-align` on
/// a paragraph or heading, `start` on `<ol>`, and a `style` of safe colours on
/// `<span>` (`color`, `background-color`) and `<mark>` (`background-color`).
#[cfg(feature = "markdown")]
pub(crate) fn dropped_table_attr(html: &str) -> Option<DroppedAttr> {
    fn style_is(style: &str, ok: &dyn Fn(&str, &str) -> bool) -> bool {
        super::style_scan::style_declarations(style)
            .into_iter()
            .all(|(name, value)| ok(&name.to_ascii_lowercase(), value.trim()))
    }
    fn element(
        tag: &str,
        attributes: &[(String, String)],
        in_task_list: bool,
    ) -> Option<DroppedAttr> {
        for (name, value) in attributes {
            let kept = match (tag, name.as_str()) {
                ("ul", TASK_TYPE) => value.trim() == TASK_LIST,
                ("li", TASK_TYPE) => in_task_list && value.trim() == TASK_ITEM,
                ("li", TASK_CHECKED) => in_task_list && matches!(value.trim(), "true" | "false"),
                ("a", "href") if !is_safe_url(value, false) => {
                    return Some(DroppedAttr::UnsafeLink);
                }
                ("img", "src") if !is_safe_url(value, true) => {
                    return Some(DroppedAttr::UnsafeImage);
                }
                ("a", "href" | "title" | "target" | "rel") | ("img", "src" | "alt" | "title") => {
                    true
                }
                ("td" | "th", "colspan") => value
                    .trim()
                    .parse::<u32>()
                    .is_ok_and(|n| (1..=1000).contains(&n)),
                ("td" | "th", "rowspan") => value.trim().parse::<u32>().is_ok_and(|n| n <= 65534),
                ("p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6", "style") => {
                    style_is(value, &|k, v| {
                        k == "text-align"
                            && matches!(
                                v.to_ascii_lowercase().as_str(),
                                "left" | "center" | "right" | "justify"
                            )
                    })
                }
                ("ol", "start") => value.trim().parse::<i64>().is_ok(),
                ("span", "style") => style_is(value, &|k, v| {
                    matches!(k, "color" | "background-color") && is_safe_css_color(v)
                }),
                ("mark", "style") => style_is(value, &|k, v| {
                    k == "background-color" && is_safe_css_color(v)
                }),
                _ => false,
            };
            if !kept {
                return Some(DroppedAttr::Other);
            }
        }
        match tag {
            "a" if attr(attributes, "href").is_none() => Some(DroppedAttr::Other),
            "img" if attr(attributes, "src").is_none() => Some(DroppedAttr::Other),
            _ => None,
        }
    }
    /// `in_task_list`: `nodes` are the children of a task list's `<ul>`.
    fn walk(nodes: &[ParsedNode], in_task_list: bool) -> Option<DroppedAttr> {
        nodes.iter().find_map(|n| match n {
            ParsedNode::Text(_) => None,
            ParsedNode::Element {
                tag,
                attributes,
                children,
                ..
            } => {
                let tag = tag.to_ascii_lowercase();
                let task_list = tag == "ul" && is_task_list(attributes);
                element(&tag, attributes, in_task_list).or_else(|| walk(children, task_list))
            }
        })
    }
    let mut parser = HtmlFragmentParser::new(html);
    parser.all_attributes = true;
    walk(&parser.parse(), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s() -> Schema {
        Schema::starter_kit()
    }

    /// Serialize HTML, parse it back to a slice, wrap in a doc, re-serialize.
    fn reserialize_via_slice(schema: &Schema, html: &str) -> String {
        let slice = slice_from_html(schema, html).unwrap();
        let doc = Node::new_branch(
            schema.node_type("doc").unwrap().clone(),
            Attrs::new(),
            slice.content.clone(),
        );
        node_to_html(&doc)
    }

    // ── Tables (M7a) ─────────────────────────────────────────────────────────

    fn table_2x2(schema: &Schema) -> Node {
        let cell = |t: &str| {
            schema
                .branch(
                    "table_cell",
                    Fragment::from_node(
                        schema
                            .branch("paragraph", Fragment::from_node(schema.text(t).unwrap()))
                            .unwrap(),
                    ),
                )
                .unwrap()
        };
        let row = |a: &str, b: &str| {
            schema
                .branch("table_row", Fragment::from_children(vec![cell(a), cell(b)]))
                .unwrap()
        };
        schema
            .branch(
                "table",
                Fragment::from_children(vec![row("a", "b"), row("c", "d")]),
            )
            .unwrap()
    }

    #[test]
    fn table_serializes_to_html() {
        let schema = s();
        assert_eq!(
            node_to_html(&table_2x2(&schema)),
            "<table><tr><td><p>a</p></td><td><p>b</p></td></tr>\
             <tr><td><p>c</p></td><td><p>d</p></td></tr></table>"
        );
    }

    #[test]
    fn schema_accepts_a_wellformed_table() {
        let schema = s();
        let table = schema.node_type("table").unwrap();
        assert!(table.content_match().matches(&["table_row", "table_row"]));
        assert!(
            !table.content_match().matches(&["table_cell"]),
            "a cell may not sit directly in a table"
        );
        let row = schema.node_type("table_row").unwrap();
        // The `cell` group accepts both cell kinds (and a mix) in one row.
        assert!(
            row.content_match()
                .matches(&["table_header_cell", "table_cell"])
        );
        // A row may be empty (`cell*`): every column can be covered by a `rowspan`
        // cell from a row above (e.g. after merging a full-width rectangle).
        assert!(row.content_match().matches(&[]), "an empty row is allowed");
        assert!(
            schema
                .node_type("table_cell")
                .unwrap()
                .content_match()
                .matches(&["paragraph"])
        );
        // A table is a block, so it can be inserted wherever block content goes.
        assert!(
            schema
                .node_type("doc")
                .unwrap()
                .content_match()
                .accepts("table")
        );
    }

    #[test]
    fn table_round_trips_through_html() {
        let schema = s();
        let html = "<table><tr><td><p>a</p></td><td><p>b</p></td></tr></table>";
        assert_eq!(reserialize_via_slice(&schema, html), html);
    }

    #[test]
    fn table_colspan_rowspan_round_trip() {
        // `colspan`/`rowspan` must survive both parse (the `filter_attributes`
        // whitelist) and serialize (`block_tags`) — without them a merged cell
        // silently un-merges, which the CSS-grid view then can't render.
        let schema = s();
        // (The rowspan needs a row to span into: one past the table's last
        // row is cut at import, as Chrome cuts it — #1176.)
        let html = "<table><tr><th colspan=\"2\"><p>h</p></th></tr>\
                    <tr><td rowspan=\"2\"><p>a</p></td><td><p>b</p></td></tr>\
                    <tr><td><p>c</p></td></tr></table>";
        assert_eq!(reserialize_via_slice(&schema, html), html);
        // And the parsed model actually carries the spans.
        let slice = slice_from_html(&schema, html).unwrap();
        let table = slice.content.child(0);
        let header_cell = table.content().child(0).content().child(0);
        assert_eq!(header_cell.attrs().get_int("colspan"), Some(2));
        let span_cell = table.content().child(1).content().child(0);
        assert_eq!(span_cell.attrs().get_int("rowspan"), Some(2));
    }

    #[test]
    fn serialize_heading_and_paragraph() {
        let schema = s();
        let heading = Node::new_branch(
            schema.node_type("heading").unwrap().clone(),
            Attrs::from_iter([("level", AttrValue::Int(2))]),
            Fragment::from_node(schema.text("Title").unwrap()),
        );
        let para = Node::new_branch(
            schema.node_type("paragraph").unwrap().clone(),
            Attrs::new(),
            Fragment::from_node(schema.text("Body").unwrap()),
        );
        let doc = Node::new_branch(
            schema.node_type("doc").unwrap().clone(),
            Attrs::new(),
            Fragment::from_children(vec![heading, para]),
        );
        assert_eq!(node_to_html(&doc), "<h2>Title</h2><p>Body</p>");
    }

    #[test]
    fn serialize_text_align_as_inline_style() {
        let schema = s();
        let block = |ty: &str, align: &str, text: &str| {
            Node::new_branch(
                schema.node_type(ty).unwrap().clone(),
                Attrs::from_iter([("text_align", AttrValue::from(align))]),
                Fragment::from_node(schema.text(text).unwrap()),
            )
        };
        // Center / right / justify project to a `text-align` inline style; the default
        // `left` (and any unrecognized value) is omitted so plain text stays clean.
        assert_eq!(
            node_to_html(&block("paragraph", "center", "c")),
            r#"<p style="text-align:center">c</p>"#
        );
        assert_eq!(
            node_to_html(&block("paragraph", "right", "r")),
            r#"<p style="text-align:right">r</p>"#
        );
        assert_eq!(
            node_to_html(&block("heading", "justify", "j")),
            r#"<h1 style="text-align:justify">j</h1>"#
        );
        assert_eq!(node_to_html(&block("paragraph", "left", "l")), "<p>l</p>");
        assert_eq!(
            node_to_html(&block("paragraph", "bogus; color:red", "x")),
            "<p>x</p>",
            "an unrecognized alignment must never leak into the style attribute"
        );
    }

    #[test]
    fn text_align_round_trips_through_html() {
        let schema = s();
        // The alignment the serializer emits must survive being parsed back — this
        // is the copy/paste path (`selection_clipboard` -> `replace_selection_with_html`)
        // and `load_html` of previously exported markup.
        for (html, want) in [
            (r#"<p style="text-align:center">hi</p>"#, Some("center")),
            (r#"<p style="text-align:right">hi</p>"#, Some("right")),
            (r#"<h2 style="text-align:justify">hi</h2>"#, Some("justify")),
        ] {
            let slice = slice_from_html(&schema, html).unwrap();
            let block = slice.content.child(0);
            assert_eq!(block.attrs().get_str("text_align"), want, "parsing {html}");
            // Full round-trip: the re-serialized markup matches the input.
            assert_eq!(node_to_html(block), html, "re-serializing {html}");
        }
    }

    #[test]
    fn text_align_parse_is_whitelisted_and_tolerant() {
        let schema = s();
        let align_of = |html: &str| {
            slice_from_html(&schema, html)
                .unwrap()
                .content
                .child(0)
                .attrs()
                .get_str("text_align")
                .map(str::to_string)
        };
        // Tolerant of real-world CSS shapes the serializer itself never emits.
        assert_eq!(
            align_of(r#"<p style="text-align: CENTER;">x</p>"#).as_deref(),
            Some("center")
        );
        assert_eq!(
            align_of(r#"<p style="color:red; text-align:right">x</p>"#).as_deref(),
            Some("right")
        );
        // Everything else leaves `text_align` **absent**. A value outside the
        // whitelist is dropped rather than trusted, and an unset optional attribute
        // is not materialized to its default — the view and the serializer resolve a
        // missing alignment to left-aligned at the point of use.
        for html in [
            r#"<p style="text-align:left">x</p>"#, // the default, stated explicitly
            r#"<p style="text-align:end">x</p>"#,  // valid CSS, outside the whitelist
            r#"<p style="text-align:inherit">x</p>"#,
            r#"<p>x</p>"#, // no style at all
        ] {
            assert_eq!(
                align_of(html),
                None,
                "an unrecognized alignment must be dropped, not stored: {html}"
            );
        }
        // A rejected alignment must also not leak into the re-serialized markup.
        let slice = slice_from_html(&schema, r#"<p style="text-align:end">x</p>"#).unwrap();
        assert_eq!(node_to_html(slice.content.child(0)), "<p>x</p>");
    }

    #[test]
    #[cfg(feature = "serde")]
    fn text_align_round_trips_through_doc_json() {
        use crate::serialize::doc_json::DocNode;
        let schema = s();
        let para = Node::new_branch(
            schema.node_type("paragraph").unwrap().clone(),
            Attrs::from_iter([("text_align", AttrValue::from("center"))]),
            Fragment::from_node(schema.text("hi").unwrap()),
        );
        let doc = Node::new_branch(
            schema.node_type("doc").unwrap().clone(),
            Attrs::new(),
            Fragment::from_node(para),
        );
        // Node -> DocNode -> JSON string -> DocNode -> Node preserves the alignment.
        let wire = doc.to_doc().unwrap();
        let json = serde_json::to_string(&wire).unwrap();
        assert!(
            json.contains(r#""text_align":"center""#),
            "alignment must appear on the wire: {json}"
        );
        let parsed: DocNode = serde_json::from_str(&json).unwrap();
        let back = schema.node_from_doc(&parsed).unwrap();
        assert_eq!(
            back.content().child(0).attrs().get_str("text_align"),
            Some("center")
        );
        // And it still projects to the inline style after the round-trip.
        assert_eq!(
            node_to_html(&back),
            r#"<p style="text-align:center">hi</p>"#
        );
    }

    #[test]
    fn serialize_marks_nested_per_run() {
        let schema = s();
        let bold = Mark::new(schema.mark_type("bold").unwrap().clone(), Attrs::new());
        let link = Mark::new(
            schema.mark_type("link").unwrap().clone(),
            schema
                .mark_type("link")
                .unwrap()
                .compute_attrs(&Attrs::from_iter([(
                    "href",
                    AttrValue::from("https://x.io"),
                )]))
                .unwrap(),
        );
        let para = Node::new_branch(
            schema.node_type("paragraph").unwrap().clone(),
            Attrs::new(),
            Fragment::from_children(vec![
                schema.text_with_marks("a", vec![bold]).unwrap(),
                schema.text_with_marks("b", vec![link]).unwrap(),
            ]),
        );
        assert_eq!(
            node_to_html(&para),
            r#"<p><strong>a</strong><a href="https://x.io">b</a></p>"#
        );
    }

    #[test]
    fn serialize_list_items_with_combined_marks() {
        // A bullet list whose items carry bold, italic, and bold+italic runs —
        // each run wraps innermost-first (marks[0] innermost), matching copy-out
        // HTML. Combined marks nest: [bold, italic] → <em><strong>…</strong></em>.
        let schema = s();
        let bold = || Mark::new(schema.mark_type("bold").unwrap().clone(), Attrs::new());
        let italic = || Mark::new(schema.mark_type("italic").unwrap().clone(), Attrs::new());
        let item = |run: Node| {
            let p = Node::new_branch(
                schema.node_type("paragraph").unwrap().clone(),
                Attrs::new(),
                Fragment::from_node(run),
            );
            Node::new_branch(
                schema.node_type("list_item").unwrap().clone(),
                Attrs::new(),
                Fragment::from_node(p),
            )
        };
        let list = Node::new_branch(
            schema.node_type("bullet_list").unwrap().clone(),
            Attrs::new(),
            Fragment::from_children(vec![
                item(schema.text_with_marks("a", vec![bold()]).unwrap()),
                item(schema.text_with_marks("b", vec![italic()]).unwrap()),
                item(schema.text_with_marks("c", vec![bold(), italic()]).unwrap()),
            ]),
        );
        assert_eq!(
            node_to_html(&list),
            "<ul><li><p><strong>a</strong></p></li>\
             <li><p><em>b</em></p></li>\
             <li><p><em><strong>c</strong></em></p></li></ul>"
        );
    }

    #[test]
    fn serialize_escapes_text_and_attrs() {
        let schema = s();
        let link = Mark::new(
            schema.mark_type("link").unwrap().clone(),
            schema
                .mark_type("link")
                .unwrap()
                .compute_attrs(&Attrs::from_iter([(
                    "href",
                    AttrValue::from("https://x.io/?a=1&b=2"),
                )]))
                .unwrap(),
        );
        let para = Node::new_branch(
            schema.node_type("paragraph").unwrap().clone(),
            Attrs::new(),
            Fragment::from_children(vec![
                schema.text("a < b & c > d").unwrap(),
                schema.text_with_marks("link", vec![link]).unwrap(),
            ]),
        );
        let html = node_to_html(&para);
        assert!(html.contains("a &lt; b &amp; c &gt; d"), "{html}");
        assert!(
            html.contains(r#"href="https://x.io/?a=1&amp;b=2""#),
            "{html}"
        );
    }

    #[test]
    fn serialize_void_elements() {
        let schema = s();
        let img = Node::new_branch(
            schema.node_type("image").unwrap().clone(),
            schema
                .node_type("image")
                .unwrap()
                .compute_attrs(&Attrs::from_iter([("src", AttrValue::from("a.png"))]))
                .unwrap(),
            Fragment::empty(),
        );
        let para = Node::new_branch(
            schema.node_type("paragraph").unwrap().clone(),
            Attrs::new(),
            Fragment::from_children(vec![
                img,
                Node::new_branch(
                    schema.node_type("hard_break").unwrap().clone(),
                    Attrs::new(),
                    Fragment::empty(),
                ),
            ]),
        );
        assert_eq!(node_to_html(&para), r#"<p><img src="a.png"><br></p>"#);
    }

    #[test]
    fn parse_drops_script_and_iframe() {
        let schema = s();
        let out = reserialize_via_slice(
            &schema,
            "<p>safe</p><script>alert(1)</script><iframe src=\"evil\"></iframe><p>after</p>",
        );
        assert!(!out.contains("alert"), "{out}");
        assert!(!out.contains("iframe"), "{out}");
        assert!(out.contains("safe"));
        assert!(out.contains("after"));
    }

    #[test]
    fn parse_drops_event_handler_attrs() {
        let schema = s();
        let slice = slice_from_html(&schema, r#"<p onclick="steal()">hi</p>"#).unwrap();
        let html = node_to_html(&Node::new_branch(
            schema.node_type("doc").unwrap().clone(),
            Attrs::new(),
            slice.content,
        ));
        assert!(!html.contains("onclick"), "{html}");
        assert!(html.contains("hi"));
    }

    #[test]
    fn parse_span_style_to_text_color() {
        let schema = s();
        let slice =
            slice_from_html(&schema, r#"<p><span style="color: red">x</span></p>"#).unwrap();
        // dig into doc > paragraph > text
        let para = slice.content.child(0);
        let text = para.child(0);
        assert_eq!(text.text(), Some("x"));
        assert!(
            text.marks()
                .iter()
                .any(|m| m.type_name() == "text_color" && m.attrs.get_str("color") == Some("red")),
            "marks: {:?}",
            text.marks()
        );
    }

    #[test]
    fn parse_preserves_bold_link_marks() {
        let schema = s();
        let slice =
            slice_from_html(&schema, r#"<p><b><a href="https://x.io">hi</a></b></p>"#).unwrap();
        let text = slice.content.child(0).child(0);
        let names: Vec<&str> = text.marks().iter().map(|m| m.type_name()).collect();
        assert!(names.contains(&"bold"), "{names:?}");
        assert!(names.contains(&"link"), "{names:?}");
        assert_eq!(
            text.marks()
                .iter()
                .find(|m| m.type_name() == "link")
                .and_then(|m| m.attrs.get_str("href")),
            Some("https://x.io")
        );
    }

    #[test]
    fn parse_strips_javascript_url_link() {
        let schema = s();
        let slice =
            slice_from_html(&schema, r#"<p><a href="javascript:alert(1)">x</a></p>"#).unwrap();
        let text = slice.content.child(0).child(0);
        assert_eq!(text.text(), Some("x"));
        assert!(
            text.marks().is_empty(),
            "javascript: link must be dropped, got {:?}",
            text.marks()
        );
    }

    #[test]
    fn parse_nested_list() {
        let schema = s();
        let slice = slice_from_html(&schema, "<ul><li>one</li><li>two</li></ul>").unwrap();
        let list = slice.content.child(0);
        assert_eq!(list.type_name(), "bullet_list");
        assert_eq!(list.child_count(), 2);
        let item0 = list.child(0);
        assert_eq!(item0.type_name(), "list_item");
        assert_eq!(item0.child(0).type_name(), "paragraph");
        assert_eq!(item0.child(0).child(0).text(), Some("one"));
    }

    #[test]
    fn parse_blockquote_wraps_inline() {
        let schema = s();
        let slice = slice_from_html(&schema, "<blockquote>quoted</blockquote>").unwrap();
        let bq = slice.content.child(0);
        assert_eq!(bq.type_name(), "blockquote");
        assert_eq!(bq.child(0).type_name(), "paragraph");
        assert_eq!(bq.child(0).child(0).text(), Some("quoted"));
    }

    #[test]
    fn parse_bare_inline_is_open_slice() {
        let schema = s();
        let slice = slice_from_html(&schema, "<strong>hi</strong>").unwrap();
        assert_eq!(slice.content.child_count(), 1);
        assert_eq!(slice.content.child(0).type_name(), "paragraph");
        // textblock first & last → open 1/1 so it merges inline on paste.
        assert_eq!((slice.open_start, slice.open_end), (1, 1));
    }

    #[test]
    fn parse_then_serialize_round_trips_common_html() {
        let schema = s();
        let html = "<h1>Title</h1><p>Some <strong>bold</strong> and <em>italic</em>.</p>";
        assert_eq!(reserialize_via_slice(&schema, html), html);
    }

    #[test]
    fn parse_ordered_list_start() {
        let schema = s();
        let slice = slice_from_html(&schema, r#"<ol start="3"><li>x</li></ol>"#).unwrap();
        let list = slice.content.child(0);
        assert_eq!(list.type_name(), "ordered_list");
        assert_eq!(list.attrs().get_int("start"), Some(3));
    }

    #[test]
    fn parse_code_block() {
        let schema = s();
        let slice = slice_from_html(&schema, "<pre>fn main() {}</pre>").unwrap();
        let code = slice.content.child(0);
        assert_eq!(code.type_name(), "code_block");
        assert_eq!(code.child(0).text(), Some("fn main() {}"));
    }

    #[test]
    fn parse_multibyte_comment_does_not_panic() {
        // Regression: the tokenizer byte-advanced then str-sliced inside comments.
        let schema = s();
        let slice = slice_from_html(&schema, "<!--é comment 🎉--><p>after</p>").unwrap();
        assert_eq!(slice.content.child(0).child(0).text(), Some("after"));
        // unterminated comment with multibyte must also be safe
        assert!(slice_from_html(&schema, "<!--café no closer 🎉").is_ok());
    }

    #[test]
    fn parse_multibyte_in_dropped_tag_does_not_panic() {
        // Regression: skip_until_close_tag byte-advanced then str-sliced (case-fold).
        let schema = s();
        let slice = slice_from_html(&schema, "<style>café {}</style><p>hi</p>").unwrap();
        assert_eq!(slice.content.child(0).child(0).text(), Some("hi"));
        assert!(slice_from_html(&schema, "<script>let x='ünïcödé';</script><p>y</p>").is_ok());
    }

    #[test]
    fn parse_rejects_unsafe_css_color() {
        let schema = s();
        let slice = slice_from_html(
            &schema,
            r#"<p><span style="color: expression(alert(1))">x</span></p>"#,
        )
        .unwrap();
        let text = slice.content.child(0).child(0);
        assert_eq!(text.text(), Some("x"));
        assert!(
            text.marks().is_empty(),
            "expression() color must be rejected, got {:?}",
            text.marks()
        );
        // a legitimate color is still accepted
        let ok =
            slice_from_html(&schema, r#"<p><span style="color:#ff0000">y</span></p>"#).unwrap();
        assert!(
            ok.content
                .child(0)
                .child(0)
                .marks()
                .iter()
                .any(|m| m.type_name() == "text_color")
        );
    }

    #[test]
    fn parse_image_data_uri_svg_rejected_raster_allowed() {
        let schema = s();
        // svg+xml can carry script → dropped
        let svg = slice_from_html(
            &schema,
            r#"<p><img src="data:image/svg+xml,<svg onload=alert(1)>"></p>"#,
        )
        .unwrap();
        assert!(
            !svg.content
                .child(0)
                .content()
                .children()
                .iter()
                .any(|n| n.type_name() == "image"),
            "data:image/svg+xml must be dropped"
        );
        // a raster data URI is allowed
        let png =
            slice_from_html(&schema, r#"<p><img src="data:image/png;base64,AAAA"></p>"#).unwrap();
        assert!(
            png.content
                .child(0)
                .content()
                .children()
                .iter()
                .any(|n| n.type_name() == "image")
        );
    }

    #[test]
    fn serialize_target_blank_link_adds_rel_noopener() {
        let schema = s();
        let mt = schema.mark_type("link").unwrap();
        let attrs = mt
            .compute_attrs(&Attrs::from_iter([
                ("href", AttrValue::from("https://x.io")),
                ("target", AttrValue::from("_blank")),
            ]))
            .unwrap();
        let para = Node::new_branch(
            schema.node_type("paragraph").unwrap().clone(),
            Attrs::new(),
            Fragment::from_node(
                schema
                    .text_with_marks("x", vec![Mark::new(mt.clone(), attrs)])
                    .unwrap(),
            ),
        );
        let html = node_to_html(&para);
        assert!(html.contains(r#"target="_blank""#), "{html}");
        assert!(html.contains(r#"rel="noopener noreferrer""#), "{html}");
    }

    // ── HTML integer attributes (#1164) ──────────────────────────────────────

    /// The first `ordered_list`'s `start` after importing `<ol start="{v}">`.
    fn imported_ol_start(v: &str) -> Option<i64> {
        let html = format!("<ol start=\"{v}\"><li><p>x</p></li></ol>");
        let slice = slice_from_html(&s(), &html).unwrap();
        slice.content.child(0).attrs().get_int("start")
    }

    /// `(colspan, rowspan)` of the first cell after importing a one-row table
    /// whose first cell carries `attrs`.
    fn imported_first_cell_spans(attrs: &str) -> (i64, i64) {
        let html = format!("<table><tr><td {attrs}><p>a</p></td></tr></table>");
        let slice = slice_from_html(&s(), &html).unwrap();
        let cell = slice.content.child(0).child(0).child(0);
        (
            cell.attrs().get_int("colspan").unwrap(),
            cell.attrs().get_int("rowspan").unwrap(),
        )
    }

    /// `<ol start>` is read by HTML's rules for parsing integers, as Chrome
    /// 153's `ol.start` reads it — not by `str::parse`, which refused a leading
    /// space or trailing junk and accepted values past `i32`.
    #[test]
    fn ol_start_is_read_by_the_html_integer_rules() {
        let rows: &[(&str, i64)] = &[
            (" 3", 3),
            ("3abc", 3),
            ("2.5", 2),
            ("+4", 4),
            ("-2", -2),
            ("7", 7),
            // Errors read as the default, 1.
            ("abc", 1),
            ("", 1),
            ("99999999999", 1),
            ("2147483648", 1),
        ];
        for &(v, want) in rows {
            assert_eq!(imported_ol_start(v), Some(want), "start={v:?}");
        }
    }

    /// `colspan` is Chrome 153's `td.colSpan`: HTML's non-negative integer,
    /// clamped to 1..=1000; an overflow is the maximum, an error or a negative
    /// value is the default 1.
    #[test]
    fn colspan_is_read_and_clamped_as_chrome_reads_it() {
        let rows: &[(&str, i64)] = &[
            ("3abc", 3),
            (" 2", 2),
            ("4.9", 4),
            ("0", 1),
            ("-3", 1),
            ("abc", 1),
            ("1000", 1000),
            ("1001", 1000),
            ("99999999999", 1000),
            ("99999999999999999999999", 1000),
        ];
        for &(v, want) in rows {
            let attrs = format!("colspan=\"{v}\"");
            assert_eq!(imported_first_cell_spans(&attrs).0, want, "colspan={v:?}");
        }
    }

    /// `rowspan` is Chrome 153's `td.rowSpan` (clamped to 0..=65534, an
    /// overflow the maximum), cut to the rows left in the cell's row group,
    /// which is the span Chrome 153 lays out (#1176). Seven rows here, so a
    /// value past them is 7. (`0` is covered by the section tests below: the
    /// model has no "to the end" value, so it is resolved at import.)
    #[test]
    fn rowspan_is_read_as_chrome_reads_it_and_cut_to_its_row_group() {
        let rows: &[(&str, i64)] = &[
            ("3abc", 3),
            (" 2", 2),
            ("-3", 1),
            ("abc", 1),
            ("6", 6),
            ("7", 7),
            ("8", 7),
            ("65534", 7),
            ("99999999999", 7),
        ];
        for &(v, want) in rows {
            let html = format!(
                "<table><tr><td rowspan=\"{v}\"><p>a</p></td></tr>{}</table>",
                "<tr><td><p>b</p></td></tr>".repeat(6)
            );
            assert_eq!(imported_rowspans(&html)[0][0], want, "rowspan={v:?}");
        }
    }

    /// A rowspan does not reach past the end of its row group: Chrome 153
    /// cuts it there (measured: a `rowspan="3"` cell in a one-row `<tbody>`
    /// leaves the next `<tbody>`'s first cell in column 0). The import writes
    /// the cut span, so the model grid is the one Chrome drew (#1176).
    #[test]
    fn a_rowspan_is_cut_at_the_end_of_its_row_group() {
        let html = "<table><tbody>\
                    <tr><td rowspan=\"3\"><p>a</p></td><td><p>b</p></td></tr>\
                    </tbody><tbody>\
                    <tr><td><p>c</p></td><td><p>d</p></td></tr>\
                    </tbody></table>";
        assert_eq!(imported_rowspans(html), vec![vec![1, 1], vec![1, 1]]);
        let html = "<table>\
                    <tr><td rowspan=\"5\"><p>a</p></td><td rowspan=\"2\"><p>b</p></td></tr>\
                    <tr><td><p>c</p></td></tr>\
                    <tr><td><p>d</p></td><td><p>e</p></td></tr>\
                    </table>";
        assert_eq!(
            imported_rowspans(html),
            vec![vec![3, 2], vec![1], vec![1, 1]]
        );
    }

    /// Every cell's `colspan`, row by row, of the first table in `html`.
    fn imported_colspans(html: &str) -> Vec<Vec<i64>> {
        let slice = slice_from_html(&s(), html).unwrap();
        let table = slice.content.child(0);
        (0..table.child_count())
            .map(|r| {
                let row = table.child(r);
                (0..row.child_count())
                    .map(|c| row.child(c).attrs().get_int("colspan").unwrap())
                    .collect()
            })
            .collect()
    }

    /// The spans of a pasted row reach at most 1000 columns, counting the
    /// columns that rowspans from the rows above carry into it: a colspan is
    /// cut to what is left, never below 1, and a cell that starts in a row
    /// already 1000 wide spans that row alone, so it carries nothing down
    /// (#1176). Each further cell adds one column. 1000 is Chrome's largest
    /// colspan; Chrome has no limit on a row's width (it lays out 3001
    /// columns), so this one is rinch's.
    #[test]
    fn a_pasted_rows_spans_reach_at_most_1000_columns() {
        let html = "<table><tr><td colspan=\"600\"><p>a</p></td>\
                    <td colspan=\"600\"><p>b</p></td><td colspan=\"7\"><p>c</p></td></tr></table>";
        assert_eq!(imported_colspans(html), vec![vec![600, 400, 1]]);
        // Columns carried down by a rowspan count against the row below.
        let html = "<table><tr><td colspan=\"700\" rowspan=\"2\"><p>a</p></td></tr>\
                    <tr><td colspan=\"500\"><p>b</p></td></tr>\
                    <tr><td colspan=\"500\"><p>c</p></td></tr></table>";
        assert_eq!(
            imported_colspans(html),
            vec![vec![700], vec![300], vec![500]]
        );
        // ...but not across a row group, which the rowspan does not reach.
        let html = "<table><tbody><tr><td colspan=\"700\" rowspan=\"2\"><p>a</p></td></tr></tbody>\
                    <tbody><tr><td colspan=\"500\"><p>b</p></td></tr></tbody></table>";
        assert_eq!(imported_colspans(html), vec![vec![700], vec![500]]);
        // A cell that finds its row full spans one row, whatever it asked:
        // row 1 is full with row 0's 1000 carried columns, and in row 0 the
        // second cell starts after the first has filled it.
        let html = "<table>\
                    <tr><td colspan=\"1000\" rowspan=\"3\"><p>a</p></td>\
                    <td rowspan=\"3\"><p>b</p></td></tr>\
                    <tr><td colspan=\"4\" rowspan=\"2\"><p>c</p></td></tr>\
                    <tr><td><p>d</p></td></tr></table>";
        assert_eq!(
            imported_colspans(html),
            vec![vec![1000, 1], vec![1], vec![1]]
        );
        assert_eq!(imported_rowspans(html), vec![vec![3, 1], vec![1], vec![1]]);
    }

    /// Every cell's `rowspan`, row by row, of the first table in `html`.
    fn imported_rowspans(html: &str) -> Vec<Vec<i64>> {
        let slice = slice_from_html(&s(), html).unwrap();
        let table = slice.content.child(0);
        (0..table.child_count())
            .map(|r| {
                let row = table.child(r);
                (0..row.child_count())
                    .map(|c| row.child(c).attrs().get_int("rowspan").unwrap())
                    .collect()
            })
            .collect()
    }

    /// `rowspan="0"` spans to the end of the cell's row group, which is what
    /// Chrome 153 lays out. The model has no such value (a span is at least
    /// 1), so the import writes the number of rows left in the group — two
    /// explicit `<tbody>`s here, so the first cell spans its own three rows
    /// and not the table's four.
    #[test]
    fn rowspan_zero_spans_to_the_end_of_its_row_group() {
        let html = "<table><tbody>\
                    <tr><td rowspan=\"0\"><p>a</p></td><td><p>b</p></td></tr>\
                    <tr><td><p>c</p></td></tr>\
                    <tr><td><p>d</p></td></tr>\
                    </tbody><tbody>\
                    <tr><td><p>e</p></td></tr>\
                    </tbody></table>";
        assert_eq!(
            imported_rowspans(html),
            vec![vec![3, 1], vec![1], vec![1], vec![1]]
        );
    }

    /// Bare rows written before an explicit `<tbody>` are their own group, and
    /// keep their place: the group is closed where the wrapper opens.
    #[test]
    fn bare_rows_before_a_tbody_are_a_group_of_their_own() {
        let html = "<table>\
                    <tr><td rowspan=\"0\"><p>a</p></td></tr>\
                    <tr><td><p>b</p></td></tr>\
                    <tbody><tr><td><p>c</p></td></tr></tbody>\
                    </table>";
        assert_eq!(imported_rowspans(html), vec![vec![2], vec![1], vec![1]]);
        assert_eq!(
            reserialize_via_slice(&s(), html),
            "<table><tr><td rowspan=\"2\"><p>a</p></td></tr>\
             <tr><td><p>b</p></td></tr><tr><td><p>c</p></td></tr></table>"
        );
    }

    /// A run of bare `<tr>`s is one row group (the HTML parser wraps it in an
    /// implicit `<tbody>`), and one that follows an explicit group starts a new
    /// one. A `rowspan="0"` in a group's last row spans that row alone.
    #[test]
    fn rowspan_zero_in_bare_rows_and_a_groups_last_row() {
        let html = "<table><thead>\
                    <tr><th rowspan=\"0\"><p>h</p></th></tr>\
                    </thead>\
                    <tr><td rowspan=\"0\"><p>a</p></td></tr>\
                    <tr><td><p>b</p></td></tr>\
                    <tr><td><p>c</p></td><td rowspan=\"0\"><p>d</p></td></tr>\
                    </table>";
        assert_eq!(
            imported_rowspans(html),
            vec![vec![1], vec![3], vec![1], vec![1, 1]]
        );
    }

    // #705: `parse_style` must not treat a `;` or `:` inside `url(...)` or a
    // quoted string as a declaration boundary.

    /// A `;` inside an unquoted `url(...)` is part of the value, not a
    /// separator — a data URL is exactly where one turns up.
    #[test]
    fn parse_style_keeps_a_semicolon_inside_a_data_url_whole() {
        let style = "background-image: url(data:image/png;base64,QUJD); color: red";
        assert_eq!(
            parse_style(style, "background-image"),
            Some("url(data:image/png;base64,QUJD)".to_string())
        );
        assert_eq!(parse_style(style, "color"), Some("red".to_string()));
    }

    /// A `;` (and a `:`) inside a quoted `content` value must not fabricate a
    /// declaration that hijacks a later real one of the same name.
    #[test]
    fn parse_style_does_not_let_a_quoted_value_hijack_a_later_declaration() {
        let style = r#"content: "a; color: blue"; color: red"#;
        assert_eq!(parse_style(style, "color"), Some("red".to_string()));
        assert_eq!(
            parse_style(style, "content"),
            Some(r#""a; color: blue""#.to_string())
        );
    }
}
