//! Remote → model: turn a converged CRDT change back into editor steps.
//!
//! When a peer's change arrives, Automerge merges it (convergence is *its* job). This
//! module closes the loop back to the model two ways:
//!
//! * [`CollabDoc::patches_to_remote_ops`] translates the Automerge `diff` patches into
//!   the salvaged high-level op shape (`InsertText`/`DeleteRange`/`SetBlockType`/
//!   `AddMark`/`RemoveMark` — design §8, salvaging `patches_to_ce_ops`), expressed in
//!   the model's char-based position space. This is the surgical, cursor-preserving
//!   translation and is unit-tested directly.
//! * [`build_remote_transaction`] is the **convergence-critical** path the session uses:
//!   it rebuilds the model from the converged CRDT ([`CollabDoc::to_doc`]) and emits a
//!   minimal **block-level** `ReplaceStep` (common-prefix/suffix on blocks, so untouched
//!   blocks keep their identity). Because the model is rebuilt from the *same* CRDT both
//!   peers converge to, `model ≡ project(model)` is restored exactly — no position-math
//!   risk. The op translator above is the documented refinement toward full surgery.

use rinch_editor_core::{EditorState, Fragment, Node, Selection, Slice, Transaction};

use crate::error::Result;
use crate::projection::CollabDoc;
use crate::sync::ChangeHash;
use automerge::{Patch, PatchAction, ReadDoc, ScalarValue, Value};

/// Transaction meta key marking a transaction as a remote (collab) application, so the
/// history plugin and any origin-sensitive logic can tell it apart from local typing.
pub const ORIGIN_REMOTE: &str = "collabOriginRemote";

/// A high-level description of one remote change, in the model's char position space.
/// (Salvaged shape of the old `CeRemoteOp`, rebound onto depth-aware positions.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteOp {
    /// Insert `text` at a model position.
    InsertText { pos: usize, text: String },
    /// Delete a model range.
    DeleteRange { start: usize, end: usize },
    /// A block's type changed (block index + new type name).
    SetBlockType { block: usize, type_name: String },
    /// Add a mark over a model range.
    AddMark {
        start: usize,
        end: usize,
        mark: String,
    },
    /// Remove a mark over a model range.
    RemoveMark {
        start: usize,
        end: usize,
        mark: String,
    },
}

impl CollabDoc {
    /// The model position just inside the start of block `index` (after its open token).
    /// Fails loud if any preceding block is missing its map or text object, rather than
    /// treating it as length 0 — a silent under-count would misalign every later patch
    /// position and break convergence.
    fn block_content_start(&self, index: usize) -> Result<usize> {
        let mut pos = 0usize;
        for j in 0..index {
            let b = self
                .block_obj(j)
                .ok_or_else(|| crate::CollabError::schema(format!("block {j} is missing")))?;
            let len = self
                .block_text_obj(&b)
                .map(|t| self.doc.length(&t))
                .ok_or_else(|| crate::CollabError::schema(format!("block {j} has no text")))?;
            pos += 2 + len; // open + content + close
        }
        Ok(pos + 1) // step inside the block's open token
    }

    /// Find the block index whose `text` object is `obj`.
    fn block_of_text(&self, obj: &automerge::ObjId) -> Option<usize> {
        for j in 0..self.block_count() {
            if let Some(b) = self.block_obj(j)
                && self.block_text_obj(&b).as_ref() == Some(obj)
            {
                return Some(j);
            }
        }
        None
    }

    /// Translate Automerge diff patches into model-space [`RemoteOp`]s. Positions are
    /// computed against the *current* (post-merge) CRDT structure.
    ///
    /// **Scope:** this covers in-block changes — text insert/delete, marks, and a
    /// block's `type` — only. **Structural** patches (a block *inserted into* or
    /// *deleted from* the content list — i.e. a split, join, or paste) are deliberately
    /// not represented, because [`RemoteOp`] has no block-structure variant; they are
    /// silently skipped here. The session's convergence-critical path
    /// ([`CollabSession::integrate_incremental`](crate::CollabSession::integrate_incremental))
    /// does **not** use this translator — it rebuilds from the converged CRDT
    /// ([`CollabDoc::to_doc`]), which handles every change including structural ones.
    /// Use this only for a surgical, text/mark-level view of a change.
    pub fn patches_to_remote_ops(&self, patches: &[Patch]) -> Result<Vec<RemoteOp>> {
        let mut ops = Vec::new();
        for patch in patches {
            match &patch.action {
                PatchAction::SpliceText { index, value, .. } => {
                    if let Some(block) = self.block_of_text(&patch.obj) {
                        let text = value.make_string();
                        if !text.is_empty() {
                            ops.push(RemoteOp::InsertText {
                                pos: self.block_content_start(block)? + index,
                                text,
                            });
                        }
                    }
                }
                PatchAction::DeleteSeq { index, length } => {
                    if let Some(block) = self.block_of_text(&patch.obj) {
                        let start = self.block_content_start(block)? + index;
                        ops.push(RemoteOp::DeleteRange {
                            start,
                            end: start + length,
                        });
                    }
                }
                PatchAction::Mark { marks } => {
                    if let Some(block) = self.block_of_text(&patch.obj) {
                        let base = self.block_content_start(block)?;
                        for m in marks {
                            let (start, end) = (base + m.start, base + m.end);
                            let mark = m.name().to_string();
                            if m.value() == &ScalarValue::Null {
                                ops.push(RemoteOp::RemoveMark { start, end, mark });
                            } else {
                                ops.push(RemoteOp::AddMark { start, end, mark });
                            }
                        }
                    }
                }
                PatchAction::PutMap { key, value, .. } => {
                    if key == "type"
                        && let Some(block) = self.block_of_content_map(&patch.obj)
                        && let Value::Scalar(s) = &value.0
                        && let ScalarValue::Str(smol) = s.as_ref()
                    {
                        ops.push(RemoteOp::SetBlockType {
                            block,
                            type_name: smol.to_string(),
                        });
                    }
                }
                // Structural content-list patches (block insert/delete) and other
                // actions are intentionally not represented — see the doc comment.
                _ => {}
            }
        }
        Ok(ops)
    }

    /// Find the block index whose map object is `obj` (for block-type patches).
    fn block_of_content_map(&self, obj: &automerge::ObjId) -> Option<usize> {
        (0..self.block_count()).find(|&j| self.block_obj(j).as_ref() == Some(obj))
    }

    /// The high-level remote ops describing how the CRDT changed since `before` heads.
    /// The surgical, cursor-preserving view of a remote change (the documented
    /// refinement of the session's converged-rebuild path). Text/mark/block-type
    /// changes only — see [`CollabDoc::patches_to_remote_ops`] for the scope bound.
    pub fn remote_ops_since(&mut self, before: &[ChangeHash]) -> Result<Vec<RemoteOp>> {
        let after = self.get_heads();
        let patches = self.diff(before, &after);
        self.patches_to_remote_ops(&patches)
    }
}

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
