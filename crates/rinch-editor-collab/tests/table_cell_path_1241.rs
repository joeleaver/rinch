//! A one-cell edit of a table projected through the cell alone writes exactly what the
//! whole-table reconcile wrote (#1241).
//!
//! A keystroke in a table cell used to read back the whole table from the CRDT, build
//! the whole table's target and compare the two — O(cells) per keystroke. It is now
//! projected through the cell (`CollabDoc::project_cell_edit`), which finds the cell's
//! map through an index kept between keystrokes and reconciles its blocks the way the
//! whole-table reconcile does. This differential runs the same random history twice
//! with pinned client ids — once with that path, once with it turned off
//! (`testing::set_table_cell_path(false)`) — and asserts after every operation that
//! every peer's document, CRDT read and CRDT agree in both: item by item
//! (`testing::same_crdt`), or where a link is in play, as many items written and
//! deleted by each client (`testing::same_writes`; see [`linked`]). The history mixes
//! typing, splits, joins, breaks, pictures, bold, links, alignment, a cell attribute,
//! merges, markdown input rules and pastes in cells (a merged one, one holding two
//! paragraphs, one with a link) of two tables, with row and column commands, undo and
//! redo, edits outside the tables (which change the CRDT under the index, so it must
//! be rebuilt), and deliveries between two peers, both ways and one way. The adversarial
//! ops, the live-collaboration test and the link-elsewhere test came from the review of
//! PR #1871.
//!
//! `TABLE_CELL_SEEDS=n` runs `n` seeds instead of the default 30 (300 passed in the
//! review).

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
        // Not asserted here that the model is the projection's read: a link added over
        // another link leaves the model two link marks where the read has one
        // (#1886), in both worlds alike. (An undo after a peer's change no longer
        // leaves unjoined text nodes, #1856.) The trial compares the two worlds'
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
    /// Only peer `.0` sends; the other integrates it.
    Deliver(usize),
    /// Merge the cells around two positions.
    Merge(usize, usize, usize),
    /// Run a named command with the caret at a position (align etc.).
    Align(usize, usize, &'static str),
    /// Add a link mark over a range.
    Link(usize, usize, usize),
    /// Type `prefix` at the start of the block around pos, then run the markdown
    /// input rules for the trigger char.
    Rule(usize, usize, &'static str, &'static str),
    /// Paste two paragraphs (open slice) or a list at a position.
    Paste(usize, usize, bool),
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
    match rng.below(10) {
        0 => return Some(Op::Exchange),
        1 => return Some(Op::Deliver(rng.below(2))),
        _ => {}
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
        17..=19 => match rng.below(6) {
            0 | 1 => {
                let p = pos(rng)?;
                Op::Merge(peer, p, pos(rng)?)
            }
            2 => {
                let names = [
                    "setTextAlignCenter",
                    "setTextAlignRight",
                    "setTextAlignLeft",
                ];
                Op::Align(peer, pos(rng)?, names[rng.below(3)])
            }
            3 => {
                let p = pos(rng)?;
                Op::Link(peer, p, (p + 1 + rng.below(4)).min(doc.content_size()))
            }
            4 => {
                let rules = [("#", " "), ("-", " "), (">", " "), ("1.", " "), ("--", "-")];
                let (a, b) = rules[rng.below(rules.len())];
                Op::Rule(peer, pos(rng)?, a, b)
            }
            _ => Op::Paste(peer, pos(rng)?, rng.below(2) == 0),
        },
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
    if let Op::Deliver(from) = *op {
        let delta = peers[from].session.save_incremental().unwrap();
        peers[1 - from].receive(&delta);
        return true;
    }
    let peer = match *op {
        Op::Merge(p, ..)
        | Op::Align(p, ..)
        | Op::Link(p, ..)
        | Op::Rule(p, ..)
        | Op::Paste(p, ..) => p,
        Op::Deliver(_) => unreachable!(),
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
        Op::Deliver(_) => unreachable!(),
        Op::Merge(_, p, q) => {
            let cell_at = |p: usize| -> Option<usize> {
                let r = st.doc.resolve(Pos(p)).ok()?;
                (1..=r.depth())
                    .rev()
                    .find(|&d| r.node(d).type_name().ends_with("cell"))
                    .and_then(|d| r.before(d))
            };
            let (Some(a), Some(b)) = (cell_at(p), cell_at(q)) else {
                return false;
            };
            if a == b {
                return false;
            }
            let mut tr = st.tr();
            tr.set_selection(Selection::cell(Pos(a), Pos(b)));
            st.apply(tr).run("mergeCells")
        }
        Op::Align(_, p, name) => {
            if !ok(p) {
                return false;
            }
            cursor(p).run(name)
        }
        Op::Link(_, p, q) => {
            if !ok(p) || !ok(q) || p >= q {
                return false;
            }
            let link = Mark::new(
                st.schema().mark_type("link").unwrap().clone(),
                Attrs::new().with(
                    "href",
                    AttrValue::from(format!("https://e.test/{p}").as_str()),
                ),
            );
            let mut tr = st.tr();
            tr.add_mark(p, q, link).is_ok().then(|| st.apply(tr))
        }
        Op::Rule(_, p, prefix, trigger) => {
            if !ok(p) {
                return false;
            }
            let r = st.doc.resolve(Pos(p)).unwrap();
            let start = p - r.parent_offset();
            let mut tr = st.tr();
            tr.set_selection(Selection::cursor(Pos(start)));
            if tr.insert_text(prefix).is_err() {
                return false;
            }
            let mid = st.apply(tr);
            let at = start + prefix.chars().count();
            let rules = rinch_editor_core::input_rules::markdown_input_rules();
            let mut c = mid.tr();
            c.set_selection(Selection::cursor(Pos(at)));
            let mid = mid.apply(c);
            match rinch_editor_core::apply_input_rules(&mid, &rules, at, trigger) {
                Some(tr) => Some(mid.apply(tr)),
                None => Some(mid),
            }
        }
        Op::Paste(_, p, list) => {
            if !ok(p) {
                return false;
            }
            let s = st.schema();
            let slice = if list {
                let item = |t: &str| branch(s, "list_item", vec![para(s, t)]);
                Slice::new(
                    Fragment::from_node(branch(s, "bullet_list", vec![item("pa"), item("pb")])),
                    0,
                    0,
                )
            } else {
                Slice::new(
                    Fragment::from_children(vec![para(s, "p1"), para(s, "p2")]),
                    1,
                    1,
                )
            };
            let mut tr = st.tr();
            tr.set_selection(Selection::cursor(Pos(p)));
            tr.replace_selection(slice).is_ok().then(|| st.apply(tr))
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
        if a {
            let k = format!("{op:?}");
            let k = k.split('(').next().unwrap().to_string();
            COUNTS.with(|c| *c.borrow_mut().entry(k).or_insert(0) += 1);
        }
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
            let any_link = linked(seed) || history.iter().any(|o| matches!(o, Op::Link(..)));
            let same = if any_link {
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
    let seeds: u64 = std::env::var("TABLE_CELL_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    for seed in 1..=seeds {
        cell_edits += trial(seed, 120);
    }
    // The positive controls: most of the edits went through the cell path, and every
    // kind of op applied somewhere.
    assert!(
        cell_edits > 1500,
        "only {cell_edits} cell edits took the cell path"
    );
    let counts = COUNTS.with(|c| c.borrow().clone());
    for kind in [
        "Type", "Command", "Bold", "Image", "History", "CellAttr", "Exchange", "Deliver", "Merge",
        "Align", "Link", "Rule", "Paste",
    ] {
        assert!(
            counts.get(kind).is_some_and(|&n| n > 0),
            "no `{kind}` op applied: {counts:?}"
        );
    }
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

thread_local! {
    /// How many ops of each kind applied in the first world: the differential's
    /// positive control that every kind ran.
    static COUNTS: std::cell::RefCell<std::collections::BTreeMap<String, u64>> =
        const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
}

fn big_world(s: &Rc<Schema>, rows: usize, edit: bool) -> Vec<Peer> {
    let table = branch(
        s,
        "table",
        (0..rows)
            .map(|r| {
                branch(
                    s,
                    "table_row",
                    (0..4)
                        .map(|c| cell(s, vec![para(s, &format!("row {r} column {c}"))]))
                        .collect(),
                )
            })
            .collect(),
    );
    let doc = branch(s, "doc", vec![para(s, "before"), table, para(s, "after")]);
    let state = EditorState::create(s.clone(), doc, plugins());
    let host = session_with_client_id(&state, 11).unwrap();
    let guest = session_from_bytes_with_client_id(&host.snapshot(), 12).unwrap();
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
            edit,
        },
    ]
}

fn type_at(peers: &mut [Peer], who: usize, needle: &str) -> (u64, u64) {
    let at = pos_of(&peers[who].state.doc, needle);
    let mut tr = peers[who].state.tr();
    tr.set_selection(Selection::cursor(Pos(at)));
    tr.insert_text("x").unwrap();
    let next = peers[who].state.apply(tr);
    let before = table_cell_edits();
    peers[who].commit(next);
    let after = table_cell_edits();
    (after.0 - before.0, after.1 - before.1)
}

/// What the index buys in live collaboration, for both ways of recording a change
/// (`record_local_edit`, what the editor view uses, and `record_local`): consecutive
/// local keystrokes in a cell build it once, and a keystroke after integrating a
/// peer's keystroke (elsewhere in the same table) builds it again, since any remote
/// update changes the CRDT under it. (`(index builds, cell edits)` per keystroke.)
#[test]
fn live_collaboration_rebuilds_the_index_after_each_peer_change() {
    let rows = 50;
    let s = Rc::new(Schema::starter_kit());
    for edit in [true, false] {
        let mut peers = big_world(&s, rows, edit);
        let needle = format!("row {} column 2", rows / 2);
        let local: Vec<_> = (0..4).map(|_| type_at(&mut peers, 0, &needle)).collect();
        assert_eq!(
            local,
            [(1, 1), (0, 1), (0, 1), (0, 1)],
            "edit={edit}: consecutive"
        );
        for i in 0..3 {
            type_at(&mut peers, 1, &format!("row {} column 1", i + 1));
            let d = peers[1].session.save_incremental().unwrap();
            peers[0].receive(&d);
            assert_eq!(
                type_at(&mut peers, 0, &needle),
                (1, 1),
                "edit={edit}: after peer change {i}"
            );
            let d = peers[0].session.save_incremental().unwrap();
            peers[1].receive(&d);
        }
        assert_eq!(peers[0].state.doc, peers[1].state.doc);
    }
}

/// A link in one cell, edits (typing, bold, unbold, align) in another: per_char
/// differs between the paths (whole table vs the cell); the CRDTs must agree item by
/// item, since no insert here goes through insert_with_attributes.
#[test]
fn a_link_in_another_cell_changes_nothing_the_cell_path_writes() {
    let s = Rc::new(Schema::starter_kit());
    let mut worlds = [world(&s, 3), world(&s, 3)]; // seed 3: linked
    for (w, on) in worlds.iter_mut().zip([true, false]) {
        set_table_cell_path(on);
        let c0 = table_cell_edits().1;
        let at = pos_of(&w[0].state.doc, "bottom 1") + 2;
        let ops: Vec<Op> = vec![
            Op::Type(0, at),
            Op::Bold(0, at - 2, at + 3),
            Op::Type(0, at + 1),
            Op::Bold(0, at - 2, at + 3),
            Op::Align(0, at, "setTextAlignCenter"),
            Op::Type(0, at + 2),
            Op::Exchange,
            Op::Type(1, at + 3),
            Op::Bold(1, at, at + 4),
            Op::Exchange,
            Op::Type(0, at + 1),
        ];
        for op in &ops {
            assert!(apply(w, op), "{op:?}");
        }
        let edits = table_cell_edits().1 - c0;
        assert_eq!(edits > 0, on, "cell path on={on}: {edits} cell edits");
        set_table_cell_path(true);
    }
    let [a, b] = &worlds;
    for p in 0..2 {
        assert_eq!(a[p].state.doc, b[p].state.doc);
        assert!(
            same_crdt(&a[p].session.snapshot(), &b[p].session.snapshot()),
            "peer {p}: the CRDTs differ item by item"
        );
    }
}
