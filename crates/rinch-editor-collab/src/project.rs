//! Local projection: fold a model [`Transaction`] into the CRDT.
//!
//! This is the **local** half of the `model ≡ project(model)` invariant. Rather than
//! pattern-matching each `Step`'s geometry (brittle for split/join), we project the
//! transaction's *net* effect (`before → after`) as a **block-list diff**:
//!
//! 1. The unchanged leading/trailing blocks are skipped by **`Rc` identity**
//!    ([`Node::same_ref`]) — the persistent model shares every untouched block, so
//!    typing one character reads and re-splices exactly one block.
//! 2. Each changed block is reconciled in place (`reconcile_node`) with a minimal
//!    common-prefix/suffix text splice, so the per-character CRDT identity of
//!    unchanged text survives and concurrent edits merge.
//! 3. The net block-count difference is inserted/deleted at the boundary, preserving
//!    the identity of the leading reconciled blocks (so a split keeps block N's text
//!    object and only *adds* the tail block).
//!
//! A changed top-level *list* reconciles recursively (same diff, one level down), so a
//! keystroke inside one list item re-splices only that item's text object. Any node
//! outside the supported scope (a non-list nested block, an inline atom) anywhere in
//! `before` or `after` fails loud ([`CollabError::Unsupported`], design A22).

use yrs::{Array, Transact};

use rinch_editor_core::{Node, Transaction};

use crate::error::Result;
use crate::projection::{CollabDoc, check_index, insert_node, read_node, reconcile_node};

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
    /// for loading content). Both must be within the projected scope.
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

        // Validation pre-pass (design A22 fail-loud must be all-or-nothing): read and
        // validate EVERY block this change will touch — the `after` blocks to
        // reconcile/insert, and the `before` blocks to delete — BEFORE issuing any
        // CRDT write, and before the write transaction is even opened. A mixed
        // in-scope/out-of-scope change (e.g. pasting or loading a table while
        // collaborating) then leaves the CRDT *exactly* at the prior converged state
        // and returns `Unsupported`, instead of partially mutating it and wedging the
        // session in a half-projected state.
        //
        // Under yrs this ordering also buys **panic safety**, not just atomicity: a
        // yrs text/array index out of range panics where automerge returned an error,
        // so no write may be attempted until the whole change is known projectable.
        let mut targets = Vec::with_capacity(post_mid);
        for k in 0..post_mid {
            targets.push(read_node(after.child(prefix + k))?);
        }
        for idx in prefix + common..prefix + pre_mid {
            // Validate (and discard) each block being deleted — fail loud if out of scope.
            read_node(before.child(idx))?;
        }

        // Writes — one transaction for the whole change.
        //
        // The all-or-nothing guarantee the pre-pass buys covers the **content-scope**
        // class only: no out-of-scope node can reach a write. An error raised from here on
        // — a corrupt-CRDT read-back, a bounds check — still commits whatever this
        // transaction wrote before it, because yrs has no rollback. Extending the pre-pass
        // to cover that class too is tracked separately.
        let content = self.content.clone();
        let mut txn = self.doc.transact_mut();
        // Reconcile the overlapping changed blocks in place (keeps identity). A changed
        // top-level list reconciles recursively, touching only the edited descendant.
        for (k, target) in targets.iter().take(common).enumerate() {
            reconcile_node(&mut txn, &content, (prefix + k) as u32, target)?;
        }
        // Insert the extra post blocks (e.g. the tail of a split). `targets` has
        // exactly `post_mid` entries, so skipping `common` yields indices
        // `common..post_mid`.
        for (k, target) in targets.iter().enumerate().skip(common) {
            insert_node(&mut txn, &content, (prefix + k) as u32, target)?;
        }
        // Delete the extra pre blocks (e.g. a join, or a block deletion). `common` is
        // the min, so at most one of insert/delete runs; when this runs the CRDT
        // indices still match `before`'s. Delete from the end so earlier indices stay
        // valid.
        for idx in (prefix + common..prefix + pre_mid).rev() {
            check_index(&txn, &content, idx as u32, false)?;
            content.remove(&mut txn, idx as u32);
        }
        Ok(())
    }
}
