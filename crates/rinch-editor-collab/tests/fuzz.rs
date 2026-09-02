//! Seeded, deterministic fuzz/property tests for the collab adapter.
//!
//! Two invariants are stress-tested over thousands of random edits, spanning both
//! flat text-blocks and list containers (nested to depth 3 in practice):
//!
//! 1. **`model ≡ project(model)`** — after EVERY local edit, the CRDT must read
//!    back (`projected_doc`) as *exactly* the editor model that produced it. This is
//!    the load-bearing invariant of the whole design (a `project_change` bug shows up
//!    here as a model/CRDT mismatch on some random edit).
//! 2. **Convergence** — N peers make concurrent random edits and relay their
//!    incremental deltas (interleaved, FIFO so a change's deps always precede it).
//!    Once every peer has seen every delta, all peers must project to the *identical*
//!    document. This is the exact incremental-delta path pimble / `EditorHandle` use.
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
use rinch_editor_core::{EditorState, Node, Pos, Schema, Selection, default_plugins};

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
/// skips. Stays inside the projected scope (no task lists, blockquotes, tables, or
/// inline atoms), so `record_local` never hits the A22 `Unsupported` boundary — a
/// failure here is a real projection bug, not an out-of-scope node.
fn random_edit(rng: &mut Rng, state: &EditorState) -> Option<EditorState> {
    match rng.below(11) {
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
        // Wrap/unwrap the block in a list, and nest/un-nest list items. These are the
        // container operations — they are what makes the projection recursive, so the
        // fuzz has to produce them or list convergence goes untested. Only the
        // projectable containers appear here: task lists and blockquotes are still
        // `Unsupported`, and generating one would (correctly) fail the projection
        // assertion below rather than find a real bug.
        9 => {
            let p = random_pos(rng, state);
            let mut tr = state.tr();
            tr.set_selection(Selection::cursor(p));
            let placed = state.apply(tr);
            let cmd = ["toggleBulletList", "toggleOrderedList"][rng.below(2)];
            placed.run(cmd)
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

/// The canonical "two docs are the same model" check.
fn same(a: &Node, b: &Node) -> bool {
    a == b
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
        }
    }

    fn peers(&self) -> usize {
        self.states.len()
    }

    /// Project peer `p`'s move to `next`, check the load-bearing invariant, and queue
    /// whatever it broadcasts.
    fn record_local(&mut self, seed: u64, p: usize, next: EditorState) {
        self.sessions[p]
            .record_local(&self.states[p].doc, &next.doc)
            .expect("flat edit projects cleanly");
        let projected = self.sessions[p].projected_doc(&self.schema).unwrap();
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

    /// Deliver peer `q`'s next unseen delta (a no-op when it is already caught up).
    fn deliver(&mut self, seed: u64, q: usize) {
        if self.seen[q] >= self.queue.len() {
            return;
        }
        let delta = self.queue[self.seen[q]].clone();
        self.seen[q] += 1;
        if let Some(next) = self.sessions[q]
            .integrate_incremental(&self.states[q], &delta)
            .expect("delta integrates")
        {
            // A remote integration also lands the model at project(CRDT).
            assert!(
                same(
                    &self.sessions[q].projected_doc(&self.schema).unwrap(),
                    &next.doc
                ),
                "model ≡ project(model) violated after integrate (seed={seed}, peer={q})"
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
    }

    /// `rounds` interleaved random edit/deliver steps.
    fn run_rounds(&mut self, seed: u64, rounds: usize, rng: &mut Rng) {
        for _ in 0..rounds {
            // 60% make a local edit, 40% deliver a pending delta.
            if rng.chance(60) {
                let p = rng.below(self.peers());
                let Some(next) = random_edit(rng, &self.states[p]) else {
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

/// Everything about a finished trial that a replay must reproduce: how many local edits
/// it made, and the full CRDT state every peer converged on (yrs's own bytes, so this
/// covers block ids and tie-breaks, not just the visible text).
#[derive(PartialEq, Eq, Debug)]
struct Fingerprint {
    edits: usize,
    snapshots: Vec<Vec<u8>>,
}

impl Swarm {
    fn fingerprint(&self) -> Fingerprint {
        Fingerprint {
            edits: self.edits,
            snapshots: (0..self.peers())
                .map(|p| self.sessions[p].snapshot())
                .collect(),
        }
    }
}

/// Run one fuzz trial: `peers` peers, `rounds` interleaved edit/deliver steps, then a
/// flush, asserting the two invariants throughout. Returns the trial's fingerprint, which
/// `replaying_a_trial_is_byte_identical` uses to pin replayability.
fn fuzz_trial(seed: u64, peers: usize, rounds: usize) -> Fingerprint {
    let schema = schema();
    let mut rng = Rng::new(seed);
    let mut swarm = Swarm::new(
        seed,
        schema.clone(),
        initial_state(&schema).doc.clone(),
        peers,
    );
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

#[test]
fn fuzz_two_peers_converge() {
    for seed in 1..=24u64 {
        let _ = fuzz_trial(seed, 2, 220);
    }
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

/// Issue #214: a `(seed, peers, rounds)` triple must reproduce a trial **exactly**, or a
/// fuzz failure cannot be debugged — the report names a seed that does something else
/// when you run it.
///
/// The check is not "the text matches": it compares each peer's whole yrs state, so a
/// differing tie-break or block id fails it even when the visible document happens to
/// agree. It also compares the edit *count*, which is the symptom that made the old
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
