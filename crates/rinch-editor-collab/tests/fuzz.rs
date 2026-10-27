//! Seeded, deterministic fuzz/property tests for the collab adapter.
//!
//! Two invariants are stress-tested over thousands of random edits, spanning flat
//! text-blocks, the containers (lists and quotes, nested to depth 3 in practice) and
//! tables:
//!
//! 1. **`model ≡ project(model)`** — after EVERY local edit, the CRDT must read
//!    back (`projected_doc`) as *exactly* the editor model that produced it. This is
//!    the load-bearing invariant of the whole design (a `project_change` bug shows up
//!    here as a model/CRDT mismatch on some random edit).
//! 2. **Convergence** — N peers make concurrent random edits and relay their
//!    incremental deltas (interleaved, FIFO so a change's deps always precede it).
//!    Once every peer has seen every delta, all peers must project to the *identical*
//!    document, and one whose block structure the schema allows (`schema_valid`).
//!    This is the exact incremental-delta path pimble / `EditorHandle` use.
//!
//! A trial is **replayable bit-for-bit** from its `(seed, peers, rounds)` triple
//! (issue #214). Two things have to be pinned for that, not one. The **edit script**
//! comes from a fixed-seed xorshift PRNG, which was always deterministic. The **client
//! ids** were not: yrs breaks concurrent-insert ties by client id and `CollabDoc::blank`
//! takes yrs's default *random* one, so the converged document differed run to run — and
//! because later random positions are computed from that document, so did the edit count
//! (measured: the same binary at the same seeds reached four different outcomes in four
//! runs). Every session here is therefore built through
//! [`rinch_editor_collab::testing::session_with_client_id`] with an id derived from the
//! trial's own seed, so a failure reproduces exactly. `replaying_a_trial_is_byte_identical`
//! pins that property.
//!
//! The seam is test-only (behind this crate's `test-util` feature) on purpose: two live
//! peers sharing a client id corrupt the shared document, so production keeps the random
//! ids.

use std::rc::Rc;

use rinch_editor_collab::CollabSession;
use rinch_editor_collab::testing::{session_from_bytes_with_client_id, session_with_client_id};
use rinch_editor_core::model::Fragment;
use rinch_editor_core::{
    AttrValue, Attrs, EditorState, Node, Pos, Schema, Selection, SetNodeAttrStep, Slice,
    default_plugins,
};
use std::sync::atomic::{AtomicUsize, Ordering};

/// xorshift64* — tiny, deterministic, no deps.
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
    /// True `pct`% of the time.
    fn chance(&mut self, pct: u64) -> bool {
        self.next() % 100 < pct
    }
}

fn schema() -> Rc<Schema> {
    Rc::new(Schema::starter_kit())
}

fn para(schema: &Schema, text: &str) -> Node {
    let content = if text.is_empty() {
        Fragment::empty()
    } else {
        Fragment::from_node(schema.text(text).unwrap())
    };
    schema.branch("paragraph", content).unwrap()
}

fn doc_of(schema: &Schema, blocks: Vec<Node>) -> Node {
    schema
        .branch("doc", Fragment::from_children(blocks))
        .unwrap()
}

/// `doc(paragraph("start"))` — the shared initial document.
fn initial_state(schema: &Rc<Schema>) -> EditorState {
    let doc = doc_of(schema, vec![para(schema, "start")]);
    EditorState::create(schema.clone(), doc, default_plugins())
}

fn doc_size(state: &EditorState) -> usize {
    state.doc.content().size()
}

/// A valid (resolvable, text-biased) position somewhere in the document.
fn random_pos(rng: &mut Rng, state: &EditorState) -> Pos {
    let size = doc_size(state);
    let raw = 1 + rng.below(size.max(1));
    Selection::near(&state.doc, Pos(raw.min(size)), 1).head()
}

/// A short run of mixed-width characters (ascii + multibyte BMP + astral) — stresses
/// the codepoint↔char position equivalence the projection relies on.
fn random_text(rng: &mut Rng) -> String {
    const ALPHABET: &[char] = &['a', 'b', 'c', ' ', 'é', 'ö', '猫', 'Z', '7', '🐱'];
    let len = 1 + rng.below(3);
    (0..len)
        .map(|_| ALPHABET[rng.below(ALPHABET.len())])
        .collect()
}

/// Apply one random *projectable* edit to `state` — insert / delete / mark / split /
/// block-type, plus the list container ops (wrap, unwrap, indent, outdent). Returns
/// `None` (skip) when the random selection makes the op invalid; the fuzz tolerates
/// skips. Stays inside the projected scope (no task lists), so
/// `record_local` never hits the A22 `Unsupported` boundary — a failure here is a real
/// projection bug, not an out-of-scope node. **Inline atoms are in scope** and are
/// generated deliberately: they are one char of a block's text carrying a reserved
/// formatting attribute, so every text splice and mark resync in the projection now has
/// to carry them, and only random interleaving exercises that at the boundaries.
/// `images` also lets the inline-atom op insert an `image`; see that arm for why only
/// the convergence-only trial (`fuzz_with_images_converges`) passes `true`.
///
/// An edit that leaves a table that does not tile its grid (a ragged row) is dropped:
/// the projection refuses one, as it should, and the edits that make one here are not
/// ones the editor's UI makes. A raw `tr.delete` across rows is one (`deleteSelection`
/// refuses that selection). `splitBlock` with the caret at a gap inside a cell was the
/// other until #1232, when it split the cell; it refuses there now.
fn random_edit(rng: &mut Rng, state: &EditorState, images: bool) -> Option<EditorState> {
    let next = random_edit_unchecked(rng, state, images)?;
    tables_tile(&next.doc).then_some(next)
}

fn random_edit_unchecked(rng: &mut Rng, state: &EditorState, images: bool) -> Option<EditorState> {
    match rng.below(16) {
        // Insert text (weighted — the common case).
        0..=3 => {
            let p = random_pos(rng, state);
            let mut tr = state.tr();
            tr.set_selection(Selection::cursor(p));
            tr.insert_text(&random_text(rng)).ok()?;
            Some(state.apply(tr))
        }
        // Delete a (non-empty) range.
        4 => {
            let a = random_pos(rng, state).0;
            let b = random_pos(rng, state).0;
            let (lo, hi) = (a.min(b), a.max(b));
            if lo == hi {
                return None;
            }
            let mut tr = state.tr();
            tr.delete(lo, hi).ok()?;
            Some(state.apply(tr))
        }
        // Toggle an inline mark over a range.
        5 => {
            let a = random_pos(rng, state).0;
            let b = random_pos(rng, state).0;
            let (lo, hi) = (a.min(b), a.max(b));
            let mut tr = state.tr();
            tr.set_selection(Selection::text(Pos(lo), Pos(hi)));
            let placed = state.apply(tr);
            let cmd = [
                "toggleBold",
                "toggleItalic",
                "toggleUnderline",
                "toggleStrike",
                "toggleCode",
            ][rng.below(5)];
            placed.run(cmd)
        }
        // Split the current textblock.
        6 => {
            let p = random_pos(rng, state);
            let mut tr = state.tr();
            tr.set_selection(Selection::cursor(p));
            state.apply(tr).run("splitBlock")
        }
        // Insert an inline atom into a line. Restricted to a position whose parent is
        // a textblock: an inline node between two blocks is not a shape the model can
        // place, and generating one would (correctly) fail the projection assertion
        // below rather than find a real bug.
        11 => {
            let p = random_pos(rng, state);
            let parent_is_textblock = state
                .doc
                .resolve(p)
                .ok()
                .is_some_and(|r| r.parent().is_textblock());
            if !parent_is_textblock {
                return None;
            }
            // A `hard_break` by default, and an `image` only when `images` is set — **not**
            // because an image is out of scope (it is not; the projection tests round-trip
            // one with all three attrs). It used to break
            // `replaying_a_trial_is_byte_identical` (issue #841): a mark value carrying two
            // or more attrs is an `Any::Map`, and yrs encodes one by iterating a `HashMap`,
            // whose order is seeded per instance, so `{"@type":"image","src":…}`
            // serializes in either order from one run to the next. Since #1851 every
            // atom's value has two keys or more (`@for`, its own char), a hard break's
            // too, and the replay compares a canonical form instead ([`canonical`]).
            //
            // Images are fuzzed in their own trial, which asserts convergence. Two `src`
            // values, so that adjacent images are sometimes identical and sometimes not.
            // In the image trial, most of these change an attr of an image already
            // in the document instead: every per-attribute write, against the peers'
            // splits, joins and deletions around it.
            if images && rng.chance(70) {
                let mut found = Vec::new();
                state
                    .doc
                    .nodes_between(0, state.doc.content_size(), &mut |n, pos, _| {
                        if n.type_name() == "image" {
                            found.push(pos);
                        }
                        true
                    });
                if !found.is_empty() {
                    let at = found[rng.below(found.len())];
                    let attr = ["alt", "title", "src"][rng.below(3)];
                    let value = format!("{attr}{}", rng.below(4));
                    let mut tr = state.tr();
                    tr.step(Box::new(SetNodeAttrStep::new(
                        at,
                        attr,
                        AttrValue::from(value.as_str()),
                    )))
                    .ok()?;
                    IMAGE_ATTRS_SET.fetch_add(1, Ordering::Relaxed);
                    return Some(state.apply(tr));
                }
            }
            let atom = if images && rng.chance(50) {
                IMAGES_INSERTED.fetch_add(1, Ordering::Relaxed);
                state
                    .schema()
                    .create_node(
                        "image",
                        Attrs::new()
                            .with("src", AttrValue::from(["a.png", "b.png"][rng.below(2)]))
                            .with("alt", AttrValue::from("x")),
                        Fragment::empty(),
                    )
                    .ok()?
            } else {
                state
                    .schema()
                    .branch("hard_break", Fragment::empty())
                    .ok()?
            };
            let mut tr = state.tr();
            tr.replace(p.0, p.0, Slice::new(Fragment::from_node(atom), 0, 0))
                .ok()?;
            Some(state.apply(tr))
        }
        // Wrap/unwrap the block in a list, and nest/un-nest list items. These are the
        // container operations — they are what makes the projection recursive, so the
        // fuzz has to produce them or list convergence goes untested. Only the
        // projectable containers appear here (lists, and quotes and tables in their own
        // arms): task lists are still `Unsupported`, and generating one would
        // (correctly) fail the projection assertion below rather than find a real bug.
        9 => {
            let p = random_pos(rng, state);
            let mut tr = state.tr();
            tr.set_selection(Selection::cursor(p));
            let placed = state.apply(tr);
            let cmd = ["toggleBulletList", "toggleOrderedList"][rng.below(2)];
            placed.run(cmd)
        }
        // Wrap the block in a quote. Its way out is the lift in the next arm, which
        // lifts a block out of whatever wraps it, a quote or a list.
        12 => {
            let p = random_pos(rng, state);
            let mut tr = state.tr();
            tr.set_selection(Selection::cursor(p));
            let next = state.apply(tr).run("wrapInBlockquote")?;
            QUOTES_WRAPPED.fetch_add(1, Ordering::Relaxed);
            Some(next)
        }
        // Tables: insert one, or change the structure of the one the caret is in.
        // Typing, splitting, wrapping and deleting inside cells come from the other
        // arms, whose random positions land in cells as anywhere else.
        13 | 14 => {
            let p = random_pos(rng, state);
            if EXTRA_TABLE_OPS.with(|x| x.get())
                && let Some(next) = extra_table_op(rng, state, p)
            {
                TABLE_EDITS.with(|c| c.set(c.get() + 1));
                return Some(next);
            }
            let mut tr = state.tr();
            tr.set_selection(Selection::cursor(p));
            let placed = state.apply(tr);
            let cmd = [
                "insertTable",
                "addRowAfter",
                "addColumnBefore",
                "deleteRow",
                "deleteColumn",
                "splitCell",
            ][rng.below(6)];
            let next = placed.run(cmd)?;
            TABLE_EDITS.with(|c| c.set(c.get() + 1));
            Some(next)
        }
        // Indent / outdent a list item (creates and collapses nesting depth).
        10 => {
            let p = random_pos(rng, state);
            let mut tr = state.tr();
            tr.set_selection(Selection::cursor(p));
            let placed = state.apply(tr);
            let cmd = ["sinkListItem", "liftListItem"][rng.below(2)];
            placed.run(cmd)
        }
        // A scene break (review of #836) — the command, or a whole textblock replaced
        // by a rule (the `---` input rule's shape).
        7 => {
            let p = random_pos(rng, state);
            let mut tr = state.tr();
            tr.set_selection(Selection::cursor(p));
            state.apply(tr).run("insertHorizontalRule")
        }
        8 => {
            let p = random_pos(rng, state);
            let r = state.doc.resolve(p).ok()?;
            if !r.parent().is_textblock() {
                return None;
            }
            let d = r.depth();
            let (start, end) = (r.before(d)?, r.after(d)?);
            let hr = state
                .schema()
                .create_node("horizontal_rule", Default::default(), Fragment::empty())
                .ok()?;
            let mut tr = state.tr();
            tr.replace_with(start, end, Fragment::from_node(hr)).ok()?;
            Some(state.apply(tr))
        }
        // Set the block type (paragraph / heading / code_block — all flat).
        _ => {
            let p = random_pos(rng, state);
            let mut tr = state.tr();
            tr.set_selection(Selection::cursor(p));
            let placed = state.apply(tr);
            let cmd = [
                "setParagraph",
                "setHeading1",
                "setHeading2",
                "setHeading3",
                "setCodeBlock",
            ][rng.below(5)];
            placed.run(cmd)
        }
    }
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

/// The canonical "two docs are the same model" check.
fn same(a: &Node, b: &Node) -> bool {
    a == b
}

/// Whether every node's children satisfy its schema content expression, and every
/// child carries only marks its parent allows: a converged document must be one a model
/// can hold, not merely the same on every peer. (Two peers each deleting one of a
/// container's two children used to converge on an empty list, which none can; a code
/// block retyped while a peer formats it used to converge on marks and a hard break
/// inside `<pre>`, #1227.)
fn schema_valid(node: &Node) -> bool {
    schema_valid_in(&Schema::starter_kit(), node)
}

fn schema_valid_in(schema: &Schema, node: &Node) -> bool {
    if node.is_text() {
        return true;
    }
    let names: Vec<&str> = (0..node.child_count())
        .map(|i| node.child(i).type_name())
        .collect();
    node.node_type().content_match().matches(&names)
        && (node.type_name() != "table" || tables_tile(node))
        && (0..node.child_count()).all(|i| {
            let child = node.child(i);
            child
                .marks()
                .iter()
                .all(|m| schema.allows_mark(node.type_name(), m.type_name()))
                && schema_valid_in(schema, child)
        })
}

/// N peers sharing one CRDT lineage, plus the global delta queue and each peer's
/// delivery watermark. Both fuzz shapes below drive this one harness: the random-edit
/// trials and the scripted deletion-to-empty scenario.
struct Swarm {
    schema: Rc<Schema>,
    states: Vec<EditorState>,
    sessions: Vec<CollabSession>,
    /// Every delta produced, in production order — a topological order of the change
    /// DAG, so FIFO delivery always satisfies dependencies.
    queue: Vec<Vec<u8>>,
    /// `seen[p]` = how many of `queue`'s deltas peer `p` has integrated.
    seen: Vec<usize>,
    /// Local edits projected so far (a trial that made none proves nothing).
    edits: usize,
    /// Whether [`random_edit`] may insert images (see its inline-atom arm).
    images: bool,
    /// Percent of local edits that are an undo or a redo instead (#1249): `0` in every
    /// trial but [`fuzz_undo_and_redo_among_remote_changes`], so the others' random
    /// streams are what they were.
    undo_percent: u64,
    /// How deltas reach the peers ([`Delivery`]), from a random stream of its own.
    delivery: Delivery,
    delivery_rng: Rng,
    /// `got[q][i]`: whether peer `q` has integrated `queue[i]` (out-of-order delivery).
    got: Vec<Vec<bool>>,
    /// Integrates that failed transiently while delivery was out of order.
    transient_errors: u64,
}

thread_local! {
    /// Undos and redos that applied and changed the document, on this thread (a
    /// test's own: the tests of this binary run in parallel) — the positive
    /// control for [`fuzz_undo_and_redo_among_remote_changes`].
    static HISTORY_EDITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Images inserted by [`random_edit`] across the whole test binary — the positive
/// control that the image trial generated any.
static IMAGES_INSERTED: AtomicUsize = AtomicUsize::new(0);
/// How many image attr changes [`random_edit`] has made, for the same control.
static IMAGE_ATTRS_SET: AtomicUsize = AtomicUsize::new(0);

/// Blocks wrapped in a quote by [`random_edit`] — the positive control that the trials
/// exercise quotes at all.
static QUOTES_WRAPPED: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// Table commands applied by [`random_edit`] on this thread — the positive
    /// control for tables.
    static TABLE_EDITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

thread_local! {
    /// Undos and redos whose tables [`assert_tables_tile`] checked (#1937), on
    /// this thread.
    static UNDOS_CHECKED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Panic unless every table of `doc` tiles its grid, after peer `p`'s `cmd`.
fn assert_tables_tile(doc: &Node, seed: u64, p: usize, cmd: &str) {
    doc.nodes_between(0, doc.content_size(), &mut |n: &Node, at, _| {
        if n.type_name() == "table" {
            let problems = rinch_editor_core::tables::table_problems(n);
            assert!(
                problems.is_empty(),
                "a {cmd} left the table at {at} ragged (seed={seed}, peer={p}): {problems:?}"
            );
        }
        true
    });
}

thread_local! {
    /// Whether [`random_edit`]'s table arm also merges cells, sets a cell attribute and
    /// swaps two rows ([`extra_table_op`]): changes of a table's grid and of a cell's
    /// own keys, which a table's cell index must not be carried through (review of
    /// #1926). Off in the older trials, so their random streams are what they were.
    static EXTRA_TABLE_OPS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// One of the table changes [`EXTRA_TABLE_OPS`] adds, around position `p`, or `None`
/// (then the table arm runs a command as before).
fn extra_table_op(rng: &mut Rng, state: &EditorState, p: Pos) -> Option<EditorState> {
    let cell_at = |doc: &Node, p: usize| -> Option<usize> {
        let r = doc.resolve(Pos(p)).ok()?;
        (1..=r.depth())
            .rev()
            .find(|&d| r.node(d).type_name().ends_with("cell"))
            .and_then(|d| r.before(d))
    };
    match rng.below(4) {
        0 => {
            // Merge the cells around two positions.
            let q = random_pos(rng, state);
            let (a, b) = (cell_at(&state.doc, p.0)?, cell_at(&state.doc, q.0)?);
            if a == b {
                return None;
            }
            let mut tr = state.tr();
            tr.set_selection(Selection::cell(Pos(a), Pos(b)));
            state.apply(tr).run("mergeCells")
        }
        1 => {
            // A cell attribute: one of the cell map's own keys, not its blocks.
            let a = cell_at(&state.doc, p.0)?;
            let mut tr = state.tr();
            let v = format!("n{}", rng.below(5));
            tr.set_node_attr(a, "data-note", AttrValue::from(v.as_str()))
                .ok()?;
            Some(state.apply(tr))
        }
        2 => {
            // Swap two rows of the table around `p`, as one replace of the table.
            let r = state.doc.resolve(p).ok()?;
            let d = (1..=r.depth()).find(|&d| r.node(d).type_name() == "table")?;
            let table = r.node(d).clone();
            let at = r.before(d)?;
            let (i, j) = (
                rng.below(table.child_count()),
                rng.below(table.child_count()),
            );
            if i == j {
                return None;
            }
            let mut rows: Vec<Node> = (0..table.child_count())
                .map(|k| table.child(k).clone())
                .collect();
            rows.swap(i, j);
            let new_table = table.copy_with_content(Fragment::from_children(rows));
            let mut tr = state.tr();
            tr.replace(
                at,
                at + table.node_size(),
                Slice::new(Fragment::from_node(new_table), 0, 0),
            )
            .ok()?;
            Some(state.apply(tr))
        }
        _ => None,
    }
}

/// How a [`Swarm`] delivers the deltas its peers produce.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Delivery {
    /// Each peer integrates every delta in production order.
    Fifo,
    /// A peer integrates the next delta it has not had 60% of the time, else any one
    /// it has not had: an update ahead of one it depends on is parked by yrs, or leaves
    /// a gap in a client's clocks.
    OutOfOrder,
    /// As `OutOfOrder`, and 15% of the time two to four undelivered deltas merged into
    /// one update (`merge_updates_v1`), which may itself carry a gap.
    Merged,
}

/// A distinct, deterministic yrs client id for peer `p` of the trial at `seed`.
///
/// Distinctness *within* a trial is by construction — the low byte is a permutation of
/// the peer indices, so no two peers of one swarm can collide, which is the case that
/// would corrupt the shared document. The value stays inside yrs's 53-bit client-id space
/// for every `(seed, peer)` this file uses (seeds here are three digits).
///
/// The low byte is a **per-seed permutation** rather than the peer index itself, because
/// the client id is what yrs breaks a concurrent-insert tie by: ids that always ascend
/// with the peer index would pin every trial in the suite to one tie-break ordering, and
/// the random ids this replaced at least varied it. The permutation comes from the
/// trial's own seed, so the ordering varies from trial to trial and a trial still replays
/// bit-for-bit.
fn client_id(seed: u64, peer: usize) -> u64 {
    assert!(
        peer < 255,
        "peer index must fit the low byte of the client id"
    );
    assert!(seed < (1 << 45), "seed must leave room for the peer byte");
    let mut rank = [0u8; 255];
    for (i, r) in rank.iter_mut().enumerate() {
        *r = i as u8;
    }
    let mut rng = Rng::new(seed ^ 0x5EED_5EED);
    for i in (1..rank.len()).rev() {
        let j = rng.below(i + 1);
        rank.swap(i, j);
    }
    (seed << 8) | (rank[peer] as u64 + 1)
}

impl Swarm {
    /// `peers` peers over `doc`, the others joining from peer 0's snapshot. Each peer's
    /// yrs client id is pinned from `seed` so the trial replays bit-for-bit (issue #214).
    fn new(seed: u64, schema: Rc<Schema>, doc: Node, peers: usize) -> Swarm {
        let init = EditorState::create(schema.clone(), doc, default_plugins());
        let host = session_with_client_id(&init, client_id(seed, 0)).unwrap();
        let snapshot = host.snapshot();

        let mut states: Vec<EditorState> = Vec::with_capacity(peers);
        let mut sessions: Vec<CollabSession> = Vec::with_capacity(peers);
        states.push(init.clone());
        sessions.push(host);
        for p in 1..peers {
            states.push(init.clone());
            sessions
                .push(session_from_bytes_with_client_id(&snapshot, client_id(seed, p)).unwrap());
        }
        Swarm {
            schema,
            states,
            sessions,
            queue: Vec::new(),
            seen: vec![0usize; peers],
            edits: 0,
            images: false,
            undo_percent: 0,
            delivery: Delivery::Fifo,
            delivery_rng: Rng::new(seed.wrapping_mul(31)),
            got: vec![Vec::new(); peers],
            transient_errors: 0,
        }
    }

    fn peers(&self) -> usize {
        self.states.len()
    }

    /// Project peer `p`'s move to `next`, check the load-bearing invariant, and queue
    /// whatever it broadcasts.
    fn record_local(&mut self, seed: u64, p: usize, next: EditorState) {
        let recorded = self.sessions[p].record_local(&self.schema, &self.states[p].doc, &next.doc);
        if let Err(e) = recorded {
            // Out of order, the shared document may not read until a missing update
            // arrives (#196's transient class), and the edit stalls outbound (#220):
            // the model keeps it, and a later edit or the flush catches up.
            assert!(
                self.delivery != Delivery::Fifo
                    && !self.sessions[p].is_poisoned()
                    && !matches!(e, rinch_editor_collab::CollabError::Unsupported(_)),
                "flat edit projects cleanly (seed={seed}, peer={p}): {e}"
            );
            self.transient_errors += 1;
            self.states[p] = next;
            return;
        }
        let projected = match self.sessions[p].projected_doc(&self.schema) {
            Ok(doc) => doc,
            // Out of order, the shared document may not read until a missing update
            // arrives (#196's transient class).
            Err(e) if self.delivery != Delivery::Fifo => {
                let _ = e;
                next.doc.clone()
            }
            Err(e) => panic!("the projection reads back (seed={seed}, peer={p}): {e}"),
        };
        if !same(&projected, &next.doc) {
            use rinch_editor_core::serialize::node_to_html;
            panic!(
                "model ≡ project(model) violated on a local edit \
                 (seed={seed}, peer={p}, edit#{})\n\
                 BEFORE: {}\n  MODEL: {}\nPROJECTED: {}",
                self.edits,
                node_to_html(&self.states[p].doc),
                node_to_html(&next.doc),
                node_to_html(&projected),
            );
        }
        self.states[p] = next;
        let delta = self.sessions[p].save_incremental().expect("delta encodes");
        if !delta.is_empty() {
            self.queue.push(delta);
        }
        self.edits += 1;
    }

    /// Deliver to peer `q` a delta it has not had (a no-op when it is caught up): the
    /// next one, or under out-of-order [`Delivery`] any one, or several merged.
    fn deliver(&mut self, seed: u64, q: usize) {
        if self.delivery == Delivery::Fifo {
            if self.seen[q] >= self.queue.len() {
                return;
            }
            let delta = self.queue[self.seen[q]].clone();
            self.seen[q] += 1;
            let next = self.sessions[q]
                .integrate_incremental(&self.states[q], &delta)
                .expect("delta integrates");
            self.integrated(seed, q, next);
            return;
        }
        self.got[q].resize(self.queue.len(), false);
        let undelivered: Vec<usize> = (0..self.queue.len()).filter(|&i| !self.got[q][i]).collect();
        if undelivered.is_empty() {
            return;
        }
        let (roll, pick) = (self.delivery_rng.next() % 100, self.delivery_rng.next());
        let picks: Vec<usize> = if roll < 60 {
            vec![undelivered[0]]
        } else if roll < 85 || self.delivery == Delivery::OutOfOrder {
            vec![undelivered[pick as usize % undelivered.len()]]
        } else {
            // Two to four of the first few undelivered, in a random order.
            let k = 2 + pick as usize % 3;
            let mut first: Vec<usize> = undelivered.iter().copied().take(k + 2).collect();
            let mut out = Vec::new();
            while !first.is_empty() && out.len() < k {
                out.push(first.remove(self.delivery_rng.below(first.len())));
            }
            out
        };
        for &i in &picks {
            self.got[q][i] = true;
        }
        self.seen[q] = self.got[q].iter().take_while(|g| **g).count();
        let delta = if picks.len() == 1 {
            self.queue[picks[0]].clone()
        } else {
            yrs::merge_updates_v1(
                picks
                    .iter()
                    .map(|&i| self.queue[i].as_slice())
                    .collect::<Vec<_>>(),
            )
            .expect("updates merge")
        };
        match self.sessions[q].integrate_incremental(&self.states[q], &delta) {
            Ok(next) => self.integrated(seed, q, next),
            Err(e) => {
                assert!(
                    !self.sessions[q].is_poisoned(),
                    "an out-of-order integrate poisoned the session (seed={seed}, peer={q}): {e}"
                );
                self.transient_errors += 1;
            }
        }
    }

    /// Check and keep what peer `q`'s integrate answered.
    fn integrated(&mut self, seed: u64, q: usize, next: Option<EditorState>) {
        if let Some(next) = next {
            // A remote integration also lands the model at project(CRDT).
            assert!(
                same(
                    &self.sessions[q].projected_doc(&self.schema).unwrap(),
                    &next.doc
                ),
                "model ≡ project(model) violated after integrate (seed={seed}, peer={q})"
            );
            assert!(
                schema_valid(&next.doc),
                "a remote integration produced a document the schema does not allow \
                 (seed={seed}, peer={q}): {}",
                rinch_editor_core::serialize::node_to_html(&next.doc)
            );
            self.states[q] = next;
        }
    }

    /// Every peer integrates every remaining delta.
    fn flush(&mut self, seed: u64) {
        for q in 0..self.peers() {
            while self.seen[q] < self.queue.len() {
                self.deliver(seed, q);
            }
        }
        if self.delivery != Delivery::Fifo {
            // A transient failure left a model behind its CRDT: every delta at once
            // (nothing new to the CRDT) brings each model to the read.
            let all =
                yrs::merge_updates_v1(self.queue.iter().map(|d| d.as_slice()).collect::<Vec<_>>())
                    .expect("updates merge");
            for q in 0..self.peers() {
                let next = self.sessions[q]
                    .integrate_incremental(&self.states[q], &all)
                    .expect("the whole history integrates");
                self.integrated(seed, q, next);
            }
        }
    }

    /// `rounds` interleaved random edit/deliver steps.
    fn run_rounds(&mut self, seed: u64, rounds: usize, rng: &mut Rng) {
        for _ in 0..rounds {
            // 60% make a local edit, 40% deliver a pending delta.
            if rng.chance(60) {
                let p = rng.below(self.peers());
                let next = if self.undo_percent > 0 && rng.chance(self.undo_percent) {
                    let cmd = if rng.chance(50) { "undo" } else { "redo" };
                    let next = self.states[p].run(cmd);
                    if next.as_ref().is_some_and(|n| n.doc != self.states[p].doc) {
                        HISTORY_EDITS.with(|c| c.set(c.get() + 1));
                    }
                    // An undo or redo leaves every table tiling its grid (#1937):
                    // one it made ragged is repaired in its own transaction, so
                    // the edit still projects (a ragged table is `Unsupported`).
                    if let Some(next) = &next {
                        assert_tables_tile(&next.doc, seed, p, cmd);
                        UNDOS_CHECKED.with(|c| c.set(c.get() + 1));
                    }
                    next
                } else {
                    random_edit(rng, &self.states[p], self.images)
                };
                let Some(next) = next else {
                    continue;
                };
                self.record_local(seed, p, next);
            } else {
                let q = rng.below(self.peers());
                self.deliver(seed, q);
            }
        }
    }

    /// Convergence: every peer projects to the identical document, and that document is
    /// what its own model holds (model ≡ project, post-flush).
    fn assert_converged(&self, seed: u64) {
        let reference = self.sessions[0].projected_doc(&self.schema).unwrap();
        assert!(
            schema_valid(&reference),
            "the converged document is not one the schema allows (seed={seed})"
        );
        // No edit here types U+FFFC (`random_text`), so one in a text node is an inline
        // atom that lost its `@atom` attribute (#1851).
        assert!(
            !holds_text_fffc(&reference),
            "an inline atom read back as a literal U+FFFC (seed={seed}): {reference:?}"
        );
        for q in 0..self.peers() {
            let projected = self.sessions[q].projected_doc(&self.schema).unwrap();
            assert!(
                same(&projected, &reference),
                "peers diverged after flush (seed={seed}, peer {q} vs peer 0)"
            );
            assert!(
                same(&self.states[q].doc, &reference),
                "peer {q}'s model disagrees with its CRDT after flush (seed={seed})"
            );
        }
        assert!(
            self.edits > 0,
            "trial should have made at least one edit (seed={seed})"
        );
    }
}

/// Whether a text node anywhere in `node` holds U+FFFC.
fn holds_text_fffc(node: &Node) -> bool {
    node.text().is_some_and(|t| t.contains('\u{FFFC}'))
        || (0..node.child_count()).any(|i| holds_text_fffc(node.child(i)))
}

/// Everything about a finished trial that a replay must reproduce: how many local edits
/// it made, and the full CRDT state every peer converged on (in a canonical form, so
/// this covers clocks, tie-breaks and formatting, not just the visible text).
#[derive(PartialEq, Eq, Debug)]
struct Fingerprint {
    edits: usize,
    snapshots: Vec<String>,
}

impl Swarm {
    fn fingerprint(&self) -> Fingerprint {
        Fingerprint {
            edits: self.edits,
            snapshots: (0..self.peers())
                .map(|p| canonical(&self.sessions[p].snapshot()))
                .collect(),
        }
    }
}

/// A snapshot in a form two replays of one trial write identically: every client's
/// clock, the delete set, and every shared type walked in order with each map's keys
/// sorted. Not the bytes: an `@atom` value is a yrs map, which encodes its keys in
/// `HashMap` order, random per run, and since #1851 every atom's value has two keys or
/// more (its type and `@for`, its own char). The order of a map's keys carries nothing.
fn canonical(bytes: &[u8]) -> String {
    use yrs::types::text::YChange;
    use yrs::updates::decoder::Decode;
    use yrs::{Any, Array, ArrayRef, Map, Out, ReadTxn, Text, Transact, TransactionMut, Update};
    fn any(a: &Any) -> String {
        match a {
            Any::Map(m) => {
                let mut kv: Vec<String> =
                    m.iter().map(|(k, v)| format!("{k}:{}", any(v))).collect();
                kv.sort();
                format!("{{{}}}", kv.join(","))
            }
            Any::Array(v) => format!("[{}]", v.iter().map(any).collect::<Vec<_>>().join(",")),
            other => format!("{other:?}"),
        }
    }
    fn out(txn: &TransactionMut, o: Out, s: &mut String) {
        match o {
            Out::Any(a) => s.push_str(&any(&a)),
            Out::YText(t) => {
                for d in t.diff(txn, YChange::identity) {
                    let attrs = d.attributes.map(|a| {
                        let mut kv: Vec<String> =
                            a.iter().map(|(k, v)| format!("{k}={}", any(v))).collect();
                        kv.sort();
                        kv.join(";")
                    });
                    s.push_str(&format!("<{:?}|{:?}>", d.insert, attrs));
                }
            }
            Out::YMap(m) => {
                let mut keys: Vec<String> = m.keys(txn).map(str::to_string).collect();
                keys.sort();
                s.push('{');
                for k in keys {
                    s.push_str(&k);
                    s.push(':');
                    out(txn, m.get(txn, &k).unwrap(), s);
                    s.push(',');
                }
                s.push('}');
            }
            Out::YArray(a) => array(txn, &a, s),
            other => s.push_str(&format!("{other:?}")),
        }
    }
    fn array(txn: &TransactionMut, a: &ArrayRef, s: &mut String) {
        s.push('[');
        for v in a.iter(txn) {
            out(txn, v, s);
            s.push(',');
        }
        s.push(']');
    }
    let doc = yrs::Doc::with_options(yrs::Options {
        offset_kind: yrs::OffsetKind::Utf16,
        ..Default::default()
    });
    let content = doc.get_or_insert_array("content");
    let atoms = doc.get_or_insert_map("atoms");
    let meta = doc.get_or_insert_map("meta");
    let mut txn = doc.transact_mut();
    txn.apply_update(Update::decode_v1(bytes).unwrap()).unwrap();
    let mut clocks: Vec<String> = txn
        .state_vector()
        .iter()
        .map(|(c, k)| format!("{c:?}={k}"))
        .collect();
    clocks.sort();
    // The delete set, per client, so content deleted alike is still compared.
    let mut deleted: Vec<String> = txn
        .snapshot()
        .delete_set
        .iter()
        .map(|(c, r)| format!("{c:?}={:?}", r.iter().collect::<Vec<_>>()))
        .collect();
    deleted.sort();
    let mut s = format!("{clocks:?}{deleted:?}");
    array(&txn, &content, &mut s);
    out(&txn, Out::YMap(atoms), &mut s);
    out(&txn, Out::YMap(meta), &mut s);
    s
}

/// Run one fuzz trial: `peers` peers, `rounds` interleaved edit/deliver steps, then a
/// flush, asserting the two invariants throughout. Returns the trial's fingerprint, which
/// `replaying_a_trial_is_byte_identical` uses to pin replayability.
fn fuzz_trial(seed: u64, peers: usize, rounds: usize) -> Fingerprint {
    fuzz_trial_with(seed, peers, rounds, false)
}

fn fuzz_trial_with(seed: u64, peers: usize, rounds: usize, images: bool) -> Fingerprint {
    let schema = schema();
    let mut rng = Rng::new(seed);
    let mut swarm = Swarm::new(
        seed,
        schema.clone(),
        initial_state(&schema).doc.clone(),
        peers,
    );
    swarm.images = images;
    swarm.run_rounds(seed, rounds, &mut rng);
    swarm.flush(seed);
    swarm.assert_converged(seed);
    swarm.fingerprint()
}

/// Delete block `index` of `state`'s document outright (its open token through its
/// close token).
fn delete_block(state: &EditorState, index: usize) -> Option<EditorState> {
    let doc = &state.doc;
    let start: usize = (0..index).map(|i| doc.child(i).node_size()).sum();
    let end = start + doc.child(index).node_size();
    let mut tr = state.tr();
    tr.delete(start, end).ok()?;
    Some(state.apply(tr))
}

/// Type `text` at model position `pos`.
fn insert_at(state: &EditorState, pos: usize, text: &str) -> Option<EditorState> {
    let mut tr = state.tr();
    tr.set_selection(Selection::cursor(Pos(pos)));
    tr.insert_text(text).ok()?;
    Some(state.apply(tr))
}

/// The #192 scenario, scripted so it is exercised **by construction** rather than left to
/// chance: `peers` peers over a document of one paragraph per peer, each deleting a
/// *different* block before seeing anyone else's delta. The union of those deletions is
/// every block, so the converged content array is provably empty — the state that used to
/// wedge the session forever. Random edits then run on top of the healed state, so the
/// ordinary invariant checks cover everything downstream of the recovery.
fn fuzz_deletion_to_empty_trial(seed: u64, peers: usize, rounds: usize) -> Fingerprint {
    assert!(peers >= 2, "the scenario needs two blocks to delete apart");
    let schema = schema();
    let mut rng = Rng::new(seed);
    let blocks: Vec<Node> = (0..peers)
        .map(|i| para(&schema, &format!("block{i}")))
        .collect();
    let mut swarm = Swarm::new(seed, schema.clone(), doc_of(&schema, blocks), peers);

    // Concurrent: every peer deletes its own block, none having seen the others'.
    for p in 0..peers {
        let next = delete_block(&swarm.states[p], p).expect("a whole-block delete applies");
        assert_eq!(
            next.doc.child_count(),
            peers - 1,
            "peer {p} deleted exactly one block (seed={seed})"
        );
        swarm.record_local(seed, p, next);
    }
    swarm.flush(seed);

    // The invariant's ONE exception, asserted precisely. With zero blocks in the CRDT
    // there is no model that mirrors it — the schema requires at least one block — so the
    // read-back is the starter paragraph: equal to what every peer's model holds, but NOT
    // backed by CRDT content. Nothing else is weakened; the equality itself must still
    // hold on both sides, on every peer.
    let starter = doc_of(&schema, vec![para(&schema, "")]);
    for p in 0..peers {
        let projected = swarm.sessions[p].projected_doc(&schema).unwrap();
        assert!(
            same(&projected, &starter),
            "every block was deleted, so peer {p} must project the starter paragraph \
             (seed={seed})"
        );
        assert!(
            same(&swarm.states[p].doc, &starter),
            "peer {p}'s model must equal that projection (seed={seed})"
        );
    }

    // Every peer now types from the empty state, so every peer exercises the recovery
    // path — and they do it concurrently, which converges to one block per peer by
    // ordinary concurrent-insert semantics rather than corrupting anything.
    for p in 0..peers {
        let next = insert_at(&swarm.states[p], 1, "R").expect("typing into the starter block");
        swarm.record_local(seed, p, next);
    }
    swarm.flush(seed);
    for p in 0..peers {
        assert_eq!(
            swarm.states[p].doc.child_count(),
            peers,
            "each peer's concurrent first edit became its own block (seed={seed})"
        );
    }
    swarm.assert_converged(seed);

    // Then the ordinary random interleaving on top of the healed state.
    swarm.run_rounds(seed, rounds, &mut rng);
    swarm.flush(seed);
    swarm.assert_converged(seed);
    swarm.fingerprint()
}

/// How many integrates on this thread read back only what their update changed,
/// and how many read the whole document, while `f` runs (#1241). Every partial read
/// is compared with a whole one as it happens (`testing::set_incremental_read_check`,
/// on in this crate's debug tests).
fn integrate_reads_in(f: impl FnOnce()) -> (u64, u64) {
    use rinch_editor_collab::testing::integrate_reads;
    let before = integrate_reads();
    f();
    let after = integrate_reads();
    (after.0 - before.0, after.1 - before.1)
}

#[test]
fn fuzz_two_peers_converge() {
    let (partial, whole) = integrate_reads_in(|| {
        for seed in 1..=24u64 {
            let _ = fuzz_trial(seed, 2, 220);
        }
    });
    // The positive control for the partial read (#1241): about half the
    // integrates of this test, of the tables test and of the images test take it
    // (2359 of 4404, 1634 of 3078 and 3586 of 6047 when written), each compared
    // with a whole read.
    assert!(partial >= 1000, "{partial} partial reads, {whole} whole");
}

#[test]
fn fuzz_wraps_blocks_in_quotes() {
    // The positive control for the quote arm of `random_edit`, counted over trials of
    // its own so it does not depend on which other tests share the binary's counter.
    let before = QUOTES_WRAPPED.load(Ordering::Relaxed);
    for seed in 800..=807u64 {
        let _ = fuzz_trial(seed, 3, 300);
    }
    let wrapped = QUOTES_WRAPPED.load(Ordering::Relaxed) - before;
    assert!(
        wrapped >= 20,
        "the trials must actually wrap blocks in quotes to test them (wrapped {wrapped})"
    );
}

#[test]
fn fuzz_edits_tables_among_everything_else() {
    // The positive control for the table arm: tables inserted and restructured while
    // the other arms type, split, wrap and delete in and around their cells.
    let before = TABLE_EDITS.with(|c| c.get());
    let (partial, whole) = integrate_reads_in(|| {
        for seed in 900..=907u64 {
            let _ = fuzz_trial(seed, 3, 300);
        }
    });
    // The partial read's positive control (see `fuzz_two_peers_converge`).
    assert!(partial >= 1000, "{partial} partial reads, {whole} whole");
    let applied = TABLE_EDITS.with(|c| c.get()) - before;
    assert!(
        applied >= 20,
        "the trials must actually edit tables to test them (applied {applied})"
    );
}

/// #1249: undo and redo among remote changes. The history maps an event through every
/// change it did not record (a peer's) before it reverts it; it used to replay the
/// steps at their old positions, which broke `model ≡ project(model)` (#1856), nested
/// a paragraph in a paragraph or reverted a peer's text. 15% of local edits here are an
/// undo or a redo (seed 275 with two peers was #1249's first repro).
#[test]
fn fuzz_undo_and_redo_among_remote_changes() {
    let before = HISTORY_EDITS.with(|c| c.get());
    for seed in (1000..=1023u64).chain([275]) {
        let peers = 2 + (seed % 2) as usize;
        let schema = schema();
        let mut rng = Rng::new(seed);
        let mut swarm = Swarm::new(
            seed,
            schema.clone(),
            initial_state(&schema).doc.clone(),
            peers,
        );
        swarm.undo_percent = 15;
        swarm.run_rounds(seed, 400, &mut rng);
        swarm.flush(seed);
        swarm.assert_converged(seed);
    }
    let applied = HISTORY_EDITS.with(|c| c.get()) - before;
    assert!(
        applied >= 200,
        "the trials must actually undo and redo to test them (applied {applied})"
    );
}

#[test]
fn fuzz_three_peers_converge() {
    for seed in 100..=118u64 {
        let _ = fuzz_trial(seed, 3, 320);
    }
}

#[test]
fn fuzz_four_peers_converge() {
    for seed in 200..=214u64 {
        let _ = fuzz_trial(seed, 4, 400);
    }
}

#[test]
fn fuzz_many_peers_converge() {
    // A few wider trials: more peers, longer runs, to shake out tail cases.
    for seed in 300..=305u64 {
        let _ = fuzz_trial(seed, 6, 500);
    }
}

/// Images in the random edit stream: every per-char `@atom` resync, and the splices
/// around a placeholder whose value carries real attrs, under random interleaving
/// (review of #838). Convergence only — never compare these trials' bytes (#841).
#[test]
fn fuzz_with_images_converges() {
    let before = IMAGES_INSERTED.load(Ordering::Relaxed);
    let attrs_before = IMAGE_ATTRS_SET.load(Ordering::Relaxed);
    let (partial, whole) = integrate_reads_in(|| {
        for seed in 700..=715u64 {
            let _ = fuzz_trial_with(seed, 2 + (seed % 3) as usize, 300, true);
        }
    });
    // The partial read's positive control (see `fuzz_two_peers_converge`).
    assert!(partial >= 1000, "{partial} partial reads, {whole} whole");
    let inserted = IMAGES_INSERTED.load(Ordering::Relaxed) - before;
    assert!(
        inserted >= 20,
        "the trial must actually insert images to test anything (inserted {inserted})"
    );
    let set = IMAGE_ATTRS_SET.load(Ordering::Relaxed) - attrs_before;
    assert!(
        set >= 20,
        "the trial must actually change images' attrs to test anything (changed {set})"
    );
}

#[test]
fn fuzz_deletion_to_empty_recovers_and_converges() {
    // Issue #192: the random trials above never reach a zero-block CRDT — a single peer's
    // model cannot get there (the schema forbids it), and only concurrent deletions of
    // *different* blocks empty it on every peer at once, which random editing effectively
    // never produces. So the emptying is scripted, and the random fuzz resumes afterwards.
    for seed in 400..=407u64 {
        let _ = fuzz_deletion_to_empty_trial(seed, 2, 220);
    }
    for seed in 500..=505u64 {
        let _ = fuzz_deletion_to_empty_trial(seed, 3, 320);
    }
    for seed in 600..=603u64 {
        let _ = fuzz_deletion_to_empty_trial(seed, 4, 400);
    }
}

// ─── Delivery out of order, and grid changes (review of #1926) ──────────────────

/// One trial with `delivery`, the extra table changes on ([`EXTRA_TABLE_OPS`]),
/// pictures every other seed and undo/redo every fourth. Answers the integrates and
/// local edits that failed transiently (out of order only).
fn delivery_trial(seed: u64, peers: usize, delivery: Delivery) -> u64 {
    let undo = if seed % 4 == 1 { 15 } else { 0 };
    delivery_trial_with(seed, peers, delivery, undo)
}

/// [`delivery_trial`] with `undo` percent of local edits an undo or a redo.
fn delivery_trial_with(seed: u64, peers: usize, delivery: Delivery, undo: u64) -> u64 {
    EXTRA_TABLE_OPS.with(|x| x.set(true));
    let schema = schema();
    let mut rng = Rng::new(seed);
    let mut swarm = Swarm::new(
        seed,
        schema.clone(),
        initial_state(&schema).doc.clone(),
        peers,
    );
    swarm.images = seed.is_multiple_of(2);
    // Undo and redo every fourth seed: with these grid changes an undo panicked in
    // the history's mapping (`StepMap::recover`, from `Branch::pop_event`) until
    // #1935.
    swarm.undo_percent = undo;
    swarm.delivery = delivery;
    swarm.delivery_rng = Rng::new(seed.wrapping_mul(31) + delivery as u64);
    swarm.run_rounds(seed, 300, &mut rng);
    swarm.flush(seed);
    swarm.assert_converged(seed);
    EXTRA_TABLE_OPS.with(|x| x.set(false));
    swarm.transient_errors
}

/// Grid changes (merged cells, a cell's attributes, swapped rows) among everything
/// else, delivered in order: every partial read is compared with a whole read
/// (`testing::set_incremental_read_check`), so a table's cell index carried through a
/// change of its grid shows as a wrong read.
#[test]
fn fuzz_grid_changes_in_order() {
    let before = TABLE_EDITS.with(|c| c.get());
    let (partial, whole) = integrate_reads_in(|| {
        for seed in 5001..=5024u64 {
            delivery_trial(seed, 2 + (seed % 3) as usize, Delivery::Fifo);
        }
    });
    assert!(partial >= 1000, "{partial} partial reads, {whole} whole");
    let applied = TABLE_EDITS.with(|c| c.get()) - before;
    assert!(applied >= 100, "only {applied} table changes applied");
}

/// Deltas delivered out of order (review of #1926). yrs parks an update that arrives
/// ahead of one it depends on, or keeps a gap in its sender's clocks, and the update
/// that fills the gap fires no event for a change inside any type its sender created
/// after the gap: no path names it, so the read must be the whole one. Seeds 9001 and
/// 9002 diverged before the record was spoiled for it (a paragraph read back without
/// the char typed in it).
#[test]
fn fuzz_out_of_order_delivery_converges() {
    let mut transient = 0;
    let (partial, whole) = integrate_reads_in(|| {
        for seed in 9001..=9024u64 {
            transient += delivery_trial(seed, 2 + (seed % 3) as usize, Delivery::OutOfOrder);
        }
    });
    // Both kinds of read ran (437 partial, 7865 whole when written: a gap stays
    // open for a while, and every integrate while it is reads the whole document).
    assert!(partial >= 200, "{partial} partial reads, {whole} whole");
    assert!(whole >= 500, "{partial} partial reads, {whole} whole");
    let _ = transient;
}

/// As [`fuzz_out_of_order_delivery_converges`], with batches of deltas merged into one
/// update, which can carry a gap of its own.
#[test]
fn fuzz_merged_delivery_converges() {
    let (partial, whole) = integrate_reads_in(|| {
        for seed in 9101..=9124u64 {
            delivery_trial(seed, 2 + (seed % 3) as usize, Delivery::Merged);
        }
    });
    assert!(partial >= 200, "{partial} partial reads, {whole} whole");
    assert!(whole >= 500, "{partial} partial reads, {whole} whole");
}

/// #1937: undo and redo among peers' grid changes (merged cells, cell attributes,
/// swapped rows), delivered in order, out of order and in merged batches. After every
/// undo and redo every table tiles its grid (`assert_tables_tile`): one the undo made
/// ragged is repaired in its own transaction (an undo of my merge after a peer's merge
/// restored cells into the peer's table and kept the spans). And the edit still
/// records to the CRDT: a ragged table is `Unsupported`, which stalled outbound.
#[test]
fn fuzz_undo_among_grid_changes_keeps_tables_tiling() {
    let before = (
        UNDOS_CHECKED.with(|c| c.get()),
        HISTORY_EDITS.with(|c| c.get()),
    );
    for (base, delivery) in [
        (7001u64, Delivery::Fifo),
        (7101, Delivery::OutOfOrder),
        (7201, Delivery::Merged),
    ] {
        for seed in base..base + 16 {
            delivery_trial_with(seed, 2 + (seed % 3) as usize, delivery, 30);
        }
    }
    let checked = UNDOS_CHECKED.with(|c| c.get()) - before.0;
    let applied = HISTORY_EDITS.with(|c| c.get()) - before.1;
    assert!(
        checked >= 1000 && applied >= 300,
        "{checked} undos and redos checked, {applied} changed the document"
    );
}

/// Issue #214: a `(seed, peers, rounds)` triple must reproduce a trial **exactly**, or a
/// fuzz failure cannot be debugged — the report names a seed that does something else
/// when you run it.
///
/// The check is not "the text matches": it compares each peer's whole yrs state (every
/// client's clock, every shared type with its formatting; see [`canonical`] for why
/// not the bytes), so a differing tie-break or block id fails it even when the visible
/// document happens to agree. It also compares the edit *count*, which is the symptom that made the old
/// non-determinism visible — random positions are drawn from the live document, so a
/// different convergence changes which later edits are applicable at all.
///
/// Both trial shapes are covered. `fuzz_deletion_to_empty_trial` is the one that matters
/// most: its whole subject is concurrent inserts into an emptied array, decided by
/// exactly the client-id tie-break this pins.
#[test]
fn replaying_a_trial_is_byte_identical() {
    for seed in [7u64, 113, 207] {
        assert_eq!(
            fuzz_trial(seed, 3, 120),
            fuzz_trial(seed, 3, 120),
            "random trial at seed {seed} must replay bit-for-bit"
        );
    }
    for seed in [401u64, 502] {
        assert_eq!(
            fuzz_deletion_to_empty_trial(seed, 3, 120),
            fuzz_deletion_to_empty_trial(seed, 3, 120),
            "deletion-to-empty trial at seed {seed} must replay bit-for-bit"
        );
    }
}

/// The determinism above must not have been bought by making every trial *identical* —
/// that would silently collapse the fuzz to one scenario repeated. Different seeds must
/// still explore different documents.
#[test]
fn different_seeds_still_diverge() {
    let a = fuzz_trial(11, 3, 120);
    let b = fuzz_trial(12, 3, 120);
    assert_ne!(a, b, "different seeds must produce different trials");
}
