//! Property tests of the history (#1249; from the review of PR #1889).
//!
//! `local_only_*`: random local edits, undos, redos, caret moves (closeHistory) and
//! time gaps, checked step by step against a snapshot model of the history (each
//! undo restores exactly the doc and selection before its event, each redo exactly
//! the doc and selection before its undo), plus undo-all / redo-all at the end.
//!
//! `remote_*`: the same with a peer's changes interleaved, applied the way
//! rinch-editor-collab applies them (one replace of the changed top-level middle,
//! `add_to_history(false)`). Peers type UPPERCASE, I type lowercase; invariants:
//! an undo/redo never changes the peer's character sequence, and undoing everything
//! leaves no lowercase char.
//!
//! `commands_and_paste_match_main`: the outputs of the commands that build slices
//! (`Slice::insert_at`'s callers) and of a paste over every place, pinned by hash
//! to what main produced before #1249 changed `insert_at` and the mark steps.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_editor_core::serialize::{node_to_html, slice_from_html};
use rinch_editor_core::{
    BaseCommandsPlugin, EditorState, Fragment, HistoryPlugin, Node, Plugin, Pos, ReplaceStep,
    Schema, Selection, Slice, Transaction,
};

static UNDOS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static REDOS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static MERGES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static EVENTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
use std::sync::atomic::Ordering::Relaxed;

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

const DEPTH: usize = 12;

fn slice_text(doc: &Node, a: usize, b: usize) -> String {
    let s = doc.slice(a, b).unwrap();
    let mut out = String::new();
    for n in s.content.iter() {
        out.push_str(&all_text(n));
    }
    out
}

fn plugins_d(depth: usize) -> Vec<Rc<dyn Plugin>> {
    vec![
        Rc::new(BaseCommandsPlugin),
        Rc::new(HistoryPlugin::with(500, depth)),
    ]
}

fn state(html: &str) -> EditorState {
    state_d(html, DEPTH)
}

thread_local! {
    static SCHEMA: Rc<Schema> = Rc::new(Schema::starter_kit());
}
fn schema() -> Rc<Schema> {
    SCHEMA.with(|s| s.clone())
}

fn state_d(html: &str, depth: usize) -> EditorState {
    let schema = schema();
    let slice = slice_from_html(&schema, html).unwrap();
    let doc = schema.branch("doc", slice.content.clone()).unwrap();
    let mut st = EditorState::create(schema, doc, plugins_d(depth));
    st.selection = Selection::cursor(Pos(1));
    st
}

fn text_positions(doc: &Node) -> Vec<usize> {
    (0..=doc.content_size())
        .filter(|&p| doc.resolve(Pos(p)).is_ok_and(|r| r.parent().is_textblock()))
        .collect()
}

fn all_text(n: &Node) -> String {
    if let Some(t) = n.text() {
        return t.to_string();
    }
    let mut s = String::new();
    for i in 0..n.child_count() {
        s.push_str(&all_text(n.child(i)));
        if n.child(i).is_textblock() {
            s.push('|');
        }
    }
    s
}

fn is_typing(tr: &Transaction) -> bool {
    !tr.steps().is_empty()
        && tr.steps().iter().all(|s| {
            s.as_any()
                .downcast_ref::<ReplaceStep>()
                .is_some_and(|r| !r.structure)
        })
}

#[derive(Clone)]
struct Ev {
    doc: Node,
    sel: Selection,
    typing: bool,
    time: u64,
}

#[derive(Default)]
struct Model {
    done: Vec<Ev>,
    undone: Vec<Ev>,
    brk: bool,
}

impl Model {
    fn record(&mut self, before: &EditorState, tr: &Transaction) {
        if !tr.doc_changed() {
            if tr.selection_set() || tr.stored_marks_set() {
                self.brk = true;
            }
            return;
        }
        let typing = is_typing(tr);
        let t = tr.time();
        let merge = !self.brk
            && typing
            && self
                .done
                .last()
                .is_some_and(|l| l.typing && t >= l.time && t - l.time < 500);
        if merge {
            MERGES.fetch_add(1, Relaxed);
            self.done.last_mut().unwrap().time = t;
        } else {
            EVENTS.fetch_add(1, Relaxed);
            self.done.push(Ev {
                doc: before.doc.clone(),
                sel: before.selection.clone(),
                typing,
                time: t,
            });
        }
        self.brk = false;
        self.undone.clear();
        while self.done.len() > DEPTH {
            self.done.remove(0);
        }
    }
}

/// Run a command with `time`, returning the state and whether it applied.
fn run_cmd(st: &EditorState, name: &str, time: u64) -> Option<(EditorState, Transaction)> {
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
    // Keep a meta-less copy for the model; apply the original (it carries meta).
    let copy = clone_tr(&tr, st);
    let next = st.apply(tr);
    Some((next, copy))
}

/// Transactions are not Clone; rebuild an equivalent one by replaying its steps.
fn clone_tr(tr: &Transaction, st: &EditorState) -> Transaction {
    let mut t = st.tr();
    for s in tr.steps() {
        t.step(s.clone()).expect("replay");
    }
    if tr.selection_set() {
        t.set_selection(tr.selection());
    }
    if tr.stored_marks_set() {
        t.set_stored_marks(tr.stored_marks().cloned());
    }
    t.set_time(tr.time());
    t
}

const CMDS: &[&str] = &[
    "splitBlock",
    "toggleBold",
    "toggleItalic",
    "setCodeBlock",
    "setParagraph",
    "wrapInBlockquote",
    "toggleBulletList",
    "toggleOrderedList",
    "sinkListItem",
    "liftListItem",
    "insertHardBreak",
    "deleteCharBackward",
    "deleteCharForward",
    "deleteWordBackward",
    "deleteSelection",
];

fn random_sel(rng: &mut Rng, doc: &Node) -> Selection {
    let tp = text_positions(doc);
    let a = tp[rng.below(tp.len())];
    if rng.chance(40) {
        let b = tp[rng.below(tp.len())];
        Selection::text(Pos(a), Pos(b))
    } else {
        Selection::cursor(Pos(a))
    }
}

/// One local edit; returns the new state and the transaction (for the model).
fn local_edit(
    rng: &mut Rng,
    st: &EditorState,
    time: u64,
    lower_only_deletes: bool,
) -> Option<(EditorState, EditorState, Transaction)> {
    match rng.below(10) {
        0..=3 => {
            let tp = text_positions(&st.doc);
            let at = tp[rng.below(tp.len())];
            let s: String = (0..1 + rng.below(3))
                .map(|_| (b'a' + rng.below(4) as u8) as char)
                .collect();
            let mut tr = st.tr();
            tr.set_time(time);
            tr.set_selection(Selection::cursor(Pos(at)));
            tr.insert_text(&s).ok()?;
            let next = st.apply(clone_tr(&tr, st));
            Some((st.clone(), next, tr))
        }
        4 => {
            // delete a range inside one textblock
            let tp = text_positions(&st.doc);
            let a = tp[rng.below(tp.len())];
            let r = st.doc.resolve(Pos(a)).ok()?;
            let end = a - r.parent_offset() + r.parent().content_size();
            if end <= a {
                return None;
            }
            let b = a + 1 + rng.below(end - a);
            let txt = slice_text(&st.doc, a, b);
            if lower_only_deletes && txt.chars().any(|c| !c.is_ascii_lowercase()) {
                return None;
            }
            let mut tr = st.tr();
            tr.set_time(time);
            tr.delete(a, b).ok()?;
            let next = st.apply(clone_tr(&tr, st));
            Some((st.clone(), next, tr))
        }
        _ => {
            let name = CMDS[rng.below(CMDS.len())];
            if lower_only_deletes && name.starts_with("delete") {
                return None;
            }
            let mut s = st.clone();
            s.selection = random_sel(rng, &st.doc);
            // With a peer's text about, a command over a range can replace it
            // (`insertHardBreak`, `splitBlock` over a selection), and undoing that
            // rightly brings it back: keep commands to a caret there.
            if lower_only_deletes {
                s.selection = Selection::cursor(s.selection.head());
            }
            let (n, tr) = run_cmd(&s, name, time)?;
            Some((s, n, tr))
        }
    }
}

const INIT: &str = "<p>ab</p><p>cd</p><ul><li><p>ef</p></li></ul><p>gh</p>";

fn check_local_seed(seed: u64, ops: usize) -> Result<(), String> {
    let mut rng = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
    let mut st = state(INIT);
    let mut model = Model::default();
    let mut time = 1000u64;
    for i in 0..ops {
        time += if rng.chance(60) { 10 } else { 1000 };
        let r = rng.below(100);
        if r < 15 {
            // undo
            let before = st.clone();
            let got = run_cmd(&st, "undo", time);
            match (model.done.pop(), got) {
                (None, None) => {}
                (Some(ev), Some((n, _))) => {
                    if n.doc != ev.doc {
                        return Err(format!(
                            "seed {seed} op {i}: undo doc\n got  {}\n want {}",
                            node_to_html(&n.doc),
                            node_to_html(&ev.doc)
                        ));
                    }
                    if n.selection != ev.sel {
                        return Err(format!(
                            "seed {seed} op {i}: undo sel got {:?} want {:?}",
                            n.selection, ev.sel
                        ));
                    }
                    UNDOS.fetch_add(1, Relaxed);
                    model.undone.push(Ev {
                        doc: before.doc.clone(),
                        sel: before.selection.clone(),
                        typing: ev.typing,
                        time,
                    });
                    model.brk = true;
                    st = n;
                }
                (a, b) => {
                    return Err(format!(
                        "seed {seed} op {i}: undo availability model {} impl {}",
                        a.is_some(),
                        b.is_some()
                    ));
                }
            }
        } else if r < 25 {
            let before = st.clone();
            let got = run_cmd(&st, "redo", time);
            match (model.undone.pop(), got) {
                (None, None) => {}
                (Some(ev), Some((n, _))) => {
                    if n.doc != ev.doc {
                        return Err(format!(
                            "seed {seed} op {i}: redo doc\n got  {}\n want {}",
                            node_to_html(&n.doc),
                            node_to_html(&ev.doc)
                        ));
                    }
                    if n.selection != ev.sel {
                        return Err(format!(
                            "seed {seed} op {i}: redo sel got {:?} want {:?}",
                            n.selection, ev.sel
                        ));
                    }
                    REDOS.fetch_add(1, Relaxed);
                    model.done.push(Ev {
                        doc: before.doc.clone(),
                        sel: before.selection.clone(),
                        typing: ev.typing,
                        time,
                    });
                    while model.done.len() > DEPTH {
                        model.done.remove(0);
                    }
                    model.brk = true;
                    st = n;
                }
                (a, b) => {
                    return Err(format!(
                        "seed {seed} op {i}: redo availability model {} impl {}",
                        a.is_some(),
                        b.is_some()
                    ));
                }
            }
        } else if r < 32 {
            // caret move (closeHistory-like)
            let mut tr = st.tr();
            tr.set_time(time);
            tr.set_selection(random_sel(&mut rng, &st.doc));
            model.record(&st, &tr);
            st = st.apply(tr);
        } else if let Some((pre, n, tr)) = local_edit(&mut rng, &st, time, false) {
            model.record(&pre, &tr);
            st = n;
        }
    }
    // undo all, then redo all
    let final_doc = st.doc.clone();
    let oldest = model.done.first().cloned();
    let mut undos = 0;
    while let Some((n, _)) = run_cmd(&st, "undo", time) {
        st = n;
        undos += 1;
    }
    if undos != model.done.len() {
        return Err(format!(
            "seed {seed}: undo-all ran {undos}, model {}",
            model.done.len()
        ));
    }
    if let Some(ev) = oldest
        && (st.doc != ev.doc || st.selection != ev.sel)
    {
        return Err(format!("seed {seed}: undo-all mismatch"));
    }
    for _ in 0..undos {
        st = run_cmd(&st, "redo", time)
            .ok_or(format!("seed {seed}: redo-all stopped"))?
            .0;
    }
    if st.doc != final_doc {
        return Err(format!(
            "seed {seed}: redo-all\n got  {}\n want {}",
            node_to_html(&st.doc),
            node_to_html(&final_doc)
        ));
    }
    Ok(())
}

fn seeds() -> u64 {
    300
}

#[test]
fn local_only_matches_snapshot_model() {
    let mut fails = Vec::new();
    for seed in 1..=seeds() {
        if let Err(e) = check_local_seed(seed, 120) {
            fails.push(e);
        }
    }
    for f in fails.iter().take(8) {
        eprintln!("{f}\n");
    }
    assert!(fails.is_empty(), "{} failing seeds", fails.len());
    // The positive control: the trials undo, redo, group and break groups.
    for (what, n) in [
        ("undos", &UNDOS),
        ("redos", &REDOS),
        ("events", &EVENTS),
        ("merges", &MERGES),
    ] {
        assert!(n.load(Relaxed) > 200, "{what}: {}", n.load(Relaxed));
    }
}

// ---------------------------------------------------------------- remote

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

fn upper(doc: &Node) -> String {
    all_text(doc)
        .chars()
        .filter(|c| c.is_ascii_uppercase())
        .collect()
}

/// A peer's edits (1-3 of them, delivered as one update) on a plain copy.
fn peer_target(rng: &mut Rng, st: &EditorState, n_edits: usize) -> Node {
    let mut p = EditorState::create(schema(), st.doc.clone(), vec![Rc::new(BaseCommandsPlugin)]);
    for _ in 0..1 + rng.below(n_edits) {
        let tp = text_positions(&p.doc);
        let at = tp[rng.below(tp.len())];
        match rng.below(6) {
            0..=2 => {
                let s: String = (0..1 + rng.below(2))
                    .map(|_| (b'A' + rng.below(3) as u8) as char)
                    .collect();
                let mut tr = p.tr();
                tr.set_selection(Selection::cursor(Pos(at)));
                if tr.insert_text(&s).is_ok() {
                    p = p.apply(tr);
                }
            }
            3 => {
                let r = p.doc.resolve(Pos(at)).unwrap();
                let end = at - r.parent_offset() + r.parent().content_size();
                if end > at {
                    let b = at + 1 + rng.below((end - at).min(3));
                    let mut tr = p.tr();
                    if tr.delete(at, b).is_ok() {
                        p = p.apply(tr);
                    }
                }
            }
            4 => {
                p.selection = Selection::cursor(Pos(at));
                if let Some(n) = p.run("splitBlock") {
                    p = n;
                }
            }
            _ => {
                let b = tp[rng.below(tp.len())];
                p.selection = Selection::text(Pos(at.min(b)), Pos(at.max(b)));
                if let Some(n) = p.run("toggleBold") {
                    p = n;
                }
            }
        }
    }
    p.doc
}

fn check_remote_seed(seed: u64, ops: usize, peer_edits: usize) -> Result<(), String> {
    let mut rng = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
    let mut st = state_d(
        "<p>AB</p><p>CA</p><ul><li><p>BC</p></li></ul><p>AA</p>",
        1000,
    );
    let mut time = 1000u64;
    for i in 0..ops {
        time += if rng.chance(60) { 10 } else { 1000 };
        let r = rng.below(100);
        if r < 30 {
            let target = peer_target(&mut rng, &st, peer_edits);
            st = collab_like(&st, &target);
        } else if r < 45 {
            let cmd = if rng.chance(60) { "undo" } else { "redo" };
            let before = upper(&st.doc);
            let html_before = node_to_html(&st.doc);
            if let Some((n, _)) = run_cmd(&st, cmd, time) {
                if upper(&n.doc) != before {
                    return Err(format!(
                        "seed {seed} op {i}: {cmd} changed the peer's text\n before {html_before}\n after  {}",
                        node_to_html(&n.doc)
                    ));
                }
                st = n;
            }
        } else if let Some((_, n, _)) = local_edit(&mut rng, &st, time, true) {
            st = n;
        }
    }
    let before_all = node_to_html(&st.doc);
    while let Some((n, _)) = run_cmd(&st, "undo", time) {
        st = n;
    }
    let t = all_text(&st.doc);
    if t.chars().any(|c| c.is_ascii_lowercase()) {
        return Err(format!(
            "seed {seed}: lowercase left after undo-all\n before {before_all}\n after  {}",
            node_to_html(&st.doc)
        ));
    }
    Ok(())
}

fn remote_failures(peer_edits: usize) -> Vec<u64> {
    let mut failing = Vec::new();

    for seed in 1..=seeds() {
        if let Err(e) = check_remote_seed(seed, 25, peer_edits) {
            eprintln!("{e}\n");
            failing.push(seed);
        }
    }
    failing
}

/// A peer's update of one edit: my undo never changes their text, and undoing
/// everything leaves none of mine. (265 of 300 seeds fail on main.)
#[test]
fn remote_undo_never_touches_peer_text_and_undoes_all_mine() {
    assert_eq!(remote_failures(1), Vec::<u64>::new());
}

/// Updates of up to three edits each, in peers' text of three letters: where the
/// documents before and after an update do not say which `A` is which, the history
/// can guess wrong (#1895). These seeds are known wrong (287 of 300 fail on main),
/// pinned so a fix flips them.
#[test]
fn remote_multi_edit_updates_known_wrong_seeds() {
    assert_eq!(remote_failures(3), KNOWN_WRONG_MULTI);
}

/// #1895's seeds of [`remote_multi_edit_updates_known_wrong_seeds`].
const KNOWN_WRONG_MULTI: [u64; 7] = [28, 39, 77, 118, 136, 187, 255];

fn st_html(html: &str) -> EditorState {
    state_d(html, 100)
}

fn show(out: &mut String, name: &str, html: &str, sel: Selection, cmd: &str) {
    let mut st = st_html(html);
    st.selection = sel;
    match st.run(cmd) {
        Some(n) => {
            let u = n.run("undo").map(|u| node_to_html(&u.doc));
            out.push_str(&format!(
                "CMD {name}: {cmd} -> {} | undo -> {:?}\n",
                node_to_html(&n.doc),
                u
            ));
        }
        None => out.push_str(&format!("CMD {name}: {cmd} -> NOT APPLIED\n")),
    }
}

/// What the commands that build their content through `Slice::insert_at` (and the
/// mark steps' re-marked copies) produce, and their undo.
fn commands_out() -> String {
    let mut out = String::new();
    let o = &mut out;
    show(
        o,
        "bold->code",
        "<p><strong>a</strong>b</p>",
        Selection::cursor(Pos(2)),
        "setCodeBlock",
    );
    show(
        o,
        "br->code",
        "<p>a<br>b</p>",
        Selection::cursor(Pos(2)),
        "setCodeBlock",
    );
    show(
        o,
        "img->code",
        "<p>a<img src=\"x.png\">b</p>",
        Selection::cursor(Pos(2)),
        "setCodeBlock",
    );
    show(
        o,
        "link->code",
        "<p><a href=\"h\">a</a>b</p>",
        Selection::cursor(Pos(2)),
        "setCodeBlock",
    );
    show(
        o,
        "code->p",
        "<pre>ab</pre>",
        Selection::cursor(Pos(2)),
        "setParagraph",
    );
    show(
        o,
        "wrapquote",
        "<p>ab</p><p>cd</p>",
        Selection::text(Pos(2), Pos(6)),
        "wrapInBlockquote",
    );
    show(
        o,
        "bullet",
        "<p>ab</p><p>cd</p>",
        Selection::text(Pos(2), Pos(6)),
        "toggleBulletList",
    );
    show(
        o,
        "tasklist",
        "<p>ab</p><p>cd</p>",
        Selection::text(Pos(2), Pos(6)),
        "toggleTaskList",
    );
    show(
        o,
        "lift",
        "<ul><li><p>ab</p></li><li><p>cd</p></li></ul>",
        Selection::cursor(Pos(9)),
        "liftListItem",
    );
    show(
        o,
        "sink",
        "<ul><li><p>ab</p></li><li><p>cd</p></li></ul>",
        Selection::cursor(Pos(9)),
        "sinkListItem",
    );
    show(
        o,
        "heading->code multi",
        "<h2>a<strong>b</strong></h2><p>c<br>d</p>",
        Selection::text(Pos(2), Pos(8)),
        "setCodeBlock",
    );
    for (from, to) in [(2, 3), (1, 2)] {
        let st = st_html("<p>a<strong>b</strong>c</p>");
        let mut s = st.clone();
        s.selection = Selection::text(Pos(from), Pos(to));
        let n = s.run("toggleBold").unwrap();
        let u = n.run("undo").unwrap();
        out.push_str(&format!(
            "BOLD {from}..{to}: {} children {} | undo {} children {} eq {}\n",
            node_to_html(&n.doc),
            n.doc.child(0).child_count(),
            node_to_html(&u.doc),
            u.doc.child(0).child_count(),
            u.doc == st.doc
        ));
    }
    out
}

/// A paste of each snippet, with each open depth, over ranges at every place.
fn paste_out() -> String {
    let docs = [
        "<p>ab<strong>cd</strong>ef</p><pre>xyz</pre>",
        "<ul><li><p>a<em>b</em>c</p></li></ul><blockquote><p>q<br>r</p></blockquote>",
        "<h2>he<strong>ad</strong></h2><p>t<img src=\"i.png\">u</p>",
        "<table><tr><td><p>a<strong>b</strong></p></td><td><p>c</p></td></tr></table><p>z</p>",
    ];
    let snippets = [
        "<p>x</p><pre>y</pre>",
        "<pre>c1</pre><p>p2</p>",
        "<p>a</p><h1>h</h1>",
        "<ul><li><p>l1</p></li><li><p>l2</p></li></ul>",
        "<blockquote><p>q</p></blockquote><pre>k</pre>",
        "<p><strong>b</strong></p><pre>z<br>w</pre>",
        "<pre>only</pre>",
        "<p>1</p><p>2</p><p>3</p>",
    ];
    let mut out = String::new();
    let schema = schema();
    for d in docs {
        for sn in snippets {
            let st0 = st_html(d);
            let slice = slice_from_html(&schema, sn).unwrap();
            for open in [(0, 0), (1, 1), (1, 0), (0, 1)] {
                let sl = Slice::new(slice.content.clone(), open.0, open.1);
                let tp = text_positions(&st0.doc);
                for (i, &a) in tp.iter().enumerate() {
                    for &b in tp.iter().skip(i).step_by(3).take(4) {
                        let mut tr = st0.tr();
                        tr.set_selection(Selection::text(Pos(a), Pos(b)));
                        let r = match tr.replace_selection(sl.clone()) {
                            Ok(_) => node_to_html(tr.doc()),
                            Err(e) => format!("ERR {e}"),
                        };
                        out.push_str(&format!("{d} | {sn} | {open:?} | {a}-{b} => {r}\n"));
                    }
                }
            }
        }
    }
    out
}

fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
    })
}

/// The commands and pastes produce, byte for byte, what they produced on main
/// before #1249 changed `Slice::insert_at` (PM's `canReplace` check) and the mark
/// steps (joined text). The hashes are of main's outputs (fb5ebc36), measured with
/// this file; the paste output is 2538 lines.
#[test]
fn commands_and_paste_match_main() {
    assert_eq!(fnv(&commands_out()), COMMANDS_HASH, "{}", commands_out());
    assert_eq!(fnv(&paste_out()), PASTE_HASH);
}

const COMMANDS_HASH: u64 = 9266202348521479181;
const PASTE_HASH: u64 = 15658643660746681621;

fn rss_kb() -> usize {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| {
            s.split_whitespace()
                .nth(1)
                .and_then(|v| v.parse::<usize>().ok())
        })
        .unwrap_or(0)
        * 4
}

/// One local edit, then N peer keystrokes (each a whole-paragraph replace in a
/// paragraph other than mine, as the collab integration applies them).
#[test]
#[ignore]
fn z_perf_remote_keystrokes() {
    let max: usize = std::env::var("R1889_MAX")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(16000);
    let paras: String = (0..50)
        .map(|i| format!("<p>paragraph number {i}</p>"))
        .collect();
    let mut st = st_html(&paras);
    st = local(&st, 1, |tr| {
        tr.set_selection(Selection::cursor(Pos(3)));
        tr.insert_text("X").unwrap();
    });
    let schema = schema();
    let base = rss_kb();
    let mut n = 0usize;
    let mut next_report = 1000usize;
    let t0 = std::time::Instant::now();
    let mut last = t0;
    while n < max {
        // peer types one char at the end of paragraph 25
        let idx = 25;
        let start: usize = (0..idx).map(|i| st.doc.child(i).node_size()).sum();
        let old = st.doc.child(idx).clone();
        let text = format!("{}{}", chars_of(&old), (b'A' + (n % 26) as u8) as char);
        let newp = schema
            .branch(
                "paragraph",
                Fragment::from_node(schema.text(&text).unwrap()),
            )
            .unwrap();
        let mut tr = st.tr();
        tr.replace(
            start,
            start + old.node_size(),
            Slice::new(Fragment::from_node(newp), 0, 0),
        )
        .unwrap();
        tr.set_add_to_history(false);
        st = st.apply(tr);
        // keep the paragraph short so doc cost stays constant
        if text.len() > 60 {
            let start: usize = (0..idx).map(|i| st.doc.child(i).node_size()).sum();
            let old = st.doc.child(idx).clone();
            let newp = schema
                .branch("paragraph", Fragment::from_node(schema.text("p").unwrap()))
                .unwrap();
            let mut tr = st.tr();
            tr.replace(
                start,
                start + old.node_size(),
                Slice::new(Fragment::from_node(newp), 0, 0),
            )
            .unwrap();
            tr.set_add_to_history(false);
            st = st.apply(tr);
            n += 1;
        }
        n += 1;
        if n >= next_report {
            let now = std::time::Instant::now();
            let per = (now - last).as_secs_f64() * 1e6 / (next_report as f64 / 2.0).max(1.0);
            eprintln!(
                "PERF n={n} last-batch us/tx={per:.1} rss_delta_kb={}",
                rss_kb().saturating_sub(base)
            );
            last = now;
            next_report *= 2;
        }
    }
    let u0 = std::time::Instant::now();
    let undone = st.run("undo").unwrap();
    eprintln!(
        "PERF undo after {n}: {:.1} ms, X gone {}",
        u0.elapsed().as_secs_f64() * 1e3,
        !chars_of(undone.doc.child(0)).contains('X')
    );
    // a local keystroke now
    let k0 = std::time::Instant::now();
    let _ = local(&st, 100_000, |tr| {
        tr.set_selection(Selection::cursor(Pos(4)));
        tr.insert_text("Y").unwrap();
    });
    eprintln!(
        "PERF local keystroke after {n}: {:.1} us",
        k0.elapsed().as_secs_f64() * 1e6
    );
}

fn chars_of(n: &Node) -> String {
    if let Some(t) = n.text() {
        return t.to_string();
    }
    (0..n.child_count()).map(|i| chars_of(n.child(i))).collect()
}

fn local(st: &EditorState, time: u64, build: impl FnOnce(&mut Transaction)) -> EditorState {
    let mut tr = st.tr();
    tr.set_time(time);
    build(&mut tr);
    st.apply(tr)
}
