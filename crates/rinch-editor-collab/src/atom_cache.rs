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
//! document and CRDT it was read from.
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
}
