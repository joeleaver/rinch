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
//!
//! The diff trusts `before` to describe what the CRDT holds — which is the invariant —
//! but it **verifies** that trust before writing (issue #194): the pre-pass checks the
//! model/CRDT block counts match and reads back every CRDT node the write phase will
//! read, so a mismatch in either direction — a stray CRDT block the model lacks, or a
//! model block the CRDT lacks — fails loud with the CRDT untouched, never a partial
//! commit and never a silent divergence. The **one** exception the diff handles itself:
//! a CRDT holding zero blocks (issue #192) has no model that mirrors it, so `before`
//! carries a starter paragraph the CRDT does not. That case skips the diff entirely and
//! inserts the whole of `after`.

use yrs::{Array, Transact};

use rinch_editor_core::{Node, Transaction};

use crate::error::{CollabError, Result};
use crate::projection::{
    CollabDoc, check_index, insert_node, read_node, read_node_data, reconcile_node,
};

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
        // A CRDT holding zero blocks is a legitimate converged state (issue #192: two
        // peers deleting *different* blocks concurrently deletes every block), and the
        // model cannot mirror it — the schema requires at least one block, so `to_doc`
        // hands the editor a starter paragraph the CRDT does not contain. `before`
        // therefore describes a block that is not there, and diffing against it would try
        // to reconcile a node the CRDT lacks (`reconcile_node: missing node`, which used
        // to wedge the session permanently: nothing projected, nothing broadcast, no
        // recovery). With no blocks in the CRDT every block of `after` is an insert, and
        // that restores `model ≡ project(model)` exactly.
        let crdt_blocks = self.content.len(&self.doc.transact());
        if crdt_blocks == 0 {
            return self.project_whole_doc(after);
        }

        let bn = before.child_count();
        let an = after.child_count();

        // Pre-pass gate 1 (issue #194): `before` must describe what the CRDT holds —
        // starting with how many blocks there are. Without this check a mismatch either
        // partially commits (model ahead: the delete/reconcile loop runs off the end of
        // the CRDT *after* earlier blocks were already reconciled, and yrs has no
        // rollback) or silently diverges (CRDT ahead: the diff reconciles the blocks the
        // model knows about and leaves the stray one, returning `Ok` — issue #192 review
        // finding 5). Both directions are the same broken invariant, so both fail loud
        // here, before any write. The zero-block CRDT is the one legitimate mismatch and
        // returned above.
        if crdt_blocks as usize != bn {
            return Err(CollabError::schema(format!(
                "model/CRDT out of step: the model document holds {bn} block(s) but the \
                 CRDT holds {crdt_blocks}"
            )));
        }

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

        // Pre-pass gate 2, model side (design A22 fail-loud must be all-or-nothing):
        // read and validate EVERY model block this change will touch — the `after`
        // blocks to reconcile/insert, and the `before` blocks to delete — BEFORE
        // issuing any CRDT write, and before the write transaction is even opened. A
        // mixed in-scope/out-of-scope change (e.g. pasting or loading a table while
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

        // Pre-pass gate 3, CRDT side (issue #194): read back every CRDT node the write
        // phase will read — the blocks being reconciled in place, recursively (a
        // container's whole subtree, exactly what `reconcile_node` walks). This
        // surfaces every CRDT-side failure the model-side gates cannot see — an embed
        // parked in a block's text, a corrupt mark value, a node that is neither
        // text-block nor container — before the first write. Blocks being *deleted*
        // are not read back: removal never reads their content. The read transaction
        // is scoped so it drops before `transact_mut` opens (yrs cannot nest
        // transaction acquisitions).
        {
            let txn = self.doc.transact();
            for k in 0..common {
                read_node_data(&txn, &self.content, (prefix + k) as u32)?;
            }
        }

        // Writes — one transaction for the whole change, which yrs commits on drop:
        // there is no rollback primitive, so the pre-pass above is the *only*
        // all-or-nothing lever. It is sufficient: gate 1 pinned the block count (so
        // every index below is in range for both the insert and delete loops), gates
        // 2–3 read back everything this phase reads (model and CRDT side), and a
        // splice/format write cannot invent content a read-back would reject. The
        // error returns below are therefore unreachable for a change the pre-pass
        // admitted; they stay as defense in depth against a projection bug, not as a
        // supported failure path.
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

    /// Insert every block of `doc` into a CRDT that holds **no** blocks — the recovery
    /// path out of the zero-block state (see [`CollabDoc::project_change`]). There is
    /// nothing to diff against, so this is an unconditional whole-document insert, which
    /// leaves the CRDT projecting to exactly `doc`.
    ///
    /// The A22 pre-pass applies here too: validate every node before opening the write
    /// transaction, so an out-of-scope document leaves the (empty) CRDT untouched.
    fn project_whole_doc(&mut self, doc: &Node) -> Result<()> {
        let mut nodes = Vec::with_capacity(doc.child_count());
        for i in 0..doc.child_count() {
            nodes.push(read_node(doc.child(i))?);
        }
        let content = self.content.clone();
        let mut txn = self.doc.transact_mut();
        for (i, nd) in nodes.iter().enumerate() {
            insert_node(&mut txn, &content, i as u32, nd)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //! The all-or-nothing pre-pass, CRDT side (issue #194). yrs transactions commit on
    //! drop with no rollback, so a write-phase failure would leave the CRDT — and the
    //! next broadcast delta — holding a partial edit. Each test therefore asserts three
    //! things: the error is loud, the CRDT bytes are untouched, and the outbox holds no
    //! delta for a peer to receive. The model-side twin of these pins lives in
    //! `tests/collab.rs` (`unsupported_change_does_not_partially_mutate`).

    use std::rc::Rc;

    use yrs::{Any, Array, Map, Out, Text, Transact};

    use rinch_editor_core::{Fragment, Node, Schema};

    use crate::error::CollabError;
    use crate::projection::CollabDoc;

    fn schema() -> Rc<Schema> {
        Rc::new(Schema::starter_kit())
    }

    fn para(s: &Schema, text: &str) -> Node {
        let content = if text.is_empty() {
            Fragment::empty()
        } else {
            Fragment::from_node(s.text(text).unwrap())
        };
        s.branch("paragraph", content).unwrap()
    }

    fn doc_of(s: &Schema, blocks: Vec<Node>) -> Node {
        s.branch("doc", Fragment::from_children(blocks)).unwrap()
    }

    /// Drain the outbox, then capture the full CRDT state — the "nothing may change
    /// past this point" baseline the assertions below compare against.
    fn baseline(cdoc: &mut CollabDoc) -> Vec<u8> {
        let _ = cdoc.take_outbox().unwrap();
        cdoc.save()
    }

    /// The three-part all-or-nothing assertion: the CRDT state is byte-for-byte what it
    /// was before the failed projection, and no broadcast delta escaped.
    fn assert_untouched(cdoc: &mut CollabDoc, baseline: &[u8], what: &str) {
        assert_eq!(
            cdoc.save(),
            baseline,
            "{what}: the CRDT must be byte-for-byte untouched"
        );
        assert!(
            cdoc.take_outbox().unwrap().is_empty(),
            "{what}: no broadcast delta may carry a partial edit"
        );
    }

    #[test]
    fn an_embed_found_in_the_crdt_mid_change_does_not_partially_mutate() {
        // Repro (a) of issue #194: the model-side pre-pass cannot see an embed parked
        // in the *CRDT's* copy of a block's text — only `reconcile_text`'s read-back
        // hits it. Before the CRDT-side pre-pass, that read-back happened during the
        // write phase, after block 0's new text "ALPHA" had already been written; the
        // transaction committed on drop and the partial edit shipped in the next delta.
        let s = schema();
        let before = doc_of(&s, vec![para(&s, "alpha"), para(&s, "beta")]);
        let mut cdoc = CollabDoc::from_doc(&before).unwrap();

        // Park an embed inside block 1's text, the way a foreign/corrupt writer could.
        {
            let mut txn = cdoc.doc.transact_mut();
            let Some(Out::YMap(node)) = cdoc.content.get(&txn, 1) else {
                panic!("block 1 must be a node map");
            };
            let Some(Out::YText(text)) = node.get(&txn, "text") else {
                panic!("block 1 must carry a text");
            };
            text.insert_embed(&mut txn, 2, Any::Bool(true));
        }
        let base = baseline(&mut cdoc);

        // Both blocks change, so the write phase would reconcile block 0 ("ALPHA")
        // before discovering the embed in block 1.
        let after = doc_of(&s, vec![para(&s, "ALPHA"), para(&s, "BETA")]);
        let err = cdoc.project_change(&before, &after).unwrap_err();
        assert!(
            matches!(err, CollabError::Unsupported(_)),
            "the embed must fail loud as Unsupported, got {err:?}"
        );
        assert_untouched(&mut cdoc, &base, "embed in CRDT text");
    }

    #[test]
    fn a_model_block_the_crdt_lacks_fails_before_any_write() {
        // Repro (b) of issue #194: the model believes in a block the CRDT does not
        // hold. Before the count gate this surfaced as `Schema("content index 2 out of
        // range")` from the write phase's own bounds check — with block 0 already
        // reconciled and committed.
        let s = schema();
        let crdt_doc = doc_of(&s, vec![para(&s, "alpha"), para(&s, "beta")]);
        let mut cdoc = CollabDoc::from_doc(&crdt_doc).unwrap();
        let base = baseline(&mut cdoc);

        let before = doc_of(
            &s,
            vec![para(&s, "alpha"), para(&s, "beta"), para(&s, "gamma")],
        );
        let after = doc_of(&s, vec![para(&s, "ALPHA"), para(&s, "beta")]);
        let err = cdoc.project_change(&before, &after).unwrap_err();
        assert!(
            matches!(err, CollabError::Schema(_)),
            "a model/CRDT count mismatch must fail loud as Schema, got {err:?}"
        );
        assert_untouched(&mut cdoc, &base, "model ahead of CRDT");
    }

    #[test]
    fn a_stray_crdt_block_the_model_lacks_fails_loud_not_silently() {
        // Issue #192 review finding 5, the mirror image of the case above: the CRDT
        // holds a block the model does not know about. This used to return `Ok`,
        // reconcile the blocks the model *did* know, and leave the stray block behind —
        // silent divergence (`model ≠ project(model)`) with no error at all, the exact
        // class the fail-loud design exists to kill.
        let s = schema();
        let crdt_doc = doc_of(&s, vec![para(&s, "solo"), para(&s, "stray")]);
        let mut cdoc = CollabDoc::from_doc(&crdt_doc).unwrap();
        let base = baseline(&mut cdoc);

        let before = doc_of(&s, vec![para(&s, "solo")]);
        let after = doc_of(&s, vec![para(&s, "SOLO")]);
        let err = cdoc.project_change(&before, &after).unwrap_err();
        assert!(
            matches!(err, CollabError::Schema(_)),
            "a stray CRDT block must fail loud as Schema, got {err:?}"
        );
        assert_untouched(&mut cdoc, &base, "CRDT ahead of model");
    }
}
