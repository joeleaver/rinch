//! What a local keystroke **reads** from the CRDT (review of #1229, F1).
//!
//! `project_change` diffs the model before and after an edit and skips every block the
//! edit did not touch, so typing one character reads back the one block it changed.
//! #1229 first read back the whole top-level document on every edit — to find the void
//! containers the diff must step over (`projection::is_void`) — through index reads that
//! are O(i) each: a keystroke in a 2,000-paragraph document went from 17 µs to 13.4 ms.
//!
//! These pins count CRDT nodes examined (`testing::node_reads`), not time: a node read
//! back whole, or a node's shape checked for a void container, is one. Timing lives in
//! `crates/rinch-bench` (`collab_keystroke`).
//!
//! A void container can only come from a **remote** merge (two peers each deleting one of
//! a quote's two paragraphs). A document that holds none pays what it paid before quotes;
//! one that holds one pays a shape scan of the top level per edit — linear, one node read
//! per block, which the last test pins as the accepted cost.

use std::rc::Rc;

use rinch_editor_collab::testing::{
    node_reads, session_from_bytes_with_client_id, session_with_client_id,
};
use rinch_editor_collab::{CollabPlugin, CollabSession};
use rinch_editor_core::{
    EditorState, Fragment, Node, Plugin, Pos, Schema, Selection, default_plugins,
};

const BLOCKS: usize = 2_000;

fn plugins() -> Vec<Rc<dyn Plugin>> {
    let mut p = default_plugins();
    p.push(Rc::new(CollabPlugin));
    p
}

fn para(s: &Schema, text: &str) -> Node {
    s.branch("paragraph", Fragment::from_node(s.text(text).unwrap()))
        .unwrap()
}

fn paragraphs(s: &Schema, n: usize) -> Vec<Node> {
    (0..n)
        .map(|i| para(s, &format!("paragraph number {i} with some text in it")))
        .collect()
}

fn doc_of(s: &Schema, blocks: Vec<Node>) -> Node {
    s.branch("doc", Fragment::from_children(blocks)).unwrap()
}

/// The model position just inside the start of top-level block `index`'s text.
fn start_of_block(doc: &Node, index: usize) -> usize {
    (0..index).map(|i| doc.child(i).node_size()).sum::<usize>() + 1
}

struct Peer {
    schema: Rc<Schema>,
    state: EditorState,
    session: CollabSession,
}

impl Peer {
    fn host(s: &Rc<Schema>, blocks: Vec<Node>, client_id: u64) -> Peer {
        let state = EditorState::create(s.clone(), doc_of(s, blocks), plugins());
        let session = session_with_client_id(&state, client_id).unwrap();
        Peer {
            schema: s.clone(),
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

    /// Type `text` at `pos` and return how many CRDT nodes projecting it read.
    fn type_at(&mut self, pos: usize, text: &str) -> u64 {
        let mut tr = self.state.tr();
        tr.set_selection(Selection::cursor(Pos(pos)));
        tr.insert_text(text).unwrap();
        let next = self.state.apply(tr);
        let before = node_reads();
        self.session
            .record_local(&self.schema, &self.state.doc, &next.doc)
            .expect("the edit projects");
        let reads = node_reads() - before;
        self.state = next;
        reads
    }

    fn delete(&mut self, from: usize, to: usize) {
        let mut tr = self.state.tr();
        tr.delete(from, to).unwrap();
        let next = self.state.apply(tr);
        self.session
            .record_local(&self.schema, &self.state.doc, &next.doc)
            .unwrap();
        self.state = next;
    }

    fn exchange(&mut self, other: &mut Peer) {
        let (mine, theirs) = (
            self.session.save_incremental().unwrap(),
            other.session.save_incremental().unwrap(),
        );
        for (peer, delta) in [(&mut *self, &theirs), (&mut *other, &mine)] {
            if let Some(next) = peer
                .session
                .integrate_incremental(&peer.state, delta)
                .unwrap()
            {
                peer.state = next;
            }
        }
    }

    fn assert_model_is_projection(&self) {
        assert_eq!(
            self.state.doc,
            self.session.projected_doc(&self.schema).unwrap()
        );
    }
}

#[test]
fn a_keystroke_reads_only_the_block_it_changed() {
    let s = Rc::new(Schema::starter_kit());
    let mut a = Peer::host(&s, paragraphs(&s, BLOCKS), 1);
    // The first, a middle and the last block: none of them reads its neighbours, the
    // last one included (a whole-document read reaches it whichever block is edited).
    for index in [0, BLOCKS / 2, BLOCKS - 1] {
        let at = start_of_block(&a.state.doc, index) + 3;
        assert_eq!(a.type_at(at, "x"), 1, "a keystroke in block {index}");
    }
    a.assert_model_is_projection();
}

#[test]
fn a_keystroke_after_a_remote_edit_reads_only_the_block_it_changed() {
    // Live collaboration: every keystroke follows a peer's delta. Integrating one must not
    // leave the next local edit paying for the whole document.
    let s = Rc::new(Schema::starter_kit());
    let mut a = Peer::host(&s, paragraphs(&s, BLOCKS), 1);
    let mut b = a.join(2);
    for round in 0..3 {
        let at = start_of_block(&b.state.doc, BLOCKS - 1) + 2;
        b.type_at(at, "b");
        a.exchange(&mut b);
        let at = start_of_block(&a.state.doc, round) + 2;
        assert_eq!(a.type_at(at, "a"), 1, "round {round}");
    }
    a.exchange(&mut b);
    assert_eq!(a.state.doc, b.state.doc);
}

#[test]
fn a_void_container_costs_one_shape_read_per_block() {
    // Two peers each delete one of a quote's two paragraphs: the quote is void, read as
    // absent, and stays in the CRDT. Every later edit has to step over it, so it reads
    // each top-level block's shape once — linear, the accepted cost of a document that
    // holds one — plus the block it changed.
    let s = Rc::new(Schema::starter_kit());
    let mut blocks = vec![
        s.branch(
            "blockquote",
            Fragment::from_children(vec![para(&s, "x"), para(&s, "y")]),
        )
        .unwrap(),
    ];
    blocks.extend(paragraphs(&s, BLOCKS));
    let mut a = Peer::host(&s, blocks, 1);
    let mut b = a.join(2);
    // Quote content: <p>x</p> at 1..4, <p>y</p> at 4..7.
    a.delete(1, 4);
    b.delete(4, 7);
    a.exchange(&mut b);
    assert_eq!(
        a.state.doc.child_count(),
        BLOCKS,
        "the quote reads as absent"
    );

    let at = start_of_block(&a.state.doc, 0) + 2;
    // One shape read per top-level block, the void quote's included (its two deleted
    // paragraphs are gone from its child list, so it costs one too), plus the changed
    // block read back.
    assert_eq!(a.type_at(at, "x"), BLOCKS as u64 + 1 + 1);
    a.assert_model_is_projection();
    a.exchange(&mut b);
    assert_eq!(a.state.doc, b.state.doc);
}

// --- per-attribute atom entries are read once per operation (review of #1503, F4) ---

/// `n` lines, each `x` + a picture, every picture's `alt` changed once (one entry each
/// in the shared document's `atoms` map); a guest joined from the host's snapshot.
fn edited_pictures(n: usize) -> (Peer, Peer) {
    use rinch_editor_core::{AttrValue, Attrs, SetNodeAttrStep};
    let s = Rc::new(Schema::starter_kit());
    let blocks = (0..n)
        .map(|i| {
            let img = s
                .create_node(
                    "image",
                    Attrs::new().with("src", AttrValue::from(format!("p{i}.png"))),
                    Fragment::empty(),
                )
                .unwrap();
            s.branch(
                "paragraph",
                Fragment::from_children(vec![s.text("x").unwrap(), img]),
            )
            .unwrap()
        })
        .collect();
    let mut host = Peer::host(&s, blocks, 1);
    for i in 0..n {
        // Each line is 4 positions: open, `x`, the picture, close.
        let mut tr = host.state.tr();
        tr.step(Box::new(SetNodeAttrStep::new(
            4 * i + 2,
            "alt",
            AttrValue::from(format!("alt {i}")),
        )))
        .unwrap();
        let next = host.state.apply(tr);
        host.session
            .record_local(&s, &host.state.doc, &next.doc)
            .unwrap();
        host.state = next;
    }
    let guest = host.join(2);
    guest.assert_model_is_projection();
    (host, guest)
}

/// Entries of the `atoms` map read by a keystroke in a line holding a picture, by
/// projecting it (`local`) and by a peer integrating it (`remote`).
fn entry_reads(n: usize) -> (u64, u64) {
    use rinch_editor_collab::testing::overlay_entry_reads;
    let (mut host, mut guest) = edited_pictures(n);
    let before = overlay_entry_reads();
    host.type_at(2, "y");
    let local = overlay_entry_reads() - before;
    let delta = host.session.save_incremental().unwrap();
    let before = overlay_entry_reads();
    let next = guest
        .session
        .integrate_incremental(&guest.state, &delta)
        .unwrap()
        .expect("the keystroke changes the guest");
    let remote = overlay_entry_reads() - before;
    guest.state = next;
    guest.assert_model_is_projection();
    assert_eq!(guest.state.doc, host.state.doc);
    (local, remote)
}

/// A keystroke reads the `atoms` map once whichever side projects it: as many entries
/// as the map holds, not that many for every line holding a picture. It was every
/// line's (n² for n edited pictures: a remote keystroke took 1.23 s at 1000 in a debug
/// build).
#[test]
fn a_keystroke_reads_the_atom_entries_once() {
    let (local_50, remote_50) = entry_reads(50);
    let (local_100, remote_100) = entry_reads(100);
    // The host has one entry per picture.
    assert_eq!((local_50, local_100), (50, 100));
    assert!(
        remote_50 <= 2 * 50 && remote_100 <= 2 * 100,
        "{remote_50} {remote_100}"
    );
    assert_eq!(remote_100, 2 * remote_50, "linear in the entries");
}
