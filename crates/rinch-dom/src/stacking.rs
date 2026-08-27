//! The CSS paint sequence for a stacking context's children.
//!
//! Paint and hit testing are the same walk run in opposite directions: paint
//! draws a stacking context's children front-to-back, hit testing probes them
//! back-to-front and returns the first that answers. They used to compute that
//! order twice, from two different sets of rules, and the two disagreed — which
//! is a class of bug where a box is drawn somewhere the taps do not go. This
//! module is the single answer both of them ask.
//!
//! # The sequence
//!
//! For a stacking-context root, [`stacking_paint_order`] returns its children in
//! CSS 2.1 Appendix E order, back to front:
//!
//! 1. **Negative-`z-index` stacking contexts**, ascending `z`, then tree order.
//! 2. **In-flow, non-positioned, non-stacking-context children**, in tree order.
//! 3. **The `z == 0` level**, in tree order: positioned descendants with
//!    `z-index: auto` (Appendix E step 8 — the step Rinch had no phase for at
//!    all), interleaved with stacking contexts whose `z-index` is `auto` or an
//!    explicit `0`.
//! 4. **Positive-`z-index` stacking contexts**, ascending `z`, then tree order.
//!
//! Steps 3 and 4 are one sort, not two passes: every hoisted entry carries a
//! `z_index` (`auto` counting as `0`) and a tree-order index, and the sort key is
//! the pair. A positioned `z-index: auto` box is entered at `z == 0`, so a FAB
//! written after a scroller in the markup lands above it, and one written before
//! it lands below — which is what tree order means at a single z level.
//!
//! # Hoisting
//!
//! A child is *hoisted* when it does not paint in its parent's own tree-order
//! run but at the nearest stacking-context ancestor's ordered sequence. Both
//! stacking contexts and positioned `z-index: auto` boxes hoist —
//! [`paints_at_stacking_root`] is the predicate, and a parent walking its
//! children in tree order must skip everything it answers `true` for, or the
//! subtree is painted twice.
//!
//! The collection walk keeps descending *through* a hoisted positioned
//! `z-index: auto` box, so its own positioned and stacking-context descendants
//! surface in this same sequence rather than inside it. That is Appendix E step
//! 8's second half — the box is treated as if it created a stacking context,
//! "but any positioned descendants and descendants which actually create a new
//! stacking context should be considered part of the parent stacking context".
//! It stops at a real stacking context, whose descendants are that context's
//! business.
//!
//! # Transforms
//!
//! The accumulated offsets never cross a CSS transform, so an entry always lands
//! in the collecting root's own space — the space the painter's `node_transform`
//! and hit testing's probe point are both already in. A transformed box creates a
//! stacking context, so the walk stops at it; and a positioned `z-index: auto`
//! box that the walk descends *through* has, by the same rule, no transform of
//! its own.
//!
//! # `position: fixed`
//!
//! A fixed box is viewport-relative and must escape every ancestor clip, so at
//! the body it is hoisted out of intermediate stacking contexts with its offsets
//! zeroed (its `layout.x`/`layout.y` are already viewport coordinates), and at
//! every deeper level it is left out entirely — the body already has it.

use crate::computed_style::PositionValue;
use crate::node::{Node, NodeTree, RawNodeId};

/// How a consumer descends into a [`PaintEntry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintKind {
    /// A child painted in its parent's tree-order run: an in-flow,
    /// non-positioned box that forms no stacking context. Descend into it as an
    /// ordinary child — it is not a stacking-context root.
    InFlow,
    /// A descendant stacking context, entered as a stacking-context root.
    StackingContext,
    /// A positioned descendant with `z-index: auto`. Entered as an ordinary node
    /// — it is *not* a stacking-context root, because its own positioned and
    /// stacking-context descendants are entries of this same sequence.
    PositionedAuto,
}

/// One child of a stacking-context root, in paint order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaintEntry {
    /// The node to descend into.
    pub node_id: RawNodeId,
    /// How to descend into it.
    pub kind: PaintKind,
    /// Accumulated X offset from the collecting root to this node's *parent*,
    /// in the same units as the `offset_x` handed to [`stacking_paint_order`].
    pub offset_x: f64,
    /// Accumulated Y offset, likewise.
    pub offset_y: f64,
    /// `z-index`, with `auto` counting as `0`. Always `0` for [`PaintKind::InFlow`].
    pub z_index: i32,
}

/// Whether `node` is positioned with `z-index: auto` — CSS 2.1 Appendix E step
/// 8's "positioned descendants with 'z-index: auto'".
fn is_positioned_z_auto(node: &Node) -> bool {
    node.computed_style.position != PositionValue::Static && node.computed_style.z_index.is_none()
}

/// Whether `node` paints at the nearest stacking-context ancestor's ordered
/// sequence rather than in its own parent's tree-order run.
///
/// A parent walking its children in tree order — paint's non-stacking-context
/// branch, hit testing's — must skip every child this answers `true` for: they
/// are already entries of an ancestor's [`stacking_paint_order`], and painting
/// them here as well would draw the subtree twice, at the wrong depth.
pub fn paints_at_stacking_root(node: &Node) -> bool {
    node.creates_stacking_context() || is_positioned_z_auto(node)
}

/// The children of the stacking-context root `node_id`, back to front.
///
/// `offset_x`/`offset_y` is the root's own scroll-adjusted content origin — the
/// offset its direct children are laid out against — and every entry's offset is
/// accumulated from there through the non-hoisted boxes in between, so an entry
/// can be descended into directly with no walk of the nodes it skipped.
/// `scale` converts `layout` units to the caller's: paint passes the DPI scale
/// and works in physical pixels, hit testing passes `1.0` and works in layout
/// pixels.
///
/// `is_body` selects the viewport-level variant, which hoists `position: fixed`
/// boxes out of intermediate stacking contexts; pass `node_id == tree.body_id`.
pub fn stacking_paint_order(
    tree: &NodeTree,
    node_id: RawNodeId,
    is_body: bool,
    scale: f64,
    offset_x: f64,
    offset_y: f64,
) -> Vec<PaintEntry> {
    let Some(node) = tree.get(node_id) else {
        return Vec::new();
    };

    // Steps 1, 3 and 4: everything hoisted to this root, gathered in tree order
    // and then sorted by (z, tree order). Collected first so `order` counts
    // every node the walk passes, direct children included.
    let mut hoisted: Vec<(usize, PaintEntry)> = Vec::new();
    let mut order = 0usize;
    collect_hoisted(
        tree,
        &node.children,
        scale,
        offset_x,
        offset_y,
        is_body,
        &mut hoisted,
        &mut order,
    );
    hoisted.sort_by_key(|(dom_order, e)| (e.z_index, *dom_order));

    let split = hoisted.partition_point(|(_, e)| e.z_index < 0);
    let mut sequence: Vec<PaintEntry> = Vec::with_capacity(hoisted.len() + node.children.len());
    sequence.extend(hoisted[..split].iter().map(|(_, e)| *e));

    // Step 2: the root's own in-flow, non-positioned children, in tree order.
    sequence.extend(node.children.iter().filter_map(|&child_id| {
        let child = tree.get(child_id)?;
        (!paints_at_stacking_root(child)).then_some(PaintEntry {
            node_id: child_id,
            kind: PaintKind::InFlow,
            offset_x,
            offset_y,
            z_index: 0,
        })
    }));

    sequence.extend(hoisted[split..].iter().map(|(_, e)| *e));
    sequence
}

/// Gather every hoisted descendant of `children` in tree order, paired with the
/// tree-order index that breaks ties within a z level.
#[allow(clippy::too_many_arguments)]
fn collect_hoisted(
    tree: &NodeTree,
    children: &[RawNodeId],
    scale: f64,
    parent_offset_x: f64,
    parent_offset_y: f64,
    hoist_fixed: bool,
    out: &mut Vec<(usize, PaintEntry)>,
    order: &mut usize,
) {
    for &child_id in children {
        let Some(child) = tree.get(child_id) else {
            continue;
        };
        let dom_order = *order;
        *order += 1;

        let is_fixed = child.computed_style.position == PositionValue::Fixed;
        let is_sc = child.creates_stacking_context();

        if !is_sc && !is_positioned_z_auto(child) {
            // Not hoisted: descend through it, accumulating its offset, to reach
            // the hoisted boxes below.
            descend(
                tree,
                child,
                scale,
                parent_offset_x,
                parent_offset_y,
                hoist_fixed,
                out,
                order,
            );
            continue;
        }

        // A fixed box belongs to the viewport, i.e. to the body's sequence.
        // Below the body it is left out; the body picks it up by walking into
        // the stacking contexts that would otherwise have hidden it.
        if is_fixed && !hoist_fixed {
            continue;
        }

        // Fixed boxes are viewport-relative: `layout.x`/`layout.y` are already
        // absolute, so the accumulated offset must not be added — to the entry,
        // or to anything hoisted out from under it.
        let (base_x, base_y) = if is_fixed {
            (0.0, 0.0)
        } else {
            (parent_offset_x, parent_offset_y)
        };

        out.push((
            dom_order,
            PaintEntry {
                node_id: child_id,
                kind: if is_sc {
                    PaintKind::StackingContext
                } else {
                    PaintKind::PositionedAuto
                },
                offset_x: base_x,
                offset_y: base_y,
                z_index: child.computed_style.z_index.unwrap_or(0),
            },
        ));

        if is_sc {
            // A stacking context owns its descendants — except the fixed ones,
            // which the body reaches past it for.
            if hoist_fixed {
                collect_fixed(tree, &child.children, out, order);
            }
        } else {
            // A positioned `z-index: auto` box is entered as an ordinary node,
            // so its own hoisted descendants are this sequence's, not its.
            descend(tree, child, scale, base_x, base_y, hoist_fixed, out, order);
        }
    }
}

/// Recurse into `child`'s children with `child`'s own layout offset and scroll
/// folded into the accumulated offset.
#[allow(clippy::too_many_arguments)]
fn descend(
    tree: &NodeTree,
    child: &Node,
    scale: f64,
    parent_offset_x: f64,
    parent_offset_y: f64,
    hoist_fixed: bool,
    out: &mut Vec<(usize, PaintEntry)>,
    order: &mut usize,
) {
    let x = parent_offset_x + child.layout.x as f64 * scale - child.scroll_offset.0 * scale;
    let y = parent_offset_y + child.layout.y as f64 * scale - child.scroll_offset.1 * scale;
    collect_hoisted(tree, &child.children, scale, x, y, hoist_fixed, out, order);
}

/// Walk into stacking contexts the body would otherwise not see past, collecting
/// the `position: fixed` boxes inside them.
///
/// A fixed modal nested in an `overflow: auto` container is viewport-level
/// content that happens to live in the markup under a clip; without this it
/// would paint inside that clip, and be hit-tested inside it too.
fn collect_fixed(
    tree: &NodeTree,
    children: &[RawNodeId],
    out: &mut Vec<(usize, PaintEntry)>,
    order: &mut usize,
) {
    for &child_id in children {
        let Some(child) = tree.get(child_id) else {
            continue;
        };
        let dom_order = *order;
        *order += 1;

        if child.computed_style.position == PositionValue::Fixed {
            // A fixed box is positioned by definition, so it is hoisted either
            // way: a stacking context when it carries a `z-index` (or an
            // opacity/transform/overflow of its own), and a step-8 entry when
            // its `z-index` is `auto`.
            out.push((
                dom_order,
                PaintEntry {
                    node_id: child_id,
                    kind: if child.creates_stacking_context() {
                        PaintKind::StackingContext
                    } else {
                        PaintKind::PositionedAuto
                    },
                    offset_x: 0.0,
                    offset_y: 0.0,
                    z_index: child.computed_style.z_index.unwrap_or(0),
                },
            ));
        }

        collect_fixed(tree, &child.children, out, order);
    }
}
