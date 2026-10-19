//! Which CRDT block each leaf block atom of an editor's current document is — kept
//! between operations so a remote change does not read the CRDT twice to find out
//! (review of #1575, F5).
//!
//! Telling an `embed` whose attributes a peer changed from another `embed` that
//! came to stand at its place ([`crate::remote::AtomIds`]) needs the atoms' CRDT
//! blocks before and after the update. The "after" comes free with the read every
//! integrate makes; the "before" is the previous operation's "after". This module
//! keeps it, per session, keyed by the model document it describes
//! ([`Node::same_ref`]) and the session's CRDT **generation** (bumped by every
//! session call that can change the CRDT), so it is used only for exactly the
//! document and CRDT it was read from. While outbound is stalled (#220) an entry
//! holds what a refused local edit carried instead, an [`unproven`] id standing
//! for each atom nothing proves.
//!
//! It is a thread-local rather than a session field because a [`Node`] is `!Send`
//! and a `CollabSession` must stay `Send`. A session used from another thread
//! finds nothing here and reads the CRDT instead; an entry left on a thread by a
//! session dropped on another is never used again (its id is unique) and holds one
//! document until that thread ends.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use rinch_editor_core::Node;

use crate::projection::BlockId;

struct Entry {
    generation: u64,
    doc: Node,
    ids: Vec<BlockId>,
}

thread_local! {
    static CACHE: RefCell<HashMap<u64, Entry>> = RefCell::new(HashMap::new());
}

/// A process-unique session id.
pub(crate) fn next_uid() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The atoms' blocks of `doc`, if session `uid` stored them at `generation` for
/// this very document.
pub(crate) fn lookup(uid: u64, generation: u64, doc: &Node) -> Option<Vec<BlockId>> {
    CACHE
        .try_with(|c| {
            c.borrow()
                .get(&uid)
                .filter(|e| e.generation == generation && e.doc.same_ref(doc))
                .map(|e| e.ids.clone())
        })
        .ok()
        .flatten()
}

/// Remember that `doc`'s leaf block atoms are the CRDT blocks `ids`, in document
/// order, for session `uid` at `generation`.
pub(crate) fn store(uid: u64, generation: u64, doc: Node, ids: Vec<BlockId>) {
    let _ = CACHE.try_with(|c| {
        c.borrow_mut().insert(
            uid,
            Entry {
                generation,
                doc,
                ids,
            },
        )
    });
}

/// Forget session `uid` (it is dropped).
pub(crate) fn forget(uid: u64) {
    let _ = CACHE.try_with(|c| c.borrow_mut().remove(&uid));
    inline_forget(uid);
}

/// A [`BlockId`] that names no CRDT block and equals no other: what a leaf block
/// atom the model holds is said to be when nothing proves which block it is (one
/// a refused local edit wrote while outbound is stalled, #220). A remote change
/// compares it with the converged document's blocks and finds no match, so the
/// atom is replaced and a node view drawn for it remounted — never re-pointed at
/// a block it was not mounted for. yrs client ids stay below 2^53, so the client
/// `u64::MAX` is never a real one.
pub(crate) fn unproven() -> BlockId {
    thread_local! {
        static NEXT: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }
    let clock = NEXT.with(|n| {
        let c = n.get();
        n.set(c.wrapping_add(1));
        c
    });
    (u64::MAX, clock)
}

/// Whether `id` is an [`unproven`] one.
pub(crate) fn is_unproven(id: BlockId) -> bool {
    id.0 == u64::MAX
}

/// `n` [`unproven`] ids.
pub(crate) fn unproven_ids(n: usize) -> Vec<BlockId> {
    (0..n).map(|_| unproven()).collect()
}

thread_local! {
    /// The count behind [`walks`].
    static WALKS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Note one walk of a change's blocks to carry atom ids forward.
pub(crate) fn note_walk() {
    WALKS.with(|w| w.set(w.get() + 1));
}

/// The running count behind `crate::testing::atom_id_walks`.
#[cfg_attr(not(feature = "test-util"), allow(dead_code))]
pub(crate) fn walks() -> u64 {
    WALKS.with(|w| w.get())
}

// ─── Inline atoms (node views C) ─────────────────────────────────────────────

thread_local! {
    /// Per session: the identity (`crate::atoms`) of each inline atom node of the
    /// editor's document as of the last read, by node (`Rc` identity), in document
    /// order. Unlike the block atoms' list it is not tied to one document: an
    /// inline atom node the model keeps through a local edit (typing beside a
    /// picture) is the same CRDT char, so its identity holds while its `Rc` does;
    /// a node the model made since (an inserted, pasted, moved or re-set picture)
    /// is not in the list, and is unknown until the next read.
    static INLINE: RefCell<HashMap<u64, Vec<(Node, String)>>> = RefCell::new(HashMap::new());
}

/// The identity of each inline atom of `doc` session `uid` knows, in document
/// order (`None` for one it does not).
pub(crate) fn inline_lookup(uid: u64, doc: &Node) -> Vec<Option<String>> {
    let mut atoms = Vec::new();
    crate::remote::inline_atom_nodes(doc, &mut atoms);
    INLINE
        .try_with(|c| {
            let c = c.borrow();
            let Some(known) = c.get(&uid) else {
                return vec![None; atoms.len()];
            };
            // Both lists are in document order, and an edit keeps the order of the
            // nodes it keeps: one pass, each known node taken once.
            let mut from = 0;
            atoms
                .iter()
                .map(|a| {
                    let found = known[from..].iter().position(|(n, _)| n.same_ref(a))?;
                    from += found + 1;
                    Some(known[from - 1].1.clone())
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Remember that `doc`'s inline atoms are, in document order, `ids`.
pub(crate) fn inline_store(uid: u64, doc: &Node, ids: &[Option<String>]) {
    let mut atoms = Vec::new();
    crate::remote::inline_atom_nodes(doc, &mut atoms);
    if atoms.len() != ids.len() {
        inline_forget(uid);
        return;
    }
    let known: Vec<(Node, String)> = atoms
        .into_iter()
        .zip(ids)
        .filter_map(|(n, id)| Some((n, id.clone()?)))
        .collect();
    let _ = INLINE.try_with(|c| c.borrow_mut().insert(uid, known));
}

/// A local change only set the attributes of these inline atoms (`old` → `new`):
/// each `new` is the CRDT char `old` was.
pub(crate) fn inline_carry(uid: u64, pairs: &[(Node, Node)]) {
    let _ = INLINE.try_with(|c| {
        if let Some(known) = c.borrow_mut().get_mut(&uid) {
            for (old, new) in pairs {
                if let Some(entry) = known.iter_mut().find(|(n, _)| n.same_ref(old)) {
                    entry.0 = new.clone();
                }
            }
        }
    });
}

/// Forget session `uid`'s inline identities (a re-base may have paired its atoms
/// with other chars, or the session is dropped).
pub(crate) fn inline_forget(uid: u64) {
    let _ = INLINE.try_with(|c| c.borrow_mut().remove(&uid));
}
