//! Property tests for the table commands from the review of #1876 (#1244,
//! #1874): `addRowBefore`/`addRowAfter` copy the neighbour row and keep a
//! well-formed grid tiled and a ragged one free of overlaps and new holes;
//! `deleteRow`/`deleteColumn` conserve content and undo exactly, by the
//! one-line-at-a-time removal and by the one pass forced onto every table.
//!
//! `TABLE_PROP_SCALE=<n>` runs n times as many cases. The defaults (60 tiled,
//! 100 ragged, 120 delete cases, about 2 s in a debug build) are trimmed for
//! CI; the review ran 500, 800 and 3000.

use rinch_editor_core::commands::table_ops;
use rinch_editor_core::tables::{self, TableMap, is_table};
use rinch_editor_core::*;
use std::collections::HashSet;
use std::rc::Rc;

const HOLE: usize = usize::MAX;

/// `n` cases, times `TABLE_PROP_SCALE`.
fn cases(n: usize) -> usize {
    let scale = std::env::var("TABLE_PROP_SCALE")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(1);
    n * scale.max(1)
}

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

fn para(s: &Schema, text: &str) -> Node {
    if text.is_empty() {
        return s
            .create_node("paragraph", Attrs::new(), Fragment::empty())
            .unwrap();
    }
    s.branch("paragraph", Fragment::from_node(s.text(text).unwrap()))
        .unwrap()
}

/// A cell; `explicit` writes `colspan`/`rowspan` even when they are 1.
fn mk_cell(s: &Schema, ty: &str, cs: i64, rs: i64, text: &str, explicit: bool) -> Node {
    let attrs = if explicit || cs != 1 || rs != 1 {
        Attrs::from_iter([
            ("colspan", AttrValue::Int(cs)),
            ("rowspan", AttrValue::Int(rs)),
        ])
    } else {
        Attrs::new()
    };
    s.create_node(ty, attrs, Fragment::from_node(para(s, text)))
        .unwrap()
}

/// rows of (type, colspan, rowspan)
fn mk_table(s: &Schema, rows: &[Vec<(&str, i64, i64)>], explicit: bool) -> Node {
    let mut n = 0;
    let rows: Vec<Node> = rows
        .iter()
        .map(|r| {
            let cells = r
                .iter()
                .map(|&(ty, c, h)| {
                    n += 1;
                    mk_cell(s, ty, c, h, &format!("c{n}"), explicit)
                })
                .collect();
            s.create_node("table_row", Attrs::new(), Fragment::from_children(cells))
                .unwrap()
        })
        .collect();
    s.create_node("table", Attrs::new(), Fragment::from_children(rows))
        .unwrap()
}

fn state_with(t: Node) -> EditorState {
    let s = Schema::starter_kit();
    let tail = para(&s, "");
    let doc = s
        .branch("doc", Fragment::from_children(vec![t, tail]))
        .unwrap();
    EditorState::create(Rc::new(s), doc, default_plugins())
}

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

fn canon(n: &Node) -> String {
    let mut out = format!("{}{:?}", n.type_name(), n.attrs());
    if let Some(t) = n.text() {
        out.push_str(&format!("{t:?}"));
    }
    out.push('[');
    for c in n.content().children() {
        out.push_str(&canon(c));
    }
    out.push(']');
    out
}

fn span(c: &Node, name: &str) -> i64 {
    c.attrs().get_int(name).unwrap_or(1)
}

fn all_cells(t: &Node) -> Vec<(usize, usize)> {
    (0..t.child_count())
        .flat_map(|r| (0..t.child(r).child_count()).map(move |i| (r, i)))
        .collect()
}

fn selections(rng: &mut Rng, t: &Node, extra: usize) -> Vec<Selection> {
    let cells = all_cells(t);
    let mut out: Vec<Selection> = cells
        .iter()
        .map(|&(r, i)| Selection::cursor(Pos(cell_pos(t, r, i) + 2)))
        .collect();
    if cells.is_empty() {
        return out;
    }
    for _ in 0..extra {
        let (r, i) = cells[rng.below(cells.len())];
        let (r2, i2) = cells[rng.below(cells.len())];
        out.push(Selection::cell(
            Pos(cell_pos(t, r, i)),
            Pos(cell_pos(t, r2, i2)),
        ));
    }
    out
}

fn texts(n: &Node, out: &mut Vec<String>) {
    if let Some(t) = n.text() {
        out.push(t.to_string());
    }
    for c in n.content().children() {
        texts(c, out);
    }
}

// ------------------------------------------------------------------
// Adding rows.
// ------------------------------------------------------------------

fn random_tiled(rng: &mut Rng, s: &Schema, maxw: usize, maxh: usize, maxspan: usize) -> Node {
    let w = 1 + rng.below(maxw);
    let h = 1 + rng.below(maxh);
    let mut owner = vec![false; w * h];
    let mut rows: Vec<Vec<(&str, i64, i64)>> = vec![Vec::new(); h];
    for r in 0..h {
        for c in 0..w {
            if owner[r * w + c] {
                continue;
            }
            let mut cs = 1 + rng.below(maxspan);
            let mut rs = 1 + rng.below(maxspan);
            cs = cs.min(w - c);
            rs = rs.min(h - r);
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
            let ty = if rng.below(4) == 0 {
                "table_header_cell"
            } else {
                "table_cell"
            };
            rows[r].push((ty, cs as i64, rs as i64));
        }
    }
    mk_table(s, &rows, false)
}

/// Every cell in a slot, spans = its rect, no overlap; holes allowed iff
/// `holes_ok`. Returns Err on failure.
fn check_grid(t: &Node, holes_ok: bool) -> Result<(), String> {
    let map = TableMap::compute(t, 1);
    let holes = map.map().iter().filter(|&&p| p == HOLE).count();
    if !holes_ok && holes > 0 {
        return Err(format!("{holes} holes"));
    }
    let mut area = 0;
    for r in 0..t.child_count() {
        for i in 0..t.child(r).child_count() {
            let pos = cell_pos(t, r, i);
            let rect = map
                .find_cell(pos)
                .ok_or_else(|| format!("row {r} cell {i} off the grid"))?;
            let c = t.child(r).child(i);
            let (cs, rs) = (span(c, "colspan"), span(c, "rowspan"));
            if cs != (rect.right - rect.left) as i64 || rs != (rect.bottom - rect.top) as i64 {
                return Err(format!("row {r} cell {i}: {cs}x{rs} covers {rect:?}"));
            }
            if rect.top != r {
                return Err(format!("row {r} cell {i} starts in row {}", rect.top));
            }
            area += (cs * rs) as usize;
        }
    }
    if area + holes != map.map().len() {
        return Err(format!(
            "overlap: area {area} + holes {holes} != {}",
            map.map().len()
        ));
    }
    Ok(())
}

fn shape(t: &Node) -> Vec<Vec<(i64, i64)>> {
    (0..t.child_count())
        .map(|r| {
            (0..t.child(r).child_count())
                .map(|i| {
                    (
                        span(t.child(r).child(i), "colspan"),
                        span(t.child(r).child(i), "rowspan"),
                    )
                })
                .collect()
        })
        .collect()
}

/// The expected new row (colspans) for inserting at `row` beside `like` in
/// a hole-free well-formed table: crossing cells grow, the rest of `like`'s
/// cells are copied by width.
fn expected_new_row(t: &Node, row: usize, like: usize) -> Vec<i64> {
    let map = TableMap::compute(t, 1);
    let w = map.width();
    let m = map.map();
    let mut out = Vec::new();
    let mut col = 0;
    while col < w {
        let p = m[like * w + col];
        let rect = map.find_cell(p).unwrap();
        let crosses = row > 0 && row < map.height() && rect.top < row && rect.bottom > row;
        if !crosses {
            out.push((rect.right - rect.left) as i64);
        }
        col = rect.right;
    }
    out
}

fn sel_rect(t: &Node, sel: &Selection) -> (usize, usize) {
    // (top, bottom) of the selection's rect in grid rows
    let map = TableMap::compute(t, 1);
    match sel {
        Selection::Cell(c) => {
            let r = map.rect_between(c.anchor_cell.0, c.head_cell.0).unwrap();
            (r.top, r.bottom)
        }
        other => {
            let p = other.head().0;
            for (r, i) in all_cells(t) {
                let cp = cell_pos(t, r, i);
                if p > cp && p < cp + t.child(r).child(i).node_size() {
                    let rect = map.find_cell(cp).unwrap();
                    return (rect.top, rect.bottom);
                }
            }
            unreachable!()
        }
    }
}

#[test]
fn tiled_tables_add_row_copies_its_neighbour_and_stays_tiled() {
    let s = Schema::starter_kit();
    let mut rng = Rng(0x1876_0002_aaaa_bbbb);
    let mut checked = 0;
    let mut with_merge = 0;
    for case in 0..cases(60) {
        let t = random_tiled(&mut rng, &s, 6, 6, 4);
        check_grid(&t, false).unwrap();
        let base = state_with(t.clone());
        for sel in selections(&mut rng, &t, 10) {
            let (top, bottom) = sel_rect(&t, &sel);
            for (cmd, row, like) in [
                ("addRowBefore", top, top),
                ("addRowAfter", bottom, bottom - 1),
            ] {
                let mut st = base.clone();
                st.selection = sel.clone();
                let what = format!("case {case} {cmd} {sel:?} in {:?}", shape(&t));
                let next = st.run(cmd).unwrap_or_else(|| panic!("{what}: refused"));
                let nt = next.doc.child(0);
                check_grid(nt, false)
                    .unwrap_or_else(|e| panic!("{what}: {e}\n -> {:?}", shape(nt)));
                let new_row: Vec<i64> = (0..nt.child(row).child_count())
                    .map(|i| span(nt.child(row).child(i), "colspan"))
                    .collect();
                let want = expected_new_row(&t, row, like);
                assert_eq!(new_row, want, "{what}");
                assert!(
                    (0..nt.child(row).child_count())
                        .all(|i| nt.child(row).child(i).type_name() == "table_cell"
                            && span(nt.child(row).child(i), "rowspan") == 1),
                    "{what}"
                );
                if want.iter().any(|&c| c > 1) {
                    with_merge += 1;
                }
                // undo/redo
                let undone = next.run("undo").expect("undo");
                assert_eq!(canon(&undone.doc), canon(&st.doc), "{what}: undo");
                let redone = undone.run("redo").expect("redo");
                assert_eq!(canon(&redone.doc), canon(&next.doc), "{what}: redo");
                checked += 1;
            }
        }
    }
    eprintln!("checked {checked}, with a merged new cell {with_merge}");
    assert!(
        checked > 1000 && with_merge > 200,
        "control: {checked} / {with_merge}"
    );
}

/// Ragged (holes at row ends) but non-overlapping tables: the new row has no
/// hole, no cell overlaps, nothing goes off grid.
#[test]
fn ragged_non_overlapping_tables_add_row_is_full() {
    let s = Schema::starter_kit();
    let mut rng = Rng(0x1876_0003_cccc_dddd);
    let mut checked = 0;
    let mut chunked = 0;
    for case in 0..cases(100) {
        let t = random_tiled(&mut rng, &s, 7, 6, 3);
        // Drop trailing cells with rowspan 1 from random rows (keeps no overlap).
        let mut rows: Vec<Node> = t.content().children().to_vec();
        for row in rows.iter_mut() {
            if rng.below(2) == 0 {
                continue;
            }
            let mut cells: Vec<Node> = row.content().children().to_vec();
            let k = rng.below(cells.len() + 1);
            for _ in 0..k {
                match cells.last() {
                    Some(c) if span(c, "rowspan") == 1 => {
                        cells.pop();
                    }
                    _ => break,
                }
            }
            *row = row.copy_with_content(Fragment::from_children(cells));
        }
        let t = t.copy_with_content(Fragment::from_children(rows));
        if check_grid(&t, true).is_err() {
            continue; // dropping changed placement; skip
        }
        let base = state_with(t.clone());
        for sel in selections(&mut rng, &t, 4) {
            for cmd in ["addRowBefore", "addRowAfter"] {
                let mut st = base.clone();
                st.selection = sel.clone();
                let what = format!("case {case} {cmd} {sel:?} in {:?}", shape(&t));
                let Some(next) = st.run(cmd) else { continue };
                let nt = next.doc.child(0);
                check_grid(nt, true).unwrap_or_else(|e| panic!("{what}: {e} -> {:?}", shape(nt)));
                // find the new row: the one whose map row has... compare holes per row
                let before = tables::row_holes(&t);
                let after = tables::row_holes(nt);
                assert_eq!(after.len(), before.len() + 1);
                // the inserted row index: first index where texts differ
                let (top, bottom) = sel_rect(&t, &sel);
                let idx = if cmd == "addRowBefore" { top } else { bottom };
                // A row could also differ because rowspans grew; the new row has only empty cells.
                let new_row = nt.child(idx);
                let mut tx = Vec::new();
                texts(new_row, &mut tx);
                assert!(tx.is_empty(), "{what}: idx {idx} not the new row");
                assert_eq!(
                    after[idx],
                    0,
                    "{what}: new row has holes -> {:?}",
                    shape(nt)
                );
                if (0..new_row.child_count()).any(|i| span(new_row.child(i), "colspan") > 1) {
                    chunked += 1;
                }
                checked += 1;
            }
        }
    }
    eprintln!("ragged checked {checked}, new row merged {chunked}");
    assert!(checked > 300, "control: {checked}");
}

// ------------------------------------------------------------------
// Deleting rows and columns.
// ------------------------------------------------------------------

const SPANS: &[i64] = &[1, 1, 1, 1, 2, 3, 0, -1, 40];

fn random_ragged(rng: &mut Rng, s: &Schema) -> Node {
    let h = 1 + rng.below(6);
    let rows: Vec<Vec<(&str, i64, i64)>> = (0..h)
        .map(|_| {
            let n = rng.below(6);
            (0..n)
                .map(|_| {
                    (
                        "table_cell",
                        SPANS[rng.below(SPANS.len())],
                        SPANS[rng.below(SPANS.len())],
                    )
                })
                .collect()
        })
        .collect();
    mk_table(s, &rows, true)
}

/// Every delete conserves the table's text (nothing duplicated or made up),
/// undoes and redoes exactly, and keeps a well-formed table well-formed and
/// a non-overlapping one non-overlapping. `forced` runs the one-pass
/// removal on every table, the path #1874 takes past its slot budget.
fn deletes_conserve_content_and_undo(forced: bool) {
    table_ops::set_fallback_slot_budget_for_tests(forced.then_some(0));
    let s = Schema::starter_kit();
    let mut rng = Rng(0x1876_0004_eeee_ffff);
    let mut applied = 0;
    for case in 0..cases(120) {
        let t = if case % 3 == 0 {
            random_tiled(&mut rng, &s, 6, 6, 3)
        } else {
            random_ragged(&mut rng, &s)
        };
        let base = state_with(t.clone());
        let mut before = Vec::new();
        texts(&t, &mut before);
        for sel in selections(&mut rng, &t, 6) {
            for cmd in ["deleteRow", "deleteColumn"] {
                let mut st = base.clone();
                st.selection = sel.clone();
                let what = format!("case {case} {cmd} {sel:?} in {:?}", shape(&t));
                let out = match st.run(cmd) {
                    None => "refused".to_string(),
                    Some(next) => {
                        applied += 1;
                        let mut after = Vec::new();
                        if let Some(nt) = next.doc.content().children().iter().find(|n| is_table(n))
                        {
                            texts(nt, &mut after);
                            for (i, n) in next.doc.content().children().iter().enumerate() {
                                if is_table(n) {
                                    let _ = TableMap::compute(n, 1 + i);
                                }
                            }
                        }
                        let mut seen = HashSet::new();
                        for x in &after {
                            assert!(seen.insert(x.clone()), "{what}: {x} duplicated");
                            assert!(before.contains(x), "{what}: {x} invented");
                        }
                        let undone = next.run("undo").expect("undo");
                        assert_eq!(canon(&undone.doc), canon(&st.doc), "{what}: undo");
                        let redone = undone.run("redo").expect("redo");
                        assert_eq!(canon(&redone.doc), canon(&next.doc), "{what}: redo");
                        // a well-formed input stays well-formed
                        if case % 3 == 0
                            && let Some(nt) =
                                next.doc.content().children().iter().find(|n| is_table(n))
                        {
                            check_grid(nt, false).unwrap_or_else(|e| panic!("{what}: {e}"));
                        }
                        // a non-overlapping ragged input stays non-overlapping
                        if check_grid(&t, true).is_ok()
                            && let Some(nt) =
                                next.doc.content().children().iter().find(|n| is_table(n))
                        {
                            check_grid(nt, true)
                                .unwrap_or_else(|e| panic!("{what}: {e} -> {:?}", shape(nt)));
                        }
                        canon(&next.doc)
                    }
                };
                let _ = out;
            }
        }
    }
    table_ops::set_fallback_slot_budget_for_tests(None);
    assert!(applied > 300, "control: {applied} deletes applied");
}

#[test]
fn deletes_conserve_content_and_undo_one_line_at_a_time() {
    deletes_conserve_content_and_undo(false);
}

#[test]
fn deletes_conserve_content_and_undo_in_one_pass() {
    deletes_conserve_content_and_undo(true);
}
