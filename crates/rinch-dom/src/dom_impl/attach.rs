//! Attaching a Taffy child at a position, for the `DomTree` mutations that place one.

use crate::node::RawNodeId;

use super::RinchDocument;

impl RinchDocument {
    /// Attach `child_taffy` at `index` in `parent_taffy`'s Taffy child list,
    /// **reporting** a structural failure instead of swallowing it (#477).
    ///
    /// All three DOM mutations that place a child at a position — `insert_before`,
    /// `insert_child` and `replace_node` — used to spell this
    /// `let _ = taffy.insert_child_at_index(..)`. Taffy rejects an index past
    /// the end (`ChildIndexOutOfBounds`), and discarding that turned a
    /// DOM↔Taffy structural divergence into an invisible one: the child was
    /// **never attached**, so it was never laid out and never painted, with no
    /// error anywhere. `insert_child` was the worst of the three — it had no
    /// `add_child` fallback at all, so even a plain append failed that way.
    ///
    /// A Taffy structural error is never benign: it means the two trees
    /// disagree about a shape one of them has already accepted. Three things
    /// happen here instead of nothing. The index is **clamped** into range, so
    /// a wrong index can misplace a node but can never fail to attach it; a
    /// clamp, or a refusal that survives the clamp, is `warn!`-logged naming
    /// both nodes; and [`crate::node::NodeTree::taffy_attach_faults`] counts
    /// it, so the divergence is observable to a test or a devtools surface
    /// without scraping logs.
    ///
    /// **What the clamp is worth, measured against taffy 0.12.2 (and re-read
    /// against 0.14.0, unchanged) rather than assumed.** `insert_child_at_index` returns `Err` for exactly one reason,
    /// `ChildIndexOutOfBounds`; its only other fallible call is `mark_dirty`,
    /// which always answers `Ok`. So the clamp does not merely *usually* avoid
    /// the error — it makes this call **total**, and with
    /// [`Self::compute_taffy_child_index`] deriving an in-range index by
    /// construction, `taffy_attach_faults` is *provably* zero in shipped code
    /// rather than hopefully zero. That is the intended tripwire semantics: a
    /// non-zero count means a regression of this class has been reintroduced.
    ///
    /// **The `add_child` fallback is therefore unreachable today, and it does
    /// not do what an earlier draft of this comment claimed.** It does *not*
    /// rescue "a parent Taffy no longer holds": that input does not produce an
    /// error at all, it **panics** inside Taffy (`invalid SlotMap key used`),
    /// and `add_child` panics identically on it, since both index the same slot
    /// maps. It is kept as defence in depth against one thing only — a future
    /// Taffy growing a second error variant here, which the clamp would not
    /// cover — and on that day "attached in the wrong place" still beats
    /// "attached nowhere", which is the state that paints no pixels and reports
    /// nothing. Do not read it as covering anything else.
    pub(crate) fn attach_taffy_child_at(
        &mut self,
        parent_taffy: taffy::NodeId,
        index: usize,
        child_taffy: taffy::NodeId,
        parent_id: RawNodeId,
        child_id: RawNodeId,
    ) {
        debug_assert!(
            self.tree.taffy.parent(child_taffy).is_none(),
            "rinch-dom: attaching node {child_id} under node {parent_id} while another \
             Taffy list still holds it — detach from `taffy.parent()` first, or the \
             tree carries a double-claim (#591)"
        );
        let len = self
            .tree
            .taffy
            .children(parent_taffy)
            .map(|c| c.len())
            .unwrap_or(0);
        let index = if index > len {
            tracing::warn!(
                "rinch-dom: Taffy child index {index} is out of range for parent \
                 node {parent_id} <{parent_tag}>, whose Taffy child list holds \
                 {len}; clamped to {len} while attaching node {child_id} \
                 <{child_tag}>. The DOM and the Taffy tree have diverged (#477).",
                // `get`, not index: a diagnostic must not be the thing that
                // panics, and a text node has no tag at all.
                parent_tag = self
                    .tree
                    .nodes
                    .get(parent_id)
                    .and_then(|n| n.tag())
                    .unwrap_or("?"),
                child_tag = self
                    .tree
                    .nodes
                    .get(child_id)
                    .and_then(|n| n.tag())
                    .unwrap_or("?"),
            );
            self.tree.taffy_attach_faults += 1;
            len
        } else {
            index
        };
        if let Err(err) = self
            .tree
            .taffy
            .insert_child_at_index(parent_taffy, index, child_taffy)
        {
            tracing::warn!(
                "rinch-dom: Taffy refused to attach node {child_id} at index \
                 {index} under parent node {parent_id}: {err:?}. Falling back to \
                 append — the child is attached in the wrong place rather than \
                 not at all (#477).",
            );
            self.tree.taffy_attach_faults += 1;
            if let Err(err) = self.tree.taffy.add_child(parent_taffy, child_taffy) {
                tracing::warn!(
                    "rinch-dom: the append fallback for node {child_id} under \
                     parent node {parent_id} failed too: {err:?}. The node is \
                     attached nowhere and will not be laid out or painted (#477).",
                );
            }
        }
    }
}

#[cfg(test)]
mod attach_taffy_child_tests {
    use super::*;
    use rinch_core::dom::DomTree;

    /// The clamp-and-report path (#477): an index past the end of the parent's
    /// Taffy child list attaches the child at the end, counts a fault and logs
    /// it — rather than dropping the child on the floor, which is what
    /// `let _ = taffy.insert_child_at_index(..)` did at all three call sites.
    ///
    /// The helper is exercised directly because the callers can no longer
    /// reach this branch: [`RinchDocument::compute_taffy_child_index`] now
    /// derives the index from the parent's live child list, so it is in range
    /// by construction. Unreachable-from-there is the *state* the tripwire
    /// exists for, not a reason to leave it untested.
    ///
    /// Kills: counting nothing, i.e. returning to a silent structural failure;
    /// and dropping the clamp **and** the `add_child` fallback together, which
    /// loses the child. Measured, not assumed: dropping *either one alone*
    /// leaves this test green, because for an out-of-range index they are
    /// observationally the same repair — clamping to `len` **is** appending, so
    /// with the clamp Taffy accepts the insert and with only the fallback it
    /// refuses and the append happens anyway. Nothing here can distinguish
    /// them, and nothing here claims to. They are not interchangeable, but the
    /// asymmetry runs the *other* way from what an earlier draft said: the
    /// clamp is what makes the call total (`ChildIndexOutOfBounds` is the only
    /// error Taffy can return here), and the fallback is the unreachable one.
    #[test]
    fn an_out_of_range_attach_is_clamped_reported_and_still_attached() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let first = doc.create_element("div");
        doc.append_child(body, first);
        let stray = doc.create_element("div");

        let parent_taffy = doc.tree.nodes[body.0].taffy_id.unwrap();
        let first_taffy = doc.tree.nodes[first.0].taffy_id.unwrap();
        let stray_taffy = doc.tree.nodes[stray.0].taffy_id.unwrap();
        assert_eq!(
            doc.tree.taffy.children(parent_taffy).unwrap().len(),
            1,
            "precondition: one attached child, so index 99 is out of range"
        );
        assert_eq!(
            doc.tree.taffy_attach_faults, 0,
            "precondition: no faults yet"
        );

        doc.attach_taffy_child_at(parent_taffy, 99, stray_taffy, body.0, stray.0);

        assert_eq!(
            doc.tree.taffy_attach_faults, 1,
            "an out-of-range index is a DOM↔Taffy divergence and must be \
             reported, not swallowed"
        );
        assert_eq!(
            doc.tree.taffy.children(parent_taffy).unwrap(),
            vec![first_taffy, stray_taffy],
            "and the child must be attached anyway, at the clamped position — \
             attached in the wrong place beats attached nowhere"
        );
    }

    /// The ordinary leg is exact and silent: the child lands at the index it
    /// was given, and nothing is reported.
    ///
    /// Kills: a clamp that fires one slot early (`index >= len` rather than
    /// `index > len`), which would turn every legitimate append into a
    /// reported fault and make the counter useless as a divergence signal.
    #[test]
    fn an_in_range_attach_is_exact_and_silent() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let a = doc.create_element("div");
        doc.append_child(body, a);
        let b = doc.create_element("div");
        doc.append_child(body, b);
        let mid = doc.create_element("div");

        let parent_taffy = doc.tree.nodes[body.0].taffy_id.unwrap();
        let a_taffy = doc.tree.nodes[a.0].taffy_id.unwrap();
        let b_taffy = doc.tree.nodes[b.0].taffy_id.unwrap();
        let mid_taffy = doc.tree.nodes[mid.0].taffy_id.unwrap();

        // Index 1 of a two-element list: strictly interior, so neither
        // endpoint can stand in for it.
        doc.attach_taffy_child_at(parent_taffy, 1, mid_taffy, body.0, mid.0);

        assert_eq!(
            doc.tree.taffy.children(parent_taffy).unwrap(),
            vec![a_taffy, mid_taffy, b_taffy]
        );
        assert_eq!(
            doc.tree.taffy_attach_faults, 0,
            "an in-range insert is not a divergence and must report nothing"
        );

        // And `index == len` is the last legal index, not the first illegal one.
        let tail = doc.create_element("div");
        let tail_taffy = doc.tree.nodes[tail.0].taffy_id.unwrap();
        doc.attach_taffy_child_at(parent_taffy, 3, tail_taffy, body.0, tail.0);
        assert_eq!(
            doc.tree.taffy.children(parent_taffy).unwrap(),
            vec![a_taffy, mid_taffy, b_taffy, tail_taffy]
        );
        assert_eq!(doc.tree.taffy_attach_faults, 0);
    }
}
