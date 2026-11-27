//! Where a textblock's `data-*` attributes go when HTML is pasted. The rules
//! are in `schema::data_attrs`.

mod data_attr_common;

use data_attr_common::*;
use rinch_editor_core::Slice;
use rinch_editor_core::commands::delete_char_backward;
use rinch_editor_core::serialize::{clipboard_slice, slice_to_html};

#[test]
fn blocks_pasted_over_an_empty_line_arrive_as_themselves() {
    let empty = state("<p></p>", 1);
    let two = state(TWO, 1);
    assert_eq!(html(&paste(&empty, &copy(&two))), TWO);
    assert_eq!(html(&paste(&empty, &open_copy(1, 7))), TWO);
    let one = state(SUB, 1);
    assert_eq!(html(&paste(&empty, &copy(&one))), SUB);
}

/// Into a block's text, the pasted content joins it: both pieces of the block
/// keep its attributes.
#[test]
fn blocks_pasted_into_text_join_it() {
    let s = state(r#"<p data-role="t">wxyz</p>"#, 3);
    assert_eq!(
        html(&paste(&s, &open_copy(1, 7))),
        r#"<p data-role="t">wxab</p><p data-role="t">cdyz</p>"#
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
    let paragraph = r#"<p data-role="a" data-rinch-clip="">X</p>"#;
    let heading = r#"<h2 data-role="a" data-rinch-clip="">X</h2>"#;
    for doc in [
        r#"<p data-role="t">wxyz</p>"#,
        r#"<ul><li><p data-role="t">wxyz</p></li></ul>"#,
    ] {
        let at = if doc.starts_with("<ul") { 3 } else { 1 };
        let out = html(&paste(&state(doc, at), paragraph));
        assert_eq!(out, doc.replace("wxyz", "Xwxyz"));
    }
    // A pasted heading makes the line a heading, as an unmarked one does: it
    // arrives with its own, the paragraph's being gone with the paragraph.
    let out = html(&paste(&state(r#"<p data-role="t">wxyz</p>"#, 1), heading));
    assert_eq!(out, r#"<h2 data-role="a">Xwxyz</h2>"#);
    let heading = r#"<h2 data-role="a" data-rinch-clip="">X</h2>"#;
    let out = html(&paste(&state("<p></p>", 1), heading));
    assert_eq!(out, r#"<h2 data-role="a">X</h2>"#);
    let out = html(&paste(
        &state(r#"<p data-role="t">wxyz</p>"#, 1),
        &open_copy(1, 7),
    ));
    assert_eq!(out, r#"<p data-role="a">ab</p><p data-role="t">cdwxyz</p>"#);
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
    let out = html(&paste(&state("<p></p>", 1), &whole));
    assert_eq!(out, r#"<p data-role="a">abcd</p>"#);
    // The line stays what it is, as it does for an unmarked block, and a
    // paragraph's attributes are not a heading's.
    let out = html(&paste(&state("<h2></h2>", 1), &whole));
    assert_eq!(out, "<h2>abcd</h2>");
    // Into text it is only its text.
    let out = html(&paste(&state("<p>wxyz</p>", 3), &whole));
    assert_eq!(out, "<p>wxabcdyz</p>");
}

/// A paste into the middle of a block leaves both pieces its attributes, as a
/// browser does; at an end, only the piece with its text.
#[test]
fn a_paste_inside_a_block_marks_both_pieces() {
    let foreign = "<p>1</p><p>2</p>";
    assert_eq!(
        html(&paste(&state(SUB, 3), foreign)),
        r#"<p data-role="sub">ab1</p><p data-role="sub">2cd</p>"#
    );
    assert_eq!(
        html(&paste(&state(SUB, 5), foreign)),
        r#"<p data-role="sub">abcd1</p><p>2</p>"#
    );
    assert_eq!(
        html(&paste(&state(SUB, 1), foreign)),
        r#"<p>1</p><p data-role="sub">2abcd</p>"#
    );
    // The same as the same lines pasted as plain text, inside the block.
    assert_eq!(
        html(&paste(&state(SUB, 3), foreign)),
        html(&paste_text(&state(SUB, 3), "1\n2"))
    );
}

/// Over two blocks, the text left of the second keeps the second's.
#[test]
fn a_paste_over_two_blocks_leaves_each_texts_own() {
    let s = select(state(TWO, 1), 2, 6);
    assert_eq!(
        html(&paste(&s, "<p>1</p><p>2</p>")),
        r#"<p data-role="a">a1</p><p data-role="b">2d</p>"#
    );
}

/// Marked blocks over all of a heading: the heading stays one, as it does for
/// unmarked ones, and takes nothing from a paragraph.
#[test]
fn blocks_over_a_whole_heading_leave_it_a_heading() {
    let s = select(state("<h2>wxyz</h2>", 1), 1, 5);
    assert_eq!(
        html(&paste(&s, &copy(&state(TWO, 1)))),
        r#"<h2>ab</h2><p data-role="b">cd</p>"#
    );
    // Its own it keeps.
    let s = select(state(r#"<h2 data-role="h">wxyz</h2>"#, 1), 1, 5);
    assert_eq!(
        html(&paste(&s, &copy(&state(TWO, 1)))),
        r#"<h2 data-role="h">ab</h2><p data-role="b">cd</p>"#
    );
}

/// A closed slice's block that takes an empty line's place keeps its own, as a
/// pasted one does.
#[test]
fn a_closed_block_over_an_empty_line_keeps_its_own() {
    let s = state(r#"<p data-role="line"></p>"#, 1);
    let source = state(SUB, 1);
    let closed = Slice::from_fragment(source.doc.content().clone());
    let mut tr = s.tr();
    tr.replace_selection(closed).unwrap();
    assert_eq!(html(&s.apply(tr)), SUB);
}

/// A single unmarked block over a marked empty line leaves the line's.
#[test]
fn an_unmarked_block_over_an_empty_marked_line_leaves_the_lines() {
    let s = state(r#"<p data-role="sub"></p>"#, 1);
    assert_eq!(html(&paste(&s, "<p>X</p>")), r#"<p data-role="sub">X</p>"#);
}

#[test]
fn undo_takes_a_paste_back_in_one_step() {
    let start = state(SUB, 3);
    let s = paste(&start, &copy(&state(TWO, 1)));
    let undo = s.command("undo").expect("history");
    assert_eq!(run(&s, undo).doc, start.doc);
    let s = run(&select(s, 1, 2), delete_char_backward());
    assert!(html(&s).starts_with("<p"), "{}", html(&s));
}
