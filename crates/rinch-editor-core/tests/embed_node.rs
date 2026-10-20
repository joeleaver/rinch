//! The starter kit's `embed`: a block-level leaf atom that stands for content an
//! app draws itself (a sheet, a canvas, a counter — the node views of PR B). The
//! document holds only the atom and the app's `data-*` attributes; this file pins
//! the model half:
//!
//! - the spec: block group, leaf, atom, selectable, keeps app data attributes;
//! - HTML: written as `<div data-pm-type="embed" data-…></div>` and read back from
//!   that element (the default paste keeps the data attributes only from rinch's
//!   own copy-out, as for `image`), with the content after it intact;
//! - `DocNode` and Markdown (an HTML block) round trips;
//! - editing: a caret, Backspace, Delete and a node selection treat an `embed`
//!   exactly as they treat a `horizontal_rule` (a differential over the two).

use std::rc::Rc;

use rinch_editor_core::schema::{MAX_DATA_ATTRS, is_app_data_attr};
use rinch_editor_core::serialize::{
    CLIPBOARD_MARK, node_to_html, slice_from_html, slice_from_pasted_html, slice_to_html,
};
use rinch_editor_core::{
    AttrValue, Attrs, BaseCommandsPlugin, CursorMotion, EditorState, Fragment, Node, NodeSelection,
    Pos, Schema, Selection, Slice, resolve_cursor_motion,
};

fn embed(schema: &Schema, attrs: &[(&str, &str)]) -> Node {
    schema
        .create_node(
            "embed",
            Attrs::from_iter(attrs.iter().map(|(k, v)| (*k, AttrValue::from(*v)))),
            Fragment::empty(),
        )
        .expect("the starter kit has an `embed` node type")
}

fn para(schema: &Schema, text: &str) -> Node {
    let content = if text.is_empty() {
        Fragment::empty()
    } else {
        Fragment::from_node(schema.text(text).unwrap())
    };
    schema.branch("paragraph", content).unwrap()
}

fn doc(schema: &Schema, blocks: Vec<Node>) -> Node {
    schema
        .branch("doc", Fragment::from_children(blocks))
        .unwrap()
}

fn data(node: &Node) -> Vec<(String, String)> {
    node.attrs()
        .iter()
        .filter(|(k, _)| k.starts_with("data-"))
        .map(|(k, v)| (k.to_string(), v.as_str().unwrap_or("").to_string()))
        .collect()
}

#[test]
fn the_starter_kits_embed_is_a_selectable_block_leaf_atom_that_keeps_data_attrs() {
    let schema = Schema::starter_kit();
    let nt = schema.node_type("embed").expect("an `embed` node type");
    assert!(nt.is_block(), "block-level");
    assert!(nt.is_leaf(), "holds no content");
    assert!(nt.is_atom(), "one opaque unit");
    let spec = nt.spec();
    assert!(spec.selectable, "a click or an arrow node-selects it");
    assert!(spec.data_attrs, "it keeps an app's data-* attributes");
    assert_eq!(spec.group.as_deref(), Some("block"));
    // The doc and every block container take it.
    for parent in ["doc", "blockquote", "list_item", "table_cell"] {
        assert!(
            schema.allows_child(parent, "embed"),
            "{parent} takes an embed"
        );
    }
    // It keeps valid, unreserved app data attributes and nothing else.
    let kept = nt
        .compute_attrs(&Attrs::from_iter([
            ("data-sheet", AttrValue::from("s1")),
            ("data-pm-type", AttrValue::from("paragraph")),
            ("data-rid", AttrValue::from("3")),
            ("src", AttrValue::from("not declared")),
        ]))
        .unwrap();
    let names: Vec<&str> = kept.iter().map(|(k, _)| k).collect();
    assert_eq!(names, ["data-sheet"]);
}

/// The writer writes the element as a block with an end tag; the reader reads it
/// back, and the paragraph after it is still a paragraph after it. (Written as a
/// void element, `<div …>` with no end tag, the paragraph after it would be read
/// as the div's content.)
#[test]
fn an_embed_round_trips_through_html_with_what_follows_it() {
    let schema = Schema::starter_kit();
    let d = doc(
        &schema,
        vec![
            para(&schema, "before"),
            embed(&schema, &[("data-sheet", "s1"), ("data-view", "grid")]),
            para(&schema, "after"),
        ],
    );
    let html = node_to_html(&d);
    assert_eq!(
        html,
        r#"<p>before</p><div data-pm-type="embed" data-sheet="s1" data-view="grid"></div><p>after</p>"#
    );
    let back = slice_from_html(&schema, &html).unwrap();
    assert_eq!(back.content, d.content().clone(), "{back:?}");
}

#[test]
fn an_embed_with_no_data_attrs_round_trips_through_html() {
    let schema = Schema::starter_kit();
    let d = doc(&schema, vec![embed(&schema, &[]), para(&schema, "x")]);
    let html = node_to_html(&d);
    assert_eq!(html, r#"<div data-pm-type="embed"></div><p>x</p>"#);
    assert_eq!(
        slice_from_html(&schema, &html).unwrap().content,
        d.content().clone()
    );
}

/// The reader keeps what the model keeps: no reserved name, at most
/// `MAX_DATA_ATTRS` (the first ones), and a `div` without the marker is no embed.
#[test]
fn the_html_reader_reads_an_embed_only_from_its_marker_and_bounds_its_data_attrs() {
    let schema = Schema::starter_kit();
    let mut attrs = String::new();
    for i in 0..40 {
        attrs.push_str(&format!(r#" data-k{i:02}="v""#));
    }
    let html = format!(r#"<div data-pm-type="embed" data-rid="9" data-on="x"{attrs}></div>"#);
    let slice = slice_from_html(&schema, &html).unwrap();
    assert_eq!(slice.content.child_count(), 1);
    let e = slice.content.child(0);
    assert_eq!(e.type_name(), "embed");
    let names: Vec<String> = data(e).into_iter().map(|(k, _)| k).collect();
    assert_eq!(names.len(), MAX_DATA_ATTRS, "{names:?}");
    assert!(names.iter().all(|n| is_app_data_attr(n)), "{names:?}");
    assert!(
        names.contains(&"data-on".to_string()),
        "data-on is an app's name"
    );
    assert!(!names.contains(&"data-rid".to_string()));

    // A plain div is a container, not an embed; a marker naming another type is
    // not read as that type.
    for html in [
        r#"<div data-sheet="s1">text</div>"#,
        r#"<div data-pm-type="paragraph">text</div>"#,
        r#"<div data-pm-type="image"></div><p>text</p>"#,
    ] {
        let slice = slice_from_html(&schema, html).unwrap();
        assert!(
            slice
                .content
                .children()
                .iter()
                .all(|n| n.type_name() != "embed" && n.type_name() != "image"),
            "{html}: {slice:?}"
        );
    }
}

/// An embed's element is read wherever a block is: inside a list item, a quote,
/// a table cell, an inline element that holds it, a heading.
#[test]
fn an_embed_is_read_wherever_a_block_is() {
    let schema = Schema::starter_kit();
    let e = r#"<div data-pm-type="embed" data-k="v"></div>"#;
    for (html, path) in [
        (format!("<ul><li>{e}</li></ul>"), vec![0usize, 0, 0]),
        (format!("<blockquote>{e}</blockquote>"), vec![0, 0]),
        (
            format!("<table><tr><td>{e}</td></tr></table>"),
            vec![0, 0, 0, 0],
        ),
        (format!("<b>{e}</b>"), vec![0]),
        (format!("<h2>a{e}b</h2>"), vec![1]),
    ] {
        let slice = slice_from_html(&schema, &html).unwrap();
        let mut n = slice.content.child(path[0]).clone();
        for i in &path[1..] {
            n = n.child(*i).clone();
        }
        assert_eq!(n.type_name(), "embed", "{html}: {slice:?}");
        assert_eq!(n.attrs().get_str("data-k"), Some("v"), "{html}");
    }
}

/// The default paste keeps an embed's data attributes only from rinch's own
/// copy-out, exactly as it does an image's: another application's markup gives
/// an embed with none.
#[test]
fn the_default_paste_keeps_an_embeds_data_attrs_from_rinchs_own_copy_only() {
    let schema = Schema::starter_kit();
    let e = embed(&schema, &[("data-sheet", "s1")]);
    let slice = Slice::new(Fragment::from_node(e.clone()), 0, 0);
    let copied = slice_to_html(&slice);
    assert!(copied.contains(CLIPBOARD_MARK), "{copied}");
    let pasted = slice_from_pasted_html(&schema, &copied).unwrap();
    assert_eq!(pasted.content.child(0), &e);

    // Another application's element naming the type is not rinch's copy, so
    // the paste reads it as the plain `<div>` it is (below).
    let foreign = r#"<div data-pm-type="embed" data-sheet="s1"></div><p>x</p>"#;
    let pasted = slice_from_pasted_html(&schema, foreign).unwrap();
    assert!(
        pasted
            .content
            .children()
            .iter()
            .all(|n| n.type_name() != "embed"),
        "{pasted:?}"
    );
}

/// Every text in `n`, in order.
fn texts(n: &Node, out: &mut String) {
    if let Some(t) = n.text() {
        out.push_str(t);
        out.push('|');
    }
    for c in n.content().children() {
        texts(c, out);
    }
}

fn slice_text(slice: &Slice) -> (String, usize) {
    let mut out = String::new();
    let mut embeds = 0;
    fn count(n: &Node, k: &mut usize) {
        if n.type_name() == "embed" {
            *k += 1;
        }
        for c in n.content().children() {
            count(c, k);
        }
    }
    for c in slice.content.children() {
        texts(c, &mut out);
        count(c, &mut embeds);
    }
    (out, embeds)
}

/// Review of #1565, F2: the default paste reads a `<div data-pm-type>` as a
/// block atom only from rinch's own copy-out ([`CLIPBOARD_MARK`]). A page that
/// shows an app's export fallback (a static table) is copied from a browser
/// without the mark, and its text arrives as text; an unclosed element no
/// longer swallows what follows it.
#[test]
fn a_foreign_embed_element_pastes_as_its_content() {
    let schema = Schema::starter_kit();
    for (html, want) in [
        (
            r#"<div data-pm-type="embed" data-k="v">Hello <b>world</b></div><p>after</p>"#,
            "Hello |world|after|",
        ),
        (
            r#"<div data-pm-type="embed"><p>unclosed embed</p><p>rest of the doc</p>"#,
            "unclosed embed|rest of the doc|",
        ),
        (
            r#"<div data-pm-type="embed"><table><tr><td>cell</td></tr></table></div>"#,
            "cell|",
        ),
    ] {
        let (text, embeds) = slice_text(&slice_from_pasted_html(&schema, html).unwrap());
        assert_eq!((text.as_str(), embeds), (want, 0), "{html}");
    }
}

/// Review of #1565, F2: the copy-out marks every embed's element, so one with
/// no data attributes still pastes back as an embed.
#[test]
fn a_rinch_copy_of_an_embed_without_data_attrs_pastes_back_as_an_embed() {
    let schema = Schema::starter_kit();
    let e = embed(&schema, &[]);
    let slice = Slice::new(
        Fragment::from_children(vec![para(&schema, "a"), e.clone(), para(&schema, "b")]),
        0,
        0,
    );
    let copied = slice_to_html(&slice);
    assert!(copied.contains(CLIPBOARD_MARK), "{copied}");
    let pasted = slice_from_pasted_html(&schema, &copied).unwrap();
    assert_eq!(pasted.content.child(1), &e, "{copied}");
}

/// Review of #1565, F2: a load ([`slice_from_html`]) still reads the element as
/// the embed whatever is inside it — exported HTML carries the app's fallback
/// there and loads back as the same embed — and ignores that content.
#[test]
fn a_load_reads_a_marked_div_as_the_embed_and_ignores_its_content() {
    let schema = Schema::starter_kit();
    let html = r#"<div data-pm-type="embed" data-k="v"><p>fallback</p></div><p>after</p>"#;
    let (text, embeds) = slice_text(&slice_from_html(&schema, html).unwrap());
    assert_eq!((text.as_str(), embeds), ("after|", 1), "{html}");
}

/// Review of #1565, F5 (M12): only a `<div>` names a block atom; the marker on
/// any other element is an unknown attribute.
#[test]
fn the_marker_on_an_element_that_is_not_a_div_is_no_embed() {
    let schema = Schema::starter_kit();
    for html in [
        r#"<span data-pm-type="embed">x</span>"#,
        r#"<p data-pm-type="embed">x</p>"#,
        r#"<section data-pm-type="embed">x</section>"#,
    ] {
        let (text, embeds) = slice_text(&slice_from_html(&schema, html).unwrap());
        assert_eq!((text.as_str(), embeds), ("x|", 0), "{html}");
    }
}

#[cfg(feature = "serde")]
#[test]
fn an_embed_round_trips_through_doc_node() {
    let schema = Schema::starter_kit();
    let d = doc(
        &schema,
        vec![
            embed(&schema, &[("data-sheet", "s1"), ("data-n", "")]),
            para(&schema, "x"),
        ],
    );
    let wire = d.to_doc().unwrap();
    let back = schema.node_from_doc(&wire).unwrap();
    assert_eq!(back, d);
}

#[cfg(feature = "markdown")]
mod markdown {
    use super::*;
    use rinch_editor_core::serialize::{
        MarkdownError, doc_from_markdown, doc_from_markdown_strict, doc_to_markdown,
    };

    /// Markdown has no embed; the writer writes one as an HTML block, and both
    /// readers take it back, data attributes included.
    #[test]
    fn an_embed_round_trips_through_markdown_as_an_html_block() {
        let schema = Schema::starter_kit();
        let d = doc(
            &schema,
            vec![
                para(&schema, "before"),
                embed(
                    &schema,
                    &[("data-sheet", "s1"), ("data-note", "two\n\nlines & <more>")],
                ),
                para(&schema, "after"),
            ],
        );
        let md = doc_to_markdown(&d);
        assert!(md.contains(r#"<div data-pm-type="embed""#), "{md}");
        assert_eq!(doc_from_markdown_strict(&schema, &md).unwrap(), d, "{md}");
        assert_eq!(doc_from_markdown(&schema, &md).unwrap(), d, "{md}");
    }

    /// In a table written as HTML too.
    #[test]
    fn an_embed_in_a_table_round_trips_through_markdown() {
        let schema = Schema::starter_kit();
        let cell = schema
            .branch(
                "table_cell",
                Fragment::from_node(embed(&schema, &[("data-sheet", "s1")])),
            )
            .unwrap();
        let row = schema
            .branch("table_row", Fragment::from_node(cell))
            .unwrap();
        let table = schema.branch("table", Fragment::from_node(row)).unwrap();
        let d = doc(&schema, vec![table]);
        let md = doc_to_markdown(&d);
        assert_eq!(doc_from_markdown_strict(&schema, &md).unwrap(), d, "{md}");
    }

    /// Review of #1565, F5 (M10): strict refuses an embed in an HTML table whose
    /// data attributes are past the bounds the import keeps (more than
    /// `MAX_DATA_ATTRS`, or a value over `MAX_DATA_ATTR_VALUE`), rather than
    /// reading it with the excess silently dropped.
    #[test]
    fn strict_refuses_an_embed_in_a_table_past_the_data_attr_bounds() {
        use rinch_editor_core::schema::MAX_DATA_ATTR_VALUE;
        let schema = Schema::starter_kit();
        let many: String = (0..=MAX_DATA_ATTRS)
            .map(|i| format!(" data-a{i}=\"v\""))
            .collect();
        let long = format!(" data-a=\"{}\"", "x".repeat(MAX_DATA_ATTR_VALUE + 1));
        for attrs in [many, long] {
            let md = format!(
                "<table><tr><td><div data-pm-type=\"embed\"{attrs}></div></td></tr></table>\n"
            );
            match doc_from_markdown_strict(&schema, &md) {
                Err(MarkdownError::Unsupported { .. }) => {}
                other => panic!("{attrs:.60}: expected a refusal, got {other:?}"),
            }
        }
        // At the bound it is read, every attribute kept.
        let ok: String = (0..MAX_DATA_ATTRS)
            .map(|i| format!(" data-a{i}=\"v\""))
            .collect();
        let md =
            format!("<table><tr><td><div data-pm-type=\"embed\"{ok}></div></td></tr></table>\n");
        let d = doc_from_markdown_strict(&schema, &md).unwrap();
        let e = d.child(0).child(0).child(0).child(0).clone();
        assert_eq!((e.type_name(), data(&e).len()), ("embed", MAX_DATA_ATTRS));
    }

    /// Strict refuses what it would drop: a reserved data attribute, text inside
    /// the element.
    #[test]
    fn strict_refuses_an_embed_block_it_would_not_carry_whole() {
        let schema = Schema::starter_kit();
        for md in [
            "<div data-pm-type=\"embed\" data-rid=\"1\"></div>\n",
            "<div data-pm-type=\"embed\">some text</div>\n",
        ] {
            match doc_from_markdown_strict(&schema, md) {
                Err(MarkdownError::Unsupported { .. }) => {}
                other => panic!("{md:?}: expected a refusal, got {other:?}"),
            }
        }
    }
}

// ── editing: an embed is edited as a horizontal rule is ──────────────────────

/// `[p "ab"] [ATOM] [p "cd"]`: positions `0 p 1 a 2 b 3 /p 4 ATOM 5 p 6 c 7 d 8 /p 9`.
fn three(schema: &Schema, atom: &str) -> Node {
    let atom = if atom == "embed" {
        embed(schema, &[("data-sheet", "s1")])
    } else {
        schema.branch(atom, Fragment::empty()).unwrap()
    };
    doc(schema, vec![para(schema, "ab"), atom, para(schema, "cd")])
}

/// The document with each atom written as `ATOM`, and the selection.
fn shape(state: &EditorState) -> String {
    fn walk(n: &Node, out: &mut String) {
        match n.type_name() {
            "embed" | "horizontal_rule" => out.push_str("ATOM"),
            "text" => out.push_str(n.text().unwrap()),
            name => {
                out.push_str(name);
                out.push('(');
                for c in n.content().children() {
                    walk(c, out);
                    out.push(' ');
                }
                out.push(')');
            }
        }
    }
    let mut out = String::new();
    walk(&state.doc, &mut out);
    format!("{out} sel={:?}", state.selection)
}

fn state(schema: &Rc<Schema>, atom: &str, selection: Selection) -> EditorState {
    let mut st = EditorState::create(
        schema.clone(),
        three(schema, atom),
        vec![Rc::new(BaseCommandsPlugin)],
    );
    st.selection = selection;
    st
}

#[test]
fn an_embed_is_edited_exactly_as_a_horizontal_rule_is() {
    let schema = Rc::new(Schema::starter_kit());
    let node_sel = Selection::Node(NodeSelection {
        anchor: Pos(4),
        head: Pos(5),
    });
    let cases: Vec<(&str, Selection, &str)> = vec![
        (
            "deleteCharBackward",
            Selection::cursor(Pos(6)),
            "Backspace after",
        ),
        (
            "deleteCharForward",
            Selection::cursor(Pos(3)),
            "Delete before",
        ),
        ("deleteCharBackward", node_sel.clone(), "Backspace on it"),
        ("deleteCharForward", node_sel.clone(), "Delete on it"),
        ("deleteSelection", node_sel.clone(), "delete selection"),
        ("splitBlock", node_sel.clone(), "split on it"),
        ("enter", node_sel.clone(), "Enter on it"),
        ("wrapInBlockquote", node_sel.clone(), "quote it"),
        (
            "deleteWordBackward",
            Selection::cursor(Pos(6)),
            "Ctrl+Backspace after",
        ),
        ("selectAll", Selection::cursor(Pos(2)), "select all"),
    ];
    for (command, sel, what) in cases {
        let hr = state(&schema, "horizontal_rule", sel.clone()).run(command);
        let em = state(&schema, "embed", sel.clone()).run(command);
        assert_eq!(
            hr.as_ref().map(shape),
            em.as_ref().map(shape),
            "{what} ({command})"
        );
    }
    // A caret moved onto it from either side node-selects it.
    for atom in ["horizontal_rule", "embed"] {
        let d = three(&schema, atom);
        for (head, motion) in [(3, CursorMotion::CharRight), (6, CursorMotion::CharLeft)] {
            let got = resolve_cursor_motion(&d, Pos(head), Pos(head), motion, false);
            assert_eq!(got, Some(node_sel.clone()), "{atom} {motion:?}");
        }
    }
    // The node selection selects the embed itself, attrs and all.
    let d = three(&schema, "embed");
    assert_eq!(
        node_sel.node(&d).map(|n| data(&n)),
        Some(vec![("data-sheet".to_string(), "s1".to_string())])
    );
}
