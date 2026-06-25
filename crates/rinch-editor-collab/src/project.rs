//! Local projection: fold a model [`Transaction`] into the Automerge CRDT.
//!
//! This is the **local** half of the `model ≡ project(model)` invariant. Rather than
//! pattern-matching each `Step`'s geometry (brittle for split/join), we project the
//! transaction's *net* effect (`before → after`) as a **block-list diff**:
//!
//! 1. The unchanged leading/trailing blocks are skipped by **`Rc` identity**
//!    ([`Node::same_ref`]) — the persistent model shares every untouched block, so
//!    typing one character reads and re-splices exactly one block.
//! 2. Each changed block is reconciled in place ([`CollabDoc::reconcile_block`]) with a
//!    minimal common-prefix/suffix text splice, so the per-character CRDT identity of
//!    unchanged text survives and concurrent edits merge.
//! 3. The net block-count difference is inserted/deleted at the boundary, preserving
//!    the identity of the leading reconciled blocks (so a split keeps block N's text
//!    object and only *adds* the tail block).
//!
//! Any non-flat node anywhere in `before` or `after` fails loud
//! ([`CollabError::Unsupported`], design A22).

use rinch_editor_core::{Node, Transaction};

use crate::error::Result;
use crate::projection::{CollabDoc, read_block};

impl CollabDoc {
    /// Project a freshly-applied local transaction onto the CRDT. A no-op for a
    /// transaction that changed no content (a bare selection move).
    pub fn project_transaction(&mut self, tr: &Transaction) -> Result<()> {
        if tr.steps().is_empty() {
            return Ok(());
        }
        // docs()[0] is the document before the first step — i.e. before the whole
        // transaction; doc() is the result.
        let before = &tr.docs()[0];
        let after = tr.doc();
        self.project_change(before, after)
    }

    /// Project an arbitrary `before → after` document change (also the building block
    /// for loading content). Both must be flat documents.
    pub fn project_change(&mut self, before: &Node, after: &Node) -> Result<()> {
        let bn = before.child_count();
        let an = after.child_count();

        // Unchanged leading blocks (Rc identity — O(1) per block).
        let mut prefix = 0;
        while prefix < bn && prefix < an && before.child(prefix).same_ref(after.child(prefix)) {
            prefix += 1;
        }
        // Unchanged trailing blocks.
        let mut suffix = 0;
        while suffix < bn - prefix
            && suffix < an - prefix
            && before
                .child(bn - 1 - suffix)
                .same_ref(after.child(an - 1 - suffix))
        {
            suffix += 1;
        }

        let pre_mid = bn - prefix - suffix; // changed pre blocks
        let post_mid = an - prefix - suffix; // changed post blocks
        let common = pre_mid.min(post_mid);

        // Reconcile the overlapping changed blocks in place (keeps identity).
        for k in 0..common {
            let target = read_block(after.child(prefix + k))?;
            self.reconcile_block(prefix + k, &target)?;
        }
        // Insert the extra post blocks (e.g. the tail of a split).
        for k in common..post_mid {
            let target = read_block(after.child(prefix + k))?;
            self.insert_block(prefix + k, &target)?;
        }
        // Delete the extra pre blocks (e.g. a join, or a block deletion). `common` is
        // the min, so at most one of insert/delete runs; when this runs the CRDT
        // indices still match `before`'s. Delete from the end so earlier indices stay
        // valid.
        for idx in (prefix + common..prefix + pre_mid).rev() {
            // Validate the deleted block was itself flat (fail loud, not silent).
            let _ = read_block(before.child(idx))?;
            self.delete_block(idx)?;
        }
        Ok(())
    }
}
