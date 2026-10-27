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

// ── adopted from the second review of #1931 ─────────────────────────────────

fn at(doc: &rinch_editor_core::Node, text: &str) -> usize {
    let mut f = None;
    doc.nodes_between(0, doc.content().size(), &mut |n, pos, _| {
        if f.is_none() && n.is_text() && n.text().is_some_and(|t| t.contains(text)) {
            f = Some(pos + n.text().unwrap().find(text).unwrap());
        }
        f.is_none()
    });
    f.unwrap_or_else(|| panic!("no {text}"))
}

fn ed(html: &str, sel: Selection) -> EditorHandle {
    let h = create_editor();
    h.load_html(html);
    h.set_selection(sel);
    h
}

/// `cell(p(ab), table(in1 | in2)) | cell(cd)`, then `p(zw)`.
const NESTED: &str = "<table><tr><td><p>ab</p><table><tr><td><p>in1</p></td>\
                      <td><p>in2</p></td></tr></table></td><td><p>cd</p></td></tr></table><p>zw</p>";

#[test]
fn a_nested_table_in_the_cell_is_inside_the_cell() {
    let d = ed(NESTED, Selection::cursor(Pos(0))).doc();
    let ab_end = Pos(at(&d, "ab") + 2);
    // "ab" is not its cell's last block (PM `atEndOfCell`): text, for the
    // character, word and line motions alike.
    for m in [CharRight, WordRight] {
        let h = ed(NESTED, Selection::cursor(ab_end));
        h.move_cursor(m, true);
        assert!(
            matches!(h.selection(), Selection::Text(t) if t.anchor == ab_end),
            "{m:?}: {:?}",
            h.selection()
        );
    }
    let h = ed(NESTED, Selection::cursor(ab_end));
    assert!(!h.shift_arrow_cells(Axis::Vert, 1, Some(Pos(at(&d, "in1") + 1))));
    assert_eq!(h.selection(), Selection::cursor(ab_end));
    // Inside the nested table its own cells are the neighbours.
    let in1_end = Pos(at(&d, "in1") + 3);
    let h = ed(NESTED, Selection::cursor(in1_end));
    h.move_cursor(CharRight, true);
    let (c1, c2) = (at(&d, "in1") - 2, at(&d, "in2") - 2);
    assert_eq!(h.selection(), Selection::cell(Pos(c1), Pos(c2)));
    // Out of the nested table's last cell into the next outer cell: that
    // cell alone, so the anchor stays in it (the plain extension made a cell
    // selection of the outer cells).
    let in2_end = Pos(at(&d, "in2") + 3);
    let h = ed(NESTED, Selection::cursor(in2_end));
    h.move_cursor(CharRight, true);
    assert_eq!(h.selection(), Selection::cell(Pos(c2), Pos(c2)));
    // Control: from "cd" leftwards the outer cells.
    let h = ed(NESTED, Selection::cursor(Pos(at(&d, "cd"))));
    h.move_cursor(CharLeft, true);
    assert_eq!(
        h.selection(),
        Selection::cell(Pos(at(&d, "cd") - 2), Pos(at(&d, "ab") - 2))
    );
}

#[test]
fn shift_arrows_at_the_documents_edges_do_nothing() {
    const ONE_ROW: &str = "<table><tr><td><p>ab</p></td><td><p>cd</p></td></tr></table>";
    let d = ed(ONE_ROW, Selection::cursor(Pos(0))).doc();
    let start = Pos(at(&d, "ab"));
    let h = ed(ONE_ROW, Selection::cursor(start));
    h.move_cursor(CharLeft, true);
    assert_eq!(h.selection(), Selection::cursor(start), "not its own cell");
    let end = Pos(at(&d, "cd") + 2);
    let h = ed(ONE_ROW, Selection::cursor(end));
    h.move_cursor(CharRight, true);
    assert_eq!(h.selection(), Selection::cursor(end));
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Random horizontal Shift-walks from every caret of tables with merged
/// cells, a nested table, lists in cells and a second table: the anchor (the
/// caret's position, or its cell once a cell selection) never moves.
#[test]
fn the_anchor_holds_in_random_horizontal_walks() {
    const MERGED: &str = "<p>x</p><table><tr><td colspan=\"2\"><p>A</p></td><td><p>B</p></td></tr>\
        <tr><td rowspan=\"2\"><p>C</p></td><td><p>D</p></td><td><p>E</p></td></tr>\
        <tr><td><p>F</p></td><td><p>G</p></td></tr></table><p>zw</p>";
    const MIXED: &str = "<p>x</p><table><tr><td><p>a b</p><p>c</p></td><td><ul><li><p>l1</p></li></ul></td></tr>\
        <tr><td><p></p></td><td><p>gh</p></td></tr></table><table><tr><td><p>t2</p></td></tr></table><p>zw</p>";
    let (mut fails, mut walks) = (Vec::new(), 0);
    for html in [MERGED, NESTED, MIXED] {
        let d = ed(html, Selection::cursor(Pos(0))).doc();
        let carets: Vec<usize> = (0..=d.content().size())
            .filter(|&p| d.resolve(Pos(p)).unwrap().parent().is_textblock())
            .collect();
        let mut rng = Rng(0x1234_5678_9abc_def1);
        for &c in &carets {
            for _ in 0..20 {
                let h = ed(html, Selection::cursor(Pos(c)));
                let start_cell = rinch_editor_core::tables::cell_at_pos(&d, Pos(c));
                walks += 1;
                for _ in 0..1 + rng.below(8) {
                    let m = [CharLeft, CharRight, WordLeft, WordRight][rng.below(4)];
                    h.move_cursor(m, true);
                    let ok = match h.selection() {
                        Selection::Text(t) => t.anchor == Pos(c) || t.anchor == t.head,
                        Selection::Cell(cs) => Some(cs.anchor_cell.0) == start_cell,
                        Selection::Node(_) => true,
                    };
                    if !ok {
                        fails.push(format!("{html} caret {c}: {:?}", h.selection()));
                        break;
                    }
                }
            }
        }
    }
    assert!(walks > 1000, "{walks} walks");
    assert!(
        fails.is_empty(),
        "{} of {walks}: {:?}",
        fails.len(),
        &fails[..fails.len().min(3)]
    );
}

#[test]
fn a_loaded_ragged_table_stays_and_a_pasted_one_is_padded() {
    let html = "<p>x</p><table><tr><td><p>A</p></td><td><p>B</p></td></tr><tr><td><p>C</p></td></tr></table><p>zw</p>";
    let h = ed(html, Selection::cursor(Pos(1)));
    assert_eq!(node_to_html(&h.doc()), html);
    let h2 = ed("<p>x</p>", Selection::cursor(Pos(1)));
    assert!(h2.paste(&PasteContent::new(None, Some(html.to_string()))));
    assert!(
        node_to_html(&h2.doc()).contains("<tr><td><p>C</p></td><td><p></p></td></tr>"),
        "{}",
        node_to_html(&h2.doc())
    );
}
