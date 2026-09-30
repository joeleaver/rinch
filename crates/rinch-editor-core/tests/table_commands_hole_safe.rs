//! #1184: the table commands on a ragged table, on a table whose spans the
//! grid cut (#1176), and on spans the document states as `i64::MAX`.
//!
//! A grid slot no cell covers (a hole) is no cell: a command reading one used
//! to hand `usize::MAX` to `Node::node_at` (a `debug_assert` in
//! `Fragment::find_index`), and a span was widened with `+ 1` on the raw
//! attribute (overflow at `i64::MAX`). Such tables reach the model through
//! `load_doc` or an app's own transaction; the HTML import clamps spans.
//!
//! Every map here is small: the largest is an `i64::MAX` span, whose grid is
//! capped at `GRID_SLOT_FLOOR` (2^22 slots, 32 MB).

use rinch_editor_core::schema::validation::validate_content;
use rinch_editor_core::tables::{TableMap, is_cell, is_table};
use rinch_editor_core::*;
use std::rc::Rc;

const CMDS: &[&str] = &[
    "addRowBefore",
    "addRowAfter",
    "addColumnBefore",
    "addColumnAfter",
    "deleteRow",
    "deleteColumn",
    "mergeCells",
    "splitCell",
    "deleteTable",
    "deleteCellSelection",
];

fn cell(s: &Schema, colspan: i64, rowspan: i64, text: &str) -> Node {
    let p = s
        .branch("paragraph", Fragment::from_node(s.text(text).unwrap()))
        .unwrap();
    s.create_node(
        "table_cell",
        Attrs::from_iter([
            ("colspan", AttrValue::Int(colspan)),
            ("rowspan", AttrValue::Int(rowspan)),
        ]),
        Fragment::from_node(p),
    )
    .unwrap()
}

/// A table from rows of `(colspan, rowspan)`.
fn table(s: &Schema, rows: &[Vec<(i64, i64)>]) -> Node {
    let mut n = 0;
    let rows = rows
        .iter()
        .map(|r| {
            let cells = r
                .iter()
                .map(|&(c, h)| {
                    n += 1;
                    cell(s, c, h, &format!("c{n}"))
                })
                .collect();
            s.create_node("table_row", Attrs::new(), Fragment::from_children(cells))
                .unwrap()
        })
        .collect();
    s.create_node("table", Attrs::new(), Fragment::from_children(rows))
        .unwrap()
}

/// The table at doc position 0, then an empty paragraph.
fn state_with(t: Node) -> EditorState {
    let s = Schema::starter_kit();
    let tail = s
        .create_node("paragraph", Attrs::new(), Fragment::empty())
        .unwrap();
    let doc = s
        .branch("doc", Fragment::from_children(vec![t, tail]))
        .unwrap();
    EditorState::create(Rc::new(s), doc, default_plugins())
}

/// The position before row `r`'s cell `i` (the table is at doc position 0).
fn cell_pos(t: &Node, r: usize, i: usize) -> usize {
    let mut pos = 1;
    for j in 0..r {
        pos += t.child(j).node_size();
    }
    pos += 1;
    let row = t.child(r);
    for k in 0..i {
        pos += row.child(k).node_size();
    }
    pos
}

/// A caret inside row `r`'s cell `i`.
fn caret_in(t: &Node, r: usize, i: usize) -> Selection {
    Selection::cursor(Pos(cell_pos(t, r, i) + 2))
}

/// Every node's children match its content expression.
fn assert_schema_valid(schema: &Schema, node: &Node, what: &str) {
    if node.is_text() {
        return;
    }
    let children: Vec<Node> = node.content().children().to_vec();
    let types: Vec<&str> = children.iter().map(|c| c.type_name()).collect();
    if let Err(e) = validate_content(schema, node.type_name(), &types) {
        panic!("{what}: {e}");
    }
    for c in &children {
        assert_schema_valid(schema, c, what);
    }
}

/// Every table cell's spans in `node`.
fn spans(node: &Node, out: &mut Vec<i64>) {
    if is_cell(node) {
        for name in ["colspan", "rowspan"] {
            out.push(node.attrs().get_int(name).unwrap_or(1));
        }
    }
    for c in node.content().children() {
        spans(c, out);
    }
}

fn span(cell: &Node, name: &str) -> i64 {
    cell.attrs().get_int(name).unwrap_or(1)
}

fn cell_count(t: &Node) -> Vec<usize> {
    (0..t.child_count())
        .map(|r| t.child(r).child_count())
        .collect()
}

// ------------------------------------------------------------------
// The issue's three shapes, and the other sites that read a hole.
// ------------------------------------------------------------------

/// A 3/2/3-cell table: row 1's slot at column 2 is a hole.
fn ragged() -> Node {
    table(
        &Schema::starter_kit(),
        &[
            vec![(1, 1), (1, 1), (1, 1)],
            vec![(1, 1), (1, 1)],
            vec![(1, 1), (1, 1), (1, 1)],
        ],
    )
}

#[test]
fn delete_row_in_the_short_row_of_a_ragged_table_removes_that_row() {
    let t = ragged();
    let mut st = state_with(t.clone());
    st.selection = caret_in(&t, 1, 0);
    let next = st.run("deleteRow").expect("deleteRow applies");
    let nt = next.doc.child(0).clone();
    assert_eq!(cell_count(&nt), vec![3, 3]);
    assert_eq!(nt.child(1).child(0).child(0).child(0).text(), Some("c6"));
}

#[test]
fn delete_row_across_a_hole_in_a_cell_selection() {
    // Rows 0..2 selected through column 2: the rect covers row 1's hole.
    let t = ragged();
    let mut st = state_with(t.clone());
    st.selection = Selection::cell(Pos(cell_pos(&t, 0, 2)), Pos(cell_pos(&t, 1, 0)));
    let next = st.run("deleteRow").expect("deleteRow applies");
    assert_eq!(cell_count(next.doc.child(0)), vec![3]);
}

#[test]
fn delete_column_through_a_hole_skips_the_short_row() {
    // Column 2 from row 0: row 1 has no cell there, and keeps both of its own.
    let t = ragged();
    let mut st = state_with(t.clone());
    st.selection = caret_in(&t, 0, 2);
    let next = st.run("deleteColumn").expect("deleteColumn applies");
    assert_eq!(cell_count(next.doc.child(0)), vec![2, 2, 2]);
}

#[test]
fn add_column_before_a_hole_gives_the_short_row_a_cell() {
    // Column 2 from row 0: row 1's hole at column 2 is no spanning cell, so
    // the row gets a new cell at its end.
    let t = ragged();
    let mut st = state_with(t.clone());
    st.selection = caret_in(&t, 0, 2);
    let next = st.run("addColumnBefore").expect("addColumnBefore applies");
    assert_eq!(cell_count(next.doc.child(0)), vec![4, 3, 4]);
}

#[test]
fn add_row_under_a_hole_is_full_width() {
    // Below row 1 (the short one): the new row has a cell in every column,
    // the hole's column included.
    let t = ragged();
    let mut st = state_with(t.clone());
    st.selection = caret_in(&t, 2, 0);
    let next = st.run("addRowBefore").expect("addRowBefore applies");
    assert_eq!(cell_count(next.doc.child(0)), vec![3, 2, 3, 3]);
}

#[test]
fn add_row_between_two_holes_is_full_width() {
    // Rows 0 and 1 both end in a hole at column 2 (2/2/3 cells). A new row
    // between them read the two holes as one cell spanning into it.
    let s = Schema::starter_kit();
    let t = table(
        &s,
        &[
            vec![(1, 1), (1, 1)],
            vec![(1, 1), (1, 1)],
            vec![(1, 1), (1, 1), (1, 1)],
        ],
    );
    let mut st = state_with(t.clone());
    st.selection = caret_in(&t, 1, 0);
    let next = st.run("addRowBefore").expect("addRowBefore applies");
    assert_eq!(cell_count(next.doc.child(0)), vec![2, 3, 2, 3]);
}

#[test]
fn add_column_between_two_holes_gives_the_row_a_cell() {
    // Row 1 is one cell in a 3-wide grid: its columns 1 and 2 are holes.
    // A column inserted between them read the two holes as one spanning cell.
    let s = Schema::starter_kit();
    let t = table(&s, &[vec![(1, 1), (1, 1), (1, 1)], vec![(1, 1)]]);
    let mut st = state_with(t.clone());
    st.selection = caret_in(&t, 0, 1);
    let next = st.run("addColumnAfter").expect("addColumnAfter applies");
    assert_eq!(cell_count(next.doc.child(0)), vec![4, 2]);
}

#[test]
fn add_column_after_under_an_i64_max_colspan_does_not_overflow() {
    // Row 0 is one cell of colspan i64::MAX; the grid cuts it (#1176). A
    // column inserted after row 1's first cell widens it by one column of
    // the grid it covers, not by one past i64::MAX.
    let s = Schema::starter_kit();
    let t = table(&s, &[vec![(i64::MAX, 1)], vec![(1, 1), (1, 1)]]);
    let width = TableMap::compute(&t, 1).width();
    assert_eq!(width, 1 << 21, "the grid cut the span: 2^22 slots / 2 rows");
    let mut st = state_with(t.clone());
    st.selection = caret_in(&t, 1, 0);
    let next = st.run("addColumnAfter").expect("addColumnAfter applies");
    let nt = next.doc.child(0).clone();
    assert_eq!(cell_count(&nt), vec![1, 3]);
    let wide = span(nt.child(0).child(0), "colspan");
    assert_eq!(wide, width as i64 + 1, "the grid's span, widened by one");
}

#[test]
fn add_row_before_under_an_i64_max_rowspan_does_not_overflow() {
    let s = Schema::starter_kit();
    let t = table(&s, &[vec![(1, i64::MAX), (1, 1)], vec![(1, 1)]]);
    let mut st = state_with(t.clone());
    st.selection = caret_in(&t, 1, 0);
    let next = st.run("addRowBefore").expect("addRowBefore applies");
    let nt = next.doc.child(0).clone();
    assert_eq!(cell_count(&nt), vec![2, 1, 1]);
    // The span covered both rows of the grid; it now covers all three.
    assert_eq!(span(nt.child(0).child(0), "rowspan"), 3);
}

#[test]
fn merge_cells_grows_over_a_hole() {
    // A 3/1/1-cell table; the rectangle (0,0)..(1,1) holds c1, c2, c4 and
    // row 1's hole at column 1. The hole is no cell to delete, and two holes
    // side by side (row 1, columns 1 and 2) are no cell bleeding out of it.
    let s = Schema::starter_kit();
    let t = table(
        &s,
        &[vec![(1, 1), (1, 1), (1, 1)], vec![(1, 1)], vec![(1, 1)]],
    );
    let mut st = state_with(t.clone());
    st.selection = Selection::cell(Pos(cell_pos(&t, 0, 1)), Pos(cell_pos(&t, 1, 0)));
    let next = st.run("mergeCells").expect("mergeCells applies");
    let nt = next.doc.child(0).clone();
    assert_eq!(cell_count(&nt), vec![2, 0, 1]);
    let master = nt.child(0).child(0);
    assert_eq!((span(master, "colspan"), span(master, "rowspan")), (2, 2));
    assert_eq!(master.child_count(), 3, "c1, c2 and c4's paragraphs");
}

#[test]
fn merge_cells_is_not_refused_for_holes_beside_its_left_or_top_edge() {
    let s = Schema::starter_kit();
    // 3/0/3: the rectangle (0,1)..(2,2) crosses the empty row 1, whose holes
    // at columns 0 and 1 sit on both sides of its left edge.
    let t = table(
        &s,
        &[
            vec![(1, 1), (1, 1), (1, 1)],
            vec![],
            vec![(1, 1), (1, 1), (1, 1)],
        ],
    );
    let mut st = state_with(t.clone());
    st.selection = Selection::cell(Pos(cell_pos(&t, 0, 1)), Pos(cell_pos(&t, 2, 2)));
    let next = st
        .run("mergeCells")
        .expect("mergeCells applies (left edge)");
    let nt = next.doc.child(0).clone();
    assert_eq!(cell_count(&nt), vec![2, 0, 1]);
    let master = nt.child(0).child(1);
    assert_eq!((span(master, "colspan"), span(master, "rowspan")), (2, 3));

    // 1/1/3: the rectangle (1,0)..(2,2) starts in row 1, whose holes at
    // columns 1 and 2 sit under row 0's, across its top edge.
    let t = table(
        &s,
        &[vec![(1, 1)], vec![(1, 1)], vec![(1, 1), (1, 1), (1, 1)]],
    );
    let mut st = state_with(t.clone());
    st.selection = Selection::cell(Pos(cell_pos(&t, 1, 0)), Pos(cell_pos(&t, 2, 2)));
    let next = st.run("mergeCells").expect("mergeCells applies (top edge)");
    let nt = next.doc.child(0).clone();
    assert_eq!(cell_count(&nt), vec![1, 1, 0]);
    let master = nt.child(1).child(0);
    assert_eq!((span(master, "colspan"), span(master, "rowspan")), (3, 2));
}

#[test]
fn every_command_in_every_cell_of_the_issues_tables_is_safe() {
    let s = Schema::starter_kit();
    let tables = [
        ragged(),
        table(&s, &[vec![(i64::MAX, 1)], vec![(1, 1), (1, 1)]]),
        table(&s, &[vec![(1, i64::MAX), (1, 1)], vec![(1, 1)]]),
        table(&s, &[vec![(3, 1)], vec![(1, 1)]]),
        table(&s, &[vec![(1, 5), (1, 1)], vec![(1, 1)], vec![]]),
    ];
    for t in tables {
        for r in 0..t.child_count() {
            for i in 0..t.child(r).child_count() {
                // The i64::MAX colspan's grid is 2^21 columns wide: each command
                // there maps 2^22 slots, so it is run from one cell (the issue's
                // row), not every one, to keep a debug build's run short.
                let huge = (0..t.child(0).child_count())
                    .any(|k| span(t.child(0).child(k), "colspan") > 1000);
                if huge && r == 0 {
                    continue;
                }
                for c in CMDS {
                    // #1185: addRow and splitCell make one cell per grid slot,
                    // 2^21 of them under that span.
                    if huge && (c.starts_with("addRow") || *c == "splitCell") {
                        continue;
                    }
                    let mut st = state_with(t.clone());
                    st.selection = caret_in(&t, r, i);
                    check_command(&st, c, &format!("{c} in row {r} cell {i} of {t:?}"));
                }
            }
        }
    }
}

// ------------------------------------------------------------------
// Property: random tables × every command.
// ------------------------------------------------------------------

/// xorshift64*: deterministic, no dependency.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Runs `c` on `st` and checks the result: the document is schema-valid, no
/// span went below what the input held (no wraparound), and every table in it
/// maps. Returns the new state, if any. (`can_run` builds the same transaction
/// as `run`, so it is not asked separately.)
fn check_command(st: &EditorState, c: &str, what: &str) -> Option<EditorState> {
    let next = st.run(c)?;
    assert_schema_valid(next.schema(), &next.doc, what);
    let (mut before, mut after) = (Vec::new(), Vec::new());
    spans(&st.doc, &mut before);
    spans(&next.doc, &mut after);
    let floor = before.iter().copied().min().unwrap_or(1).min(1);
    assert!(
        after.iter().all(|&v| v >= floor),
        "{what}: a span below {floor}: {after:?}"
    );
    for (i, n) in next.doc.content().children().iter().enumerate() {
        if is_table(n) {
            let _ = TableMap::compute(n, 1 + i);
        }
    }
    Some(next)
}

/// A random selection in `t` (at doc position 0): a caret in a cell, or a
/// cell selection between two cells of the grid.
fn random_selection(rng: &mut Rng, t: &Node) -> Option<Selection> {
    let cells: Vec<(usize, usize)> = (0..t.child_count())
        .flat_map(|r| (0..t.child(r).child_count()).map(move |i| (r, i)))
        .collect();
    if cells.is_empty() {
        return None;
    }
    let (r, i) = cells[rng.below(cells.len())];
    if rng.below(2) == 0 {
        return Some(caret_in(t, r, i));
    }
    let (r2, i2) = cells[rng.below(cells.len())];
    Some(Selection::cell(
        Pos(cell_pos(t, r, i)),
        Pos(cell_pos(t, r2, i2)),
    ))
}

/// Ragged rows of random spans, holes and overlaps included.
#[test]
fn random_ragged_tables_x_every_command_are_safe() {
    let s = Schema::starter_kit();
    let mut rng = Rng(0x1184_1184_dead_beef);
    const SPANS: &[i64] = &[1, 1, 1, 1, 2, 3, 0, -1, 40];
    let mut applied = 0;
    for case in 0..600 {
        let h = 1 + rng.below(5);
        let rows: Vec<Vec<(i64, i64)>> = (0..h)
            .map(|_| {
                let n = rng.below(5);
                (0..n)
                    .map(|_| (SPANS[rng.below(SPANS.len())], SPANS[rng.below(SPANS.len())]))
                    .collect()
            })
            .collect();
        let t = table(&s, &rows);
        let Some(sel) = random_selection(&mut rng, &t) else {
            continue;
        };
        let mut st = state_with(t);
        st.selection = sel;
        for c in CMDS {
            let what = format!("case {case}: {c} at {:?} in {rows:?}", st.selection);
            if check_command(&st, c, &what).is_some() {
                applied += 1;
            }
        }
    }
    // The positive control: the property saw commands apply, not only refuse.
    assert!(applied > 1500, "only {applied} commands applied");
}

/// Each row's `(colspan, rowspan)`s, for a failure message.
fn table_spans(t: &Node) -> Vec<Vec<(i64, i64)>> {
    (0..t.child_count())
        .map(|r| {
            let row = t.child(r);
            (0..row.child_count())
                .map(|i| (span(row.child(i), "colspan"), span(row.child(i), "rowspan")))
                .collect()
        })
        .collect()
}

/// A random well-formed table: a `w × h` grid tiled with rectangles.
fn random_tiled(rng: &mut Rng, s: &Schema) -> (Node, usize, usize) {
    let w = 1 + rng.below(5);
    let h = 1 + rng.below(5);
    let mut owner = vec![false; w * h];
    let mut rows: Vec<Vec<(i64, i64)>> = vec![Vec::new(); h];
    for r in 0..h {
        for c in 0..w {
            if owner[r * w + c] {
                continue;
            }
            let mut cs = 1 + rng.below(3);
            let mut rs = 1 + rng.below(3);
            cs = cs.min(w - c);
            rs = rs.min(h - r);
            // Shrink to the free run to the right.
            let mut free = 0;
            while c + free < w && !owner[r * w + c + free] && free < cs {
                free += 1;
            }
            cs = free;
            for rr in r..r + rs {
                for cc in c..c + cs {
                    owner[rr * w + cc] = true;
                }
            }
            rows[r].push((cs as i64, rs as i64));
        }
    }
    (table(s, &rows), w, h)
}

/// Whether `t` is a well-formed grid: every slot covered, no two cells
/// overlapping, every span exactly the grid it covers.
fn well_formed(t: &Node) -> Result<(), String> {
    let map = TableMap::compute(t, 1);
    if map.map().contains(&usize::MAX) {
        return Err("a hole".into());
    }
    let mut area = 0;
    for r in 0..t.child_count() {
        for i in 0..t.child(r).child_count() {
            let pos = cell_pos(t, r, i);
            let rect = map
                .find_cell(pos)
                .ok_or_else(|| format!("row {r} cell {i} is off the grid"))?;
            let c = t.child(r).child(i);
            let (cs, rs) = (span(c, "colspan"), span(c, "rowspan"));
            if cs != (rect.right - rect.left) as i64 || rs != (rect.bottom - rect.top) as i64 {
                return Err(format!("row {r} cell {i}: {cs}x{rs} covers {rect:?}"));
            }
            area += (cs * rs) as usize;
        }
    }
    if area != map.map().len() {
        return Err(format!("cells cover {area} of {} slots", map.map().len()));
    }
    Ok(())
}

/// A well-formed table stays well-formed under every command, and the
/// commands that always make sense on one apply.
#[test]
fn random_well_formed_tables_stay_well_formed_under_every_command() {
    let s = Schema::starter_kit();
    let mut rng = Rng(0x0929_1184_c0ff_ee00);
    for case in 0..400 {
        let (t, _, _) = random_tiled(&mut rng, &s);
        well_formed(&t).unwrap_or_else(|e| panic!("case {case}: the generator: {e}"));
        let sel = random_selection(&mut rng, &t).expect("a cell");
        let mut st = state_with(t);
        st.selection = sel;
        for c in CMDS {
            let what = format!(
                "case {case}: {c} at {:?} in {:?}",
                st.selection,
                table_spans(st.doc.child(0))
            );
            let next = check_command(&st, c, &what);
            // On a well-formed table every add and delete command applies.
            if c.starts_with("add") || (c.starts_with("delete") && *c != "deleteCellSelection") {
                assert!(next.is_some(), "{what}: refused");
            }
            if let Some(next) = next
                && let Some(nt) = next.doc.content().children().iter().find(|n| is_table(n))
            {
                well_formed(nt).unwrap_or_else(|e| panic!("{what}: {e}"));
            }
        }
    }
}

// ------------------------------------------------------------------
// From the review of #1191.
// ------------------------------------------------------------------

/// Canonical text of a node: type, spans, text.
fn canon(n: &Node, out: &mut String) {
    if let Some(t) = n.text() {
        out.push_str(&format!("{t:?}"));
        return;
    }
    out.push_str(n.type_name());
    if is_cell(n) {
        out.push_str(&format!("[{}x{}]", span(n, "colspan"), span(n, "rowspan")));
    }
    out.push('(');
    for c in n.content().children() {
        canon(c, out);
        out.push(',');
    }
    out.push(')');
}

fn canon_state(st: &EditorState) -> String {
    let mut s = String::new();
    canon(&st.doc, &mut s);
    s.push_str(&format!(" sel={:?}", st.selection));
    s
}

/// Every selection worth trying in `t`: a caret in each cell, and a cell
/// selection between every ordered pair of cells (capped).
fn all_selections(t: &Node) -> Vec<Selection> {
    let cells: Vec<(usize, usize)> = (0..t.child_count())
        .flat_map(|r| (0..t.child(r).child_count()).map(move |i| (r, i)))
        .collect();
    let mut out = Vec::new();
    for &(r, i) in &cells {
        out.push(caret_in(t, r, i));
    }
    for &(r, i) in &cells {
        for &(r2, i2) in &cells {
            out.push(Selection::cell(
                Pos(cell_pos(t, r, i)),
                Pos(cell_pos(t, r2, i2)),
            ));
        }
    }
    out
}

/// Undo then redo of each command on random ragged and tiled tables
/// restores the documents exactly.
#[test]
fn undo_redo_round_trips_on_ragged_and_tiled_tables() {
    let s = Schema::starter_kit();
    let mut rng = Rng(0x0dd0_1191_aaaa_5555);
    const SPANS: &[i64] = &[1, 1, 1, 1, 2, 3, 0, -1, 40];
    let mut checked = 0;
    for case in 0..400 {
        let t = if case % 2 == 0 {
            let h = 1 + rng.below(5);
            let rows: Vec<Vec<(i64, i64)>> = (0..h)
                .map(|_| {
                    let n = rng.below(5);
                    (0..n)
                        .map(|_| (SPANS[rng.below(SPANS.len())], SPANS[rng.below(SPANS.len())]))
                        .collect()
                })
                .collect();
            table(&s, &rows)
        } else {
            random_tiled(&mut rng, &s).0
        };
        let Some(sel) = random_selection(&mut rng, &t) else {
            continue;
        };
        let mut st = state_with(t);
        st.selection = sel;
        for c in CMDS {
            let Some(next) = st.run(c) else { continue };
            if next.doc == st.doc {
                continue;
            }
            let what = format!("case {case} {c} {}", canon_state(&st));
            let undone = next
                .run("undo")
                .unwrap_or_else(|| panic!("{what}: undo refused"));
            assert!(
                undone.doc == st.doc,
                "{what}: undo\n got {}\n want {}",
                canon_state(&undone),
                canon_state(&st)
            );
            let redone = undone
                .run("redo")
                .unwrap_or_else(|| panic!("{what}: redo refused"));
            assert!(
                redone.doc == next.doc,
                "{what}: redo\n got {}\n want {}",
                canon_state(&redone),
                canon_state(&next)
            );
            checked += 1;
        }
    }
    eprintln!("undo/redo checked {checked}");
    assert!(checked > 1500, "positive control: {checked}");
}

/// A grid oracle for deleteRow / deleteColumn on well-formed tables: each
/// cell keeps its content, loses the removed rows/columns from its span, is
/// dropped when nothing is left, and lands in its first surviving row, in
/// column order.
#[test]
fn delete_row_and_column_match_a_grid_oracle_on_well_formed_tables() {
    let s = Schema::starter_kit();
    let mut rng = Rng(0x0a0c_1e11_1191_0002);
    let mut compared = 0;
    // 40 tables (every caret and cell pair on each): about 1500 comparisons,
    // a few seconds in a debug build.
    for case in 0..40 {
        let (t, _, _) = random_tiled(&mut rng, &s);
        let map = TableMap::compute(&t, 1);
        let (w, h) = (map.width(), map.height());
        // (text, rect) of every cell
        let mut cells = Vec::new();
        for r in 0..t.child_count() {
            for i in 0..t.child(r).child_count() {
                let rect = map.find_cell(cell_pos(&t, r, i)).unwrap();
                let text = t
                    .child(r)
                    .child(i)
                    .child(0)
                    .child(0)
                    .text()
                    .unwrap()
                    .to_string();
                cells.push((text, rect));
            }
        }
        for sel in all_selections(&t) {
            let rect = match &sel {
                Selection::Cell(c) => map.rect_between(c.anchor_cell.0, c.head_cell.0).unwrap(),
                other => {
                    let p = other.head().0;
                    // the cell whose content holds p
                    let (_, r) = cells
                        .iter()
                        .zip(0..)
                        .map(|((_, rc), k)| (k, *rc))
                        .find(|(k, _)| {
                            let (rr, ii) = nth_cell(&t, *k);
                            let cp = cell_pos(&t, rr, ii);
                            p > cp && p < cp + t.child(rr).child(ii).node_size()
                        })
                        .unwrap();
                    r
                }
            };
            for (c, axis_rows) in [("deleteRow", true), ("deleteColumn", false)] {
                let (lo, hi, n) = if axis_rows {
                    (rect.top, rect.bottom, h)
                } else {
                    (rect.left, rect.right, w)
                };
                let mut st = state_with(t.clone());
                st.selection = sel.clone();
                let next = st.run(c).expect("applies");
                let nt = next
                    .doc
                    .content()
                    .children()
                    .iter()
                    .find(|x| is_table(x))
                    .cloned();
                if lo == 0 && hi == n {
                    assert!(nt.is_none(), "case {case} {c}: whole table not deleted");
                    continue;
                }
                let nt = nt.expect("table");
                let removed_before = |x: usize| (lo..hi).filter(|&k| k < x).count();
                let new_h = if axis_rows { h - (hi - lo) } else { h };
                let mut expect: Vec<Vec<(usize, String, i64, i64)>> = vec![Vec::new(); new_h];
                for (text, rc) in &cells {
                    let (a, b) = if axis_rows {
                        (rc.top, rc.bottom)
                    } else {
                        (rc.left, rc.right)
                    };
                    let ov = (a..b).filter(|k| (lo..hi).contains(k)).count();
                    let len = (b - a) - ov;
                    if len == 0 {
                        continue;
                    }
                    let first = (a..b).find(|k| !(lo..hi).contains(k)).unwrap();
                    let start = first - removed_before(first);
                    let (row, col, cs, rs) = if axis_rows {
                        (start, rc.left, rc.right - rc.left, len)
                    } else {
                        (rc.top, start, len, rc.bottom - rc.top)
                    };
                    expect[row].push((col, text.clone(), cs as i64, rs as i64));
                }
                for r in &mut expect {
                    r.sort();
                }
                let got: Vec<Vec<(String, i64, i64)>> = (0..nt.child_count())
                    .map(|r| {
                        (0..nt.child(r).child_count())
                            .map(|i| {
                                let cc = nt.child(r).child(i);
                                (
                                    cc.child(0).child(0).text().unwrap().to_string(),
                                    span(cc, "colspan"),
                                    span(cc, "rowspan"),
                                )
                            })
                            .collect()
                    })
                    .collect();
                let want: Vec<Vec<(String, i64, i64)>> = expect
                    .into_iter()
                    .map(|r| r.into_iter().map(|(_, t, a, b)| (t, a, b)).collect())
                    .collect();
                assert_eq!(
                    got,
                    want,
                    "case {case} {c} over {lo}..{hi} in {:?}",
                    table_spans(&t)
                );
                compared += 1;
            }
        }
    }
    eprintln!("oracle compared {compared}");
    assert!(compared > 1000, "positive control: {compared}");
}

fn nth_cell(t: &Node, k: usize) -> (usize, usize) {
    let mut k = k;
    for r in 0..t.child_count() {
        let n = t.child(r).child_count();
        if k < n {
            return (r, k);
        }
        k -= n;
    }
    unreachable!()
}
