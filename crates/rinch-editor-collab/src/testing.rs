//! **Test-only** determinism seam: sessions whose yrs client id is pinned by the
//! caller (issue #214). Compiled only under the crate's `test-util` feature, which
//! nothing but this crate's own `[dev-dependencies]` turns on — a downstream release
//! build cannot reach these functions at all.
//!
//! # Why this exists
//!
//! The fuzz suites (`tests/fuzz.rs`) drive a swarm of peers from a fixed-seed PRNG, so
//! the *edit script* is deterministic. The resulting document was not. yrs breaks a tie
//! between two concurrent inserts at the same position by **client id**, and
//! `Options::default()` calls `ClientID::random()` — so the converged document, and with
//! it every later random position computed from that document, differed run to run.
//! Two runs of the same binary at the same seed reached different documents and even
//! different edit counts, which meant a failing trial could not be reproduced from its
//! `(seed, peers, rounds)` triple. Pinning the ids makes a trial replayable bit-for-bit.
//!
//! # Why it must not be reachable from production
//!
//! The client id is a replica's **identity** in the CRDT. Two live peers sharing one
//! produce blocks with colliding ids, which yrs treats as the same block: the replicas
//! silently stop converging and the shared document is corrupt with no error anywhere.
//! That is precisely the divergence class this crate exists to kill, so the seam is
//! gated rather than merely `#[doc(hidden)]`:
//!
//! * the `test-util` feature is **off by default** and is enabled only by this crate's
//!   self dev-dependency, so it is on for `cargo test -p rinch-editor-collab` and off
//!   everywhere else;
//! * the facade (`rinch`'s `collaboration` feature) never names it, so no downstream
//!   feature unification can switch it on;
//! * every public path stays random — [`CollabSession::new`] and
//!   [`CollabSession::from_bytes`] are untouched.
//!
//! # Use
//!
//! Give each peer of a trial a **distinct** id derived from the trial's own seed, never
//! a process-global counter: tests run in parallel threads, so a shared counter would
//! reintroduce exactly the scheduling-dependent non-determinism this seam removes.
//!
//! ```ignore
//! let host = testing::session_with_client_id(&state, client_id(seed, 0))?;
//! let guest = testing::session_from_bytes_with_client_id(&host.snapshot(), client_id(seed, 1))?;
//! ```

use rinch_editor_core::EditorState;

use crate::error::Result;
use crate::session::CollabSession;

/// [`CollabSession::new`] with this replica's yrs client id pinned to `client_id`.
///
/// Every peer of one collaboration must be given a **different** id — see the module
/// docs for what sharing one does to the shared document.
pub fn session_with_client_id(state: &EditorState, client_id: u64) -> Result<CollabSession> {
    CollabSession::new_with_client_id(state, checked(client_id))
}

/// [`CollabSession::from_bytes`] with this replica's yrs client id pinned to
/// `client_id` — the joining half of [`session_with_client_id`].
///
/// Every peer of one collaboration must be given a **different** id, the host included.
pub fn session_from_bytes_with_client_id(bytes: &[u8], client_id: u64) -> Result<CollabSession> {
    CollabSession::from_bytes_with_client_id(bytes, checked(client_id))
}

/// How many CRDT nodes this **thread** has examined so far: one per node read back
/// (its text, marks and children) and one per node whose shape was checked for a void
/// container. Take the difference across an operation to count what it read — the pin
/// that a local keystroke reads the block it changed and not the whole document.
pub fn node_reads() -> u64 {
    crate::projection::node_reads()
}

/// How many entries of the shared document's per-attribute atom map (`atoms`) this
/// **thread** has read so far. Take the difference across an operation: a remote
/// integrate or a local change reads the map at most once (review of #1503, F4), so
/// the count grows with the entries, never with the entries times the blocks.
pub fn overlay_entry_reads() -> u64 {
    crate::atoms::overlay_entry_reads()
}

/// How many block texts (a block's characters and formatting) this **thread** has
/// scanned so far. Take the difference across an operation: a local keystroke scans
/// the blocks it changed, never every block of the document.
pub fn text_scans() -> u64 {
    crate::atoms::text_scans()
}

/// How many block pairs the projection has scored on this thread so far to place the
/// blocks a split or join inserts or deletes (`write_child_diff`, #1850). Each changed
/// pair is scored at most twice per change.
pub fn gap_scores() -> u64 {
    crate::projection::gap_scores()
}

/// What the edits of one table cell have cost on this **thread** so far (#1241): how
/// many times a whole table was read to find the cells of a table, and how many cell
/// edits were projected through that cell alone. Take the difference across an
/// operation; the second count is the positive control that a test exercised the
/// cell path.
pub fn table_cell_edits() -> (u64, u64) {
    crate::table::cell_index_counts()
}

/// Project a one-cell edit of a table by reconciling the whole table (`false`), as
/// before #1241, or through the cell alone (`true`, the default), on this **thread**:
/// the switch a test comparing the two turns.
pub fn set_table_cell_path(on: bool) {
    crate::table::set_cell_path(on)
}

/// Whether two snapshots ([`CollabSession::snapshot`]) hold the same CRDT: the same
/// items, with the same ids, origins and content, and the same deletions. Comparing
/// the bytes is not that: a map value (an inline atom's `@atom`) encodes its keys in
/// a `HashMap`'s order, which differs between two writes of one value.
pub fn same_crdt(a: &[u8], b: &[u8]) -> bool {
    use yrs::Update;
    use yrs::updates::decoder::Decode;
    match (Update::decode_v1(a), Update::decode_v1(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// [`same_crdt`], or where that cannot hold, the same **writes**: as many items
/// written by each client (the state vectors agree) and as many of them deleted.
///
/// Two sessions making the same calls do not always build the same CRDT: text
/// inserted with its formatting (`insert_with_attributes`, which the projection
/// uses for a char typed right after a link) writes one format item per attribute,
/// in the order of a `HashMap` that is seeded at random, so which item id ends up as
/// which marker — and which of two redundant markers yrs later deletes — differs
/// from run to run.
pub fn same_writes(a: &[u8], b: &[u8]) -> bool {
    use std::collections::BTreeMap;
    use yrs::Update;
    use yrs::updates::decoder::Decode;
    let (Ok(a), Ok(b)) = (Update::decode_v1(a), Update::decode_v1(b)) else {
        return false;
    };
    let deleted = |u: &Update| -> BTreeMap<u64, u32> {
        u.delete_set()
            .iter()
            .map(|(client, ranges)| {
                (
                    client.get(),
                    ranges.iter().map(|r| r.end - r.start).sum::<u32>(),
                )
            })
            .collect()
    };
    a == b || (a.state_vector() == b.state_vector() && deleted(&a) == deleted(&b))
}

/// yrs client ids are **53-bit** (`ClientID::new` debug-asserts it, and a release build
/// would silently fold the high bits into the mask instead), so two ids that differ only
/// above bit 52 would collide — the corruption this module exists to warn about. Reject
/// that here rather than let it through in a release-profile test run.
fn checked(client_id: u64) -> u64 {
    assert!(
        client_id < (1u64 << 53),
        "collab client ids are 53-bit; {client_id} does not fit and would collide with \
         another id that differs only in its high bits"
    );
    client_id
}

/// A foreign writer's update on top of `snapshot` (a session's
/// [`CollabSession::snapshot`]): `n` row lines and `n` column lines appended to the
/// first top-level table, so it reads as a table too large to read once `n` is a
/// few hundred. What a peer on another build, or a hostile one, can send; the tests
/// of the freeze (`CollabError::OversizedTable`) forge it with this, in any crate,
/// without linking `yrs` themselves. `None` when the document holds no top-level
/// table.
pub fn grow_first_table_update(snapshot: &[u8], n: usize) -> Option<Vec<u8>> {
    use yrs::updates::decoder::Decode;
    use yrs::{Any, Array, ArrayRef, Map, MapPrelim, Out, ReadTxn, Transact, Update};
    let doc = yrs::Doc::with_client_id(999);
    {
        let mut txn = doc.transact_mut();
        txn.apply_update(Update::decode_v1(snapshot).ok()?).ok()?;
    }
    let sv = doc.transact().state_vector();
    {
        let content: ArrayRef = doc.get_or_insert_array("content");
        let mut txn = doc.transact_mut();
        let table = content.iter(&txn).find_map(|child| match child {
            Out::YMap(m) if m.get(&txn, "rows").is_some() => Some(m),
            _ => None,
        })?;
        let (Some(Out::YArray(cols)), Some(Out::YArray(rows))) =
            (table.get(&txn, "cols"), table.get(&txn, "rows"))
        else {
            return None;
        };
        // Inserted at index 1, not pushed: `push_back` walks the whole array per insert.
        for i in 0..n {
            let m = cols.insert(&mut txn, 1, MapPrelim::default());
            m.insert(&mut txn, "id", Any::String(format!("grown-c{i}").into()));
            let r = rows.insert(&mut txn, 1, MapPrelim::default());
            r.insert(&mut txn, "id", Any::String(format!("grown-r{i}").into()));
        }
    }
    Some(doc.transact().encode_state_as_update_v1(&sv))
}

/// How many times this **thread** has walked a local change's top-level blocks to
/// carry the CRDT blocks of its leaf block atoms forward (the `atom_cache` of the
/// node views, #1575). Take the difference across an operation: a local keystroke
/// in a document holding no leaf block atom walks none, before or after the
/// session has integrated a peer's change (review 2 of #1575, N3).
pub fn atom_id_walks() -> u64 {
    crate::atom_cache::walks()
}

/// The atoms `tr` only set attributes of, as they stand in its final document, each
/// with the node it was in `tr`'s first document: what an editor hands
/// [`CollabSession::record_local_edit`] (the view's `kept_nodes`, restated for a test
/// that drives a session itself). A node an earlier step put in place, or a later
/// step removed, is not one of them.
pub fn kept_atom_pairs(
    tr: &rinch_editor_core::Transaction,
) -> Vec<(rinch_editor_core::Node, rinch_editor_core::Node)> {
    let (steps, mapping) = (tr.steps(), tr.mapping());
    let mut out: Vec<(usize, rinch_editor_core::Node, rinch_editor_core::Node)> = Vec::new();
    for (i, step) in steps.iter().enumerate() {
        let Some(a) = step
            .as_any()
            .downcast_ref::<rinch_editor_core::SetNodeAttrStep>()
        else {
            continue;
        };
        let after = mapping.slice(i + 1, mapping.len()).map_result(a.pos, 1);
        if after.deleted_after() || out.iter().any(|(p, _, _)| *p == after.pos) {
            continue;
        }
        let mut inserted = false;
        let mut before = a.pos;
        for map in mapping.maps()[..i].iter().rev() {
            let r = map.invert().map_result(before, 1);
            inserted |= r.deleted_after();
            before = r.pos;
        }
        if inserted {
            continue;
        }
        if let Some(node) = tr.doc().node_at(after.pos)
            && node.is_leaf()
            && !node.is_text()
            && let Some(old) = tr.docs().first().and_then(|d| d.node_at(before))
        {
            out.push((after.pos, old, node));
        }
    }
    out.into_iter().map(|(_, o, n)| (o, n)).collect()
}

/// The new halves of [`kept_atom_pairs`]: what [`CollabSession::record_local_keeping`]
/// takes.
pub fn kept_atoms(tr: &rinch_editor_core::Transaction) -> Vec<rinch_editor_core::Node> {
    kept_atom_pairs(tr).into_iter().map(|(_, n)| n).collect()
}
