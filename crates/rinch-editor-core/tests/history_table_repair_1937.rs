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
