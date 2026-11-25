//! Margin indent, text alignment and the paragraph reset.

use super::*;

#[test]
fn indent_and_outdent_a_paragraph() {
    let mut s = editor("hello");
    s.selection = Selection::cursor(Pos(2));
    // Indent bumps the paragraph's `indent` attr 0 → 1 → 2.
    let i1 = s.run("indent").expect("indent applies");
    assert_eq!(i1.doc.child(0).attrs().get_int("indent"), Some(1));
    let i2 = i1.run("indent").expect("indent applies again");
    assert_eq!(i2.doc.child(0).attrs().get_int("indent"), Some(2));
    // Outdent steps it back down to 0.
    let o1 = i2.run("outdent").expect("outdent applies");
    assert_eq!(o1.doc.child(0).attrs().get_int("indent"), Some(1));
    let o2 = o1.run("outdent").expect("outdent applies");
    assert_eq!(o2.doc.child(0).attrs().get_int("indent"), Some(0));
    // Outdent at indent 0 is a no-op (so the toolbar button can stay disabled).
    assert!(o2.run("outdent").is_none());
}

#[test]
fn set_text_align_on_a_paragraph() {
    let mut s = editor("hello");
    s.selection = Selection::cursor(Pos(2));
    // A fresh paragraph carries no explicit alignment (it reads as the "left"
    // default; the attr only materializes on the DocNode wire via `to_doc`).
    assert_eq!(s.doc.child(0).attrs().get_str("text_align"), None);
    // Setting an alignment mutates the attr; re-applying the same value is a no-op.
    let c = s
        .run("setTextAlignCenter")
        .expect("setTextAlignCenter applies");
    assert_eq!(c.doc.child(0).attrs().get_str("text_align"), Some("center"));
    assert!(
        c.run("setTextAlignCenter").is_none(),
        "re-selecting the current alignment must not apply"
    );
    // Switching to another alignment applies again.
    let r = c
        .run("setTextAlignRight")
        .expect("setTextAlignRight applies");
    assert_eq!(r.doc.child(0).attrs().get_str("text_align"), Some("right"));
    // Left stores an explicit `left`, and is then a no-op.
    let l = r.run("setTextAlignLeft").expect("setTextAlignLeft applies");
    assert_eq!(l.doc.child(0).attrs().get_str("text_align"), Some("left"));
    assert!(l.run("setTextAlignLeft").is_none());
}

/// An unset alignment is not `left` (#2210): align-left on a fresh
/// paragraph stores an explicit `left`, which then holds against a `th`'s
/// centring, an RTL editor and an app's rules.
#[test]
fn set_text_align_left_on_an_unset_paragraph_stores_left() {
    let mut s = editor("hello");
    s.selection = Selection::cursor(Pos(2));
    assert_eq!(s.doc.child(0).attrs().get_str("text_align"), None);
    let l = s
        .run("setTextAlignLeft")
        .expect("align-left on an unset alignment applies");
    assert_eq!(l.doc.child(0).attrs().get_str("text_align"), Some("left"));
    assert!(l.run("setTextAlignLeft").is_none());
}

#[test]
fn set_paragraph_resets_alignment() {
    let mut s = editor("centered");
    s.selection = Selection::cursor(Pos(2));
    let c = s.run("setTextAlignCenter").expect("center");
    assert_eq!(c.doc.child(0).attrs().get_str("text_align"), Some("center"));
    let p = c.run("setParagraph").expect("reset to normal text");
    assert_eq!(
        p.doc.child(0).attrs().get("text_align"),
        None,
        "the Paragraph reset unsets the alignment"
    );
    // An explicit `left` is formatting too, so the reset unsets it (#2210).
    let l = s.run("setTextAlignLeft").expect("left");
    let p = l.run("setParagraph").expect("reset to normal text");
    assert_eq!(p.doc.child(0).attrs().get("text_align"), None);
}

#[test]
fn outdent_does_not_unquote_a_blockquote() {
    let mut s = editor("quoted");
    s.selection = Selection::cursor(Pos(2));
    let bq = s.run("wrapInBlockquote").expect("wrap applies");
    assert!(
        bq.doc
            .content()
            .iter()
            .any(|n| n.type_name() == "blockquote")
    );
    // Outdent inside the (un-indented) blockquote must do nothing — not lift the
    // paragraph out of the quote (the old generic-`lift` surprise).
    assert!(bq.run("outdent").is_none());
}

#[test]
fn set_paragraph_unquotes_a_blockquote() {
    let mut s = editor("quoted");
    s.selection = Selection::cursor(Pos(2));
    let bq = s.run("wrapInBlockquote").expect("wrap applies");
    assert!(
        bq.doc
            .content()
            .iter()
            .any(|n| n.type_name() == "blockquote")
    );
    // The Paragraph button lifts the block out of the quote (un-quote).
    let out = bq.run("setParagraph").expect("setParagraph un-quotes");
    assert!(
        !out.doc
            .content()
            .iter()
            .any(|n| n.type_name() == "blockquote"),
        "blockquote should be gone"
    );
    assert_eq!(out.doc.child(0).type_name(), "paragraph");
    assert_eq!(all_text(out.doc.child(0)), "quoted");
}

#[test]
fn indent_first_list_item_is_a_noop() {
    let mut s = editor("a");
    s.selection = Selection::cursor(Pos(2));
    let list = s.run("toggleBulletList").expect("make a list");
    // The first (only) item has no previous sibling to nest under — indent must do
    // nothing, and in particular must NOT margin-indent the item's paragraph.
    assert!(
        list.run("indent").is_none(),
        "indent on the first list item is a no-op"
    );
}

#[test]
fn set_paragraph_resets_each_kind_of_formatting() {
    // (a) Inline marks are stripped.
    let mut s = editor("abc");
    s.selection = Selection::text(Pos(1), Pos(4));
    let bold = s.run("toggleBold").expect("bold applies");
    assert!(doc_has_mark(&bold.doc, "bold"));
    let plain = bold.run("setParagraph").expect("clears marks");
    assert!(!doc_has_mark(&plain.doc, "bold"), "marks stripped");

    // (b) Heading → paragraph.
    let mut s = editor("title");
    s.selection = Selection::cursor(Pos(2));
    let h = s.run("setHeading1").expect("h1 applies");
    assert_eq!(h.doc.child(0).type_name(), "heading");
    let p = h.run("setParagraph").expect("heading → paragraph");
    assert_eq!(p.doc.child(0).type_name(), "paragraph");

    // (c) List item → top-level paragraph (un-listed).
    let mut s = editor("item");
    s.selection = Selection::cursor(Pos(2));
    let list = s.run("toggleBulletList").expect("list applies");
    assert!(
        list.doc
            .content()
            .iter()
            .any(|n| n.type_name() == "bullet_list")
    );
    let p = list.run("setParagraph").expect("un-list");
    assert!(
        !p.doc
            .content()
            .iter()
            .any(|n| n.type_name() == "bullet_list"),
        "list removed"
    );
    assert_eq!(p.doc.child(0).type_name(), "paragraph");
    assert_eq!(all_text(p.doc.child(0)), "item");

    // (d) Indent reset to 0.
    let mut s = editor("ind");
    s.selection = Selection::cursor(Pos(2));
    let i = s.run("indent").expect("indent applies");
    assert_eq!(i.doc.child(0).attrs().get_int("indent"), Some(1));
    let p = i.run("setParagraph").expect("resets indent");
    assert_eq!(p.doc.child(0).attrs().get_int("indent"), Some(0));
}

#[test]
fn set_paragraph_resets_all_formatting_at_once() {
    // A bold heading wrapped in a blockquote — one Paragraph click clears it all.
    let mut s = editor("mixed");
    s.selection = Selection::text(Pos(1), Pos(6));
    let b = s.run("toggleBold").expect("bold");
    let h = b.run("setHeading1").expect("h1");
    let q = h.run("wrapInBlockquote").expect("quote");
    assert!(
        q.doc
            .content()
            .iter()
            .any(|n| n.type_name() == "blockquote")
    );
    assert!(doc_has_mark(&q.doc, "bold"));

    let p = q.run("setParagraph").expect("reset to normal text");
    assert!(
        !p.doc
            .content()
            .iter()
            .any(|n| n.type_name() == "blockquote"),
        "un-quoted"
    );
    assert_eq!(p.doc.child(0).type_name(), "paragraph", "is a paragraph");
    assert!(!doc_has_mark(&p.doc, "bold"), "marks stripped");
    assert_eq!(all_text(p.doc.child(0)), "mixed");
}

#[test]
fn set_paragraph_in_a_table_cell_resets_without_breaking_the_table() {
    // Paragraph reset inside a cell strips the heading/marks but must keep the cell's
    // block INSIDE the table — never lift it out (which would corrupt the table).
    let s = Rc::new(Schema::starter_kit());
    let cell = s
        .branch("table_cell", Fragment::from_node(p_of(&s, "c")))
        .unwrap();
    let table = s
        .branch(
            "table",
            Fragment::from_node(s.branch("table_row", Fragment::from_node(cell)).unwrap()),
        )
        .unwrap();
    let doc = s.branch("doc", Fragment::from_node(table)).unwrap();
    let mut state = EditorState::create(s, doc, default_plugins());
    state.selection = Selection::cursor(Pos(cursor_in_para(&state.doc, "c")));

    let h = state.run("setHeading1").expect("heading in the cell");
    let reset = h.run("setParagraph").expect("reset in the cell");
    // The table survives and the cell holds a plain paragraph again.
    let table = reset
        .doc
        .content()
        .iter()
        .find(|n| n.type_name() == "table")
        .expect("table preserved");
    let cell_block = table.child(0).child(0).child(0);
    assert_eq!(cell_block.type_name(), "paragraph");
    assert_eq!(all_text(cell_block), "c");
}

#[test]
fn set_paragraph_across_a_table_lifts_outside_blocks_but_keeps_the_table() {
    // A selection from a top-level blockquote INTO a table cell: the blockquote block
    // un-quotes to the top level, while the cell's block stays trapped in the table.
    let s = Rc::new(Schema::starter_kit());
    let bq = s
        .branch("blockquote", Fragment::from_node(p_of(&s, "q")))
        .unwrap();
    let cell = s
        .branch("table_cell", Fragment::from_node(p_of(&s, "c")))
        .unwrap();
    let table = s
        .branch(
            "table",
            Fragment::from_node(s.branch("table_row", Fragment::from_node(cell)).unwrap()),
        )
        .unwrap();
    let doc = s
        .branch("doc", Fragment::from_children(vec![bq, table]))
        .unwrap();
    let mut state = EditorState::create(s, doc, default_plugins());
    let from = cursor_in_para(&state.doc, "q");
    let to = cursor_in_para(&state.doc, "c");
    state.selection = Selection::text(Pos(from), Pos(to));

    let reset = state.run("setParagraph").expect("setParagraph applies");
    assert!(
        reset
            .doc
            .content()
            .iter()
            .all(|n| n.type_name() != "blockquote"),
        "blockquote lifted out to the top level"
    );
    let table = reset
        .doc
        .content()
        .iter()
        .find(|n| n.type_name() == "table")
        .expect("table preserved");
    let cell_block = table.child(0).child(0).child(0);
    assert_eq!(
        cell_block.type_name(),
        "paragraph",
        "cell stays in the table"
    );
    assert_eq!(all_text(cell_block), "c");
}
