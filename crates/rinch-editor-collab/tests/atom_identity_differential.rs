//! A seeded random differential for an inline atom's identity (review of #1503, F2).
//!
//! Two peers share a random history (typing, Enter, Backspace joins, pictures inserted,
//! moved, copied, replaced and deleted, attrs and `src` set); then peer A sets `board`
//! on one picture (half the time after a random edit of its own) while peer B makes one
//! more random edit that keeps that picture. After the
//! peers sync, the board is:
//!
//! * **wrong** when it shows on any other picture: what an app must never see, since
//!   what it draws over a picture would show over another. The test asserts there is
//!   none.
//! * **doubled** when both peers moved the target at once and the converged document
//!   holds it twice, the board on both copies of it: counted and printed.
//! * **lost** when it shows on no picture: counted and printed, and asserted only for
//!   the edits that cannot lose it. A `src` change of the target loses it by design (a
//!   `src` change makes a new picture: a wrong attribution is worse than a lost one).
//!
//! Every picture carries a unique `title`, which is how the test tells them apart; the
//! projection never reads it. `ATOM_DIFF_SEEDS` runs more seeds (default 60, both
//! client-id orders); `ATOM_DIFF_ONLY` one seed, printing the histories.
#![allow(dead_code)]
use std::collections::HashMap;
use std::rc::Rc;

use rinch_editor_collab::testing::{session_from_bytes_with_client_id, session_with_client_id};
use rinch_editor_collab::{CollabPlugin, CollabSession};
use rinch_editor_core::{
    AttrSpec, AttrValue, Attrs, EditorState, Fragment, Node, NodeSpec, Plugin, Pos, Schema,
    Selection, SetNodeAttrStep, Slice, Transaction, default_plugins,
};

const SRC: &str = "pimble-blob:6f1c2a0e/b3-9f86d081884c7d65";
const ID_ORDERS: [(u64, u64); 2] = [(11, 22), (22, 11)];

/// `doc > paragraph+ > (text | image | hard_break)*`, with `image` carrying `src`,
/// `alt`, `title` and, when `app` is set, `board` and `width`.
fn schema(app: bool) -> Rc<Schema> {
    let mut b = Schema::builder();
    b = b.node("doc", NodeSpec::builder("doc").content("block+").build());
    b = b.node(
        "paragraph",
        NodeSpec::builder("paragraph")
            .content("inline*")
            .group("block")
            .build(),
    );
    b = b.node(
        "text",
        NodeSpec::builder("text").group("inline").inline().build(),
    );
    b = b.node("hard_break", {
        let mut spec = NodeSpec::atom("hard_break");
        spec.group = Some("inline".into());
        spec.inline = true;
        spec
    });
    b = b.node("image", {
        let mut spec = NodeSpec::atom("image");
        spec.group = Some("inline".into());
        spec.inline = true;
        spec.attrs.insert("src".into(), AttrSpec::required());
        spec.attrs.insert("alt".into(), AttrSpec::optional(""));
        spec.attrs.insert("title".into(), AttrSpec::optional(""));
        if app {
            spec.attrs.insert("board".into(), AttrSpec::optional(""));
            spec.attrs.insert("width".into(), AttrSpec::optional(0i64));
        }
        spec
    });
    Rc::new(b.build())
}

fn plugins() -> Vec<Rc<dyn Plugin>> {
    let mut p = default_plugins();
    p.push(Rc::new(CollabPlugin));
    p
}

fn image(s: &Schema, src: &str, alt: &str) -> Node {
    s.create_node(
        "image",
        Attrs::new()
            .with("src", AttrValue::from(src))
            .with("alt", AttrValue::from(alt)),
        Fragment::empty(),
    )
    .unwrap()
}

fn para(s: &Schema, children: Vec<Node>) -> Node {
    s.branch("paragraph", Fragment::from_children(children))
        .unwrap()
}

/// `ab` + an image + `cd` (the image at model position 3), and, when `second` is set,
/// a second line `ef` + another image + `gh` (that image at position 10).
fn document(s: &Schema, second: bool) -> Node {
    let mut blocks = vec![para(
        s,
        vec![
            s.text("ab").unwrap(),
            image(s, SRC, "old alt"),
            s.text("cd").unwrap(),
        ],
    )];
    if second {
        blocks.push(para(
            s,
            vec![
                s.text("ef").unwrap(),
                image(s, "pimble-blob:two", "second"),
                s.text("gh").unwrap(),
            ],
        ));
    }
    s.branch("doc", Fragment::from_children(blocks)).unwrap()
}

struct Peer {
    state: EditorState,
    session: CollabSession,
}

impl Peer {
    fn local(&mut self, f: impl FnOnce(&mut Transaction)) {
        let mut tr = self.state.tr();
        f(&mut tr);
        let before = self.state.doc.clone();
        let after = self.state.apply(tr);
        self.session
            .record_local(self.state.schema(), &before, &after.doc)
            .expect("project local");
        self.state = after;
    }

    /// Set attr `name` of the `nth` image in the document, wherever it now is.
    fn set_nth(&mut self, nth: usize, name: &str, value: AttrValue) {
        let at = image_positions(&self.state.doc)[nth];
        self.local(|tr| {
            tr.step(Box::new(SetNodeAttrStep::new(at, name, value)))
                .unwrap();
        });
    }

    fn set(&mut self, name: &str, value: impl Into<AttrValue>) {
        self.set_nth(0, name, value.into());
    }

    fn type_at(&mut self, at: usize, text: &str) {
        self.local(|tr| {
            tr.set_selection(Selection::cursor(Pos(at)));
            tr.insert_text(text).unwrap();
        });
    }

    fn enter_at(&mut self, at: usize) {
        self.local(|tr| {
            tr.split(at, 1, None).unwrap();
        });
    }
}

/// Every image's position, in document order.
fn image_positions(doc: &Node) -> Vec<usize> {
    let mut out = Vec::new();
    doc.nodes_between(0, doc.content_size(), &mut |n, pos, _| {
        if n.type_name() == "image" {
            out.push(pos);
        }
        true
    });
    out
}

/// Every image's attrs, in document order.
fn images(doc: &Node) -> Vec<Attrs> {
    let mut out = Vec::new();
    doc.nodes_between(0, doc.content_size(), &mut |n, _, _| {
        if n.type_name() == "image" {
            out.push(n.attrs().clone());
        }
        true
    });
    out
}

fn attr(a: &Attrs, k: &str) -> String {
    match a.get(k) {
        Some(AttrValue::Str(s)) => s.to_string(),
        Some(AttrValue::Int(i)) => i.to_string(),
        Some(other) => format!("{other:?}"),
        // Absent: an attr the image was never given reads as nothing.
        None => String::new(),
    }
}

fn two_peers(schema: &Rc<Schema>, doc: Node, ids: (u64, u64)) -> (Peer, Peer) {
    let a_state = EditorState::create(schema.clone(), doc, plugins());
    let session_a = session_with_client_id(&a_state, ids.0).expect("session A");
    let b = peer_from_bytes(schema, &session_a.snapshot(), ids.1);
    (
        Peer {
            state: a_state,
            session: session_a,
        },
        b,
    )
}

fn peer_from_bytes(schema: &Rc<Schema>, bytes: &[u8], id: u64) -> Peer {
    let session = session_from_bytes_with_client_id(bytes, id).expect("join");
    let doc = session.projected_doc(schema).expect("project");
    Peer {
        state: EditorState::create(schema.clone(), doc, plugins()),
        session,
    }
}

/// Exchange state vectors and diffs until neither side changes.
fn sync(a: &mut Peer, b: &mut Peer) {
    for _ in 0..8 {
        let to_b = a.session.sync_diff(&b.session.state_vector()).unwrap();
        let to_a = b.session.sync_diff(&a.session.state_vector()).unwrap();
        let mut changed = false;
        if let Some(ns) = b.session.integrate_incremental(&b.state, &to_b).unwrap() {
            b.state = ns;
            changed = true;
        }
        if let Some(ns) = a.session.integrate_incremental(&a.state, &to_a).unwrap() {
            a.state = ns;
            changed = true;
        }
        if !changed {
            return;
        }
    }
    panic!("the state-vector exchange did not settle");
}

/// Both peers hold one document, each what its CRDT reads as, and so does a peer
/// joining now from either's snapshot. Returns the images.
fn converged(a: &Peer, b: &Peer, schema: &Rc<Schema>) -> Vec<Attrs> {
    assert_eq!(a.state.doc, b.state.doc, "the peers diverged");
    for peer in [a, b] {
        assert_eq!(
            peer.state.doc,
            peer.session.projected_doc(schema).unwrap(),
            "a peer's document is not what its CRDT holds"
        );
    }
    let late = CollabSession::from_bytes(&b.session.snapshot()).unwrap();
    assert_eq!(late.projected_doc(schema).unwrap(), a.state.doc);
    images(&a.state.doc)
}

// --- the differential ---------------------------------------------------------

struct R(u64);
impl R {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.next() % 100 < pct
    }
}

fn titled(s: &Schema, title: &str, src: &str) -> Node {
    s.create_node(
        "image",
        Attrs::new()
            .with("src", AttrValue::from(src))
            .with("alt", AttrValue::from("a"))
            .with("title", AttrValue::from(title)),
        Fragment::empty(),
    )
    .unwrap()
}

/// (start of content, end of content) of every paragraph.
fn paras(doc: &Node) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    doc.nodes_between(0, doc.content_size(), &mut |n, pos, _| {
        if n.type_name() == "paragraph" {
            out.push((pos + 1, pos + 1 + n.content_size()));
        }
        true
    });
    out
}

fn inline_pos(r: &mut R, doc: &Node) -> usize {
    let ps = paras(doc);
    let (a, b) = ps[r.below(ps.len())];
    a + r.below(b - a + 1)
}

/// Every picture whose title is `t`: its position and node.
fn with_title(doc: &Node, t: &str) -> Vec<(usize, Node)> {
    let mut out = Vec::new();
    doc.nodes_between(0, doc.content_size(), &mut |n, pos, _| {
        if n.type_name() == "image" && n.attrs().get_str("title") == Some(t) {
            out.push((pos, n.clone()));
        }
        true
    });
    out
}

fn titles(doc: &Node) -> Vec<String> {
    images(doc).iter().map(|a| attr(a, "title")).collect()
}

/// A unique title per run (`seed`, `ids`, a counter), so runs never share one.
struct Titles(String, usize);
impl Titles {
    fn fresh(&mut self) -> String {
        self.1 += 1;
        format!("{}-{}", self.0, self.1)
    }
}

const OPS: [&str; 11] = [
    "type",
    "enter",
    "join",
    "insert-image",
    "set-attr",
    "set-attr",
    "delete-image",
    "move-image",
    "copy-image",
    "replace-image",
    "set-src",
];

/// One random edit on `p`. `keep`: a title whose picture this edit must not delete,
/// replace or copy (it may move it).
fn random_op(
    r: &mut R,
    t: &mut Titles,
    s: &Schema,
    p: &mut Peer,
    keep: Option<&str>,
) -> &'static str {
    let doc = p.state.doc.clone();
    let ts = titles(&doc);
    let others: Vec<&String> = ts.iter().filter(|x| Some(x.as_str()) != keep).collect();
    let op = OPS[r.below(OPS.len())];
    match op {
        "type" => {
            let at = inline_pos(r, &doc);
            p.type_at(at, "x");
        }
        "enter" => {
            let at = inline_pos(r, &doc);
            p.enter_at(at);
        }
        "join" => {
            let ps = paras(&doc);
            if ps.len() < 2 {
                return "noop";
            }
            let k = 1 + r.below(ps.len() - 1);
            let open = ps[k].0 - 1;
            p.local(|tr| {
                tr.delete(open - 1, open + 1).unwrap();
            });
        }
        "insert-image" => {
            // Right before or after a picture (where the text diff could mistake one
            // for the other), or anywhere.
            let at = if !ts.is_empty() && r.chance(70) {
                let (pos, _) = with_title(&doc, &ts[r.below(ts.len())])[0].clone();
                pos + r.below(2)
            } else {
                inline_pos(r, &doc)
            };
            let src = if r.chance(50) { "s1" } else { "s2" };
            let n = titled(s, &t.fresh(), src);
            p.local(|tr| {
                tr.replace(at, at, Slice::new(Fragment::from_node(n), 0, 0))
                    .unwrap();
            });
        }
        "set-attr" => {
            if ts.is_empty() {
                return "noop";
            }
            let (at, _) = with_title(&doc, &ts[r.below(ts.len())])[0].clone();
            let k = ["alt", "board", "width"][r.below(3)];
            let v = if k == "width" {
                AttrValue::Int(1 + r.below(500) as i64)
            } else {
                AttrValue::from(format!("{k}{}", r.below(9)))
            };
            p.local(|tr| {
                tr.step(Box::new(SetNodeAttrStep::new(at, k, v))).unwrap();
            });
        }
        "delete-image" => {
            if others.is_empty() {
                return "noop";
            }
            let (at, _) = with_title(&doc, others[r.below(others.len())])[0].clone();
            p.local(|tr| {
                tr.delete(at, at + 1).unwrap();
            });
        }
        "move-image" => {
            // A drag: the same node, taken out and put back elsewhere in one step.
            if ts.is_empty() {
                return "noop";
            }
            let (from, node) = with_title(&doc, &ts[r.below(ts.len())])[0].clone();
            p.local(|tr| {
                tr.delete(from, from + 1).unwrap();
                let doc = tr.doc().clone();
                let at = inline_pos(r, &doc);
                tr.replace(at, at, Slice::new(Fragment::from_node(node), 0, 0))
                    .unwrap();
            });
        }
        "copy-image" => {
            // A copy within the editor keeps the node: the same `Rc` twice.
            if others.is_empty() {
                return "noop";
            }
            let (_, node) = with_title(&doc, others[r.below(others.len())])[0].clone();
            let at = inline_pos(r, &doc);
            p.local(|tr| {
                tr.replace(at, at, Slice::new(Fragment::from_node(node), 0, 0))
                    .unwrap();
            });
        }
        "set-src" => {
            // The target included: a `src` change makes a new picture, and a
            // concurrent change of the old one is lost (never shown on another).
            if ts.is_empty() {
                return "noop";
            }
            let (at, _) = with_title(&doc, &ts[r.below(ts.len())])[0].clone();
            let v = AttrValue::from(format!("s{}", 4 + r.below(9)));
            p.local(|tr| {
                tr.step(Box::new(SetNodeAttrStep::new(at, "src", v)))
                    .unwrap();
            });
        }
        _ => {
            // A picture pasted over a selected one: the same place, a new node.
            if others.is_empty() {
                return "noop";
            }
            let (at, _) = with_title(&doc, others[r.below(others.len())])[0].clone();
            let n = titled(s, &t.fresh(), "s3");
            p.local(|tr| {
                tr.replace(at, at + 1, Slice::new(Fragment::from_node(n), 0, 0))
                    .unwrap();
            });
        }
    }
    op
}

fn start_doc(s: &Schema, t: &mut Titles) -> Node {
    let (a, b, c) = (t.fresh(), t.fresh(), t.fresh());
    s.branch(
        "doc",
        Fragment::from_children(vec![
            para(
                s,
                vec![
                    s.text("ab").unwrap(),
                    titled(s, &a, "s1"),
                    titled(s, &b, "s1"),
                    s.text("cd").unwrap(),
                ],
            ),
            para(
                s,
                vec![
                    s.text("ef").unwrap(),
                    titled(s, &c, "s2"),
                    s.text("gh").unwrap(),
                ],
            ),
        ]),
    )
    .unwrap()
}

#[derive(Default)]
struct Tally {
    checked: usize,
    wrong: Vec<String>,
    lost: HashMap<&'static str, usize>,
    by_op: HashMap<&'static str, usize>,
    history_copies: Vec<String>,
    lost_cases: Vec<String>,
    doubled: Vec<String>,
}

fn run(seed: u64, ids: (u64, u64), verbose: bool, tally: &mut Tally) {
    let s = schema(true);
    let mut r = R(seed.wrapping_mul(2654435761).wrapping_add(7) | 1);
    let mut t = Titles(format!("t{seed}.{}", ids.0), 0);
    let (mut a, mut b) = two_peers(&s, start_doc(&s, &mut t), ids);
    for _ in 0..r.below(12) {
        let count = |doc: &Node| {
            let mut c: HashMap<String, usize> = HashMap::new();
            for title in titles(doc) {
                *c.entry(title).or_default() += 1;
            }
            c
        };
        let was = count(&a.state.doc);
        let (who, p) = if r.chance(50) {
            ("A", &mut a)
        } else {
            ("B", &mut b)
        };
        let op = random_op(&mut r, &mut t, &s, p, None);
        if verbose {
            eprintln!("{who} {op}: {:?}", titles(&p.state.doc));
        }
        sync(&mut a, &mut b);
        // A sequential edit makes a second picture of one title only by copying it.
        if op != "copy-image" {
            for (title, n) in count(&a.state.doc) {
                if n > was.get(&title).copied().unwrap_or(1) {
                    tally
                        .history_copies
                        .push(format!("seed {seed} {ids:?} after {op}: {n} of {title}"));
                }
            }
        }
    }
    // A target with exactly one picture.
    let ts: Vec<String> = titles(&a.state.doc)
        .into_iter()
        .filter(|x| with_title(&a.state.doc, x).len() == 1)
        .collect();
    if ts.is_empty() {
        return;
    }
    let target = ts[r.below(ts.len())].clone();
    let mine = format!("board-of-{seed}");
    // Half the time A also makes an edit of its own first, concurrent with B's (a
    // move, a join, a copy beside the target: both sides' writes then meet in yrs).
    let a_op = if r.chance(50) {
        random_op(&mut r, &mut t, &s, &mut a, Some(&target))
    } else {
        "none"
    };
    if with_title(&a.state.doc, &target).len() != 1 {
        return;
    }
    let (at, _) = with_title(&a.state.doc, &target)[0].clone();
    a.local(|tr| {
        tr.step(Box::new(SetNodeAttrStep::new(
            at,
            "board",
            AttrValue::from(mine.as_str()),
        )))
        .unwrap();
    });
    let b_doc = b.state.doc.clone();
    let op = random_op(&mut r, &mut t, &s, &mut b, Some(&target));
    if verbose {
        eprintln!(
            "B before {b_doc:?}\n  {:?}\nB after {:?}\n  {:?}",
            images(&b_doc),
            b.state.doc,
            images(&b.state.doc)
        );
    }
    let b_sets_board = with_title(&b.state.doc, &target)
        .first()
        .is_some_and(|(_, n)| n.attrs().get("board").is_some());

    sync(&mut a, &mut b);
    let imgs = converged(&a, &b, &s);
    tally.checked += 1;
    *tally.by_op.entry(op).or_default() += 1;
    let carrying: Vec<String> = imgs
        .iter()
        .filter(|i| attr(i, "board") == mine)
        .map(|i| attr(i, "title"))
        .collect();
    if verbose {
        eprintln!("target {target}; B {op}; boards on {carrying:?}; {imgs:?}");
    }
    if carrying.len() > 1 && carrying.iter().all(|x| *x == target) {
        // Both peers moved the target at once (Enter before it on each side, a drag
        // and an Enter): yrs keeps both new chars, so the converged document holds the
        // picture twice, each copy carrying its identity until one is changed
        // (`two_copies_from_concurrent_splits_are_edited_apart`). The board is on the
        // target, twice; counted on its own.
        tally.doubled.push(format!(
            "seed {seed} {ids:?} A {a_op} B {op}: {target} is doubled, the board on both"
        ));
    } else if carrying.iter().any(|x| *x != target) {
        tally.wrong.push(format!(
            "seed {seed} {ids:?} A {a_op} B {op}: the board of {target} shows on {carrying:?}"
        ));
    } else if carrying.is_empty() && !b_sets_board {
        *tally.lost.entry(op).or_default() += 1;
        tally.lost_cases.push(format!(
            "seed {seed} {ids:?} B {op}: the board of {target} is lost"
        ));
    }
}

#[test]
fn a_board_never_shows_on_another_picture() {
    let seeds: u64 = std::env::var("ATOM_DIFF_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let only: Option<u64> = std::env::var("ATOM_DIFF_ONLY")
        .ok()
        .and_then(|v| v.parse().ok());
    let mut tally = Tally::default();
    for seed in 1..=seeds {
        if only.is_some_and(|o| o != seed) {
            continue;
        }
        for ids in ID_ORDERS {
            run(seed, ids, only.is_some(), &mut tally);
        }
    }
    let mut lost: Vec<_> = tally.lost.iter().collect();
    lost.sort();
    let mut by_op: Vec<_> = tally.by_op.iter().collect();
    by_op.sort();
    eprintln!(
        "checked {}; wrong {}; lost {} {lost:?}; doubled by concurrent moves {}; by B's edit {by_op:?}; copies in histories {}",
        tally.checked,
        tally.wrong.len(),
        tally.lost.values().sum::<usize>(),
        tally.doubled.len(),
        tally.history_copies.len()
    );
    for w in tally
        .wrong
        .iter()
        .chain(&tally.history_copies)
        .chain(&tally.lost_cases)
    {
        eprintln!("  {w}");
    }
    assert!(
        only.is_some() || tally.checked > seeds as usize,
        "positive control: {}",
        tally.checked
    );
    assert!(tally.wrong.is_empty(), "a board on another picture");
    assert!(
        tally.history_copies.is_empty(),
        "a picture duplicated by a sequential history"
    );
    // B's edits that keep the target where it is, or move it in one step, lose nothing.
    for op in [
        "type",
        "insert-image",
        "set-attr",
        "delete-image",
        "move-image",
        "copy-image",
    ] {
        assert_eq!(tally.lost.get(op), None, "{op} lost a board");
    }
}
