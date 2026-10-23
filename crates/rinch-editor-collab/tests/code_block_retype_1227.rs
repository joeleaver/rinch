//! Issue #1227: a textblock retyped into one that allows less, while a peer formats it.
//!
//! `reconcile_node` retypes a textblock in place (its `type` key rewritten, its `Text`
//! kept), so a peer's concurrent typing survives a paragraph → code block change. But
//! formatting and inline atoms merge into that `Text` independently of the `type` key:
//! peer A runs `setCodeBlock` while peer B bolds a character and inserts a hard break,
//! and both used to converge on `<pre>a<strong>b</strong>c<br></pre>` — content the
//! starter kit's `code_block` (`text*`, no marks) forbids, which no local edit can make.
//!
//! The read normalises (ProseMirror's `setBlockType` / `clearIncompatible` rule): a mark
//! the block type does not allow is dropped, a `hard_break` in a code block is the line
//! break `"\n"`, and any other inline node the block does not accept is dropped. Every
//! replica reads the same CRDT, so the normalisation converges by construction; the
//! CRDT keeps the stray formatting until a local edit of that block writes the block
//! back as the model holds it.

use std::rc::Rc;

use rinch_editor_collab::testing::{session_from_bytes_with_client_id, session_with_client_id};
use rinch_editor_collab::{CollabPlugin, CollabSession};
use rinch_editor_core::{
    AttrValue, Attrs, EditorState, Fragment, Node, Plugin, Pos, Schema, Selection, Slice,
    Transaction, default_plugins,
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
    s.branch("paragraph", Fragment::from_node(s.text(text).unwrap()))
        .unwrap()
}

fn doc_of(s: &Schema, blocks: Vec<Node>) -> Node {
    s.branch("doc", Fragment::from_children(blocks)).unwrap()
}

/// Every node's children satisfy its content expression **and** every mark is one its
/// parent allows: the document is one a model can hold.
fn assert_valid(schema: &Schema, node: &Node) {
    if node.is_text() {
        return;
    }
    let names: Vec<&str> = (0..node.child_count())
        .map(|i| node.child(i).type_name())
        .collect();
    assert!(
        node.node_type().content_match().matches(&names),
        "<{}> holds {names:?}, which its schema does not allow",
        node.type_name()
    );
    for i in 0..node.child_count() {
        let child = node.child(i);
        for m in child.marks() {
            assert!(
                schema.allows_mark(node.type_name(), m.type_name()),
                "<{}> holds a `{}` mark, which its schema does not allow",
                node.type_name(),
                m.type_name()
            );
        }
        assert_valid(schema, child);
    }
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
        self.assert_model_is_projection("after a local edit");
    }

    fn local(&mut self, f: impl FnOnce(&mut Transaction)) {
        let mut tr = self.state.tr();
        f(&mut tr);
        let next = self.state.apply(tr);
        self.commit(next);
    }

    fn run_at(&mut self, sel: Selection, name: &str) {
        let mut tr = self.state.tr();
        tr.set_selection(sel);
        let placed = self.state.apply(tr);
        let next = placed
            .run(name)
            .unwrap_or_else(|| panic!("`{name}` applies"));
        self.commit(next);
    }

    fn run(&mut self, pos: usize, name: &str) {
        self.run_at(Selection::cursor(Pos(pos)), name);
    }

    fn insert_node(&mut self, pos: usize, node: Node) {
        self.local(|tr| {
            tr.replace(pos, pos, Slice::new(Fragment::from_node(node), 0, 0))
                .unwrap();
        });
    }

    fn type_at(&mut self, pos: usize, text: &str) {
        self.local(|tr| {
            tr.set_selection(Selection::cursor(Pos(pos)));
            tr.insert_text(text).unwrap();
        });
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
        self.assert_model_is_projection("after a remote change");
    }

    fn assert_model_is_projection(&self, what: &str) {
        let projected = self.session.projected_doc(&self.schema).unwrap();
        assert_eq!(self.state.doc, projected, "{what}: model ≡ project(model)");
    }
}

/// A textblock's text, an inline atom as `[name]`.
fn block_text(block: &Node) -> String {
    (0..block.child_count())
        .map(|i| {
            let c = block.child(i);
            c.text()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("[{}]", c.type_name()))
        })
        .collect()
}

fn html(doc: &Node) -> String {
    rinch_editor_core::serialize::node_to_html(doc)
}

/// B's edit: bold `b`, then a hard break after `c` (positions in `<p>abc</p>`).
fn bold_b_and_break_after_c(b: &mut Peer) {
    b.run_at(Selection::text(Pos(2), Pos(3)), "toggleBold");
    b.run(4, "insertHardBreak");
}

/// The issue's scenario, merged in both client-id orders and both delivery orders.
/// Returns the converged peers of each run.
fn scenario(b_edit: impl Fn(&mut Peer)) -> Vec<(Peer, Peer)> {
    let mut out = Vec::new();
    for ids in [(11u64, 22u64), (22, 11)] {
        for a_first in [true, false] {
            let s = schema();
            let mut a = Peer::host(&s, vec![para(&s, "abc")], ids.0);
            let mut b = a.join(ids.1);
            let _ = a.send();
            a.run(1, "setCodeBlock");
            b_edit(&mut b);
            let (da, db) = (a.send(), b.send());
            if a_first {
                a.receive(&db);
                b.receive(&da);
            } else {
                b.receive(&da);
                a.receive(&db);
            }
            assert_eq!(
                a.state.doc, b.state.doc,
                "the peers must converge (ids {ids:?}, a_first {a_first})"
            );
            assert_valid(&s, &a.state.doc);
            out.push((a, b));
        }
    }
    out
}

#[test]
fn a_code_block_retyped_while_a_peer_bolds_and_breaks_it_reads_valid() {
    for (a, _) in scenario(bold_b_and_break_after_c) {
        assert_eq!(
            html(&a.state.doc),
            "<pre>abc\n</pre>",
            "the bold is dropped and the hard break is a line break"
        );
    }
}

#[test]
fn a_local_edit_after_the_repair_writes_the_block_back_clean() {
    for (mut a, mut b) in scenario(bold_b_and_break_after_c) {
        // A types at the end of the code block: the block is written back as the
        // model holds it, so the CRDT's stray bold and break are gone for good.
        a.type_at(5, "d");
        let da = a.send();
        b.receive(&da);
        assert_eq!(a.state.doc, b.state.doc);
        assert_eq!(html(&a.state.doc), "<pre>abc\nd</pre>");
        // A send with nothing new must stay empty-handed: the read and the CRDT agree,
        // so nothing fights over the block.
        let again = a.send();
        b.receive(&again);
        assert_eq!(a.state.doc, b.state.doc);

        // Now B turns the block back into a paragraph. The bold B made concurrently
        // with the retype is not resurrected: the model never held it after the
        // repair, and the reconcile wrote the block from the model.
        b.run(1, "setParagraph");
        let db = b.send();
        a.receive(&db);
        assert_eq!(a.state.doc, b.state.doc);
        assert_valid(&a.schema, &a.state.doc);
        assert!(
            !html(&a.state.doc).contains("<strong>"),
            "a dropped mark must not come back: {}",
            html(&a.state.doc)
        );
    }
}

#[test]
fn retyping_the_untouched_block_back_does_not_resurrect_the_peers_bold() {
    for (mut a, mut b) in scenario(bold_b_and_break_after_c) {
        // No edit of the block in between: the CRDT still holds the stray formatting,
        // so this pins that the retype's own reconcile clears it.
        a.run(1, "setParagraph");
        let da = a.send();
        b.receive(&da);
        assert_eq!(a.state.doc, b.state.doc);
        assert_valid(&a.schema, &a.state.doc);
        assert!(
            !html(&a.state.doc).contains("<strong>"),
            "a dropped mark must not come back: {}",
            html(&a.state.doc)
        );
    }
}

#[test]
fn an_image_inserted_into_a_block_being_made_code_is_dropped() {
    let image = |b: &mut Peer| {
        let img = b
            .schema
            .create_node(
                "image",
                Attrs::new()
                    .with("src", AttrValue::from("a.png"))
                    .with("alt", AttrValue::from("x")),
                Fragment::empty(),
            )
            .unwrap();
        b.insert_node(3, img);
        b.run_at(Selection::text(Pos(1), Pos(2)), "toggleItalic");
    };
    for (mut a, mut b) in scenario(image) {
        assert_eq!(html(&a.state.doc), "<pre>abc</pre>");
        // Typing after the dropped image lands where the model says, on both peers.
        a.type_at(4, "X");
        let da = a.send();
        b.receive(&da);
        assert_eq!(a.state.doc, b.state.doc);
        assert_eq!(html(&a.state.doc), "<pre>abcX</pre>");
    }
}

// --- edits in a normalised block, made concurrently (review of #1846) -----------------
//
// The CRDT still holds `abc` + a hard-break atom where the model reads `abc\n`. The
// write-back must diff the block as it reads, not as it is stored, or each peer's edit
// rewrites the block from its edit point to the end and the merge holds both rewrites.

/// From the converged scenario, A types `ta` at `pa` and B types `tb` at `pb`, each
/// before receiving the other's; then both exchange. Returns the code block's text.
fn type_concurrently(pa: usize, ta: &str, pb: usize, tb: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (mut a, mut b) in scenario(bold_b_and_break_after_c) {
        a.type_at(pa, ta);
        b.type_at(pb, tb);
        let (da, db) = (a.send(), b.send());
        a.receive(&db);
        b.receive(&da);
        assert_eq!(a.state.doc, b.state.doc, "the peers must converge");
        assert_valid(&a.schema, &a.state.doc);
        out.push(block_text(a.state.doc.child(0)));
    }
    out
}

#[test]
fn concurrent_typing_at_the_start_and_middle_of_a_normalised_block_lands_once() {
    for text in type_concurrently(1, "X", 3, "Y") {
        assert_eq!(text, "XabYc\n");
    }
}

#[test]
fn concurrent_typing_at_one_place_in_a_normalised_block_keeps_the_block_once() {
    for text in type_concurrently(1, "X", 1, "Z") {
        assert!(
            text == "XZabc\n" || text == "ZXabc\n",
            "each typed char once and the block once: {text:?}"
        );
    }
}

#[test]
fn concurrent_typing_at_the_end_of_a_normalised_block_keeps_one_line_break() {
    for text in type_concurrently(5, "X", 5, "Y") {
        assert!(
            text == "abc\nXY" || text == "abc\nYX",
            "one line break, each typed char once: {text:?}"
        );
    }
}

#[test]
fn a_sticky_index_in_a_normalised_block_resolves_before_any_write_back() {
    for (a, b) in scenario(bold_b_and_break_after_c) {
        // Positions 1..=5 in `<pre>abc\n</pre>`: before `a` … after the line break.
        for pos in 1..=5 {
            let bytes = a
                .session
                .sticky_index(&a.state.doc, Pos(pos))
                .unwrap_or_else(|| panic!("a sticky index at {pos}"));
            assert_eq!(
                b.session.resolve_sticky(&b.state.doc, &bytes),
                Some(Pos(pos)),
                "the index made at {pos} resolves there on the other peer"
            );
        }
    }
}

#[test]
fn a_sticky_index_after_a_dropped_image_resolves_where_the_model_has_it() {
    let image = |b: &mut Peer| {
        let img = b
            .schema
            .create_node(
                "image",
                Attrs::new().with("src", AttrValue::from("a.png")),
                Fragment::empty(),
            )
            .unwrap();
        b.insert_node(2, img);
    };
    for (a, b) in scenario(image) {
        assert_eq!(html(&a.state.doc), "<pre>abc</pre>");
        for pos in 1..=4 {
            let bytes = a
                .session
                .sticky_index(&a.state.doc, Pos(pos))
                .unwrap_or_else(|| panic!("a sticky index at {pos}"));
            assert_eq!(
                b.session.resolve_sticky(&b.state.doc, &bytes),
                Some(Pos(pos)),
                "the index made at {pos} resolves there on the other peer"
            );
        }
    }
}

// --- randomized: typed text survives exactly once -------------------------------------

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

/// A random position whose parent is a textblock.
fn text_pos(rng: &mut Rng, doc: &Node) -> Option<usize> {
    let p = rng.below(doc.content_size() + 1);
    doc.resolve(Pos(p))
        .ok()
        .filter(|r| r.parent().is_textblock())
        .map(|_| p)
}

/// One random edit by `peer`: type a char no one else ever types, retype a block to
/// code or back, bold a range, insert a hard break or an image. Never deletes text.
fn random_edit(rng: &mut Rng, peer: &mut Peer, unique: &mut u32, typed: &mut Vec<char>) {
    let st = &peer.state;
    let Some(p) = text_pos(rng, &st.doc) else {
        return;
    };
    let next = match rng.below(7) {
        0..=2 => {
            let c = char::from_u32(0x4E00 + *unique).unwrap();
            *unique += 1;
            let mut tr = st.tr();
            tr.set_selection(Selection::cursor(Pos(p)));
            if tr.insert_text(&c.to_string()).is_err() {
                return;
            }
            typed.push(c);
            Some(st.apply(tr))
        }
        3 => {
            let cmd = ["setCodeBlock", "setParagraph"][rng.below(2)];
            let mut tr = st.tr();
            tr.set_selection(Selection::cursor(Pos(p)));
            st.apply(tr).run(cmd)
        }
        4 => {
            let Some(q) = text_pos(rng, &st.doc) else {
                return;
            };
            let mut tr = st.tr();
            tr.set_selection(Selection::text(Pos(p.min(q)), Pos(p.max(q))));
            st.apply(tr).run("toggleBold")
        }
        5 => {
            let mut tr = st.tr();
            tr.set_selection(Selection::cursor(Pos(p)));
            st.apply(tr).run("insertHardBreak")
        }
        _ => {
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
    };
    if let Some(next) = next {
        peer.commit(next);
    }
}

/// Every char a peer typed is in the converged document exactly once — not merely
/// converged: a write-back that rewrites a block from the edit point on converges on
/// both rewrites, the block's text twice.
#[test]
fn randomized_typed_text_lands_exactly_once_around_retypes_and_formatting() {
    let mut code_in_play = 0;
    for seed in 1..=60u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let s = schema();
        let peers = 2 + (seed % 2) as usize;
        let mut ps = vec![Peer::host(&s, vec![para(&s, "ab"), para(&s, "cd")], 1)];
        for i in 1..peers {
            let joined = ps[0].join(1 + i as u64);
            ps.push(joined);
        }
        let _ = ps[0].send();
        let (mut queue, mut seen) = (Vec::<Vec<u8>>::new(), vec![0usize; peers]);
        let (mut unique, mut typed) = (0u32, Vec::new());
        for _ in 0..80 {
            if rng.below(10) < 6 {
                let p = rng.below(peers);
                random_edit(&mut rng, &mut ps[p], &mut unique, &mut typed);
                let d = ps[p].send();
                if !d.is_empty() {
                    queue.push(d);
                }
            } else {
                let q = rng.below(peers);
                while seen[q] < queue.len() {
                    ps[q].receive(&queue[seen[q]].clone());
                    seen[q] += 1;
                }
            }
            if ps.iter().any(|p| html(&p.state.doc).contains("<pre>")) {
                code_in_play += 1;
            }
        }
        for q in 0..peers {
            while seen[q] < queue.len() {
                ps[q].receive(&queue[seen[q]].clone());
                seen[q] += 1;
            }
        }
        for q in 1..peers {
            assert_eq!(ps[q].state.doc, ps[0].state.doc, "seed {seed}: converged");
        }
        let doc = &ps[0].state.doc;
        assert_valid(&s, doc);
        let text: String = (0..doc.child_count())
            .map(|i| block_text(doc.child(i)))
            .collect();
        for c in &typed {
            assert_eq!(
                text.matches(*c).count(),
                1,
                "seed {seed}: typed {c:?} must land exactly once in {text:?}"
            );
        }
    }
    assert!(
        code_in_play > 100,
        "the trials must put code blocks in play ({code_in_play})"
    );
}
