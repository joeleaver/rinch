//! A one-cell edit of a table projected through the cell alone writes exactly what the
//! whole-table reconcile wrote (#1241).
//!
//! A keystroke in a table cell used to read back the whole table from the CRDT, build
//! the whole table's target and compare the two — O(cells) per keystroke. It is now
//! projected through the cell (`CollabDoc::project_cell_edit`), which finds the cell's
//! map through an index kept between keystrokes and reconciles its blocks the way the
//! whole-table reconcile does. This differential runs the same random history twice
//! with pinned client ids — once with that path, once with it turned off
//! (`testing::set_table_cell_path(false)`) — and asserts that every peer's CRDT is the
//! same **bytes** in both after every operation: the same writes, not only the same
//! document. The history mixes typing, splits, joins, breaks, pictures, bold, undo
//! and redo in cells (a merged one, one holding two paragraphs, one with a link) of
//! two tables, with row and column commands and edits outside the tables (which
//! change the CRDT under the index, so it must be rebuilt) and deliveries between
//! two peers.

use std::rc::Rc;

use rinch_editor_collab::testing::{
    same_crdt, same_writes, session_from_bytes_with_client_id, session_with_client_id,
    set_table_cell_path, table_cell_edits,
};
use rinch_editor_collab::{CollabPlugin, CollabSession};
use rinch_editor_core::{
    AttrValue, Attrs, EditorState, Fragment, Mark, Node, Plugin, Pos, Schema, Selection, Slice,
    default_plugins,
};

fn plugins() -> Vec<Rc<dyn Plugin>> {
    let mut p = default_plugins();
    p.push(Rc::new(CollabPlugin));
    p
}

fn para(s: &Schema, text: &str) -> Node {
    let content = if text.is_empty() {
        Fragment::empty()
    } else {
        Fragment::from_node(s.text(text).unwrap())
    };
    s.branch("paragraph", content).unwrap()
}

fn branch(s: &Schema, name: &str, children: Vec<Node>) -> Node {
    s.branch(name, Fragment::from_children(children)).unwrap()
}

fn cell(s: &Schema, blocks: Vec<Node>) -> Node {
    branch(s, "table_cell", blocks)
}

/// A table whose first row holds a cell spanning two columns, and a cell with two
/// paragraphs. With `link`, one cell holds a link (see [`trial`] for what that costs
/// the comparison).
fn first_table(s: &Schema, link: bool) -> Node {
    let link = link.then(|| {
        Mark::new(
            s.mark_type("link").unwrap().clone(),
            Attrs::new().with("href", AttrValue::from("https://example.test/")),
        )
    });
    let linked = match link {
        Some(link) => s.branch(
            "paragraph",
            Fragment::from_children(vec![
                s.text("see ").unwrap(),
                s.text("the link").unwrap().with_marks(vec![link]),
                s.text(" here").unwrap(),
            ]),
        ),
        None => Ok(para(s, "see the link here")),
    };
    let merged = s
        .create_node(
            "table_cell",
            Attrs::new().with("colspan", AttrValue::Int(2)),
            Fragment::from_node(para(s, "merged across two")),
        )
        .unwrap();
    branch(
        s,
        "table",
        vec![
            branch(
                s,
                "table_row",
                vec![merged, cell(s, vec![para(s, "top right")])],
            ),
            branch(
                s,
                "table_row",
                vec![
                    cell(s, vec![para(s, "one"), para(s, "two")]),
                    cell(s, vec![linked.unwrap()]),
                    cell(s, vec![para(s, "")]),
                ],
            ),
            branch(
                s,
                "table_row",
                (0..3)
                    .map(|c| cell(s, vec![para(s, &format!("bottom {c}"))]))
                    .collect(),
            ),
        ],
    )
}

fn second_table(s: &Schema) -> Node {
    branch(
        s,
        "table",
        (0..2)
            .map(|r| {
                branch(
                    s,
                    "table_row",
                    (0..2)
                        .map(|c| cell(s, vec![para(s, &format!("t2 r{r} c{c}"))]))
                        .collect(),
                )
            })
            .collect(),
    )
}

struct Peer {
    schema: Rc<Schema>,
    state: EditorState,
    session: CollabSession,
    /// Whether this peer projects with `record_local_edit` (an editor) or `record_local`.
    edit: bool,
}

impl Peer {
    fn commit(&mut self, next: EditorState) {
        if self.edit {
            self.session
                .record_local_edit(&self.schema, &self.state.doc, &next.doc, &[])
                .expect("the edit projects");
        } else {
            self.session
                .record_local(&self.schema, &self.state.doc, &next.doc)
                .expect("the edit projects");
        }
        // Not asserted here that the model is the projection's read: an undo after a
        // peer's change can leave the editor two adjacent unmarked text nodes the read
        // joins (#1856), in both worlds alike. The trial compares the two worlds'
        // reads instead.
        self.state = next;
    }

    fn receive(&mut self, delta: &[u8]) {
        if let Some(next) = self
            .session
            .integrate_incremental(&self.state, delta)
            .unwrap()
        {
            self.state = next;
        }
    }
}

/// Two peers over one document, client ids pinned.
fn world(s: &Rc<Schema>, seed: u64) -> Vec<Peer> {
    let doc = branch(
        s,
        "doc",
        vec![
            para(s, "before the tables"),
            first_table(s, linked(seed)),
            para(s, "between"),
            second_table(s),
            para(s, "after"),
        ],
    );
    let state = EditorState::create(s.clone(), doc, plugins());
    let host = session_with_client_id(&state, (seed << 8) | 1).unwrap();
    let guest = session_from_bytes_with_client_id(&host.snapshot(), (seed << 8) | 2).unwrap();
    let edit = seed.is_multiple_of(2);
    let guest_doc = guest.projected_doc(s).unwrap();
    vec![
        Peer {
            schema: s.clone(),
            state,
            session: host,
            edit,
        },
        Peer {
            schema: s.clone(),
            state: EditorState::create(s.clone(), guest_doc, plugins()),
            session: guest,
            edit: !edit,
        },
    ]
}

struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x % n as u64) as usize
    }
}

#[derive(Clone, Debug)]
enum Op {
    Type(usize, usize),
    Command(usize, usize, &'static str),
    Bold(usize, usize, usize),
    Image(usize, usize),
    /// Set an app attribute on the cell around a position.
    CellAttr(usize, usize),
    History(usize, &'static str),
    /// Both peers send what they have not sent, and each integrates the other's.
    Exchange,
}

fn is_text_pos(doc: &Node, p: usize) -> bool {
    p <= doc.content_size()
        && doc
            .resolve(Pos(p))
            .ok()
            .is_some_and(|r| r.parent().is_textblock())
}

/// Whether `p` is inside a table.
fn in_table(doc: &Node, p: usize) -> bool {
    doc.resolve(Pos(p))
        .ok()
        .is_some_and(|r| (0..=r.depth()).any(|d| r.node(d).type_name() == "table"))
}

/// One random operation for the world's current document (peer 0's, or peer 1's).
fn random_op(rng: &mut Rng, docs: [&Node; 2]) -> Option<Op> {
    if rng.below(12) == 0 {
        return Some(Op::Exchange);
    }
    let peer = rng.below(2);
    let doc = docs[peer];
    let pos = |rng: &mut Rng| {
        // Nine in ten positions inside a table.
        for _ in 0..20 {
            let p = rng.below(doc.content_size() + 1);
            if is_text_pos(doc, p) && (in_table(doc, p) || rng.below(10) == 0) {
                return Some(p);
            }
        }
        None
    };
    Some(match rng.below(20) {
        0..=7 => Op::Type(peer, pos(rng)?),
        8 => Op::Command(peer, pos(rng)?, "splitBlock"),
        9 => Op::Command(peer, pos(rng)?, "deleteCharBackward"),
        10 => Op::Command(peer, pos(rng)?, "deleteCharForward"),
        11 => Op::Command(peer, pos(rng)?, "insertHardBreak"),
        12 => Op::Image(peer, pos(rng)?),
        13 => {
            let p = pos(rng)?;
            Op::Bold(peer, p, (p + 1 + rng.below(4)).min(doc.content_size()))
        }
        14 => {
            let names = [
                "addRowAfter",
                "addColumnBefore",
                "deleteRow",
                "deleteColumn",
                "splitCell",
            ];
            Op::Command(peer, pos(rng)?, names[rng.below(names.len())])
        }
        15 => Op::History(peer, if rng.below(2) == 0 { "undo" } else { "redo" }),
        16 => Op::CellAttr(peer, pos(rng)?),
        _ => Op::Type(peer, pos(rng)?),
    })
}

/// Apply `op` to `peers`; `false` when it does not apply there.
fn apply(peers: &mut [Peer], op: &Op) -> bool {
    if let Op::Exchange = op {
        let deltas: Vec<Vec<u8>> = peers
            .iter_mut()
            .map(|p| p.session.save_incremental().unwrap())
            .collect();
        peers[0].receive(&deltas[1]);
        peers[1].receive(&deltas[0]);
        return true;
    }
    let peer = match *op {
        Op::Type(p, _) | Op::Command(p, ..) | Op::Bold(p, ..) | Op::Image(p, _) => p,
        Op::CellAttr(p, _) => p,
        Op::History(p, _) => p,
        Op::Exchange => unreachable!(),
    };
    let st = &peers[peer].state;
    let ok = |p: usize| is_text_pos(&st.doc, p);
    let cursor = |p: usize| {
        let mut tr = st.tr();
        tr.set_selection(Selection::cursor(Pos(p)));
        st.apply(tr)
    };
    let next = match *op {
        Op::Type(_, p) => {
            if !ok(p) {
                return false;
            }
            let mut tr = st.tr();
            tr.set_selection(Selection::cursor(Pos(p)));
            if tr.insert_text("x").is_err() {
                return false;
            }
            Some(st.apply(tr))
        }
        Op::Command(_, p, name) => {
            if !ok(p) {
                return false;
            }
            cursor(p).run(name)
        }
        Op::Bold(_, p, q) => {
            if !ok(p) || !ok(q) {
                return false;
            }
            let mut tr = st.tr();
            tr.set_selection(Selection::text(Pos(p), Pos(q)));
            st.apply(tr).run("toggleBold")
        }
        Op::Image(_, p) => {
            if !ok(p) {
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
            tr.replace(p, p, Slice::new(Fragment::from_node(img), 0, 0))
                .is_ok()
                .then(|| st.apply(tr))
        }
        Op::History(_, name) => st.run(name),
        Op::CellAttr(_, p) => {
            let Ok(r) = st.doc.resolve(Pos(p)) else {
                return false;
            };
            let Some(cell) = (1..=r.depth())
                .rev()
                .find(|&d| r.node(d).type_name().ends_with("cell"))
                .and_then(|d| r.before(d))
            else {
                return false;
            };
            let mut tr = st.tr();
            tr.set_node_attr(cell, "data-note", AttrValue::from(format!("n{p}").as_str()))
                .is_ok()
                .then(|| st.apply(tr))
        }
        Op::Exchange => unreachable!(),
    };
    match next {
        // A command that leaves a ragged table is dropped, as the fuzz drops it: the
        // projection refuses one (`table::layout`), the same on both paths.
        Some(next) if next.doc != peers[peer].state.doc && tables_tile(&next.doc) => {
            peers[peer].commit(next);
            true
        }
        _ => false,
    }
}

/// Whether the trial at `seed` has a link in its first table: half of them.
///
/// A trial without one compares the two worlds' CRDTs item by item (`same_crdt`).
/// One with a link cannot: a char typed right after a link is inserted with its
/// formatting, one yrs format item per attribute in a `HashMap`'s random order, so
/// even two worlds both on the whole-table path build different item ids from the
/// same calls (seed 96 diverged in five runs of six that way). It compares what each
/// client wrote and deleted instead (`same_writes`).
fn linked(seed: u64) -> bool {
    !seed.is_multiple_of(2)
}

/// Run `ops` random operations at `seed` in two worlds, one projecting with the cell
/// path and one without, and assert every peer's CRDT bytes agree after each.
/// Returns how many cell edits the first world projected through the cell.
fn trial(seed: u64, ops: usize) -> u64 {
    let s = Rc::new(Schema::starter_kit());
    let mut with = world(&s, seed);
    let mut without = world(&s, seed);
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut history = Vec::new();
    let mut cell_edits = 0;
    for _ in 0..ops {
        let Some(op) = random_op(&mut rng, [&with[0].state.doc, &with[1].state.doc]) else {
            continue;
        };
        let before = table_cell_edits().1;
        let a = apply(&mut with, &op);
        cell_edits += table_cell_edits().1 - before;
        set_table_cell_path(false);
        let before = table_cell_edits().1;
        let b = apply(&mut without, &op);
        let leaked = table_cell_edits().1 - before;
        set_table_cell_path(true);
        assert_eq!(leaked, 0, "the switch turns the cell path off");
        history.push(op);
        assert_eq!(
            a, b,
            "seed {seed}: the op applied in one world only: {history:?}"
        );
        for p in 0..2 {
            assert_eq!(
                with[p].state.doc, without[p].state.doc,
                "seed {seed}, peer {p}: the documents differ after {history:?}"
            );
            let read = |peer: &Peer| peer.session.projected_doc(&s).unwrap();
            assert_eq!(
                read(&with[p]),
                read(&without[p]),
                "seed {seed}, peer {p}: the CRDTs read differently after {history:?}"
            );
            let (x, y) = (with[p].session.snapshot(), without[p].session.snapshot());
            let same = if linked(seed) {
                same_writes(&x, &y)
            } else {
                same_crdt(&x, &y)
            };
            assert!(
                same,
                "seed {seed}, peer {p}: the CRDTs differ after {history:?}"
            );
        }
    }
    // Converge, and the two peers agree.
    apply(&mut with, &Op::Exchange);
    apply(&mut with, &Op::Exchange);
    assert_eq!(
        with[0].state.doc, with[1].state.doc,
        "seed {seed}: converged"
    );
    cell_edits
}

#[test]
fn a_cell_edit_writes_what_the_whole_table_reconcile_writes() {
    let mut cell_edits = 0;
    for seed in 1..=40 {
        cell_edits += trial(seed, 120);
    }
    // The positive control: most of the edits went through the cell path.
    assert!(
        cell_edits > 2000,
        "only {cell_edits} cell edits took the cell path"
    );
}

/// A keystroke in a cell after the CRDT changed under the index (an edit elsewhere, a
/// peer's change) rebuilds it rather than trusting it; consecutive keystrokes in one
/// table use it.
#[test]
fn the_cell_index_is_rebuilt_after_any_other_change() {
    let s = Rc::new(Schema::starter_kit());
    let mut peers = world(&s, 7);
    let type_in = |peers: &mut [Peer], needle: &str| {
        let doc = &peers[0].state.doc;
        let at = pos_of(doc, needle);
        let mut tr = peers[0].state.tr();
        tr.set_selection(Selection::cursor(Pos(at)));
        tr.insert_text("x").unwrap();
        let next = peers[0].state.apply(tr);
        let before = table_cell_edits();
        peers[0].commit(next);
        let after = table_cell_edits();
        (after.0 - before.0, after.1 - before.1)
    };
    // (index builds, cell edits)
    assert_eq!(type_in(&mut peers, "bottom 1"), (1, 1), "the first: built");
    assert_eq!(type_in(&mut peers, "bottom 1"), (0, 1), "the next: kept");
    assert_eq!(type_in(&mut peers, "top right"), (0, 1), "another cell");
    assert_eq!(type_in(&mut peers, "t2 r0 c1"), (1, 1), "another table");
    assert_eq!(type_in(&mut peers, "bottom 2"), (1, 1), "back again");
    assert_eq!(type_in(&mut peers, "between"), (0, 0), "outside the tables");
    assert_eq!(type_in(&mut peers, "bottom 2"), (1, 1), "after that");
    // A peer's change elsewhere.
    let at = pos_of(&peers[1].state.doc, "after");
    let mut tr = peers[1].state.tr();
    tr.set_selection(Selection::cursor(Pos(at)));
    tr.insert_text("y").unwrap();
    let next = peers[1].state.apply(tr);
    peers[1].commit(next);
    apply(&mut peers, &Op::Exchange);
    assert_eq!(
        type_in(&mut peers, "bottom 2"),
        (1, 1),
        "after a peer's change"
    );
    apply(&mut peers, &Op::Exchange);
    assert_eq!(peers[0].state.doc, peers[1].state.doc);
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

/// Whether every table in `node` tiles its grid: each slot covered by exactly one cell.
fn tables_tile(node: &Node) -> bool {
    if node.type_name() == "table" {
        let width = rinch_editor_core::tables::column_count(node);
        let height = node.child_count();
        let mut taken = vec![false; width * height];
        for r in 0..height {
            let row = node.child(r);
            let mut c = 0;
            for k in 0..row.child_count() {
                while c < width && taken[r * width + c] {
                    c += 1;
                }
                let cell = row.child(k);
                let cs = cell.attrs().get_int("colspan").unwrap_or(1).max(1) as usize;
                let rs = cell.attrs().get_int("rowspan").unwrap_or(1).max(1) as usize;
                if c + cs > width || r + rs > height {
                    return false;
                }
                for rr in r..r + rs {
                    for cc in c..c + cs {
                        if std::mem::replace(&mut taken[rr * width + cc], true) {
                            return false;
                        }
                    }
                }
                c += cs;
            }
        }
        if width == 0 || taken.iter().any(|t| !t) {
            return false;
        }
    }
    (0..node.child_count()).all(|i| tables_tile(node.child(i)))
}
