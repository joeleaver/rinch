//! #1847, review of #1931: Shift+arrow across table cells, through
//! `EditorHandle::move_cursor` (every platform's horizontal motion) and
//! `shift_arrow_cells` (the platforms' Shift+Up/Down). prosemirror-tables'
//! `shiftArrow`: at a cell's edge the key selects that cell and its
//! neighbour, and over a cell selection it moves the head cell.
//!
//! Positions: `p(x)` 0..3; the table's cells at 5 ("ab", text 7..9), 11
//! ("cd", 13..15), 19 ("ef", 21..23), 25 ("gh", 27..29); `p(zw)` text 34..36.

use rinch_editor_core::serialize::node_to_html;
use rinch_editor_core::{Axis, CursorMotion, PasteContent, Pos, Selection};
use rinch_editor_view::{EditorHandle, create_editor};

const GRID: &str = "<p>x</p><table><tr><td><p>ab</p></td><td><p>cd</p></td></tr>\
                    <tr><td><p>ef</p></td><td><p>gh</p></td></tr></table><p>zw</p>";

fn grid(sel: Selection) -> EditorHandle {
    let h = create_editor();
    h.load_html(GRID);
    h.set_selection(sel);
    h
}

/// The selections after each `motion` with Shift held.
fn walk(start: usize, motions: &[CursorMotion]) -> Vec<Selection> {
    let h = grid(Selection::cursor(Pos(start)));
    motions
        .iter()
        .map(|&m| {
            h.move_cursor(m, true);
            h.selection()
        })
        .collect()
}

use CursorMotion::{CharLeft, CharRight, WordLeft, WordRight};

#[test]
fn shift_right_at_a_cells_end_selects_it_and_the_next() {
    // From "a|b": text to the cell's end, then the two cells; at the row's
    // end of the table's right column there is no cell further right.
    assert_eq!(
        walk(8, &[CharRight, CharRight, CharRight]),
        [
            Selection::text(Pos(8), Pos(9)),
            Selection::cell(Pos(5), Pos(11)),
            Selection::cell(Pos(5), Pos(11)),
        ]
    );
    // Right at the end: straight to the cells (it did nothing, the
    // boundary-only range collapsing).
    assert_eq!(walk(9, &[CharRight]), [Selection::cell(Pos(5), Pos(11))]);
}

#[test]
fn shift_left_at_a_cells_start_keeps_the_anchor_cell() {
    assert_eq!(
        walk(13, &[CharLeft, CharRight]),
        [
            Selection::cell(Pos(11), Pos(5)),
            Selection::cell(Pos(11), Pos(11))
        ]
    );
}

#[test]
fn shift_word_right_crosses_into_the_next_cell() {
    assert_eq!(
        walk(8, &[WordRight, WordRight]),
        [
            Selection::text(Pos(8), Pos(9)),
            Selection::cell(Pos(5), Pos(11))
        ]
    );
    assert_eq!(walk(13, &[WordLeft]), [Selection::cell(Pos(11), Pos(5))]);
}

#[test]
fn shift_right_at_a_rows_end_reaches_the_next_rows_first_cell() {
    // No cell to the right of "cd": the cell the motion reached, "ef".
    assert_eq!(
        walk(15, &[CharRight, CharRight]),
        [
            Selection::cell(Pos(11), Pos(19)),
            Selection::cell(Pos(11), Pos(25))
        ]
    );
}

#[test]
fn shift_right_out_of_the_table_is_a_text_selection() {
    // The end of the last cell: the plain extension, out of the table.
    assert_eq!(walk(29, &[CharRight]), [Selection::text(Pos(29), Pos(34))]);
}

#[test]
fn shift_up_and_down_step_cells_with_the_platforms_step() {
    let h = grid(Selection::cursor(Pos(8)));
    // The platform's vertical step stayed in the cell: not the cell rule's.
    assert!(!h.shift_arrow_cells(Axis::Vert, 1, Some(Pos(9))));
    assert_eq!(h.selection(), Selection::cursor(Pos(8)));
    // It left the cell (the caret was on the cell's bottom line).
    assert!(h.shift_arrow_cells(Axis::Vert, 1, Some(Pos(22))));
    assert_eq!(h.selection(), Selection::cell(Pos(5), Pos(19)));
    // Over a cell selection the head cell steps, and stops at the edge.
    assert!(h.shift_arrow_cells(Axis::Horiz, 1, None));
    assert_eq!(h.selection(), Selection::cell(Pos(5), Pos(25)));
    assert!(h.shift_arrow_cells(Axis::Vert, 1, None));
    assert_eq!(h.selection(), Selection::cell(Pos(5), Pos(25)));
    assert!(h.shift_arrow_cells(Axis::Vert, -1, None));
    assert_eq!(h.selection(), Selection::cell(Pos(5), Pos(11)));
    // Up out of the top row from a caret: the plain extension.
    let h = grid(Selection::cursor(Pos(8)));
    assert!(!h.shift_arrow_cells(Axis::Vert, -1, Some(Pos(2))));
}

#[test]
fn a_plain_arrow_over_a_cell_selection_collapses_it() {
    // Control: without Shift the cell rule is not asked.
    let h = grid(Selection::cell(Pos(5), Pos(11)));
    assert!(h.move_cursor(CharRight, false));
    assert!(matches!(h.selection(), Selection::Text(t) if t.anchor == t.head));
}

// ── adopted from the review's view probe ────────────────────────────────────

#[test]
fn a_copied_cell_selection_pastes_as_its_cells() {
    let src = grid(Selection::text(Pos(8), Pos(28)));
    assert_eq!(src.selection(), Selection::cell(Pos(5), Pos(25)));
    let (html, text) = src.selection_clipboard().unwrap();
    assert_eq!(
        html,
        "<table><tr><td><p>ab</p></td><td><p>cd</p></td></tr>\
         <tr><td><p>ef</p></td><td><p>gh</p></td></tr></table>"
    );
    // Into a paragraph: the table goes in whole.
    let dst = create_editor();
    dst.load_html("<p>xy</p>");
    dst.set_selection(Selection::cursor(Pos(2)));
    assert!(dst.paste(&PasteContent::new(Some(text), Some(html))));
    assert_eq!(
        node_to_html(&dst.doc()),
        format!("<p>x</p>{}<p>y</p>", src.selection_clipboard().unwrap().0)
    );
}

#[test]
fn the_handles_commands_over_a_cell_selection() {
    let h = grid(Selection::cell(Pos(5), Pos(25)));
    assert!(!h.command("enter"), "Enter refuses a cell selection");
    assert!(h.command("insertHardBreak"));
    assert_eq!(
        node_to_html(&h.doc()),
        "<p>x</p><table><tr><td><p><br></p></td><td><p></p></td></tr>\
         <tr><td><p></p></td><td><p></p></td></tr></table><p>zw</p>"
    );
    let h = grid(Selection::cell(Pos(5), Pos(25)));
    assert!(h.insert_text("Z"));
    assert_eq!(
        node_to_html(&h.doc()),
        "<p>x</p><table><tr><td><p>Z</p></td><td><p></p></td></tr>\
         <tr><td><p></p></td><td><p></p></td></tr></table><p>zw</p>"
    );
}
