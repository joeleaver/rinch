//! The oracle: `hit_test` as it was before the prune and the cache, walking
//! every subtree with no extent and no memo.

use super::super::{Frame, descend, hit_test};
use rinch_core::dom::{DomContent, DomTree};
use rinch_dom::RinchDocument;
use rinch_dom::stacking::{paints_at_stacking_root, stacking_paint_order};

// ── The oracle, verbatim from before the prune ──────────────────────────────

#[allow(clippy::too_many_arguments)]
pub(super) fn reference_hit_test_node(
    tree: &rinch_dom::NodeTree,
    node_id: u64,
    offset_x: f32,
    offset_y: f32,
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    // The outermost clip the probe is outside of that a `position: fixed`
    // descendant (#549) or an escaping absolute one (#2815) escapes; see
    // `hit_test_node`. Unpruned here: every entry is entered.
    clipped: Option<u64>,
) -> Option<u64> {
    tree.perf
        .bump(rinch_dom::perf::Counter::HitTestNodesVisited);
    let node = tree.get(node_id)?;

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

    if clipped.is_some() && rinch_dom::paint::contains_fixed_descendants(node) {
        return None;
    }

    let nw = node.layout.width;
    let nh = node.layout.height;

    // A hidden box is not hit itself, but its subtree is always walked: a
    // descendant may set `visibility: visible` again (#843). No "anything
    // visible below" prune here — that is one of the optimisations under test.
    // A text node is drawn with its parent's visibility.
    let is_hidden = |n: &rinch_dom::Node| {
        matches!(
            n.computed_style.visibility,
            rinch_dom::computed_style::VisibilityValue::Hidden
                | rinch_dom::computed_style::VisibilityValue::Collapse
        )
    };
    let hidden = if node.is_text() {
        node.parent.and_then(|p| tree.get(p)).is_some_and(is_hidden)
    } else {
        is_hidden(node)
    };

    let point_in_bounds = x >= nx && x <= nx + nw && y >= ny && y <= ny + nh;

    // Nodes with overflow clipping must restrict child hit testing to within
    // the same box paint clips pixels to (#324) — the **padding** box, not
    // the border box (#536), per axis (#535); see `hit_test_node`'s identical
    // arm, which this function is a verbatim pre-prune copy of.
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

        if !check_children && rinch_dom::paint::clips_fixed_descendants(node) {
            // A transformed or filtered root is its fixed descendants'
            // containing block: its own clip is one none of them escapes
            // (#549), so a probe outside it reaches nothing in its sequence.
        } else if node_id == tree.body_id || node.creates_stacking_context() {
            // A stacking-context root probes exactly the sequence paint draws,
            // read backwards — the last box painted is the first one tapped.
            // Same function, same offsets, opposite direction: the two cannot
            // drift apart the way two hand-written phase walks did.
            let order =
                stacking_paint_order(tree, node_id, 1.0, (nx - sx) as f64, (ny - sy) as f64);
            let mut run_probed = !(check_children
                && clipped.is_none()
                && rinch_dom::paint::run_paints_in_phases(tree, node));
            for entry in order.iter().rev() {
                let Some(child) = tree.get(entry.node_id) else {
                    continue;
                };
                // The in-flow entries' lines and floats (#2276), at the first
                // of them.
                if !run_probed && entry.kind == rinch_dom::stacking::PaintKind::InFlow {
                    run_probed = true;
                    if let Some(hit) = super::super::phased_run_hit_with(
                        tree,
                        order
                            .iter()
                            .filter(|e| e.kind == rinch_dom::stacking::PaintKind::InFlow)
                            .map(|e| (e.node_id, e.offset_x as f32, e.offset_y as f32)),
                        x,
                        y,
                        &|id, ox, oy| reference_hit_test_node(tree, id, ox, oy, x, y, vx, vy, None),
                        &|_| true,
                    ) {
                        return Some(hit);
                    }
                }
                // This root's own bounds gate. A `position: fixed` entry is
                // exempt: this root is not its containing block, so its clip
                // does not apply — paint lifts the same bracket around it
                // (`paint_children_with_stacking`, #545). Without the exemption
                // a fixed modal inside an `overflow: hidden; z-index: 1` panel
                // would be painted and never tappable.
                // One whose containing block is a transformed ancestor
                // (`box_position` reads `absolute`, #1372) is clipped by it.
                let escapes_clips =
                    child.box_position() == rinch_dom::computed_style::PositionValue::Fixed;
                if is_stretched_ifc_text(child) {
                    continue;
                }
                // A relative inline element: its atomic inlines, then its
                // drawn text, which answers its IFC root (#2819).
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
                    if inside {
                        for id in rinch_dom::text_query::layered_span_boxes(tree, child)
                            .into_iter()
                            .rev()
                        {
                            if let Some(hit) = reference_hit_test_node(
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
                        && !rinch_dom::visibility::hides_own(tree, child)
                        && !matches!(
                            child.computed_style.pointer_events,
                            rinch_dom::computed_style::PointerEventsValue::None
                        )
                        && let Some(root) = child.ifc_root
                    {
                        return Some(root);
                    }
                    continue;
                }
                // The clipping ancestors this entry was hoisted past. Since
                // #324 an `overflow` box is not a stacking context, so the
                // walk went straight through them and the `check_children`
                // gate below never saw them — the chain is what keeps a
                // hoisted box unreachable exactly where it is unpainted.
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
                // A fixed entry escapes this root's gate, its own (empty)
                // chain and every clip `clipped` stands for (#545, #549); an
                // absolute one whose containing block is above this root
                // escapes the outermost of them when that block holds it
                // (#2815); anything else clipped away is still entered,
                // clipped, to look for one.
                let child_clipped = if escapes_clips {
                    None
                } else {
                    clipped
                        .or((!check_children).then_some(node_id))
                        .or(chain_out)
                        .filter(|&c| {
                            !rinch_dom::paint::absolute_escapes_clip(tree, node_id, child, c)
                        })
                };
                if let Some(hit) = reference_hit_test_node(
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
            // A run painted in Appendix E phases (#2276) answers from its lines
            // and floats first, by the same passes, entered through this walk.
            let kids = rinch_dom::RinchDocument::box_tree_children(&tree.nodes, node.id);
            if rinch_dom::paint::run_paints_in_phases(tree, node)
                && let Some(hit) = super::super::phased_run_hit_with(
                    tree,
                    kids.iter().map(|&id| (id, nx - sx, ny - sy)),
                    x,
                    y,
                    &|id, ox, oy| reference_hit_test_node(tree, id, ox, oy, x, y, vx, vy, None),
                    &|_| true,
                )
            {
                return Some(hit);
            }
            for &child_id in kids.iter().rev() {
                let Some(child) = tree.get(child_id) else {
                    continue;
                };
                if paints_at_stacking_root(child) || is_stretched_ifc_text(child) {
                    continue;
                }
                if let Some(hit) =
                    reference_hit_test_node(tree, child_id, nx - sx, ny - sy, x, y, vx, vy, None)
                {
                    return Some(hit);
                }
            }
        }
    }

    // An inline element's padding or border past an IFC root's box (#2647):
    // what the root is hit for, not a prune, so the oracle has it too.
    let in_inline_box = !point_in_bounds
        && !hidden
        && !clip_x
        && !clip_y
        && super::super::inline_box_outside(tree, node, x - nx, y - ny);
    // Text on a line that overflows the root's box (#2956): Chrome 158 hits
    // the line's text, as it draws it, wherever the line is, except where
    // the root clips it (per axis: `check_children`). `flow_extent` holds
    // the lines on the open axes, so the prune reaches them.
    let on_overflowing_line = !point_in_bounds
        && !in_inline_box
        && check_children
        && node.text_layout.is_some()
        && super::super::visible_text_at(tree, node, x - nx, y - ny);
    if clipped.is_some()
        || !(point_in_bounds || in_inline_box || on_overflowing_line)
        || (hidden && !super::super::visible_text_at(tree, node, x - nx, y - ny))
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

    Some(node_id)
}

/// #535 review: `reference_hit_test_node`'s own doc comment claims it is "a
/// verbatim pre-prune copy of `hit_test_node`'s identical arm", but its
/// `check_children` was not updated when `hit_test_node`'s became per-axis
/// (#535) — it kept the old `!clips_overflow() || <both-axes border-box
/// test>` shape. The random fuzzer below never generated the one pair
/// (`overflow-x: clip; overflow-y: visible`, or its mirror) that could expose
/// that divergence, so it went unnoticed by `random_documents_hit_identically`
/// even though that test is specifically the oracle for this function.
///
/// Direct, not fuzzed: on the exact fixture the production `hit_test_node`
/// fixture in `hit_testing/tests/clip.rs` uses (#535's own
/// `a_click_past_an_open_axis_still_reaches_overflowing_content_...`), the
/// real path and the oracle must now agree — a click past the container's
/// own box on the *open* (`overflow-y: visible`) axis still reaches the
/// overflowing child.
///
/// The mutant this kills: reverting `reference_hit_test_node`'s
/// `check_children` to `!node.clips_overflow() || <border-box-both-axes
/// test>` makes the oracle answer `Some(container)` here while the real path
/// answers `Some(child)` — this was true before `check_children` was fixed
/// (`real != oracle`, confirmed by this test's own first draft) and the fix
/// is what turns it into equality.
#[test]
fn reference_hit_test_node_is_per_axis_like_the_real_one() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "width: 100px; height: 100px; overflow-x: clip; overflow-y: visible",
    );
    doc.append_child(body, container);
    let child = doc.create_element("div");
    doc.set_attribute(child, "style", "width: 200px; height: 200px");
    doc.append_child(container, child);
    doc.resolve_layout(800.0, 600.0);

    let real = hit_test(&doc.tree, 50.0, 150.0);
    let oracle = reference_hit_test_node(
        &doc.tree,
        doc.tree.body_id,
        0.0,
        0.0,
        50.0,
        150.0,
        50.0,
        150.0,
        None,
    );
    assert_eq!(
        real,
        Some(child.0),
        "the real path must reach the overflowing child on the open axis"
    );
    assert_eq!(
        real, oracle,
        "real={real:?} oracle={oracle:?} — reference_hit_test_node must agree \
         with hit_test_node's per-axis check_children (#535)"
    );
}
