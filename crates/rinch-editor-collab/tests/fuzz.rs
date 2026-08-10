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
//! Deterministic (a fixed-seed xorshift PRNG) so any failure reproduces from its
//! `(seed, peers, rounds)` triple.

use std::rc::Rc;

use rinch_editor_collab::CollabSession;
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

/// `doc(paragraph("start"))` — the shared initial document.
fn initial_state(schema: &Rc<Schema>) -> EditorState {
    let para = schema
        .branch(
            "paragraph",
            Fragment::from_node(schema.text("start").unwrap()),
        )
        .unwrap();
    let doc = schema.branch("doc", Fragment::from_node(para)).unwrap();
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

/// Run one fuzz trial: `peers` peers, `rounds` interleaved edit/deliver steps, then a
/// flush, asserting the two invariants throughout.
fn fuzz_trial(seed: u64, peers: usize, rounds: usize) {
    let schema = schema();
    let mut rng = Rng::new(seed);

    // All peers start from peer 0's snapshot, so they share one CRDT lineage.
    let init = initial_state(&schema);
    let host = CollabSession::new(&init).unwrap();
    let snapshot = host.snapshot();

    let mut states: Vec<EditorState> = Vec::with_capacity(peers);
    let mut sessions: Vec<CollabSession> = Vec::with_capacity(peers);
    states.push(init.clone());
    sessions.push(host);
    for _ in 1..peers {
        states.push(init.clone());
        sessions.push(CollabSession::from_bytes(&snapshot).unwrap());
    }

    // A global FIFO of every delta produced (production order is a topological order
    // of the change DAG, so FIFO delivery always satisfies dependencies). `seen[p]`
    // is how many of the queue's deltas peer `p` has integrated.
    let mut queue: Vec<Vec<u8>> = Vec::new();
    let mut seen = vec![0usize; peers];

    let mut edits = 0usize;
    for _ in 0..rounds {
        // 60% make a local edit, 40% deliver a pending delta.
        if rng.chance(60) {
            let p = rng.below(peers);
            let Some(next) = random_edit(&mut rng, &states[p]) else {
                continue;
            };
            // Project the local change and check the load-bearing invariant.
            sessions[p]
                .record_local(&states[p].doc, &next.doc)
                .expect("flat edit projects cleanly");
            let projected = sessions[p].projected_doc(&schema).unwrap();
            if !same(&projected, &next.doc) {
                use rinch_editor_core::serialize::node_to_html;
                panic!(
                    "model ≡ project(model) violated on a local edit \
                     (seed={seed}, peer={p}, edit#{edits})\n\
                     BEFORE: {}\n  MODEL: {}\nPROJECTED: {}",
                    node_to_html(&states[p].doc),
                    node_to_html(&next.doc),
                    node_to_html(&projected),
                );
            }
            states[p] = next;
            let delta = sessions[p].save_incremental().expect("delta encodes");
            if !delta.is_empty() {
                queue.push(delta);
            }
            edits += 1;
        } else {
            // Deliver the next unseen delta to a random peer (FIFO per peer).
            let q = rng.below(peers);
            if seen[q] < queue.len() {
                let delta = queue[seen[q]].clone();
                seen[q] += 1;
                if let Some(next) = sessions[q]
                    .integrate_incremental(&states[q], &delta)
                    .expect("delta integrates")
                {
                    // A remote integration also lands the model at project(CRDT).
                    assert!(
                        same(&sessions[q].projected_doc(&schema).unwrap(), &next.doc),
                        "model ≡ project(model) violated after integrate (seed={seed}, peer={q})"
                    );
                    states[q] = next;
                }
            }
        }
    }

    // Flush: every peer integrates every remaining delta.
    for q in 0..peers {
        while seen[q] < queue.len() {
            let delta = queue[seen[q]].clone();
            seen[q] += 1;
            if let Some(next) = sessions[q]
                .integrate_incremental(&states[q], &delta)
                .expect("delta integrates on flush")
            {
                states[q] = next;
            }
        }
    }

    // Convergence: every peer projects to the identical document, and that document
    // is what its own model holds (model ≡ project, post-flush).
    let reference = sessions[0].projected_doc(&schema).unwrap();
    for q in 0..peers {
        let projected = sessions[q].projected_doc(&schema).unwrap();
        assert!(
            same(&projected, &reference),
            "peers diverged after flush (seed={seed}, peer {q} vs peer 0)"
        );
        assert!(
            same(&states[q].doc, &reference),
            "peer {q}'s model disagrees with its CRDT after flush (seed={seed})"
        );
    }

    assert!(
        edits > 0,
        "trial should have made at least one edit (seed={seed})"
    );
}

#[test]
fn fuzz_two_peers_converge() {
    for seed in 1..=24u64 {
        fuzz_trial(seed, 2, 220);
    }
}

#[test]
fn fuzz_three_peers_converge() {
    for seed in 100..=118u64 {
        fuzz_trial(seed, 3, 320);
    }
}

#[test]
fn fuzz_four_peers_converge() {
    for seed in 200..=214u64 {
        fuzz_trial(seed, 4, 400);
    }
}

#[test]
fn fuzz_many_peers_converge() {
    // A few wider trials: more peers, longer runs, to shake out tail cases.
    for seed in 300..=305u64 {
        fuzz_trial(seed, 6, 500);
    }
}
