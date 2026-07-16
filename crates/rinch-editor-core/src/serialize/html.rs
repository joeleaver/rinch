//! Schema-driven HTML serialization (copy-out) and parsing (paste-in).
//!
//! Both directions are driven by the schema's `parse_html_tags` (on `NodeSpec` and
//! `MarkSpec`) — one table, used both ways — unifying the old engine's three
//! divergent hardcoded tag maps (`wrap_mark`, `block_type_to_tag`,
//! `mark_type_to_tag`). A few genuinely attr-dependent cases (heading level,
//! ordered-list `start`, `<span style>` → `text_color`/`highlight`) are handled
//! explicitly, mirroring ProseMirror's per-type `toDOM`/`parseDOM`.
//!
//! HTML is a **lossy clipboard interchange** — the durable, total format is
//! [`super::doc_json`]. The parse direction is a **whitelist**: unknown tags,
//! marks, and attributes are dropped; `<script>`/`<iframe>`/`<object>`/`<embed>`
//! never materialize; `javascript:`/`vbscript:` URLs are stripped. This kills the
//! audit's raw-DOM-paste hole.
//!
//! The zero-dependency tokenizer is salvaged from `rinch/src/app/html_parser.rs`.

use crate::EditorError;
use crate::model::{AttrValue, Attrs, Fragment, Mark, MarkType, Node, NodeType, Slice};
use crate::schema::Schema;
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
/// on paste by [`slice_from_html`]'s open-depth heuristic. So a within-block copy
/// (whose `content` is the bare inline runs) round-trips back into a textblock, and
/// a multi-block copy emits its `<p>`/`<h1>`/… children directly.
pub fn slice_to_html(slice: &Slice) -> String {
    let mut out = String::new();
    for child in slice.content.children() {
        write_node(child, &mut out);
    }
    out
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
        "table_cell" | "table_header_cell" => {
            let tag = primary_tag(node.node_type());
            let mut open = format!("<{tag}");
            for (name, attr) in [("colspan", "colspan"), ("rowspan", "rowspan")] {
                let n = node.attrs().get_int(attr).unwrap_or(1);
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
/// declared tags (e.g. `task_list`). HTML is lossy here by design; DocNode is not.
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
        "text_color" => {
            let color = mark.attrs.get_str("color").unwrap_or("");
            format!(
                "<span style=\"color:{}\">{}</span>",
                escape_attr(color),
                inner
            )
        }
        "highlight" => {
            let color = mark.attrs.get_str("color").unwrap_or("");
            if color.is_empty() {
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
/// The result is a structurally valid slice with an open-depth heuristic suitable
/// for `replaceSelection` (M6): the first/last block is "open" when it is a
/// textblock, so pasted leading/trailing paragraphs merge inline at the insertion
/// point. Unknown tags/marks/attrs are dropped; dangerous elements never
/// materialize.
pub fn slice_from_html(schema: &Schema, html: &str) -> Result<Slice, EditorError> {
    let forest = parse_html_fragment(html);
    let parser = HtmlParser::new(schema)?;
    let blocks = parser.parse_blocks(&forest)?;
    if blocks.is_empty() {
        return Ok(Slice::empty());
    }
    let open_start = usize::from(blocks.first().is_some_and(Node::is_textblock));
    let open_end = usize::from(blocks.last().is_some_and(Node::is_textblock));
    Ok(Slice::new(
        Fragment::from_children(blocks),
        open_start,
        open_end,
    ))
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
        Ok(Self {
            schema,
            block,
            inline_leaf,
            marks,
            paragraph,
            list_item,
        })
    }

    /// True if `tag` should be parsed as inline content at block level.
    fn is_inline_tag(&self, tag: &str) -> bool {
        self.inline_leaf.contains_key(tag)
            || self.marks.contains_key(tag)
            || matches!(
                tag,
                "span"
                    | "font"
                    | "abbr"
                    | "cite"
                    | "q"
                    | "small"
                    | "big"
                    | "var"
                    | "kbd"
                    | "samp"
                    | "time"
                    | "bdi"
                    | "bdo"
            )
    }

    fn parse_blocks(&self, nodes: &[ParsedNode]) -> Result<Vec<Node>, EditorError> {
        let mut blocks: Vec<Node> = Vec::new();
        let mut loose: Vec<Node> = Vec::new();

        for n in nodes {
            match n {
                ParsedNode::Text(t) => {
                    // Inter-block whitespace is dropped; meaningful text accumulates.
                    if !t.trim().is_empty() {
                        self.parse_inline(std::slice::from_ref(n), &[], &mut loose)?;
                    }
                }
                ParsedNode::Element { tag, children, .. } => {
                    if is_dropped(tag) {
                        continue;
                    }
                    if let Some(nt) = self.block.get(tag.as_str()) {
                        self.flush_loose(&mut loose, &mut blocks)?;
                        blocks.push(self.build_block(nt, tag, n)?);
                    } else if self.is_inline_tag(tag) {
                        self.parse_inline(std::slice::from_ref(n), &[], &mut loose)?;
                    } else {
                        // Unknown container (div, section, …): recurse as blocks.
                        let inner = self.parse_blocks(children)?;
                        if inner.is_empty() {
                            self.parse_inline(children, &[], &mut loose)?;
                        } else {
                            self.flush_loose(&mut loose, &mut blocks)?;
                            blocks.extend(inner);
                        }
                    }
                }
            }
        }
        self.flush_loose(&mut loose, &mut blocks)?;
        Ok(blocks)
    }

    /// Flush accumulated inline content as a paragraph block.
    fn flush_loose(
        &self,
        loose: &mut Vec<Node>,
        blocks: &mut Vec<Node>,
    ) -> Result<(), EditorError> {
        if loose.is_empty() {
            return Ok(());
        }
        let content = Fragment::from_children(std::mem::take(loose));
        blocks.push(self.make_node(self.paragraph, Attrs::new(), content)?);
        Ok(())
    }

    fn build_block(
        &self,
        nt: &NodeType,
        tag: &str,
        element: &ParsedNode,
    ) -> Result<Node, EditorError> {
        let (attributes, children) = match element {
            ParsedNode::Element {
                attributes,
                children,
                ..
            } => (attributes.as_slice(), children.as_slice()),
            ParsedNode::Text(_) => (&[][..], &[][..]),
        };
        match nt.name() {
            "heading" => {
                let level = heading_level(tag);
                let content = self.parse_inline_children(children)?;
                self.make_node(
                    nt,
                    Attrs::from_iter([("level", AttrValue::Int(level))]),
                    content,
                )
            }
            "code_block" => {
                let mut text = String::new();
                collect_text(children, &mut text);
                let content = if text.is_empty() {
                    Fragment::empty()
                } else {
                    Fragment::from_node(self.schema.text(&text)?)
                };
                self.make_node(nt, Attrs::new(), content)
            }
            "blockquote" => {
                let inner = self.ensure_block_plus(self.parse_blocks(children)?)?;
                self.make_node(nt, Attrs::new(), Fragment::from_children(inner))
            }
            "bullet_list" | "ordered_list" => {
                let mut items = self.parse_list_items(children)?;
                if items.is_empty() {
                    items.push(self.empty_list_item()?);
                }
                let attrs = if nt.name() == "ordered_list" {
                    let start = attr(attributes, "start")
                        .and_then(|s| s.parse::<i64>().ok())
                        .unwrap_or(1);
                    Attrs::from_iter([("start", AttrValue::Int(start))])
                } else {
                    Attrs::new()
                };
                self.make_node(nt, attrs, Fragment::from_children(items))
            }
            "list_item" => {
                let inner = self.ensure_block_plus(self.parse_blocks(children)?)?;
                self.make_node(nt, Attrs::new(), Fragment::from_children(inner))
            }
            "horizontal_rule" => self.make_node(nt, Attrs::new(), Fragment::empty()),
            "table" => {
                let mut rows = self.parse_table_rows(children)?;
                // `table > table_row+` must be non-empty: a table with no valid rows
                // gets one empty single-cell row rather than an invalid node.
                if rows.is_empty() {
                    rows.push(self.empty_table_row()?);
                }
                self.make_node(nt, Attrs::new(), Fragment::from_children(rows))
            }
            _ => {
                // Any other textblock-ish block: treat content as inline.
                let _ = attributes;
                let content = self.parse_inline_children(children)?;
                self.make_node(nt, Attrs::new(), content)
            }
        }
    }

    fn parse_list_items(&self, children: &[ParsedNode]) -> Result<Vec<Node>, EditorError> {
        let mut items = Vec::new();
        for child in children {
            // Only <li> children become items; whitespace / stray tags are dropped.
            let ParsedNode::Element {
                tag,
                children: li_children,
                ..
            } = child
            else {
                continue;
            };
            if tag != "li" || is_dropped(tag) {
                continue;
            }
            let inner = self.ensure_block_plus(self.parse_blocks(li_children)?)?;
            items.push(self.make_node(
                self.list_item,
                Attrs::new(),
                Fragment::from_children(inner),
            )?);
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

    /// A table node type by name, erroring if the schema lacks tables (only
    /// reachable when `<table>` was a known block, so this is a schema-consistency
    /// guard rather than an expected path).
    fn table_node_type(&self, name: &str) -> Result<&'a NodeType, EditorError> {
        self.schema
            .node_type(name)
            .ok_or_else(|| EditorError::HtmlParse(format!("schema has no '{name}' type")))
    }

    /// Parse a table's `<tr>` children into `table_row` nodes, transparently
    /// descending through `<thead>`/`<tbody>`/`<tfoot>` wrappers. Rows with no
    /// recognized cells are dropped.
    fn parse_table_rows(&self, children: &[ParsedNode]) -> Result<Vec<Node>, EditorError> {
        let row_type = self.table_node_type("table_row")?;
        let mut rows = Vec::new();
        for child in children {
            let ParsedNode::Element {
                tag,
                children: tr_children,
                ..
            } = child
            else {
                continue;
            };
            if is_dropped(tag) {
                continue;
            }
            if matches!(tag.as_str(), "thead" | "tbody" | "tfoot") {
                rows.extend(self.parse_table_rows(tr_children)?);
                continue;
            }
            if tag != "tr" {
                continue;
            }
            let cells = self.parse_table_cells(tr_children)?;
            if cells.is_empty() {
                continue;
            }
            rows.push(self.make_node(row_type, Attrs::new(), Fragment::from_children(cells))?);
        }
        Ok(rows)
    }

    /// Parse a row's `<td>`/`<th>` children into cell nodes (block content, with
    /// `colspan`/`rowspan` carried through). Non-cell children are dropped.
    fn parse_table_cells(&self, children: &[ParsedNode]) -> Result<Vec<Node>, EditorError> {
        let cell_type = self.table_node_type("table_cell")?;
        let header_type = self.table_node_type("table_header_cell")?;
        let mut cells = Vec::new();
        for child in children {
            let ParsedNode::Element {
                tag,
                children: cell_children,
                attributes,
            } = child
            else {
                continue;
            };
            let nt = match tag.as_str() {
                "td" => cell_type,
                "th" => header_type,
                _ => continue,
            };
            let inner = self.ensure_block_plus(self.parse_blocks(cell_children)?)?;
            cells.push(self.make_node(
                nt,
                self.cell_span_attrs(attributes),
                Fragment::from_children(inner),
            )?);
        }
        Ok(cells)
    }

    /// `colspan`/`rowspan` attrs parsed from a cell element (defaulting to 1, the
    /// minimum) — the only table attributes that survive paste.
    fn cell_span_attrs(&self, attributes: &[(String, String)]) -> Attrs {
        let span = |name: &str| {
            attr(attributes, name)
                .and_then(|s| s.parse::<i64>().ok())
                .filter(|&n| n >= 1)
                .unwrap_or(1)
        };
        Attrs::from_iter([
            ("colspan", AttrValue::Int(span("colspan"))),
            ("rowspan", AttrValue::Int(span("rowspan"))),
        ])
    }

    /// A `table_row` holding one empty (single-paragraph) `table_cell` — the
    /// fallback for a `<table>` that parsed no usable rows.
    fn empty_table_row(&self) -> Result<Node, EditorError> {
        let para = self.make_node(self.paragraph, Attrs::new(), Fragment::empty())?;
        let cell = self.make_node(
            self.table_node_type("table_cell")?,
            self.cell_span_attrs(&[]),
            Fragment::from_node(para),
        )?;
        self.make_node(
            self.table_node_type("table_row")?,
            Attrs::new(),
            Fragment::from_node(cell),
        )
    }

    fn parse_inline_children(&self, children: &[ParsedNode]) -> Result<Fragment, EditorError> {
        let mut out = Vec::new();
        self.parse_inline(children, &[], &mut out)?;
        Ok(Fragment::from_children(out))
    }

    fn parse_inline(
        &self,
        nodes: &[ParsedNode],
        active: &[Mark],
        out: &mut Vec<Node>,
    ) -> Result<(), EditorError> {
        for n in nodes {
            match n {
                ParsedNode::Text(t) => {
                    if !t.is_empty() {
                        out.push(self.schema.text_with_marks(t, active.to_vec())?);
                    }
                }
                ParsedNode::Element {
                    tag,
                    attributes,
                    children,
                } => {
                    if is_dropped(tag) {
                        continue;
                    }
                    // Inline leaves (br, img).
                    if let Some(nt) = self.inline_leaf.get(tag.as_str()) {
                        if let Some(leaf) = self.build_inline_leaf(nt, attributes, active)? {
                            out.push(leaf);
                        }
                        continue;
                    }
                    // Mark-bearing tags (strong, em, a, mark, …).
                    if self.marks.contains_key(tag.as_str()) {
                        if let Some(mark) = self.mark_for(tag, attributes)? {
                            let next = mark.add_to_set(active);
                            self.parse_inline(children, &next, out)?;
                        } else {
                            // e.g. <a> with no/unsafe href → transparent, keep text.
                            self.parse_inline(children, active, out)?;
                        }
                        continue;
                    }
                    // <span style>: style-derived marks (color → text_color, bg → highlight).
                    if tag == "span" {
                        let next = self.span_marks(attributes, active)?;
                        self.parse_inline(children, &next, out)?;
                        continue;
                    }
                    // Any other inline element (font, etc.) → transparent.
                    self.parse_inline(children, active, out)?;
                }
            }
        }
        Ok(())
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
                if let Some(alt) = attr(attributes, "alt") {
                    pairs.push(("alt", AttrValue::from(alt)));
                }
                if let Some(title) = attr(attributes, "title") {
                    pairs.push(("title", AttrValue::from(title)));
                }
                let img = self.make_node(nt, Attrs::from_iter(pairs), Fragment::empty())?;
                Ok(Some(if active.is_empty() {
                    img
                } else {
                    img.mark(active.to_vec())
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
            _ => {
                let attrs = mt.compute_attrs(&Attrs::new())?;
                Ok(Some(Mark::new(mt.clone(), attrs)))
            }
        }
    }

    /// Marks contributed by a `<span style>`: `color` → `text_color`,
    /// `background-color` → `highlight`. Returns the new active mark set.
    fn span_marks(
        &self,
        attributes: &[(String, String)],
        active: &[Mark],
    ) -> Result<Vec<Mark>, EditorError> {
        let mut next = active.to_vec();
        let Some(style) = attr(attributes, "style") else {
            return Ok(next);
        };
        if let (Some(color), Some(mt)) = (
            parse_style(style, "color").filter(|c| is_safe_css_color(c)),
            self.schema.mark_type("text_color"),
        ) {
            let attrs = mt.compute_attrs(&Attrs::from_iter([("color", AttrValue::from(color))]))?;
            next = Mark::new(mt.clone(), attrs).add_to_set(&next);
        }
        if let (Some(bg), Some(mt)) = (
            parse_style(style, "background-color").filter(|c| is_safe_css_color(c)),
            self.schema.mark_type("highlight"),
        ) {
            let attrs = mt.compute_attrs(&Attrs::from_iter([("color", AttrValue::from(bg))]))?;
            next = Mark::new(mt.clone(), attrs).add_to_set(&next);
        }
        Ok(next)
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

/// Tags whose entire subtree is dropped on paste (never materialize).
fn is_dropped(tag: &str) -> bool {
    matches!(
        tag,
        "script"
            | "style"
            | "meta"
            | "link"
            | "head"
            | "title"
            | "iframe"
            | "object"
            | "embed"
            | "noscript"
            | "base"
            | "form"
            | "input"
            | "button"
            | "select"
            | "textarea"
            | "svg"
            | "math"
            | "applet"
            | "frame"
            | "frameset"
    )
}

/// The heading level encoded in an `h1`..`h6` tag (defaults to 1).
fn heading_level(tag: &str) -> i64 {
    tag.strip_prefix('h')
        .and_then(|d| d.parse::<i64>().ok())
        .filter(|l| (1..=6).contains(l))
        .unwrap_or(1)
}

/// Recursively gather text content (ignoring tags) — used for code blocks.
fn collect_text(nodes: &[ParsedNode], out: &mut String) {
    for n in nodes {
        match n {
            ParsedNode::Text(t) => out.push_str(t),
            ParsedNode::Element { tag, children, .. } => {
                if !is_dropped(tag) {
                    collect_text(children, out);
                }
            }
        }
    }
}

/// Find an attribute value (case-insensitive name) in a parsed attribute list.
fn attr<'b>(attributes: &'b [(String, String)], name: &str) -> Option<&'b str> {
    attributes
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// Extract a single CSS property value from an inline `style` attribute.
fn parse_style(style: &str, property: &str) -> Option<String> {
    for decl in style.split(';') {
        if let Some((k, v)) = decl.split_once(':')
            && k.trim().eq_ignore_ascii_case(property)
        {
            let value = v.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
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

// ─── Salvaged zero-dependency HTML tokenizer ─────────────────────────────────
//
// Ported from `rinch/src/app/html_parser.rs`. Produces a tree of `ParsedNode`s
// from a (possibly messy, clipboard-sourced) HTML fragment. Strips
// `<html>/<head>/<body>` wrappers and `<script>/<style>/<meta>/<link>/<title>`
// subtrees; attribute whitelisting drops `on*` handlers and anything not needed
// by the schema mapper.

/// A parsed HTML node.
#[derive(Debug, Clone)]
enum ParsedNode {
    Text(String),
    Element {
        tag: String,
        attributes: Vec<(String, String)>,
        children: Vec<ParsedNode>,
    },
}

fn parse_html_fragment(html: &str) -> Vec<ParsedNode> {
    let mut parser = HtmlFragmentParser::new(html);
    let nodes = parser.parse();
    unwrap_body(nodes)
}

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
    let mut result = Vec::new();
    let mut found_wrapper = false;
    for node in &nodes {
        if let ParsedNode::Element { tag, children, .. } = node {
            if tag == "html" || tag == "body" {
                result.extend(unwrap_body(children.clone()));
                found_wrapper = true;
            } else if tag == "head" || tag == "meta" || tag == "style" || tag == "script" {
                found_wrapper = true;
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
            if let Some(end) = end_tag
                && self.looking_at_close_tag(end)
            {
                self.consume_close_tag();
                break;
            }
            if self.peek() == Some('<') {
                if self.input[self.pos..].starts_with("<!--") {
                    self.skip_comment();
                    continue;
                }
                if self.input[self.pos..].starts_with("</") {
                    if end_tag.is_none() {
                        self.skip_to('>');
                    }
                    break;
                }
                if let Some(element) = self.parse_element() {
                    nodes.push(element);
                }
            } else {
                let text = self.parse_text();
                if !text.is_empty() {
                    nodes.push(ParsedNode::Text(text));
                }
            }
        }
        nodes
    }

    fn parse_element(&mut self) -> Option<ParsedNode> {
        self.advance(); // consume '<'
        let tag = self.parse_tag_name()?.to_ascii_lowercase();
        if matches!(
            tag.as_str(),
            "script" | "style" | "meta" | "link" | "head" | "title"
        ) {
            self.skip_until_close_tag(&tag);
            return None;
        }
        let attributes = self.parse_attributes();
        self.skip_whitespace();
        let self_closing = self.peek() == Some('/');
        if self_closing {
            self.advance();
        }
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
                self.advance();
                continue;
            }
            self.skip_whitespace();
            if self.peek() == Some('=') {
                self.advance();
                self.skip_whitespace();
                let value = if self.peek() == Some('"') {
                    self.advance();
                    let v = self.consume_until('"');
                    self.advance();
                    decode_entities(&v)
                } else if self.peek() == Some('\'') {
                    self.advance();
                    let v = self.consume_until('\'');
                    self.advance();
                    decode_entities(&v)
                } else {
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
        decode_entities(&self.input[start..self.pos])
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
        // Compare on bytes: `self.pos` is a char boundary here (parse_nodes only
        // reaches this with `pos` at a `<`), but `tag.len()` may land mid-char if
        // the close tag's name is multibyte, so never str-slice by `tag.len()`.
        let remaining = &self.input.as_bytes()[self.pos..];
        if !remaining.starts_with(b"</") {
            return false;
        }
        let after = &remaining[2..];
        if after.len() < tag.len() {
            return false;
        }
        after[..tag.len()].eq_ignore_ascii_case(tag.as_bytes())
            && after
                .get(tag.len())
                .is_none_or(|&b| b == b'>' || b.is_ascii_whitespace())
    }

    fn consume_close_tag(&mut self) {
        self.skip_to('>');
    }

    fn skip_comment(&mut self) {
        self.pos += 4; // skip "<!--"
        // Scan on bytes: `pos` is advanced one byte at a time, so it may land
        // inside a multibyte char — comparing `&str` slices there would panic.
        let bytes = self.input.as_bytes();
        while self.pos < bytes.len() {
            if bytes[self.pos..].starts_with(b"-->") {
                self.pos += 3;
                return;
            }
            self.pos += 1;
        }
        self.pos = self.input.len();
    }

    fn skip_until_close_tag(&mut self, tag: &str) {
        let close = format!("</{tag}");
        let close_bytes = close.as_bytes();
        // Byte-level, case-insensitive scan — `pos` advances byte-by-byte and may
        // sit mid-char in non-ASCII element bodies (e.g. `<style>café</style>`).
        let bytes = self.input.as_bytes();
        while self.pos < bytes.len() {
            if bytes[self.pos..].len() >= close_bytes.len()
                && bytes[self.pos..self.pos + close_bytes.len()].eq_ignore_ascii_case(close_bytes)
            {
                self.skip_to('>');
                return;
            }
            self.pos += 1;
        }
    }
}

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
            )
        })
        .collect()
}

fn decode_entities(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&nbsp;", "\u{00A0}")
        .replace("&amp;", "&")
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
        let html = "<table><tr><th colspan=\"2\"><p>h</p></th></tr>\
                    <tr><td rowspan=\"2\"><p>a</p></td><td><p>b</p></td></tr></table>";
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
        assert_eq!(node_to_html(&back), r#"<p style="text-align:center">hi</p>"#);
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
}
