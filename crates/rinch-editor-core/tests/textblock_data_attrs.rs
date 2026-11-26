//! A paragraph and a heading keep an app's `data-*` attributes: through the
//! commands that set them, every structural edit, copy and paste, `DocNode`
//! and Markdown's HTML tables. The rules are in `schema::data_attrs`.

use rinch_editor_core::commands::{
    block_data_attr, delete_char_backward, delete_char_forward, insert_horizontal_rule, lift,
    set_block_data_attr, set_block_type, set_paragraph, split_block, split_block_or_list_item,
    toggle_list, wrap_in,
};
use rinch_editor_core::schema::{MAX_DATA_ATTR_VALUE, MAX_DATA_ATTRS};
use rinch_editor_core::serialize::{
    CLIPBOARD_MARK, clipboard_slice, node_to_html, slice_from_html, slice_from_pasted_html,
    slice_from_text, slice_to_html,
};
use rinch_editor_core::{
    AttrValue, Attrs, Command, EditorState, Node, Pos, Schema, Selection, default_plugins,
};
use std::rc::Rc;

/// An editor over `html` (loaded, so every data attribute is read) with the
/// caret at `at`.
fn state(html: &str, at: usize) -> EditorState {
    let schema = Rc::new(Schema::starter_kit());
    let slice = slice_from_html(&schema, html).unwrap();
    let doc = schema.branch("doc", slice.content).unwrap();
    let s = EditorState::create(schema, doc, default_plugins());
    select(s, at, at)
}

fn select(s: EditorState, from: usize, to: usize) -> EditorState {
    let mut tr = s.tr();
    tr.set_selection(Selection::text(Pos(from), Pos(to)));
    s.apply(tr)
}

fn run(s: &EditorState, cmd: Command) -> EditorState {
    let mut out = None;
    let mut dispatch = |tr| out = Some(s.apply(tr));
    assert!(cmd(s, Some(&mut dispatch)), "the command applies");
    out.expect("a transaction")
}

fn applies(s: &EditorState, cmd: Command) -> bool {
    cmd(s, None)
}

/// The document as exported HTML.
fn html(s: &EditorState) -> String {
    node_to_html(&s.doc)
}

fn role(name: &str) -> Command {
    set_block_data_attr("data-role", Some(name))
}

const SUB: &str = r#"<p data-role="sub">abcd</p>"#;

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

// ── Enter ────────────────────────────────────────────────────────────────

#[test]
fn enter_at_the_end_makes_a_plain_paragraph() {
    for cmd in [split_block(), split_block_or_list_item()] {
        let s = run(&state(SUB, 5), cmd);
        assert_eq!(html(&s), r#"<p data-role="sub">abcd</p><p></p>"#);
    }
    // The paragraph's other attributes go on as they did.
    let s = run(
        &state(r#"<p style="text-align:center" data-role="sub">ab</p>"#, 3),
        split_block(),
    );
    assert_eq!(
        html(&s),
        r#"<p style="text-align:center" data-role="sub">ab</p><p style="text-align:center"></p>"#
    );
    // A heading's end already made a plain paragraph.
    let s = run(&state(r#"<h1 data-role="sub">ab</h1>"#, 3), split_block());
    assert_eq!(html(&s), r#"<h1 data-role="sub">ab</h1><p></p>"#);
}

#[test]
fn enter_at_the_start_leaves_a_plain_block_above() {
    let s = run(&state(SUB, 1), split_block());
    assert_eq!(html(&s), r#"<p></p><p data-role="sub">abcd</p>"#);
    // The caret stays with the text.
    assert_eq!(s.selection.from(), Pos(3));
    let s = run(&state(r#"<h2 data-role="sub">ab</h2>"#, 1), split_block());
    assert_eq!(html(&s), r#"<h2></h2><h2 data-role="sub">ab</h2>"#);
}

#[test]
fn enter_in_the_middle_keeps_them_on_both_halves() {
    let s = run(&state(SUB, 3), split_block());
    assert_eq!(
        html(&s),
        r#"<p data-role="sub">ab</p><p data-role="sub">cd</p>"#
    );
}

#[test]
fn enter_over_a_selection_follows_where_the_split_falls() {
    // To the end.
    let s = run(&select(state(SUB, 1), 3, 5), split_block());
    assert_eq!(html(&s), r#"<p data-role="sub">ab</p><p></p>"#);
    // From the start.
    let s = run(&select(state(SUB, 1), 1, 3), split_block());
    assert_eq!(html(&s), r#"<p></p><p data-role="sub">cd</p>"#);
}

/// An empty block is at its own end: the block keeps them, the new one has none.
#[test]
fn enter_in_an_empty_block_makes_a_plain_one_below() {
    let s = run(&state(r#"<p data-role="sub"></p>"#, 1), split_block());
    assert_eq!(html(&s), r#"<p data-role="sub"></p><p></p>"#);
}

#[test]
fn enter_in_a_list_item_follows_the_same_rules() {
    let list = r#"<ul><li><p data-role="sub">abcd</p></li></ul>"#;
    let enter = |at| html(&run(&state(list, at), split_block_or_list_item()));
    assert_eq!(
        enter(7),
        r#"<ul><li><p data-role="sub">abcd</p></li><li><p></p></li></ul>"#
    );
    assert_eq!(
        enter(3),
        r#"<ul><li><p></p></li><li><p data-role="sub">abcd</p></li></ul>"#
    );
    assert_eq!(
        enter(5),
        r#"<ul><li><p data-role="sub">ab</p></li><li><p data-role="sub">cd</p></li></ul>"#
    );
}

/// An empty item's Enter leaves the list, moving the block as it is.
#[test]
fn enter_in_an_empty_list_item_lifts_the_block_with_its_attributes() {
    let s = state(r#"<ul><li><p data-role="sub"></p></li></ul>"#, 3);
    let s = run(&s, split_block_or_list_item());
    assert_eq!(html(&s), r#"<p data-role="sub"></p>"#);
}

#[test]
fn a_rule_inserted_at_the_end_leaves_no_second_styled_block() {
    let s = run(&state(SUB, 5), insert_horizontal_rule());
    assert_eq!(html(&s).matches("data-role").count(), 1, "{}", html(&s));
}

// ── Joining ──────────────────────────────────────────────────────────────

const TWO: &str = r#"<p data-role="a">ab</p><p data-role="b">cd</p>"#;

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
}

#[test]
fn an_empty_block_removed_before_another_leaves_that_one_as_it_was() {
    for (doc, kept) in [
        (
            r#"<p data-role="a"></p><p data-role="b">cd</p>"#,
            r#"<p data-role="b">cd</p>"#,
        ),
        // Only one of the two has any.
        (r#"<p data-role="a"></p><p>cd</p>"#, "<p>cd</p>"),
        (
            r#"<p></p><p data-role="b">cd</p>"#,
            r#"<p data-role="b">cd</p>"#,
        ),
        // The type too: the block that stays is not retyped.
        (r#"<p data-role="a"></p><h2>cd</h2>"#, "<h2>cd</h2>"),
    ] {
        for (at, cmd) in [(3, delete_char_backward()), (1, delete_char_forward())] {
            let s = run(&state(doc, at), cmd);
            assert_eq!(html(&s), kept, "{doc}");
            assert_eq!(s.selection.from(), Pos(1), "{doc}");
        }
    }
}

/// The empty block after: it goes with its own.
#[test]
fn an_empty_block_removed_after_another_drops_only_its_own() {
    let doc = r#"<p data-role="a">ab</p><p data-role="b"></p>"#;
    let kept = r#"<p data-role="a">ab</p>"#;
    assert_eq!(html(&run(&state(doc, 5), delete_char_backward())), kept);
    assert_eq!(html(&run(&state(doc, 3), delete_char_forward())), kept);
}

// ── Block types, wrapping ────────────────────────────────────────────────

#[test]
fn a_change_of_type_keeps_them_where_the_new_type_does() {
    let heading = set_block_type("heading", Attrs::from_iter([("level", AttrValue::Int(2))]));
    let s = run(&state(SUB, 2), heading.clone());
    assert_eq!(html(&s), r#"<h2 data-role="sub">abcd</h2>"#);
    // Already that markup, attributes aside: nothing to do.
    assert!(!applies(&s, heading));
    let back = run(&s, set_block_type("paragraph", Attrs::new()));
    assert_eq!(html(&back), SUB);
    assert!(!applies(&back, set_block_type("paragraph", Attrs::new())));
    // A code block keeps none, so the way back has none.
    let code = run(&back, set_block_type("code_block", Attrs::new()));
    assert_eq!(html(&code), "<pre>abcd</pre>");
    let plain = run(&code, set_block_type("paragraph", Attrs::new()));
    assert_eq!(html(&plain), "<p>abcd</p>");
}

#[test]
fn a_change_of_type_keeps_each_blocks_own() {
    let s = select(state(TWO, 1), 2, 6);
    let s = run(
        &s,
        set_block_type("heading", Attrs::from_iter([("level", AttrValue::Int(1))])),
    );
    assert_eq!(
        html(&s),
        r#"<h1 data-role="a">ab</h1><h1 data-role="b">cd</h1>"#
    );
}

#[test]
fn the_paragraph_reset_removes_them() {
    let s = run(&state(SUB, 2), set_paragraph());
    assert_eq!(html(&s), "<p>abcd</p>");
    assert!(!applies(&s, set_paragraph()));
    let s = run(
        &state(r#"<blockquote><h2 data-role="sub">ab</h2></blockquote>"#, 3),
        set_paragraph(),
    );
    assert_eq!(html(&s), "<p>ab</p>");
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

// ── HTML, paste, DocNode, Markdown ───────────────────────────────────────

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

fn paste(s: &EditorState, clip: &str) -> EditorState {
    let slice = slice_from_pasted_html(s.schema(), clip).unwrap();
    let mut tr = s.tr();
    let (from, to) = (s.selection.from().0, s.selection.to().0);
    tr.replace_range(from, to, slice).unwrap();
    s.apply(tr)
}

fn copy(s: &EditorState) -> String {
    slice_to_html(&s.doc.slice(0, s.doc.content_size()).unwrap())
}

/// A slice cut from inside two blocks, as a selection's copy is.
fn open_copy(from: usize, to: usize) -> String {
    slice_to_html(&state(TWO, 1).doc.slice(from, to).unwrap())
}

#[test]
fn blocks_pasted_over_an_empty_line_arrive_as_themselves() {
    let empty = state("<p></p>", 1);
    let two = state(TWO, 1);
    assert_eq!(html(&paste(&empty, &copy(&two))), TWO);
    assert_eq!(html(&paste(&empty, &open_copy(1, 7))), TWO);
    let one = state(SUB, 1);
    assert_eq!(html(&paste(&empty, &copy(&one))), SUB);
}

/// Into a block's text, the first pasted block joins it and it keeps its own
/// attributes; the last takes the text after the caret and keeps its own.
#[test]
fn blocks_pasted_into_text_join_it() {
    let s = state(r#"<p data-role="t">wxyz</p>"#, 3);
    assert_eq!(
        html(&paste(&s, &open_copy(1, 7))),
        r#"<p data-role="t">wxab</p><p data-role="b">cdyz</p>"#
    );
    // A selection inside one block is inline content: no attributes travel.
    assert_eq!(
        html(&paste(&s, &open_copy(1, 3))),
        r#"<p data-role="t">wxabyz</p>"#
    );
}

/// At the start of a block's text it is still a paste into that text.
#[test]
fn a_paste_at_the_start_of_text_joins_that_block() {
    for doc in [
        r#"<p data-role="t">wxyz</p>"#,
        r#"<ul><li><p data-role="t">wxyz</p></li></ul>"#,
    ] {
        let at = if doc.starts_with("<ul") { 3 } else { 1 };
        let one = r#"<p data-role="a" data-rinch-clip="">X</p>"#;
        let out = html(&paste(&state(doc, at), one));
        assert_eq!(out, doc.replace("wxyz", "Xwxyz"));
    }
    let out = html(&paste(
        &state(r#"<p data-role="t">wxyz</p>"#, 1),
        &open_copy(1, 7),
    ));
    assert_eq!(out, r#"<p data-role="t">ab</p><p data-role="b">cdwxyz</p>"#);
}

/// A copy of all the text of one block takes the block along.
#[test]
fn a_copy_of_one_whole_block_carries_it() {
    let source = state(r#"<p data-role="a">abcd</p><p>z</p>"#, 1);
    let copy = |from, to| slice_to_html(&clipboard_slice(&source.doc, from, to).unwrap());
    assert_eq!(copy(2, 4), "bc");
    assert_eq!(copy(1, 4), "abc");
    let whole = copy(1, 5);
    assert_eq!(whole, r#"<p data-role="a" data-rinch-clip="">abcd</p>"#);
    for empty in ["<p></p>", "<h2></h2>"] {
        let out = html(&paste(&state(empty, 1), &whole));
        assert_eq!(out, r#"<p data-role="a">abcd</p>"#, "{empty}");
    }
    // Into text it is only its text.
    let out = html(&paste(&state("<p>wxyz</p>", 3), &whole));
    assert_eq!(out, "<p>wxabcdyz</p>");
}

#[test]
fn lines_of_plain_text_place_them_as_enter_does() {
    let lines = |at, to| {
        let s = select(state(SUB, 1), at, to);
        let mut tr = s.tr();
        let slice = slice_from_text(s.schema(), "1\n2\n3").unwrap();
        tr.replace_selection_with_lines(slice).unwrap();
        html(&s.apply(tr))
    };
    assert_eq!(
        lines(1, 1),
        r#"<p>1</p><p>2</p><p data-role="sub">3abcd</p>"#
    );
    assert_eq!(
        lines(3, 3),
        r#"<p data-role="sub">ab1</p><p>2</p><p data-role="sub">3cd</p>"#
    );
    assert_eq!(
        lines(5, 5),
        r#"<p data-role="sub">abcd1</p><p>2</p><p>3</p>"#
    );
    assert_eq!(lines(1, 3), r#"<p>1</p><p>2</p><p data-role="sub">3cd</p>"#);
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
