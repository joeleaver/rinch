//! Markdown I/O via `pulldown-cmark` — feature `markdown`.
//!
//! Salvaged from the old `rinch-editor` `document/fragment.rs`, but rebound to the
//! real [`Node`] model and producing **properly nested** structure (a markdown
//! bullet list becomes one `bullet_list` node containing `list_item`s, each with a
//! `paragraph` — not the old flat "one block per item").
//!
//! The dialect is CommonMark plus GFM strikethrough and pipe tables, with a small,
//! exact set of inline HTML for the marks Markdown has no syntax for:
//!
//! | mark | written and read as |
//! | --- | --- |
//! | `underline` | `<u>…</u>` |
//! | `highlight` | `<mark>…</mark>`, or `<mark style="background-color:C">…</mark>` |
//! | `text_color` | `<span style="color:C">…</span>` |
//! | `subscript` | `<sub>…</sub>` |
//! | `superscript` | `<sup>…</sup>` |
//!
//! `C` is a colour the HTML paste path accepts (`#rgb`, `#rrggbb`, `rgb()`, a
//! named colour, …); a mark whose colour is not one is written without it
//! (`highlight`) or not at all (`text_color`), so no attribute value reaches the
//! output unchecked.
//!
//! Bold, italic and strike are `**`, `*` and `~~`, except where CommonMark would
//! not read the delimiter as one (it does not flank: `x**[a](u)**s`,
//! `*~~a~~*b`, `**a.**b`); there the writer uses `<strong>`, `<em>` and `<s>`
//! for that run. The reader takes those tags, and `<b>`, `<i>` and `<del>`, as
//! the same marks. Whitespace at the edge of a bold, italic or strike run is
//! written outside it, so the text round-trips and that whitespace leaves the
//! mark.
//!
//! A hard break is `\` and a newline, except where that does not read back —
//! at the end of a textblock and anywhere in a heading — where it is `<br>`;
//! the reader takes `<br>` and `<br/>` anywhere.
//!
//! A table with a header row, no merged cells and one inline
//! paragraph per cell is a GFM pipe table (column alignment is the cells'
//! `text_align`); any other table is an HTML `<table>` block (`colspan`/`rowspan`,
//! block content in cells), and both read back.
//!
//! A `task_list` is GFM's `- [ ] …` / `- [x] …` (`[X]` is read too); an item
//! that starts with a block other than a paragraph has the marker alone on its
//! line and the block on the next. In a table written as HTML it is
//! `<ul data-type="taskList">` > `<li data-type="taskItem" data-checked="…">`.
//! A bullet or task list right after another is written with the other bullet
//! (`*` / `-`), so CommonMark does not join them. A marker on an item of an
//! ordered list, or of a list where some items have none, has no place in the
//! model.
//!
//! [`doc_from_markdown`] is lenient: what it cannot represent (other raw HTML,
//! unsafe URLs) it drops. [`doc_from_markdown_strict`] parses the same way but
//! fails with a [`MarkdownError`] naming the first construct it would drop and its
//! line. The durable, total format is still [`super::doc_json`].

use crate::EditorError;
use crate::model::{AttrValue, Attrs, Fragment, Mark, Node};
use crate::schema::Schema;
use crate::serialize::html::{
    DroppedAttr, dropped_table_attr, is_safe_css_color, is_safe_url, node_to_html, slice_from_html,
};
use pulldown_cmark::{
    Alignment, CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd,
};
use std::fmt;
use std::ops::Range;

// ─── errors ──────────────────────────────────────────────────────────────────

/// A Markdown construct [`doc_from_markdown`] would drop or degrade, and which
/// [`doc_from_markdown_strict`] refuses.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Construct {
    /// Inline HTML other than the mark tags in the module docs.
    InlineHtml,
    /// A block of raw HTML other than a `<table>`, or a `<table>` holding
    /// something a table cell cannot: another tag, stray text, or an attribute
    /// the document does not keep (`style` other than a safe colour or
    /// `text-align`, `colspan` above 1000, `class`, an event handler, …).
    HtmlBlock,
    /// A mark tag (`<u>`, `<span style>`, …) opened and not closed in its block,
    /// or closed without being opened.
    UnmatchedTag,
    /// A footnote reference or definition.
    Footnote,
    /// A task-list item marker (`- [ ]`, `- [x]`) the document cannot keep:
    /// on an item of an ordered list, or of a list where some items have none.
    TaskList,
    /// A link whose URL is not allowed (`javascript:`, `data:`, …), in Markdown
    /// or in an HTML table.
    UnsafeLink,
    /// An image whose URL is not allowed, in Markdown or in an HTML table.
    UnsafeImage,
    /// Markdown syntax for a mark the schema does not have.
    UnsupportedMark,
    /// Anything else the parser produced that has no place in the document.
    Other,
}

impl Construct {
    /// A short human name ("task list", "inline HTML").
    pub fn name(self) -> &'static str {
        match self {
            Construct::InlineHtml => "inline HTML",
            Construct::HtmlBlock => "HTML block",
            Construct::UnmatchedTag => "unmatched HTML tag",
            Construct::Footnote => "footnote",
            Construct::TaskList => "task list",
            Construct::UnsafeLink => "unsafe link",
            Construct::UnsafeImage => "unsafe image",
            Construct::UnsupportedMark => "unsupported formatting",
            Construct::Other => "unsupported Markdown",
        }
    }
}

impl fmt::Display for Construct {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Why [`doc_from_markdown_strict`] refused its input.
#[derive(Debug)]
pub enum MarkdownError {
    /// The input holds a construct the document would drop or degrade.
    Unsupported {
        /// What it is.
        construct: Construct,
        /// Its 1-based line in the input.
        line: usize,
        /// The source text of the construct (at most 80 characters).
        source: String,
    },
    /// The document could not be built (schema validation).
    Invalid(EditorError),
}

impl fmt::Display for MarkdownError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MarkdownError::Unsupported {
                construct,
                line,
                source,
            } => write!(f, "line {line}: {construct} is not supported: {source}"),
            MarkdownError::Invalid(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for MarkdownError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            MarkdownError::Invalid(e) => Some(e),
            MarkdownError::Unsupported { .. } => None,
        }
    }
}

impl From<EditorError> for MarkdownError {
    fn from(e: EditorError) -> Self {
        MarkdownError::Invalid(e)
    }
}

// ─── markdown → doc ──────────────────────────────────────────────────────────

/// Parse a markdown string into a `doc` node, validated against `schema`.
///
/// Lenient: raw HTML other than the mark tags and tables, unsafe link and image
/// URLs, and anything else without a place in the document are dropped. Use
/// [`doc_from_markdown_strict`] to be told instead.
pub fn doc_from_markdown(schema: &Schema, md: &str) -> Result<Node, EditorError> {
    match parse(schema, md, false) {
        Ok(doc) => Ok(doc),
        Err(MarkdownError::Invalid(e)) => Err(e),
        // Lenient mode never refuses.
        Err(e @ MarkdownError::Unsupported { .. }) => {
            Err(EditorError::SchemaValidation(e.to_string()))
        }
    }
}

/// Parse a markdown string into a `doc` node, failing on the first construct
/// [`doc_from_markdown`] would drop or degrade (raw HTML other than the mark tags
/// and tables, footnotes, a task-list marker in an ordered or mixed list, unsafe
/// URLs, unmatched mark tags), with its 1-based line. What it accepts it parses exactly as [`doc_from_markdown`] does.
pub fn doc_from_markdown_strict(schema: &Schema, md: &str) -> Result<Node, MarkdownError> {
    parse(schema, md, true)
}

fn parse(schema: &Schema, md: &str, strict: bool) -> Result<Node, MarkdownError> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);
    if strict {
        // Recognised only so they can be refused; the lenient parse keeps their
        // text as text, as it always has.
        options.insert(Options::ENABLE_FOOTNOTES);
    }

    let mut builder = MdBuilder::new(schema, md, strict);
    for (event, range) in Parser::new_ext(md, options).into_offset_iter() {
        builder.range = range;
        builder.handle(event)?;
    }
    builder.finish()
}

/// A container being assembled (doc, blockquote, list, list_item, table parts).
struct Container {
    type_name: &'static str,
    attrs: Attrs,
    children: Vec<Node>,
    /// Where it starts in the source.
    start: usize,
    /// A list item's task marker: found in the source when the item opens
    /// ([`task_marker_at`]), and confirmed by the item's first event.
    task: Option<TaskMarker>,
    /// Whether a list item's first event has been seen.
    task_settled: bool,
    /// A list's items' task markers, one per child.
    item_tasks: Vec<Option<TaskMarker>>,
}

/// A task-list marker (`[ ]`, `[x]`, `[X]`) at the start of a list item.
#[derive(Clone)]
struct TaskMarker {
    checked: bool,
    /// The marker's source range.
    range: Range<usize>,
    /// Nothing follows it on its line (`- [ ]`): pulldown-cmark reads that as
    /// the text `[ ]`, and the reader takes it as an empty item's marker when
    /// the item's first paragraph is exactly it.
    bare: bool,
}

/// A textblock being assembled (paragraph, heading, code_block).
struct TextBlock {
    type_name: &'static str,
    attrs: Attrs,
    content: Vec<Node>,
    /// The last text node, not yet in `content`, and its text so far: text
    /// pulldown splits at escapes and entities is appended here, so merging
    /// the pieces is linear in the text, not quadratic.
    pending: Option<(Node, String)>,
    is_code: bool,
}

impl TextBlock {
    /// Move the pending text node into `content`.
    fn flush_pending(&mut self) {
        if let Some((node, text)) = self.pending.take() {
            let node = if node.text() == Some(text.as_str()) {
                node
            } else {
                node.with_text(text.into())
            };
            self.content.push(node);
        }
    }
}

/// An open inline-HTML mark tag.
struct HtmlMark {
    tag: &'static str,
    mark: Mark,
    range: Range<usize>,
}

struct MdBuilder<'a> {
    schema: &'a Schema,
    source: &'a str,
    strict: bool,
    /// Byte offsets where each line starts.
    line_starts: Vec<usize>,
    /// Source range of the event being handled.
    range: Range<usize>,
    /// Container stack; `containers[0]` is always the `doc`.
    containers: Vec<Container>,
    /// The open textblock, if any.
    inline: Option<TextBlock>,
    /// Active inline marks.
    marks: Vec<Mark>,
    /// Mark tags opened by inline HTML, outermost first.
    html_marks: Vec<HtmlMark>,
    /// In-progress image: `(src, title, alt-accumulator)`.
    image: Option<(String, String, String)>,
    /// Column alignments of the open tables, innermost last.
    table_aligns: Vec<Vec<Alignment>>,
    /// Inside a table's header row.
    in_table_head: bool,
    /// Column of the next cell in the open row.
    cell_index: usize,
    /// An HTML block being collected, with where it started.
    html_block: Option<(String, Range<usize>)>,
}

impl<'a> MdBuilder<'a> {
    fn new(schema: &'a Schema, source: &'a str, strict: bool) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(source.match_indices('\n').map(|(i, _)| i + 1));
        Self {
            schema,
            source,
            strict,
            line_starts,
            range: 0..0,
            containers: vec![Container::new("doc", Attrs::new(), 0)],
            inline: None,
            marks: Vec::new(),
            html_marks: Vec::new(),
            image: None,
            table_aligns: Vec::new(),
            in_table_head: false,
            cell_index: 0,
            html_block: None,
        }
    }

    /// Note that `construct` (at `range`) is being dropped: an error when strict.
    fn refuse_at(&self, construct: Construct, range: Range<usize>) -> Result<(), MarkdownError> {
        if !self.strict {
            return Ok(());
        }
        let line = self.line_starts.partition_point(|&s| s <= range.start);
        let raw = self.source.get(range).unwrap_or("").trim();
        let mut source: String = raw.chars().take(80).collect();
        if source.len() < raw.len() {
            source.push('…');
        }
        Err(MarkdownError::Unsupported {
            construct,
            line,
            source,
        })
    }

    fn refuse(&self, construct: Construct) -> Result<(), MarkdownError> {
        self.refuse_at(construct, self.range.clone())
    }

    fn handle(&mut self, event: Event<'_>) -> Result<(), MarkdownError> {
        self.settle_task_marker(&event);
        match event {
            Event::Start(tag) => self.start(tag)?,
            Event::End(tag_end) => self.end(tag_end)?,
            Event::Text(t) => self.push_text(&t)?,
            Event::Code(t) => self.push_code(&t)?,
            Event::SoftBreak => self.push_text(" ")?,
            Event::HardBreak => self.push_hard_break()?,
            Event::Rule => self.push_rule()?,
            Event::InlineHtml(h) => self.inline_html(&h)?,
            Event::Html(h) => match &mut self.html_block {
                Some((buf, _)) => buf.push_str(&h),
                None => self.refuse(Construct::HtmlBlock)?,
            },
            Event::FootnoteReference(_) => self.refuse(Construct::Footnote)?,
            // An item's marker is found when the item opens (pulldown emits
            // it inside whatever paragraph comes first, or not at all).
            Event::TaskListMarker(checked) => self.claim_task_marker(checked),
            _ => self.refuse(Construct::Other)?, // math (not enabled)
        }
        Ok(())
    }

    fn start(&mut self, tag: Tag<'_>) -> Result<(), MarkdownError> {
        match tag {
            Tag::Heading { level, .. } => {
                self.flush_inline()?;
                let attrs = Attrs::from_iter([("level", AttrValue::Int(heading_level(level)))]);
                self.open_inline("heading", attrs, false);
            }
            Tag::Paragraph => {
                self.flush_inline()?;
                self.open_inline("paragraph", Attrs::new(), false);
            }
            Tag::CodeBlock(kind) => {
                self.flush_inline()?;
                let attrs = match kind {
                    CodeBlockKind::Fenced(lang) if !lang.is_empty() => {
                        Attrs::from_iter([("language", AttrValue::from(lang.to_string()))])
                    }
                    _ => Attrs::new(),
                };
                self.open_inline("code_block", attrs, true);
            }
            Tag::BlockQuote(_) => {
                self.flush_inline()?;
                self.push_container("blockquote", Attrs::new());
            }
            Tag::List(start) => {
                self.flush_inline()?;
                match start {
                    Some(s) => self.push_container(
                        "ordered_list",
                        Attrs::from_iter([("start", AttrValue::Int(s as i64))]),
                    ),
                    None => self.push_container("bullet_list", Attrs::new()),
                }
            }
            Tag::Item => {
                self.flush_inline()?;
                self.push_container("list_item", Attrs::new());
                let start = self.range.start;
                let task = task_marker_at(self.source, start);
                let item = self.top_mut();
                item.task = task;
            }
            Tag::Table(aligns) => {
                self.flush_inline()?;
                self.table_aligns.push(aligns);
                self.push_container("table", Attrs::new());
            }
            Tag::TableHead => {
                self.in_table_head = true;
                self.cell_index = 0;
                self.push_container("table_row", Attrs::new());
            }
            Tag::TableRow => {
                self.cell_index = 0;
                self.push_container("table_row", Attrs::new());
            }
            Tag::TableCell => {
                let cell = if self.in_table_head {
                    "table_header_cell"
                } else {
                    "table_cell"
                };
                self.push_container(cell, Attrs::new());
                // GFM cells hold inline content directly; the schema's cells hold
                // blocks, so the content goes into one paragraph.
                let align = self
                    .table_aligns
                    .last()
                    .and_then(|a| a.get(self.cell_index))
                    .copied()
                    .unwrap_or(Alignment::None);
                let mut attrs = Attrs::new();
                let align = match align {
                    Alignment::Center => Some("center"),
                    Alignment::Right => Some("right"),
                    Alignment::Left | Alignment::None => None,
                };
                if let Some(a) = align
                    && self
                        .schema
                        .node_type("paragraph")
                        .is_some_and(|nt| nt.spec().attrs.contains_key("text_align"))
                {
                    attrs = Attrs::from_iter([("text_align", AttrValue::from(a.to_string()))]);
                }
                self.open_inline("paragraph", attrs, false);
            }
            Tag::HtmlBlock => {
                self.flush_inline()?;
                self.html_block = Some((String::new(), self.range.clone()));
            }
            Tag::Strong => self.add_md_mark("bold", Attrs::new())?,
            Tag::Emphasis => self.add_md_mark("italic", Attrs::new())?,
            Tag::Strikethrough => self.add_md_mark("strike", Attrs::new())?,
            Tag::Link {
                link_type,
                dest_url,
                title,
                ..
            } => {
                // Mirror the HTML ingress whitelist: a javascript:/vbscript:/data:
                // href never enters the model. On rejection the link mark is simply
                // not pushed (children parse transparently); the matching End(Link)
                // `remove_mark("link")` is then a harmless no-op.
                if self.image.is_some() {
                    // A link inside an image's alt text: only the text survives.
                } else if is_safe_url(&dest_url, false) {
                    let href = if link_type == LinkType::Email {
                        format!("mailto:{dest_url}")
                    } else {
                        dest_url.to_string()
                    };
                    let mut pairs: Vec<(&str, AttrValue)> = vec![("href", AttrValue::from(href))];
                    if !title.is_empty() {
                        pairs.push(("title", AttrValue::from(title.to_string())));
                    }
                    self.add_md_mark("link", Attrs::from_iter(pairs))?;
                } else {
                    self.refuse(Construct::UnsafeLink)?;
                }
            }
            Tag::Image {
                dest_url, title, ..
            } => {
                if self.image.is_none() {
                    self.image = Some((dest_url.to_string(), title.to_string(), String::new()));
                }
            }
            Tag::FootnoteDefinition(_) => self.refuse(Construct::Footnote)?,
            _ => self.refuse(Construct::Other)?,
        }
        Ok(())
    }

    fn end(&mut self, tag_end: TagEnd) -> Result<(), MarkdownError> {
        match tag_end {
            TagEnd::Heading(_) | TagEnd::Paragraph | TagEnd::CodeBlock => self.flush_inline()?,
            TagEnd::List(_) => {
                self.flush_inline()?;
                self.pop_list()?;
            }
            TagEnd::BlockQuote(_) | TagEnd::Item => {
                self.flush_inline()?;
                self.pop_container()?;
            }
            TagEnd::TableCell => {
                self.flush_inline()?;
                self.pop_container()?;
                self.cell_index += 1;
            }
            TagEnd::TableHead => {
                self.pop_container()?;
                self.in_table_head = false;
            }
            TagEnd::TableRow => self.pop_container()?,
            TagEnd::Table => {
                self.pop_container()?;
                self.table_aligns.pop();
            }
            TagEnd::HtmlBlock => {
                if let Some((html, range)) = self.html_block.take() {
                    if let Some(breaks) = br_only_block(&html) {
                        // `<br>` alone on a line is CommonMark's HTML block; the
                        // writer writes a textblock of only hard breaks that way.
                        self.open_inline("paragraph", Attrs::new(), false);
                        for _ in 0..breaks {
                            self.push_hard_break()?;
                        }
                        self.flush_inline()?;
                        return Ok(());
                    }
                    match self.html_tables(&html, &range)? {
                        Some(tables) => self.top_mut().children.extend(tables),
                        None => self.refuse_at(Construct::HtmlBlock, range)?,
                    }
                }
            }
            TagEnd::Image => {
                if let Some((src, title, alt)) = self.image.take() {
                    // Unsafe src → drop the image entirely. The alt text was captured
                    // into the image buffer (not leaked into the paragraph) so nothing
                    // dangerous and nothing stray remains.
                    if is_safe_url(&src, true) {
                        self.push_image(&src, &title, &alt)?;
                    } else {
                        self.refuse(Construct::UnsafeImage)?;
                    }
                }
            }
            TagEnd::Strong => self.remove_mark("bold"),
            TagEnd::Emphasis => self.remove_mark("italic"),
            TagEnd::Strikethrough => self.remove_mark("strike"),
            // A link inside an image's alt text never opened.
            TagEnd::Link if self.image.is_none() => self.remove_mark("link"),
            _ => {}
        }
        Ok(())
    }

    // ── inline HTML marks ──

    fn inline_html(&mut self, html: &str) -> Result<(), MarkdownError> {
        if is_br_tag(html) {
            return self.push_hard_break();
        }
        let Some(tag) = parse_inline_tag(html) else {
            return self.refuse(Construct::InlineHtml);
        };
        if self.image.is_some() {
            // Alt text is plain: a mark tag inside it has nothing to mark.
            return Ok(());
        }
        match tag {
            InlineTag::Open { tag, mark, color } => {
                let Some(mt) = self.schema.mark_type(mark) else {
                    return self.refuse(Construct::UnsupportedMark);
                };
                let attrs = match color {
                    Some(c) => Attrs::from_iter([("color", AttrValue::from(c))]),
                    None => Attrs::new(),
                };
                let mark = Mark::new(mt.clone(), mt.compute_attrs(&attrs)?);
                self.html_marks.push(HtmlMark {
                    tag,
                    mark,
                    range: self.range.clone(),
                });
                self.recompute_html_marks(None);
            }
            InlineTag::Close { tag } => match self.html_marks.iter().rposition(|m| m.tag == tag) {
                Some(i) => {
                    let closed = self.html_marks.remove(i);
                    self.recompute_html_marks(Some(closed.mark.type_name().to_string()));
                }
                None => self.refuse(Construct::UnmatchedTag)?,
            },
        }
        Ok(())
    }

    /// Re-derive the active marks after an HTML mark tag opened or closed:
    /// drop the closed type, then re-add the open tags outermost first, so the
    /// innermost of a type (or of a pair that excludes each other) wins.
    fn recompute_html_marks(&mut self, closed_type: Option<String>) {
        if let Some(t) = closed_type {
            self.marks.retain(|m| m.type_name() != t);
            // A type the closed tag excluded (`<sub>` inside `<sup>`) comes back
            // through the re-add below.
        }
        for m in &self.html_marks {
            self.marks = m.mark.add_to_set(&self.marks);
        }
    }

    /// Close any mark tags still open at the end of a textblock.
    fn close_html_marks(&mut self) -> Result<(), MarkdownError> {
        if let Some(first) = self.html_marks.first() {
            self.refuse_at(Construct::UnmatchedTag, first.range.clone())?;
        }
        for m in std::mem::take(&mut self.html_marks) {
            self.marks.retain(|x| x != &m.mark);
        }
        Ok(())
    }

    // ── HTML tables ──

    /// The tables an HTML block holds, or `None` if it is not (only) tables.
    /// Strict, it refuses (at `range`) an attribute the tables would lose: an
    /// unsafe URL as such, anything else as an HTML block.
    fn html_tables(
        &self,
        html: &str,
        range: &Range<usize>,
    ) -> Result<Option<Vec<Node>>, MarkdownError> {
        let trimmed = html.trim();
        let lower = trimmed.to_ascii_lowercase();
        if !lower.starts_with("<table") || !lower.ends_with("</table>") {
            return Ok(None);
        }
        if self.strict && !html_tags_allowed(trimmed) {
            return Ok(None);
        }
        // The writer encodes newlines in cell text (code blocks) as `&#10;` so a
        // blank line never ends the HTML block early.
        let decoded = trimmed.replace("&#10;", "\n");
        if self.strict
            && let Some(dropped) = dropped_table_attr(&decoded)
        {
            let construct = match dropped {
                DroppedAttr::UnsafeLink => Construct::UnsafeLink,
                DroppedAttr::UnsafeImage => Construct::UnsafeImage,
                DroppedAttr::Other => Construct::HtmlBlock,
            };
            self.refuse_at(construct, range.clone())?;
        }
        let slice = slice_from_html(self.schema, &decoded)?;
        let tables: Vec<Node> = slice
            .content
            .children()
            .iter()
            .map(drop_unit_spans)
            .collect();
        if tables.is_empty() || tables.iter().any(|t| t.type_name() != "table") {
            return Ok(None);
        }
        if self.strict {
            // Every character of text in the HTML must have arrived.
            let want: String = html_text(&decoded)
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            let mut got = String::new();
            for t in &tables {
                collect_text(t, &mut got);
            }
            let got: String = got.chars().filter(|c| !c.is_whitespace()).collect();
            if want != got {
                return Ok(None);
            }
        }
        Ok(Some(tables))
    }

    // ── building ──

    fn open_inline(&mut self, type_name: &'static str, attrs: Attrs, is_code: bool) {
        self.inline = Some(TextBlock {
            type_name,
            attrs,
            content: Vec::new(),
            pending: None,
            is_code,
        });
    }

    fn ensure_inline(&mut self) {
        if self.inline.is_none() {
            self.open_inline("paragraph", Attrs::new(), false);
        }
    }

    fn flush_inline(&mut self) -> Result<(), MarkdownError> {
        self.close_html_marks()?;
        let Some(mut tb) = self.inline.take() else {
            return Ok(());
        };
        tb.flush_pending();
        let content = if tb.is_code {
            let mut text = String::new();
            for n in &tb.content {
                text.push_str(n.text().unwrap_or(""));
            }
            // pulldown appends a trailing newline to code blocks.
            if text.ends_with('\n') {
                text.pop();
            }
            if text.is_empty() {
                Fragment::empty()
            } else {
                Fragment::from_node(self.schema.text(&text)?)
            }
        } else {
            Fragment::from_children(tb.content)
        };
        let node = self.make_node(tb.type_name, tb.attrs, content)?;
        self.top_mut().children.push(node);
        Ok(())
    }

    fn push_container(&mut self, type_name: &'static str, attrs: Attrs) {
        self.containers
            .push(Container::new(type_name, attrs, self.range.start));
    }

    fn pop_container(&mut self) -> Result<(), MarkdownError> {
        // Never pop the root doc container.
        if self.containers.len() <= 1 {
            return Ok(());
        }
        let mut c = self.containers.pop().expect("non-root container");
        let task = if c.type_name == "list_item" {
            match c.task.clone() {
                // `- [ ]` with nothing after it on its line: a marker when it
                // is the whole first paragraph, which is then not content.
                Some(t) if t.bare => {
                    let text = self.source.get(t.range.clone());
                    let is_marker = c.children.first().is_some_and(|b| {
                        b.type_name() == "paragraph"
                            && b.child_count() == 1
                            && b.child(0).marks().is_empty()
                            && b.child(0).text().is_some()
                            && b.child(0).text() == text
                    });
                    if is_marker {
                        c.children.remove(0);
                        Some(t)
                    } else {
                        None
                    }
                }
                task => task,
            }
        } else {
            None
        };
        let is_item = c.type_name == "list_item";
        let node = self.build_container(c)?;
        let parent = self.top_mut();
        parent.children.push(node);
        if is_item {
            parent.item_tasks.push(task);
        }
        Ok(())
    }

    /// Close a list. A bullet list whose items all start with a task marker
    /// is a `task_list`; a marker anywhere else is refused when strict, and
    /// kept as the text it was when lenient.
    fn pop_list(&mut self) -> Result<(), MarkdownError> {
        let is_list = self
            .containers
            .last()
            .is_some_and(|c| matches!(c.type_name, "bullet_list" | "ordered_list"));
        if !is_list || self.containers.len() <= 1 {
            return self.pop_container();
        }
        let mut c = self.containers.pop().expect("a list");
        let tasks = std::mem::take(&mut c.item_tasks);
        let all = c.type_name == "bullet_list"
            && !c.children.is_empty()
            && tasks.len() == c.children.len()
            && tasks.iter().all(Option::is_some)
            && self.schema.node_type("task_list").is_some()
            && self.schema.node_type("task_item").is_some();
        let node = if all {
            let mut items = Vec::with_capacity(c.children.len());
            for (item, task) in c.children.iter().zip(&tasks) {
                let checked = task.as_ref().is_some_and(|t| t.checked);
                let attrs = Attrs::from_iter([("checked", AttrValue::Bool(checked))]);
                items.push(self.make_node("task_item", attrs, item.content().clone())?);
            }
            self.make_node("task_list", Attrs::new(), Fragment::from_children(items))?
        } else {
            for (item, task) in c.children.iter_mut().zip(&tasks) {
                if let Some(task) = task {
                    self.refuse_at(Construct::TaskList, task.range.clone())?;
                    let text = self.source.get(task.range.clone()).unwrap_or("[ ]");
                    *item = self.with_marker_text(item, text, task.bare)?;
                }
            }
            self.build_container(c)?
        };
        self.top_mut().children.push(node);
        Ok(())
    }

    /// `item` with a task marker's `text` put back at the start of its first
    /// paragraph (a new paragraph, if it starts with another block), as the
    /// parse without task lists read it.
    fn with_marker_text(&self, item: &Node, text: &str, bare: bool) -> Result<Node, EditorError> {
        let mut blocks: Vec<Node> = item.content().children().to_vec();
        // A bare marker was a paragraph of its own.
        let first_para = blocks
            .first()
            .filter(|b| b.type_name() == "paragraph" && !(bare && b.child_count() > 0));
        let para = match first_para {
            Some(para) => {
                let mut inline: Vec<Node> = para.content().children().to_vec();
                match inline.first() {
                    Some(t) if t.is_text() && t.marks().is_empty() => {
                        let joined = format!("{text} {}", t.text().unwrap_or(""));
                        inline[0] = self.schema.text(&joined)?;
                    }
                    Some(_) => inline.insert(0, self.schema.text(&format!("{text} "))?),
                    None => inline.push(self.schema.text(text)?),
                }
                let para = Node::new_branch(
                    para.node_type().clone(),
                    para.attrs().clone(),
                    Fragment::from_children(inline),
                );
                blocks.remove(0);
                para
            }
            None => self.make_node(
                "paragraph",
                Attrs::new(),
                Fragment::from_node(self.schema.text(text)?),
            )?,
        };
        blocks.insert(0, para);
        Ok(Node::new_branch(
            item.node_type().clone(),
            item.attrs().clone(),
            Fragment::from_children(blocks),
        ))
    }

    /// The first event inside a list item decides whether the marker found
    /// in the source when it opened is one: pulldown emits a
    /// `TaskListMarker` for it, or (when another block follows it) consumes
    /// it silently, so that the first event starts after it. Text starting
    /// at the marker (`- [ ]` with nothing after it) means it is not one.
    fn settle_task_marker(&mut self, event: &Event<'_>) {
        let start = self.range.start;
        let Some(item) = self.containers.last_mut() else {
            return;
        };
        if item.type_name != "list_item" || item.task_settled {
            return;
        }
        item.task_settled = true;
        let Some(task) = &item.task else {
            return;
        };
        let confirmed = match event {
            // An item's first event can only be its own marker (pulldown
            // reports it from the whitespace before `[`, when a tab is there).
            Event::TaskListMarker(_) => true,
            // Decided when the item closes, by its first paragraph.
            _ if task.bare => true,
            _ => start >= task.range.end,
        };
        if !confirmed {
            item.task = None;
        }
    }

    /// A `TaskListMarker` event belongs to the innermost open item it falls
    /// in. That item has normally found it in its source already
    /// ([`task_marker_at`]); a marker the scan does not know is taken from the
    /// event, so the text pulldown consumed for it is never lost.
    fn claim_task_marker(&mut self, checked: bool) {
        let range = self.range.clone();
        let owner = self
            .containers
            .iter_mut()
            .rev()
            .find(|c| c.type_name == "list_item" && c.start <= range.start);
        if let Some(item) = owner
            && item.task.as_ref().is_none_or(|t| t.bare)
        {
            item.task = Some(TaskMarker {
                checked,
                range,
                bare: false,
            });
        }
    }

    /// Append an inline node, merging it into the previous text node when both
    /// carry the same marks (pulldown splits text at escapes and entities).
    fn push_inline_node(&mut self, node: Node) {
        self.ensure_inline();
        let tb = self.inline.as_mut().expect("inline open");
        if node.is_text() {
            if let Some((last, text)) = &mut tb.pending
                && last.same_markup(&node)
            {
                text.push_str(node.text().unwrap_or(""));
                return;
            }
            tb.flush_pending();
            let text = node.text().unwrap_or("").to_string();
            tb.pending = Some((node, text));
            return;
        }
        tb.flush_pending();
        tb.content.push(node);
    }

    fn push_text(&mut self, t: &str) -> Result<(), MarkdownError> {
        if self.html_block.is_some() {
            // pulldown emits an empty-range space at the start of an HTML
            // block that follows a task marker (`- [ ] <br>`); the block's
            // own content arrives as `Html`.
            return Ok(());
        }
        if let Some((_, _, alt)) = &mut self.image {
            alt.push_str(t);
            return Ok(());
        }
        if t.is_empty() {
            return Ok(());
        }
        let node = self.schema.text_with_marks(t, self.marks.clone())?;
        self.push_inline_node(node);
        Ok(())
    }

    fn push_code(&mut self, t: &str) -> Result<(), MarkdownError> {
        if let Some((_, _, alt)) = &mut self.image {
            alt.push_str(t);
            return Ok(());
        }
        if t.is_empty() {
            return Ok(());
        }
        let marks = match self.schema.mark_type("code") {
            Some(mt) => {
                Mark::new(mt.clone(), mt.compute_attrs(&Attrs::new())?).add_to_set(&self.marks)
            }
            None => {
                self.refuse(Construct::UnsupportedMark)?;
                self.marks.clone()
            }
        };
        let node = self.schema.text_with_marks(t, marks)?;
        self.push_inline_node(node);
        Ok(())
    }

    fn push_hard_break(&mut self) -> Result<(), MarkdownError> {
        if self.image.is_some() {
            return Ok(());
        }
        let node = self.make_node("hard_break", Attrs::new(), Fragment::empty())?;
        self.push_inline_node(node);
        Ok(())
    }

    fn push_rule(&mut self) -> Result<(), MarkdownError> {
        self.flush_inline()?;
        let hr = self.make_node("horizontal_rule", Attrs::new(), Fragment::empty())?;
        self.top_mut().children.push(hr);
        Ok(())
    }

    fn push_image(&mut self, src: &str, title: &str, alt: &str) -> Result<(), MarkdownError> {
        let mut pairs: Vec<(&str, AttrValue)> = vec![("src", AttrValue::from(src.to_string()))];
        if !alt.is_empty() {
            pairs.push(("alt", AttrValue::from(alt.to_string())));
        }
        if !title.is_empty() {
            pairs.push(("title", AttrValue::from(title.to_string())));
        }
        let img = self.make_node("image", Attrs::from_iter(pairs), Fragment::empty())?;
        let img = if self.marks.is_empty() {
            img
        } else {
            img.with_marks(self.marks.clone())
        };
        self.push_inline_node(img);
        Ok(())
    }

    /// Add a mark Markdown syntax asked for; strict refuses one the schema lacks.
    fn add_md_mark(&mut self, name: &str, attrs: Attrs) -> Result<(), MarkdownError> {
        if let Some(mt) = self.schema.mark_type(name) {
            let mark = Mark::new(mt.clone(), mt.compute_attrs(&attrs)?);
            self.marks = mark.add_to_set(&self.marks);
            Ok(())
        } else {
            self.refuse(Construct::UnsupportedMark)
        }
    }

    fn remove_mark(&mut self, name: &str) {
        self.marks.retain(|m| m.type_name() != name);
    }

    fn top_mut(&mut self) -> &mut Container {
        self.containers.last_mut().expect("doc container")
    }

    fn build_container(&self, c: Container) -> Result<Node, EditorError> {
        match c.type_name {
            "blockquote" | "list_item" | "table_cell" | "table_header_cell" => {
                let children = self.ensure_block_plus(c.children)?;
                self.make_node(c.type_name, c.attrs, Fragment::from_children(children))
            }
            "bullet_list" | "ordered_list" => {
                let items = if c.children.is_empty() {
                    vec![self.empty_list_item()?]
                } else {
                    c.children
                };
                self.make_node(c.type_name, c.attrs, Fragment::from_children(items))
            }
            _ => self.make_node(c.type_name, c.attrs, Fragment::from_children(c.children)),
        }
    }

    fn ensure_block_plus(&self, mut blocks: Vec<Node>) -> Result<Vec<Node>, EditorError> {
        if blocks.is_empty() {
            blocks.push(self.make_node("paragraph", Attrs::new(), Fragment::empty())?);
        }
        Ok(blocks)
    }

    fn empty_list_item(&self) -> Result<Node, EditorError> {
        let para = self.make_node("paragraph", Attrs::new(), Fragment::empty())?;
        self.make_node("list_item", Attrs::new(), Fragment::from_node(para))
    }

    fn make_node(
        &self,
        type_name: &str,
        attrs: Attrs,
        content: Fragment,
    ) -> Result<Node, EditorError> {
        let nt = self
            .schema
            .node_type(type_name)
            .ok_or_else(|| EditorError::UnknownNodeType(type_name.to_string()))?;
        let attrs = nt.compute_attrs(&attrs)?;
        Ok(Node::new_branch(nt.clone(), attrs, content))
    }

    fn finish(mut self) -> Result<Node, MarkdownError> {
        self.flush_inline()?;
        // Close any containers the (malformed) input left open.
        while self.containers.len() > 1 {
            self.pop_container()?;
        }
        let doc = self.containers.pop().expect("doc container");
        let children = self.ensure_block_plus(doc.children)?;
        Ok(self.make_node("doc", Attrs::new(), Fragment::from_children(children))?)
    }
}

impl Container {
    fn new(type_name: &'static str, attrs: Attrs, start: usize) -> Self {
        Self {
            type_name,
            attrs,
            children: Vec::new(),
            start,
            task: None,
            task_settled: false,
            item_tasks: Vec::new(),
        }
    }
}

/// The task marker a list item starting at `start` opens with, if its source
/// does: the bullet (`-`, `*`, `+`, or up to nine digits and `.` or `)`),
/// spaces or tabs, then `[ ]`, `[x]` or `[X]` and a space, tab, VT or FF
/// (pulldown-cmark's rule), or the end of the line (a `bare` marker, which
/// pulldown reads as text).
fn task_marker_at(source: &str, start: usize) -> Option<TaskMarker> {
    let bytes = source.as_bytes();
    let mut i = start;
    let skip_ws = |mut i: usize| {
        while matches!(bytes.get(i), Some(b' ' | b'\t')) {
            i += 1;
        }
        i
    };
    i = skip_ws(i);
    match bytes.get(i)? {
        b'-' | b'*' | b'+' => i += 1,
        b'0'..=b'9' => {
            let digits = bytes[i..].iter().take_while(|b| b.is_ascii_digit()).count();
            if digits > 9 || !matches!(bytes.get(i + digits), Some(b'.' | b')')) {
                return None;
            }
            i += digits + 1;
        }
        _ => return None,
    }
    let after_bullet = skip_ws(i);
    if after_bullet == i {
        return None;
    }
    i = after_bullet;
    let checked = match bytes.get(i..i + 3)? {
        // pulldown takes a tab, VT or FF between the brackets too.
        [b'[', b' ' | b'\t' | 0x0b | 0x0c, b']'] => false,
        b"[x]" | b"[X]" => true,
        _ => return None,
    };
    let range = i..i + 3;
    let bare = match bytes.get(i + 3) {
        Some(b' ' | b'\t' | 0x0b | 0x0c) => false,
        None | Some(b'\n' | b'\r') => true,
        Some(_) => return None,
    };
    Some(TaskMarker {
        checked,
        range,
        bare,
    })
}

fn heading_level(level: HeadingLevel) -> i64 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// One of the exact inline-HTML shapes the mark tags take.
#[derive(Debug, PartialEq)]
enum InlineTag {
    Open {
        tag: &'static str,
        mark: &'static str,
        color: Option<String>,
    },
    Close {
        tag: &'static str,
    },
}

/// The inline HTML tags read as marks: the tag each HTML-only mark is written
/// as, the tags a bold, italic or strike run is written as where its delimiters
/// would not flank, and the other common spellings of those three.
const MARK_TAGS: [(&str, &str); 11] = [
    ("u", "underline"),
    ("mark", "highlight"),
    ("span", "text_color"),
    ("sub", "subscript"),
    ("sup", "superscript"),
    ("strong", "bold"),
    ("b", "bold"),
    ("em", "italic"),
    ("i", "italic"),
    ("s", "strike"),
    ("del", "strike"),
];

/// The number of `<br>` tags an HTML block consists of, if that is all it holds.
fn br_only_block(html: &str) -> Option<usize> {
    let mut rest = html.trim();
    let mut n = 0;
    while !rest.is_empty() {
        let end = rest.find('>')? + 1;
        if !is_br_tag(&rest[..end]) {
            return None;
        }
        n += 1;
        rest = rest[end..].trim_start();
    }
    (n > 0).then_some(n)
}

/// `<br>`, `<br/>` or `<br />`, in any case: a hard break.
fn is_br_tag(html: &str) -> bool {
    html.strip_prefix('<')
        .and_then(|h| h.strip_suffix('>'))
        .map(|h| h.trim_end().trim_end_matches('/').trim_end())
        .is_some_and(|name| name.eq_ignore_ascii_case("br"))
}

/// Parse `<u>`, `</u>`, `<mark>`, `<mark style="background-color:C">`,
/// `<span style="color:C">`, `<sub>`, `<sup>` and their closing tags; anything
/// else is `None`.
fn parse_inline_tag(html: &str) -> Option<InlineTag> {
    let inner = html.strip_prefix('<')?.strip_suffix('>')?;
    if let Some(name) = inner.strip_prefix('/') {
        let name = name.trim_end().to_ascii_lowercase();
        let (tag, _) = MARK_TAGS.iter().find(|(t, _)| *t == name)?;
        return Some(InlineTag::Close { tag });
    }
    let name_end = inner
        .find(|c: char| !c.is_ascii_alphanumeric())
        .unwrap_or(inner.len());
    let name = inner[..name_end].to_ascii_lowercase();
    let &(tag, mark) = MARK_TAGS.iter().find(|(t, _)| *t == name)?;
    let rest = inner[name_end..].trim();
    if rest.is_empty() {
        // `<span>` alone carries nothing.
        return (tag != "span").then_some(InlineTag::Open {
            tag,
            mark,
            color: None,
        });
    }
    let property = match tag {
        "span" => "color",
        "mark" => "background-color",
        _ => return None,
    };
    let value = rest
        .strip_prefix("style")?
        .trim_start()
        .strip_prefix('=')?
        .trim_start();
    let quote = value.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let value = value[1..].strip_suffix(quote)?;
    if value.contains(quote) {
        return None;
    }
    let mut decls = value.split(';').map(str::trim).filter(|d| !d.is_empty());
    let decl = decls.next()?;
    if decls.next().is_some() {
        return None;
    }
    let (k, v) = decl.split_once(':')?;
    let v = v.trim();
    if !k.trim().eq_ignore_ascii_case(property) || !is_safe_css_color(v) {
        return None;
    }
    Some(InlineTag::Open {
        tag,
        mark,
        color: Some(v.to_string()),
    })
}

/// `node` with every `colspan`/`rowspan` of 1 left unset, as the editor and a
/// pipe table leave them (the HTML import writes them out).
fn drop_unit_spans(node: &Node) -> Node {
    if node.is_text() || node.is_leaf() {
        return node.clone();
    }
    let mut attrs = node.attrs().clone();
    for name in ["colspan", "rowspan"] {
        if attrs.get_int(name) == Some(1) {
            attrs = attrs.without(name);
        }
    }
    let children: Vec<Node> = node
        .content()
        .children()
        .iter()
        .map(drop_unit_spans)
        .collect();
    Node::new_branch(
        node.node_type().clone(),
        attrs,
        Fragment::from_children(children),
    )
}

/// Tags an HTML table block may hold in strict mode: what the HTML import turns
/// into table content without dropping anything.
fn html_tags_allowed(html: &str) -> bool {
    const ALLOWED: &[&str] = &[
        "table",
        "thead",
        "tbody",
        "tfoot",
        "tr",
        "td",
        "th",
        "p",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "ul",
        "ol",
        "li",
        "pre",
        "code",
        "strong",
        "b",
        "em",
        "i",
        "s",
        "del",
        "strike",
        "u",
        "mark",
        "span",
        "sub",
        "sup",
        "a",
        "br",
        "img",
        "hr",
        "blockquote",
    ];
    let bytes = html.as_bytes();
    let mut i = 0;
    while let Some(off) = html[i..].find('<') {
        let start = i + off + 1;
        let mut j = start;
        if bytes.get(j) == Some(&b'/') {
            j += 1;
        }
        let name_start = j;
        while j < bytes.len() && bytes[j].is_ascii_alphanumeric() {
            j += 1;
        }
        let name = html[name_start..j].to_ascii_lowercase();
        if name.is_empty() || !ALLOWED.contains(&name.as_str()) {
            return false;
        }
        i = j;
    }
    true
}

/// The text of an HTML fragment: tags removed, the entities the HTML import
/// decodes decoded.
fn html_text(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&nbsp;", "\u{00A0}")
        .replace("&amp;", "&")
}

/// Concatenate all descendant text.
fn collect_text(node: &Node, out: &mut String) {
    if let Some(t) = node.text() {
        out.push_str(t);
    } else {
        for child in node.content().children() {
            collect_text(child, out);
        }
    }
}

// ─── doc → markdown ──────────────────────────────────────────────────────────

/// Serialize a `doc` node to a markdown string. Text is escaped so that it reads
/// back as the same text; the marks Markdown has no syntax for are written as
/// the inline HTML in the module docs.
pub fn doc_to_markdown(doc: &Node) -> String {
    let md = blocks_to_md(doc.content().children());
    trim_md_end(&md)
}

/// `md` without trailing whitespace, except the space after an empty task
/// item's marker (`- [ ] `): without it the marker is the text `[ ]`. Text
/// never ends a line in an unescaped `[ ]` or `[x]` (`[` and `]` are always
/// escaped), so only a marker does.
fn trim_md_end(md: &str) -> String {
    let trimmed = md.trim_end();
    let marker = ["- [ ]", "- [x]", "* [ ]", "* [x]"]
        .iter()
        .any(|m| trimmed.ends_with(m));
    if marker && trimmed.len() < md.len() {
        format!("{trimmed} ")
    } else {
        trimmed.to_string()
    }
}

fn blocks_to_md(blocks: &[Node]) -> String {
    let mut out = String::new();
    // CommonMark continues a list across blank lines when the next item has
    // the same bullet: a bullet or task list right after another is written
    // with the other bullet, so the two stay apart.
    let mut prev_bullet: Option<char> = None;
    for b in blocks {
        let bullet = if matches!(b.type_name(), "bullet_list" | "task_list") {
            Some(if prev_bullet == Some('-') { '*' } else { '-' })
        } else {
            None
        };
        let md = block_to_md(b, bullet.unwrap_or('-'));
        // A block that writes nothing (an empty paragraph) leaves the lists
        // around it adjacent.
        if !md.trim().is_empty() {
            prev_bullet = bullet;
        }
        out.push_str(&md);
    }
    out
}

fn block_to_md(node: &Node, bullet: char) -> String {
    match node.type_name() {
        "heading" => {
            let level = node.attrs().get_int("level").unwrap_or(1).clamp(1, 6) as usize;
            format!(
                "{} {}\n\n",
                "#".repeat(level),
                inline_to_md(node, Ctx::Heading)
            )
        }
        "code_block" => {
            let lang = node.attrs().get_str("language").unwrap_or("");
            let text = block_text(node);
            let fence = "`".repeat(longest_run(&text, '`').max(2) + 1);
            format!("{fence}{lang}\n{text}\n{fence}\n\n")
        }
        "blockquote" => {
            let inner = blocks_to_md(node.content().children());
            format!("{}\n\n", prefix_lines(&trim_md_end(&inner), "> "))
        }
        "bullet_list" => list_to_md(node, ListKind::Bullet(bullet)),
        "task_list" => list_to_md(node, ListKind::Task(bullet)),
        "ordered_list" => list_to_md(
            node,
            ListKind::Ordered(node.attrs().get_int("start").unwrap_or(1)),
        ),
        "horizontal_rule" => "---\n\n".to_string(),
        "table" => table_to_md(node),
        // paragraph and any other textblock-ish node.
        _ => format!("{}\n\n", inline_to_md(node, Ctx::Block)),
    }
}

#[derive(Clone, Copy)]
enum ListKind {
    /// A bullet list, with its bullet character.
    Bullet(char),
    /// A task list (GFM's `- [ ]` / `- [x]`), with its bullet character.
    Task(char),
    /// An ordered list, with its start.
    Ordered(i64),
}

fn list_to_md(list: &Node, kind: ListKind) -> String {
    let mut out = String::new();
    for (i, item) in list.content().children().iter().enumerate() {
        let item_md = blocks_to_md(item.content().children());
        // An empty first paragraph writes nothing, and `- ` before a blank
        // line ends the item: its next block would fall out of the list.
        let item_md = trim_md_end(item_md.trim_start_matches('\n'));
        let marker = match kind {
            ListKind::Ordered(s) => format!("{}. ", s + i as i64),
            ListKind::Bullet(b) | ListKind::Task(b) => format!("{b} "),
        };
        let indent = " ".repeat(marker.len());
        if let ListKind::Task(_) = kind {
            let check = if item.attrs().get_bool("checked").unwrap_or(false) {
                "[x]"
            } else {
                "[ ]"
            };
            // GFM's marker starts the item's first paragraph. Before any other
            // block it is alone on its line, the block on the next: written on
            // the marker's line, pulldown-cmark reads a quote's or a nested
            // list's continuation lines wrongly. The space after the marker is
            // what makes `- [ ]` alone on a line a marker.
            let first_is_para = item
                .content()
                .children()
                .iter()
                .find(|b| !writes_nothing(b))
                .is_some_and(|b| b.type_name() == "paragraph");
            let first = format!("{marker}{check} ");
            if item_md.is_empty() {
                out.push_str(&first);
            } else if first_is_para {
                out.push_str(&prefix_first_then_rest(&item_md, &first, &indent));
            } else {
                // `---` under the marker's line is a setext heading (`[ ]` as
                // its text) to GitHub: a rule there is written `***`.
                // (Nothing else the writer starts a block with begins `---`.)
                let item_md = match item_md.strip_prefix("---") {
                    Some(rest) => format!("***{rest}"),
                    None => item_md,
                };
                out.push_str(&first);
                out.push('\n');
                out.push_str(&prefix_lines_nonblank(&item_md, &indent));
            }
        } else {
            out.push_str(&prefix_first_then_rest(&item_md, &marker, &indent));
        }
        out.push('\n');
    }
    out.push('\n');
    out
}

// ── tables ──

/// A GFM pipe table when the table is one (a header row and only that, no
/// merged cells, one inline paragraph per cell, one alignment per column);
/// otherwise an HTML `<table>` block.
fn table_to_md(table: &Node) -> String {
    match pipe_table(table) {
        Some(md) => md,
        None => html_table(table),
    }
}

fn pipe_table(table: &Node) -> Option<String> {
    let rows = table.content().children();
    let width = rows.first()?.child_count();
    if width == 0 {
        return None;
    }
    let mut aligns: Vec<Option<&str>> = vec![None; width];
    let mut lines: Vec<String> = Vec::with_capacity(rows.len() + 1);
    for (r, row) in rows.iter().enumerate() {
        if row.child_count() != width {
            return None;
        }
        let mut cells = Vec::with_capacity(width);
        for (c, cell) in row.content().children().iter().enumerate() {
            let want = if r == 0 {
                "table_header_cell"
            } else {
                "table_cell"
            };
            if cell.type_name() != want
                || cell.attrs().get_int("colspan").unwrap_or(1) != 1
                || cell.attrs().get_int("rowspan").unwrap_or(1) != 1
                || cell.child_count() != 1
            {
                return None;
            }
            let para = cell.child(0);
            if para.type_name() != "paragraph"
                || para
                    .content()
                    .children()
                    .iter()
                    .any(|n| n.type_name() == "hard_break")
            {
                return None;
            }
            let align = match para.attrs().get_str("text_align") {
                None | Some("left") => "left",
                Some(a @ ("center" | "right")) => a,
                Some(_) => return None,
            };
            match aligns[c] {
                None => aligns[c] = Some(align),
                Some(a) if a == align => {}
                Some(_) => return None,
            }
            cells.push(inline_to_md(para, Ctx::Cell));
        }
        lines.push(pipe_row(&cells));
        if r == 0 {
            let delims: Vec<String> = aligns
                .iter()
                .map(|a| match a {
                    Some("center") => ":---:".to_string(),
                    Some("right") => "---:".to_string(),
                    _ => "---".to_string(),
                })
                .collect();
            lines.push(pipe_row(&delims));
        }
    }
    Some(format!("{}\n\n", lines.join("\n")))
}

fn pipe_row(cells: &[String]) -> String {
    let mut s = String::from("|");
    for c in cells {
        s.push(' ');
        s.push_str(c);
        s.push_str(" |");
    }
    s
}

/// An HTML table block. A blank line would end the block, so newlines in cell
/// text (a code block) are written as `&#10;`; each row gets a line of its own.
fn html_table(table: &Node) -> String {
    let html = node_to_html(table)
        .replace('\n', "&#10;")
        .replace("</tr>", "</tr>\n");
    let html = match html.strip_prefix("<table>") {
        Some(rest) => format!("<table>\n{rest}"),
        None => html,
    };
    format!("{html}\n\n")
}

// ── inline content ──

/// Where inline content is written: escaping differs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ctx {
    /// A paragraph: its first text starts a line.
    Block,
    /// A heading: as a paragraph, and a trailing `#` run must not close it.
    Heading,
    /// A pipe-table cell: `|` must be escaped; nothing starts a line.
    Cell,
}

/// Inline markdown for a textblock's content.
///
/// Markdown marks (bold, italic, strike, link) are opened and closed as runs,
/// not per text node, so `**a *b* c**` comes out as written; the one that runs
/// longer is opened outside. The HTML marks are tags the reader takes one at a
/// time, so they need not nest with the Markdown runs and are opened and closed
/// where they start and end. Whitespace at the edge of a bold, italic or strike
/// run is moved outside its delimiters (CommonMark does not read `**a **` as
/// bold). Where a delimiter still would not flank, [`InlineWriter::finish`]
/// writes its run as tags.
fn inline_to_md(block: &Node, ctx: Ctx) -> String {
    let nodes = block.content().children();
    let mut w = InlineWriter {
        out: String::new(),
        active: Vec::new(),
        html: Vec::new(),
        pending_ws: String::new(),
        line_start: ctx != Ctx::Cell,
        ctx,
        delims: Vec::new(),
        pairs: 0,
    };
    for (i, node) in nodes.iter().enumerate() {
        if node.type_name() == "hard_break" {
            // A `\` break needs a line after it with something on it, and a
            // heading has one line: a `<br>` instead (the reader takes it).
            let content_follows = nodes[i + 1..].iter().any(|n| match n.text() {
                Some(t) => !t.trim_matches([' ', '\t']).is_empty(),
                None => n.type_name() != "hard_break",
            });
            if ctx == Ctx::Heading || !content_follows {
                w.flush_ws();
                w.write_raw("<br>");
                continue;
            }
            // A delimiter cannot close at the start of the next line: close the
            // runs the content after the break does not continue.
            let next_md = nodes[i + 1..]
                .iter()
                .find(|n| n.type_name() != "hard_break")
                .map(|n| syntax_marks(n).0)
                .unwrap_or_default();
            let keep = w.common_prefix(&next_md);
            w.close_md_to(keep);
            w.flush_ws();
            w.out.push_str("\\\n");
            w.line_start = ctx != Ctx::Cell;
            continue;
        }
        let (md_marks, html_marks) = syntax_marks(node);
        let is_code_text = node.is_text() && node.marks().iter().any(|m| m.type_name() == "code");
        if let Some(text) = node.text()
            && !is_code_text
            && text.chars().all(char::is_whitespace)
        {
            // A whitespace-only run opens no delimiters (`** **` is not bold).
            let keep = w.common_prefix(&md_marks);
            w.close_md_to(keep);
            // Whitespace still pending belongs to the run before, as below.
            w.flush_ws();
            w.set_html(&html_marks);
            w.pending_ws.push_str(text);
            continue;
        }
        let keep = w.common_prefix(&md_marks);
        w.close_md_to(keep);
        // Pending whitespace belongs to the previous run: inside its HTML marks.
        w.flush_ws();
        w.set_html(&html_marks);
        // Open the missing Markdown marks, the longest-running outermost.
        let mut to_open: Vec<(usize, usize, Mark)> = md_marks
            .iter()
            .enumerate()
            .filter(|(_, m)| !w.active.contains(m))
            .map(|(rank, m)| (run_length(nodes, i, m), rank, m.clone()))
            .collect();
        to_open.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then((b.2.type_name() == "link").cmp(&(a.2.type_name() == "link")))
                .then(a.1.cmp(&b.1))
        });
        let (lead, core, trail) = match node.text() {
            Some(t) if !is_code_text => split_ws(t),
            _ => ("", node.text().unwrap_or(""), ""),
        };
        // Leading whitespace goes before the first delimiter opened here.
        let first_delim = to_open
            .iter()
            .position(|(_, _, m)| is_delimiter(m))
            .unwrap_or(to_open.len());
        for (k, (_, _, m)) in to_open.iter().enumerate() {
            if k == first_delim {
                w.write_ws(lead);
            }
            if m.type_name() == "link" {
                w.escape_trailing_bang();
            }
            w.open_md(m);
        }
        if first_delim == to_open.len() {
            w.write_ws(lead);
        }
        if is_code_text {
            w.write_raw(&code_span(core, ctx));
        } else if node.is_text() {
            let escaped = escape_text(core, ctx, w.line_start);
            w.write_raw(&escaped);
        } else if node.type_name() == "image" {
            w.write_raw(&image_md(node, ctx));
        }
        w.pending_ws.push_str(trail);
    }
    w.close_md_to(0);
    w.flush_ws();
    w.set_html(&[]);
    w.finish()
}

struct InlineWriter {
    out: String,
    /// Open Markdown marks, outermost first.
    active: Vec<Mark>,
    /// Open HTML marks, in the order they were opened.
    html: Vec<Mark>,
    /// Trailing whitespace of the last run, written once its delimiters close.
    pending_ws: String,
    line_start: bool,
    ctx: Ctx,
    /// Where each bold, italic and strike delimiter goes in `out`, in order. They
    /// are written by [`InlineWriter::finish`], which can only tell once the
    /// characters on both sides are known whether a delimiter run flanks.
    delims: Vec<Delim>,
    /// Pairs opened so far (the next pair's id).
    pairs: usize,
}

/// A bold, italic or strike delimiter, written at `pos` in the output.
struct Delim {
    pos: usize,
    /// Its opening and closing delimiter share the id.
    pair: usize,
    /// `*`, `**` or `~~`.
    md: &'static str,
    /// The tag written instead when the delimiter would not flank.
    tag: &'static str,
    open: bool,
}

impl InlineWriter {
    /// Open a Markdown mark: a delimiter recorded for [`Self::finish`], or a
    /// link's `[`.
    fn open_md(&mut self, m: &Mark) {
        match delim_syntax(m) {
            Some((md, tag)) => {
                self.delims.push(Delim {
                    pos: self.out.len(),
                    pair: self.pairs,
                    md,
                    tag,
                    open: true,
                });
                self.pairs += 1;
                self.line_start = false;
            }
            None => self.write_raw(&open_mark(m)),
        }
        self.active.push(m.clone());
    }

    /// Close a Markdown mark opened with [`Self::open_md`].
    fn close_md(&mut self, m: &Mark) {
        match delim_syntax(m) {
            Some((md, tag)) => {
                let pair = self
                    .delims
                    .iter()
                    .rev()
                    .find(|d| d.open && d.md == md)
                    .map(|d| d.pair)
                    .expect("a closed delimiter was opened");
                self.delims.push(Delim {
                    pos: self.out.len(),
                    pair,
                    md,
                    tag,
                    open: false,
                });
                self.line_start = false;
            }
            None => {
                let s = close_mark(m, self.ctx);
                self.write_raw(&s);
            }
        }
    }

    /// The output with every delimiter written: as Markdown where the run it
    /// joins flanks the way CommonMark needs (an opener left-flanking, a closer
    /// right-flanking, never both kinds in one run), and otherwise, for both ends
    /// of its pair, as the HTML tag the reader also takes (`<strong>`, `<em>`,
    /// `<s>`). So `x**[a](u)**s` and `*~~a~~*b`, which CommonMark reads as
    /// literal asterisks, are written with tags instead.
    fn finish(self) -> String {
        let delims = self.delims;
        if delims.is_empty() {
            return self.out;
        }
        let out = self.out;
        let mut as_tag = vec![false; self.pairs];
        let rendered = |d: &Delim, as_tag: &[bool]| -> String {
            match (as_tag[d.pair], d.open) {
                (false, _) => d.md.to_string(),
                (true, true) => format!("<{}>", d.tag),
                (true, false) => format!("</{}>", d.tag),
            }
        };
        let slot = |ch: char| usize::from(ch == '~');
        loop {
            let mut changed = false;
            // Markdown runs of each character (`*`, `~`) open before `i`.
            let mut open = [0usize; 2];
            let mut counted = vec![false; self.pairs];
            let mut i = 0;
            while i < delims.len() {
                if as_tag[delims[i].pair] {
                    let d = &delims[i];
                    if !d.open && counted[d.pair] {
                        open[slot(d.md.chars().next().unwrap_or('*'))] -= 1;
                    }
                    i += 1;
                    continue;
                }
                // The run: adjacent Markdown delimiters of one character.
                let ch = delims[i].md.chars().next().unwrap_or('*');
                let mut j = i + 1;
                while j < delims.len()
                    && delims[j].pos == delims[i].pos
                    && !as_tag[delims[j].pair]
                    && delims[j].md.starts_with(ch)
                {
                    j += 1;
                }
                let pos = delims[i].pos;
                let prev = if i > 0 && delims[i - 1].pos == pos {
                    rendered(&delims[i - 1], &as_tag).chars().last()
                } else {
                    out[..pos].chars().last()
                };
                let next = if j < delims.len() && delims[j].pos == pos {
                    rendered(&delims[j], &as_tag).chars().next()
                } else {
                    out[pos..].chars().next()
                };
                let len: usize = delims[i..j].iter().map(|d| d.md.len()).sum();
                let cell = self.ctx == Ctx::Cell;
                let opens = delims[i..j].iter().any(|d| d.open);
                let closes = delims[i..j].iter().any(|d| !d.open);
                // Runs of this character still open, other than the ones
                // this run closes.
                let others_open = open[slot(ch)]
                    - delims[i..j]
                        .iter()
                        .filter(|d| !d.open && counted[d.pair])
                        .count()
                    > 0;
                // Whichever way an unknown character counts as punctuation.
                let ok = !(opens && closes)
                    && punctuation_readings(prev).iter().all(|&pp| {
                        punctuation_readings(next).iter().all(|&np| {
                            let run = Run {
                                ch,
                                len,
                                prev,
                                next,
                                prev_punct: pp,
                                next_punct: np,
                                cell,
                            };
                            let wanted = if opens {
                                run.can_open()
                            } else {
                                run.can_close()
                            };
                            // A run that can both open and close is paired by
                            // CommonMark's rule of 3, which the writer does not
                            // model: with another run of its character open it
                            // can pair with that one (`***b*é*c***`), and
                            // between two punctuation characters with one it
                            // was not written for (`***'#*<…*\\*<***`).
                            let both = run.can_open() && run.can_close() && others_open;
                            wanted && !both
                        })
                    });
                if !ok {
                    for d in &delims[i..j] {
                        as_tag[d.pair] = true;
                    }
                    changed = true;
                }
                for d in &delims[i..j] {
                    if d.open && !as_tag[d.pair] {
                        counted[d.pair] = true;
                        open[slot(ch)] += 1;
                    } else if !d.open && counted[d.pair] {
                        open[slot(ch)] -= 1;
                    }
                }
                i = j;
            }
            if !changed {
                break;
            }
        }
        let mut s = String::with_capacity(out.len() + delims.len() * 4);
        let mut at = 0;
        for d in &delims {
            s.push_str(&out[at..d.pos]);
            at = d.pos;
            s.push_str(&rendered(d, &as_tag));
        }
        s.push_str(&out[at..]);
        s
    }

    fn write_raw(&mut self, s: &str) {
        // Spaces and tabs at the start of a line are CommonMark's to strip, and
        // written they would move the block: `-  a` puts a list item's content
        // one column right of its next block, four make a code block.
        let s = if self.line_start && self.ctx != Ctx::Cell {
            s.trim_start_matches([' ', '\t'])
        } else {
            s
        };
        if !s.is_empty() {
            self.out.push_str(s);
            self.line_start = false;
        }
    }

    /// Escape a `!` the output ends with, unless it is escaped already: `![`
    /// would open an image where a link was meant.
    fn escape_trailing_bang(&mut self) {
        if self.delims.last().is_some_and(|d| d.pos == self.out.len()) {
            // A delimiter goes between the `!` and the link.
            return;
        }
        let Some(head) = self.out.strip_suffix('!') else {
            return;
        };
        let backslashes = head.len() - head.trim_end_matches('\\').len();
        if backslashes % 2 == 0 {
            self.out.insert(self.out.len() - 1, '\\');
        }
    }

    fn flush_ws(&mut self) {
        let ws = std::mem::take(&mut self.pending_ws);
        self.write_ws(&ws);
    }

    /// Write whitespace from the text, a line break as an entity, as
    /// `escape_text` writes it.
    fn write_ws(&mut self, ws: &str) {
        if ws.contains(['\n', '\r']) {
            let ws = ws.replace('\n', "&#10;").replace('\r', "&#13;");
            self.write_raw(&ws);
        } else {
            self.write_raw(ws);
        }
    }

    /// How many of the open Markdown marks (from the outside) `marks` keeps.
    fn common_prefix(&self, marks: &[Mark]) -> usize {
        self.active.iter().take_while(|m| marks.contains(m)).count()
    }

    /// Close open Markdown marks down to `keep`, innermost first. Pending
    /// whitespace is written once the last delimiter mark is closed, so it lands
    /// outside it.
    fn close_md_to(&mut self, keep: usize) {
        if self.active.len() <= keep {
            return;
        }
        let closing: Vec<Mark> = self.active.drain(keep..).rev().collect();
        let last_delim = closing.iter().rposition(is_delimiter);
        if last_delim.is_none() {
            // Only a link closes: its text may end in whitespace.
            self.flush_ws();
        }
        for (k, m) in closing.iter().enumerate() {
            self.close_md(m);
            if Some(k) == last_delim {
                self.flush_ws();
            }
        }
    }

    /// Close the HTML marks not in `want` and open the ones not yet open.
    fn set_html(&mut self, want: &[Mark]) {
        let mut k = 0;
        while k < self.html.len() {
            if want.contains(&self.html[k]) {
                k += 1;
            } else {
                let m = self.html.remove(k);
                let s = close_mark(&m, self.ctx);
                self.write_raw(&s);
            }
        }
        for m in want {
            if !self.html.contains(m) {
                self.write_raw(&open_mark(m));
                self.html.push(m.clone());
            }
        }
    }
}

/// The marks of `node` written as Markdown runs (bold, italic, strike, link)
/// and as HTML tags, each in schema order. `code` is written as the text itself.
fn syntax_marks(node: &Node) -> (Vec<Mark>, Vec<Mark>) {
    let mut md = Vec::new();
    let mut html = Vec::new();
    for m in node.marks() {
        match m.type_name() {
            "bold" | "italic" | "strike" | "link" => md.push(m.clone()),
            // A colour that would not read back is not written (see `open_mark`).
            "text_color" if !mark_color(m).is_some_and(is_safe_css_color) => {}
            "underline" | "highlight" | "text_color" | "subscript" | "superscript" => {
                html.push(m.clone())
            }
            _ => {}
        }
    }
    (md, html)
}

/// How many inline nodes from `i` on carry `mark` (hard breaks continue a run).
fn run_length(nodes: &[Node], i: usize, mark: &Mark) -> usize {
    nodes[i..]
        .iter()
        .take_while(|n| n.type_name() == "hard_break" || n.marks().contains(mark))
        .count()
}

/// A delimiter mark's Markdown syntax and the HTML tag written when that would
/// not flank; `None` for any other mark.
fn delim_syntax(mark: &Mark) -> Option<(&'static str, &'static str)> {
    match mark.type_name() {
        "bold" => Some(("**", "strong")),
        "italic" => Some(("*", "em")),
        "strike" => Some(("~~", "s")),
        _ => None,
    }
}

/// A run of `len` delimiter characters `ch` between `prev` and `next` (`None`:
/// the start or end of the inline content), with whether each neighbour counts
/// as punctuation.
struct Run {
    ch: char,
    len: usize,
    prev: Option<char>,
    next: Option<char>,
    prev_punct: bool,
    next_punct: bool,
    cell: bool,
}

impl Run {
    /// Whether the run can open emphasis or strikethrough, as pulldown-cmark 0.12
    /// decides it (`delim_run_can_open`).
    fn can_open(&self) -> bool {
        let Some(next) = self.next else {
            return false;
        };
        if next.is_whitespace() {
            return false;
        }
        let Some(prev) = self.prev else {
            return true;
        };
        if self.cell && next == '|' {
            return false;
        }
        if (self.ch == '*' && !self.next_punct) || (self.ch == '~' && self.len > 1) {
            return true;
        }
        prev.is_whitespace() || self.prev_punct
    }

    /// Whether the run can close (`delim_run_can_close`).
    fn can_close(&self) -> bool {
        let Some(prev) = self.prev else {
            return false;
        };
        if prev.is_whitespace() {
            return false;
        }
        let Some(next) = self.next else {
            return true;
        };
        if self.cell && next == '|' {
            return true;
        }
        if (self.ch == '*' || (self.ch == '~' && self.len > 1)) && !self.prev_punct {
            return true;
        }
        next.is_whitespace() || self.next_punct
    }
}

/// Whether CommonMark counts `c` as punctuation: one answer for ASCII, letters,
/// digits and whitespace, both for any other character (Unicode punctuation and
/// symbols, which this crate has no table for), so a run is written as Markdown
/// only when it flanks either way.
fn punctuation_readings(c: Option<char>) -> &'static [bool] {
    match c {
        Some(c) if c.is_ascii() => {
            if c.is_ascii_punctuation() {
                &[true]
            } else {
                &[false]
            }
        }
        Some(c) if !c.is_alphanumeric() && !c.is_whitespace() => &[false, true],
        _ => &[false],
    }
}

/// A mark written as a Markdown delimiter run, which whitespace must not touch.
fn is_delimiter(mark: &Mark) -> bool {
    matches!(mark.type_name(), "bold" | "italic" | "strike")
}

fn open_mark(mark: &Mark) -> String {
    match mark.type_name() {
        "bold" => "**".to_string(),
        "italic" => "*".to_string(),
        "strike" => "~~".to_string(),
        "link" => "[".to_string(),
        "underline" => "<u>".to_string(),
        // Only a colour `is_safe_css_color` accepts is written: the attribute
        // can arrive unchecked (a `DocNode`, a collaboration peer), and the
        // value lands inside raw HTML. A `text_color` without one is not
        // written at all (`syntax_marks`).
        "highlight" => match mark_color(mark).filter(|c| is_safe_css_color(c)) {
            Some(c) => format!("<mark style=\"background-color:{c}\">"),
            None => "<mark>".to_string(),
        },
        "text_color" => match mark_color(mark).filter(|c| is_safe_css_color(c)) {
            Some(c) => format!("<span style=\"color:{c}\">"),
            None => String::new(),
        },
        "subscript" => "<sub>".to_string(),
        "superscript" => "<sup>".to_string(),
        _ => String::new(),
    }
}

/// A colour mark's `color`, trimmed.
fn mark_color(mark: &Mark) -> Option<&str> {
    mark.attrs.get_str("color").map(str::trim)
}

fn close_mark(mark: &Mark, ctx: Ctx) -> String {
    match mark.type_name() {
        "bold" => "**".to_string(),
        "italic" => "*".to_string(),
        "strike" => "~~".to_string(),
        "link" => {
            let href = mark.attrs.get_str("href").unwrap_or("");
            let title = mark.attrs.get_str("title").unwrap_or("");
            format!("]({})", link_target(href, title, ctx))
        }
        "underline" => "</u>".to_string(),
        "highlight" => "</mark>".to_string(),
        "text_color" => "</span>".to_string(),
        "subscript" => "</sub>".to_string(),
        "superscript" => "</sup>".to_string(),
        _ => String::new(),
    }
}

/// `href "title"`, with the href in angle brackets when it holds spaces or
/// parentheses.
fn link_target(href: &str, title: &str, ctx: Ctx) -> String {
    let mut s = if href.contains(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '<' | '>'))
    {
        let inner = href
            .replace('\\', "\\\\")
            .replace('<', "\\<")
            .replace('>', "\\>");
        format!("<{inner}>")
    } else {
        href.to_string()
    };
    if !title.is_empty() {
        let t = title
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "&#10;")
            .replace('\r', "&#13;");
        s.push_str(&format!(" \"{t}\""));
    }
    if ctx == Ctx::Cell {
        s = s.replace('|', "\\|");
    }
    s
}

fn image_md(node: &Node, ctx: Ctx) -> String {
    let src = node.attrs().get_str("src").unwrap_or("");
    let alt = node.attrs().get_str("alt").unwrap_or("");
    let title = node.attrs().get_str("title").unwrap_or("");
    format!(
        "![{}]({})",
        escape_text(alt, ctx, false),
        link_target(src, title, ctx)
    )
}

/// A code span that reads back as exactly `text`.
fn code_span(text: &str, ctx: Ctx) -> String {
    let text = if ctx == Ctx::Cell {
        text.replace('|', "\\|")
    } else {
        text.to_string()
    };
    let ticks = "`".repeat(longest_run(&text, '`') + 1);
    // CommonMark strips one space from each side when both sides have one, and a
    // backtick at an edge would join the fence: pad with a space either way.
    let pad = text.starts_with('`')
        || text.ends_with('`')
        || (text.starts_with(' ') && text.ends_with(' ') && !text.trim().is_empty());
    if pad {
        format!("{ticks} {text} {ticks}")
    } else {
        format!("{ticks}{text}{ticks}")
    }
}

fn longest_run(text: &str, ch: char) -> usize {
    let mut best = 0;
    let mut cur = 0;
    for c in text.chars() {
        if c == ch {
            cur += 1;
            best = best.max(cur);
        } else {
            cur = 0;
        }
    }
    best
}

/// `(leading whitespace, the rest, trailing whitespace)`.
fn split_ws(t: &str) -> (&str, &str, &str) {
    let core_start = t.len() - t.trim_start().len();
    let core_end = t.trim_end().len();
    if core_end <= core_start {
        return (t, "", "");
    }
    (&t[..core_start], &t[core_start..core_end], &t[core_end..])
}

/// Escape `text` so it reads back as the same text.
fn escape_text(text: &str, ctx: Ctx, line_start: bool) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut line_escape: Option<usize> = None;
    if line_start && ctx != Ctx::Cell {
        line_escape = line_start_escape(&chars);
    }
    let mut trailing_hash: Option<usize> = None;
    if ctx == Ctx::Heading {
        // `# Title #` drops the closing `#` run when a space precedes it.
        let n = chars.iter().rev().take_while(|c| **c == '#').count();
        if n > 0 && (n == chars.len() || chars[chars.len() - n - 1] == ' ') {
            trailing_hash = Some(chars.len() - n);
        }
    }
    let mut out = String::with_capacity(text.len() + 8);
    for (i, &c) in chars.iter().enumerate() {
        // A line break would end the line, or the block: an entity keeps it
        // in the text.
        match c {
            '\n' => {
                out.push_str("&#10;");
                continue;
            }
            '\r' => {
                out.push_str("&#13;");
                continue;
            }
            _ => {}
        }
        let prev = i.checked_sub(1).map(|p| chars[p]);
        let next = chars.get(i + 1).copied();
        let escape = match c {
            '\\' | '*' | '`' | '[' | ']' | '~' => true,
            '_' => {
                !(prev.is_some_and(char::is_alphanumeric)
                    && next.is_some_and(char::is_alphanumeric))
            }
            // Unless whitespace follows in the text: a tag, or an autolink
            // (`<a@b.c>`, and `<~~a~~@b>` across the run that follows).
            '<' => !next.is_some_and(char::is_whitespace),
            '&' => looks_like_entity(&chars[i + 1..]),
            '|' => ctx == Ctx::Cell,
            _ => Some(i) == line_escape || Some(i) == trailing_hash,
        };
        if escape {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The index of the character to escape so a line starting with `chars` is not
/// read as a block marker (heading, quote, list item, rule, setext underline).
fn line_start_escape(chars: &[char]) -> Option<usize> {
    let first = chars.iter().position(|c| *c != ' ' && *c != '\t')?;
    let after = |i: usize| chars.get(i).copied();
    let ends_marker = |i: usize| after(i).is_none_or(|c| c == ' ' || c == '\t');
    match chars[first] {
        '#' => {
            let n = chars[first..].iter().take_while(|c| **c == '#').count();
            (n <= 6 && ends_marker(first + n)).then_some(first)
        }
        '>' => Some(first),
        '+' => ends_marker(first + 1).then_some(first),
        c @ ('-' | '=') => {
            let rule = chars[first..]
                .iter()
                .all(|x| *x == c || *x == ' ' || *x == '\t');
            (rule || (c == '-' && ends_marker(first + 1))).then_some(first)
        }
        d if d.is_ascii_digit() => {
            let n = chars[first..]
                .iter()
                .take_while(|c| c.is_ascii_digit())
                .count();
            let delim = first + n;
            (n <= 9 && matches!(after(delim), Some('.' | ')')) && ends_marker(delim + 1))
                .then_some(delim)
        }
        _ => None,
    }
}

/// `&name;` or `&#…;` follows: an entity reference the reader would decode.
fn looks_like_entity(rest: &[char]) -> bool {
    let body: Vec<char> = rest
        .iter()
        .take(33)
        .take_while(|c| **c != ';')
        .copied()
        .collect();
    if body.len() == rest.len().min(33) || body.is_empty() {
        return false; // no `;` close enough
    }
    let body = match body.first() {
        Some('#') => &body[1..],
        _ => &body[..],
    };
    !body.is_empty() && body.iter().all(char::is_ascii_alphanumeric)
}

/// Concatenate all descendant text (used for code blocks).
fn block_text(node: &Node) -> String {
    let mut out = String::new();
    collect_text(node, &mut out);
    out
}

fn prefix_lines(text: &str, prefix: &str) -> String {
    text.lines()
        .map(|line| format!("{prefix}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A paragraph that writes nothing: no content, or only whitespace.
fn writes_nothing(block: &Node) -> bool {
    block.type_name() == "paragraph"
        && block
            .content()
            .children()
            .iter()
            .all(|n| n.text().is_some_and(|t| t.trim().is_empty()))
}

/// `text` with `prefix` before every line that is not empty.
fn prefix_lines_nonblank(text: &str, prefix: &str) -> String {
    text.lines()
        .map(|line| {
            if line.is_empty() {
                String::new()
            } else {
                format!("{prefix}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn prefix_first_then_rest(text: &str, first: &str, rest: &str) -> String {
    let mut out = String::new();
    for (i, line) in text.lines().enumerate() {
        if i == 0 {
            out.push_str(&format!("{first}{line}"));
        } else {
            out.push('\n');
            out.push_str(&format!("{rest}{line}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s() -> Schema {
        Schema::starter_kit()
    }

    #[test]
    fn heading_round_trips() {
        let schema = s();
        let doc = doc_from_markdown(&schema, "## Title").unwrap();
        let h = doc.child(0);
        assert_eq!(h.type_name(), "heading");
        assert_eq!(h.attrs().get_int("level"), Some(2));
        assert_eq!(h.child(0).text(), Some("Title"));
        assert_eq!(doc_to_markdown(&doc), "## Title");
    }

    #[test]
    fn bold_and_italic() {
        let schema = s();
        let doc = doc_from_markdown(&schema, "This is **bold** and *italic*.").unwrap();
        let md = doc_to_markdown(&doc);
        assert!(md.contains("**bold**"), "{md}");
        assert!(md.contains("*italic*"), "{md}");
    }

    #[test]
    fn nested_bullet_list_structure() {
        let schema = s();
        let doc = doc_from_markdown(&schema, "- one\n- two").unwrap();
        // doc > bullet_list > [list_item > paragraph > text] x2
        let list = doc.child(0);
        assert_eq!(list.type_name(), "bullet_list");
        assert_eq!(list.child_count(), 2);
        let item0 = list.child(0);
        assert_eq!(item0.type_name(), "list_item");
        assert_eq!(item0.child(0).type_name(), "paragraph");
        assert_eq!(item0.child(0).child(0).text(), Some("one"));
        // round-trips to bullets
        let md = doc_to_markdown(&doc);
        assert!(md.contains("- one"), "{md}");
        assert!(md.contains("- two"), "{md}");
    }

    #[test]
    fn ordered_list_start_and_numbering() {
        let schema = s();
        let doc = doc_from_markdown(&schema, "3. a\n4. b").unwrap();
        let list = doc.child(0);
        assert_eq!(list.type_name(), "ordered_list");
        assert_eq!(list.attrs().get_int("start"), Some(3));
        let md = doc_to_markdown(&doc);
        assert!(md.contains("3. a"), "{md}");
        assert!(md.contains("4. b"), "{md}");
    }

    #[test]
    fn code_block_with_language() {
        let schema = s();
        let doc = doc_from_markdown(&schema, "```rust\nfn main() {}\n```").unwrap();
        let cb = doc.child(0);
        assert_eq!(cb.type_name(), "code_block");
        assert_eq!(cb.attrs().get_str("language"), Some("rust"));
        assert_eq!(cb.child(0).text(), Some("fn main() {}"));
        let md = doc_to_markdown(&doc);
        assert!(md.contains("```rust"), "{md}");
        assert!(md.contains("fn main() {}"), "{md}");
    }

    #[test]
    fn blockquote_structure_and_prefix() {
        let schema = s();
        let doc = doc_from_markdown(&schema, "> quoted text").unwrap();
        let bq = doc.child(0);
        assert_eq!(bq.type_name(), "blockquote");
        assert_eq!(bq.child(0).type_name(), "paragraph");
        assert_eq!(bq.child(0).child(0).text(), Some("quoted text"));
        let md = doc_to_markdown(&doc);
        assert!(md.contains("> quoted text"), "{md}");
    }

    #[test]
    fn link_round_trips() {
        let schema = s();
        let doc = doc_from_markdown(&schema, "[click](https://example.com)").unwrap();
        let text = doc.child(0).child(0);
        assert_eq!(text.text(), Some("click"));
        assert!(
            text.marks().iter().any(|m| m.type_name() == "link"
                && m.attrs.get_str("href") == Some("https://example.com")),
            "marks: {:?}",
            text.marks()
        );
        let md = doc_to_markdown(&doc);
        assert!(md.contains("[click](https://example.com)"), "{md}");
    }

    #[test]
    fn horizontal_rule() {
        let schema = s();
        let doc = doc_from_markdown(&schema, "---").unwrap();
        assert_eq!(doc.child(0).type_name(), "horizontal_rule");
        assert_eq!(doc_to_markdown(&doc), "---");
    }

    #[test]
    fn image_round_trips() {
        let schema = s();
        let doc = doc_from_markdown(&schema, "![logo](a.png)").unwrap();
        let img = doc.child(0).child(0);
        assert_eq!(img.type_name(), "image");
        assert_eq!(img.attrs().get_str("src"), Some("a.png"));
        assert_eq!(img.attrs().get_str("alt"), Some("logo"));
        let md = doc_to_markdown(&doc);
        assert!(md.contains("![logo](a.png)"), "{md}");
    }

    #[test]
    fn empty_input_is_one_empty_paragraph() {
        let schema = s();
        let doc = doc_from_markdown(&schema, "").unwrap();
        assert_eq!(doc.child_count(), 1);
        assert_eq!(doc.child(0).type_name(), "paragraph");
    }

    #[test]
    fn inline_code() {
        let schema = s();
        let doc = doc_from_markdown(&schema, "use `Vec` here").unwrap();
        let para = doc.child(0);
        let has_code = para
            .content()
            .children()
            .iter()
            .any(|n| n.marks().iter().any(|m| m.type_name() == "code"));
        assert!(has_code, "expected a code-marked run");
        assert!(doc_to_markdown(&doc).contains("`Vec`"));
    }

    #[test]
    fn javascript_link_is_dropped() {
        let schema = s();
        let doc = doc_from_markdown(&schema, "[click](javascript:alert(1))").unwrap();
        let text = doc.child(0).child(0);
        assert_eq!(text.text(), Some("click"));
        assert!(
            text.marks().is_empty(),
            "javascript: link must not enter the model, got {:?}",
            text.marks()
        );
    }

    #[test]
    fn javascript_image_is_dropped() {
        let schema = s();
        let doc = doc_from_markdown(&schema, "![x](javascript:alert(1))").unwrap();
        // No image node should exist; the alt text must not leak as a stray run.
        let has_image = doc
            .child(0)
            .content()
            .children()
            .iter()
            .any(|n| n.type_name() == "image");
        assert!(!has_image, "javascript: image must be dropped");
    }

    // ── helpers for the round-trip tests ──

    fn mark(schema: &Schema, name: &str, attrs: &[(&str, &str)]) -> Mark {
        let mt = schema.mark_type(name).unwrap();
        let attrs = Attrs::from_iter(
            attrs
                .iter()
                .map(|(k, v)| (*k, AttrValue::from(v.to_string()))),
        );
        Mark::new(mt.clone(), mt.compute_attrs(&attrs).unwrap())
    }

    fn marks(list: &[&Mark]) -> Vec<Mark> {
        let mut set: Vec<Mark> = Vec::new();
        for m in list {
            set = m.add_to_set(&set);
        }
        set
    }

    fn t(schema: &Schema, text: &str, m: &[&Mark]) -> Node {
        schema.text_with_marks(text, marks(m)).unwrap()
    }

    fn node(schema: &Schema, name: &str, attrs: Attrs, children: Vec<Node>) -> Node {
        schema
            .create_node(name, attrs, Fragment::from_children(children))
            .unwrap()
    }

    fn para(schema: &Schema, inline: Vec<Node>) -> Node {
        node(schema, "paragraph", Attrs::new(), inline)
    }

    fn doc(schema: &Schema, blocks: Vec<Node>) -> Node {
        node(schema, "doc", Attrs::new(), blocks)
    }

    /// Write `d`, read it back strictly, and require the same document.
    fn rt(schema: &Schema, d: &Node) -> String {
        let md = doc_to_markdown(d);
        let back = doc_from_markdown_strict(schema, &md)
            .unwrap_or_else(|e| panic!("strict read of {md:?} failed: {e}"));
        assert_eq!(&back, d, "round trip through {md:?}");
        // And writing again changes nothing.
        assert_eq!(doc_to_markdown(&back), md);
        md
    }

    fn all_marks(schema: &Schema) -> Vec<(Mark, &'static str, &'static str)> {
        vec![
            (mark(schema, "bold", &[]), "**", "**"),
            (mark(schema, "italic", &[]), "*", "*"),
            (mark(schema, "strike", &[]), "~~", "~~"),
            (mark(schema, "underline", &[]), "<u>", "</u>"),
            (mark(schema, "highlight", &[]), "<mark>", "</mark>"),
            (
                mark(schema, "highlight", &[("color", "#ffee00")]),
                "<mark style=\"background-color:#ffee00\">",
                "</mark>",
            ),
            (
                mark(schema, "text_color", &[("color", "#c0392b")]),
                "<span style=\"color:#c0392b\">",
                "</span>",
            ),
            (mark(schema, "subscript", &[]), "<sub>", "</sub>"),
            (mark(schema, "superscript", &[]), "<sup>", "</sup>"),
            (
                mark(schema, "link", &[("href", "https://example.com/a")]),
                "[",
                "](https://example.com/a)",
            ),
        ]
    }

    // ── marks ──

    #[test]
    fn every_mark_alone_round_trips() {
        let schema = s();
        for (m, open, close) in all_marks(&schema) {
            let d = doc(
                &schema,
                vec![para(
                    &schema,
                    vec![
                        t(&schema, "a ", &[]),
                        t(&schema, "word", &[&m]),
                        t(&schema, " b", &[]),
                    ],
                )],
            );
            let md = rt(&schema, &d);
            assert_eq!(md, format!("a {open}word{close} b"));
        }
    }

    /// The text of each run with the names of its marks.
    fn runs(d: &Node) -> Vec<(String, Vec<String>)> {
        let mut out = Vec::new();
        fn walk(n: &Node, out: &mut Vec<(String, Vec<String>)>) {
            if let Some(t) = n.text() {
                let names = n
                    .marks()
                    .iter()
                    .map(|m| m.type_name().to_string())
                    .collect();
                out.push((t.to_string(), names));
            } else {
                for c in n.content().children() {
                    walk(c, out);
                }
            }
        }
        walk(d, &mut out);
        out
    }

    #[test]
    fn every_mark_with_edge_whitespace_round_trips() {
        let schema = s();
        let code = mark(&schema, "code", &[]);
        let mut cases: Vec<Mark> = all_marks(&schema).into_iter().map(|(m, _, _)| m).collect();
        cases.push(code);
        for m in &cases {
            for (before, text, after) in [
                ("x", "bold words ", "y"),
                ("x", " bold words", "y"),
                ("x", " both ", "y"),
                ("", "edge ", "z"),
                ("z", " edge", ""),
            ] {
                let mut inline = Vec::new();
                if !before.is_empty() {
                    inline.push(t(&schema, before, &[]));
                }
                inline.push(t(&schema, text, &[m]));
                if !after.is_empty() {
                    inline.push(t(&schema, after, &[]));
                }
                let d = doc(&schema, vec![para(&schema, inline)]);
                let md = doc_to_markdown(&d);
                let back = doc_from_markdown_strict(&schema, &md)
                    .unwrap_or_else(|e| panic!("{md:?}: {e}"));
                // The same text, the word still marked, no stray delimiters.
                let want_text = format!("{before}{text}{after}");
                let got = runs(&back);
                let got_text: String = got.iter().map(|(t, _)| t.as_str()).collect();
                assert_eq!(got_text, want_text, "{md:?}");
                let word = got
                    .iter()
                    .find(|(t, _)| t.contains(text.trim()))
                    .unwrap_or_else(|| panic!("{md:?}: {got:?}"));
                assert_eq!(word.0.trim(), text.trim(), "{md:?}: {got:?}");
                assert!(word.1.iter().any(|n| n == m.type_name()), "{md:?}: {got:?}");
                // Writing what was read changes nothing (a fixed point).
                rt(&schema, &back);
                if is_delimiter(m) {
                    // Whitespace sits outside the delimiters, never inside.
                    let (open, close) = (open_mark(m), close_mark(m, Ctx::Block));
                    let tight = format!("{open}{}{close}", text.trim());
                    assert!(md.contains(&tight), "{md:?} lacks {tight:?}");
                } else {
                    // Everything else keeps its whitespace exactly.
                    assert_eq!(back, d, "{md:?}");
                }
            }
        }
        // The case from the report.
        let bold = mark(&schema, "bold", &[]);
        let d = doc(
            &schema,
            vec![para(
                &schema,
                vec![
                    t(&schema, "bold words ", &[&bold]),
                    t(&schema, "after", &[]),
                ],
            )],
        );
        assert_eq!(doc_to_markdown(&d), "**bold words** after");
    }

    #[test]
    fn nested_marks_round_trip() {
        let schema = s();
        let bold = mark(&schema, "bold", &[]);
        let italic = mark(&schema, "italic", &[]);
        let u = mark(&schema, "underline", &[]);
        let red = mark(&schema, "text_color", &[("color", "#ff0000")]);
        let hl = mark(&schema, "highlight", &[("color", "#00ff00")]);
        let sup = mark(&schema, "superscript", &[]);
        let link = mark(&schema, "link", &[("href", "https://x.org")]);
        let d = doc(
            &schema,
            vec![
                // bold around italic
                para(
                    &schema,
                    vec![
                        t(&schema, "a ", &[&bold]),
                        t(&schema, "b", &[&bold, &italic]),
                        t(&schema, " c", &[&bold]),
                    ],
                ),
                // underline overlapping a bold boundary
                para(
                    &schema,
                    vec![
                        t(&schema, "one ", &[&u]),
                        t(&schema, "two", &[&u, &bold]),
                        t(&schema, " three", &[&bold]),
                    ],
                ),
                // colour inside highlight inside a link, with a superscript
                para(
                    &schema,
                    vec![
                        t(&schema, "E=mc", &[&link, &hl]),
                        t(&schema, "2", &[&link, &hl, &red, &sup]),
                        t(&schema, " ok", &[&link]),
                    ],
                ),
            ],
        );
        let md = rt(&schema, &d);
        assert!(md.contains("**a *b* c**"), "{md}");
    }

    #[test]
    fn adjacent_marks_round_trip() {
        let schema = s();
        let all: Vec<Mark> = all_marks(&schema).into_iter().map(|(m, _, _)| m).collect();
        for a in &all {
            for b in &all {
                if a.same_type(b) {
                    continue;
                }
                let d = doc(
                    &schema,
                    vec![para(
                        &schema,
                        vec![t(&schema, "left", &[a]), t(&schema, "right", &[b])],
                    )],
                );
                rt(&schema, &d);
            }
        }
    }

    #[test]
    fn subscript_inside_superscript_reads_back() {
        let schema = s();
        let doc = doc_from_markdown_strict(&schema, "<sup>a<sub>b</sub>c</sup>").unwrap();
        let p = doc.child(0);
        let names: Vec<Vec<&str>> = p
            .content()
            .children()
            .iter()
            .map(|n| n.marks().iter().map(|m| m.type_name()).collect())
            .collect();
        assert_eq!(
            names,
            vec![vec!["superscript"], vec!["subscript"], vec!["superscript"]]
        );
    }

    #[test]
    fn mark_tags_parse_case_and_quotes() {
        let schema = s();
        let doc = doc_from_markdown_strict(
            &schema,
            "<U>u</U> <span style='color: #123456;'>c</span> <mark style=\"background-color:yellow\">h</mark>",
        )
        .unwrap();
        let p = doc.child(0);
        let c = p
            .content()
            .children()
            .iter()
            .find(|n| n.text() == Some("c"))
            .unwrap();
        assert_eq!(c.marks()[0].attrs.get_str("color"), Some("#123456"));
        let h = p
            .content()
            .children()
            .iter()
            .find(|n| n.text() == Some("h"))
            .unwrap();
        assert_eq!(h.marks()[0].attrs.get_str("color"), Some("yellow"));
        assert!(p.content().children()[0].marks()[0].type_name() == "underline");
    }

    #[test]
    fn other_inline_html_is_dropped_leniently_and_refused_strictly() {
        let schema = s();
        for (md, construct) in [
            (
                "a <span style=\"color:expression(x)\">b</span>",
                Construct::InlineHtml,
            ),
            ("a <span>b</span>", Construct::InlineHtml),
            (
                "a <span style=\"color:red;font-size:9px\">b</span>",
                Construct::InlineHtml,
            ),
            ("a <u title=\"x\">b</u>", Construct::InlineHtml),
            ("a <br class=\"x\"> b", Construct::InlineHtml),
            ("a <kbd>b</kbd>", Construct::InlineHtml),
        ] {
            let doc = doc_from_markdown(&schema, md).unwrap();
            assert!(
                doc.child(0)
                    .content()
                    .children()
                    .iter()
                    .all(|n| n.marks().is_empty()),
                "{md}: {doc:?}"
            );
            match doc_from_markdown_strict(&schema, md) {
                Err(MarkdownError::Unsupported {
                    construct: c, line, ..
                }) => {
                    assert_eq!(c, construct, "{md}");
                    assert_eq!(line, 1);
                }
                other => panic!("{md}: expected a refusal, got {other:?}"),
            }
        }
    }

    // ── tables ──

    fn cell(schema: &Schema, header: bool, blocks: Vec<Node>, span: Option<(i64, i64)>) -> Node {
        let mut attrs = Attrs::new();
        if let Some((c, r)) = span {
            if c != 1 {
                attrs = attrs.with("colspan", AttrValue::Int(c));
            }
            if r != 1 {
                attrs = attrs.with("rowspan", AttrValue::Int(r));
            }
        }
        let name = if header {
            "table_header_cell"
        } else {
            "table_cell"
        };
        node(schema, name, attrs, blocks)
    }

    fn text_cell(schema: &Schema, header: bool, text: &str) -> Node {
        let inline = if text.is_empty() {
            vec![]
        } else {
            vec![t(schema, text, &[])]
        };
        cell(schema, header, vec![para(schema, inline)], None)
    }

    fn row(schema: &Schema, cells: Vec<Node>) -> Node {
        node(schema, "table_row", Attrs::new(), cells)
    }

    fn table(schema: &Schema, rows: Vec<Node>) -> Node {
        node(schema, "table", Attrs::new(), rows)
    }

    #[test]
    fn gfm_table_reads_into_table_nodes() {
        let schema = s();
        let md = "| Name | Qty |\n| :--- | ---: |\n| apple | 3 |\n| **pear** |  |";
        let d = doc_from_markdown_strict(&schema, md).unwrap();
        let tb = d.child(0);
        assert_eq!(tb.type_name(), "table");
        assert_eq!(tb.child_count(), 3);
        assert_eq!(tb.child(0).child(0).type_name(), "table_header_cell");
        assert_eq!(tb.child(1).child(0).type_name(), "table_cell");
        let qty = tb.child(1).child(1).child(0);
        assert_eq!(qty.type_name(), "paragraph");
        assert_eq!(qty.attrs().get_str("text_align"), Some("right"));
        assert_eq!(qty.child(0).text(), Some("3"));
        let pear = tb.child(2).child(0).child(0).child(0);
        assert_eq!(pear.marks()[0].type_name(), "bold");
        // An empty cell still holds a paragraph.
        assert_eq!(tb.child(2).child(1).child(0).child_count(), 0);
        let out = doc_to_markdown(&d);
        assert_eq!(
            out,
            "| Name | Qty |\n| --- | ---: |\n| apple | 3 |\n| **pear** |  |"
        );
        rt(&schema, &d);
    }

    #[test]
    fn plain_table_with_header_round_trips_as_pipes() {
        let schema = s();
        let d = doc(
            &schema,
            vec![table(
                &schema,
                vec![
                    row(
                        &schema,
                        vec![text_cell(&schema, true, "a"), text_cell(&schema, true, "b")],
                    ),
                    row(
                        &schema,
                        vec![
                            text_cell(&schema, false, "1"),
                            text_cell(&schema, false, "2"),
                        ],
                    ),
                ],
            )],
        );
        assert_eq!(rt(&schema, &d), "| a | b |\n| --- | --- |\n| 1 | 2 |");
    }

    #[test]
    fn pipes_in_cells_are_escaped() {
        let schema = s();
        let code = mark(&schema, "code", &[]);
        let link = mark(&schema, "link", &[("href", "https://x.org/?a|b")]);
        let d = doc(
            &schema,
            vec![table(
                &schema,
                vec![
                    row(
                        &schema,
                        vec![
                            text_cell(&schema, true, "a | b"),
                            text_cell(&schema, true, "c"),
                        ],
                    ),
                    row(
                        &schema,
                        vec![
                            cell(
                                &schema,
                                false,
                                vec![para(&schema, vec![t(&schema, "x|y", &[&code])])],
                                None,
                            ),
                            cell(
                                &schema,
                                false,
                                vec![para(&schema, vec![t(&schema, "l|k", &[&link])])],
                                None,
                            ),
                        ],
                    ),
                ],
            )],
        );
        let md = rt(&schema, &d);
        assert!(md.starts_with("| a \\| b | c |"), "{md}");
    }

    #[test]
    fn merged_cells_round_trip_as_html() {
        let schema = s();
        let d = doc(
            &schema,
            vec![
                para(&schema, vec![t(&schema, "before", &[])]),
                table(
                    &schema,
                    vec![
                        row(
                            &schema,
                            vec![cell(
                                &schema,
                                true,
                                vec![para(&schema, vec![t(&schema, "wide", &[])])],
                                Some((2, 1)),
                            )],
                        ),
                        row(
                            &schema,
                            vec![
                                cell(
                                    &schema,
                                    false,
                                    vec![para(&schema, vec![t(&schema, "tall", &[])])],
                                    Some((1, 2)),
                                ),
                                text_cell(&schema, false, "x"),
                            ],
                        ),
                        row(&schema, vec![text_cell(&schema, false, "y")]),
                    ],
                ),
                para(&schema, vec![t(&schema, "after", &[])]),
            ],
        );
        let md = rt(&schema, &d);
        assert!(md.contains("<th colspan=\"2\">"), "{md}");
        assert!(md.contains("<td rowspan=\"2\">"), "{md}");
    }

    #[test]
    fn tables_that_are_not_pipe_tables_round_trip_as_html() {
        let schema = s();
        let bold = mark(&schema, "bold", &[]);
        let u = mark(&schema, "underline", &[]);
        let code_block = node(
            &schema,
            "code_block",
            Attrs::new(),
            vec![t(&schema, "fn a() {}\n\nfn b() {}", &[])],
        );
        let list = node(
            &schema,
            "bullet_list",
            Attrs::new(),
            vec![node(
                &schema,
                "list_item",
                Attrs::new(),
                vec![para(&schema, vec![t(&schema, "item", &[])])],
            )],
        );
        let hard_break = node(&schema, "hard_break", Attrs::new(), vec![]);
        let d = doc(
            &schema,
            vec![
                // No header row.
                table(
                    &schema,
                    vec![row(
                        &schema,
                        vec![
                            text_cell(&schema, false, "a"),
                            text_cell(&schema, false, "b"),
                        ],
                    )],
                ),
                // Several blocks in a cell, a code block with a blank line, marks.
                table(
                    &schema,
                    vec![
                        row(&schema, vec![text_cell(&schema, true, "h")]),
                        row(
                            &schema,
                            vec![cell(
                                &schema,
                                false,
                                vec![
                                    para(
                                        &schema,
                                        vec![
                                            t(&schema, "x ", &[&bold]),
                                            t(&schema, "<y> & z", &[&u]),
                                        ],
                                    ),
                                    code_block,
                                    list,
                                ],
                                None,
                            )],
                        ),
                    ],
                ),
                // A hard break in a cell.
                table(
                    &schema,
                    vec![
                        row(&schema, vec![text_cell(&schema, true, "h")]),
                        row(
                            &schema,
                            vec![cell(
                                &schema,
                                false,
                                vec![para(
                                    &schema,
                                    vec![t(&schema, "a", &[]), hard_break, t(&schema, "b", &[])],
                                )],
                                None,
                            )],
                        ),
                    ],
                ),
            ],
        );
        let md = rt(&schema, &d);
        assert_eq!(md.matches("<table>").count(), 3, "{md}");
        assert!(
            !md.contains("\n\n<tr>"),
            "no blank line inside an HTML block: {md}"
        );
    }

    // ── strict ──

    fn refusal(md: &str) -> (Construct, usize) {
        match doc_from_markdown_strict(&s(), md) {
            Err(MarkdownError::Unsupported {
                construct, line, ..
            }) => (construct, line),
            other => panic!("{md:?}: expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn strict_names_the_construct_and_its_line() {
        assert_eq!(
            refusal("# T\n\ntext[^1]\n\n[^1]: note"),
            (Construct::Footnote, 3)
        );
        assert_eq!(
            refusal("intro\n\n- [ ] todo\n- done"),
            (Construct::TaskList, 3)
        );
        assert_eq!(
            refusal("a\n\nb\n\n<div>\nraw\n</div>"),
            (Construct::HtmlBlock, 5)
        );
        assert_eq!(refusal("<!-- note -->"), (Construct::HtmlBlock, 1));
        assert_eq!(refusal("one\ntwo <kbd> three"), (Construct::InlineHtml, 2));
        assert_eq!(
            refusal("a\n\nsome <u>underlined"),
            (Construct::UnmatchedTag, 3)
        );
        assert_eq!(refusal("stray</sup>"), (Construct::UnmatchedTag, 1));
        assert_eq!(
            refusal("x\n\n[c](javascript:alert(1))"),
            (Construct::UnsafeLink, 3)
        );
        assert_eq!(
            refusal("x\ny\n![c](javascript:alert(1))"),
            (Construct::UnsafeImage, 3)
        );
        assert_eq!(
            refusal(
                "<table><tr><td>a</td></tr></table>\n\n<table><caption>c</caption><tr><td>a</td></tr></table>"
            ),
            (Construct::HtmlBlock, 3)
        );
        let e = doc_from_markdown_strict(&s(), "\n\n- [ ] todo\n- not").unwrap_err();
        assert_eq!(e.to_string(), "line 3: task list is not supported: [ ]");
    }

    #[test]
    fn lenient_keeps_what_strict_refuses_as_before() {
        let schema = s();
        // Footnotes, and a task marker in a list some of whose items have
        // none, are text to the lenient parse.
        let d = doc_from_markdown(&schema, "- [ ] todo\n- not").unwrap();
        assert_eq!(
            d.child(0).child(0).child(0).child(0).text(),
            Some("[ ] todo")
        );
        // Raw HTML blocks are dropped.
        let d = doc_from_markdown(&schema, "<div>x</div>\n\nafter").unwrap();
        assert_eq!(d.child_count(), 1);
        // An unclosed tag marks the rest of its block only.
        let d = doc_from_markdown(&schema, "a <u>b\n\nc").unwrap();
        assert_eq!(d.child(0).child(1).marks()[0].type_name(), "underline");
        assert!(d.child(1).child(0).marks().is_empty());
    }

    #[test]
    fn strict_accepts_html_tables_without_paragraphs() {
        let schema = s();
        let d = doc_from_markdown_strict(
            &schema,
            "<table>\n<thead><tr><th>h</th></tr></thead>\n<tbody><tr><td><b>x</b></td></tr></tbody>\n</table>",
        )
        .unwrap();
        let tb = d.child(0);
        assert_eq!(tb.type_name(), "table");
        assert_eq!(tb.child(0).child(0).type_name(), "table_header_cell");
    }

    // ── escaping and stability ──

    #[test]
    fn literal_syntax_in_text_round_trips() {
        let schema = s();
        for text in [
            "*not italic* and **not bold**",
            "snake_case and _under_",
            "# not a heading",
            "1. not a list",
            "- not a list",
            "+ not a list",
            "> not a quote",
            "---",
            "===",
            "<u>not underlined</u>",
            "a &amp; b & c",
            "[not](a link) ![nor](an image)",
            "`not code` ~~nor strike~~",
            "back\\slash",
            "pipes | stay",
            "x < y > z",
        ] {
            let d = doc(&schema, vec![para(&schema, vec![t(&schema, text, &[])])]);
            rt(&schema, &d);
        }
        // A trailing `#` run in a heading is kept.
        let h = node(
            &schema,
            "heading",
            Attrs::from_iter([("level", AttrValue::Int(2))]),
            vec![t(&schema, "C #", &[])],
        );
        rt(&schema, &doc(&schema, vec![h]));
    }

    #[test]
    fn code_with_backticks_round_trips() {
        let schema = s();
        let code = mark(&schema, "code", &[]);
        for text in ["a`b", "`edge", "``", " padded ", "x"] {
            let d = doc(
                &schema,
                vec![para(&schema, vec![t(&schema, text, &[&code])])],
            );
            rt(&schema, &d);
        }
        let cb = node(
            &schema,
            "code_block",
            Attrs::from_iter([("language", AttrValue::from("md".to_string()))]),
            vec![t(&schema, "```\nfenced\n```", &[])],
        );
        rt(&schema, &doc(&schema, vec![cb]));
    }

    #[test]
    fn hard_break_round_trips() {
        let schema = s();
        let br = node(&schema, "hard_break", Attrs::new(), vec![]);
        let d = doc(
            &schema,
            vec![para(
                &schema,
                vec![t(&schema, "line one", &[]), br, t(&schema, "- two", &[])],
            )],
        );
        assert_eq!(rt(&schema, &d), "line one\\\n\\- two");
    }

    #[test]
    fn representative_documents_are_stable() {
        let schema = s();
        for md in [
            "# Trip notes\n\nWe left at **6am** and *finally* reached <u>the ridge</u> by noon.\n\n## Packing\n\n- boots\n- water, <mark>3 litres</mark>\n  - one in the pack\n  - two in the car\n- map\n\n1. drive\n2. hike\n\n```bash\necho done\n```\n\n> It was <span style=\"color:#2e86c1\">cold</span>.\n\n---\n\nH<sub>2</sub>O and x<sup>2</sup>, see [the guide](https://example.com/guide \"Guide\").",
            "| Day | Miles |\n| --- | ---: |\n| Mon | 12 |\n| Tue | 8 \\| 9 |\n\nTotal: ~~20~~ 21.",
            "Plain paragraph.\\\nWith a break.\n\n<table>\n<tr><td colspan=\"2\"><p>merged</p></td></tr>\n<tr><td><p>a</p></td><td><p>b</p></td></tr>\n</table>",
        ] {
            let d = doc_from_markdown_strict(&schema, md).unwrap();
            let out = doc_to_markdown(&d);
            let d2 = doc_from_markdown_strict(&schema, &out).unwrap();
            assert_eq!(d2, d, "{out}");
            assert_eq!(doc_to_markdown(&d2), out);
        }
        // The first one is already in the writer's own form.
        let md =
            "# Trip notes\n\nWe left at **6am** and *finally* reached <u>the ridge</u> by noon.";
        assert_eq!(
            doc_to_markdown(&doc_from_markdown(&schema, md).unwrap()),
            md
        );
    }

    #[test]
    fn email_autolink_gets_mailto() {
        let schema = s();
        let d = doc_from_markdown_strict(&schema, "<joe@example.com>").unwrap();
        let text = d.child(0).child(0);
        assert_eq!(
            text.marks()[0].attrs.get_str("href"),
            Some("mailto:joe@example.com")
        );
        rt(&schema, &d);
    }

    #[test]
    fn parse_inline_tag_shapes() {
        assert_eq!(
            parse_inline_tag("<u>"),
            Some(InlineTag::Open {
                tag: "u",
                mark: "underline",
                color: None
            })
        );
        assert_eq!(
            parse_inline_tag("</mark >"),
            Some(InlineTag::Close { tag: "mark" })
        );
        assert_eq!(parse_inline_tag("<span>"), None);
        assert_eq!(parse_inline_tag("<sub style=\"color:red\">"), None);
        assert_eq!(parse_inline_tag("<span style=\"color:url(x)\">"), None);
        assert_eq!(
            parse_inline_tag("<span style=\"color:red\" id=\"x\">"),
            None
        );
        assert_eq!(parse_inline_tag("<u/>"), None);
    }
}
