//! Issue #1885: a local transaction that changes one textblock at two or more places
//! is written into the CRDT place by place, so the text between the places keeps its
//! identity and a peer's concurrent edit there merges instead of being undone (a
//! delete coming back) or duplicated (an insert kept beside a copy of its
//! surroundings).
//!
//! Every char in these documents is unique, so "lands once" and "stays deleted" are
//! checks on a set: an edit is merged exactly when every char any peer deleted is gone
//! and every other char (original or typed) is present exactly once.

use std::collections::BTreeSet;
use std::rc::Rc;

use rinch_editor_collab::testing::{session_from_bytes_with_client_id, session_with_client_id};
use rinch_editor_collab::{CollabPlugin, CollabSession};
use rinch_editor_core::{
    EditorState, Fragment, Node, Plugin, Schema, Slice, Transaction, default_plugins,
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

struct Peer {
    schema: Rc<Schema>,
    state: EditorState,
    session: CollabSession,
}

impl Peer {
    fn host(blocks: &[&str]) -> Peer {
        let schema = Rc::new(Schema::starter_kit());
        let blocks = blocks.iter().map(|t| para(&schema, t)).collect();
        let doc = schema
            .branch("doc", Fragment::from_children(blocks))
            .unwrap();
        let state = EditorState::create(schema.clone(), doc, plugins());
        let session = session_with_client_id(&state, 1).unwrap();
        Peer {
            schema,
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

    /// Apply and record `tr` the way the editor does: with its mapping.
    fn commit(&mut self, tr: Transaction) {
        let mapping = tr.mapping().clone();
        let next = self.state.apply(tr);
        self.session
            .record_local_mapped(&self.schema, &self.state.doc, &next.doc, &mapping)
            .expect("the edit projects");
        self.state = next;
        assert_eq!(
            self.state.doc,
            self.session.projected_doc(&self.schema).unwrap(),
            "after a local edit: model ≡ project(model)"
        );
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
    }

    fn text(&self) -> String {
        let doc = &self.state.doc;
        (0..doc.child_count())
            .map(|i| flat(doc.child(i)))
            .collect::<Vec<_>>()
            .join("|")
    }
}

/// The text under `n`, in order.
fn flat(n: &Node) -> String {
    match n.text() {
        Some(t) => t.to_string(),
        None => (0..n.child_count()).map(|i| flat(n.child(i))).collect(),
    }
}

/// Two peers on `blocks`, synced.
fn two_peers(blocks: &[&str]) -> (Peer, Peer) {
    let mut a = Peer::host(blocks);
    let mut b = a.join(2);
    let _ = (a.send(), b.send());
    (a, b)
}

fn exchange(a: &mut Peer, b: &mut Peer) {
    let (da, db) = (a.send(), b.send());
    a.receive(&db);
    b.receive(&da);
    assert_eq!(a.text(), b.text(), "converged");
}

/// The issue's repro: A deletes `f` and `a` in one transaction while B deletes `cd`.
#[test]
fn two_deletes_in_one_transaction_keep_a_peers_delete_between_them() {
    let (mut a, mut b) = two_peers(&["abcdef", "gh"]);
    let mut tr = b.state.tr();
    tr.delete(3, 5).unwrap(); // B deletes "cd"
    b.commit(tr);
    let mut tr = a.state.tr();
    tr.delete(6, 7).unwrap(); // A deletes "f"
    tr.delete(1, 2).unwrap(); // and "a", in the same transaction
    a.commit(tr);
    exchange(&mut a, &mut b);
    assert_eq!(a.text(), "be|gh");
}

/// The insert form: A types at both ends of a line in one transaction while B types
/// in its middle. B's char lands once, and the text around it is not written twice.
#[test]
fn two_inserts_in_one_transaction_keep_a_peers_insert_between_them_once() {
    let (mut a, mut b) = two_peers(&["abcdef"]);
    let mut tr = b.state.tr();
    tr.replace(
        4,
        4,
        Slice::new(Fragment::from_node(b.schema.text("X").unwrap()), 0, 0),
    )
    .unwrap();
    b.commit(tr);
    let mut tr = a.state.tr();
    let s = a.schema.clone();
    tr.replace(
        7,
        7,
        Slice::new(Fragment::from_node(s.text("Z").unwrap()), 0, 0),
    )
    .unwrap();
    tr.replace(
        1,
        1,
        Slice::new(Fragment::from_node(s.text("Y").unwrap()), 0, 0),
    )
    .unwrap();
    a.commit(tr);
    exchange(&mut a, &mut b);
    assert_eq!(a.text(), "YabcXdefZ");
}

// --- randomized: three peers, interleaved multi-place transactions -----------------

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

/// The text positions of `doc`'s textblocks, each as `(content start, content size)`.
fn blocks(doc: &Node) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut pos = 0;
    for i in 0..doc.child_count() {
        let c = doc.child(i);
        out.push((pos + 1, c.content_size()));
        pos += c.node_size();
    }
    out
}

/// One local transaction on `peer`: one to three edits in one block (a delete of one
/// or two chars, or a fresh char typed), applied from the block's end backwards so
/// each position is the one drawn. Records what it typed and deleted.
fn random_edit(
    peer: &mut Peer,
    rng: &mut Rng,
    fresh: &mut impl FnMut() -> char,
    deleted: &mut BTreeSet<char>,
) {
    let bl = blocks(&peer.state.doc);
    let (start, size) = bl[rng.below(bl.len())];
    let k = 1 + rng.below(3);
    let mut places: Vec<usize> = (0..k).map(|_| rng.below(size + 1)).collect();
    places.sort_unstable();
    places.dedup();
    let before = peer.state.doc.clone();
    let mut tr = peer.state.tr();
    let mut floor = size; // no edit reaches past the previous (later) one's start
    for &off in places.iter().rev() {
        if off > floor {
            continue;
        }
        let at = start + off;
        if rng.below(2) == 0 && off < floor {
            let len = (1 + rng.below(2)).min(floor - off);
            for c in flat(&before.cut(at, at + len)).chars() {
                deleted.insert(c);
            }
            tr.delete(at, at + len).unwrap();
        } else {
            let t = peer.schema.text(&fresh().to_string()).unwrap();
            tr.replace(at, at, Slice::new(Fragment::from_node(t), 0, 0))
                .unwrap();
        }
        floor = off;
    }
    if !tr.steps().is_empty() {
        peer.commit(tr);
    }
}

/// Runs one seeded trial; `Err` names the defect.
fn trial(seed: u64) -> Result<(), String> {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let initial = ["abcdefghij", "klmnopqrst"];
    let mut peers = vec![Peer::host(&initial)];
    for id in 2..=3 {
        let p = peers[0].join(id);
        peers.push(p);
    }
    for p in &mut peers {
        let _ = p.send();
    }
    let mut next_char = 0x4E00u32;
    let mut fresh = move || {
        next_char += 1;
        char::from_u32(next_char).unwrap()
    };
    let mut deleted = BTreeSet::new();
    // Each peer's sent deltas, and how many of them each other peer has.
    let mut outbox: Vec<Vec<Vec<u8>>> = vec![Vec::new(); 3];
    let mut seen = [[0usize; 3]; 3];
    for _ in 0..40 {
        let p = rng.below(3);
        match rng.below(4) {
            0 | 1 => random_edit(&mut peers[p], &mut rng, &mut fresh, &mut deleted),
            2 => {
                let d = peers[p].send();
                outbox[p].push(d);
            }
            _ => {
                let from = rng.below(3);
                if from != p {
                    while seen[p][from] < outbox[from].len() {
                        let d = outbox[from][seen[p][from]].clone();
                        peers[p].receive(&d);
                        seen[p][from] += 1;
                    }
                }
            }
        }
    }
    for p in 0..3 {
        let d = peers[p].send();
        outbox[p].push(d);
    }
    for p in 0..3 {
        for from in 0..3 {
            if from != p {
                for d in &outbox[from][seen[p][from]..] {
                    peers[p].receive(d);
                }
            }
        }
    }
    let text = peers[0].text();
    for p in &peers[1..] {
        if p.text() != text {
            return Err(format!("diverged: {text:?} vs {:?}", p.text()));
        }
    }
    let mut counts = std::collections::BTreeMap::new();
    for c in text.chars().filter(|&c| c != '|') {
        *counts.entry(c).or_insert(0usize) += 1;
    }
    for (c, n) in &counts {
        if *n > 1 {
            return Err(format!("{c:?} lands {n} times in {text:?}"));
        }
        if deleted.contains(c) {
            return Err(format!("deleted {c:?} came back in {text:?}"));
        }
    }
    for c in initial.concat().chars() {
        if !deleted.contains(&c) && !counts.contains_key(&c) {
            return Err(format!("{c:?} was lost from {text:?}"));
        }
    }
    Ok(())
}

/// Three peers, each making transactions that edit a block at up to three places,
/// exchanging deltas at random. No split or join, so no text moves between blocks
/// (that is #1857): every delete must stay deleted and every char land exactly once.
#[test]
fn randomized_multi_place_transactions_merge_exactly() {
    let seeds = std::env::var("TWO_PLACE_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200u64);
    let bad: Vec<String> = (1..=seeds)
        .filter_map(|s| trial(s).err().map(|e| format!("seed {s}: {e}")))
        .collect();
    assert!(
        bad.is_empty(),
        "{} of {seeds} seeds:\n{}",
        bad.len(),
        bad.join("\n")
    );
}
