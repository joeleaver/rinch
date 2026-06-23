//! Markdown I/O via `pulldown-cmark` — feature `markdown`.
//!
//! Salvaged from the old `rinch-editor` `document/fragment.rs`, but rebound to the
//! real [`Node`] model and producing **properly nested** structure (a markdown
//! bullet list becomes one `bullet_list` node containing `list_item`s, each with a
//! `paragraph` — not the old flat "one block per item"). Markdown is a lossy
//! interchange: marks without a markdown equivalent (underline, highlight,
//! text_color, sub/superscript) serialize as plain text. The durable, total
//! format is [`super::doc_json`].

use crate::EditorError;
use crate::model::{AttrValue, Attrs, Fragment, Mark, Node};
use crate::schema::Schema;
use crate::serialize::html::is_safe_url;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

// ─── markdown → doc ──────────────────────────────────────────────────────────

/// Parse a markdown string into a `doc` node, validated against `schema`.
pub fn doc_from_markdown(schema: &Schema, md: &str) -> Result<Node, EditorError> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);

    let mut builder = MdBuilder::new(schema);
    for event in Parser::new_ext(md, options) {
        builder.handle(event)?;
    }
    builder.finish()
}

/// A container being assembled (doc, blockquote, list, list_item).
struct Container {
    type_name: &'static str,
    attrs: Attrs,
    children: Vec<Node>,
}

/// A textblock being assembled (paragraph, heading, code_block).
struct TextBlock {
    type_name: &'static str,
    attrs: Attrs,
    content: Vec<Node>,
    is_code: bool,
}

struct MdBuilder<'a> {
    schema: &'a Schema,
    /// Container stack; `containers[0]` is always the `doc`.
    containers: Vec<Container>,
    /// The open textblock, if any.
    inline: Option<TextBlock>,
    /// Active inline marks.
    marks: Vec<Mark>,
    /// In-progress image: `(src, title, alt-accumulator)`.
    image: Option<(String, String, String)>,
}

impl<'a> MdBuilder<'a> {
    fn new(schema: &'a Schema) -> Self {
        Self {
            schema,
            containers: vec![Container {
                type_name: "doc",
                attrs: Attrs::new(),
                children: Vec::new(),
            }],
            inline: None,
            marks: Vec::new(),
            image: None,
        }
    }

    fn handle(&mut self, event: Event<'_>) -> Result<(), EditorError> {
        match event {
            Event::Start(tag) => self.start(tag)?,
            Event::End(tag_end) => self.end(tag_end)?,
            Event::Text(t) => self.push_text(&t)?,
            Event::Code(t) => self.push_code(&t)?,
            Event::SoftBreak => self.push_text(" ")?,
            Event::HardBreak => self.push_hard_break()?,
            Event::Rule => self.push_rule()?,
            _ => {} // raw HTML, footnotes, math, task markers: ignored
        }
        Ok(())
    }

    fn start(&mut self, tag: Tag<'_>) -> Result<(), EditorError> {
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
            }
            Tag::Strong => self.add_mark("bold", Attrs::new())?,
            Tag::Emphasis => self.add_mark("italic", Attrs::new())?,
            Tag::Strikethrough => self.add_mark("strike", Attrs::new())?,
            Tag::Link {
                dest_url, title, ..
            } => {
                // Mirror the HTML ingress whitelist: a javascript:/vbscript:/data:
                // href never enters the model. On rejection the link mark is simply
                // not pushed (children parse transparently); the matching End(Link)
                // `remove_mark("link")` is then a harmless no-op.
                if is_safe_url(&dest_url, false) {
                    let mut pairs: Vec<(&str, AttrValue)> =
                        vec![("href", AttrValue::from(dest_url.to_string()))];
                    if !title.is_empty() {
                        pairs.push(("title", AttrValue::from(title.to_string())));
                    }
                    self.add_mark("link", Attrs::from_iter(pairs))?;
                }
            }
            Tag::Image {
                dest_url, title, ..
            } => {
                self.image = Some((dest_url.to_string(), title.to_string(), String::new()));
            }
            _ => {}
        }
        Ok(())
    }

    fn end(&mut self, tag_end: TagEnd) -> Result<(), EditorError> {
        match tag_end {
            TagEnd::Heading(_) | TagEnd::Paragraph | TagEnd::CodeBlock => self.flush_inline()?,
            TagEnd::BlockQuote(_) | TagEnd::List(_) | TagEnd::Item => {
                self.flush_inline()?;
                self.pop_container()?;
            }
            TagEnd::Image => {
                if let Some((src, title, alt)) = self.image.take() {
                    // Unsafe src → drop the image entirely. The alt text was captured
                    // into the image buffer (not leaked into the paragraph) so nothing
                    // dangerous and nothing stray remains.
                    if is_safe_url(&src, true) {
                        self.push_image(&src, &title, &alt)?;
                    }
                }
            }
            TagEnd::Strong => self.remove_mark("bold"),
            TagEnd::Emphasis => self.remove_mark("italic"),
            TagEnd::Strikethrough => self.remove_mark("strike"),
            TagEnd::Link => self.remove_mark("link"),
            _ => {}
        }
        Ok(())
    }

    fn open_inline(&mut self, type_name: &'static str, attrs: Attrs, is_code: bool) {
        self.inline = Some(TextBlock {
            type_name,
            attrs,
            content: Vec::new(),
            is_code,
        });
    }

    fn ensure_inline(&mut self) {
        if self.inline.is_none() {
            self.open_inline("paragraph", Attrs::new(), false);
        }
    }

    fn flush_inline(&mut self) -> Result<(), EditorError> {
        let Some(tb) = self.inline.take() else {
            return Ok(());
        };
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
        self.containers.push(Container {
            type_name,
            attrs,
            children: Vec::new(),
        });
    }

    fn pop_container(&mut self) -> Result<(), EditorError> {
        // Never pop the root doc container.
        if self.containers.len() <= 1 {
            return Ok(());
        }
        let c = self.containers.pop().expect("non-root container");
        let node = self.build_container(c)?;
        self.top_mut().children.push(node);
        Ok(())
    }

    fn push_text(&mut self, t: &str) -> Result<(), EditorError> {
        if let Some((_, _, alt)) = &mut self.image {
            alt.push_str(t);
            return Ok(());
        }
        if t.is_empty() {
            return Ok(());
        }
        self.ensure_inline();
        let node = self.schema.text_with_marks(t, self.marks.clone())?;
        self.inline
            .as_mut()
            .expect("inline open")
            .content
            .push(node);
        Ok(())
    }

    fn push_code(&mut self, t: &str) -> Result<(), EditorError> {
        if self.image.is_some() {
            return Ok(());
        }
        self.ensure_inline();
        let marks = match self.schema.mark_type("code") {
            Some(mt) => {
                Mark::new(mt.clone(), mt.compute_attrs(&Attrs::new())?).add_to_set(&self.marks)
            }
            None => self.marks.clone(),
        };
        let node = self.schema.text_with_marks(t, marks)?;
        self.inline
            .as_mut()
            .expect("inline open")
            .content
            .push(node);
        Ok(())
    }

    fn push_hard_break(&mut self) -> Result<(), EditorError> {
        if self.image.is_some() {
            return Ok(());
        }
        self.ensure_inline();
        let node = self.make_node("hard_break", Attrs::new(), Fragment::empty())?;
        self.inline
            .as_mut()
            .expect("inline open")
            .content
            .push(node);
        Ok(())
    }

    fn push_rule(&mut self) -> Result<(), EditorError> {
        self.flush_inline()?;
        let hr = self.make_node("horizontal_rule", Attrs::new(), Fragment::empty())?;
        self.top_mut().children.push(hr);
        Ok(())
    }

    fn push_image(&mut self, src: &str, title: &str, alt: &str) -> Result<(), EditorError> {
        self.ensure_inline();
        let mut pairs: Vec<(&str, AttrValue)> = vec![("src", AttrValue::from(src.to_string()))];
        if !alt.is_empty() {
            pairs.push(("alt", AttrValue::from(alt.to_string())));
        }
        if !title.is_empty() {
            pairs.push(("title", AttrValue::from(title.to_string())));
        }
        let img = self.make_node("image", Attrs::from_iter(pairs), Fragment::empty())?;
        self.inline.as_mut().expect("inline open").content.push(img);
        Ok(())
    }

    fn add_mark(&mut self, name: &str, attrs: Attrs) -> Result<(), EditorError> {
        if let Some(mt) = self.schema.mark_type(name) {
            let mark = Mark::new(mt.clone(), mt.compute_attrs(&attrs)?);
            self.marks = mark.add_to_set(&self.marks);
        }
        Ok(())
    }

    fn remove_mark(&mut self, name: &str) {
        self.marks.retain(|m| m.type_name() != name);
    }

    fn top_mut(&mut self) -> &mut Container {
        self.containers.last_mut().expect("doc container")
    }

    fn build_container(&self, c: Container) -> Result<Node, EditorError> {
        match c.type_name {
            "blockquote" | "list_item" => {
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

    fn finish(mut self) -> Result<Node, EditorError> {
        self.flush_inline()?;
        // Close any containers the (malformed) input left open.
        while self.containers.len() > 1 {
            self.pop_container()?;
        }
        let doc = self.containers.pop().expect("doc container");
        let children = self.ensure_block_plus(doc.children)?;
        self.make_node("doc", Attrs::new(), Fragment::from_children(children))
    }
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

// ─── doc → markdown ──────────────────────────────────────────────────────────

/// Serialize a `doc` node to a markdown string.
pub fn doc_to_markdown(doc: &Node) -> String {
    let md = blocks_to_md(doc.content().children());
    md.trim_end().to_string()
}

fn blocks_to_md(blocks: &[Node]) -> String {
    let mut out = String::new();
    for b in blocks {
        out.push_str(&block_to_md(b));
    }
    out
}

fn block_to_md(node: &Node) -> String {
    match node.type_name() {
        "heading" => {
            let level = node.attrs().get_int("level").unwrap_or(1).clamp(1, 6) as usize;
            format!("{} {}\n\n", "#".repeat(level), inline_to_md(node))
        }
        "code_block" => {
            let lang = node.attrs().get_str("language").unwrap_or("");
            format!("```{lang}\n{}\n```\n\n", block_text(node))
        }
        "blockquote" => {
            let inner = blocks_to_md(node.content().children());
            format!("{}\n\n", prefix_lines(inner.trim_end(), "> "))
        }
        "bullet_list" => list_to_md(node, None),
        "ordered_list" => list_to_md(node, Some(node.attrs().get_int("start").unwrap_or(1))),
        "horizontal_rule" => "---\n\n".to_string(),
        // paragraph and any other textblock-ish node.
        _ => format!("{}\n\n", inline_to_md(node)),
    }
}

fn list_to_md(list: &Node, start: Option<i64>) -> String {
    let mut out = String::new();
    for (i, item) in list.content().children().iter().enumerate() {
        let item_md = blocks_to_md(item.content().children());
        let marker = match start {
            Some(s) => format!("{}. ", s + i as i64),
            None => "- ".to_string(),
        };
        let indent = " ".repeat(marker.len());
        out.push_str(&prefix_first_then_rest(
            item_md.trim_end(),
            &marker,
            &indent,
        ));
        out.push('\n');
    }
    out.push('\n');
    out
}

/// Inline markdown for a textblock's content.
fn inline_to_md(block: &Node) -> String {
    let mut out = String::new();
    for inline in block.content().children() {
        if let Some(text) = inline.text() {
            let mut s = text.to_string();
            for mark in inline.marks() {
                s = wrap_mark_md(mark, &s);
            }
            out.push_str(&s);
        } else if inline.type_name() == "hard_break" {
            out.push_str("  \n");
        } else if inline.type_name() == "image" {
            let src = inline.attrs().get_str("src").unwrap_or("");
            let alt = inline.attrs().get_str("alt").unwrap_or("");
            let title = inline.attrs().get_str("title").unwrap_or("");
            if title.is_empty() {
                out.push_str(&format!("![{alt}]({src})"));
            } else {
                out.push_str(&format!("![{alt}]({src} \"{title}\")"));
            }
        }
    }
    out
}

fn wrap_mark_md(mark: &Mark, text: &str) -> String {
    match mark.type_name() {
        "bold" => format!("**{text}**"),
        "italic" => format!("*{text}*"),
        "strike" => format!("~~{text}~~"),
        "code" => format!("`{text}`"),
        "link" => {
            let href = mark.attrs.get_str("href").unwrap_or("");
            let title = mark.attrs.get_str("title").unwrap_or("");
            if title.is_empty() {
                format!("[{text}]({href})")
            } else {
                format!("[{text}]({href} \"{title}\")")
            }
        }
        // underline, highlight, text_color, subscript, superscript: no markdown
        // equivalent → emit plain (lossy; the lossless format is DocNode).
        _ => text.to_string(),
    }
}

/// Concatenate all descendant text (used for code blocks).
fn block_text(node: &Node) -> String {
    let mut out = String::new();
    collect(node, &mut out);
    fn collect(node: &Node, out: &mut String) {
        if let Some(t) = node.text() {
            out.push_str(t);
        } else {
            for child in node.content().children() {
                collect(child, out);
            }
        }
    }
    out
}

fn prefix_lines(text: &str, prefix: &str) -> String {
    text.lines()
        .map(|line| format!("{prefix}{line}"))
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
}
