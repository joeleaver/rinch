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
    atom_id_walks, node_reads, session_from_bytes_with_client_id, session_with_client_id,
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

/// Entries of the `atoms` map and block texts read by a keystroke in a line holding a
/// picture, by projecting it (`local`) and by a peer integrating it (`remote`, twice:
/// the first and a second keystroke).
fn keystroke_reads(n: usize) -> ((u64, u64), u64, u64, u64) {
    use rinch_editor_collab::testing::{overlay_entry_reads, text_scans};
    let (mut host, mut guest) = edited_pictures(n);
    use rinch_editor_core::{AttrValue, SetNodeAttrStep};
    let mut remote = Vec::new();
    let mut local = (0, 0);
    for k in 0..2 {
        if k == 0 {
            // A change of the map first, so the guest's read of it is stale.
            let mut tr = host.state.tr();
            tr.step(Box::new(SetNodeAttrStep::new(
                2,
                "alt",
                AttrValue::from("x"),
            )))
            .unwrap();
            let next = host.state.apply(tr);
            host.session
                .record_local(&host.schema, &host.state.doc, &next.doc)
                .unwrap();
            host.state = next;
        }
        let (e, t) = (overlay_entry_reads(), text_scans());
        host.type_at(2 + k, "y");
        local = (overlay_entry_reads() - e, text_scans() - t);
        let delta = host.session.save_incremental().unwrap();
        let before = overlay_entry_reads();
        let next = guest
            .session
            .integrate_incremental(&guest.state, &delta)
            .unwrap()
            .expect("the keystroke changes the guest");
        remote.push(overlay_entry_reads() - before);
        guest.state = next;
    }
    guest.assert_model_is_projection();
    assert_eq!(guest.state.doc, host.state.doc);
    let entries = host.session.snapshot().len() as u64;
    (local, remote[0], remote[1], entries)
}

/// A keystroke reads the `atoms` map at most once whichever side projects it — as many
/// entries as the map holds, not that many for every line holding a picture (it was:
/// a remote keystroke took 1.23 s at 1000 edited pictures in a debug build) — and not
/// at all while the map has not changed since the last read. Projecting it locally
/// scans only the line it changed (it scanned every block once: 1.4 ms a key at 1000
/// pictures).
#[test]
fn a_keystroke_reads_the_atom_entries_once() {
    let ((entries_50, scans_50), first_50, second_50, _) = keystroke_reads(50);
    let ((entries_100, scans_100), first_100, second_100, _) = keystroke_reads(100);
    // The host read the map when it last changed it.
    assert_eq!((entries_50, entries_100), (0, 0));
    assert_eq!(
        scans_50, scans_100,
        "a keystroke's scans do not grow with the document"
    );
    assert!(scans_50 <= 8, "{scans_50}");
    // The guest reads the changed map once, then not again until it changes.
    assert_eq!((first_50, first_100), (50, 100));
    assert_eq!((second_50, second_100), (0, 0));
}

// --- a remote keystroke beside an embed (review of #1575, F5) ---

/// CRDT nodes read by integrating each of a peer's keystrokes, both ways, in live
/// collaboration (each side types between the other's), with `n` paragraphs and —
/// when `embed` — an attributed embed in the middle.
fn remote_keystroke_reads(n: usize, embed: bool) -> Vec<u64> {
    use rinch_editor_core::{AttrValue, Attrs};
    let s = Rc::new(Schema::starter_kit());
    let mut blocks = paragraphs(&s, n);
    if embed {
        let e = s
            .create_node(
                "embed",
                Attrs::from_iter([("data-sheet", AttrValue::from("s1"))]),
                Fragment::empty(),
            )
            .unwrap();
        blocks.insert(n / 2, e);
    }
    let a = Peer::host(&s, blocks, 1);
    let b = a.join(2);
    let mut peers = [a, b];
    let mut reads = Vec::new();
    for round in 0..3 {
        for f in [0, 1] {
            let [p0, p1] = &mut peers;
            let (from, to) = if f == 0 { (p0, p1) } else { (p1, p0) };
            let last = from.state.doc.child_count() - 1;
            let at = start_of_block(&from.state.doc, last) + 2 + round;
            from.type_at(at, "x");
            let delta = from.session.save_incremental().unwrap();
            let before = node_reads();
            let next = to
                .session
                .integrate_incremental(&to.state, &delta)
                .unwrap()
                .expect("the keystroke changes the peer");
            reads.push(node_reads() - before);
            to.state = next;
        }
    }
    let [a, b] = peers;
    a.assert_model_is_projection();
    assert_eq!(a.state.doc, b.state.doc);
    reads
}

/// A peer's keystroke reads the CRDT once (its converged rebuild) whether or not
/// the document holds an embed: the embed costs what one more paragraph costs and nothing more. It
/// used to cost a second whole read — the old document's atoms' blocks, read again
/// before every update to tell an embed whose attributes a peer changed from
/// another — about 50% more at any size (6003 nodes against 4000 at 2000
/// paragraphs). Those blocks are now kept from the previous operation
/// (`atom_cache`), on the host from the session's start and on a guest from the
/// read it joined with.
#[test]
fn a_remote_keystroke_beside_an_embed_reads_the_crdt_once() {
    for n in [50, 100] {
        let without = remote_keystroke_reads(n, false);
        let with = remote_keystroke_reads(n, true);
        // What one more block costs: a paragraph's share of the read.
        let per_block = without[0] / n as u64;
        let plus_one: Vec<u64> = without.iter().map(|r| r + per_block).collect();
        assert_eq!(with, plus_one, "n = {n}");
    }
}

/// What a **local** keystroke pays for the node views' atom-id cache in a live
/// session, i.e. after the session has integrated a peer's change. In a document
/// with no leaf block atom it walks nothing: it used to walk every top-level block
/// a second time and rewrite the cache entry the integrate left, about 19% of a
/// keystroke's `record_local` at 2000 paragraphs, a cost the `collab_keystroke`
/// bench (which never integrates) could not see (review 2 of #1575, N3). With an
/// embed it walks the blocks once, which is what spares the next remote change a
/// second read of the CRDT (`a_remote_keystroke_beside_an_embed_reads_the_crdt_once`).
#[test]
fn a_local_keystroke_after_a_remote_change_walks_no_block_without_an_embed() {
    use rinch_editor_core::{AttrValue, Attrs};
    for embed in [false, true] {
        let s = Rc::new(Schema::starter_kit());
        let mut blocks = paragraphs(&s, BLOCKS);
        if embed {
            let e = s
                .create_node(
                    "embed",
                    Attrs::from_iter([("data-sheet", AttrValue::from("s1"))]),
                    Fragment::empty(),
                )
                .unwrap();
            blocks.insert(BLOCKS / 2, e);
        }
        let mut a = Peer::host(&s, blocks, 1);
        let mut b = a.join(2);
        let mut walks = Vec::new();
        for round in 0..3 {
            let last = b.state.doc.child_count() - 1;
            let at = start_of_block(&b.state.doc, last) + 2;
            b.type_at(at, "b");
            a.exchange(&mut b);
            let at = start_of_block(&a.state.doc, round) + 2;
            let before = atom_id_walks();
            a.type_at(at, "a");
            walks.push(atom_id_walks() - before);
        }
        let want = if embed { [1, 1, 1] } else { [0, 0, 0] };
        assert_eq!(walks, want, "embed: {embed}");
        a.exchange(&mut b);
        a.assert_model_is_projection();
        assert_eq!(a.state.doc, b.state.doc);
    }
}

// --- a keystroke in one cell of a table (#1241) ---

/// A `rows × 4` table whose cells read `row {r} column {c}`.
fn table(s: &Schema, rows: usize) -> Node {
    let branch = |name: &str, children: Vec<Node>| {
        s.branch(name, Fragment::from_children(children)).unwrap()
    };
    branch(
        "table",
        (0..rows)
            .map(|r| {
                branch(
                    "table_row",
                    (0..4)
                        .map(|c| {
                            branch("table_cell", vec![para(s, &format!("row {r} column {c}"))])
                        })
                        .collect(),
                )
            })
            .collect(),
    )
}

/// The model position of the first char of `needle`, searching every text node.
fn pos_of(doc: &Node, needle: &str) -> usize {
    fn walk(node: &Node, content_start: usize, needle: &str) -> Option<usize> {
        let mut at = content_start;
        for i in 0..node.child_count() {
            let child = node.child(i);
            if let Some(text) = child.text() {
                if let Some(byte) = text.find(needle) {
                    return Some(at + text[..byte].chars().count());
                }
            } else if let Some(found) = walk(child, at + 1, needle) {
                return Some(found);
            }
            at += child.node_size();
        }
        None
    }
    walk(doc, 0, needle).unwrap_or_else(|| panic!("`{needle}` is not in the document"))
}

/// CRDT nodes read by each of three keystrokes in one cell of a `rows`-row table,
/// after a first keystroke in that cell.
fn table_keystroke_reads(rows: usize) -> Vec<u64> {
    let s = Rc::new(Schema::starter_kit());
    let blocks = vec![para(&s, "before"), table(&s, rows), para(&s, "after")];
    let mut a = Peer::host(&s, blocks, 1);
    let at = pos_of(&a.state.doc, &format!("row {} column 2", rows / 2)) + 3;
    a.type_at(at, "x");
    let reads = (1..=3).map(|n| a.type_at(at + n, "x")).collect();
    a.assert_model_is_projection();
    reads
}

/// A keystroke in a table cell reads that cell's block, whatever the table's size: it
/// used to read back (and compare) every cell of the table, twice — 26 ms per
/// keystroke at 1000 × 4 (#1241). The first keystroke in a table after any other
/// change still reads the table once, to find its cells in the CRDT.
#[test]
fn a_keystroke_in_a_table_cell_reads_only_its_block() {
    let small = table_keystroke_reads(10);
    let big = table_keystroke_reads(200);
    assert_eq!(small, vec![1, 1, 1], "a keystroke in a 10-row table");
    assert_eq!(big, small, "a keystroke in a 200-row table");
}

// --- live collaboration in a table: a peer's keystroke, then mine (#1241) ---

/// What each round of live collaboration in one table of `rows` × 4 cells costs, after
/// a warm-up round: the guest types in one cell of the middle row and the host
/// integrates it (CRDT nodes read), then the host types in another cell of that row
/// (CRDT nodes read, and how many times a whole table was read to find its cells).
fn live_table_rounds(rows: usize) -> Vec<(u64, u64, u64)> {
    use rinch_editor_collab::testing::table_cell_edits;
    let s = Rc::new(Schema::starter_kit());
    let blocks = vec![para(&s, "before"), table(&s, rows), para(&s, "after")];
    let mut a = Peer::host(&s, blocks, 1);
    let mut b = a.join(2);
    let mine = format!("row {} column 1", rows / 2);
    let theirs = format!("row {} column 2", rows / 2);
    let mut out = Vec::new();
    for round in 0..4 {
        let at = pos_of(&b.state.doc, &theirs) + theirs.len();
        b.type_at(at, "b");
        let delta = b.session.save_incremental().unwrap();
        let before = node_reads();
        a.state = a
            .session
            .integrate_incremental(&a.state, &delta)
            .unwrap()
            .expect("the peer's keystroke changes the host");
        let integrate = node_reads() - before;
        let at = pos_of(&a.state.doc, &mine) + mine.len();
        let (builds, _) = table_cell_edits();
        let local = a.type_at(at, "a");
        let builds = table_cell_edits().0 - builds;
        a.exchange(&mut b);
        if round > 0 {
            out.push((integrate, local, builds));
        }
    }
    a.assert_model_is_projection();
    b.assert_model_is_projection();
    assert_eq!(a.state.doc, b.state.doc);
    out
}

/// In live collaboration a keystroke in a table cell costs what the cell holds, on
/// both sides, whatever the table's size: integrating a peer's keystroke reads the
/// block it changed (it read the whole document back: 77M instructions at 1000 rows,
/// `editor::collab_table_integrate`), and my next keystroke finds its cell without
/// reading the table again (any remote change made it re-read the whole table:
/// `editor::collab_table_keystroke_after_remote`).
#[test]
fn live_collaboration_in_a_table_costs_the_same_at_any_size() {
    let small = live_table_rounds(10);
    let big = live_table_rounds(1000);
    assert_eq!(big, small, "a 1000-row table against a 10-row one");
    for (integrate, local, builds) in small {
        assert!(integrate <= 3, "integrating a peer's keystroke read {integrate}");
        assert_eq!((local, builds), (1, 0), "my keystroke after it");
    }
}
