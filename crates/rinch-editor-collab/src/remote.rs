//! Remote → model: turn a converged CRDT change back into editor steps.
//!
//! When a peer's change arrives, yrs merges it (convergence is *its* job). This module
//! closes the loop back to the model: [`build_remote_transaction`] rebuilds the model
//! from the converged CRDT ([`CollabDoc::to_doc`](crate::CollabDoc::to_doc)) and emits a
//! minimal **block-level** `ReplaceStep` (common-prefix/suffix on blocks, so untouched
//! blocks keep their identity). Because the model is rebuilt from the *same* CRDT both
//! peers converge to, `model ≡ project(model)` is restored exactly — no position-math
//! risk.
//!
//! There is deliberately no engine type in this file. The convergence-critical path only
//! ever needs "give me the converged document as a `Node`", which is why swapping the
//! CRDT engine (#190) left it untouched.
//!
//! A surgical, cursor-preserving translation (a remote change described as
//! insert/delete/mark ops, so a caret could be re-anchored instead of re-derived) used
//! to sit alongside this and was **dropped with automerge**: it consumed
//! `automerge::Patch`, was documented as not convergence-critical, and never fed the
//! session. It can be rebuilt on yrs observer deltas (`TextRef::observe` → `TextEvent`)
//! if a future refinement wants it.

use rinch_editor_core::{EditorState, Fragment, Node, Selection, Slice, Transaction};

use crate::error::Result;

/// Transaction meta key marking a transaction as a remote (collab) application, so the
/// history plugin and any origin-sensitive logic can tell it apart from local typing.
pub const ORIGIN_REMOTE: &str = "collabOriginRemote";

/// Build the transaction that brings `state`'s model up to the converged CRDT
/// document `target`, as a minimal block-level replace. Returns `None` if nothing
/// changed. The transaction is tagged remote + non-undoable.
pub fn build_remote_transaction(state: &EditorState, target: &Node) -> Result<Option<Transaction>> {
    let old = &state.doc;
    let on = old.child_count();
    let nn = target.child_count();

    // Common leading/trailing blocks by structural equality (different trees, so no
    // Rc fast-path here).
    let mut prefix = 0;
    while prefix < on && prefix < nn && old.child(prefix) == target.child(prefix) {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < on - prefix
        && suffix < nn - prefix
        && old.child(on - 1 - suffix) == target.child(nn - 1 - suffix)
    {
        suffix += 1;
    }
    if prefix == on && nn == on {
        return Ok(None); // identical
    }

    // Model position before the first changed block, and after the last changed block.
    let start: usize = (0..prefix).map(|j| old.child(j).node_size()).sum();
    let end: usize = (0..on - suffix).map(|j| old.child(j).node_size()).sum();

    // The replacement blocks (the changed middle of the new doc).
    let mid: Vec<Node> = (prefix..nn - suffix)
        .map(|j| target.child(j).clone())
        .collect();
    let slice = Slice::new(Fragment::from_children(mid), 0, 0);

    let mut tr = state.tr();
    tr.replace(start, end, slice)?;
    // Re-anchor selection to a safe spot in the new doc (mapping already shifted it;
    // `near` guarantees a valid text position even if the old block was replaced).
    let head = tr.selection().head();
    let new_doc = tr.doc().clone();
    tr.set_selection(Selection::near(&new_doc, head, 1));
    tr.set_add_to_history(false);
    tr.set_meta(ORIGIN_REMOTE, true);
    Ok(Some(tr))
}
