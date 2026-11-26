//! A paragraph and a heading keep an app's `data-*` attributes: through the
//! commands that set them, joins, changes of type, HTML, `DocNode` and
//! Markdown's HTML tables. Splits are in `textblock_data_attrs_split`, pastes
//! in `textblock_data_attrs_paste`. The rules are in `schema::data_attrs`.

mod data_attr_common;

use data_attr_common::*;
use rinch_editor_core::commands::{
    block_data_attr, delete_char_backward, delete_char_forward, lift, set_block_data_attr,
    set_block_type, set_paragraph, split_block_or_list_item, toggle_list, wrap_in,
};
use rinch_editor_core::schema::{MAX_DATA_ATTR_VALUE, MAX_DATA_ATTRS};
use rinch_editor_core::serialize::{
    CLIPBOARD_MARK, node_to_html, slice_from_html, slice_from_pasted_html, slice_to_html,
};
use rinch_editor_core::{AttrValue, Attrs, Command, EditorState, Node, Pos, Schema, Selection};

#[test]
fn the_command_sets_changes_and_removes_it_on_the_carets_block() {
    let s = state("<p>ab</p><p>cd</p>", 2);
    let s = run(&s, role("sub"));
    assert_eq!(html(&s), r#"<p data-role="sub">ab</p><p>cd</p>"#);
    assert_eq!(block_data_attr(&s, "data-role").as_deref(), Some("sub"));
    // Setting it again changes nothing.
    assert!(!applies(&s, role("sub")));
    let s = run(&s, role("lead"));
    assert_eq!(html(&s), r#"<p data-role="lead">ab</p><p>cd</p>"#);
    let s = run(&s, set_block_data_attr("data-role", None::<String>));
    assert_eq!(html(&s), "<p>ab</p><p>cd</p>");
    assert!(!applies(
        &s,
        set_block_data_attr("data-role", None::<String>)
    ));
}

#[test]
fn the_command_reaches_every_textblock_of_a_selection_that_keeps_them() {
    let s = state(
        "<p>ab</p><pre>x</pre><blockquote><h2>cd</h2></blockquote>",
        1,
    );
    let end = s.doc.content_size() - 2;
    let s = select(s, 2, end);
    let s = run(&s, role("sub"));
    assert_eq!(
        html(&s),
        r#"<p data-role="sub">ab</p><pre>x</pre><blockquote><h2 data-role="sub">cd</h2></blockquote>"#
    );
    // A code block keeps none and does not spoil the shared value.
    assert_eq!(block_data_attr(&s, "data-role").as_deref(), Some("sub"));
    let one = run(&select(s, 2, 2), role("lead"));
    let end = one.doc.content_size() - 2;
    assert_eq!(block_data_attr(&select(one, 2, end), "data-role"), None);
}

#[test]
fn the_command_reaches_the_cells_of_a_cell_selection() {
    let s = state(
        "<table><tr><td><p>a</p></td><td><p>b</p></td></tr></table>",
        4,
    );
    // The two cells, by the positions before them.
    let second = 2 + s.doc.child(0).child(0).child(0).node_size();
    let mut tr = s.tr();
    tr.set_selection(Selection::cell(Pos(2), Pos(second)));
    let s = s.apply(tr);
    let s = run(&s, role("sub"));
    assert_eq!(
        html(&s).matches(r#"data-role="sub""#).count(),
        2,
        "{}",
        html(&s)
    );
}

#[test]
fn the_command_is_inapplicable_where_nothing_can_keep_it() {
    let s = state("<pre>x</pre>", 1);
    assert!(!applies(&s, role("sub")));
    assert_eq!(block_data_attr(&s, "data-role"), None);
}

#[test]
fn a_refused_name_or_value_changes_nothing() {
    let s = state("<p>ab</p>", 1);
    for name in ["role", "data-", "data-Role", "data-rid", "data-pm-type"] {
        assert!(!applies(&s, set_block_data_attr(name, Some("x"))), "{name}");
        let mut tr = s.tr();
        assert!(tr.set_block_data_attr(1, 1, name, Some("x")).is_err());
        assert!(!tr.doc_changed());
    }
    let long = "x".repeat(MAX_DATA_ATTR_VALUE + 1);
    assert!(!applies(&s, set_block_data_attr("data-a", Some(long))));
    let mut tr = s.tr();
    assert!(tr.set_block_data_attr(9, 99, "data-a", Some("x")).is_err());
}

/// A block holding the most it may refuses one more, rather than dropping
/// another; it still changes and removes the ones it has.
#[test]
fn a_full_block_refuses_one_more() {
    let mut s = state("<p>ab</p>", 1);
    for i in 0..MAX_DATA_ATTRS {
        s = run(&s, set_block_data_attr(format!("data-k{i:02}"), Some("v")));
    }
    assert!(!applies(&s, set_block_data_attr("data-a", Some("v"))));
    let s = run(&s, set_block_data_attr("data-k00", Some("w")));
    let s = run(&s, set_block_data_attr("data-k01", None::<String>));
    assert!(applies(&s, set_block_data_attr("data-a", Some("v"))));
    assert_eq!(s.doc.child(0).attrs().len(), MAX_DATA_ATTRS - 1);
}

#[test]
fn undo_takes_the_attribute_back() {
    let s = state("<p>ab</p>", 1);
    let set = run(&s, role("sub"));
    let undo = set.command("undo").expect("history");
    assert_eq!(run(&set, undo).doc, s.doc);
}

// ── Joining ──────────────────────────────────────────────────────────────

#[test]
fn joining_keeps_the_first_blocks_attributes() {
    let joined = r#"<p data-role="a">abcd</p>"#;
    assert_eq!(html(&run(&state(TWO, 5), delete_char_backward())), joined);
    assert_eq!(html(&run(&state(TWO, 3), delete_char_forward())), joined);
    // Deleting across the boundary.
    let s = select(state(TWO, 1), 2, 6);
    assert_eq!(
        html(&run(&s, delete_char_backward())),
        r#"<p data-role="a">ad</p>"#
    );
    // An empty block after goes with its own.
    let doc = r#"<p data-role="a">ab</p><p data-role="b"></p>"#;
    let kept = r#"<p data-role="a">ab</p>"#;
    assert_eq!(html(&run(&state(doc, 5), delete_char_backward())), kept);
    assert_eq!(html(&run(&state(doc, 3), delete_char_forward())), kept);
}

/// With none of the first block's text left, the attributes are those of the
/// text that is.
#[test]
fn a_join_that_keeps_only_the_seconds_text_has_the_seconds() {
    for (doc, joined) in [
        (
            r#"<p data-role="a"></p><p data-role="b">cd</p>"#,
            r#"<p data-role="b">cd</p>"#,
        ),
        (r#"<p data-role="a"></p><p>cd</p>"#, "<p>cd</p>"),
        (
            r#"<h2 data-role="a"></h2><h2 data-role="b">cd</h2>"#,
            r#"<h2 data-role="b">cd</h2>"#,
        ),
    ] {
        let at = 3;
        assert_eq!(html(&run(&state(doc, at), delete_char_backward())), joined);
        assert_eq!(html(&run(&state(doc, 1), delete_char_forward())), joined);
    }
    // A selection from the start of the first into the second, deleted or
    // typed over.
    let s = select(state(TWO, 1), 1, 6);
    assert_eq!(
        html(&run(&s, delete_char_backward())),
        r#"<p data-role="b">d</p>"#
    );
    let mut tr = s.tr();
    tr.insert_text("Z").unwrap();
    assert_eq!(html(&s.apply(tr)), r#"<p data-role="b">Zd</p>"#);
    // Neither has any text left: the first block's.
    let both = r#"<p data-role="a"></p><p data-role="b"></p>"#;
    assert_eq!(
        html(&run(&state(both, 3), delete_char_backward())),
        r#"<p data-role="a"></p>"#
    );
}

/// A join never gives a block another element's attributes: the second's go
/// with its text only into a block of the second's element.
#[test]
fn a_join_into_another_element_takes_none_of_the_seconds() {
    for (doc, joined) in [
        (r#"<h2></h2><p data-role="b">cd</p>"#, "<h2>cd</h2>"),
        (r#"<h1></h1><p data-role="b">cd</p>"#, "<h1>cd</h1>"),
        (
            r#"<h2 data-role="a"></h2><p data-role="b">cd</p>"#,
            "<h2>cd</h2>",
        ),
        // Two heading levels are two elements.
        (r#"<h1></h1><h2 data-role="b">cd</h2>"#, "<h1>cd</h1>"),
        (
            r#"<p data-role="a"></p><h2 data-role="b">cd</h2>"#,
            "<p>cd</p>",
        ),
    ] {
        let at = 3;
        assert_eq!(
            html(&run(&state(doc, at), delete_char_backward())),
            joined,
            "Backspace in {doc}"
        );
        assert_eq!(
            html(&run(&state(doc, 1), delete_char_forward())),
            joined,
            "Delete in {doc}"
        );
    }
    // From a heading's start into a paragraph's middle: deleted, typed over
    // and pasted over with a line of plain text.
    let doc = r#"<h2 data-role="a">ab</h2><p data-role="b">cd</p>"#;
    let s = select(state(doc, 1), 1, 6);
    assert_eq!(html(&run(&s, delete_char_backward())), "<h2>d</h2>");
    let mut tr = s.tr();
    tr.insert_text("Z").unwrap();
    assert_eq!(html(&s.apply(tr)), "<h2>Zd</h2>");
    assert_eq!(html(&paste_text(&s, "Z")), "<h2>Zd</h2>");
}

/// The attributes never decide the shape: every join, deletion, paste and
/// Enter makes the blocks it makes without them.
#[test]
fn the_shape_is_the_unmarked_documents() {
    let docs = [
        r#"<h2></h2><p data-role="b">cd</p>"#,
        r#"<p data-role="a"></p><h2>cd</h2>"#,
        r#"<h2 data-role="a">ab</h2><p data-role="b">cd</p>"#,
        r#"<p data-role="a">ab</p><h2 data-role="b"></h2>"#,
        r#"<ul><li><p data-role="a">ab</p></li></ul><p data-role="b">cd</p>"#,
        r#"<blockquote><p data-role="a"></p></blockquote><h1 data-role="b">cd</h1>"#,
    ];
    let clips = [
        r#"<p data-role="x" data-rinch-clip="">1</p>"#,
        r#"<h3 data-role="x" data-rinch-clip="">1</h3>"#,
        r#"<p data-role="x" data-rinch-clip="">1</p><p data-role="y" data-rinch-clip="">2</p>"#,
        r#"<h3 data-role="x" data-rinch-clip="">1</h3><p data-role="y" data-rinch-clip="">2</p>"#,
    ];
    for doc in docs {
        let plain = shape(doc);
        let size = state(doc, 1).doc.content_size();
        let carets: Vec<usize> = (0..=size)
            .filter(|&p| {
                let s = state(doc, 1);
                s.doc
                    .resolve(Pos(p))
                    .is_ok_and(|r| r.parent().is_textblock())
            })
            .collect();
        for &from in &carets {
            for &to in carets.iter().filter(|&&to| to >= from) {
                let check = |what: &str, edit: &dyn Fn(&EditorState) -> EditorState| {
                    let marked = edit(&select(state(doc, 1), from, to));
                    let unmarked = edit(&select(state(&plain, 1), from, to));
                    assert_eq!(
                        shape(&html(&marked)),
                        html(&unmarked),
                        "{what} {from}..{to} of {doc}"
                    );
                };
                let command = |s: &EditorState, cmd: Command| {
                    let mut out = s.clone();
                    cmd(s, Some(&mut |tr| out = s.apply(tr)));
                    out
                };
                check("Backspace", &|s| command(s, delete_char_backward()));
                check("Delete", &|s| command(s, delete_char_forward()));
                check("Enter", &|s| command(s, split_block_or_list_item()));
                check("lines", &|s| paste_text(s, "1\n2"));
                check("empty first line", &|s| paste_text(s, "\n2"));
                for clip in clips {
                    check(clip, &|s| paste(s, &shape_of_clip(clip, s)));
                }
            }
        }
    }
}

/// `clip` as it is for a marked document, and without its attributes for an
/// unmarked one.
fn shape_of_clip(clip: &str, s: &EditorState) -> String {
    if html(s).contains("data-role") {
        clip.to_string()
    } else {
        shape(clip)
    }
}

// ── Block types, wrapping ────────────────────────────────────────────────

fn heading(level: i64) -> Command {
    set_block_type(
        "heading",
        Attrs::from_iter([("level", AttrValue::Int(level))]),
    )
}

/// A change of element drops them, as a browser's `formatBlock` does.
#[test]
fn a_change_of_element_drops_them() {
    let s = run(&state(SUB, 2), heading(2));
    assert_eq!(html(&s), "<h2>abcd</h2>");
    let marked = state(r#"<h2 data-role="sub">ab</h2>"#, 2);
    assert_eq!(html(&run(&marked, heading(1))), "<h1>ab</h1>");
    assert_eq!(
        html(&run(&marked, set_block_type("paragraph", Attrs::new()))),
        "<p>ab</p>"
    );
    assert_eq!(
        html(&run(
            &state(SUB, 2),
            set_block_type("code_block", Attrs::new())
        )),
        "<pre>abcd</pre>"
    );
}

#[test]
fn a_block_that_stays_the_same_element_keeps_them() {
    let s = state(SUB, 2);
    assert!(!applies(&s, set_block_type("paragraph", Attrs::new())));
    assert!(!applies(&s, set_paragraph()));
    let marked = state(r#"<h2 data-role="sub">ab</h2>"#, 2);
    assert!(!applies(&marked, heading(2)));
    // One block of the selection changes element and one does not.
    let doc = r#"<p data-role="a">ab</p><h1 data-role="b">cd</h1>"#;
    let s = run(&select(state(doc, 1), 2, 6), heading(1));
    assert_eq!(html(&s), r#"<h1>ab</h1><h1 data-role="b">cd</h1>"#);
    // The reset takes the alignment and leaves the attributes of a paragraph.
    let aligned = state(r#"<p style="text-align:center" data-role="sub">ab</p>"#, 2);
    assert_eq!(
        html(&run(&aligned, set_paragraph())),
        r#"<p data-role="sub">ab</p>"#
    );
    let quoted = state(r#"<blockquote><h2 data-role="sub">ab</h2></blockquote>"#, 3);
    assert_eq!(html(&run(&quoted, set_paragraph())), "<p>ab</p>");
}

#[test]
fn wrapping_and_lifting_move_the_block_as_it_is() {
    let s = run(&state(SUB, 2), wrap_in("blockquote"));
    assert_eq!(html(&s), format!("<blockquote>{SUB}</blockquote>"));
    assert_eq!(html(&run(&s, lift())), SUB);
    let s = run(&state(SUB, 2), toggle_list("bullet_list"));
    assert_eq!(html(&s), format!("<ul><li>{SUB}</li></ul>"));
    assert_eq!(html(&run(&s, toggle_list("bullet_list"))), SUB);
}

// ── HTML, DocNode, Markdown ──────────────────────────────────────────────

#[test]
fn html_round_trips_them_on_paragraphs_and_headings() {
    let source = concat!(
        r#"<h3 style="text-align:center" data-id="a&amp;b" data-role="">t</h3>"#,
        r#"<ul><li><p data-role="x">y</p></li></ul>"#,
        r#"<table><tr><td><p data-role="c">z</p></td></tr></table>"#,
    );
    let s = state(source, 1);
    let slice = slice_from_html(s.schema(), &html(&s)).unwrap();
    assert_eq!(s.schema().branch("doc", slice.content).unwrap(), s.doc);
    assert_eq!(s.doc.child(0).attrs().get_str("data-id"), Some("a&b"));
    assert_eq!(s.doc.child(0).attrs().get_str("data-role"), Some(""));
    assert_eq!(html(&s), source);
}

#[test]
fn the_default_paste_keeps_them_from_rinchs_own_copy_only() {
    let s = state(TWO, 1);
    let copied = slice_to_html(&s.doc.slice(0, s.doc.content_size()).unwrap());
    assert!(copied.contains(CLIPBOARD_MARK), "{copied}");
    assert!(!html(&s).contains(CLIPBOARD_MARK));
    let schema = Schema::starter_kit();
    let pasted = slice_from_pasted_html(&schema, &copied).unwrap();
    assert_eq!(
        pasted.content.child(0).attrs().get_str("data-role"),
        Some("a")
    );
    assert_eq!(pasted.content.child(0).attrs().get(CLIPBOARD_MARK), None);
    // Another application's.
    let foreign = r#"<p data-v-1="" data-reactid=".0">a</p><h1 data-sourcepos="1:1">b</h1>"#;
    let pasted = slice_from_pasted_html(&schema, foreign).unwrap();
    for block in pasted.content.children() {
        assert!(
            block.attrs().iter().all(|(k, _)| !k.starts_with("data-")),
            "{block:?}"
        );
    }
    // A reserved name is never read.
    let loaded = slice_from_html(&schema, r#"<p data-rid="3" data-pm-type="x">a</p>"#).unwrap();
    assert!(loaded.content.child(0).attrs().is_empty());
}

#[test]
fn an_element_holding_blocks_gives_its_attributes_to_none_of_them() {
    let s = state(r#"<h1 data-role="x">a<div>b</div>c</h1>"#, 1);
    assert!(!html(&s).contains("data-role"), "{}", html(&s));
}

#[cfg(feature = "serde")]
#[test]
fn doc_node_round_trips_them() {
    let s = state(TWO, 1);
    let schema = s.schema();
    let back = schema.node_from_doc(&s.doc.to_doc().unwrap()).unwrap();
    assert_eq!(back, s.doc);
}

/// An app's own textblock that opts in keeps them through HTML too, a code one
/// included.
#[test]
fn an_apps_own_code_block_that_opts_in_keeps_them_through_html() {
    let kit = Schema::starter_kit();
    let mut b = Schema::builder();
    for (name, spec) in &kit.nodes {
        let mut spec = spec.clone();
        spec.data_attrs |= name == "code_block";
        b = b.node(name.clone(), spec);
    }
    let schema = b.build();
    let slice = slice_from_html(&schema, r#"<pre data-role="c">x</pre>"#).unwrap();
    let block: Node = slice.content.child(0).clone();
    assert_eq!(block.attrs().get_str("data-role"), Some("c"));
    assert_eq!(node_to_html(&block), r#"<pre data-role="c">x</pre>"#);
}

#[cfg(feature = "markdown")]
mod markdown {
    use super::*;
    use rinch_editor_core::serialize::{doc_from_markdown_strict, doc_to_markdown};

    /// Markdown has no syntax for them.
    #[test]
    fn markdown_writes_none() {
        let s = state(
            r#"<h1 data-role="t">a</h1><p data-role="sub">b</p><ul><li><p data-role="x">c</p></li></ul>"#,
            1,
        );
        let md = doc_to_markdown(&s.doc);
        assert!(!md.contains("data-"), "{md}");
    }

    /// A table written as HTML keeps them, and the strict reader takes it.
    #[test]
    fn an_html_table_keeps_them() {
        let source = r#"<table><tr><td colspan="2"><p data-role="c">z</p></td></tr><tr><td><p>a</p></td><td><p>b</p></td></tr></table>"#;
        let s = state(source, 1);
        let md = doc_to_markdown(&s.doc);
        assert!(md.contains(r#"<p data-role="c">"#), "{md}");
        let back = doc_from_markdown_strict(s.schema(), &md).unwrap();
        assert_eq!(node_to_html(&back), html(&s));
    }
}
