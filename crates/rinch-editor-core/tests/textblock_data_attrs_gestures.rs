//! The selection gestures tell the data attribute rules what they already know
//! of where their range lies (inside one textblock, or its two ends), so that
//! typing and deleting inside a block cost nothing for attributes. What they
//! tell must not change what an edit does: every gesture is held against the
//! same edit made from positions alone.

mod data_attr_common;

use data_attr_common::*;
use rinch_editor_core::commands::{delete_char_backward, delete_char_forward};
use rinch_editor_core::{EditorState, Pos, Selection, Transform};

/// Marked blocks at the top level and in a list, and two cells holding several
/// marked blocks, a heading and a list among them.
const DOC: &str = concat!(
    r#"<p data-role="a">ab</p><p data-role="b">cd</p><h2 data-role="h">ef</h2>"#,
    r#"<ul><li><p data-role="l">gh</p></li><li><p>ij</p></li></ul>"#,
    r#"<table><tr><td><p data-role="c">kl</p><p data-role="d">mn</p><p>op</p>"#,
    r#"<h2 data-role="i">qr</h2></td>"#,
    r#"<td><p>st</p><ul><li><p data-role="e">uv</p></li></ul></td></tr></table>"#,
);

/// The positions of `s` inside a textblock, each with the cell around it.
fn text_positions(s: &EditorState) -> Vec<(usize, Option<usize>)> {
    (0..=s.doc.content_size())
        .filter_map(|p| {
            let r = s.doc.resolve(Pos(p)).ok()?;
            let cell = (1..=r.depth())
                .rev()
                .find(|&d| r.node(d).node_type().name() == "table_cell")
                .and_then(|d| r.before(d));
            r.parent().is_textblock().then_some((p, cell))
        })
        .collect()
}

/// Every range inside one cell or wholly outside the table: there the gestures
/// repair no table, so they make exactly the edit.
fn confined_ranges(s: &EditorState) -> Vec<(usize, usize)> {
    let at = text_positions(s);
    let table = (0..s.doc.child_count() - 1)
        .map(|i| s.doc.child(i).node_size())
        .sum::<usize>();
    let mut out = Vec::new();
    for (i, &(from, a)) in at.iter().enumerate() {
        for &(to, b) in &at[i..] {
            if a == b && (a.is_some() || to < table || from > table) {
                out.push((from, to));
            }
        }
    }
    out
}

#[test]
fn typing_over_a_range_is_the_edit_made_from_its_positions() {
    let s = state(DOC, 1);
    let mut joined = 0;
    for (from, to) in confined_ranges(&s) {
        let mut gesture = s.tr();
        gesture.set_selection(Selection::text(Pos(from), Pos(to)));
        let typed = gesture.insert_text("x").is_ok();
        let mut plain = Transform::new(s.schema(), s.doc.clone());
        assert_eq!(plain.insert_text(from, to, "x").is_ok(), typed);
        assert_eq!(gesture.doc(), &plain.doc, "typing over {from}..{to}");
        joined += usize::from(gesture.steps().len() > 1);
    }
    assert!(joined > 0, "some joins moved an attribute");
}

#[test]
fn deleting_a_selection_is_the_edit_made_from_its_positions() {
    let s = state(DOC, 1);
    let mut joined = 0;
    for (from, to) in confined_ranges(&s) {
        if from == to {
            continue;
        }
        let mut gesture = s.tr();
        gesture.set_selection(Selection::text(Pos(from), Pos(to)));
        let deleted = gesture.delete_selection().is_ok();
        let mut plain = Transform::new(s.schema(), s.doc.clone());
        assert_eq!(plain.delete(from, to).is_ok(), deleted);
        assert_eq!(gesture.doc(), &plain.doc, "deleting {from}..{to}");
        joined += usize::from(gesture.steps().len() > 1);
    }
    assert!(joined > 0, "some joins moved an attribute");
}

/// A join inside a cell where none of the first block's text is left: the
/// attributes are the second block's, as outside a table.
#[test]
fn typing_over_blocks_in_a_cell_follows_the_text_left() {
    let cell = r#"<table><tr><td><p data-role="a">ab</p><p data-role="b">cd</p></td></tr></table>"#;
    // From the start of "ab" to between "c" and "d".
    let s = select(state(cell, 1), 4, 9);
    let mut tr = s.tr();
    tr.insert_text("x").unwrap();
    assert_eq!(
        html(&s.apply(tr)),
        r#"<table><tr><td><p data-role="b">xd</p></td></tr></table>"#
    );
}

#[test]
fn backspace_and_delete_inside_a_block_keep_its_attributes() {
    let s = state(r#"<p data-role="a">abc</p>"#, 3);
    let back = run(&s, delete_char_backward());
    assert_eq!(html(&back), r#"<p data-role="a">ac</p>"#);
    let forward = run(&s, delete_char_forward());
    assert_eq!(html(&forward), r#"<p data-role="a">ab</p>"#);
}
