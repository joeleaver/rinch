use super::extent::{flow_hoists, flow_subtree_may_contain, stacking_context_misses};
use super::paint_order::{cached_paint_order, in_relative_split_inline, phased_run_hit};
use super::probe::{DeferredScope, PlainHit, Probe, bar_under, deferred_bar_under};
use super::targets::{hides_own_hit, inline_box_outside, shows_below, visible_text_at};
use rinch_dom::stacking::paints_at_stacking_root;

/// Simple hit testing: find the deepest node whose layout rect contains (x, y).
/// Respects CSS stacking contexts so that elements with higher z-index
/// are tested before visually-behind siblings, and CSS transforms so that a
/// transformed subtree is hit where it is *painted* (#199).
pub(crate) fn hit_test(tree: &rinch_dom::NodeTree, x: f32, y: f32) -> Option<u64> {
    tree.perf.bump(rinch_dom::perf::Counter::HitTests);
    hit_test_node(&PlainHit, tree, tree.body_id, 0.0, 0.0, x, y, x, y, None)
}

/// Map `(px, py)` — a point in the coordinate space the node's layout box lives
/// in — into the node's own space by inverting the node's CSS transform.
///
/// `paint_node` draws a node's *untransformed* box under the affine composed by
/// [`rinch_dom::paint::compose_node_transform`] and hands that same affine to the
/// node's children. So the mirror image on the input side is to push the probe
/// point through the inverse: every bounds test then happens in the box's own
/// space, against the plain layout rect. Composition falls out for free — each
/// level inverts only its own step, and the product of those inverses is the
/// inverse of paint's accumulated transform.
///
/// `scale = 1.0` because hit testing works in layout pixels while paint works in
/// physical pixels. `compose_node_transform` is covariant in that argument — it
/// multiplies the CSS-px translate components by `scale`, and the linear part
/// (scale/rotate/skew) is unit-invariant — so composing at `scale = 1.0` yields
/// exactly paint's transform expressed in layout pixels, at any DPI (#202).
///
/// Returns `None` when the transform is not invertible (`scale(0)`, a degenerate
/// matrix): such a subtree paints to zero area, so nothing in it can be hit.
fn local_point(node: &rinch_dom::Node, nx: f32, ny: f32, px: f32, py: f32) -> Option<(f32, f32)> {
    use peniko::kurbo::{Affine, Point};

    // A plain inline span's transform does not apply (#1080), and paint's
    // composition skips it, so the inverse must too.
    if !node.has_applied_transform() {
        return Some((px, py));
    }
    // A display:contents node has no box, so paint gives it no transform box
    // either — `paint_node`'s zero-size branch passes the parent transform
    // straight through to the children.
    if node.computed_style.display == rinch_dom::computed_style::DisplayValue::Contents
        && (node.layout.width == 0.0 || node.layout.height == 0.0)
    {
        return Some((px, py));
    }

    let t =
        rinch_dom::paint::compose_node_transform(node, nx as f64, ny as f64, 1.0, Affine::IDENTITY);
    let det = t.determinant();
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let p = t.inverse() * Point::new(px as f64, py as f64);
    if !p.x.is_finite() || !p.y.is_finite() {
        return None;
    }
    Some((p.x as f32, p.y as f32))
}

/// One level of a top-down probe-point descent: where the node's box is, and
/// where the probe point is once it has been moved into the node's own space.
///
/// Every walk in this file that asks "what is under this point" goes through
/// [`descend`] to produce one, so the scroll searches and the scrollbar search
/// cannot drift away from `hit_test` — or from paint — the way four hand-rolled
/// copies of the arithmetic did (#203).
pub(super) struct Frame {
    /// The node's painted origin, in the space `x`/`y` are expressed in.
    pub(super) nx: f32,
    pub(super) ny: f32,
    /// The probe point in this node's own space — the space paint draws the
    /// node's untransformed box in, so a plain rect test is valid against it.
    pub(super) x: f32,
    pub(super) y: f32,
    /// The probe point in viewport space, carried down for `position: fixed`
    /// descendants.
    pub(super) vx: f32,
    pub(super) vy: f32,
}

/// Enter a node during a top-down probe-point walk, mirroring `paint_node`'s
/// own descent.
///
/// Four adjustments, in paint's order:
///
/// - a `position: fixed` box is viewport-relative, because paint gives its entry
///   zeroed offsets and the body's transform wherever it is hoisted to (#545) —
///   so the probe resumes from `vx`/`vy` and the node's own
///   `layout.x`/`layout.y` alone;
/// - an inline-block an IFC positions stores `layout.x`/`layout.y` against the
///   IFC root's *content* box while the offsets accumulated here are border-box
///   origins, so [`rinch_dom::paint::ifc_content_box_offset`] bridges the two
///   (a fixed box is exempt — paint never routes it through
///   `paint_inline_layout`);
/// - the node's CSS transform is inverted into the probe point ([`local_point`]),
///   so everything below works in the box's own space;
/// - the body re-seeds viewport space from its own post-transform point, which
///   is the space paint hands every fixed entry, whichever stacking context
///   hoisted it (`body_paint_transform`, #545).
///
/// `None` means nothing in this subtree can be under any point: the transform is
/// not invertible, so the subtree paints to zero area.
#[allow(clippy::too_many_arguments)]
pub(super) fn descend(
    tree: &rinch_dom::NodeTree,
    node: &rinch_dom::Node,
    node_id: u64,
    offset_x: f32,
    offset_y: f32,
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
) -> Option<Frame> {
    let is_fixed = node.box_position() == rinch_dom::computed_style::PositionValue::Fixed;
    let (x, y) = if is_fixed { (vx, vy) } else { (x, y) };
    let (nx, ny) = if is_fixed {
        (node.layout.x, node.layout.y)
    } else {
        let (dx, dy) = rinch_dom::paint::ifc_content_box_offset(tree, node);
        (offset_x + node.layout.x + dx, offset_y + node.layout.y + dy)
    };

    let (x, y) = local_point(node, nx, ny, x, y)?;

    // Fixed descendants resolve against the viewport, which paint models as the
    // body level — so viewport space is body's *post*-transform space.
    let (vx, vy) = if node_id == tree.body_id {
        (x, y)
    } else {
        (vx, vy)
    };

    Some(Frame {
        nx,
        ny,
        x,
        y,
        vx,
        vy,
    })
}

/// Hit-test a subtree.
///
/// `x`/`y` is the probe point in the coordinate space `offset_x`/`offset_y` live
/// in — i.e. the local (pre-transform) space of the nearest transformed ancestor,
/// which is the same space paint accumulates its offsets in. `vx`/`vy` is the
/// probe point in viewport space, kept for `position: fixed` subtrees, which
/// paint places with zeroed offsets and the body's transform however deep the
/// stacking context that owns them (#545).
///
/// `clipped` names the outermost clip on the way here that the probe point
/// is outside of, by its clipping box: one that a `position: fixed`
/// descendant escapes (#549) — a clip between this node and the nearest
/// ancestor that contains fixed descendants
/// ([`rinch_dom::paint::contains_fixed_descendants`]), or the viewport — or
/// that an absolute descendant whose containing block is above it does
/// (#2815). Paint lifts exactly those clips around such an entry, so the walk
/// goes on looking for one — through stacking contexts that hold one, and
/// nowhere else — and hits nothing else on the way.
#[allow(clippy::too_many_arguments)]
pub(super) fn hit_test_node<P: Probe>(
    probe: &P,
    tree: &rinch_dom::NodeTree,
    node_id: u64,
    offset_x: f32,
    offset_y: f32,
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    clipped: Option<u64>,
) -> Option<u64> {
    tree.perf.bump(P::VISITS);
    let node = tree.get_live(node_id)?;

    // The `position: fixed` hoist, the IFC content-box offset, the transform
    // inverse and the body's viewport re-seed all live in `descend`, which every
    // probe-point walk in this file shares.
    let Frame {
        nx,
        ny,
        x,
        y,
        vx,
        vy,
    } = descend(tree, node, node_id, offset_x, offset_y, x, y, vx, vy)?;

    // A box that contains its fixed descendants keeps every clip above it
    // around them, as paint does (#549): a probe already clipped away from it
    // reaches nothing under it.
    if clipped.is_some() && rinch_dom::paint::contains_fixed_descendants(node) {
        return None;
    }

    // A stacking context whose subtree hoists nothing paints exactly its flow
    // walk, so a point outside its flow extent hits nothing in it: answered
    // before its sequence is built (#1719). The loop below prunes such an
    // entry before entering it; this is the transformed one, which only its
    // own inverse — `descend`, above — can place.
    if stacking_context_misses(tree, node, node_id, nx, ny, x, y) {
        return None;
    }

    let nw = node.layout.width;
    let nh = node.layout.height;

    // A hidden box is not hit itself, but `visibility` is inherited and a
    // descendant may set it back to `visible`: that descendant is painted
    // (#829) and hit, as in Chrome (#843). So a hidden node's subtree is
    // skipped only when nothing below it is visible — the common case, which
    // keeps the old whole-subtree skip — and otherwise walked for the visible
    // boxes alone.
    let hidden = hides_own_hit(tree, node);
    if hidden && !shows_below(tree, node_id) {
        return None;
    }

    let point_in_bounds = x >= nx && x <= nx + nw && y >= ny && y <= ny + nh;

    // A `mask-image` clips everything the box paints to its border box
    // (`mask-clip`), and Chrome 155 hit-tests the same way: nothing of a
    // masked box is hit outside it (the mask's own alpha is not consulted
    // inside it). #1798.
    if !point_in_bounds
        && !node.computed_style.mask_image.is_empty()
        && node.computed_style.display != rinch_dom::computed_style::DisplayValue::Contents
    {
        return None;
    }

    // The scrollbar search: this node's own bars, which paint draws over
    // everything it draws itself — after its in-flow content, and for a
    // stacking context after its whole sequence — unless the sequence it is
    // in draws them later, after its last contained entry (below, #1717).
    // So they are tried before its children, and whatever is drawn over them
    // has already been tried before this node (#1727).
    if let Some(search) = probe.bars()
        && !hidden
        && clipped.is_none()
        && (node.scrolls_y() || node.scrolls_x())
        && !search.deferred.borrow().contains(&node_id)
        && let Some(axis) = bar_under(tree, node, node_id, x - nx, y - ny)
    {
        return search.found(node_id, axis);
    }

    // Nodes with overflow clipping must restrict child hit testing to within
    // the same box paint clips pixels to (#324) — the **padding** box, not
    // the border box (#536): `rinch_dom::paint::padding_box_insets` is the
    // one function that measures it, the same one `clip_shape` scales for
    // paint, so a click on the container's own border resolves to the
    // container (`point_in_bounds`, below, is unaffected and stays the border
    // box — the border itself is still part of the element's own hit target)
    // rather than falling through to content that cannot be drawn there.
    //
    // Per axis (#535), matching `clip_shape`: an axis whose own `overflow`
    // computes to `visible` does not bound children at all, so
    // `overflow-x: clip; overflow-y: visible` only restricts `x`.
    //
    // `clip_axes()` (not the two single-axis predicates) runs the shared
    // display/contents guards once per node instead of twice, and the common
    // "neither axis clips" case skips `padding_box_insets` entirely — this is
    // the hottest node-visit site in the pointer-move path (every visited
    // node, every probe), and both cost a measured +12% on
    // `pointer_move_warm` in CI's Callgrind before this (#535 Perf CI).
    let (clip_x, clip_y) = node.clip_axes();
    let check_children = if !clip_x && !clip_y {
        true
    } else {
        let (left, top, right, bottom) = rinch_dom::paint::padding_box_insets(node);
        let x_ok = !clip_x || (x >= nx + left && x <= nx + nw - right);
        let y_ok = !clip_y || (y >= ny + top && y <= ny + nh - bottom);
        x_ok && y_ok
    };

    let sx = node.scroll_offset.0 as f32;
    let sy = node.scroll_offset.1 as f32;

    // `check_children` is deliberately not a gate around the whole block: a
    // `position: fixed` entry of this root's sequence is not clipped by this
    // root (#545), so it must be probed even when the point is outside these
    // bounds. It is applied per entry below instead.
    //
    // The cost was weighed rather than overlooked: a stacking-context root now
    // builds its `stacking_paint_order` and walks it even when the probe misses
    // its box, skipping every non-fixed entry. That is real work per probe on a
    // tree with many out-of-bounds clipping stacking contexts, and probes run
    // per pointer move. Nothing cheaper is correct — knowing whether a subtree
    // holds a fixed box is what building the sequence answers — and if it ever
    // shows up in a profile the fix is a per-node "has a fixed descendant" bit
    // maintained at style time, not moving this gate back.
    {
        // An IFC text node's layout is stretched to the whole container
        // (write_inline_positions, for scroll-height) — those artificial bounds
        // would shadow inline-block siblings laid out in the same text flow,
        // swallowing clicks meant for e.g. a button. Skip it: the IFC root
        // itself is returned for plain-text hits and text selection resolves by
        // walking up to it.
        let is_stretched_ifc_text =
            |child: &rinch_dom::Node| child.is_text() && child.ifc_root.is_some();

        if (node_id == tree.body_id || node.creates_stacking_context())
            && rinch_dom::RinchDocument::box_tree_children(&tree.nodes, node.id).is_empty()
        {
            // No sequence to build for a stacking context with nothing in its
            // box tree: there is nothing below it to probe (#1719).
        } else if !check_children && rinch_dom::paint::clips_fixed_descendants(node) {
            // A transformed or filtered root is the containing block of its
            // fixed descendants, so its own clip is the one none of them
            // escapes (#549): paint takes the fixed floor after this root's
            // bracket. A probe outside it hits nothing in its sequence.
        } else if node_id == tree.body_id || node.creates_stacking_context() {
            // A stacking-context root probes exactly the sequence paint draws,
            // read backwards — the last box painted is the first one tapped.
            // Same function, same offsets, opposite direction: the two cannot
            // drift apart the way two hand-written phase walks did.
            let order = cached_paint_order::<P>(tree, node_id, (nx - sx) as f64, (ny - sy) as f64);
            let _deferred = probe.bars().map(|s| DeferredScope::enter(s, &order.bars));
            let mut next_bar = order.bars.len();
            // The in-flow entries are the root's in-flow run: in a run painted
            // in Appendix E phases (#2276), their lines and floats answer
            // before any entry's background, at the first of them the walk
            // meets outside the lifted pass.
            let probe_at = if check_children
                && clipped.is_none()
                && rinch_dom::paint::run_paints_in_phases(tree, node)
            {
                order
                    .iter()
                    .rposition(|e| e.kind == rinch_dom::stacking::PaintKind::InFlow)
                    .unwrap_or(usize::MAX)
            } else {
                usize::MAX
            };
            // The boxes in a relative split inline first (`lifted`).
            for (lifted, (index, entry)) in [true, false]
                .into_iter()
                .flat_map(|pass| order.iter().enumerate().rev().map(move |e| (pass, e)))
            {
                // The scrollbar search: the bars paint draws right after this
                // entry, over it and below everything after it (#1717), the
                // outer scroller's over the inner's. Under this root's own
                // clip, as paint draws them inside its bracket.
                if let Some(search) = probe.bars()
                    && !lifted
                {
                    while let Some(bar) = next_bar
                        .checked_sub(1)
                        .map(|b| &order.bars[b])
                        .filter(|b| b.after == index)
                    {
                        next_bar -= 1;
                        if check_children
                            && clipped.is_none()
                            && let Some(axis) = deferred_bar_under(tree, &order, bar, x, y)
                        {
                            return search.found(bar.node_id, axis);
                        }
                    }
                }
                // The cached order is dropped by every mutation that can move or
                // free a node (`hit_cache.invalidate()`), so its ids are as live
                // as the walk that built it (#723). Unchecked on purpose: this
                // runs per entry per probe. PR 2: every route that frees a slot
                // (free-on-discard included) must drop the hit cache before the
                // slot can be reused — `NodeTree::remove_subtree` does, so a
                // free that goes through it is covered; a new one must too.
                let Some(child) = tree.get_live(entry.node_id) else {
                    continue;
                };
                if in_relative_split_inline(tree, child) != lifted {
                    continue;
                }
                if index == probe_at
                    && !lifted
                    && let Some(hit) = phased_run_hit(
                        probe,
                        tree,
                        order
                            .iter()
                            .filter(|e| e.kind == rinch_dom::stacking::PaintKind::InFlow)
                            .map(|e| (e.node_id, e.offset_x as f32, e.offset_y as f32)),
                        x,
                        y,
                        vx,
                        vy,
                    )
                {
                    return Some(hit);
                }
                // A `position: relative` inline element is painted here, in
                // its place among the positioned boxes (#2819), so its text is
                // tapped here too: the hit is its IFC root's, as for any text
                // in its lines, and the target is found from there. It has no
                // box, so nothing below would find it.
                if rinch_dom::paint::paints_in_own_entry(child) {
                    let inside = check_children
                        && clipped.is_none()
                        && order
                            .clips_for(entry)
                            .iter()
                            .all(|c| c.contains(x as f64, y as f64));
                    let over = inside
                        .then(|| {
                            rinch_dom::text_query::layered_span_at(
                                tree,
                                child,
                                entry.offset_x as f32,
                                entry.offset_y as f32,
                                x,
                                y,
                            )
                        })
                        .flatten();
                    // Its atomic inlines first, drawn over its text; placed
                    // from the root's scrolled origin, as the root's own walk
                    // places them. Wherever the point is, not only over its
                    // text: an inline-block beside the span's text is off
                    // that text, and only this entry draws it (#2869).
                    if inside {
                        for id in rinch_dom::text_query::layered_span_boxes(tree, child)
                            .into_iter()
                            .rev()
                        {
                            if let Some(hit) = hit_test_node(
                                probe,
                                tree,
                                id,
                                entry.offset_x as f32,
                                entry.offset_y as f32,
                                x,
                                y,
                                vx,
                                vy,
                                None,
                            ) {
                                return Some(hit);
                            }
                        }
                    }
                    if over == Some(true)
                        && !hides_own_hit(tree, child)
                        && !matches!(
                            child.computed_style.pointer_events,
                            rinch_dom::computed_style::PointerEventsValue::None
                        )
                        && let Some(root) = child.ifc_root
                        && probe.accepts(tree, root)
                    {
                        return Some(root);
                    }
                    continue;
                }
                // A box entered as an ordinary node — in flow, or positioned
                // with `z-index: auto` — can be skipped outright when its
                // flow subtree reaches nowhere near the point. A stacking
                // context can when nothing under it is hoisted (#1719): its
                // sequence is then its flow walk. One that hoists cannot: its
                // own hoisted descendants may be anywhere.
                if (entry.kind != rinch_dom::stacking::PaintKind::StackingContext
                    || !flow_hoists(tree, entry.node_id))
                    && !flow_subtree_may_contain(
                        tree,
                        child,
                        entry.offset_x as f32,
                        entry.offset_y as f32,
                        x,
                        y,
                    )
                {
                    continue;
                }
                if is_stretched_ifc_text(child) {
                    continue;
                }
                // This root's own bounds gate, and the clipping ancestors this
                // entry was hoisted past. Since #324 an `overflow` box is not
                // a stacking context, so the walk went straight through them
                // and the `check_children` gate never saw them — the chain is
                // what keeps a hoisted box unreachable exactly where it is
                // unpainted.
                //
                // The clips are in this root's own space, which is the space
                // `x`/`y` are in here, and at scale 1.0 because that is what
                // this walk asked `stacking_paint_order` for.
                let chain_out = if check_children {
                    order
                        .clips_for(entry)
                        .iter()
                        .find(|c| !c.contains(x as f64, y as f64))
                        .map(|c| c.owner)
                } else {
                    None
                };
                let inside_clips = check_children && chain_out.is_none();
                // A `position: fixed` entry escapes all of those, and every
                // clip `clipped` stands for: its containing block is the
                // viewport, and paint lifts each of them around it
                // (`paint_sequence_entry`, #545, #549). Without that a fixed
                // modal inside an `overflow: hidden; z-index: 1` panel — or
                // under one inside a plain `overflow: hidden` shell — would be
                // painted and never tappable.
                // One whose containing block is a transformed ancestor
                // (`box_position` reads `absolute`, #1372) is clipped by it.
                // Asked only of an entry clipped away: the common probe is
                // inside every clip and pays nothing for it.
                //
                // An absolute entry whose containing block is above this root
                // escapes the clips of that block's descendants (#2815), so it
                // is entered unclipped when the outermost clip the probe is
                // outside of is one of them: this root's own, or a link of the
                // chain, or one above (`clipped`), outermost first.
                let child_clipped = if (inside_clips && clipped.is_none())
                    || child.box_position() == rinch_dom::computed_style::PositionValue::Fixed
                {
                    None
                } else {
                    clipped
                        .or((!check_children).then_some(node_id))
                        .or(chain_out)
                        .filter(|&c| {
                            !rinch_dom::paint::absolute_escapes_clip(tree, node_id, child, c)
                        })
                };
                // Clipped away, the entry can still hold a fixed or absolute
                // box that is not: only a stacking context can (one that is
                // not hoists such descendants into this sequence), and only
                // one with such a box under it — a cached bit, so a probe
                // outside a scroller full of stacking-context rows walks none
                // of them.
                if child_clipped.is_some()
                    && !(entry.kind == rinch_dom::stacking::PaintKind::StackingContext
                        && (rinch_dom::paint::holds_escaping_fixed(tree, entry.node_id)
                            || rinch_dom::paint::holds_escaping_absolute(tree, entry.node_id)))
                {
                    continue;
                }
                if let Some(hit) = hit_test_node(
                    probe,
                    tree,
                    entry.node_id,
                    entry.offset_x as f32,
                    entry.offset_y as f32,
                    x,
                    y,
                    vx,
                    vy,
                    child_clipped,
                ) {
                    return Some(hit);
                }
            }
        } else if check_children && clipped.is_none() {
            // Not a stacking-context root — test only the children that were
            // not hoisted to an ancestor's sequence, in reverse tree order.
            // Nothing here can be a fixed box: one creates a stacking context
            // unconditionally, so `paints_at_stacking_root` skips it and it is
            // an entry of some ancestor's sequence instead. So this branch keeps
            // the plain bounds gate.
            //
            // The boxes in a relative split inline first (`lifted`).
            //
            // A run painted in Appendix E phases (#2276): its lines and floats
            // are over every child's background, so after the lifted boxes
            // they answer first, and the plain walk is the backgrounds pass.
            let kids = rinch_dom::RinchDocument::box_tree_children(&tree.nodes, node.id);
            let phased = rinch_dom::paint::run_paints_in_phases(tree, node);
            for lifted in [true, false] {
                if phased
                    && !lifted
                    && let Some(hit) = phased_run_hit(
                        probe,
                        tree,
                        kids.iter().map(|&id| (id, nx - sx, ny - sy)),
                        x,
                        y,
                        vx,
                        vy,
                    )
                {
                    return Some(hit);
                }
                for &child_id in kids.iter().rev() {
                    let Some(child) = tree.get_live(child_id) else {
                        continue;
                    };
                    if paints_at_stacking_root(child)
                        || is_stretched_ifc_text(child)
                        || in_relative_split_inline(tree, child) != lifted
                    {
                        continue;
                    }
                    if !flow_subtree_may_contain(tree, child, nx - sx, ny - sy, x, y) {
                        continue;
                    }
                    if let Some(hit) =
                        hit_test_node(probe, tree, child_id, nx - sx, ny - sy, x, y, vx, vy, None)
                    {
                        return Some(hit);
                    }
                }
            }
        }
    }

    // An inline element's padding or border past the root's box (#2647).
    let in_inline_box = !point_in_bounds
        && !hidden
        && !clip_x
        && !clip_y
        && inline_box_outside(tree, node, x - nx, y - ny);
    // Text on a line that overflows the root's box (#2956): Chrome 158 hits
    // the line's text, as it draws it, wherever the line is. Not where the
    // root clips, per axis as paint does: `check_children` is the region its
    // clip lets through (all of it on an axis that does not clip), so an
    // `overflow-x: clip` root's line below it is hit and one past its right
    // edge is not. `flow_extent` holds the lines on the open axes, so the
    // prune reaches them.
    let on_overflowing_line = !point_in_bounds
        && !in_inline_box
        && check_children
        && node.text_layout.is_some()
        && visible_text_at(tree, node, x - nx, y - ny);
    if clipped.is_some()
        || !(point_in_bounds || in_inline_box || on_overflowing_line)
        || (hidden && !visible_text_at(tree, node, x - nx, y - ny))
    {
        return None;
    }

    // Skip this element if pointer-events: none (children still checked above)
    if matches!(
        node.computed_style.pointer_events,
        rinch_dom::computed_style::PointerEventsValue::None
    ) {
        return None;
    }

    probe.accepts(tree, node_id).then_some(node_id)
}
