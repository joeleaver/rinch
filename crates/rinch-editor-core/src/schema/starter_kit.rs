//! The starter-kit schema: the default node and mark types.

use super::*;

impl Schema {
    /// Create a schema with the starter-kit defaults.
    ///
    /// Includes standard block nodes (paragraph, heading, lists, etc.),
    /// inline nodes (text, image, hard_break), and common marks
    /// (bold, italic, underline, strike, code, link, highlight, etc.).
    pub fn starter_kit() -> Self {
        let mut builder = SchemaBuilder::new();

        // === Nodes ===

        // doc - top-level container
        builder = builder.node("doc", NodeSpec::builder("doc").content("block+").build());

        // paragraph
        builder = builder.node(
            "paragraph",
            NodeSpec::builder("paragraph")
                .content("inline*")
                .group("block")
                // Indent level (0 = flush left); rendered as a left margin. Bumped by
                // the indent / outdent commands when the cursor isn't in a list.
                .attr("indent", AttrSpec::optional(AttrValue::Int(0)))
                // Horizontal alignment of the block's text: "left", "center", "right"
                // or "justify", or unset (the attribute absent, the default). Set by the
                // setTextAlign* commands; a stored value projects to `text-align` in the
                // view and copy-out HTML, an unset one to none, so the stylesheet decides
                // (a `th` centres, an RTL editor starts right). A stored "left" is an
                // explicit choice, kept apart from unset (#2210).
                .attr("text_align", AttrSpec::optional("left"))
                // An app's own `data-*` attributes (a style it names, an id):
                // see `schema::data_attrs`.
                .data_attrs(true)
                .parse_html(vec!["p".into()])
                .build(),
        );

        // heading
        builder = builder.node(
            "heading",
            NodeSpec::builder("heading")
                .content("inline*")
                .group("block")
                .attr("level", AttrSpec::optional(AttrValue::Int(1)))
                .defining(true)
                .attr("indent", AttrSpec::optional(AttrValue::Int(0)))
                // Horizontal text alignment — see the paragraph node above.
                .attr("text_align", AttrSpec::optional("left"))
                // An app's `data-*` attributes, as on the paragraph.
                .data_attrs(true)
                .parse_html(vec![
                    "h1".into(),
                    "h2".into(),
                    "h3".into(),
                    "h4".into(),
                    "h5".into(),
                    "h6".into(),
                ])
                .build(),
        );

        // blockquote
        builder = builder.node(
            "blockquote",
            NodeSpec::builder("blockquote")
                .content("block+")
                .group("block")
                .defining(true)
                .parse_html(vec!["blockquote".into()])
                .build(),
        );

        // code_block
        builder = builder.node(
            "code_block",
            NodeSpec::builder("code_block")
                .content("text*")
                .group("block")
                .marks(MarkSet::None)
                .attr("language", AttrSpec::optional(""))
                .defining(true)
                .code(true)
                .parse_html(vec!["pre".into()])
                .build(),
        );

        // bullet_list
        builder = builder.node(
            "bullet_list",
            NodeSpec::builder("bullet_list")
                .content("list_item+")
                .group("block")
                .parse_html(vec!["ul".into()])
                .build(),
        );

        // ordered_list
        builder = builder.node(
            "ordered_list",
            NodeSpec::builder("ordered_list")
                .content("list_item+")
                .group("block")
                .attr("start", AttrSpec::optional(AttrValue::Int(1)))
                .parse_html(vec!["ol".into()])
                .build(),
        );

        // list_item
        builder = builder.node(
            "list_item",
            NodeSpec::builder("list_item")
                .content("block+")
                .defining(true)
                .parse_html(vec!["li".into()])
                .build(),
        );

        // task_list
        builder = builder.node(
            "task_list",
            NodeSpec::builder("task_list")
                .content("task_item+")
                .group("block")
                .build(),
        );

        // task_item
        builder = builder.node(
            "task_item",
            NodeSpec::builder("task_item")
                .content("block+")
                .defining(true)
                .attr("checked", AttrSpec::optional(AttrValue::Bool(false)))
                .build(),
        );

        // horizontal_rule
        builder = builder.node("horizontal_rule", {
            let mut spec = NodeSpec::atom("horizontal_rule");
            spec.group = Some("block".into());
            spec.parse_html_tags = vec!["hr".into()];
            spec
        });

        // embed — a block an app draws itself (a sheet, a canvas, a counter: a
        // node view). The document holds the atom and the app's own `data-*`
        // attributes, which are what the app keys its state to. It has no HTML
        // tag of its own: it is written as `<div data-pm-type="embed" data-…>`
        // and read back from that element (`serialize::html`), and an app can
        // give the HTML export markup to put inside it
        // (`serialize::set_html_fallback`).
        builder = builder.node("embed", {
            let mut spec = NodeSpec::atom("embed");
            spec.group = Some("block".into());
            spec.data_attrs = true;
            spec
        });

        // hard_break
        builder = builder.node("hard_break", {
            let mut spec = NodeSpec::atom("hard_break");
            spec.group = Some("inline".into());
            spec.inline = true;
            spec.parse_html_tags = vec!["br".into()];
            spec
        });

        // text
        builder = builder.node(
            "text",
            NodeSpec::builder("text").group("inline").inline().build(),
        );

        // image
        builder = builder.node("image", {
            let mut spec = NodeSpec::atom("image");
            spec.group = Some("inline".into());
            spec.inline = true;
            spec.attrs.insert("src".into(), AttrSpec::required());
            spec.attrs.insert("alt".into(), AttrSpec::optional(""));
            spec.attrs.insert("title".into(), AttrSpec::optional(""));
            // `width` is the width the picture is shown at, in whole CSS
            // pixels, at most `IMAGE_MAX_WIDTH` (absent or not positive: its
            // natural width — the editor's stylesheet does not cap it).
            spec.attrs.insert("width".into(), AttrSpec::optional(0i64));
            // An app's own `data-*` attributes (an annotation id, a reference
            // into its store): kept through edits, HTML, `DocNode` and
            // collaboration, and written on the host `<img>`. A GFM image
            // `![alt](src)` carries neither them nor `width`; an image in a
            // table written as HTML keeps both.
            spec.data_attrs = true;
            spec.parse_html_tags = vec!["img".into()];
            spec
        });

        // === Tables ===
        // A table is a block whose children are rows; each row holds cells; each
        // cell holds block content. Cells (`table_cell`/`table_header_cell`) share
        // a `cell` group so a row's content is simply `cell+` (the content-match
        // grammar has no alternation, so a group stands in for `(td | th)+`).

        // table > table_row+
        builder = builder.node(
            "table",
            NodeSpec::builder("table")
                .content("table_row+")
                .group("block")
                .parse_html(vec!["table".into()])
                .build(),
        );

        // table_row > cell*
        // Zero-or-more (not `cell+`): a row can be **empty** when every one of its
        // columns is covered by a `rowspan` cell from a row above (e.g. after merging
        // a full-width rectangle of cells). This matches ProseMirror's row content.
        builder = builder.node(
            "table_row",
            NodeSpec::builder("table_row")
                .content("cell*")
                .parse_html(vec!["tr".into()])
                .build(),
        );

        // table_cell > block+
        // Cells are `isolating`: edits (backspace/delete, lift/wrap, find-wrapping)
        // never cross a cell boundary — a cell is its own editing context, just as
        // in ProseMirror. `find_cut_before`/`find_cut_after` already honor this flag.
        builder = builder.node(
            "table_cell",
            NodeSpec::builder("table_cell")
                .content("block+")
                .group("cell")
                .isolating(true)
                .attr("colspan", AttrSpec::optional(AttrValue::Int(1)))
                .attr("rowspan", AttrSpec::optional(AttrValue::Int(1)))
                .parse_html(vec!["td".into()])
                .build(),
        );

        // table_header_cell > block+
        builder = builder.node(
            "table_header_cell",
            NodeSpec::builder("table_header_cell")
                .content("block+")
                .group("cell")
                .isolating(true)
                .attr("colspan", AttrSpec::optional(AttrValue::Int(1)))
                .attr("rowspan", AttrSpec::optional(AttrValue::Int(1)))
                .parse_html(vec!["th".into()])
                .build(),
        );

        // === Marks ===

        // bold
        let mut bold = MarkSpec::simple("bold");
        bold.parse_html_tags = vec!["strong".into(), "b".into()];
        builder = builder.mark("bold", bold);

        // italic
        let mut italic = MarkSpec::simple("italic");
        italic.parse_html_tags = vec!["em".into(), "i".into()];
        builder = builder.mark("italic", italic);

        // underline
        let mut underline = MarkSpec::simple("underline");
        underline.parse_html_tags = vec!["u".into()];
        builder = builder.mark("underline", underline);

        // strike
        let mut strike = MarkSpec::simple("strike");
        strike.parse_html_tags = vec!["s".into(), "del".into(), "strike".into()];
        builder = builder.mark("strike", strike);

        // code
        let mut code = MarkSpec::simple("code");
        code.excludes = Some("bold italic underline strike link".to_string());
        code.parse_html_tags = vec!["code".into()];
        builder = builder.mark("code", code);

        // link
        let mut link_attrs = BTreeMap::new();
        link_attrs.insert("href".into(), AttrSpec::required());
        link_attrs.insert("title".into(), AttrSpec::optional(""));
        link_attrs.insert("target".into(), AttrSpec::optional(""));
        let mut link = MarkSpec::with_attrs("link", link_attrs);
        link.parse_html_tags = vec!["a".into()];
        // Non-inclusive, as ProseMirror's own example schema and tiptap's Link have
        // it: text typed right after a link is not part of it.
        link.inclusive = false;
        builder = builder.mark("link", link);

        // highlight
        let mut hl_attrs = BTreeMap::new();
        hl_attrs.insert("color".into(), AttrSpec::optional(""));
        let mut highlight = MarkSpec::with_attrs("highlight", hl_attrs);
        highlight.parse_html_tags = vec!["mark".into()];
        builder = builder.mark("highlight", highlight);

        // text_color
        let mut tc_attrs = BTreeMap::new();
        tc_attrs.insert("color".into(), AttrSpec::required());
        let text_color = MarkSpec::with_attrs("text_color", tc_attrs);
        builder = builder.mark("text_color", text_color);

        // subscript
        let mut subscript = MarkSpec::simple("subscript");
        subscript.excludes = Some("superscript".to_string());
        subscript.parse_html_tags = vec!["sub".into()];
        builder = builder.mark("subscript", subscript);

        // superscript
        let mut superscript = MarkSpec::simple("superscript");
        superscript.excludes = Some("subscript".to_string());
        superscript.parse_html_tags = vec!["sup".into()];
        builder = builder.mark("superscript", superscript);

        builder.build()
    }
}
