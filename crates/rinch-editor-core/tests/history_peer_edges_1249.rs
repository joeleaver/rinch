//! Undo at the edges of a peer's changes (#1249; from the second review of PR #1889):
//! a peer typing or deleting right beside my edits, inside my typed run, or around a
//! char I deleted, one change at a time (so the history folds them); markup-only peer
//! changes against the same session without them; and `StepMap::compose_exact`
//! against mapping through its two maps in turn.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_editor_core::serialize::{node_to_html, slice_from_html};
use rinch_editor_core::{
    BaseCommandsPlugin, EditorState, Fragment, HistoryPlugin, Node, Plugin, Pos, Schema, Selection,
    Slice, Transaction,
};

thread_local! {
    static SCHEMA: Rc<Schema> = Rc::new(Schema::starter_kit());
}
fn schema() -> Rc<Schema> {
    SCHEMA.with(|s| s.clone())
}

fn plugins() -> Vec<Rc<dyn Plugin>> {
    vec![
        Rc::new(BaseCommandsPlugin),
        Rc::new(HistoryPlugin::with(500, 1000)),
    ]
}

fn state(html: &str) -> EditorState {
    let s = schema();
    let slice = slice_from_html(&s, html).unwrap();
    let doc = s.branch("doc", slice.content.clone()).unwrap();
    let mut st = EditorState::create(s, doc, plugins());
    st.selection = Selection::cursor(Pos(1));
    st
}

fn html(st: &EditorState) -> String {
    node_to_html(&st.doc)
}

fn local(st: &EditorState, time: u64, build: impl FnOnce(&mut Transaction)) -> EditorState {
    let mut tr = st.tr();
    tr.set_time(time);
    build(&mut tr);
    st.apply(tr)
}

fn type_at(st: &EditorState, at: usize, s: &str, time: u64) -> EditorState {
    local(st, time, |tr| {
        tr.set_selection(Selection::cursor(Pos(at)));
        tr.insert_text(s).unwrap();
    })
}

fn remote(st: &EditorState, build: impl FnOnce(&mut Transaction)) -> EditorState {
    let mut tr = st.tr();
    build(&mut tr);
    tr.set_add_to_history(false);
    st.apply(tr)
}

fn ins_remote(st: &EditorState, at: usize, s: &str) -> EditorState {
    let f = Fragment::from_node(st.schema().text(s).unwrap());
    remote(st, |tr| {
        tr.replace_with(at, at, f).unwrap();
    })
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

// ------------------------------------------------------------- split delete

/// My typing group typed `abc`, then deleted `b` (one group). A peer types `P`
/// between my `a` and `c`. Undo should leave exactly the peer's `P`.
#[test]
fn t1_split_delete_with_mirror_in_group() {
    let st = state("<p></p><p>z</p>");
    let st = type_at(&st, 1, "abc", 0);
    let st = local(&st, 10, |tr| {
        tr.delete(2, 3).unwrap();
    });
    assert_eq!(html(&st), "<p>ac</p><p>z</p>");
    // control: without the peer the group undoes completely
    let ctl = st.run("undo").unwrap();
    assert_eq!(html(&ctl), "<p></p><p>z</p>");
    let st = ins_remote(&st, 2, "P");
    assert_eq!(html(&st), "<p>aPc</p><p>z</p>");
    let u = st.run("undo").unwrap();
    let r = u.run("redo").expect("redo applies");
    assert_eq!(html(&r), "<p>aPc</p><p>z</p>");
    assert_eq!(html(&u), "<p>P</p><p>z</p>");
}

/// The same with the peer's change delivered as a whole-paragraph replace.
#[test]
fn t1b_split_delete_with_mirror_in_group_collab_like() {
    let st = state("<p></p><p>z</p>");
    let st = type_at(&st, 1, "abc", 0);
    let st = local(&st, 10, |tr| {
        tr.delete(2, 3).unwrap();
    });
    let target = ins_remote(&st, 2, "P").doc;
    let st = collab_like(&st, &target);
    assert_eq!(html(&st), "<p>aPc</p><p>z</p>");
    let u = st.run("undo").unwrap();
    assert_eq!(html(&u), "<p>P</p><p>z</p>");
}

/// Several peer insertions inside my typed run, and a peer deletion of one of my
/// chars, then undo and redo.
#[test]
fn t3_split_delete_many_peer_inserts_and_a_delete() {
    let st = state("<p>z</p>");
    let st = type_at(&st, 1, "abcdef", 0); // abcdefz
    // peer: P between b|c, Q between d|e, R before a, S after f, deletes e
    let st = ins_remote(&st, 3, "P"); // abPcdefz
    let st = ins_remote(&st, 6, "Q"); // abPcdQefz
    let st = ins_remote(&st, 1, "R"); // RabPcdQefz
    let st = ins_remote(&st, 10, "S"); // RabPcdQefSz
    assert_eq!(html(&st), "<p>RabPcdQefSz</p>");
    let st = remote(&st, |tr| {
        tr.delete(8, 9).unwrap(); // e
    });
    assert_eq!(html(&st), "<p>RabPcdQfSz</p>");
    let u = st.run("undo").unwrap();
    let r = u.run("redo").unwrap();
    assert_eq!(html(&u), "<p>RPQSz</p>");
    assert_eq!(html(&r), "<p>RabPcdQfSz</p>");
    // and once more: undo the redo
    let u2 = r.run("undo").unwrap();
    assert_eq!(html(&u2), "<p>RPQSz</p>");
}

/// Peer inserts inside, then a peer change after the undo, then redo.
#[test]
fn t3b_split_then_peer_then_redo() {
    let st = state("<p>z</p>");
    let st = type_at(&st, 1, "abcd", 0); // abcdz
    let st = ins_remote(&st, 3, "P"); // abPcdz
    let u = st.run("undo").unwrap();
    assert_eq!(html(&u), "<p>Pz</p>");
    let u = ins_remote(&u, 2, "Q"); // PQz
    let r = u.run("redo").unwrap();
    // ab before P, cd after P; Q after P: PM would give abPcdQz or abPQcdz
    assert!(html(&r).contains('P') && html(&r).contains('Q'));
    let txt: String = html(&r)
        .chars()
        .filter(|c| c.is_ascii_lowercase() && *c != 'p')
        .collect();
    assert_eq!(html(&r), "<p>abPQcdz</p>", "{txt}");
}

/// A peer insertion inside my typed run across a group of several keystrokes, the
/// inner one being a delete of a char typed earlier in the group.
#[test]
fn t3c_group_with_backspace_and_peer_insert() {
    let st = state("<p>z</p>");
    let st = type_at(&st, 1, "abcd", 0); // abcdz
    let st = local(&st, 10, |tr| {
        tr.delete(4, 5).unwrap(); // d -> abcz
    });
    let st = type_at(&st, 4, "e", 20); // abcez
    let st = ins_remote(&st, 2, "P"); // aPbcez
    let st = ins_remote(&st, 5, "Q"); // aPbcQez
    assert_eq!(html(&st), "<p>aPbcQez</p>");
    let u = st.run("undo").unwrap();
    let r = u.run("redo").unwrap();
    assert_eq!(html(&u), "<p>PQz</p>");
    assert_eq!(html(&r), html(&st));
}

// ------------------------------------------------------------- fold

/// I delete `m`; a peer then deletes the char after it and the char before it, as
/// two separate changes. ProseMirror maps through the two maps in turn and my undo
/// re-inserts `m` between `b` and `e`. Folded into one map, the position is inside
/// one deleted range on both sides.
#[test]
fn t2_fold_two_peer_deletes_around_my_deleted_char() {
    let st = state("<p>abcmde</p>");
    let st = local(&st, 0, |tr| {
        tr.delete(4, 5).unwrap();
    });
    assert_eq!(html(&st), "<p>abcde</p>");
    let st = remote(&st, |tr| {
        tr.delete(4, 5).unwrap(); // d
    });
    let st = remote(&st, |tr| {
        tr.delete(3, 4).unwrap(); // c
    });
    assert_eq!(html(&st), "<p>abe</p>");
    let u = st.run("undo").unwrap();
    assert_eq!(html(&u), "<p>abme</p>");
}

/// The same with a peer's typed char between: peer deletes `d`, types `X` where my
/// `m` was, deletes `c`.
#[test]
fn t2b_fold_with_insert_between() {
    let st = state("<p>abcmde</p>");
    let st = local(&st, 0, |tr| {
        tr.delete(4, 5).unwrap();
    });
    let st = remote(&st, |tr| {
        tr.delete(4, 5).unwrap(); // d -> abce
    });
    let st = ins_remote(&st, 4, "X"); // abcXe
    let st = remote(&st, |tr| {
        tr.delete(3, 4).unwrap(); // c -> abXe
    });
    assert_eq!(html(&st), "<p>abXe</p>");
    let u = st.run("undo").unwrap();
    assert!(html(&u).contains('m'), "{}", html(&u));
}

/// Separated by a local recorded step (not folded) as a control: the same peer
/// deletes, but my own unrelated edit in another paragraph between them.
#[test]
fn t2c_control_not_folded() {
    let st = state("<p>abcmde</p><p>zz</p>");
    let st = local(&st, 0, |tr| {
        tr.delete(4, 5).unwrap();
    });
    let st = remote(&st, |tr| {
        tr.delete(4, 5).unwrap();
    });
    // my edit in paragraph 2 (its own event, time far apart)
    let st = type_at(&st, 8, "y", 5000);
    let st = remote(&st, |tr| {
        tr.delete(3, 4).unwrap();
    });
    assert_eq!(html(&st), "<p>abe</p><p>zyz</p>");
    let u = st.run("undo").unwrap().run("undo").unwrap();
    assert_eq!(html(&u), "<p>abme</p><p>zz</p>");
}

// ------------------------------------------------------------- range command (#2)

/// My command over a range that includes a peer's text; then undo.
#[test]
fn t4_range_command_over_peer_text() {
    for cmd in [
        "insertHardBreak",
        "splitBlock",
        "deleteSelection",
        "deleteCharBackward",
    ] {
        let st = state("<p>abcd</p><p>z</p>");
        let st = type_at(&st, 3, "xy", 0); // abxycd
        let st = ins_remote(&st, 4, "PQ"); // abxPQycd
        assert_eq!(html(&st), "<p>abxPQycd</p><p>z</p>");
        let mut s = st.clone();
        s.selection = Selection::text(Pos(2), Pos(7)); // bxPQy
        let mut tr = s.tr();
        tr.set_time(5000);
        let applied: RefCell<Option<Transaction>> = RefCell::new(None);
        let c = s.command(cmd).unwrap();
        let ok = c(
            &s,
            Some(&mut |mut t: Transaction| {
                t.set_time(5000);
                *applied.borrow_mut() = Some(t);
            }),
        );
        drop(tr);
        assert!(ok, "{cmd}");
        let n = s.apply(applied.into_inner().unwrap());
        // a peer types after my command, somewhere else
        let n = ins_remote(&n, 1, "R");
        let u1 = n.run("undo").unwrap();
        let u2 = u1.run("undo").unwrap();
        let r1 = u2.run("redo").unwrap();
        let r2 = r1.run("redo").unwrap();
        assert_eq!(html(&r2), html(&n), "{cmd}: undo twice, redo twice");
        assert!(
            html(&u1).contains("PQ") && html(&u1).contains('R'),
            "{cmd}: {}",
            html(&u1)
        );
    }
}

// ------------------------------------------------------------- markup differential (#1)

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
}

fn text_positions(doc: &Node) -> Vec<usize> {
    (0..=doc.content_size())
        .filter(|&p| doc.resolve(Pos(p)).is_ok_and(|r| r.parent().is_textblock()))
        .collect()
}

/// Chars and structure, ignoring marks, block types and attributes.
fn skeleton(n: &Node, out: &mut String) {
    if let Some(t) = n.text() {
        out.push_str(t);
    } else if n.is_leaf() {
        out.push('[');
        out.push_str(n.type_name());
        out.push(']');
    } else {
        out.push('<');
        for i in 0..n.child_count() {
            skeleton(n.child(i), out);
        }
        out.push('>');
    }
}
fn skel(n: &Node) -> String {
    let mut s = String::new();
    skeleton(n, &mut s);
    s
}

const MY_CMDS: &[&str] = &[
    "splitBlock",
    "toggleBold",
    "toggleItalic",
    "wrapInBlockquote",
    "toggleBulletList",
    "sinkListItem",
    "liftListItem",
    "insertHardBreak",
    "deleteCharBackward",
    "deleteCharForward",
    "deleteWordBackward",
    "deleteSelection",
    "insertHorizontalRule",
];

const PEER_MARKUP: &[&str] = &[
    "toggleBold",
    "toggleItalic",
    "toggleUnderline",
    "toggleStrike",
    "setHeading1",
    "setHeading2",
    "setParagraph",
    "setTextAlignCenter",
    "setTextAlignRight",
    "setTextAlignLeft",
];

fn run_cmd(st: &EditorState, sel: Selection, name: &str, time: u64) -> Option<EditorState> {
    let mut s = st.clone();
    s.selection = sel;
    let c = s.command(name)?;
    let got: RefCell<Option<Transaction>> = RefCell::new(None);
    let ok = c(
        &s,
        Some(&mut |mut t: Transaction| {
            t.set_time(time);
            *got.borrow_mut() = Some(t);
        }),
    );
    if !ok {
        return None;
    }
    Some(s.apply(got.into_inner()?))
}

fn hist(st: &EditorState, cmd: &str, time: u64) -> Option<EditorState> {
    let sel = st.selection.clone();
    run_cmd(st, sel, cmd, time)
}

/// Returns Err(description) on the first divergence caused by an undo/redo,
/// Ok(true) if a local op diverged first (seed inconclusive), Ok(false) if clean.
fn markup_seed(seed: u64, raw_steps: bool) -> Result<bool, String> {
    let peer_ops: Vec<&str> = PEER_MARKUP.to_vec();
    let mut rng = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
    let init = "<p>abc def</p><h2>ghi</h2><ul><li><p>jk</p></li><li><p>lmn</p></li></ul><blockquote><p>op</p></blockquote><p>qrs</p>";
    let mut a = state(init); // with the peer
    let mut b = state(init); // without
    let mut time = 1000u64;
    let mut log: Vec<String> = Vec::new();
    for i in 0..60 {
        time += if rng.below(10) < 6 { 10 } else { 1000 };
        let r = rng.below(100);
        let tp = text_positions(&b.doc);
        let p1 = tp[rng.below(tp.len())];
        let p2 = tp[rng.below(tp.len())];
        if r < 35 {
            // peer markup change on `a` only
            let name = peer_ops[rng.below(peer_ops.len())];
            let sel = if name.starts_with("toggleB")
                || name.starts_with("toggleI")
                || name.starts_with("toggleU")
                || name.starts_with("toggleS")
                || name == "wrapInBlockquote"
                || name == "lift"
            {
                Selection::text(Pos(p1.min(p2)), Pos(p1.max(p2)))
            } else {
                Selection::cursor(Pos(p1))
            };
            let mut peer =
                EditorState::create(schema(), a.doc.clone(), vec![Rc::new(BaseCommandsPlugin)]);
            peer.selection = sel;
            let Some(c) = peer.command(name) else {
                continue;
            };
            let got: RefCell<Option<Transaction>> = RefCell::new(None);
            if !c(
                &peer,
                Some(&mut |t: Transaction| *got.borrow_mut() = Some(t)),
            ) {
                continue;
            }
            let ptr = got.into_inner().unwrap();
            if skel(ptr.doc()) != skel(&a.doc) {
                // not pure markup (e.g. heading strips a hard break) — skip it
                log.push(format!("{i}: peer {name} not pure, skipped"));
                continue;
            }
            log.push(format!("{i}: peer {name} {:?}", ptr.selection()));
            if raw_steps {
                let mut tr = a.tr();
                for s in ptr.steps() {
                    tr.step(s.clone()).unwrap();
                }
                tr.set_add_to_history(false);
                a = a.apply(tr);
            } else {
                let target = ptr.doc().clone();
                a = collab_like(&a, &target);
            }
            continue;
        }
        let (na, nb, what) = if r < 55 {
            let cmd = if rng.below(10) < 6 { "undo" } else { "redo" };
            (hist(&a, cmd, time), hist(&b, cmd, time), cmd.to_string())
        } else if r < 75 {
            let s: String = (0..1 + rng.below(3))
                .map(|_| (b'a' + rng.below(26) as u8) as char)
                .collect();
            let f = |st: &EditorState| {
                let mut tr = st.tr();
                tr.set_time(time);
                tr.set_selection(Selection::cursor(Pos(p1)));
                tr.insert_text(&s).ok()?;
                Some(st.apply(tr))
            };
            (f(&a), f(&b), format!("type {p1} {s}"))
        } else if r < 82 {
            // caret move
            let f = |st: &EditorState| {
                let mut tr = st.tr();
                tr.set_time(time);
                tr.set_selection(Selection::cursor(Pos(p1)));
                Some(st.apply(tr))
            };
            (f(&a), f(&b), format!("caret {p1}"))
        } else {
            let name = MY_CMDS[rng.below(MY_CMDS.len())];
            let sel = if rng.below(10) < 4 {
                Selection::text(Pos(p1.min(p2)), Pos(p1.max(p2)))
            } else {
                Selection::cursor(Pos(p1))
            };
            (
                run_cmd(&a, sel.clone(), name, time),
                run_cmd(&b, sel.clone(), name, time),
                format!("cmd {name} {sel:?}"),
            )
        };
        let is_hist = what == "undo" || what == "redo";
        match (na, nb) {
            (None, None) => {}
            (Some(x), Some(y)) => {
                a = x;
                b = y;
            }
            (x, y) => {
                let msg = format!(
                    "seed {seed} op {i} {what}: applies with peer {} without {}\n log {}",
                    x.is_some(),
                    y.is_some(),
                    log.join("; ")
                );
                if is_hist {
                    return Err(msg);
                }
                return Ok(true);
            }
        }
        log.push(format!("{i}: {what}"));
        if skel(&a.doc) != skel(&b.doc) {
            let msg = format!(
                "seed {seed} op {i} {what}:\n with    {} {:?}\n without {} {:?}\n log {}",
                node_to_html(&a.doc),
                a.selection,
                node_to_html(&b.doc),
                b.selection,
                log.join("; ")
            );
            if is_hist {
                return Err(msg);
            }
            eprintln!("LOCAL-DIVERGE {msg}");
            return Ok(true);
        }
    }
    Ok(false)
}

fn markup_campaign(raw: bool) {
    let n: u64 = 300;
    let mut fails = Vec::new();
    let mut inconclusive = 0;
    for seed in 1..=n {
        match markup_seed(seed, raw) {
            Ok(false) => {}
            Ok(true) => inconclusive += 1,
            Err(e) => fails.push(e),
        }
    }
    for f in fails.iter().take(6) {
        eprintln!("{f}\n");
    }
    eprintln!(
        "MARKUP raw={raw}: {} fail, {inconclusive} inconclusive of {n}",
        fails.len()
    );
    assert!(fails.is_empty());
}

#[test]
fn t5_markup_only_peer_changes_collab_like() {
    markup_campaign(false);
}

#[test]
fn t5_markup_only_peer_changes_raw_steps() {
    markup_campaign(true);
}

/// Minimal: I type `e`; a peer types `Q` right before it, then deletes my `e`
/// (two updates). My undo has nothing left to delete.
#[test]
fn t6_fold_peer_insert_then_delete_my_char() {
    let st = state("<p>xy</p>");
    let st = type_at(&st, 2, "e", 0); // xey
    let st = ins_remote(&st, 2, "Q"); // xQey
    let st = remote(&st, |tr| {
        tr.delete(3, 4).unwrap(); // e
    });
    assert_eq!(html(&st), "<p>xQy</p>");
    let u = st.run("undo").unwrap();
    assert_eq!(html(&u), "<p>xQy</p>");
}

/// Same, peer types after my char instead.
#[test]
fn t6b_fold_peer_insert_after_then_delete_my_char() {
    let st = state("<p>xy</p>");
    let st = type_at(&st, 2, "e", 0); // xey
    let st = ins_remote(&st, 3, "Q"); // xeQy
    let st = remote(&st, |tr| {
        tr.delete(2, 3).unwrap(); // e
    });
    assert_eq!(html(&st), "<p>xQy</p>");
    let u = st.run("undo").unwrap();
    assert_eq!(html(&u), "<p>xQy</p>");
}

fn peer_cmd_raw(st: &EditorState, sel: Selection, name: &str) -> EditorState {
    let mut peer = EditorState::create(schema(), st.doc.clone(), vec![Rc::new(BaseCommandsPlugin)]);
    peer.selection = sel;
    let c = peer.command(name).unwrap();
    let got: RefCell<Option<Transaction>> = RefCell::new(None);
    assert!(c(
        &peer,
        Some(&mut |t: Transaction| *got.borrow_mut() = Some(t))
    ));
    let ptr = got.into_inner().unwrap();
    let mut tr = st.tr();
    for s in ptr.steps() {
        tr.step(s.clone()).unwrap();
    }
    tr.set_add_to_history(false);
    st.apply(tr)
}

#[test]
fn t7_raw_set_block_type_then_undo_split() {
    for (which, name) in [
        (0usize, "setHeading1"),
        (1, "setHeading1"),
        (0, "setTextAlignCenter"),
        (1, "setTextAlignCenter"),
    ] {
        let mut st = state("<p>abcd</p><p>z</p>");
        st.selection = Selection::cursor(Pos(3));
        let st = hist(&st, "splitBlock", 0).unwrap(); // <p>ab</p><p>cd</p>
        let at = if which == 0 { 2 } else { 6 };
        let st = peer_cmd_raw(&st, Selection::cursor(Pos(at)), name);
        let u = st.run("undo").unwrap();
        assert_eq!(
            chars_only(&u.doc),
            "abcdz",
            "T7 {name} on block {which}: {}",
            html(&u)
        );
        assert_eq!(
            u.doc.child_count(),
            2,
            "T7 {name} on block {which}: the split is undone"
        );
    }
}

fn chars_only(n: &Node) -> String {
    if let Some(t) = n.text() {
        return t.to_string();
    }
    if n.is_leaf() {
        return format!("[{}]", n.type_name());
    }
    (0..n.child_count())
        .map(|i| chars_only(n.child(i)))
        .collect()
}

/// `compose_exact` against mapping through the two maps in turn, for single-range
/// maps: where it folds, every position maps the same, with the same flags. (Folding
/// every touching pair, as #1889's first fold did, differed at 1114 positions and in
/// 330 `deleted_across` flags over 20k pairs.)
#[test]
fn t8_compose_exact_matches_sequential() {
    use rinch_editor_core::transform::step_map::StepMap;
    let mut rng = Rng(12345);
    let (mut folded, mut refused) = (0, 0);
    for _ in 0..20000 {
        let len = 12usize;
        let s1 = rng.below(len);
        let o1 = rng.below(4).min(len - s1);
        let n1 = rng.below(4);
        let m1 = StepMap::new(vec![s1, o1, n1]);
        let mid = len - o1 + n1;
        let s2 = rng.below(mid);
        let o2 = rng.below(4).min(mid - s2);
        let n2 = rng.below(4);
        let m2 = StepMap::new(vec![s2, o2, n2]);
        let Some(c) = m1.compose_exact(&m2) else {
            refused += 1;
            continue;
        };
        folded += 1;
        for p in 0..=len {
            for assoc in [-1, 1] {
                let (r1, r2) = (
                    m1.map_result(p, assoc),
                    m2.map_result(m1.map(p, assoc), assoc),
                );
                let rc = c.map_result(p, assoc);
                assert_eq!(
                    rc.pos, r2.pos,
                    "m1={m1:?} m2={m2:?} c={c:?} p={p} assoc={assoc}"
                );
                assert_eq!(
                    rc.deleted_across(),
                    r1.deleted_across() || r2.deleted_across(),
                    "across: m1={m1:?} m2={m2:?} c={c:?} p={p} assoc={assoc}"
                );
                assert_eq!(
                    rc.deleted_after(),
                    r1.deleted_after() || r2.deleted_after(),
                    "after: m1={m1:?} m2={m2:?} c={c:?} p={p} assoc={assoc}"
                );
                assert_eq!(
                    rc.deleted(),
                    r1.deleted() || r2.deleted(),
                    "deleted: m1={m1:?} m2={m2:?} c={c:?} p={p} assoc={assoc}"
                );
            }
        }
    }
    // Both halves happen: separate ranges fold, touching insertions fold, the rest
    // is refused.
    assert!(
        folded > 5000 && refused > 1000,
        "folded {folded}, refused {refused}"
    );
}

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

/// A long session: peer keystrokes in paragraph 25 (whole-paragraph replaces), and
/// every 40 of them a local keystroke of mine as a new undo group; every 7th group I
/// undo, every 11th I redo. Default history (depth 200).
#[test]
#[ignore]
fn z_perf_mixed_session() {
    let max: usize = std::env::var("R1889B_MAX")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(64000);
    // A timing probe, run by hand (`--ignored --nocapture`, release); the cost pins
    // are in `history_remote_1249.rs`.
    let s = schema();
    let slice = slice_from_html(
        &s,
        &(0..50)
            .map(|i| format!("<p>paragraph number {i}</p>"))
            .collect::<String>(),
    )
    .unwrap();
    let doc = s.branch("doc", slice.content.clone()).unwrap();
    let mut st = EditorState::create(
        s.clone(),
        doc,
        vec![Rc::new(BaseCommandsPlugin), Rc::new(HistoryPlugin::new())],
    );
    let base = rss_kb();
    let mut next = 1000;
    let t0 = std::time::Instant::now();
    let mut last = t0;
    let mut groups = 0u64;
    let (mut peer_t, mut local_t) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
    let mut local_n = 0u64;
    for n in 1..=max {
        let idx = 25;
        let start: usize = (0..idx).map(|i| st.doc.child(i).node_size()).sum();
        let old = st.doc.child(idx).clone();
        let mut text: String = (0..old.child_count())
            .map(|i| old.child(i).text().unwrap_or("").to_string())
            .collect();
        if text.len() > 60 {
            text = "p".into();
        }
        text.push((b'A' + (n % 26) as u8) as char);
        let p = s
            .branch("paragraph", Fragment::from_node(s.text(&text).unwrap()))
            .unwrap();
        let tp = std::time::Instant::now();
        st = remote(&st, |tr| {
            tr.replace_with(start, start + old.node_size(), Fragment::from_node(p))
                .unwrap();
        });
        peer_t += tp.elapsed();
        if n % 40 == 0 {
            groups += 1;
            let tl = std::time::Instant::now();
            // my keystroke in paragraph 3 (keep it short: delete when long)
            let pstart: usize = (0..3).map(|i| st.doc.child(i).node_size()).sum();
            let plen = st.doc.child(3).content_size();
            if plen > 40 {
                st = local(&st, groups * 1000, |tr| {
                    tr.delete(pstart + 1, pstart + 1 + plen - 5).unwrap();
                });
            } else {
                st = type_at(&st, pstart + 1, "x", groups * 1000);
            }
            local_t += tl.elapsed();
            local_n += 1;
            if groups.is_multiple_of(7)
                && let Some(u) = st.run("undo")
            {
                st = u;
            }
            if groups.is_multiple_of(11)
                && let Some(r) = st.run("redo")
            {
                st = r;
            }
        }
        if n >= next {
            let now = std::time::Instant::now();
            eprintln!(
                "MIXED n={n} batch_us_per_tx={:.2} peer_avg_us={:.2} local_avg_us={:.2} rss_delta_kb={}",
                (now - last).as_secs_f64() * 1e6 / (n / 2) as f64,
                peer_t.as_secs_f64() * 1e6 / n as f64,
                local_t.as_secs_f64() * 1e6 / local_n.max(1) as f64,
                rss_kb().saturating_sub(base)
            );
            last = now;
            next *= 2;
        }
    }
    let u0 = std::time::Instant::now();
    let mut k = 0;
    while let Some(u) = st.run("undo") {
        st = u;
        k += 1;
    }
    eprintln!(
        "MIXED undo-all {k} undos in {:.1} ms",
        u0.elapsed().as_secs_f64() * 1e3
    );
}
