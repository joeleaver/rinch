use super::*;

/// #535 review, white-box: directly inspects the cached `flow_extent` for
/// a box clipping x alone (open y) before and after it is scrolled, to
/// prove `NodeTree::mark_scrolled`'s per-axis `extent_reads_scroll`
/// (`!clips_overflow_x() || !clips_overflow_y()`) actually drops the stale
/// cached extent rather than merely being plausible. Reading `hit_test`'s
/// final answer instead of `flow_extent` directly would not discriminate
/// here — the production hit-test walk has another, uncached route to the
/// right answer, so a caching bug can hide behind a correct final result.
///
/// An ordinary (non-positioned) sibling spacer pushes the target child
/// down in normal flow, avoiding both margin-collapse-with-parent and the
/// absolute-hoist path (which `flow_extent` folds in through a different
/// route and never reaches here).
///
/// The mutant this kills: reverting `mark_scrolled`'s
/// `extent_reads_scroll` to the pre-#535 `!node.clips_overflow()` (both
/// axes) leaves the cache believing this box's extent cannot depend on
/// its own scroll offset, so after scrolling back to 0 the second
/// `flow_extent` call returns the stale scrolled-to-300 value instead of
/// recomputing — `after` comes back `[0.0, -300.0, 100.0, 100.0]` instead
/// of `[0.0, 0.0, 100.0, 350.0]`.
#[test]
fn mark_scrolled_invalidates_flow_extent_for_a_box_clipping_only_one_axis() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "width: 100px; height: 100px; overflow-x: clip; overflow-y: visible;",
    );
    doc.append_child(body, container);
    let spacer = doc.create_element("div");
    doc.set_attribute(spacer, "style", "width: 1px; height: 300px;");
    doc.append_child(container, spacer);
    let child = doc.create_element("div");
    doc.set_attribute(child, "style", "width: 50px; height: 50px;");
    doc.append_child(container, child);
    doc.resolve_layout(800.0, 600.0);

    doc.set_scroll_top(container, 300.0);
    let fits = flow_extent(&doc.tree, container.0);
    assert_eq!(
        fits,
        [0.0, -300.0, 100.0, 100.0],
        "max-y stays at the container's own 100 while scrolled to 300 \
         (the child's bottom at 50 doesn't exceed it): {fits:?}"
    );

    doc.set_scroll_top(container, 0.0);
    let after = flow_extent(&doc.tree, container.0);
    assert_eq!(
        after,
        [0.0, 0.0, 100.0, 350.0],
        "scrolling back must recompute the extent (open y axis), not \
         reuse the stale scroll=300 value — if this fails, \
         mark_scrolled did not invalidate the cache for this \
         partially-clipping box: {after:?}"
    );
}
