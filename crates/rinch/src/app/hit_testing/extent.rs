use super::targets::hides_own_hit;
use rinch_dom::stacking::paints_at_stacking_root;

/// Slack added around a subtree extent before it is allowed to prune. The
/// extent is summed relative to each box while `hit_test_node` sums absolute
/// offsets down the tree, and the two roundings can differ by an ulp at an
/// edge; a pixel of slack keeps the prune strictly conservative.
const EXTENT_SLACK: f32 = 1.0;

/// [`flow_extent`]'s cache hit, inlined where the warm pointer move's 500-row
/// loop asks it ([`flow_subtree_may_contain`]); a miss computes it out of line.
/// `flow_extent` itself stays one function, cache check included, for its own
/// recursion: inlining the check there cost the cold move about 4% (review of
/// #1720, which also measured +18% on `shell::pointer_move_warm` while
/// nothing inlined the hit).
#[inline]
fn cached_flow_extent(tree: &rinch_dom::NodeTree, id: u64) -> rinch_dom::hit_cache::Extent {
    match tree.hit_cache.extent(id) {
        Some(e) => e,
        None => flow_extent(tree, id),
    }
}

/// Whether `hit_test_node(child)` — entered at parent offset `(ox, oy)` as an
/// ordinary node, i.e. not as a stacking-context root — could possibly return
/// a hit for the point `(x, y)`.
///
/// `false` is a proof, not a guess: the point lies outside the union of every
/// box the child's flow walk can test (see [`flow_extent`]), so the walk would
/// visit the whole subtree and find nothing. `true` means "walk it".
pub(super) fn flow_subtree_may_contain(
    tree: &rinch_dom::NodeTree,
    child: &rinch_dom::Node,
    ox: f32,
    oy: f32,
    x: f32,
    y: f32,
) -> bool {
    // Mirror `descend` for a non-fixed, untransformed box. Only a stacking
    // context can be either, and callers never hand one in — but a box that
    // is one anyway is simply walked.
    if child.box_position() == rinch_dom::computed_style::PositionValue::Fixed
        || child.has_applied_transform()
    {
        return true;
    }
    let (dx, dy) = rinch_dom::paint::ifc_content_box_offset(tree, child);
    let cx = ox + child.layout.x + dx;
    let cy = oy + child.layout.y + dy;
    let [x0, y0, x1, y1] = cached_flow_extent(tree, child.id);
    x >= cx + x0 - EXTENT_SLACK
        && x <= cx + x1 + EXTENT_SLACK
        && y >= cy + y0 - EXTENT_SLACK
        && y <= cy + y1 + EXTENT_SLACK
}

/// The rectangle, relative to `id`'s own border-box origin, that contains
/// every box `hit_test_node` can test when it enters `id` as an ordinary node:
/// the node's own box, and — unless the node clips, which confines every child
/// hit to its own box — the extents of the children that walk recurses into
/// (the same enumeration: box-tree children, minus those hoisted to a stacking
/// root and the stretched IFC text it never tests), placed where the walk
/// places them, scroll offset included.
///
/// Hoisted descendants are deliberately absent: they are entries of an
/// ancestor stacking root's sequence, and that root tests them itself.
///
/// A NaN component (a box with no finite layout) is dropped by `min`/`max`,
/// which is conservative: a subtree placed at NaN cannot be hit anyway.
///
/// Memoised in the tree's hit cache until something invalidates it. The one
/// input that is not relative to `id`'s own origin is its **own** scroll offset
/// — a descendant's extent never reads an ancestor's — and only when `id` does
/// not clip, which is what lets a scroll of a box that clips keep every extent
/// (`HitCache::invalidate_scroll`, #911).
///
/// Per axis (#535): a box clipping on only one axis still folds its children
/// into the *open* axis — `overflow-x: clip; overflow-y: visible` confines `x`
/// to the box but lets `y` grow with whatever a child reaches, which is what a
/// sibling's `flow_subtree_may_contain` has to see to still walk into a child
/// overflowing *this* node vertically.
pub(super) fn flow_extent(tree: &rinch_dom::NodeTree, id: u64) -> rinch_dom::hit_cache::Extent {
    if let Some(e) = tree.hit_cache.extent(id) {
        return e;
    }
    tree.perf.bump(rinch_dom::perf::Counter::HitExtentsComputed);
    let Some(node) = tree.get_live(id) else {
        return [0.0; 4];
    };
    let mut e = [0.0, 0.0, node.layout.width, node.layout.height];
    let (clip_x, clip_y) = node.clip_axes();
    // An IFC root's inline elements' padding and border, which can reach
    // past its box and are hit there (#2647; `inline_box_outside`). Not a
    // hidden root's, which `hit_test_node` does not grow.
    if !clip_x
        && !clip_y
        && let Some(inline) = node.text_layout.as_ref()
        && !hides_own_hit(tree, node)
        && let Some([r0, r1, r2, r3]) =
            rinch_dom::paint::inline_box_reach(|id| tree.get(id), inline)
    {
        let (ox, oy) = rinch_dom::paint::ifc_root_content_origin(node);
        let (sx, sy) = (node.scroll_offset.0 as f32, node.scroll_offset.1 as f32);
        e[0] = e[0].min(r0 + ox - sx);
        e[1] = e[1].min(r1 + oy - sy);
        e[2] = e[2].max(r2 + ox - sx);
        e[3] = e[3].max(r3 + oy - sy);
    }
    // An IFC root's lines, which are hit wherever they overflow the box: in
    // a run painted in Appendix E phases (#2276, `PhasedRun::member`) and by
    // `hit_test_node`'s own bounds test (#2956), on each axis the root does
    // not clip. Placed at the content origin, so a line no larger than the
    // box still overflows it when the root's padding or border pushes it
    // out; a size-only shortcut here would let the prune skip that line,
    // which the hit test answers. Across, the lines' own reach
    // (`InlineLayout::lines_x_reach`, measured when they are built): a line
    // can start left of the origin (a negative `text-indent`, an rtl line
    // wider than its box), which `0..width()` would miss. Where they are
    // drawn, not where they were laid out: a relative inline element's text
    // is drawn moved by its offset and hit there (`visible_text_at` through
    // `unmoved_point`). One that paints in its own entry is hit by that entry,
    // but a split one (a block inside it) has none, and its text in an
    // anonymous block's lines is reached only through this extent (#3056).
    if (!clip_x || !clip_y)
        && let Some(inline) = node.text_layout.as_ref()
        && let Some([lx0, lx1]) = inline.lines_x_reach()
    {
        let (ox, oy) = rinch_dom::paint::ifc_root_content_origin(node);
        let (sx, sy) = (node.scroll_offset.0 as f32, node.scroll_offset.1 as f32);
        let lh = inline.layout.height();
        let [d0, d1, d2, d3] = inline.relative_reach().map(|v| v as f32);
        if lx1 > lx0 && lh > 0.0 {
            if !clip_x {
                e[0] = e[0].min(ox - sx + lx0 + d0);
                e[2] = e[2].max(ox - sx + lx1 + d2);
            }
            if !clip_y {
                e[1] = e[1].min(oy - sy + d1);
                e[3] = e[3].max(oy - sy + lh + d3);
            }
        }
    }
    if !clip_x || !clip_y {
        let sx = node.scroll_offset.0 as f32;
        let sy = node.scroll_offset.1 as f32;
        for &child_id in rinch_dom::RinchDocument::box_tree_children(&tree.nodes, id).iter() {
            let Some(child) = tree.get_live(child_id) else {
                continue;
            };
            if paints_at_stacking_root(child) || (child.is_text() && child.ifc_root.is_some()) {
                continue;
            }
            let (dx, dy) = rinch_dom::paint::ifc_content_box_offset(tree, child);
            let cx = child.layout.x + dx - sx;
            let cy = child.layout.y + dy - sy;
            let [c0, c1, c2, c3] = flow_extent(tree, child_id);
            if !clip_x {
                e[0] = e[0].min(cx + c0);
                e[2] = e[2].max(cx + c2);
            }
            if !clip_y {
                e[1] = e[1].min(cy + c1);
                e[3] = e[3].max(cy + c3);
            }
        }
    }
    tree.hit_cache.store_extent(id, e);
    e
}

/// Whether `node` is a stacking context (not the body) whose subtree hoists
/// nothing and whose flow extent misses `(x, y)` — in its own space, after
/// `descend`'s inverse — so its hit test can answer `None` without building
/// its sequence (#1719). Kept out of line: `hit_test_node` runs it for every
/// node it visits, and inlining it pushed `flow_subtree_may_contain` out of
/// `hit_test_node` (+4% on `shell::pointer_move_cold`, review of #1720).
#[inline(never)]
pub(super) fn stacking_context_misses(
    tree: &rinch_dom::NodeTree,
    node: &rinch_dom::Node,
    node_id: u64,
    nx: f32,
    ny: f32,
    x: f32,
    y: f32,
) -> bool {
    if node_id == tree.body_id || !node.creates_stacking_context() || flow_hoists(tree, node_id) {
        return false;
    }
    let [x0, y0, x1, y1] = flow_extent(tree, node_id);
    !(x >= nx + x0 - EXTENT_SLACK
        && x <= nx + x1 + EXTENT_SLACK
        && y >= ny + y0 - EXTENT_SLACK
        && y <= ny + y1 + EXTENT_SLACK)
}

/// Whether the flow walk under `id` passes over a box painted at a stacking
/// root (positioned, or a stacking context). A stacking context with no such
/// box has a sequence holding only its in-flow children — the flow walk,
/// exactly — so its flow extent bounds every box its own hit test can reach,
/// and it can be pruned like an in-flow box (#1719). A box that clips on both
/// axes answers conservatively ("hoists", if it has children) without walking.
/// Kept apart from [`flow_extent`] so a document with no stacking context
/// below the body pays nothing for it.
pub(super) fn flow_hoists(tree: &rinch_dom::NodeTree, id: u64) -> bool {
    if let Some(h) = tree.hit_cache.hoists(id) {
        return h;
    }
    let Some(node) = tree.get_live(id) else {
        return false;
    };
    let (clip_x, clip_y) = node.clip_axes();
    let hoists = if clip_x && clip_y {
        !node.children.is_empty()
    } else {
        rinch_dom::RinchDocument::box_tree_children(&tree.nodes, id)
            .iter()
            .any(|&child_id| {
                tree.get_live(child_id).is_some_and(|child| {
                    paints_at_stacking_root(child)
                        || (!(child.is_text() && child.ifc_root.is_some())
                            && flow_hoists(tree, child_id))
                })
            })
    };
    tree.hit_cache.store_hoists(id, hoists);
    hoists
}
