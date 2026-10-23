//! Issues #1850 and #1851: concurrent editing that converges but changes what was typed.
//!
//! A random two- or three-peer trial (typing, deletes, Backspace/Delete, `splitBlock`,
//! undo/redo, bold, hard breaks, images, code blocks) used to converge on a document in
//! which a typed character appeared **twice** (#1850) or an inline atom's char had lost
//! its `@atom` attribute and read back as a literal U+FFFC character (#1851). Both peers
//! agreed, so a convergence check could not see either.

use std::rc::Rc;

use rinch_editor_collab::testing::{session_from_bytes_with_client_id, session_with_client_id};
use rinch_editor_collab::{CollabPlugin, CollabSession};
use rinch_editor_core::{
    AttrValue, Attrs, EditorState, Fragment, Node, Plugin, Pos, Schema, Selection, Slice,
    default_plugins,
};

fn plugins() -> Vec<Rc<dyn Plugin>> {
    let mut p = default_plugins();
    p.push(Rc::new(CollabPlugin));
    p
}

fn schema() -> Rc<Schema> {
    Rc::new(Schema::starter_kit())
}

fn para(s: &Schema, text: &str) -> Node {
    let content = if text.is_empty() {
        Fragment::empty()
    } else {
        Fragment::from_node(s.text(text).unwrap())
    };
    s.branch("paragraph", content).unwrap()
}

fn doc_of(s: &Schema, blocks: Vec<Node>) -> Node {
    s.branch("doc", Fragment::from_children(blocks)).unwrap()
}

struct Peer {
    schema: Rc<Schema>,
    state: EditorState,
    session: CollabSession,
}

impl Peer {
    fn host(schema: &Rc<Schema>, blocks: Vec<Node>, client_id: u64) -> Peer {
        let state = EditorState::create(schema.clone(), doc_of(schema, blocks), plugins());
        let session = session_with_client_id(&state, client_id).unwrap();
        Peer {
            schema: schema.clone(),
            state,
            session,
        }
    }

    fn join(&self, client_id: u64) -> Peer {
        let session =
            session_from_bytes_with_client_id(&self.session.snapshot(), client_id).unwrap();
        let doc = session.projected_doc(&self.schema).unwrap();
        Peer {
            schema: self.schema.clone(),
            state: EditorState::create(self.schema.clone(), doc, plugins()),
            session,
        }
    }

    fn commit(&mut self, next: EditorState) {
        self.session
            .record_local(&self.schema, &self.state.doc, &next.doc)
            .expect("the edit projects");
        self.state = next;
        let projected = self.session.projected_doc(&self.schema).unwrap();
        assert_eq!(
            self.state.doc, projected,
            "after a local edit: model ≡ project(model)"
        );
    }

    fn local_replace(&mut self, at: usize, content: Fragment) {
        let mut tr = self.state.tr();
        tr.replace(at, at, Slice::new(content, 0, 0)).unwrap();
        let next = self.state.apply(tr);
        self.commit(next);
    }

    fn send(&mut self) -> Vec<u8> {
        self.session.save_incremental().unwrap()
    }

    fn receive(&mut self, delta: &[u8]) {
        if let Some(next) = self
            .session
            .integrate_incremental(&self.state, delta)
            .unwrap()
        {
            self.state = next;
        }
        let projected = self.session.projected_doc(&self.schema).unwrap();
        assert_eq!(
            self.state.doc, projected,
            "after a remote change: model ≡ project(model)"
        );
    }
}

/// A textblock's text, an inline atom as `[name]`, blocks joined by `|`.
fn doc_text(doc: &Node) -> String {
    fn walk(n: &Node, out: &mut Vec<String>) {
        if n.is_textblock() {
            let mut t = String::new();
            for i in 0..n.child_count() {
                let c = n.child(i);
                match c.text() {
                    Some(s) => t.push_str(s),
                    None => t.push_str(&format!("[{}]", c.type_name())),
                }
            }
            out.push(t);
            return;
        }
        for i in 0..n.child_count() {
            walk(n.child(i), out);
        }
    }
    let mut out = Vec::new();
    walk(doc, &mut out);
    out.join("|")
}
// --- the trial, as a replayable list of operations ------------------------------------

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
        (self.next() % n as u64) as usize
    }
}

#[derive(Clone, Debug)]
enum Op {
    Type(usize, usize, char),
    Code(usize, usize, bool),
    Bold(usize, usize, usize),
    Break(usize, usize),
    Image(usize, usize),
    DeleteSel(usize, usize, usize),
    Backspace(usize, usize),
    Delete(usize, usize),
    Split(usize, usize),
    Undo(usize),
    Redo(usize),
    /// Peer `p` sends what it has not sent yet.
    Send(usize),
    /// Peer `p` integrates every delta it has not seen yet.
    Deliver(usize),
}

fn is_text_pos(doc: &Node, p: usize) -> bool {
    p <= doc.content_size()
        && doc
            .resolve(Pos(p))
            .ok()
            .is_some_and(|r| r.parent().is_textblock())
}

/// Apply `op` to `peer`. `false` when the op does not apply (the position names no
/// place in a textblock, or the command refuses) — a shrunk history skips it.
fn apply(peer: &mut Peer, op: &Op) -> bool {
    let st = &peer.state;
    let cursor = |p: usize| {
        let mut tr = st.tr();
        tr.set_selection(Selection::cursor(Pos(p)));
        st.apply(tr)
    };
    let range = |p: usize, q: usize| {
        let mut tr = st.tr();
        tr.set_selection(Selection::text(Pos(p.min(q)), Pos(p.max(q))));
        st.apply(tr)
    };
    let pos_ok = |p: usize| is_text_pos(&st.doc, p);
    let next = match *op {
        Op::Type(_, p, c) => {
            if !pos_ok(p) {
                return false;
            }
            let mut tr = st.tr();
            tr.set_selection(Selection::cursor(Pos(p)));
            if tr.insert_text(&c.to_string()).is_err() {
                return false;
            }
            Some(st.apply(tr))
        }
        Op::Code(_, p, code) => {
            if !pos_ok(p) {
                return false;
            }
            cursor(p).run(if code { "setCodeBlock" } else { "setParagraph" })
        }
        Op::Bold(_, p, q) => {
            if !pos_ok(p) || !pos_ok(q) {
                return false;
            }
            range(p, q).run("toggleBold")
        }
        Op::Break(_, p) => {
            if !pos_ok(p) {
                return false;
            }
            cursor(p).run("insertHardBreak")
        }
        Op::Image(_, p) => {
            if !pos_ok(p) {
                return false;
            }
            let img = st
                .schema()
                .create_node(
                    "image",
                    Attrs::new().with("src", AttrValue::from("a.png")),
                    Fragment::empty(),
                )
                .unwrap();
            let mut tr = st.tr();
            let ok = tr
                .replace(p, p, Slice::new(Fragment::from_node(img), 0, 0))
                .is_ok();
            ok.then(|| st.apply(tr))
        }
        Op::DeleteSel(_, p, q) => {
            if !pos_ok(p) || !pos_ok(q) {
                return false;
            }
            range(p, q).run("deleteSelection")
        }
        Op::Backspace(_, p) | Op::Delete(_, p) => {
            if !pos_ok(p) {
                return false;
            }
            let name = if matches!(op, Op::Backspace(..)) {
                "deleteCharBackward"
            } else {
                "deleteCharForward"
            };
            cursor(p).run(name)
        }
        Op::Split(_, p) => {
            if !pos_ok(p) {
                return false;
            }
            cursor(p).run("splitBlock")
        }
        Op::Undo(_) => st.run("undo"),
        Op::Redo(_) => st.run("redo"),
        Op::Send(_) | Op::Deliver(_) => unreachable!(),
    };
    match next {
        Some(next) => {
            peer.commit(next);
            true
        }
        None => false,
    }
}

fn peer_of(op: &Op) -> usize {
    match *op {
        Op::Type(p, ..)
        | Op::Code(p, ..)
        | Op::Bold(p, ..)
        | Op::Break(p, ..)
        | Op::Image(p, ..)
        | Op::DeleteSel(p, ..)
        | Op::Backspace(p, ..)
        | Op::Delete(p, ..)
        | Op::Split(p, ..)
        | Op::Undo(p)
        | Op::Redo(p)
        | Op::Send(p)
        | Op::Deliver(p) => p,
    }
}

/// One random edit for `peer`, in the probe's proportions.
fn random_op(rng: &mut Rng, peer: usize, doc: &Node, unique: &mut u32) -> Option<Op> {
    let pos = |rng: &mut Rng| {
        let p = rng.below(doc.content_size() + 1);
        is_text_pos(doc, p).then_some(p)
    };
    if rng.below(10) < 6 {
        let p = pos(rng)?;
        return Some(match rng.below(7) {
            0..=2 => {
                let c = char::from_u32(0x4E00 + *unique).unwrap();
                *unique += 1;
                Op::Type(peer, p, c)
            }
            3 => Op::Code(peer, p, rng.below(2) == 0),
            4 => Op::Bold(peer, p, pos(rng)?),
            5 => Op::Break(peer, p),
            _ => Op::Image(peer, p),
        });
    }
    let p = pos(rng)?;
    Some(match rng.below(5) {
        0 => Op::DeleteSel(peer, p, pos(rng)?),
        1 | 2 => {
            if rng.below(2) == 0 {
                Op::Backspace(peer, p)
            } else {
                Op::Delete(peer, p)
            }
        }
        3 => Op::Split(peer, p),
        _ => {
            if rng.below(2) == 0 {
                Op::Undo(peer)
            } else {
                Op::Redo(peer)
            }
        }
    })
}

const INITIAL: [char; 4] = ['\u{3041}', '\u{3042}', '\u{3043}', '\u{3044}'];

struct Run {
    peers: Vec<Peer>,
    queue: Vec<Vec<u8>>,
    seen: Vec<usize>,
    typed: Vec<char>,
}

impl Run {
    fn new(peers: usize) -> Run {
        let s = schema();
        let host = Peer::host(
            &s,
            vec![para(&s, "\u{3041}\u{3042}"), para(&s, "\u{3043}\u{3044}")],
            1,
        );
        let mut ps = vec![host];
        for i in 1..peers {
            let j = ps[0].join(1 + i as u64);
            ps.push(j);
        }
        let _ = ps[0].send();
        Run {
            peers: ps,
            queue: Vec::new(),
            seen: vec![0; peers],
            typed: INITIAL.to_vec(),
        }
    }

    fn step(&mut self, op: &Op) -> bool {
        match *op {
            Op::Send(p) => {
                let d = self.peers[p].send();
                if !d.is_empty() {
                    self.queue.push(d);
                }
                true
            }
            Op::Deliver(q) => {
                while self.seen[q] < self.queue.len() {
                    let d = self.queue[self.seen[q]].clone();
                    self.peers[q].receive(&d);
                    self.seen[q] += 1;
                }
                true
            }
            _ => {
                let p = peer_of(op);
                let applied = apply(&mut self.peers[p], op);
                if applied && let Op::Type(_, _, c) = op {
                    self.typed.push(*c);
                }
                applied
            }
        }
    }

    fn finish(&mut self) {
        for p in 0..self.peers.len() {
            self.step(&Op::Send(p));
        }
        for q in 0..self.peers.len() {
            self.step(&Op::Deliver(q));
        }
    }

    /// The converged document's defect, if any.
    fn verdict(&self) -> Option<String> {
        let doc = &self.peers[0].state.doc;
        for (q, p) in self.peers.iter().enumerate().skip(1) {
            if p.state.doc != *doc {
                return Some(format!("peer {q} diverged"));
            }
        }
        let text = doc_text(doc);
        if text.contains('\u{FFFC}') {
            return Some(format!("literal U+FFFC in {text:?}"));
        }
        for c in &self.typed {
            let n = text.matches(*c).count();
            if n > 1 {
                return Some(format!("typed {c:?} lands {n} times in {text:?}"));
            }
        }
        None
    }
}

/// The trial at `seed` as a list of operations (each edit followed by its peer's send).
/// A trial that panics part-way stops there; its ops end with the one that panicked.
fn trial_ops(seed: u64) -> (usize, Vec<Op>) {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xABCD | 1);
    let peers = 2 + (seed % 2) as usize;
    let mut run = Run::new(peers);
    let mut ops = Vec::new();
    let mut unique = 0u32;
    for _ in 0..100 {
        let op = if rng.below(10) < 6 {
            let p = rng.below(peers);
            let doc = run.peers[p].state.doc.clone();
            match random_op(&mut rng, p, &doc, &mut unique) {
                Some(op) => op,
                None => continue,
            }
        } else {
            Op::Deliver(rng.below(peers))
        };
        let stepped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run.step(&op)));
        match stepped {
            Ok(false) => {}
            Ok(true) => {
                let p = peer_of(&op);
                let edit = !matches!(op, Op::Deliver(_));
                ops.push(op);
                if edit {
                    ops.push(Op::Send(p));
                    run.step(&Op::Send(p));
                }
            }
            Err(_) => {
                ops.push(op);
                break;
            }
        }
    }
    (peers, ops)
}

/// Replay `ops` (skipping any that no longer apply) and return the defect.
fn replay(peers: usize, ops: &[Op]) -> Result<Option<String>, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut run = Run::new(peers);
        for op in ops {
            run.step(op);
        }
        run.finish();
        run.verdict()
    }))
    .map_err(|e| {
        e.downcast_ref::<String>()
            .cloned()
            .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default()
    })
}

/// The ceilings at the default 300 seeds (see the trial), measured when #1850 landed.
const DUP_300: usize = 16;
const PANIC_300: usize = 0;

/// How many seeds the trial runs (`TYPED_ONCE_SEEDS`, default 300).
fn seeds() -> u64 {
    std::env::var("TYPED_ONCE_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(300)
}

/// The trial's bad seeds by kind, each with its defect.
#[derive(Default)]
struct Tally {
    atom_as_text: Vec<String>,
    duplicate: Vec<String>,
    other: Vec<String>,
    panicked: Vec<String>,
}

fn tally(seeds: std::ops::RangeInclusive<u64>) -> Tally {
    let mut t = Tally::default();
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    for seed in seeds {
        let (peers, ops) = trial_ops(seed);
        match replay(peers, &ops) {
            Ok(None) => {}
            Ok(Some(d)) if d.contains("U+FFFC") => t.atom_as_text.push(format!("seed {seed}: {d}")),
            Ok(Some(d)) if d.contains("lands") => t.duplicate.push(format!("seed {seed}: {d}")),
            Ok(Some(d)) => t.other.push(format!("seed {seed}: {d}")),
            Err(p) => t.panicked.push(format!("seed {seed}: {p}")),
        }
    }
    std::panic::set_hook(hook);
    t
}

/// No atom reads back as text, ever (#1851), and the peers always converge. A typed
/// char lands twice only where two peers moved the same text at once (Enter at one
/// place on both, both joining the same lines, a join while the other splits): the
/// projection has no move, so each copy is a fresh write (#1850; #1857 is the design
/// that removes them). Those, and an undo
/// after remote changes that leaves a model the projection does not read back (an
/// editor-core history bug, #1856), are held to the counts measured when this
/// landed: the trial is deterministic (fixed client ids), so a count that grows is a
/// regression. Measured over seeds 1..=1000: 0 literal U+FFFC (14 before #1851),
/// 39 duplicates (104 before #1850's placement fix), 3 undo failures.
#[test]
fn randomized_typed_text_lands_once_and_atoms_stay_atoms() {
    let n = seeds();
    let t = tally(1..=n);
    let report = |v: &[String]| v.join("\n");
    assert!(t.atom_as_text.is_empty(), "{}", report(&t.atom_as_text));
    assert!(t.other.is_empty(), "{}", report(&t.other));
    // Ceilings at the default 300 seeds and at 1000.
    let (dup_max, panic_max) = match n {
        300 => (DUP_300, PANIC_300),
        1000 => (39, 3),
        _ => (usize::MAX, usize::MAX),
    };
    assert!(
        t.duplicate.len() <= dup_max,
        "{} duplicates (ceiling {dup_max}):\n{}",
        t.duplicate.len(),
        report(&t.duplicate)
    );
    assert!(
        t.panicked.len() <= panic_max,
        "{} panics (ceiling {panic_max}):\n{}",
        t.panicked.len(),
        report(&t.panicked)
    );
}

/// Shrinks a failing trial to a minimal history (`SHRINK_SEED=n`, `--ignored --nocapture`).
#[test]
#[ignore]
fn shrink() {
    let seed: u64 = std::env::var("SHRINK_SEED").unwrap().parse().unwrap();
    let (peers, mut ops) = trial_ops(seed);
    let class = |d: &Result<Option<String>, String>| match d {
        Ok(Some(s)) if s.contains("U+FFFC") => Some("fffc"),
        Ok(Some(s)) if s.contains("lands") => Some("dup"),
        Ok(Some(_)) => Some("other"),
        Err(_) => Some("panic"),
        Ok(None) => None,
    };
    std::panic::set_hook(Box::new(|_| {}));
    let want = class(&replay(peers, &ops)).expect("the seed fails");
    let want = std::env::var("SHRINK_CLASS")
        .ok()
        .map(|s| &*Box::leak(s.into_boxed_str()))
        .unwrap_or(want);
    loop {
        let mut shrunk = false;
        let mut chunk = ops.len() / 2;
        while chunk >= 1 {
            let mut i = 0;
            while i < ops.len() {
                let mut cand = ops.clone();
                cand.drain(i..(i + chunk).min(ops.len()));
                if class(&replay(peers, &cand)) == Some(want) {
                    ops = cand;
                    shrunk = true;
                } else {
                    i += chunk;
                }
            }
            chunk /= 2;
        }
        if !shrunk {
            break;
        }
    }
    let _ = std::panic::take_hook();
    eprintln!("seed {seed} ({want}), {peers} peers, {} ops:", ops.len());
    for op in &ops {
        eprintln!("  {op:?},");
    }
    eprintln!("=> {:?}", replay(peers, &ops));
}

// --- minimal histories ------------------------------------------------------------------

fn two_peers(blocks: impl Fn(&Schema) -> Vec<Node>) -> (Peer, Peer) {
    let s = schema();
    let a = Peer::host(&s, blocks(&s), 1);
    let mut b = a.join(2);
    let mut a = a;
    let _ = a.send();
    let _ = b.send();
    (a, b)
}

fn exchange(a: &mut Peer, b: &mut Peer) {
    let (da, db) = (a.send(), b.send());
    a.receive(&db);
    b.receive(&da);
    assert_eq!(a.state.doc, b.state.doc, "converged");
}

fn image(s: &Schema) -> Node {
    s.create_node(
        "image",
        Attrs::new().with("src", AttrValue::from("a.png")),
        Fragment::empty(),
    )
    .unwrap()
}

/// `<p><img>a<br>b</p><p>cd</p>`: the break is 3..4.
fn with_break(s: &Schema) -> Vec<Node> {
    let br = s
        .create_node("hard_break", Attrs::new(), Fragment::empty())
        .unwrap();
    let p = s
        .branch(
            "paragraph",
            Fragment::from_children(vec![
                image(s),
                s.text("a").unwrap(),
                br,
                s.text("b").unwrap(),
            ]),
        )
        .unwrap();
    vec![p, para(s, "cd")]
}

/// #1851: a hard break inserted right after one a peer deletes concurrently.
#[test]
fn a_break_inserted_beside_one_a_peer_deletes_stays_a_break() {
    for b_first in [false, true] {
        let (mut a, mut b) = two_peers(with_break);
        assert!(apply(&mut a, &Op::DeleteSel(0, 2, 4)));
        assert!(apply(&mut b, &Op::Break(1, 4)));
        if b_first {
            let db = b.send();
            a.receive(&db);
            let da = a.send();
            b.receive(&da);
        } else {
            exchange(&mut a, &mut b);
        }
        assert_eq!(doc_text(&a.state.doc), "[image][hard_break]b|cd");
    }
}

/// #1850: two peers press Enter at the start of one line.
#[test]
fn enter_at_a_lines_start_on_two_peers_keeps_the_line_once() {
    let (mut a, mut b) = two_peers(|s| vec![para(s, "ab"), para(s, "cd")]);
    assert!(apply(&mut a, &Op::Split(0, 1)));
    assert!(apply(&mut b, &Op::Split(1, 1)));
    exchange(&mut a, &mut b);
    assert_eq!(doc_text(&a.state.doc), "||ab|cd");
}

/// `<p>a<img>x<img>b</p><p>cd</p>`: `x` is 3..4.
fn x_between_images(s: &Schema) -> Vec<Node> {
    let p = s
        .branch(
            "paragraph",
            Fragment::from_children(vec![
                s.text("a").unwrap(),
                image(s),
                s.text("x").unwrap(),
                image(s),
                s.text("b").unwrap(),
            ]),
        )
        .unwrap();
    vec![p, para(s, "cd")]
}

/// #1851: the text between two equal pictures deleted while a peer types after the
/// first.
#[test]
fn typing_between_two_pictures_whose_text_a_peer_deletes_keeps_both_pictures() {
    for b_first in [false, true] {
        let (mut a, mut b) = two_peers(x_between_images);
        assert!(apply(&mut a, &Op::DeleteSel(0, 3, 4)));
        assert!(apply(&mut b, &Op::Type(1, 3, 'y')));
        if b_first {
            let db = b.send();
            a.receive(&db);
            let da = a.send();
            b.receive(&da);
        } else {
            exchange(&mut a, &mut b);
        }
        assert_eq!(doc_text(&a.state.doc), "a[image]y[image]b|cd");
    }
}

/// #1850: Enter at the start of a line while a peer types in it. The line keeps its
/// text in place (the empty block is the one inserted), so the peer's char stays in it.
#[test]
fn typing_in_a_line_a_peer_splits_at_its_start_stays_in_the_line() {
    let (mut a, mut b) = two_peers(|s| vec![para(s, "ab"), para(s, "cd")]);
    assert!(apply(&mut a, &Op::Split(0, 1)));
    assert!(apply(&mut b, &Op::Type(1, 2, 'x')));
    exchange(&mut a, &mut b);
    assert_eq!(doc_text(&a.state.doc), "|axb|cd");
}

fn sticky(p: &Peer, pos: usize) -> Vec<u8> {
    p.session.sticky_index(&p.state.doc, Pos(pos)).unwrap()
}

fn resolve(p: &Peer, bytes: &[u8]) -> Option<usize> {
    p.session.resolve_sticky(&p.state.doc, bytes).map(|q| q.0)
}

/// #1850 (#917's two cases): Enter at the start of a paragraph and Backspace deleting
/// the empty line above it keep a sticky index on its chars, at the top level and in a
/// list. They used to copy the paragraph's text into another block: the index
/// resolved into the new empty paragraph, then to `None`.
#[test]
fn enter_at_a_lines_start_and_deleting_the_empty_line_keep_its_sticky_indexes() {
    let (mut a, _) = two_peers(|s| vec![para(s, "ab"), para(s, "hello")]);
    // `<p>ab</p><p>hello</p>`: `l` is at 7.
    let l = sticky(&a, 7);
    assert!(apply(&mut a, &Op::Split(0, 5)));
    assert_eq!(doc_text(&a.state.doc), "ab||hello");
    assert_eq!(resolve(&a, &l), Some(9), "the `l` of `hello`, one block on");
    assert!(apply(&mut a, &Op::Backspace(0, 7)));
    assert_eq!(doc_text(&a.state.doc), "ab|hello");
    assert_eq!(resolve(&a, &l), Some(7));

    let s = schema();
    let item = |t: &str| {
        s.branch("list_item", Fragment::from_node(para(&s, t)))
            .unwrap()
    };
    let list = s
        .branch(
            "bullet_list",
            Fragment::from_children(vec![item("ab"), item("hello")]),
        )
        .unwrap();
    let mut a = Peer::host(&s, vec![list], 1);
    // `<ul><li><p>ab</p></li><li><p>hello</p></li></ul>`: `hello` starts at 9.
    let l = sticky(&a, 11);
    let mut tr = a.state.tr();
    tr.split(9, 1, None).unwrap();
    let next = a.state.apply(tr);
    a.commit(next);
    assert_eq!(doc_text(&a.state.doc), "ab||hello");
    assert_eq!(resolve(&a, &l), Some(13));
}

/// Block `block`'s text as stored: each run of chars with its `@atom` value's keys and
/// `@for`, read with yrs alone.
fn raw_atom_runs(p: &Peer, block: u32) -> Vec<(String, Option<String>)> {
    use yrs::types::text::YChange;
    use yrs::updates::decoder::Decode;
    use yrs::{Any, Array, Map, Out, Text, Transact, Update};
    let doc = yrs::Doc::with_options(yrs::Options {
        offset_kind: yrs::OffsetKind::Utf16,
        ..Default::default()
    });
    let content = doc.get_or_insert_array("content");
    doc.transact_mut()
        .apply_update(Update::decode_v1(&p.session.snapshot()).unwrap())
        .unwrap();
    let txn = doc.transact();
    let Some(Out::YMap(m)) = content.get(&txn, block) else {
        panic!("no block {block}");
    };
    let Some(Out::YText(t)) = m.get(&txn, "text") else {
        panic!("block {block} has no text");
    };
    t.diff(&txn, YChange::identity)
        .into_iter()
        .map(|d| {
            let Out::Any(Any::String(s)) = d.insert else {
                panic!("not a string")
            };
            let atom_for = d
                .attributes
                .as_ref()
                .and_then(|a| a.get("@atom").cloned())
                .map(|v| match v {
                    Any::Map(m) => format!("{:?}", m.get("@for")),
                    other => format!("{other:?}"),
                });
            (s.to_string(), atom_for)
        })
        .collect()
}

/// `<p>a<br>b</p><p>cd</p>`: the break is 2..3.
fn a_break_b(s: &Schema) -> Vec<Node> {
    let br = s
        .create_node("hard_break", Attrs::new(), Fragment::empty())
        .unwrap();
    let p = s
        .branch(
            "paragraph",
            Fragment::from_children(vec![s.text("a").unwrap(), br, s.text("b").unwrap()]),
        )
        .unwrap();
    vec![p, para(s, "cd")]
}

fn breaks(s: &Schema, n: usize) -> Fragment {
    Fragment::from_children(
        (0..n)
            .map(|_| {
                s.create_node("hard_break", Attrs::new(), Fragment::empty())
                    .unwrap()
            })
            .collect(),
    )
}

/// #1851: three breaks inserted in one edit right after a break (a paste) each get a
/// value of their own, so no two share formatting markers. Every inserted char
/// inherits the break before it, and each is written named for itself: four runs, four
/// different `@for`s.
#[test]
fn three_equal_atoms_inserted_after_an_equal_one_are_four_runs() {
    let (mut a, _) = two_peers(a_break_b);
    let s = a.schema.clone();
    a.local_replace(3, breaks(&s, 3));
    assert_eq!(
        doc_text(&a.state.doc),
        "a[hard_break][hard_break][hard_break][hard_break]b|cd"
    );
    let runs = raw_atom_runs(&a, 0);
    let atoms: Vec<_> = runs.iter().filter(|r| r.1.is_some()).collect();
    assert_eq!(atoms.len(), 4, "one run per break: {runs:?}");
    for (s, f) in &atoms {
        assert_eq!(s.chars().count(), 1, "{runs:?}");
        assert!(
            f.as_deref().is_some_and(|f| f.starts_with("Some")),
            "{runs:?}"
        );
    }
}

/// #1859 review: the placement of a split's or join's new block scores each changed
/// pair at most twice, so a whole-document replace is linear in its blocks (scoring
/// every pair per candidate gap made 8000 paragraphs take 11.5 s).
#[test]
fn placing_the_blocks_of_a_whole_document_replace_scores_each_pair_twice() {
    for n in [100usize, 400] {
        let s = schema();
        let before: Vec<Node> = (0..n).map(|i| para(&s, &format!("old {i}"))).collect();
        let after: Vec<Node> = (0..=n).map(|i| para(&s, &format!("new {i}"))).collect();
        let mut a = Peer::host(&s, before, 1);
        let next = doc_of(&s, after);
        let start = rinch_editor_collab::testing::gap_scores();
        a.session
            .record_local(&a.schema, &a.state.doc, &next)
            .unwrap();
        let scored = rinch_editor_collab::testing::gap_scores() - start;
        assert_eq!(scored, 2 * n as u64, "{n} blocks replaced by {}", n + 1);
    }
}

/// The cost of #1850's placement, pinned so a change to it is a decision: A types at
/// the end of a line while B deletes that line's text and the boundary after it (a
/// selection from the line's start to the next line's start), which joins what is left
/// of it, nothing, into the next line. B's join keeps the next line's block and deletes
/// the emptied one, so A's character goes with it. Before #1850 the emptied line's
/// block was kept and the next line's text written into it, which kept A's character
/// (`xcd`), and wrote `cd` anew, the copy a peer's concurrent edit of it missed. User
/// guide: `contenteditable.md`.
#[test]
fn typing_into_a_line_a_peer_empties_and_joins_the_next_line_into_is_lost() {
    let (mut a, mut b) = two_peers(|s| vec![para(s, "ab"), para(s, "q"), para(s, "cd")]);
    // `<p>ab</p><p>q</p><p>cd</p>`: `q` is 5..6, `cd` starts at 8.
    assert!(apply(&mut a, &Op::Type(0, 6, 'x')));
    assert!(apply(&mut b, &Op::DeleteSel(1, 5, 8)));
    exchange(&mut a, &mut b);
    assert_eq!(doc_text(&a.state.doc), "ab|cd");
}

// --- #1856 / #1249: undo after remote changes ------------------------------------------

/// Replay a shrunk history and return the converged document of peer 0, failing on a
/// panic (an undo leaving `model ≢ project(model)`) or a defect.
fn replay_converged(peers: usize, ops: &[Op]) -> Node {
    let mut run = Run::new(peers);
    for op in ops {
        run.step(op);
    }
    run.finish();
    assert_eq!(run.verdict(), None);
    run.peers[0].state.doc.clone()
}

fn has_mark(n: &Node) -> bool {
    !n.marks().is_empty() || (0..n.child_count()).any(|i| has_mark(n.child(i)))
}

/// #1856, seed 322 shrunk: B bolds `ぃ一` after A's first delete and before A's
/// second, then undoes. The undo used to remove the bold at its old positions, one
/// char to the right (`一い`), which unbolded `い` (never bold) into a text node of its
/// own next to `一`: `model ≢ project(model)`. Mapped through A's delete, it unbolds
/// exactly `ぃ一`.
#[test]
fn undo_of_a_bold_after_a_peers_delete_before_it_1856_seed_322() {
    let doc = replay_converged(
        2,
        &[
            Op::Type(1, 6, '一'),
            Op::Delete(0, 2),
            Op::Send(0),
            Op::Deliver(1),
            Op::Delete(0, 1),
            Op::Send(0),
            Op::Bold(1, 4, 6),
            Op::Deliver(1),
            Op::Undo(1),
        ],
    );
    assert_eq!(doc_text(&doc), "|ぃ一い");
    assert!(!has_mark(&doc), "the undo took the bold off: {doc:?}");
}

/// #1856, seed 934 shrunk: A deletes across a block boundary and bolds, B inserts a
/// hard break before A's bold, A undoes the bold. It used to unbold at the old
/// positions and leave `丅`, `ぃ` as two text nodes.
#[test]
fn undo_of_a_bold_after_a_peers_break_before_it_1856_seed_934() {
    let doc = replay_converged(
        2,
        &[
            Op::Type(1, 5, '丅'),
            Op::Send(1),
            Op::Image(0, 1),
            Op::Type(0, 1, '丆'),
            Op::Send(0),
            Op::Deliver(1),
            Op::Deliver(0),
            Op::DeleteSel(0, 7, 2),
            Op::Bold(0, 3, 5),
            Op::Break(1, 7),
            Op::Send(1),
            Op::Deliver(0),
            Op::Undo(0),
        ],
    );
    assert!(!has_mark(&doc), "the undo took the bold off: {doc:?}");
}

/// #1856, seed 902 shrunk: the undo used to leave a bold char inside a code block.
#[test]
fn undo_after_a_peers_bold_and_retype_leaves_no_mark_in_a_code_block_1856_seed_902() {
    replay_converged(
        2,
        &[
            Op::Code(0, 2, true),
            Op::Type(0, 1, '一'),
            Op::Type(1, 2, '丂'),
            Op::Type(0, 1, '七'),
            Op::Send(0),
            Op::Type(1, 3, '丄'),
            Op::Deliver(1),
            Op::Image(1, 10),
            Op::Type(1, 9, '丅'),
            Op::Code(1, 7, false),
            Op::Send(1),
            Op::Deliver(0),
            Op::Bold(0, 13, 6),
            Op::Send(0),
            Op::Deliver(1),
            Op::Undo(1),
        ],
    );
}
