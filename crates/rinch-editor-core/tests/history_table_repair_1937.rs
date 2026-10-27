//! #1937: an undo or redo among a peer's table changes leaves every table tiling its
//! grid. The steps an undo reverts are mapped through the changes made since (a
//! peer's, which the history did not record), and a mapped table command can apply
//! in part: a `mergeCells` undone after a peer's merge restored cells into the
//! peer's table and kept the spans. The undo repairs the tables it made ragged in
//! its own transaction (#1931's `repair_table`), as #1847 decided for every edit,
//! and leaves alone a table that was ragged before it.
//!
//! A randomized history over one 3 × 3 table: merges, splits, rows and columns
//! added and deleted, a cell attribute and typing, each mine (recorded) or a
//! peer's (the table replaced whole, unrecorded, as a collaboration integrate
//! brings a change of the grid), with undo and redo. After every undo and redo the
//! table tiles. `RINCH_UNDO_TILE_SEEDS` runs more seeds.

use std::rc::Rc;

use rinch_editor_core::serialize::{node_to_html, slice_from_html};
use rinch_editor_core::tables::table_problems;
use rinch_editor_core::{
    AttrValue, EditorState, Fragment, Node, Pos, Schema, Selection, Slice, default_plugins,
};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

fn state() -> EditorState {
    let schema = Rc::new(Schema::starter_kit());
    let html = "<p>a</p><table>\
        <tr><td><p>c00</p></td><td><p>c01</p></td><td><p>c02</p></td></tr>\
        <tr><td><p>c10</p></td><td><p>c11</p></td><td><p>c12</p></td></tr>\
        <tr><td><p>c20</p></td><td><p>c21</p></td><td><p>c22</p></td></tr>\
        </table><p>z</p>";
    let slice = slice_from_html(&schema, html).unwrap();
    let doc = schema.branch("doc", slice.content.clone()).unwrap();
    EditorState::create(schema, doc, default_plugins())
}

/// The position before each cell of `doc`, in document order.
fn cells(doc: &Node) -> Vec<usize> {
    let mut out = Vec::new();
    doc.nodes_between(0, doc.content_size(), &mut |n: &Node, at, _| {
        if n.type_name().ends_with("cell") {
            out.push(at);
        }
        true
    });
    out
}

/// The first table of `doc` and the position before it.
fn table_at(doc: &Node) -> Option<(usize, Node)> {
    let mut found = None;
    doc.nodes_between(0, doc.content_size(), &mut |n: &Node, at, _| {
        if found.is_none() && n.type_name() == "table" {
            found = Some((at, n.clone()));
        }
        found.is_none()
    });
    found
}

/// Every problem of every table of `doc`.
fn problems(doc: &Node) -> Vec<String> {
    let mut out = Vec::new();
    doc.nodes_between(0, doc.content_size(), &mut |n: &Node, at, _| {
        if n.type_name() == "table" {
            out.extend(
                table_problems(n)
                    .iter()
                    .map(|p| format!("table at {at}: {p:?}")),
            );
        }
        true
    });
    out
}

const COMMANDS: [&str; 6] = [
    "splitCell",
    "addRowAfter",
    "addColumnBefore",
    "deleteRow",
    "deleteColumn",
    "mergeCells",
];

/// One random change of `st`, mine or (`remote`) a peer's: a table command with a
/// cell selection or a caret in a cell, a cell attribute, or a keystroke.
fn change(rng: &mut Rng, st: &EditorState, remote: bool, time: u64) -> Option<EditorState> {
    let cs = cells(&st.doc);
    if cs.is_empty() {
        return None; // the table was deleted
    }
    let pick = rng.below(8);
    let mine = if pick < 6 {
        let (a, b) = (cs[rng.below(cs.len())], cs[rng.below(cs.len())]);
        let mut tr = st.tr();
        tr.set_time(time);
        if COMMANDS[pick] == "mergeCells" {
            if a == b {
                return None;
            }
            tr.set_selection(Selection::cell(Pos(a), Pos(b)));
        } else {
            tr.set_selection(Selection::cursor(Pos(a + 2)));
        }
        st.apply(tr).run(COMMANDS[pick])?
    } else if pick == 6 {
        let mut tr = st.tr();
        tr.set_time(time);
        let v = format!("n{}", rng.below(3));
        tr.set_node_attr(
            cs[rng.below(cs.len())],
            "data-note",
            AttrValue::from(v.as_str()),
        )
        .ok()?;
        st.apply(tr)
    } else {
        let at = cs[rng.below(cs.len())] + 2;
        let mut tr = st.tr();
        tr.set_time(time);
        tr.set_selection(Selection::cursor(Pos(at)));
        tr.insert_text("x").ok()?;
        st.apply(tr)
    };
    if !remote {
        return Some(mine);
    }
    // A peer's: the table replaced whole, unrecorded.
    let (at, old) = table_at(&st.doc)?;
    let (_, new) = table_at(&mine.doc)?;
    let mut tr = st.tr();
    tr.replace(
        at,
        at + old.node_size(),
        Slice::new(Fragment::from_node(new), 0, 0),
    )
    .ok()?;
    tr.set_add_to_history(false);
    Some(st.apply(tr))
}

/// One history of `n` changes, undos and redos. Answers how many undos and redos
/// it checked.
fn trial(seed: u64, n: usize) -> usize {
    let mut rng = Rng(seed ^ 0x9E37_79B9_7F4A_7C15);
    let mut st = state();
    let mut checked = 0;
    for t in 0..n {
        let roll = rng.below(10);
        let next = if roll < 3 {
            let cmd = if roll < 2 { "undo" } else { "redo" };
            let Some(next) = st.run(cmd) else { continue };
            assert_eq!(
                problems(&next.doc),
                Vec::<String>::new(),
                "seed {seed}, step {t}: the {cmd} left a ragged table: {}",
                node_to_html(&next.doc)
            );
            checked += 1;
            next
        } else {
            let remote = rng.below(2) == 0;
            match change(&mut rng, &st, remote, t as u64 * 1000) {
                Some(next) => next,
                None => continue,
            }
        };
        st = next;
    }
    checked
}

#[test]
fn undo_and_redo_among_a_peers_table_changes_leave_tables_tiling() {
    let seeds: u64 = std::env::var("RINCH_UNDO_TILE_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(400);
    let checked: usize = (1..=seeds).map(|seed| trial(seed, 30)).sum();
    // The positive control: the trials undo and redo.
    assert!(
        checked >= seeds as usize * 3,
        "only {checked} undos and redos checked"
    );
}

// ─── Review of #1950 ───────────────────────────────────────────────────────

/// The first table of a document of `<p>a</p>`, a table and `<p>z</p>` replaced
/// by `table_html`'s, unrecorded: a peer's change as an integrate brings it.
fn peer_table(st: &EditorState, table_html: &str) -> EditorState {
    let schema = st.schema().clone();
    let slice = slice_from_html(&schema, &format!("<p>a</p>{table_html}<p>z</p>")).unwrap();
    let new = schema
        .branch("doc", slice.content.clone())
        .unwrap()
        .child(1)
        .clone();
    let (at, old) = table_at(&st.doc).unwrap();
    let mut tr = st.tr();
    tr.replace(
        at,
        at + old.node_size(),
        Slice::new(Fragment::from_node(new), 0, 0),
    )
    .unwrap();
    tr.set_add_to_history(false);
    st.apply(tr)
}

fn doc_state(html: &str) -> EditorState {
    let schema = Rc::new(Schema::starter_kit());
    let slice = slice_from_html(&schema, html).unwrap();
    let doc = schema.branch("doc", slice.content.clone()).unwrap();
    EditorState::create(schema, doc, default_plugins())
}

/// I paste "XY" after "a" in the first cell. A peer adds a column after it and
/// moves "Y" into the new cell. My undo deletes "XY": mapped through the peer's
/// change, from the first cell into the new one, and joins the two cells. The
/// first round skipped every undo of typing without looking (review of #1950;
/// its differential found 3 such in 5,000 seeds).
#[test]
fn the_undo_of_my_paste_after_a_peer_moved_half_of_it_into_a_new_column() {
    let st = doc_state(
        "<p>a</p><table><tr><td><p>a</p></td><td><p>b</p></td></tr>\
         <tr><td><p>c</p></td><td><p>d</p></td></tr></table><p>z</p>",
    );
    let mut tr = st.tr();
    // After "a" in the first cell: <p>a</p> is 3, the table, row, cell and
    // paragraph open, then "a".
    tr.set_selection(Selection::cursor(Pos(3 + 4 + 1)));
    tr.insert_text("XY").unwrap();
    let mine = st.apply(tr);
    let theirs = peer_table(
        &mine,
        "<table><tr><td><p>aX</p></td><td><p>Y</p></td><td><p>b</p></td></tr>\
         <tr><td><p>c</p></td><td><p></p></td><td><p>d</p></td></tr></table>",
    );
    assert_eq!(problems(&theirs.doc), Vec::<String>::new(), "the setup");
    let undone = theirs.run("undo").expect("the undo applies");
    assert_eq!(
        problems(&undone.doc),
        Vec::<String>::new(),
        "{}",
        node_to_html(&undone.doc)
    );
}

/// I delete from inside the first row's cell into the second row's, in a
/// one-column table (it still tiles: the two rows join). A peer adds a column;
/// my undo puts the deleted rows back as a slice open at both ends, at one point
/// inside a cell, which splits the table's row around it. The first round looked
/// only inside the slice's range (review of #1950).
#[test]
fn the_undo_of_a_delete_across_rows_puts_an_open_slice_in_a_cell() {
    let st = doc_state(
        "<p>a</p><table><tr><td><p>abc</p></td></tr><tr><td><p>def</p></td></tr></table><p>z</p>",
    );
    // "abc" is 7..10 and "def" 16..19: from after "a" to after "de".
    let mut tr = st.tr();
    tr.delete(8, 18).unwrap();
    let mine = st.apply(tr);
    assert_eq!(problems(&mine.doc), Vec::<String>::new(), "the delete");
    let theirs = peer_table(
        &mine,
        "<table><tr><td><p></p></td><td><p>af</p></td></tr></table>",
    );
    let undone = theirs.run("undo").expect("the undo applies");
    assert_eq!(
        problems(&undone.doc),
        Vec::<String>::new(),
        "{}",
        node_to_html(&undone.doc)
    );
}

/// A richer random change than [`change`]'s, from the review of #1950: table
/// commands, typing and pasting at one point, a hard break, Enter, a delete inside
/// a textblock, text moved from one textblock to another, a delete from any text
/// position to any other. Not on the pre-ragged table (`skip`, its position).
fn one_op(rng: &mut Rng, st: &EditorState, time: u64) -> Option<EditorState> {
    let cs = cells(&st.doc);
    if cs.is_empty() {
        return None;
    }
    let mut tps = Vec::new();
    st.doc
        .nodes_between(0, st.doc.content_size(), &mut |n: &Node, at, _| {
            if n.is_textblock() {
                let s = at + 1;
                let e = s + n.content_size();
                tps.extend((s..=e).map(|p| (p, s, e)));
            }
            true
        });
    let pick = rng.below(13);
    let mut tr = st.tr();
    tr.set_time(time);
    match pick {
        0..=5 => {
            let (a, b) = (cs[rng.below(cs.len())], cs[rng.below(cs.len())]);
            if COMMANDS[pick] == "mergeCells" {
                if a == b {
                    return None;
                }
                tr.set_selection(Selection::cell(Pos(a), Pos(b)));
            } else {
                tr.set_selection(Selection::cursor(Pos(a + 2)));
            }
            return st.apply(tr).run(COMMANDS[pick]);
        }
        6 | 7 => {
            let (p, _, _) = tps[rng.below(tps.len())];
            let n = 1 + rng.below(3);
            let t: String = (0..n).map(|i| (b'P' + i as u8) as char).collect();
            tr.set_selection(Selection::cursor(Pos(p)));
            tr.insert_text(&t).ok()?;
        }
        8 => {
            let (p, _, _) = tps[rng.below(tps.len())];
            let hb = st.schema().branch("hard_break", Fragment::empty()).ok()?;
            tr.replace(p, p, Slice::new(Fragment::from_node(hb), 0, 0))
                .ok()?;
        }
        9 => {
            let (p, _, _) = tps[rng.below(tps.len())];
            tr.split(p, 1, None).ok()?;
        }
        10 => {
            let (p, _, e) = tps[rng.below(tps.len())];
            if e <= p {
                return None;
            }
            let q = p + 1 + rng.below(e - p);
            tr.delete(p, q).ok()?;
        }
        11 => {
            let (p, _, e) = tps[rng.below(tps.len())];
            if e <= p {
                return None;
            }
            let q = p + 1 + rng.below(e - p);
            let sl = st.doc.slice(p, q).ok()?;
            let (d, _, _) = tps[rng.below(tps.len())];
            if d >= p && d <= q {
                return None;
            }
            if d > q {
                tr.replace(d, d, sl).ok()?;
                tr.delete(p, q).ok()?;
            } else {
                tr.delete(p, q).ok()?;
                tr.replace(d, d, sl).ok()?;
            }
        }
        _ => {
            let (p, _, _) = tps[rng.below(tps.len())];
            let (q, _, _) = tps[rng.below(tps.len())];
            if p == q {
                return None;
            }
            tr.delete(p.min(q), p.max(q)).ok()?;
        }
    }
    Some(st.apply(tr))
}

/// How many tables of `doc` fail to tile.
fn ragged_tables(doc: &Node) -> usize {
    let mut n = 0;
    doc.nodes_between(0, doc.content_size(), &mut |node: &Node, _, _| {
        if node.type_name() == "table" && !table_problems(node).is_empty() {
            n += 1;
        }
        true
    });
    n
}

/// A change of mine (refused when it leaves a table ragged that tiled: the
/// editing paths repair those since #1847, and the history records only what
/// tiles), or a peer's: one to three of [`one_op`] on a copy, the first table
/// replaced whole when it still tiles and nothing else changed.
fn rich_change(rng: &mut Rng, st: &EditorState, remote: bool, time: u64) -> Option<EditorState> {
    if !remote {
        let n = one_op(rng, st, time)?;
        return (ragged_tables(&n.doc) <= ragged_tables(&st.doc)
            && table_at(&n.doc).is_some_and(|(_, t)| table_problems(&t).is_empty()))
        .then_some(n);
    }
    let mut copy = st.clone();
    for _ in 0..1 + rng.below(3) {
        if let Some(n) = one_op(rng, &copy, time) {
            copy = n;
        }
    }
    let (at, old) = table_at(&st.doc)?;
    let (_, new) = table_at(&copy.doc)?;
    if !table_problems(&new).is_empty() || st.doc.child_count() != copy.doc.child_count() {
        return None;
    }
    let mut tr = st.tr();
    tr.replace(
        at,
        at + old.node_size(),
        Slice::new(Fragment::from_node(new), 0, 0),
    )
    .ok()?;
    tr.set_add_to_history(false);
    Some(st.apply(tr))
}

/// One history from `start`, its document after every step, with the
/// repair looking where the steps can have changed a grid (`full` 0) or at every
/// table (`full` 1; `full` 2 after local undos too).
fn rich_trial(seed: u64, start: &EditorState, full: u8) -> Vec<String> {
    rinch_editor_core::state::transaction::__set_full_repair_scan(full);
    let mut rng = Rng(seed ^ 0x9E37_79B9_7F4A_7C15);
    let mut st = start.clone();
    let mut log = Vec::new();
    for t in 0..30 {
        let roll = rng.below(10);
        let next = if roll < 3 {
            let cmd = if roll < 2 { "undo" } else { "redo" };
            match st.run(cmd) {
                Some(next) => next,
                None => continue,
            }
        } else {
            let remote = rng.below(2) == 0;
            match rich_change(&mut rng, &st, remote, t as u64 * 1000) {
                Some(next) => next,
                None => continue,
            }
        };
        st = next;
        log.push(format!("{t}: {}", node_to_html(&st.doc)));
    }
    rinch_editor_core::state::transaction::__set_full_repair_scan(0);
    log
}

/// The differential (review of #1950): one random history, each undo and redo
/// repaired by looking where its steps can have changed a grid, and by looking at
/// every table. The two must give the same document at every step. The first
/// round differed in 3 of 5,000 seeds (489, 1678, 2054: an undo of typing a peer
/// had split across cells), and in the open-slice case. The full scan also runs
/// after local undos and redos, which the repair skips: one restores the document
/// before the event, table for table.
///
/// With `ragged_before`, a second table that does not tile sits after the first,
/// and must stay as it is either way: the exclusion of a table that was ragged
/// before the undo. There the full scan runs only where the repair runs: a local
/// undo can bring that table back after a delete took it away, ragged as it was
/// before the event, and a scan of the undo's own document cannot tell it from a
/// table the undo made.
fn differential(ragged_before: bool) {
    let seeds: u64 = std::env::var("RINCH_UNDO_TILE_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(400);
    let start = if ragged_before {
        doc_state(
            "<p>a</p><table>\
             <tr><td><p>c00</p></td><td><p>c01</p></td><td><p>c02</p></td></tr>\
             <tr><td><p>c10</p></td><td><p>c11</p></td><td><p>c12</p></td></tr>\
             <tr><td><p>c20</p></td><td><p>c21</p></td><td><p>c22</p></td></tr>\
             </table><p>m</p><table><tr><td><p>r0</p></td><td><p>r1</p></td></tr>\
             <tr><td><p>short</p></td></tr></table><p>z</p>",
        )
    } else {
        state()
    };
    let ragged_at_start = ragged_tables(&start.doc);
    let mut steps = 0;
    for seed in 1..=seeds {
        let looked = rich_trial(seed, &start, 0);
        let full = rich_trial(seed, &start, if ragged_before { 1 } else { 2 });
        if let Some(i) = looked.iter().zip(&full).position(|(a, b)| a != b) {
            panic!(
                "seed {seed}: the repair differs from a full scan at step {i}\n  \
                 looked: {}\n  full:   {}",
                looked[i], full[i]
            );
        }
        assert_eq!(looked.len(), full.len(), "seed {seed}");
        steps += looked.len();
    }
    assert!(steps >= seeds as usize * 10, "only {steps} steps");
    if ragged_before {
        assert_eq!(ragged_at_start, 1, "the setup");
    }
}

#[test]
fn the_repair_finds_what_a_full_scan_finds() {
    differential(false);
}

#[test]
fn the_repair_finds_what_a_full_scan_finds_beside_a_table_ragged_before() {
    differential(true);
}

/// I delete the first row; a peer widens the cell left in the first column
/// (`colspan = 2`), an attribute alone, which moves no position, so the history
/// keeps no map of it; I undo, and the row comes back two cells wide beside a row
/// three wide. With no map after my event the undo looked like a local one,
/// which restores the document as it was and checks nothing (the differential,
/// seed 754).
#[test]
fn the_undo_after_a_peers_span_change_that_moved_no_position() {
    let st = doc_state(
        "<p>a</p><table><tr><td><p>a</p></td><td><p>b</p></td></tr>\
         <tr><td><p>c</p></td><td><p>d</p></td></tr></table><p>z</p>",
    );
    let mut tr = st.tr();
    tr.set_selection(Selection::cursor(Pos(3 + 4)));
    let mine = st.apply(tr).run("deleteRow").expect("the row goes");
    let theirs = peer_table(
        &mine,
        "<table><tr><td colspan=\"2\"><p>c</p></td><td><p>d</p></td></tr></table>",
    );
    assert_eq!(problems(&theirs.doc), Vec::<String>::new(), "the setup");
    let undone = theirs.run("undo").expect("the undo applies");
    assert_eq!(
        problems(&undone.doc),
        Vec::<String>::new(),
        "{}",
        node_to_html(&undone.doc)
    );
}
