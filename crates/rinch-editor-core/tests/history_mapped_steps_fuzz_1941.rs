//! Undo and redo among a peer's changes with every step type (the review of
//! #1938's fuzz): no panic, and every selection inside its document. At main
//! before #1941's fix, seed 1159 panicked in `ReplaceAroundStep::invert`
//! (a collapsed unwrap redone across a peer's insert); seeds 40693 and 48017
//! are #1942's (an undo's selection outside the document).
//!
//! `RINCH_HIST_FUZZ_START`, `RINCH_HIST_FUZZ_SEEDS` and `RINCH_HIST_FUZZ_OPS`
//! widen the search (5000 seeds clean); `RINCH_HIST_FUZZ_LOUD` prints panics.
use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use rinch_editor_core::serialize::slice_from_html;
use rinch_editor_core::{
    BaseCommandsPlugin, EditorState, Fragment, HistoryPlugin, Node, Plugin, Pos, Schema, Selection,
    Slice, Step, Transaction,
};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.next() % 100 < pct
    }
}

thread_local! {
    static SCHEMA: Rc<Schema> = Rc::new(Schema::starter_kit());
}
fn schema() -> Rc<Schema> {
    SCHEMA.with(|s| s.clone())
}

fn plugins(depth: usize) -> Vec<Rc<dyn Plugin>> {
    vec![
        Rc::new(BaseCommandsPlugin),
        Rc::new(HistoryPlugin::with(500, depth)),
    ]
}

const INITS: &[&str] = &[
    "<p>a</p><table>\
     <tr><td><p>c00</p></td><td><p>c01</p></td><td><p>c02</p></td></tr>\
     <tr><td><p>c10</p></td><td><p>c11</p></td><td><p>c12</p></td></tr>\
     <tr><td><p>c20</p></td><td><p>c21</p></td><td><p>c22</p></td></tr>\
     </table><p>z</p>",
    "<p>ab</p><table><tr><th><p>h0</p></th><th><p>h1</p></th></tr>\
     <tr><td><p>x</p></td><td><p>y</p></td></tr></table>\
     <ul><li><p>ef</p></li><li><p>gh</p></li></ul><blockquote><p>q</p></blockquote>",
    "<p>ab</p><p>cd</p><ul><li><p>ef</p></li></ul><table><tr><td><p>1</p></td><td><p>2</p></td>\
     <td><p>3</p></td><td><p>4</p></td></tr><tr><td colspan=\"2\"><p>5</p></td><td><p>6</p></td>\
     <td><p>7</p></td></tr></table><p>gh</p>",
];

fn state(html: &str, depth: usize) -> EditorState {
    let schema = schema();
    let slice = slice_from_html(&schema, html).unwrap();
    let doc = schema.branch("doc", slice.content.clone()).unwrap();
    let mut st = EditorState::create(schema, doc, plugins(depth));
    st.selection = Selection::cursor(Pos(1));
    st
}

fn text_positions(doc: &Node) -> Vec<usize> {
    (0..=doc.content_size())
        .filter(|&p| doc.resolve(Pos(p)).is_ok_and(|r| r.parent().is_textblock()))
        .collect()
}

/// Cells, grouped by the table they are in (by the table's position).
fn cells(doc: &Node) -> Vec<Vec<usize>> {
    let mut tables: Vec<(usize, Vec<usize>)> = Vec::new();
    doc.nodes_between(0, doc.content_size(), &mut |n: &Node, at, _| {
        if n.type_name() == "table" {
            tables.push((at, Vec::new()));
        } else if (n.type_name().ends_with("cell") || n.type_name().ends_with("header"))
            && let Some(t) = tables.last_mut()
        {
            t.1.push(at);
        }
        true
    });
    tables
        .into_iter()
        .map(|t| t.1)
        .filter(|c| !c.is_empty())
        .collect()
}

const CMDS: &[&str] = &[
    "splitBlock",
    "enter",
    "toggleBold",
    "toggleItalic",
    "toggleCode",
    "toggleHighlight",
    "setCodeBlock",
    "setParagraph",
    "setTextAlignCenter",
    "setTextAlignRight",
    "wrapInBlockquote",
    "toggleBulletList",
    "toggleOrderedList",
    "toggleTaskList",
    "sinkListItem",
    "liftListItem",
    "indent",
    "outdent",
    "insertHardBreak",
    "insertHorizontalRule",
    "insertTable",
    "deleteCharBackward",
    "deleteCharForward",
    "deleteWordBackward",
    "deleteSelection",
    "selectAll",
];

const TABLE_CMDS: &[&str] = &[
    "mergeCells",
    "mergeCells",
    "mergeCells",
    "splitCell",
    "splitCell",
    "addRowBefore",
    "addRowAfter",
    "addColumnBefore",
    "addColumnAfter",
    "deleteRow",
    "deleteColumn",
    "deleteCellSelection",
    "deleteTable",
];

/// Run a command, capturing its transaction's steps.
fn run_cmd(st: &EditorState, name: &str, time: u64) -> Option<(EditorState, Vec<Box<dyn Step>>)> {
    let cmd = st.command(name)?;
    let got: RefCell<Option<Transaction>> = RefCell::new(None);
    let applied = {
        let mut d = |mut tr: Transaction| {
            tr.set_time(time);
            *got.borrow_mut() = Some(tr);
        };
        cmd(st, Some(&mut d))
    };
    if !applied {
        return None;
    }
    let tr = got.into_inner()?;
    let steps: Vec<Box<dyn Step>> = tr.steps().to_vec();
    Some((st.apply(tr), steps))
}

fn edit(
    rng: &mut Rng,
    st: &EditorState,
    time: u64,
    upper: bool,
) -> Option<(EditorState, Vec<Box<dyn Step>>)> {
    let base = if upper { b'A' } else { b'a' };
    match rng.below(12) {
        0..=2 => {
            let tp = text_positions(&st.doc);
            if tp.is_empty() {
                return None;
            }
            let at = tp[rng.below(tp.len())];
            let s: String = (0..1 + rng.below(3))
                .map(|_| (base + rng.below(4) as u8) as char)
                .collect();
            let mut tr = st.tr();
            tr.set_time(time);
            tr.set_selection(Selection::cursor(Pos(at)));
            tr.insert_text(&s).ok()?;
            let steps = tr.steps().to_vec();
            Some((st.apply(tr), steps))
        }
        3 => {
            let tp = text_positions(&st.doc);
            if tp.is_empty() {
                return None;
            }
            let a = tp[rng.below(tp.len())];
            let r = st.doc.resolve(Pos(a)).ok()?;
            let end = a - r.parent_offset() + r.parent().content_size();
            if end <= a {
                return None;
            }
            let b = a + 1 + rng.below(end - a);
            let mut tr = st.tr();
            tr.set_time(time);
            tr.delete(a, b).ok()?;
            let steps = tr.steps().to_vec();
            Some((st.apply(tr), steps))
        }
        4..=7 => {
            let tabs = cells(&st.doc);
            if tabs.is_empty() {
                return None;
            }
            let t = &tabs[rng.below(tabs.len())];
            let a = t[rng.below(t.len())];
            let b = t[rng.below(t.len())];
            let mut s = st.clone();
            s.selection = if rng.chance(80) {
                Selection::cell(Pos(a), Pos(b))
            } else {
                Selection::cursor(Pos(a + 2))
            };
            let name = TABLE_CMDS[rng.below(TABLE_CMDS.len())];
            run_cmd(&s, name, time)
        }
        _ => {
            let tp = text_positions(&st.doc);
            if tp.is_empty() {
                return None;
            }
            let a = tp[rng.below(tp.len())];
            let mut s = st.clone();
            s.selection = if rng.chance(40) {
                let b = tp[rng.below(tp.len())];
                Selection::text(Pos(a), Pos(b))
            } else {
                Selection::cursor(Pos(a))
            };
            let name = CMDS[rng.below(CMDS.len())];
            run_cmd(&s, name, time)
        }
    }
}

/// Apply `target` the way rinch-editor-collab's `replace_changed_blocks` does.
fn collab_like(st: &EditorState, target: &Node) -> EditorState {
    let old = &st.doc;
    let (on, nn) = (old.child_count(), target.child_count());
    let mut prefix = 0;
    while prefix < on && prefix < nn && old.child(prefix) == target.child(prefix) {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < on - prefix
        && suffix < nn - prefix
        && old.child(on - 1 - suffix) == target.child(nn - 1 - suffix)
    {
        suffix += 1;
    }
    if prefix == on && on == nn {
        return st.clone();
    }
    let start: usize = (0..prefix).map(|i| old.child(i).node_size()).sum();
    let end: usize = start
        + (prefix..on - suffix)
            .map(|i| old.child(i).node_size())
            .sum::<usize>();
    let mid: Vec<Node> = (prefix..nn - suffix)
        .map(|j| target.child(j).clone())
        .collect();
    let mut tr = st.tr();
    tr.replace(start, end, Slice::new(Fragment::from_children(mid), 0, 0))
        .unwrap();
    tr.set_add_to_history(false);
    st.apply(tr)
}

fn peer(rng: &mut Rng, st: &EditorState) -> EditorState {
    let mut p = EditorState::create(schema(), st.doc.clone(), vec![Rc::new(BaseCommandsPlugin)]);
    let mut steps: Vec<Box<dyn Step>> = Vec::new();
    for _ in 0..1 + rng.below(3) {
        if let Some((n, s)) = edit(rng, &p, 0, true) {
            p = n;
            steps.extend(s);
        }
    }
    if steps.is_empty() {
        return st.clone();
    }
    match rng.below(3) {
        0 => {
            // Exact: the peer's own steps, unrecorded.
            let mut tr = st.tr();
            for s in &steps {
                tr.step(s.clone()).expect("peer replay");
            }
            tr.set_add_to_history(false);
            st.apply(tr)
        }
        _ => collab_like(st, &p.doc),
    }
}

fn trial(seed: u64, ops: usize) {
    let mut rng = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
    let depth = if rng.chance(20) { 3 } else { 100 };
    let mut st = state(INITS[rng.below(INITS.len())], depth);
    let mut time = 0u64;
    for _ in 0..ops {
        time += if rng.chance(50) { 100 } else { 1000 };
        let r = rng.below(100);
        if r < 22 {
            if let Some(n) = st.run("undo") {
                st = n;
            }
        } else if r < 32 {
            if let Some(n) = st.run("redo") {
                st = n;
            }
        } else if r < 55 {
            st = peer(&mut rng, &st);
        } else if let Some((n, _)) = edit(&mut rng, &st, time, false) {
            st = n;
        }
        assert!(
            st.selection.to().0 <= st.doc.content_size(),
            "selection out of the doc"
        );
    }
    // Undo everything, then redo everything, with peers in between sometimes.
    for _ in 0..200 {
        match st.run("undo") {
            Some(n) => st = n,
            None => break,
        }
        assert!(
            st.selection.to().0 <= st.doc.content_size(),
            "selection out of the doc"
        );
        if rng.chance(20) {
            st = peer(&mut rng, &st);
        }
    }
    for _ in 0..200 {
        match st.run("redo") {
            Some(n) => st = n,
            None => break,
        }
        assert!(
            st.selection.to().0 <= st.doc.content_size(),
            "selection out of the doc"
        );
    }
}

#[test]
fn undo_and_redo_among_peers_never_panic() {
    let start: u64 = std::env::var("RINCH_HIST_FUZZ_START")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let n: u64 = std::env::var("RINCH_HIST_FUZZ_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(300);
    let ops: usize = std::env::var("RINCH_HIST_FUZZ_OPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    if std::env::var("RINCH_HIST_FUZZ_LOUD").is_err() {
        std::panic::set_hook(Box::new(|_| {}));
    }
    let mut fails: Vec<(u64, String)> = Vec::new();
    // The seeds the review found, then a range.
    for seed in [1159, 40693, 48017].into_iter().chain(start..start + n) {
        let r = catch_unwind(AssertUnwindSafe(|| trial(seed, ops)));
        if let Err(e) = r {
            let msg = e
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            fails.push((seed, msg));
        }
    }
    let _ = std::panic::take_hook();
    eprintln!(
        "seeds {start}..{} ops {ops}: {} panics",
        start + n,
        fails.len()
    );
    for (s, m) in fails.iter().take(400) {
        eprintln!("  seed {s}: {m}");
    }
    assert!(fails.is_empty(), "{} panics", fails.len());
}
