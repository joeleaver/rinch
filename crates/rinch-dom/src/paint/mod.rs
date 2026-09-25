//! Abstract scene building for rinch-dom.
//!
//! Walks the node tree and emits drawing commands via the `Painter` trait
//! for backgrounds, borders, and text.

mod blur;
mod borders;
pub mod clip;
mod contenteditable;
pub mod damage;
pub mod image;
mod layer_bounds;
pub mod painter;
pub mod scrollbar;
mod select;
mod svg;
mod text;
mod text_shadow;
pub mod vello_painter;

#[cfg(feature = "software-renderer")]
pub mod skia_painter;

use borders::*;
pub use clip::{border_radii, clip_shape};
use contenteditable::*;
pub use damage::{DamageRegion, MAX_DAMAGE_RECTS};
pub use layer_bounds::{UNBOUNDED, opacity_layer_bounds};
use layer_bounds::{
    clip_cuts_nothing, opacity_layer_shape, subtree_is_entirely_outside, subtree_paint_extent,
};
use svg::*;
use text::*;
pub use text_shadow::{
    TAP_GLYPH_BUDGET, TAP_GLYPHS_PER_SHADOW, clear_text_shadow_cache, force_tapped_text_shadows,
    text_shadow_scratch_size,
};

use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, BezPath, Point, Rect, RoundedRect, RoundedRectRadii, Shape, Vec2};
use peniko::{Brush, Fill};

use painter::{BlendMode, PaintShape, Painter};

use crate::computed_style::{
    BackgroundValue, DisplayValue, OverflowValue, PositionValue, VisibilityValue,
};
use crate::node::{Node, NodeKind, NodeTree, RawNodeId};
use crate::stacking::{ClipSpan, PaintKind, paints_at_stacking_root, stacking_paint_order};

/// The fraction of the surface at or above which a dirty region is not worth
/// clipping to, and the software renderer repaints in full instead.
/// [`compute_dirty_region`] stops measuring once its region reaches it.
pub const FULL_REPAINT_FRACTION: f64 = 0.5;

/// Compute the dirty region (union of all paint-dirty node rects) in physical pixels.
///
/// Returns `None` if no nodes are dirty. For every dirty node the region takes
/// two rects:
///
/// - **where it is now**, grown by how far its own ink (outset `box-shadow`,
///   `outline`) reaches past its box;
/// - **where it was last painted** ([`previous_painted_rect`]), from the
///   parent-relative state the last paint recorded
///   ([`NodeTree::consume_paint_dirty`]) summed up its box-tree chain — so it
///   is exact however many layout passes ran since, when an ancestor moved
///   rather than the node, and when its ink or transform changed in the same
///   frame.
///
/// A **positioned** box (`absolute`, `fixed` or `relative`) whose position
/// changed also grows both rects by how far its painted subtree reaches past
/// it ([`opacity_layer_bounds`]): a dragged panel's overflowing children move
/// with it without being dirty themselves, and so does the absolute badge of a
/// `relative` row, whether an author moved it or reflow shifted it — the
/// resized parent's rect does not reach a descendant that overflows it.
/// Descendants that *are* dirty or removed bring their own painted rects,
/// which is what keeps a child that closed or flipped side in the same frame
/// covered. Static boxes are not walked: walking every row a reflow shifts
/// made a frame cost O(rows × subtree). What that leaves out is a static
/// box's in-flow child whose own ink (a spread shadow) reaches past a parent
/// that reflow shifted.
///
/// Once the region reaches [`FULL_REPAINT_FRACTION`] of the surface the answer
/// is the whole surface, and nothing further is measured.
///
/// This is the bounding box of [`compute_damage`]'s rects (the whole surface
/// when that is full), kept for callers that want one rect. The software
/// renderer paints the rects themselves.
pub fn compute_dirty_region(
    tree: &NodeTree,
    scale: f64,
    viewport_w: f64,
    viewport_h: f64,
) -> Option<Rect> {
    if tree.paint_dirty_nodes.is_empty() {
        return None;
    }
    let damage = compute_damage(tree, scale, viewport_w, viewport_h);
    if damage.is_full() {
        return Some(Rect::new(0.0, 0.0, viewport_w, viewport_h));
    }
    damage.bounds()
}

/// The damage the paint-dirty nodes name, as a short list of rects in
/// physical pixels ([`DamageRegion`]). The rects are the ones
/// [`compute_dirty_region`] documents; they are merged only where they overlap
/// or where one rect is cheaper than two, so two small changes far apart cost
/// their own areas and not the span between them.
///
/// Empty when nothing paint-dirty is on screen. Full once the rects' **total**
/// area reaches [`FULL_REPAINT_FRACTION`] of the surface, or when a moved
/// subtree cannot be bounded.
pub fn compute_damage(
    tree: &NodeTree,
    scale: f64,
    viewport_w: f64,
    viewport_h: f64,
) -> DamageRegion {
    let mut region = DamageRegion::new(viewport_w, viewport_h);
    if tree.paint_dirty_nodes.is_empty() {
        return region;
    }

    let margin = 4.0; // pixels margin for anti-aliasing
    // Whether the node being measured has named any rect yet. A paint-dirty
    // node that names none is not "nothing changed": see the fallback at the
    // end of the loop.
    let named = Cell::new(false);
    // `clip`: what the rect's node is clipped to (`clip_chain_bounds`, #909).
    // A rect clipped away entirely still counts as *named* — the node said
    // where it paints, and it paints nowhere visible — so the owner fallback
    // below does not stand in for it with a box the clip was cutting away.
    let mut add = |r: Rect, clip: Option<Rect>| -> bool {
        if r.width() > 0.0 && r.height() > 0.0 {
            named.set(true);
        }
        let r = Rect::new(r.x0 - margin, r.y0 - margin, r.x1 + margin, r.y1 + margin);
        let r = match clip {
            // Tested before `add` snaps it: an empty intersection at a
            // fractional edge would snap out to a 1px sliver.
            Some(c) => match r.intersect(c) {
                r if r.width() > 0.0 && r.height() > 0.0 => r,
                _ => return false,
            },
            None => r,
        };
        region.add(r)
    };
    let clip_now = |id: RawNodeId| clip_chain_bounds(tree, id, scale, false);
    let clip_then = |id: RawNodeId| clip_chain_bounds(tree, id, scale, true);

    // Deduplicate — paint_dirty_nodes may have duplicates
    let mut seen = HashSet::new();
    for &node_id in &tree.paint_dirty_nodes {
        if !seen.insert(node_id) {
            continue;
        }
        let Some(node) = tree.get(node_id) else {
            continue;
        };
        named.set(false);

        // Current position. CSS transforms displace where a node renders, so
        // use the transform-aware absolute rect — the region must cover the
        // node's visual position, not its untransformed layout rect (#143).
        let (ax, ay, transform) = compute_absolute_position_and_transform(tree, node_id, scale);
        let w = node.layout.width as f64 * scale;
        let h = node.layout.height as f64 * scale;

        let mut ink = Outsets::from_css(own_ink_outsets(&node.computed_style), scale);
        // A positioned box that moved carries its whole painted subtree with
        // it (see the doc above). A subtree the walk cannot bound answers
        // `UNBOUNDED` and the frame repaints in full.
        let moved = node.painted.is_some()
            && (node.prev_layout.x != node.layout.x || node.prev_layout.y != node.layout.y);
        if moved
            && w > 0.0
            && h > 0.0
            && matches!(
                node.computed_style.position,
                PositionValue::Absolute | PositionValue::Fixed | PositionValue::Relative
            )
        {
            let bounds = opacity_layer_bounds(tree, node_id, scale, ax, ay);
            if bounds == UNBOUNDED {
                return DamageRegion::full(viewport_w, viewport_h);
            }
            ink = ink.max(Outsets {
                left: ax - bounds.x0,
                top: ay - bounds.y0,
                right: bounds.x1 - (ax + w),
                bottom: bounds.y1 - (ay + h),
            });
        }

        if w > 0.0
            && h > 0.0
            && add(
                transform.transform_rect_bbox(ink.grow(Rect::new(ax, ay, ax + w, ay + h))),
                clip_now(node_id),
            )
        {
            return DamageRegion::full(viewport_w, viewport_h);
        }

        // A flowed inline element owns no box (`0x0`): its glyphs, its
        // decorations and its background are drawn by the IFC root it
        // flows into. So a restyle that changes only how it *paints* —
        // `visibility` above all, which is read at paint time and moves
        // nothing (#829) — has to dirty that root's rect, or the change
        // never reaches the incremental frame. Only for a box-less node:
        // anything with a box of its own is covered by that box.
        //
        // Deliberately NOT recorded in `seen`: the root may be paint-dirty
        // in its own right (it moved), and its own entry must still add
        // its *previous* rect. Marking it seen here skipped that and left
        // the line painted where it used to be (#844 review, round 2). A
        // root reached from several spans adds the same rect repeatedly,
        // which the union absorbs.
        if (w <= 0.0 || h <= 0.0)
            && let Some(root_id) = node.ifc_root
            && root_id != node_id
            && let Some(root) = tree.get(root_id)
        {
            let (rx, ry, rt) = compute_absolute_position_and_transform(tree, root_id, scale);
            let rw = root.layout.width as f64 * scale;
            let rh = root.layout.height as f64 * scale;
            if rw > 0.0
                && rh > 0.0
                && add(
                    rt.transform_rect_bbox(Rect::new(rx, ry, rx + rw, ry + rh)),
                    clip_now(root_id),
                )
            {
                return DamageRegion::full(viewport_w, viewport_h);
            }
        }

        // A box-less element that is not a flowed inline — a
        // `display: contents` wrapper, a zero-size parent or positioned
        // anchor — still has children whose paint its restyle or move can
        // reach (an inherited colour, a descendant selector, a tooltip under a
        // 0x0 anchor). Its own rect is empty, so cover what its subtree paints
        // now and, when it has been painted before, the same subtree where it
        // was then: children that did not move relative to it are not dirty
        // themselves and would otherwise be left behind.
        if (w <= 0.0 || h <= 0.0)
            && node.ifc_root.is_none()
            && !node.children.is_empty()
            && !matches!(node.computed_style.display, DisplayValue::None)
        {
            // A subtree that paints nothing now — a block whose only content
            // was just hidden — adds nothing here; its old pixels are named by
            // its painted rect below.
            let Some(bounds) = subtree_paint_extent(tree, node_id, scale, ax, ay) else {
                return DamageRegion::full(viewport_w, viewport_h);
            };
            if bounds.width() > 0.0 && bounds.height() > 0.0 {
                // Unclipped: the subtree's own boxes may escape clippers the
                // wrapper does not (an absolute child), and one chain cannot
                // speak for all of them.
                if add(transform.transform_rect_bbox(bounds), None) {
                    return DamageRegion::full(viewport_w, viewport_h);
                }
                if node.painted.is_some() {
                    let (px, py, pt) =
                        position_and_transform_in(tree, node_id, scale, Frame::Painted);
                    let old = bounds + (Vec2::new(px - ax, py - ay));
                    if add(pt.transform_rect_bbox(old), None) {
                        return DamageRegion::full(viewport_w, viewport_h);
                    }
                }
            }
        }

        // Where it was last painted, with the ink it was painted with — and,
        // for a moved out-of-flow box, the subtree reach measured above (the
        // same subtree, translated; a descendant that changed brings its own
        // painted rect).
        if let Some(r) = painted_rect_with(tree, node_id, scale, ink)
            && add(r, clip_then(node_id))
        {
            return DamageRegion::full(viewport_w, viewport_h);
        }

        // A node that was marked paint-dirty and named no rect at all — an
        // `<option>` (`display: none`, painted by its `<select>`), a text node
        // under one, an element painted by an ancestor — still changed
        // something. Damage the ancestor that paints it: the nearest one with
        // a box, where it is and where it was painted. A node that is detached
        // or inside a `display: none` subtree paints nothing and names nothing.
        if !named.get()
            && let Some(owner) = boxed_owner(tree, node_id)
        {
            if let Some(owner_node) = tree.get(owner) {
                let (ox, oy, ot) = compute_absolute_position_and_transform(tree, owner, scale);
                let ow = owner_node.layout.width as f64 * scale;
                let oh = owner_node.layout.height as f64 * scale;
                let ink = Outsets::from_css(own_ink_outsets(&owner_node.computed_style), scale);
                if add(
                    ot.transform_rect_bbox(ink.grow(Rect::new(ox, oy, ox + ow, oy + oh))),
                    clip_now(owner),
                ) {
                    return DamageRegion::full(viewport_w, viewport_h);
                }
            }
            if let Some(r) = painted_rect_with(tree, owner, scale, Outsets::ZERO)
                && add(r, clip_then(owner))
            {
                return DamageRegion::full(viewport_w, viewport_h);
            }
        }
    }

    // Include rects from removed nodes (saved before deletion).
    for &(rx, ry, rw, rh) in &tree.paint_dirty_removed_rects {
        // Rects stored at scale=1; apply current scale
        if rw > 0.0
            && rh > 0.0
            && add(
                Rect::new(rx * scale, ry * scale, (rx + rw) * scale, (ry + rh) * scale),
                None,
            )
        {
            return DamageRegion::full(viewport_w, viewport_h);
        }
    }

    // Every rect was clamped to the surface as it was added.
    region
}

/// The screen rect every clipping ancestor of `node_id` confines its paint to
/// — in the current frame, or as it was last painted — or `None` when nothing
/// is known to clip it (#909).
///
/// Damage is intersected with it: a change a scroller clips away cannot reach
/// a pixel, so a row moved below the scroller's viewport names nothing. The
/// walk follows what clips a box in paint, and errs **wide** in the cases
/// below, since a rect too large costs a repaint and a rect too small leaves
/// stale pixels:
///
/// - a `position: fixed` box, or anything inside one, is clipped by nothing
///   above the fixed box (paint can clip one more than that, #549 — never
///   less);
/// - an `absolute` box escapes every clipper below its containing block; the
///   block's own clip applies, as it does in `Collector::span`;
/// - a clipper with no box (`display: contents`, a degenerate axis) is not
///   taken; nothing above the body is asked (paint starts there).
///
/// `painted` asks about the pixels on screen: each node's clipping, position
/// and box **as it was painted** (`PaintedState`, `prev_layout`), placed by
/// [`Frame::Painted`], and it answers `None` — no clip — the moment a node on
/// the walk was never painted. It walks the **current** parents, which is
/// right for a node still under the parents it was painted under. The moves
/// see to that: a move from one parent to another records the subtree's old
/// rects under the old chain and forgets its painted state first
/// (`record_pixels_left_by_move`), as a removal does. **Not every detach
/// forgets**, and these are gaps, all pre-existing (#966): `set_text_content`
/// orphans an element's children with `parent = None` and their painted state
/// kept, `replace_node` does the same to the node it displaces, and
/// `set_inner_html` frees children without recording their rects. Such a
/// node, attached again, has its old rect placed along its new parents.
/// `Frame::Painted` also places a box by its *current* `position` (a
/// same-frame static↔fixed change, #961).
///
/// Without `painted` the question is the next paint's, and the current state
/// is exactly what that paint clips with.
pub(crate) fn clip_chain_bounds(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    painted: bool,
) -> Option<Rect> {
    clip_chain_bounds_counted(tree, node_id, scale, painted, &mut 0)
}

/// [`clip_chain_bounds`], adding to `steps` one per ancestor it walks.
pub(crate) fn clip_chain_bounds_counted(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    painted: bool,
    steps: &mut u64,
) -> Option<Rect> {
    let frame = if painted {
        Frame::Painted
    } else {
        Frame::Current
    };
    let style = |node: &Node| -> Option<PaintedStyle> {
        if painted {
            node.painted.as_ref().map(PaintedStyle::then)
        } else {
            Some(PaintedStyle::now(node))
        }
    };
    let node = tree.get(node_id)?;
    let mut position = style(node)?.position;
    // Skipping clippers until the containing block of an absolute box.
    let mut escaping = false;
    let mut clip: Option<Rect> = None;
    let mut current = crate::RinchDocument::box_tree_parent(&tree.nodes, node_id);
    loop {
        match position {
            PositionValue::Fixed => break,
            PositionValue::Absolute => escaping = true,
            _ => {}
        }
        let Some(id) = current else { break };
        *steps += 1;
        let ancestor = tree.get(id)?;
        let a = style(ancestor)?;
        // The containing block ends the escape, and its own clip applies: a
        // `position: relative; overflow: hidden` box clips its absolute
        // children (`Collector::span` counts the chain after its clip, too).
        if escaping && a.contains_abs {
            escaping = false;
        }
        if !escaping && a.clips {
            let size = match frame {
                Frame::Current => ancestor.layout,
                Frame::Painted => ancestor.prev_layout,
            };
            let (w, h) = (size.width as f64 * scale, size.height as f64 * scale);
            if w > 0.0 && h > 0.0 {
                let (x, y, t) = position_and_transform_in(tree, id, scale, frame);
                let r = t.transform_rect_bbox(Rect::new(x, y, x + w, y + h));
                clip = Some(clip.map_or(r, |c| c.intersect(r)));
            }
        }
        // Paint starts at the body: nothing above it clips anything, and
        // nothing above it is ever painted (so has no painted state to ask).
        if id == tree.body_id {
            break;
        }
        position = a.position;
        current = crate::RinchDocument::box_tree_parent(&tree.nodes, id);
    }
    clip
}

/// The style facts [`clip_chain_bounds`] walks on.
#[derive(Clone, Copy)]
struct PaintedStyle {
    clips: bool,
    position: PositionValue,
    contains_abs: bool,
}

impl PaintedStyle {
    /// What a node was painted with. A [`PaintedState`](crate::node::PaintedState)
    /// is written for a node by every paint that consumes it and, after a
    /// whole-document restyle, for every painted node
    /// ([`NodeTree::consume_paint_dirty`]) — and any other style change
    /// pushes the node, so between two consumptions nothing it records moves.
    fn then(painted: &crate::node::PaintedState) -> Self {
        Self {
            clips: painted.clips,
            position: painted.position,
            contains_abs: painted.contains_abs,
        }
    }

    fn now(node: &Node) -> Self {
        Self {
            clips: node.clips_overflow(),
            position: node.computed_style.position,
            contains_abs: node.establishes_abs_containing_block(),
        }
    }
}

/// The ancestor whose paint covers `node_id` when `node_id` names no rect of
/// its own: a `<select>` for anything inside it (it paints its options), else
/// the nearest ancestor with a non-empty box, now or as it was last painted. `None` when the node is not in
/// the document or has a `display: none` *ancestor* (other than a select's
/// options) — it paints nothing, so there is nothing to repaint. The node's own
/// `display` is not asked: one that has just become `display: none` still has
/// last frame's pixels on screen.
fn boxed_owner(tree: &NodeTree, node_id: RawNodeId) -> Option<RawNodeId> {
    // Connected to the document at all? A removed or not-yet-inserted node
    // keeps whatever layout it last had and must not name its old ancestors.
    let mut cur = Some(node_id);
    let mut connected = false;
    while let Some(id) = cur {
        if id == tree.root_id {
            connected = true;
            break;
        }
        cur = tree.get(id).and_then(|n| n.parent);
    }
    if !connected {
        return None;
    }
    // Only an *ancestor* being hidden means "paints nothing": the node itself
    // may have just become `display: none` (a span leaving its line), and what
    // it painted last frame is still on screen.
    let hidden = |n: &Node| {
        n.tag().is_some()
            && matches!(n.computed_style.display, DisplayValue::None)
            && !matches!(n.tag(), Some("option" | "optgroup"))
    };
    let mut cur = tree.get(node_id)?.parent;
    while let Some(id) = cur {
        let n = tree.get(id)?;
        if n.tag() == Some("select") {
            return Some(id);
        }
        if hidden(n) {
            return None;
        }
        // A box now, or a box it was painted with: the node's old pixels are
        // inside whichever it had last frame. A paragraph whose only span was
        // just hidden collapses to 0 tall, and asking only about today's box
        // climbed past it to the page and repainted everything.
        let painted_box =
            n.painted.is_some() && n.prev_layout.width > 0.0 && n.prev_layout.height > 0.0;
        if (n.layout.width > 0.0 && n.layout.height > 0.0) || painted_box {
            return Some(id);
        }
        cur = n.parent;
    }
    None
}

/// The box at `(x, y)`, `w` x `h` physical px, grown by the node's own ink
/// ([`own_ink_outsets`], scaled): what the paint prune tests. Free for a node
/// with no shadow and no outline.
fn ink_rect(
    cs: &crate::computed_style::ComputedStyle,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    scale: f64,
) -> Rect {
    let r = Rect::new(x, y, x + w, y + h);
    if cs.box_shadow.is_empty() && cs.text_shadow.is_empty() && cs.outline_width <= 0.0 {
        return r;
    }
    Outsets::from_css(own_ink_outsets(cs), scale).grow(r)
}

/// How far a node's **own** ink reaches past its border box, in CSS px:
/// `[left, top, right, bottom]`. Outset `box-shadow` (the whole blur radius
/// plus spread, around its offset — the same slack `layer_bounds` allows),
/// `outline` (width plus a positive offset) and `text-shadow` (one and a half
/// blur radii — three standard deviations — around its offset, #980). Inset
/// shadows paint inside the box. Never negative.
///
/// A `text-shadow` is measured from the border box, where the text it
/// shadows usually is; it is inherited, so a text node's parent and every IFC
/// root carry it, and an inline element's damage falls back to the box that
/// paints it. Text that overflows its box casts a shadow past this reach.
pub(crate) fn own_ink_outsets(cs: &crate::computed_style::ComputedStyle) -> [f32; 4] {
    let mut o = [0.0_f32; 4];
    for shadow in &cs.text_shadow {
        let reach = shadow.blur_radius.abs() * text_shadow::REACH_PER_BLUR as f32;
        o[0] = o[0].max(reach - shadow.offset_x);
        o[1] = o[1].max(reach - shadow.offset_y);
        o[2] = o[2].max(reach + shadow.offset_x);
        o[3] = o[3].max(reach + shadow.offset_y);
    }
    for shadow in &cs.box_shadow {
        if shadow.inset {
            continue;
        }
        let reach = shadow.blur_radius.abs() + shadow.spread_radius.abs();
        o[0] = o[0].max(reach - shadow.offset_x);
        o[1] = o[1].max(reach - shadow.offset_y);
        o[2] = o[2].max(reach + shadow.offset_x);
        o[3] = o[3].max(reach + shadow.offset_y);
    }
    if cs.outline_width > 0.0 {
        let reach = cs.outline_width + cs.outline_offset.max(0.0);
        for side in &mut o {
            *side = side.max(reach);
        }
    }
    o
}

/// The rect `node_id`'s pixels are in from the last paint, in physical pixels
/// — `None` when it has not been painted since it was created (or removed), or
/// was painted with an empty box.
///
/// Its position is the painted-frame sum up its box-tree chain
/// ([`Frame::Painted`]), so it is right whether the node or an ancestor moved
/// since, and under a transform that changed since; it is grown by the ink the
/// node was painted with. The removal path records it for every node of a
/// removed subtree; [`compute_dirty_region`] adds it for every dirty node.
pub fn previous_painted_rect(tree: &NodeTree, node_id: RawNodeId, scale: f64) -> Option<Rect> {
    painted_rect_with(tree, node_id, scale, Outsets::ZERO)
}

/// [`previous_painted_rect`], additionally grown by `extra` (physical px).
fn painted_rect_with(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    extra: Outsets,
) -> Option<Rect> {
    let node = tree.get(node_id)?;
    let painted = node.painted.as_ref()?;
    let pw = node.prev_layout.width as f64 * scale;
    let ph = node.prev_layout.height as f64 * scale;
    if pw <= 0.0 || ph <= 0.0 {
        return None;
    }
    let (px, py, pt) = position_and_transform_in(tree, node_id, scale, Frame::Painted);
    let ink = Outsets::from_css(painted.ink, scale).max(extra);
    Some(pt.transform_rect_bbox(ink.grow(Rect::new(px, py, px + pw, py + ph))))
}

/// How far past a border box something reaches, per side, in physical pixels.
/// Never negative.
#[derive(Clone, Copy)]
struct Outsets {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

impl Outsets {
    const ZERO: Self = Self {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    };

    fn from_css(o: [f32; 4], scale: f64) -> Self {
        Self {
            left: o[0] as f64 * scale,
            top: o[1] as f64 * scale,
            right: o[2] as f64 * scale,
            bottom: o[3] as f64 * scale,
        }
    }

    fn max(self, o: Self) -> Self {
        Self {
            left: self.left.max(o.left).max(0.0),
            top: self.top.max(o.top).max(0.0),
            right: self.right.max(o.right).max(0.0),
            bottom: self.bottom.max(o.bottom).max(0.0),
        }
    }

    fn grow(self, r: Rect) -> Rect {
        Rect::new(
            r.x0 - self.left,
            r.y0 - self.top,
            r.x1 + self.right,
            r.y1 + self.bottom,
        )
    }
}

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

/// Pixel data for a render surface, keyed by surface ID.
pub struct SurfacePixelData {
    /// RGBA8 pixel data. Shared with the surface's own buffer, so handing a
    /// frame to paint copies nothing — a paint with no new frame used to clone
    /// the whole frame just to have it on hand (#361).
    pub data: std::sync::Arc<Vec<u8>>,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Every pixel's alpha is 255 — the producer's promise, checked where the
    /// frame was submitted (`SurfaceWriter::submit_frame`, on the submitting
    /// thread). An opaque frame is drawn with no premultiply, and onto whole
    /// pixels (any positive scale, unrotated) as a straight row copy (#361).
    /// `false` promises nothing.
    pub opaque: bool,
}

thread_local! {
    /// Viewport names that have active surface frames this paint cycle.
    /// When set, only these viewports get holes punched in backgrounds.
    /// When empty, ALL viewports get holes (GPU compositor default).
    static ACTIVE_VIEWPORTS: RefCell<Option<HashSet<String>>> = const { RefCell::new(None) };

    /// Dirty region for incremental painting. When set, paint_node can
    /// skip subtrees entirely outside this rect (in physical pixels).
    static DIRTY_REGION: RefCell<Option<Vec<Rect>>> = const { RefCell::new(None) };

    /// Surface pixel data for inline painting, keyed by surface ID.
    /// Set before paint_document() and cleared after.
    static SURFACE_PIXELS: RefCell<Option<HashMap<usize, SurfacePixelData>>> = const { RefCell::new(None) };

    /// Surface pixel data for inline painting, keyed by **viewport name** —
    /// the software backend's video frames (issue #358).
    ///
    /// `SURFACE_PIXELS` is keyed by the `usize` surface id a `RenderSurface`
    /// component stamps into `data-render-surface`; a video viewport carries no
    /// such id, only the `data-viewport` name its player was created with, so
    /// the two registries cannot share a key space. Set before
    /// `paint_document()` and cleared after, like `SURFACE_PIXELS`.
    static VIEWPORT_PIXELS: RefCell<Option<HashMap<String, SurfacePixelData>>> = const { RefCell::new(None) };

    /// The render target for this paint cycle. Nothing outside it can be
    /// seen, so nothing outside it is drawn — see [`intersects_dirty_region`]
    /// for the cull and card K43 for what it is worth.
    ///
    /// Set by `paint_document` and **cleared by it**, like `SURFACE_PIXELS`:
    /// `paint_subtree` paints into a pixmap of its own and has no window, so a
    /// value left behind here would cull against the wrong target.
    static VIEWPORT: Cell<Option<Viewport>> = const { Cell::new(None) };

    /// The clips the painter has open right now, as cull rects: one entry per
    /// `push_clip` or `push_layer` the painter has not yet popped, each holding
    /// the running intersection of every clip beneath it and itself, in the
    /// physical pixels paint works in (`None` = nothing clips yet). Written
    /// only by [`ClipTrackingPainter`], which is what makes it the painter's
    /// own clip stack rather than a second account of it (#910).
    static CLIP_CULL: RefCell<Vec<Option<Rect>>> = const { RefCell::new(Vec::new()) };
}

/// The rect nothing drawn right now can reach past: the intersection of every
/// clip the painter has open, grown by [`INK_MARGIN_CSS_PX`]. `None` while no
/// clip is open.
fn open_clip_cull() -> Option<Rect> {
    CLIP_CULL.with(|c| c.borrow().last().copied().flatten())
}

/// The rect, in the paint's device pixels, that anything drawn right now can
/// be seen in: the render target (grown by the ink margin) intersected with
/// every open clip and with the bounds of a partial repaint's damage. `None`
/// when none is known (`paint_subtree`, no clip, no damage).
pub(super) fn visible_paint_rect() -> Option<Rect> {
    let target = VIEWPORT.with(|v| v.get().map(|vp| vp.cull));
    let visible = match (target, open_clip_cull()) {
        (Some(a), Some(b)) => Some(a.intersect(b)),
        (a, b) => a.or(b),
    };
    // A partial repaint's damage clip is pushed outside `paint_document`'s
    // `ClipTrackingPainter`, so `CLIP_CULL` never sees it; its rects are here.
    let damage = DIRTY_REGION.with(|v| {
        v.borrow()
            .as_ref()
            .and_then(|rects| rects.iter().copied().reduce(|a, b| a.union(b)))
    });
    match (visible, damage) {
        (Some(a), Some(b)) => Some(a.intersect(b)),
        (a, b) => a.or(b),
    }
}

/// A [`Painter`] that forwards everything and keeps [`CLIP_CULL`] in step with
/// the clips it forwards (#910).
///
/// The dirty-region and window culls say what nothing can be *seen* outside;
/// this one says what nothing can be *drawn* outside — a scroller's rows past
/// its clip are on the window and inside a dirty rect, and are clipped away
/// all the same. Mirroring the painter's own stack, rather than pushing a cull
/// rect at the call sites that open a bracket, is what makes it exact by
/// construction: a hoisted entry's clip chain, the #545 lift of a stacking
/// root's bracket around a `position: fixed` entry (a `pop_layer` and a
/// `push_clip` of the same shape), and a bracket card K43 elided are all
/// whatever the painter was actually told, with no site to forget.
///
/// A layer pushes an unchanged copy of the entry beneath it: `pop_layer` pops
/// a clip and a layer alike, and a layer's bounds are no clip on the software
/// path. The shape's transformed **bounding box** stands for the shape — never
/// smaller than what it clips to — grown by the ink margin, for the same
/// reason the window's cull rect is: a box's layout rect is not the last word
/// on where it puts ink, and a shadow cast back into a scroller's viewport by
/// a row just past its edge is inside the clip.
struct ClipTrackingPainter<'a> {
    inner: &'a mut dyn Painter,
    ink_margin: f64,
}

impl ClipTrackingPainter<'_> {
    /// Run `f` with `painter` wrapped, on a clip-cull stack of its own. The
    /// stack of an enclosing paint, if any, is set aside and put back.
    ///
    /// The paint's blurred `text-shadow` bookkeeping runs with it
    /// ([`text_shadow::begin_paint`]), and the masks it rasterised are counted
    /// on `perf` (`text_shadow_masks_rasterised`).
    fn run<R>(
        painter: &mut dyn Painter,
        scale: f64,
        perf: &crate::perf::PerfCounters,
        f: impl FnOnce(&mut dyn Painter) -> R,
    ) -> R {
        text_shadow::begin_paint();
        let saved = CLIP_CULL.with(|c| std::mem::take(&mut *c.borrow_mut()));
        let mut tracking = ClipTrackingPainter {
            inner: painter,
            ink_margin: INK_MARGIN_CSS_PX * scale,
        };
        let out = f(&mut tracking);
        CLIP_CULL.with(|c| *c.borrow_mut() = saved);
        perf.add(
            crate::perf::Counter::TextShadowMasksRasterised,
            text_shadow::end_paint(),
        );
        out
    }
}

impl Painter for ClipTrackingPainter<'_> {
    fn reset(&mut self) {
        CLIP_CULL.with(|c| c.borrow_mut().clear());
        self.inner.reset();
    }
    fn fill(&mut self, fill: Fill, transform: Affine, brush: &Brush, shape: &PaintShape) {
        self.inner.fill(fill, transform, brush, shape);
    }
    fn fill_color(
        &mut self,
        fill: Fill,
        transform: Affine,
        color: AlphaColor<Srgb>,
        shape: &PaintShape,
    ) {
        self.inner.fill_color(fill, transform, color, shape);
    }
    fn stroke(
        &mut self,
        stroke: &peniko::kurbo::Stroke,
        transform: Affine,
        brush: &Brush,
        shape: &PaintShape,
    ) {
        self.inner.stroke(stroke, transform, brush, shape);
    }
    fn stroke_color(
        &mut self,
        stroke: &peniko::kurbo::Stroke,
        transform: Affine,
        color: AlphaColor<Srgb>,
        shape: &PaintShape,
    ) {
        self.inner.stroke_color(stroke, transform, color, shape);
    }
    fn draw_glyphs(
        &mut self,
        font: &peniko::FontData,
        font_size: f32,
        transform: Affine,
        glyph_transform: Option<Affine>,
        brush: &Brush,
        hint: bool,
        normalized_coords: &[i16],
        glyphs: &[painter::PaintGlyph],
    ) {
        self.inner.draw_glyphs(
            font,
            font_size,
            transform,
            glyph_transform,
            brush,
            hint,
            normalized_coords,
            glyphs,
        );
    }
    fn draw_image(&mut self, image: &painter::PaintImage<'_>, transform: Affine) {
        self.inner.draw_image(image, transform);
    }
    fn draw_alpha_mask(
        &mut self,
        mask: &[u8],
        width: u32,
        height: u32,
        color: AlphaColor<Srgb>,
        transform: Affine,
    ) {
        self.inner
            .draw_alpha_mask(mask, width, height, color, transform);
    }
    fn push_clip(&mut self, fill: Fill, transform: Affine, shape: &PaintShape) {
        let bounds = transform
            .transform_rect_bbox(shape.bounding_box())
            .inflate(self.ink_margin, self.ink_margin);
        CLIP_CULL.with(|c| {
            let mut c = c.borrow_mut();
            let next = match c.last().copied().flatten() {
                Some(open) => open.intersect(bounds),
                None => bounds,
            };
            c.push(Some(next));
        });
        self.inner.push_clip(fill, transform, shape);
    }
    fn push_layer(
        &mut self,
        blend: BlendMode,
        opacity: f32,
        transform: Affine,
        bounds: &PaintShape,
    ) {
        CLIP_CULL.with(|c| {
            let mut c = c.borrow_mut();
            let open = c.last().copied().flatten();
            c.push(open);
        });
        self.inner.push_layer(blend, opacity, transform, bounds);
    }
    fn push_isolated_layer(&mut self, opacity: f32, transform: Affine, bounds: &PaintShape) {
        CLIP_CULL.with(|c| {
            let mut c = c.borrow_mut();
            let open = c.last().copied().flatten();
            c.push(open);
        });
        self.inner.push_isolated_layer(opacity, transform, bounds);
    }
    fn pop_layer(&mut self) {
        CLIP_CULL.with(|c| {
            c.borrow_mut().pop();
        });
        self.inner.pop_layer();
    }
}

/// The render target, and the rect outside which paint emits nothing.
///
/// Two rects rather than one because the two questions want different answers.
/// `target` is the window exactly, and a clip that contains it is removing only
/// discarded pixels. `cull` is `target` grown by [`INK_MARGIN_CSS_PX`], because
/// a node's layout box is not its ink: a `box-shadow`, an `outline`, or a text
/// run that overflows its own box all reach past the rect being tested, and
/// none of them are in it. Deriving both once, here, is what keeps the margin
/// out of the comparison sites — where it was a bare physical-pixel constant
/// that shrank to a third of its intended size on a 3x display.
#[derive(Clone, Copy)]
struct Viewport {
    target: Rect,
    cull: Rect,
}

/// How far past the window a box may be and still paint ink inside it, in
/// **CSS** pixels — the units the thing it protects is authored in.
///
/// The framework's own largest shadow (`--rinch-shadow-xl`) reaches
/// `36 + 28 - 7 = 57` CSS px past its box, and `paint_box_shadow` stops
/// shorter than that again (`blur * 0.5 + spread`), so this clears it with
/// room. It is not a bound on what an *application* can author: a
/// `box-shadow: 0 0 200px` on a box 100px below the fold will be culled and
/// its shadow lost. Computing each node's true ink extent instead — which
/// `layer_bounds` knows how to do — costs more per frame than the cull saves,
/// which is why this is a margin and not a measurement.
const INK_MARGIN_CSS_PX: f64 = 64.0;

/// Set the active viewport names for hole-punching during this paint cycle.
///
/// Only viewports in this set will have holes cut in ancestor backgrounds.
/// Call with `None` to revert to the default (all viewports get holes).
pub fn set_active_viewports(names: Option<HashSet<String>>) {
    ACTIVE_VIEWPORTS.with(|v| *v.borrow_mut() = names);
}

/// Set surface pixel data for inline painting during the current paint cycle.
///
/// Call with `Some(map)` before `paint_document()` and `None` after.
/// When set, `paint_node()` will draw surface pixels inline at the
/// element's position, like `<img>` elements.
pub fn set_surface_pixels(pixels: Option<HashMap<usize, SurfacePixelData>>) {
    SURFACE_PIXELS.with(|v| *v.borrow_mut() = pixels);
}

/// Set viewport frame data for inline painting during the current paint cycle,
/// keyed by `data-viewport` name.
///
/// This is the **software** backend's path for video (issue #358) and
/// `GameViewport` (issue #361). A `data-viewport` node with an entry here paints
/// its frame inline, during paint, at its own z-order — so anything drawn above
/// it (a drawer, a modal, a dropdown, a game's HUD) covers it by ordinary paint
/// order. A node with no entry falls through to normal element painting, which
/// is what leaves the GPU compositor path untouched: that backend never sets
/// this map, so every `data-viewport` node there still paints as a plain
/// element and gets its hole punched.
///
/// A node that also punches a hole ([`set_active_viewports`]) paints its frame
/// straight into that hole; one that does not paints an opaque black backdrop
/// under it first — the letterbox a browser paints for `<video>`.
///
/// Call with `Some(map)` before `paint_document()` and `None` after.
pub fn set_viewport_pixels(pixels: Option<HashMap<String, SurfacePixelData>>) {
    VIEWPORT_PIXELS.with(|v| *v.borrow_mut() = pixels);
}

/// Whether a **usable** inline frame is available for the viewport named
/// `name`.
///
/// The dimensions are validated here, exactly as the `data-render-surface` arm
/// validates its own before taking the inline path. An entry whose pixels
/// cannot be drawn — zero-sized, or a buffer shorter than `width * height * 4`
/// (`submit_frame` only `debug_assert!`s that, so a release build can deliver
/// one) — must leave the node on the ordinary element path and its `#000`
/// placeholder background, rather than take the inline arm and paint a bare
/// black box with no frame inside it.
fn has_viewport_pixels(name: &str) -> bool {
    VIEWPORT_PIXELS.with(|v| {
        v.borrow()
            .as_ref()
            .and_then(|map| map.get(name))
            .is_some_and(|pixels| viewport_frame_bytes(pixels).is_some())
    })
}

/// The byte length a frame must have to be drawable, or `None` if it is not.
fn viewport_frame_bytes(pixels: &SurfacePixelData) -> Option<usize> {
    if pixels.width == 0 || pixels.height == 0 {
        return None;
    }
    let needed = pixels.width as usize * pixels.height as usize * 4;
    (pixels.data.len() >= needed).then_some(needed)
}

/// Set the dirty region for incremental painting.
///
/// When set, `paint_node` will skip subtrees whose bounds are entirely
/// outside this rect, avoiding expensive glyph rasterization and path
/// operations for unchanged content. Set to `None` for full repaint.
pub fn set_dirty_region(region: Option<Rect>) {
    DIRTY_REGION.with(|v| *v.borrow_mut() = region.map(|r| vec![r]));
}

/// [`set_dirty_region`] for several rects: a node is painted when it touches
/// any of them. `None` means a full repaint; an empty slice culls everything.
pub fn set_dirty_rects(rects: Option<&[Rect]>) {
    DIRTY_REGION.with(|v| *v.borrow_mut() = rects.map(<[Rect]>::to_vec));
}

/// The part of the device that anything drawn right now can show on: the
/// render target, the clips the painter has open and the damage, intersected.
/// `None` when none of them bounds it (a `paint_subtree` with no clip open).
/// What a blurred `text-shadow` crops its mask to (#980).
#[cfg(feature = "software-renderer")]
pub(super) fn visible_device_rect() -> Option<Rect> {
    let mut r = VIEWPORT.with(|v| v.get().map(|vp| vp.target));
    let mut meet = |b: Rect| r = Some(r.map_or(b, |a| a.intersect(b)));
    if let Some(c) = open_clip_cull() {
        meet(c);
    }
    let damage = DIRTY_REGION.with(|v| {
        v.borrow().as_ref().map(|rects| {
            rects.iter().fold(Rect::ZERO, |acc: Rect, d| {
                if acc.area() == 0.0 { *d } else { acc.union(*d) }
            })
        })
    });
    if let Some(d) = damage {
        meet(d);
    }
    r
}

/// Check whether a node rect can put anything on screen this paint: it must
/// touch the dirty region (always, on a full repaint), the window's cull rect,
/// and the cull rect of the clips the painter has open ([`CLIP_CULL`], #910).
fn intersects_dirty_region(x: f64, y: f64, w: f64, h: f64) -> bool {
    let inside_dirty = DIRTY_REGION.with(|v| {
        let guard = v.borrow();
        match guard.as_ref() {
            None => true, // No dirty region → full repaint, paint everything
            Some(rects) => rects
                .iter()
                .any(|dr| x < dr.x1 && x + w > dr.x0 && y < dr.y1 && y + h > dr.y0),
        }
    });
    if !inside_dirty {
        return false;
    }

    // …and against every clip the painter has open (#910). A scroller's rows
    // below its viewport are on the window and, often, inside the dirty rect —
    // and are clipped away all the same, so drawing them is work with no
    // output. See [`ClipTrackingPainter`].
    if let Some(clip) = open_clip_cull()
        && !(x < clip.x1 && x + w > clip.x0 && y < clip.y1 && y + h > clip.y0)
    {
        return false;
    }

    // …and the same test against the render target, which is a dirty region
    // that is always there. A node that falls entirely off the window cannot
    // be seen however clean the frame is, and on the GPU path it is not free
    // to emit: vello culls invisible paths in its coarse stage, but it has
    // already flattened every one of them on the way there.
    //
    // The library screen is the case that motivated this. Its scroller holds
    // all 25 songs whether or not they are on screen, and it carries three
    // bottom sheets parked below the fold, so a frame mid-fling was handing
    // vello 793 glyphs to draw about 240 of, 82 text runs for 28, and 41 clip
    // layers for 21. On the moto g stylus 5G (Adreno 619, 1080x2460) that
    // costs 1.4ms of a 17.8ms frame — measured by waiting on vello's own
    // submission before any swapchain image is involved, which is card K42's
    // technique and the only one that has not lied about this pipeline yet.
    // See card K43.
    //
    // The rect this tests against is deliberately larger than the window; see
    // [`INK_MARGIN_CSS_PX`] for how much larger and why a box's layout rect is
    // not the last word on where it puts ink.
    VIEWPORT.with(|v| match v.get() {
        None => true,
        Some(vp) => x < vp.cull.x1 && x + w > vp.cull.x0 && y < vp.cull.y1 && y + h > vp.cull.y0,
    })
}

/// A node's origin in the space its *own* box is laid out in — the accumulated
/// parent-chain offset, before any composed CSS transform is applied.
///
/// This is [`compute_absolute_position_and_transform`] with the affine dropped,
/// and it exists for the two callers that want exactly that: layout's
/// out-of-flow correction (#204), which writes a **parent-relative** delta back
/// into `LayoutResult` and so must stay in untransformed layout space, and
/// anything else reasoning about where Taffy put a box rather than where paint
/// draws it.
///
/// **It is not a screen rect.** For "where is this box on screen" — the answer
/// a click, a drag or a dirty region needs — use [`painted_border_box`], which
/// applies the transform this function discards.
pub fn compute_absolute_position(tree: &NodeTree, node_id: RawNodeId, scale: f64) -> (f64, f64) {
    let (x, y, _) = compute_absolute_position_and_transform(tree, node_id, scale);
    (x, y)
}

/// The offset from a node's summed layout origin to the origin it is *painted*
/// at, for a box an IFC positions.
///
/// An atomic inline ([`crate::DisplayMode::is_atomic_inline`]) laid out by an
/// inline formatting context stores its
/// `layout.x`/`layout.y` relative to the IFC root's **content** box, while a
/// parent-chain sum like [`compute_absolute_position`] adds up **border**-box
/// origins. Paint bridges the two: it hands `paint_inline_layout` the root's
/// content-box origin, so the box lands one padding+border in from where the
/// sum alone puts it. Anything mapping a screen point back into such a box —
/// hit testing, caret placement — has to add the same offset or it is looking
/// at the box's old address.
///
/// Returns `(0.0, 0.0)` for every box the IFC does not position, which is all
/// of them outside a text flow — and for a box an **anonymous** IFC root
/// positions, whose content-box origin *is* its layout origin (#319).
pub fn ifc_content_box_offset(tree: &NodeTree, node: &Node) -> (f32, f32) {
    if !node.display_mode.is_atomic_inline() {
        return (0.0, 0.0);
    }
    let Some(root) = node.ifc_root.and_then(|id| tree.get(id)) else {
        return (0.0, 0.0);
    };
    ifc_root_content_origin(root)
}

/// Where an IFC root starts drawing its inline content, relative to its
/// border-box origin: one padding+border in — the content-box origin — in
/// unscaled CSS px.
///
/// Every site that positions or bounds what `paint_inline_layout` draws reads
/// this one definition rather than re-summing padding and border by hand:
/// paint's IFC arm, the selection-highlight block, `layer_bounds`' two inline
/// arms, and [`ifc_content_box_offset`] (the child-side bridge that stacking,
/// hit testing and caret placement go through). Hand-rolled copies of this sum
/// are exactly how paint and its geometry consumers drift apart (#466).
///
/// For an **anonymous block box** the answer is `(0, 0)` by construction:
/// [`crate::computed_style::ComputedStyle::for_anonymous_box`] carries no box
/// model (#319), so its laid-out and painted geometry coincide — the parent's
/// padding is applied once, by Taffy, when it places the anonymous box inside
/// the parent's content box, and never again here.
pub(crate) fn ifc_root_content_origin(root: &Node) -> (f32, f32) {
    let cs = &root.computed_style;
    (
        cs.padding_left.to_px() + cs.border_left_width.to_px(),
        cs.padding_top.to_px() + cs.border_top_width.to_px(),
    )
}

/// A box whose IFC has a live inline layout is drawn **by that IFC and by
/// nothing else** — not by a tree-order walk, not by a stacking sequence
/// (#365).
///
/// This replaced a positional predicate that had two independent ways to miss:
///
/// ```text
/// skip_ifc_children && kind != PaintKind::StackingContext && child.ifc_root == Some(node_id)
/// ```
///
/// The third term can never match for a subtree hoisted to an *ancestor* — the
/// box's `ifc_root` names its own IFC, not the node being painted — and the
/// second excluded any inline-level box that is itself a stacking context,
/// even as a direct child of the node doing the painting. Either miss drew the
/// box twice: once by `paint_inline_layout` at the IFC root's **content**
/// origin, once by the stacking sequence at its **border-box** origin, exactly
/// one padding+border apart. `position: relative` alone is enough to reach it
/// (it makes an inline-block `is_positioned_z_auto`), so
/// `<button style="position: relative">` inside a padded paragraph — the
/// ordinary tooltip-anchor idiom — reproduced it.
///
/// `text_layout.is_some()` is load-bearing, not a nicety: an IFC root that is
/// virtualized (`estimated_height`) or has no cached layout draws nothing at
/// all, and skipping its children there would make them **vanish** rather than
/// double. That is what the old `skip_ifc_children` flag was standing in for,
/// positionally and only at the sites that happened to pass it.
///
/// **Known divergence.** The IFC's draw is the survivor, so an inline-level
/// box paints in *inline order* rather than at its `z-index` — visible for
/// something like `<button style="position: relative; z-index: -1">` inside
/// text. Preserving z-order instead means making the hoisted entry the
/// survivor and having `paint_inline_layout` skip boxes that
/// `paints_at_stacking_root`, which needs the offset correction below to be
/// exactly right first. Tracked separately; the simple rule is correct about
/// *where* and *how many*, which is what was broken.
///
/// **#513 is fixed, and this predicate is no longer asked about that shape.**
/// It used to be the second half of the defect: an inline element holding a
/// block was marked and detached, this predicate answered `true` for it, and
/// `paint_children_with_stacking` skipped it and never descended — so the block
/// and everything after it rendered nowhere. A `display: inline` element holding
/// an in-flow block-level box is now **split** around it
/// ([`crate::node::Node::is_split_inline`]), which takes it out of
/// `box_tree_children` entirely: paint reaches its pieces through the anonymous
/// block boxes that lay each run out, and the block directly from the container.
/// This function is never called with such an element, because paint never sees
/// one as a child.
///
/// **What #513's fix does NOT model is fragment identity.** CSS 2.1 §9.2.1.1
/// splits the inline box into one fragment per side of the block, each a real
/// inline box with its own background, border and padding; rinch gives the
/// *geometry* of those fragments and keeps no box for the element itself. So do
/// not read "block-in-inline splitting landed" as "rinch has inline fragments" —
/// it does not, and anything that needs per-fragment boxes is unbuilt.
///
/// **#591 is fixed too, and this predicate was never the reason.** An
/// **out-of-flow** child of an inline element is not split around (CSS 2.1
/// §9.4.2), so the element is still detached whole and this predicate still
/// skips it in the tree-order walk — but the child is *positioned*, so it was
/// never reached through that walk in the first place: the stacking collector
/// descends through the inline (it consults no `ifc_root`) and hoists the box
/// to its stacking root, and `paint_children_with_stacking` paints the entry
/// because the child's own `ifc_root` is `None`. It vanished because it was
/// `0x0` — its Taffy node left the tree with the inline's. It is now a **unit
/// of its host** (`Node::hoisted_out_of_flow_to`, `collect_run_units`): a
/// box-tree child of the block container, so the host's Taffy list, paint
/// sequence, hit-test sequence and — the consumer this predicate's skip used
/// to hide it from — `layer_bounds` all see it directly, and the inline is never
/// descended through to find it. If such a box vanishes now, look at the unit
/// collection and the Taffy edge it produces, not here.
pub(crate) fn drawn_by_its_ifc(tree: &NodeTree, child: &Node) -> bool {
    child
        .ifc_root
        .and_then(|r| tree.get(r))
        .is_some_and(|r| r.text_layout.is_some())
}

/// Compose a node's CSS transform onto `parent_transform`, applied about the
/// node's transform-origin. Percentage-based translate values are resolved
/// against the node's layout box (so they resolve to 0 on a collapsed axis).
/// Returns `parent_transform` unchanged for identity transforms.
///
/// `x`/`y` are the node's absolute position in physical pixels. This is the
/// single source of truth for transform composition — the paint arms, the
/// dirty-region cull test, dirty-region tracking, and hit testing must all
/// agree on it (#142, #143, #199).
///
/// The result is covariant in `scale`: composing at `(s·x, s·y, s)` equals
/// `S · compose(x, y, 1) · S⁻¹` for `S = scale(s)`, which is what lets hit
/// testing pass `scale = 1.0` and work in layout pixels (#202).
pub fn compose_node_transform(
    node: &Node,
    x: f64,
    y: f64,
    scale: f64,
    parent_transform: Affine,
) -> Affine {
    let tf = &node.computed_style.transform;
    if tf.is_identity {
        return parent_transform;
    }
    let cs = &node.computed_style;
    let origin = (
        cs.transform_origin_x.resolve(node.layout.width),
        cs.transform_origin_y.resolve(node.layout.height),
    );
    compose_transform_parts(
        tf,
        origin,
        (node.layout.width, node.layout.height),
        cs.backface_visibility_hidden,
        x,
        y,
        scale,
        parent_transform,
    )
}

/// The arithmetic of [`compose_node_transform`], over explicit inputs: the
/// transform, its origin resolved in CSS px, and the box it resolves a
/// percentage translate against. The painted frame
/// ([`Frame::Painted`]) composes the transform a node was *painted* with this
/// way, from [`crate::node::PaintedState`].
///
/// A transform that turns the element's back to the viewer under
/// `backface_hidden` (`backface-visibility: hidden`, #997) composes to the zero
/// matrix, the path `scale(0)` and a plane behind the viewer already take: the
/// element and its subtree are neither drawn nor hit — except a `position:
/// fixed` descendant, which paints under the body's transform and so never
/// meets this one (no containment, #386/#415).
#[allow(clippy::too_many_arguments)]
fn compose_transform_parts(
    tf: &crate::computed_style::TransformValue,
    origin: (f32, f32),
    size: (f32, f32),
    backface_hidden: bool,
    x: f64,
    y: f64,
    scale: f64,
    parent_transform: Affine,
) -> Affine {
    if backface_hidden && tf.back_facing {
        return parent_transform * Affine::new([0.0; 6]);
    }
    let mut m = tf.matrix;
    // A percentage translate resolves against the element's own border box —
    // but *in the frame its position in the function list establishes*, so its
    // contribution is a linear form in the box's width and height, not a pair
    // of pixel offsets added to the end of the composed matrix (#212).
    // `TransformValue` carries the four coefficients; this is where the box
    // they multiply finally arrives.
    let (w, h) = (size.0 as f64, size.1 as f64);
    m[4] += tf.pct_translate_w[0] * w + tf.pct_translate_h[0] * h;
    m[5] += tf.pct_translate_w[1] * w + tf.pct_translate_h[1] * h;
    // The translate components are *lengths*: `m[4]`/`m[5]` come from the
    // stylesheet in CSS px and the percentage part resolves against the CSS-px
    // layout box, so both need converting to the physical-pixel space this
    // function composes in (its `x`/`y` inputs and the origin below already
    // are). The linear part (a, b, c, d) is a pure ratio — unit-invariant —
    // and must NOT be scaled (#202).
    m[4] *= scale;
    m[5] *= scale;
    let cx = x + origin.0 as f64 * scale;
    let cy = y + origin.1 as f64 * scale;
    parent_transform * Affine::translate((cx, cy)) * Affine::new(m) * Affine::translate((-cx, -cy))
}

/// [`compose_node_transform`], plus the one case paint never asks it about.
///
/// A `display: contents` node generates no box, so `paint_node`'s zero-size
/// branch hands `parent_transform` straight down to the children rather than
/// composing anything for it — a transform declared on such a node has no box
/// to apply about and is simply not drawn. Hit testing makes the same exception
/// (`local_point`, `crates/rinch/src/app/hit_testing.rs`); a forward walk that
/// composed it anyway would place the node's descendants somewhere paint never
/// puts them.
fn compose_transform_step(
    node: &Node,
    x: f64,
    y: f64,
    scale: f64,
    parent_transform: Affine,
    frame: Frame,
) -> Affine {
    let layout = frame.layout(node);
    if node.computed_style.display == DisplayValue::Contents
        && (layout.width == 0.0 || layout.height == 0.0)
    {
        return parent_transform;
    }
    match frame.painted(node) {
        Some(painted) => match &painted.transform {
            Some(t) => compose_transform_parts(
                &t.value,
                t.origin,
                (layout.width, layout.height),
                t.backface_hidden,
                x,
                y,
                scale,
                parent_transform,
            ),
            None => parent_transform,
        },
        None => compose_node_transform(node, x, y, scale, parent_transform),
    }
}

/// Which state of the tree a position is asked about.
///
/// [`Frame::Current`] is the tree as laid out now — what the next paint draws.
/// [`Frame::Painted`] is the tree as the **last paint drew it**: each node's
/// `prev_layout` and [`crate::node::PaintedState`] where it has one, and its
/// current values where it has not been painted (a node created since, or an
/// anonymous box the last IFC pass minted, whose descendants were painted under
/// a box that no longer exists — the current one is the best answer there).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Frame {
    Current,
    Painted,
}

impl Frame {
    fn painted(self, node: &Node) -> Option<&crate::node::PaintedState> {
        match self {
            Frame::Current => None,
            Frame::Painted => node.painted.as_ref(),
        }
    }

    fn layout(self, node: &Node) -> crate::node::LayoutResult {
        if self.painted(node).is_some() {
            node.prev_layout
        } else {
            node.layout
        }
    }

    fn is_identity(self, node: &Node) -> bool {
        match self.painted(node) {
            Some(p) => p.transform.is_none(),
            None => node.computed_style.transform.is_identity,
        }
    }
}

/// The transform every hoisted `position: fixed` box paints under.
///
/// `paint_document` enters at the body with zero offsets and the identity
/// transform, so the body's own composed transform is viewport space — which is
/// what `hit_test_node` re-seeds `vx`/`vy` from.
///
/// A fixed box is hoisted only to its nearest ancestor stacking context (#545),
/// which need not be the body, so `paint_children_with_stacking` asks for this
/// explicitly for a fixed entry rather than handing on the collecting root's
/// transform. Same value as before for the common case, and the reason it is a
/// function rather than an argument: the entry has to end up in the same space
/// [`compute_absolute_position_and_transform`] reports it in, whichever root
/// happens to own it.
fn body_paint_transform(tree: &NodeTree, scale: f64) -> Affine {
    body_transform_in(tree, scale, Frame::Current)
}

fn body_transform_in(tree: &NodeTree, scale: f64, frame: Frame) -> Affine {
    let Some(body) = tree.get(tree.body_id) else {
        return Affine::IDENTITY;
    };
    // Through the same origin step the chain below uses, so the body's
    // transform is composed about the same point either way in.
    let (bx, by) = painted_origin_step(tree, body, 0.0, 0.0, scale, frame);
    compose_transform_step(body, bx, by, scale, Affine::IDENTITY, frame)
}

/// One step of the descent: the node's own painted origin, given the origin its
/// parent resolves children against.
///
/// Kept as one function so the transformed and untransformed paths below cannot
/// drift: both add the layout offset and the IFC content-box offset, and both
/// hand children a scroll-adjusted origin.
///
/// `hit_test_node` additionally exempts a `position: fixed` box from the IFC
/// offset. That exemption is not repeated here because it cannot fire: Stylo
/// blockifies an out-of-flow box, so a fixed one is never an atomic inline
/// ([`crate::DisplayMode::is_atomic_inline`]) and [`ifc_content_box_offset`] already answers
/// `(0, 0)` for it. `a_fixed_inline_block_keeps_its_viewport_origin` pins the
/// outcome, so if that ever stops being true the exemption gets added here
/// rather than discovered in the wild.
fn painted_origin_step(
    tree: &NodeTree,
    node: &Node,
    off_x: f64,
    off_y: f64,
    scale: f64,
    frame: Frame,
) -> (f64, f64) {
    let (dx, dy) = ifc_content_box_offset(tree, node);
    let layout = frame.layout(node);
    (
        off_x + (layout.x + dx) as f64 * scale,
        off_y + (layout.y + dy) as f64 * scale,
    )
}

/// Compute a node's absolute position in physical pixels together with the
/// composed CSS transform affecting it (its own and its ancestors'), mirroring
/// paint's own descent so that anything asking "where does this node end up"
/// gets the answer paint acts on (#143, #203).
///
/// The mirror is the whole contract, so each of paint's four adjustments is
/// made here too:
///
/// - the chain stops at a `position: fixed` node, whose box is viewport-relative
///   because `stacking::Collector` gives its entry zeroed offsets — **and
///   resumes from the body's own transform**, which is what paint hands a fixed
///   entry whichever stacking context it was hoisted to (#545);
/// - a `display: contents` node contributes no transform (see
///   [`compose_transform_step`]);
/// - an inline-block an IFC positions gets [`ifc_content_box_offset`] added,
///   because its `layout.x`/`layout.y` are relative to the IFC root's content
///   box while the chain sums border-box origins;
/// - children resolve against a scroll-adjusted origin.
///
/// The chain also stops at the body, which is where `paint_document` enters:
/// nothing above it is painted, so nothing above it may displace or transform
/// what is.
///
/// `scale` is the DPI scale factor the result is expressed in.
/// [`compose_node_transform`] is covariant in it (#202), so a caller working in
/// layout pixels — hit testing, click bounds — passes `1.0` and gets paint's
/// transform expressed in layout pixels.
///
/// The common untransformed case takes an allocation-free fast path and returns
/// `Affine::IDENTITY`.
pub fn compute_absolute_position_and_transform(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
) -> (f64, f64, Affine) {
    position_and_transform_in(tree, node_id, scale, Frame::Current)
}

/// [`compute_absolute_position_and_transform`] in either [`Frame`].
fn position_and_transform_in(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    frame: Frame,
) -> (f64, f64, Affine) {
    // First pass: does anything on the chain transform (cheap pointer walk)?
    let mut any_transform = false;
    let mut hoisted_fixed = false;
    let mut current = Some(node_id);
    while let Some(id) = current {
        let Some(node) = tree.get(id) else { break };
        any_transform |= !frame.is_identity(node);
        if node.computed_style.position == PositionValue::Fixed {
            hoisted_fixed = true;
            break;
        }
        if id == tree.body_id {
            break;
        }
        current = crate::RinchDocument::box_tree_parent(&tree.nodes, id);
    }
    // A hoisted fixed box drops its ancestors' transforms but not the body's.
    if hoisted_fixed && let Some(body) = tree.get(tree.body_id) {
        any_transform |= !frame.is_identity(body);
    }

    if !any_transform {
        // Same sum, bottom-up: the offsets do not depend on the direction of
        // travel, only the transform composition does.
        let (mut x, mut y) = (0.0_f64, 0.0_f64);
        let mut current = Some(node_id);
        while let Some(id) = current {
            let Some(node) = tree.get(id) else { break };
            let (nx, ny) = painted_origin_step(tree, node, x, y, scale, frame);
            x = nx;
            y = ny;
            if node.computed_style.position == PositionValue::Fixed || id == tree.body_id {
                break;
            }
            // The **box** tree (#566): a run's member is positioned by the
            // anonymous box that lays it out, so the box's own offset is part
            // of this sum. Stepping to `parent` here lands every run after the
            // first at its container's origin.
            let up = crate::RinchDocument::box_tree_parent(&tree.nodes, id);
            if let Some(parent) = up.and_then(|pid| tree.get(pid)) {
                x -= parent.scroll_offset.0 * scale;
                y -= parent.scroll_offset.1 * scale;
            }
            current = up;
        }
        return (x, y, Affine::IDENTITY);
    }

    // Transformed chain: collect node → root, then walk root → node
    // accumulating offsets and composing affines top-down — the same order
    // paint_node applies them.
    let mut chain: Vec<RawNodeId> = Vec::new();
    let mut current = Some(node_id);
    while let Some(id) = current {
        let Some(node) = tree.get(id) else { break };
        chain.push(id);
        if node.computed_style.position == PositionValue::Fixed || id == tree.body_id {
            break;
        }
        current = crate::RinchDocument::box_tree_parent(&tree.nodes, id);
    }

    let mut off_x = 0.0_f64;
    let mut off_y = 0.0_f64;
    // Seed with the body's transform for a hoisted fixed box — unless the body
    // *is* that box, in which case the loop below composes it and seeding here
    // would apply it twice.
    let mut transform = if hoisted_fixed && chain.last() != Some(&tree.body_id) {
        body_transform_in(tree, scale, frame)
    } else {
        Affine::IDENTITY
    };
    let (mut x, mut y) = (0.0_f64, 0.0_f64);
    for &id in chain.iter().rev() {
        let Some(node) = tree.get(id) else { break };
        let (nx, ny) = painted_origin_step(tree, node, off_x, off_y, scale, frame);
        x = nx;
        y = ny;
        transform = compose_transform_step(node, x, y, scale, transform, frame);
        // Children resolve against this node's scroll-adjusted origin.
        off_x = x - node.scroll_offset.0 * scale;
        off_y = y - node.scroll_offset.1 * scale;
    }
    (x, y, transform)
}

/// The axis-aligned box a node is *painted* in — the desktop answer to
/// `getBoundingClientRect()`.
///
/// This is [`compute_absolute_position_and_transform`] with the node's own size
/// pushed through the composed transform. A rotated box has no axis-aligned
/// answer, so this is its bounding box; for the translate and scale that every
/// real transformed layout uses it is exact, and it is the same approximation
/// the browser makes.
///
/// `scale` is the DPI scale factor the result is expressed in — pass `1.0` to
/// work in layout pixels.
pub fn painted_border_box(tree: &NodeTree, node_id: RawNodeId, scale: f64) -> Rect {
    let (x, y, transform) = compute_absolute_position_and_transform(tree, node_id, scale);
    let (w, h) = tree.get(node_id).map_or((0.0, 0.0), |node| {
        (
            node.layout.width as f64 * scale,
            node.layout.height as f64 * scale,
        )
    });
    transform.transform_rect_bbox(Rect::new(x, y, x + w, y + h))
}

/// Map a viewport point **into** a node's own painted space, expressed relative
/// to the node's border-box origin.
///
/// The inverse direction of [`painted_border_box`], and the one every
/// screen-point-to-content question needs: which character was clicked, where
/// inside a render surface the pointer is, how far along a scrollbar track a
/// press landed. Subtracting a painted AABB's origin is **not** a substitute —
/// under `transform: scale(2)` a point 40px right of the box's painted left
/// edge is 20px into the box's own space, not 40.
///
/// Returns `None` when the composed transform is not invertible (`scale(0)`, a
/// degenerate matrix): such a subtree paints to zero area, so no screen point
/// corresponds to anything inside it. `hit_test`'s `local_point` takes the same
/// exit for the same reason.
///
/// `scale` is the DPI scale factor `px`/`py` are expressed in — pass `1.0` to
/// work in layout pixels.
pub fn point_in_painted_box(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    px: f64,
    py: f64,
) -> Option<(f64, f64)> {
    let (x, y, transform) = compute_absolute_position_and_transform(tree, node_id, scale);
    if transform == Affine::IDENTITY {
        return Some((px - x, py - y));
    }
    let det = transform.determinant();
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let p = transform.inverse() * Point::new(px, py);
    if !p.x.is_finite() || !p.y.is_finite() {
        return None;
    }
    Some((p.x - x, p.y - y))
}

/// Map a point given relative to a node's border-box origin, in the node's own
/// painted space, **out** to viewport coordinates.
///
/// The forward direction of [`point_in_painted_box`], for the callers that
/// produce a position inside a box and need to place something at it in window
/// space — the caret, the IME candidate rectangle, a probe point for a
/// subsequent hit test.
///
/// `scale` is the DPI scale factor `lx`/`ly` are expressed in and the result is
/// returned in — pass `1.0` to work in layout pixels.
pub fn point_from_painted_box(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    lx: f64,
    ly: f64,
) -> (f64, f64) {
    let (x, y, transform) = compute_absolute_position_and_transform(tree, node_id, scale);
    let p = transform * Point::new(x + lx, y + ly);
    (p.x, p.y)
}

/// Find viewport descendant rects (absolute pixel positions) for hole-punching.
/// Returns a list of viewport rects in physical pixel coordinates.
fn find_viewport_rects(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    offset_x: f64,
    offset_y: f64,
    result: &mut Vec<Rect>,
) {
    let Some(node) = tree.get(node_id) else {
        return;
    };
    let nx = offset_x + node.layout.x as f64 * scale;
    let ny = offset_y + node.layout.y as f64 * scale;

    if let Some(viewport_name) = node.attributes.get("data-viewport") {
        if viewport_punches(node, viewport_name) {
            let vw = node.layout.width as f64 * scale;
            let vh = node.layout.height as f64 * scale;
            result.push(Rect::new(nx, ny, nx + vw, ny + vh));
        }
        return;
    }

    // Account for scroll offset when recursing into children
    let sx = node.scroll_offset.0 * scale;
    let sy = node.scroll_offset.1 * scale;
    // The box tree, not the element tree (#566) — see `box_tree_children`. A
    // viewport hole inside an inline run would otherwise be unreachable.
    for &child_id in crate::RinchDocument::box_tree_children(&tree.nodes, node_id).iter() {
        find_viewport_rects(tree, child_id, scale, nx - sx, ny - sy, result);
    }
}

/// Whether the `data-viewport` node `node`, named `viewport_name`, cuts a hole
/// through its ancestors' backgrounds this paint.
fn viewport_punches(node: &Node, viewport_name: &str) -> bool {
    // If ACTIVE_VIEWPORTS is set, only cut holes for viewports with active frames
    let active = ACTIVE_VIEWPORTS.with(|v| {
        let guard = v.borrow();
        match guard.as_ref() {
            None => true, // No filter set — all viewports get holes (GPU default)
            Some(set) => set.contains(viewport_name),
        }
    });
    // A hole is only worth cutting if something will fill it. A viewport
    // that declares itself not ready — rinch-video before its first decoded
    // frame, or after a `PlaybackState::Error` — keeps its ancestors'
    // backgrounds intact; otherwise a video that never loads is see-through
    // to the desktop on a transparent window (issue #186).
    //
    // The attribute is an opt-OUT: a node that does not carry it punches
    // unconditionally, which is what `GameViewport` wants — the game owns
    // its hole from the first frame and stamps nothing. A node that does
    // carry it must say `"true"` to punch, so a mis-stamped value fails to
    // the safe side (an opaque placeholder, never a see-through window).
    let ready = node
        .attributes
        .get("data-viewport-ready")
        .is_none_or(|v| v == "true");
    active && ready
}

/// Build a BezPath for the background shape with viewport holes cut out.
/// Uses EvenOdd fill rule: outer shape wound clockwise, holes wound counter-clockwise.
fn build_background_with_holes(
    rect: Rect,
    radii: RoundedRectRadii,
    radius: f64,
    holes: &[Rect],
) -> BezPath {
    let mut path = BezPath::new();

    // Outer contour: the background shape (clockwise)
    if radius > 0.0 {
        let rrect = rect.to_rounded_rect(radii);
        path.extend(rrect.path_elements(0.1));
    } else {
        path.extend(rect.path_elements(0.1));
    }

    // Inner contours: viewport holes (counter-clockwise = reversed winding)
    for hole in holes {
        // Rect path_elements goes clockwise, so we reverse for counter-clockwise
        path.move_to((hole.x0, hole.y0));
        path.line_to((hole.x0, hole.y1));
        path.line_to((hole.x1, hole.y1));
        path.line_to((hole.x1, hole.y0));
        path.close_path();
    }

    path
}

/// Paint a subtree rooted at `root_node_id` into `painter`, positioned at (0, 0).
///
/// Used for drag-and-drop snapshot capture. The subtree is translated so its
/// top-left corner is at the scene origin, making it easy to reposition later.
pub fn paint_subtree(
    tree: &NodeTree,
    painter: &mut dyn Painter,
    root_node_id: RawNodeId,
    scale: f64,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<Brush>,
) {
    // paint_node computes each node's position as: offset + layout.x * scale.
    // To place the subtree root at (0, 0), we negate just the root's own
    // layout position — NOT the full absolute position (which would over-shift
    // by the entire ancestor chain).
    let Some(node) = tree.get(root_node_id) else {
        return;
    };
    let offset_x = -(node.layout.x as f64 * scale);
    let offset_y = -(node.layout.y as f64 * scale);
    ClipTrackingPainter::run(painter, scale, &tree.perf, |painter| {
        paint_node(
            tree,
            root_node_id,
            painter,
            scale,
            offset_x,
            offset_y,
            font_cx,
            layout_cx,
            Affine::IDENTITY,
        )
    });
}

/// Paint the entire document using a Painter.
///
/// `scale` is the DPI scale factor (1.0 = 96dpi).
///
/// `viewport` is the viewport size in **logical (CSS) pixels** — the layout
/// viewport, which is what every caller already has to hand. The parameter was
/// documented as *physical* and then ignored, so the two never disagreed about
/// anything; card K43 gave it a job (it is now the rect outside which nothing
/// is drawn) and the unit had to be settled. Logical, because that is what
/// `logical_size()` returns at every desktop, DevTools and Android call site,
/// and because `embed` — the one caller that passed physical — is a one-line
/// change rather than a fleet of them.
///
/// Passing a zero in either axis means "no target", and disables the cull and
/// the covers-the-window clip elision rather than culling the whole document.
pub fn paint_document(
    tree: &NodeTree,
    painter: &mut dyn Painter,
    scale: f64,
    viewport: (f32, f32),
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<Brush>,
) {
    // The window, in the physical pixels paint works in, plus the margin the
    // cull tests against. Nothing outside the second is emitted; see
    // [`intersects_dirty_region`] and [`INK_MARGIN_CSS_PX`].
    //
    // A zero in either axis means "no target given" and disables both the cull
    // and the covers-the-window elision, rather than culling everything.
    VIEWPORT.with(|v| {
        v.set(if viewport.0 > 0.0 && viewport.1 > 0.0 {
            let target = Rect::new(
                0.0,
                0.0,
                viewport.0 as f64 * scale,
                viewport.1 as f64 * scale,
            );
            Some(Viewport {
                target,
                cull: target.inflate(INK_MARGIN_CSS_PX * scale, INK_MARGIN_CSS_PX * scale),
            })
        } else {
            None
        })
    });
    ClipTrackingPainter::run(painter, scale, &tree.perf, |painter| {
        paint_node(
            tree,
            tree.body_id,
            painter,
            scale,
            0.0,
            0.0,
            font_cx,
            layout_cx,
            Affine::IDENTITY,
        )
    });
    // Cleared, not left behind. `paint_subtree` renders into a pixmap sized to
    // one element and never sets this, so a stale window from the last frame
    // would have it culling against a rect that has nothing to do with what it
    // is drawing into.
    VIEWPORT.with(|v| v.set(None));
}

/// Paint the children of `node_id`, front to back.
///
/// A stacking-context root (and the body, which is one by fiat) paints the
/// sequence [`stacking_paint_order`] gives it — CSS 2.1 Appendix E order, with
/// descendant stacking contexts and positioned `z-index: auto` boxes hoisted out
/// of their parents and sorted in among each other.
///
/// Any other node paints only the children that were *not* hoisted, in tree
/// order; an ancestor stacking context has the rest. [`paints_at_stacking_root`]
/// is the one predicate that decides which is which, and hit testing asks it the
/// same question so that what is on top is also what is tapped.
///
/// An entry hoisted past a clipping ancestor carries that ancestor's clip in
/// its [`clip chain`](crate::stacking::PaintOrder::clips_for) — pushed here,
/// around the entry, because `overflow` no longer creates a stacking context
/// and the bracket `paint_node` would have opened for it is not on the
/// painter's stack any more (#324). The clips are in this root's own space,
/// like the offsets, so they all go under `node_transform` with no per-clip
/// composition.
///
/// Consecutive entries that share a chain share one push. That is not a
/// heuristic: identical [`ClipSpan`](crate::stacking::ClipSpan)s name the same
/// clips, so leaving them on the stack across the run is exactly what pushing
/// and popping each time would do — and the run is the common shape, since the
/// rows of one scroller are contiguous in tree order and usually all at `z: 0`.
/// It matters because every push costs `TinySkiaPainter` a mask fill and an
/// intersection with the enclosing mask over the clip's bounds. (It used to
/// cost a freshly allocated full-surface `Mask` as well; the painter pools its
/// masks now, but a push is still not free.)
#[allow(clippy::too_many_arguments)]
fn paint_children_with_stacking(
    tree: &NodeTree,
    node_id: RawNodeId,
    painter: &mut dyn Painter,
    scale: f64,
    offset_x: f64,
    offset_y: f64,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<Brush>,
    node_transform: Affine,
    root_clip: Option<&PaintShape>,
    // This node clips, and `paint_node` decided the bracket would remove
    // nothing and did not open one (card K43). A truthful `None` for a reason
    // the `clips_overflow()` proxy below cannot see.
    clip_elided: bool,
) {
    let Some(node) = tree.get(node_id) else {
        return;
    };

    // `root_clip` is a *declaration*, not a hint, and this is what makes a wrong
    // one loud. Getting it wrong is silent otherwise: a `None` passed while a
    // bracket is open reads exactly like a `None` passed with none open, the
    // fixed-entry lift below simply does not happen, and the only symptom is a
    // `position: fixed` box that stops being drawn. #540 arrived at that state
    // by *clean* three-way merge — it was written before #545 added this
    // parameter, `git` auto-merged its new bracket beside the `None` that used
    // to be true, and nothing in a 620-test suite noticed.
    //
    // A node that clips has a bracket on the painter's stack: both arms of
    // `paint_node` that push one derive it from the same `clip_shape` call this
    // predicate asks about. There are two exceptions, and neither is a
    // counter-example:
    //
    // - `display: contents` generates no box, so there is nothing to clip and
    //   no bracket is pushed for it however its `overflow` computes;
    // - a clip card K43 proved removes no drawn pixel is not pushed at all, and
    //   `clip_elided` is how the caller says so. Without it this assertion fires
    //   on **every document**, because rinch's own body clips (the UA sheet's
    //   `overflow-y: auto`) and its box is the window, which is the purest case
    //   of the covers-the-target elision. That is what makes `clips_overflow()`
    //   a *proxy* for "a bracket is open" rather than the fact itself: it was
    //   exact until layers could be elided, and a proxy that has gone stale is
    //   indistinguishable from the bug this assertion exists to catch.
    //
    // Both exceptions are truthful `None`s: no bracket is open, so a hoisted
    // `position: fixed` entry has nothing to be lifted out of, and the lift
    // correctly does not happen.
    debug_assert!(
        root_clip.is_some()
            || clip_elided
            || !node.clips_overflow()
            || node.computed_style.display == DisplayValue::Contents,
        "`root_clip` must name the bracket `paint_node` opened around this \
         sequence: this node clips, so a bracket is on the painter's stack, and \
         a `position: fixed` entry hoisted to this root would be swallowed by it \
         instead of lifted out of it (#545)"
    );

    // Content already drawn as inline boxes, via `paint_inline_layout`:
    // painting it again as a box would double it. See `drawn_by_its_ifc`.
    let already_drawn_inline = |child: &Node, _kind: PaintKind| drawn_by_its_ifc(tree, child);

    if node_id == tree.body_id || node.creates_stacking_context() {
        let order = stacking_paint_order(tree, node_id, scale, offset_x, offset_y);
        let mut open = ClipSpan::EMPTY;
        for entry in order.iter() {
            let child = tree.get(entry.node_id);
            if let Some(child) = child
                && already_drawn_inline(child, entry.kind)
            {
                continue;
            }
            if entry.clips != open {
                for _ in 0..open.len() {
                    painter.pop_layer();
                }
                open = entry.clips;
                for clip in order.clips_for(entry) {
                    let shape: PaintShape = if clip.radii.top_left > 0.0
                        || clip.radii.top_right > 0.0
                        || clip.radii.bottom_right > 0.0
                        || clip.radii.bottom_left > 0.0
                    {
                        clip.rect.to_rounded_rect(clip.radii).into()
                    } else {
                        clip.rect.into()
                    };
                    painter.push_clip(Fill::NonZero, node_transform, &shape);
                }
            }

            // A `position: fixed` entry is hoisted no further than this root
            // (#545), but this root is not its containing block: neither the
            // bracket `paint_node` opened around this sequence nor this root's
            // transform applies to it. Lift the bracket for the length of the
            // entry and put back the very same shape; and paint it under the
            // body's transform, which is the space its `layout` coordinates and
            // `compute_absolute_position_and_transform` both already use.
            //
            // `open` is necessarily `EMPTY` here — a fixed entry's own chain is
            // empty (`Collector::span`), so the branch above has just popped
            // whatever the previous entry left open — which is what makes the
            // root's bracket the top of the clip stack and safe to pop.
            let escapes_root =
                child.is_some_and(|c| c.computed_style.position == PositionValue::Fixed);
            let (entry_transform, lifted) = if escapes_root {
                debug_assert!(
                    open.is_empty(),
                    "a fixed entry's clip chain is empty, so nothing may be open over the root's bracket"
                );
                if let Some(shape) = root_clip {
                    painter.pop_layer();
                    (body_paint_transform(tree, scale), Some(shape))
                } else {
                    (body_paint_transform(tree, scale), None)
                }
            } else {
                (node_transform, None)
            };

            // Asked here, after the entry's chain is pushed, so the cull it
            // reads is the one the entry would paint under (#910). A lifted
            // fixed entry is never asked — the helper answers `false` for one
            // — and a stacking context neither.
            if lifted.is_none()
                && paints_nothing_without_visit(
                    tree,
                    entry.node_id,
                    scale,
                    entry.offset_x,
                    entry.offset_y,
                    entry_transform,
                    0,
                )
            {
                continue;
            }

            paint_node(
                tree,
                entry.node_id,
                painter,
                scale,
                entry.offset_x,
                entry.offset_y,
                font_cx,
                layout_cx,
                entry_transform,
            );

            if let Some(shape) = lifted {
                painter.push_clip(Fill::NonZero, node_transform, shape);
            }
        }
        for _ in 0..open.len() {
            painter.pop_layer();
        }
    } else {
        // The **box** tree, not the element tree (#566): an inline run is drawn
        // by the anonymous block box that lays it out, and that box is not in
        // `children`. Walking `children` here paints nothing for the run at all
        // — the line is drawn from `text_layout` on the box itself.
        for &child_id in crate::RinchDocument::box_tree_children(&tree.nodes, node_id).iter() {
            let Some(child) = tree.get(child_id) else {
                continue;
            };
            if paints_at_stacking_root(child) || already_drawn_inline(child, PaintKind::InFlow) {
                continue;
            }
            if paints_nothing_without_visit(
                tree,
                child_id,
                scale,
                offset_x,
                offset_y,
                node_transform,
                0,
            ) {
                continue;
            }
            paint_node(
                tree,
                child_id,
                painter,
                scale,
                offset_x,
                offset_y,
                font_cx,
                layout_cx,
                node_transform,
            );
        }
    }
}

/// Whether the ordinary (non-stacking-context) box `node_id`, entered at
/// `offset` under `parent_transform`, would paint nothing at all — decided
/// without a [`paint_node`] visit (#910).
///
/// This is [`paint_node`]'s own cull, asked one level up: a box outside every
/// cull ([`intersects_dirty_region`]) draws nothing of its own, and a box that
/// clips then returns, while one that does not recurses into its box-tree
/// children, skipping those its IFC draws and those painted at a stacking root.
/// So the answer is `true` exactly when paint would have walked the subtree and
/// drawn nothing: the box clips, or every child it would descend into is itself
/// such a subtree. What it asks nothing about, it answers `false` for, and
/// [`paint_node`] decides as it always has — a stacking context (hoisted, and
/// `subtree_is_entirely_outside` is the walk that knows what escapes one),
/// `position: fixed` or `sticky`, `display: contents`, a box degenerate in
/// either axis.
///
/// The point is the count, not the test: a 500-row scroller showing 20 rows
/// used to enter `paint_node` for every row, each to be culled on arrival. The
/// loop still meets each row; it no longer composes a transform, walks a
/// sticky chain or reads a dozen style fields to dismiss it.
#[allow(clippy::too_many_arguments)]
fn paints_nothing_without_visit(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    offset_x: f64,
    offset_y: f64,
    parent_transform: Affine,
    depth: usize,
) -> bool {
    // Shallow on purpose; past it, visit and let paint decide. Every level
    // that *is* visited asks this again of its own children, so a subtree
    // outside the cull whose content reaches back in late in DOM order is
    // re-walked once per level of nesting above it. The cap bounds that to
    // `MAX_DEPTH + 1` rect tests per node, where 32 let it grow with the
    // nesting: 2000 `column-reverse` rows under 30 overflowing wrappers
    // painted in 0.30 ms against 0.13 ms with no cull, and 0.15 ms with this
    // cap (review of #957, release, best of 5). A list row — the row, its
    // content, a line or two inside that — still prunes whole within it.
    const MAX_DEPTH: usize = 3;
    if depth > MAX_DEPTH {
        return false;
    }
    let Some(node) = tree.get(node_id) else {
        return false;
    };
    let cs = &node.computed_style;
    if cs.display == DisplayValue::None {
        return true;
    }
    if node.creates_stacking_context()
        || matches!(cs.position, PositionValue::Fixed | PositionValue::Sticky)
        || cs.display == DisplayValue::Contents
        || node.estimated_height.is_some()
    {
        return false;
    }
    let layout = &node.layout;
    if layout.width == 0.0 || layout.height == 0.0 {
        return false;
    }
    let x = offset_x + layout.x as f64 * scale;
    let y = offset_y + layout.y as f64 * scale;
    let w = layout.width as f64 * scale;
    let h = layout.height as f64 * scale;
    let node_transform = compose_node_transform(node, x, y, scale, parent_transform);
    let ink = ink_rect(cs, x, y, w, h, scale);
    let inside = if node_transform == Affine::IDENTITY {
        intersects_dirty_region(ink.x0, ink.y0, ink.width(), ink.height())
    } else {
        let bbox = node_transform.transform_rect_bbox(ink);
        intersects_dirty_region(bbox.x0, bbox.y0, bbox.width(), bbox.height())
    };
    if inside {
        return false;
    }
    if node.clips_overflow() {
        return true;
    }
    let child_x = x - node.scroll_offset.0 * scale;
    let child_y = y - node.scroll_offset.1 * scale;
    crate::RinchDocument::box_tree_children(&tree.nodes, node_id)
        .iter()
        .all(|&child_id| {
            let Some(child) = tree.get(child_id) else {
                return true;
            };
            paints_at_stacking_root(child)
                || drawn_by_its_ifc(tree, child)
                || paints_nothing_without_visit(
                    tree,
                    child_id,
                    scale,
                    child_x,
                    child_y,
                    node_transform,
                    depth + 1,
                )
        })
}

#[allow(clippy::too_many_arguments)]
fn paint_node(
    tree: &NodeTree,
    node_id: RawNodeId,
    painter: &mut dyn Painter,
    scale: f64,
    offset_x: f64,
    offset_y: f64,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<Brush>,
    parent_transform: Affine,
) {
    tree.perf.bump(crate::perf::Counter::PaintNodesVisited);
    let Some(node) = tree.get(node_id) else {
        return;
    };

    // Skip elements that generate no box at all.
    //
    // **`display: none` is tested here because nothing else was testing it**
    // (#543). The comment that used to sit on this block claimed it did while
    // the code below checked only the tag, and the sole `DisplayValue::None`
    // test anywhere under `paint/` was in `layer_bounds.rs`. It was harmless for
    // as long as a hidden box was laid out `0x0` and so drew nothing — which is
    // what Taffy gives every hidden node that is *in* the Taffy tree. It stopped
    // being harmless once one was taken out of it: `mark_inline_descendants`
    // detaches a hidden child of an IFC root, Taffy keeps serving a detached
    // node its last computed layout, and `read_layout_results` wrote that stale
    // box back onto the node — which this function then drew. A closed menu that
    // stayed on screen indefinitely.
    //
    // **That is repaired where the box is written** (`read_layout_results`
    // zeroes a hidden subtree), and that repair, not this one, is what fixes the
    // reported bug — measured: with the zeroing in place and this test removed,
    // every producer-level fixture still passes. This test is for the *class*,
    // and the two are independent: zeroing keeps boxes honest for every reader,
    // hit testing included, which paint cannot help with; the test here makes
    // paint correct for any node that computes `none` however its box arose.
    // Its only witness is a checker test that corrupts `layout` by hand
    // (`display_none_ghost_tests::painted::paint_refuses_a_hidden_node_that_still_carries_a_box`),
    // which says so in its own doc — do not read it as a producer pin.
    //
    // `Contents` must NOT be skipped: a `display: contents` element generates no
    // box of its own but its children must still be drawn, and returning here
    // would take the whole subtree with it.
    if node.computed_style.display == crate::computed_style::values::DisplayValue::None {
        return;
    }
    if let NodeKind::Element(ref el) = node.kind
        && matches!(
            el.tag.as_str(),
            "style" | "script" | "head" | "meta" | "link"
        )
    {
        return;
    }

    // Skip collapsed blocks (virtualized contenteditable) — they have
    // estimated heights for layout but no Parley text layout for painting.
    if node.estimated_height.is_some() {
        return;
    }

    // Nothing inside `opacity: 0` can be seen, so nothing inside it is drawn.
    //
    // This is not an approximation. `opacity < 1` makes a node a stacking
    // context (`Node::creates_stacking_context`), so its whole subtree is
    // composited through the one group layer this node opens, and that layer is
    // composited back with `SourceOver` at alpha 0 — which writes no pixel,
    // whatever was painted into it. Every draw between the `push_layer` and the
    // `pop_layer` is therefore work with no output.
    //
    // Skipping the group rather than painting it is worth a great deal more
    // than it looks, because of what the group *cost* in the software painter
    // when this was written: `TinySkiaPainter::push_layer` allocated a second
    // pixmap the size of the whole surface and `pop_layer` composited all of it
    // back. (The painter now pools its layer pixmaps and composites only the
    // part anything was drawn into, which removes most of that cost, but every
    // draw inside the group is still work with no output.) The idiom this
    // was found in is the always-mounted bottom sheet — a full-screen scrim
    // that fades in, parked at `opacity: 0` until a chip is tapped. On the
    // moto g stylus 5G, at 1080×2460, one such scrim cost about 48ms a frame
    // while being invisible: a 10MB pixmap allocation, an 18ms full-screen fill
    // into it, and a 28ms full-screen composite back out. The app carries three
    // of those sheets on its library screen, so roughly 145ms of every 290ms
    // frame was spent drawing three things that were not there. See card K24.
    //
    // The cut is on paint only. Hit-testing and layout do not come through
    // here, and CSS keeps an `opacity: 0` element in the box tree and reachable
    // by the pointer — which is exactly what a sheet parked at zero relies on.
    //
    // `display: contents` is the one exception, and it is the same exception
    // the zero-size branch below already makes: it generates no box, so there
    // is no group for opacity to composite and no layer is pushed for it — its
    // children paint into the grandparent at full strength, before this change
    // and after it.
    if node.computed_style.opacity <= 0.0 && node.computed_style.display != DisplayValue::Contents {
        return;
    }
    let layout = &node.layout;

    // Skip zero-size elements (display: none produces 0x0 layout).
    // display:contents nodes also have 0x0 layout (no box), but their children
    // still need to be painted — recurse into children then return.
    // Also skip elements with either dimension zero — they have no fillable area
    // for backgrounds/borders, and attempting to paint them produces degenerate
    // zero-area rects that trigger warnings in renderers like tiny-skia.
    if layout.width == 0.0 || layout.height == 0.0 {
        if node.computed_style.display == DisplayValue::Contents {
            // display:contents has no box, so it never forms a SC itself and
            // has no transform box — its children are laid out in the
            // grandparent's coordinate space, so recurse with the parent's
            // offsets and transform unchanged.
            //
            // Use full stacking-context-aware painting so that SC children
            // (e.g. z-indexed absolute children) are properly collected and
            // painted in z-index order, rather than being skipped.
            paint_children_with_stacking(
                tree,
                node_id,
                painter,
                scale,
                offset_x,
                offset_y,
                font_cx,
                layout_cx,
                parent_transform,
                // No bracket is open: this branch returns before the node's own
                // clip is pushed.
                None,
                false,
            );
        } else if (layout.width == 0.0) != (layout.height == 0.0) {
            // A real box collapsed to zero in one dimension (e.g. an
            // auto-height container with only absolutely-positioned children)
            // may still have overflowing children that need painting. Unlike
            // display:contents it keeps its own origin, transform, and
            // opacity — only the degenerate background/border fill is
            // skipped (#142).
            let x = offset_x + layout.x as f64 * scale;
            let y = offset_y + layout.y as f64 * scale;
            let node_transform = compose_node_transform(node, x, y, scale, parent_transform);

            let opacity = node.computed_style.opacity;
            let has_opacity = opacity < 1.0;
            if has_opacity {
                // The element's own rect is zero-area and Vello clips a layer
                // to its bounds shape, which would blank the subtree. The
                // subtree's own extent is the right answer here as it is
                // everywhere else, and [`opacity_layer_bounds`] falls back to
                // the conservative `UNBOUNDED` rect this branch used to pass
                // whenever it cannot work one out (card K36).
                let bounds = opacity_layer_shape(tree, node_id, scale, x, y);
                painter.push_layer(BlendMode::Normal, opacity, node_transform, &bounds);
            }

            // Card K51. #142's fix (above) answers "does this node still need
            // to paint its children" and never asks what its `overflow` is —
            // which was fine for the case it was written for, an *unclipped*
            // auto-height wrapper whose only content is absolutely positioned
            // and already escaping it. The same zero-in-one-dimension shape
            // also describes a card J1 group closed to `height: 0` on purpose,
            // whose wrapper carries `overflow: hidden` precisely so its real,
            // in-flow rows stop being seen — and every ordinary place this file
            // turns `overflow: hidden` into a clip lives below the early
            // `return` this branch takes, so it never ran for a node whose own
            // layout box was degenerate. The rows kept painting at full size,
            // in their old on-screen position, for exactly as long as the
            // wrapper's height said not to: always, since nothing here ever
            // asked. Pushing the clip this node's own `overflow` already asks
            // for — its own (zero-area) rect — before painting children keeps
            // #142's case exactly as it was: an absolutely-positioned escapee
            // is never inside an ancestor clip that doesn't exist. It only
            // starts clipping the case #142 didn't have in front of it.
            //
            // Both halves of that come from [`clip::clip_shape`], the way the
            // full-size bracket below does, because this is an *eighth* clip
            // site and #324 stage A landed to stop there being eight answers.
            // An earlier draft of this hunk spelled the predicate as
            // `overflow_y` against `Hidden | Scroll | Auto` — one of the four
            // spellings stage A deleted, and the one that misses
            // `overflow: clip`. The radii come back and are dropped rather
            // than pushed as a `RoundedRect`, and the reason is the *clamp*,
            // not the resolution. Measured, because the obvious argument is
            // false: [`clip::border_radii`] resolves against
            // `min(width, height)`, which is `0` here, so a **percentage**
            // radius does come back as `0` — but `LengthPercentageValue::resolve`
            // ignores the basis for a `Length`, so `border-radius: 10px` on a
            // `height: 0` box hands back `{10, 10, 10, 10}`. What makes the two
            // shapes identical anyway is kurbo: `Rect::to_rounded_rect` clamps
            // every corner to half the shorter side, which is `0` for a
            // zero-area rect, so both spellings arrive at square corners.
            // Checked against `10px` and `50%`, not reasoned from the type.
            //
            // #536 (paint clips to the border box where CSS clips to the
            // padding box) cannot interact here either: `layout.height` *is*
            // the border box, so a collapsed box carrying a `border-width`
            // never reaches this branch at all.
            //
            // Built once and handed down, exactly like the full-size bracket's
            // `root_clip` below, because opening a bracket and telling
            // `paint_children_with_stacking` that none is open are the same
            // mistake spelled two ways: it *pops* this bracket around a
            // `position: fixed` entry and puts it back (#545), so a `None` here
            // means the pop never happens and every fixed box hoisted to this
            // root is swallowed by a clip that is not its containing block's.
            // The `debug_assert!` at the top of that function is what makes the
            // omission loud rather than a wrong render.
            let root_clip: Option<PaintShape> =
                clip_shape(node, scale, x, y).map(|(clip_rect, _radii)| clip_rect.into());
            if let Some(shape) = &root_clip {
                painter.push_clip(Fill::NonZero, node_transform, shape);
            }

            let scroll_x = node.scroll_offset.0 * scale;
            let scroll_y = node.scroll_offset.1 * scale;
            paint_children_with_stacking(
                tree,
                node_id,
                painter,
                scale,
                x - scroll_x,
                y - scroll_y,
                font_cx,
                layout_cx,
                node_transform,
                root_clip.as_ref(),
                false,
            );

            if root_clip.is_some() {
                painter.pop_layer();
            }

            if has_opacity {
                painter.pop_layer();
            }
        }
        return;
    }

    // Absolute position of this node
    let x = offset_x + layout.x as f64 * scale;
    let y = offset_y + layout.y as f64 * scale;
    let w = layout.width as f64 * scale;
    let h = layout.height as f64 * scale;

    // Sticky position adjustment
    let (x, y) = if node.computed_style.position == PositionValue::Sticky {
        // Find the nearest scroll ancestor's scroll offset
        let mut scroll_y = 0.0_f64;
        let mut ancestor_id = node.parent;
        while let Some(aid) = ancestor_id {
            if let Some(ancestor) = tree.get(aid) {
                let ov = ancestor.computed_style.overflow_y;
                if matches!(
                    ov,
                    OverflowValue::Auto | OverflowValue::Scroll | OverflowValue::Hidden
                ) {
                    scroll_y = ancestor.scroll_offset.1 * scale;
                    break;
                }
                ancestor_id = ancestor.parent;
            } else {
                break;
            }
        }

        // Apply sticky top constraint: element should not scroll above `top` offset from container
        let sticky_top = node.computed_style.top.to_px() as f64 * scale;
        let adjusted_y = y.max(offset_y + scroll_y + sticky_top);
        (x, adjusted_y)
    } else {
        (x, y)
    };

    // Compose this node's CSS transform (about its transform-origin) onto
    // the parent transform. Shared by every paint arm below and by the cull
    // test — a transformed node renders at its visual position, not its
    // layout rect (#143).
    let node_transform = compose_node_transform(node, x, y, scale, parent_transform);

    // Dirty region optimization: skip drawing for nodes entirely outside
    // the dirty region. If overflow is clipped or node is a leaf, skip the
    // entire subtree. Otherwise skip only this node's drawing but still
    // recurse into children (they may have absolute positioning inside the
    // dirty region).
    //
    // Cull against the transformed bounding box; untransformed nodes keep
    // the plain AABB test (#143).
    //
    // The rect is the box grown by the node's own ink — `box-shadow`,
    // `outline`, `text-shadow` — so damage over a shadow's bleed and not its
    // box repaints the shadow there, where it used to be cleared and left
    // blank (#889 item 1).
    let ink = ink_rect(&node.computed_style, x, y, w, h, scale);
    let node_outside_dirty = if node_transform == Affine::IDENTITY {
        !intersects_dirty_region(ink.x0, ink.y0, ink.width(), ink.height())
    } else {
        let bbox = node_transform.transform_rect_bbox(ink);
        !intersects_dirty_region(bbox.x0, bbox.y0, bbox.width(), bbox.height())
    };
    // **A box outside says nothing about a box that is not in its coordinate
    // space.** A `position: fixed` descendant is hoisted to its nearest
    // stacking-context ancestor and painted there with *zeroed* offsets, in
    // viewport space — so when that ancestor both clips and is outside, the
    // `return` below discards a box that is on screen. Measured: an
    // `overflow: hidden; z-index: 3` box at `top: 3000px` holding a fixed box
    // at `(300, 100)` paints 6400 green pixels without this guard and none
    // with it.
    //
    // This is the fifth instance of the shape #547 named three times over —
    // *a walk that stops early may not claim anything about what it did not
    // visit* — and it is here rather than filed because it changed character.
    // It was reachable only through a dirty region, where the pruned root has
    // to fall outside the damaged rect; card K43's off-window cull gives
    // `intersects_dirty_region` a second, always-present region, which makes it
    // fire on every full repaint.
    //
    // `creates_stacking_context` is the exact gate and not an approximation:
    // a fixed box hoists to the nearest stacking context, so a node that is not
    // one can never own such an entry, and anything hoisted *past* this node is
    // painted by an ancestor that this prune does not touch. The other arm —
    // skip-draw-and-recurse — needs no guard, because it already paints the
    // stacking sequence, hoisted entries and all. A childless node owns nothing
    // either way.
    //
    // Declining the shortcut means painting the node in full rather than taking
    // the cheaper skip-draw path, because that path pushes no clip: its
    // non-hoisted children would then paint *unclipped*, which trades a rare
    // missing box for a rare escaping one.
    //
    // **So the box is the wrong thing to ask, and #562 is what that cost.**
    // Declining for every stacking context meant an `opacity < 1` one parked
    // below the fold allocated, filled and composited a whole-surface pixmap
    // every frame — 26.7ms against 0.8ms on three full-screen `opacity: 0.5`
    // sheets at 1080x2460, most of this cull's own win handed back.
    //
    // The question is not "is this node a stacking context" but **"can anything
    // in this subtree paint on screen"**, and `layer_bounds` already answers
    // that for the two other callers that need it. So a stacking context is
    // pruned on a *definite* subtree extent that misses the target, and painted
    // in full on every not-knowing — a `position: fixed` descendant (which is
    // #561's case, and makes the extent escape by construction), a sticky one,
    // the visit budget, `MAX_DEPTH`. That is a bounded subtree walk in place of
    // a 10MB pixmap.
    //
    // **The narrowing #562 originally proposed — decline only for a stacking
    // context that also clips — is not lossless, and the difference is not one
    // a pixel count catches.** It hands the non-clipping ones to the
    // skip-draw-and-recurse arm, which sits before every `push_layer` here, so
    // the sheet's on-screen fixed descendant still painted and came back at
    // `[0, 200, 0, 255]` instead of `[0, 100, 0, 128]` — unfaded, with all 1017
    // tests in the crate green.
    let may_own_hoisted_entries = !node.children.is_empty() && node.creates_stacking_context();
    if node_outside_dirty && may_own_hoisted_entries {
        // The walk is only ever asked about a subtree paint is otherwise about
        // to draw in full, so on a `true` it replaces that paint rather than
        // adding to it. On a `false` it is an addition, not an alternative —
        // the subtree is painted anyway and the walk was overhead — which is
        // bounded by `MAX_VISITS` and measured at the adverse shape in that
        // constant's own doc. A `false` otherwise leaves this node exactly where
        // #561 left it.
        if subtree_is_entirely_outside(tree, node_id, scale, x, y, node_transform, |r| {
            intersects_dirty_region(r.x0, r.y0, r.width(), r.height())
        }) {
            return;
        }
    } else if node_outside_dirty {
        if node.clips_overflow() || node.children.is_empty() {
            return;
        }
        // Skip drawing this node but recurse into children.
        //
        // At the children's own origin, like every other call in this function:
        // a container's children are laid out against its *scrolled* content
        // origin, so passing the unscrolled one would place the whole subtree
        // `scroll_offset` too far down and right. That was #408 — a latent
        // trap rather than a live bug, because a node only carries a scroll
        // offset if it scrolls and a node that scrolls answers
        // `clips_overflow`, so the guard one line up already returned. Both
        // halves of that argument read the same predicate, except in one place
        // since #591 PR 1: a non-atomic inline element never clips, while the
        // scroll-container walks (`find_scroll_container_at_point_recursive`,
        // `clamp_scroll_offsets`, the scrollbar geometry) read `overflow`
        // against `Scroll | Auto` directly, so a `<span style="overflow: auto">`
        // can carry a scroll offset and not clip. Its box is `0x0`, so no ink is
        // at stake — and correctness here should not depend on a guard above it
        // at all.
        let scroll_x = node.scroll_offset.0 * scale;
        let scroll_y = node.scroll_offset.1 * scale;
        paint_children_with_stacking(
            tree,
            node_id,
            painter,
            scale,
            x - scroll_x,
            y - scroll_y,
            font_cx,
            layout_cx,
            node_transform,
            // Ditto: the skip-drawing branch returns before the clip push.
            None,
            false,
        );
        return;
    }

    match &node.kind {
        NodeKind::Element(el) if el.tag == "svg" => {
            paint_svg(tree, node, painter, scale, x, y, w, h, node_transform);
        }
        NodeKind::Element(el) if el.tag == "img" => {
            let rect = Rect::new(x, y, x + w, y + h);

            let fit = node.computed_style.object_fit;

            // Opacity. Bounds as in the general element arm below: the
            // subtree's painted extent, not this element's border box.
            let opacity = node.computed_style.opacity;
            if opacity < 1.0 {
                let bounds = opacity_layer_shape(tree, node_id, scale, x, y);
                painter.push_layer(BlendMode::Normal, opacity, node_transform, &bounds);
            }

            // Paint background (if any) before image
            let visible = !matches!(
                node.computed_style.visibility,
                VisibilityValue::Hidden | VisibilityValue::Collapse
            );
            if visible {
                if let BackgroundValue::Color(bg_color) = &node.computed_style.background {
                    painter.fill_color(Fill::NonZero, node_transform, *bg_color, &rect.into());
                }

                // Paint the image itself
                if let Some(src) = node.attributes.get("src")
                    && let Some(decoded) = tree.image_cache.get(src)
                {
                    image::paint_image(painter, decoded, rect, scale, fit, node_transform);
                }

                // Borders (no border-radius for img elements)
                paint_borders(painter, node, scale, x, y, w, h, 0.0.into(), node_transform);
            }

            if opacity < 1.0 {
                painter.pop_layer();
            }
        }
        // Inline painting for a `data-viewport` node whose frame arrives by
        // name — the software backend's video path (issue #358).
        //
        // Sibling of the `data-render-surface` arm below and for the same
        // reason: a frame drawn *here*, during paint, sits at the node's own
        // z-order, so a drawer or a modal painted after it covers it by
        // ordinary paint order. The software backend used to blit video onto
        // the finished pixel buffer instead, clipped only by its
        // overflow-clipping ancestors, which destroyed every overlay above it.
        //
        // The guard is the map, not the attribute: with no entry for this name
        // the node falls through to normal element painting, which is what
        // leaves the whole GPU compositor path untouched — that backend never
        // sets `VIEWPORT_PIXELS` at all. A `GameViewport` takes this arm too
        // (issue #361); what sets it apart from video is its hole, below.
        NodeKind::Element(_)
            if node
                .attributes
                .get("data-viewport")
                .is_some_and(|name| has_viewport_pixels(name)) =>
        {
            let rect = Rect::new(x, y, x + w, y + h);
            let opacity = node.computed_style.opacity;
            if opacity < 1.0 {
                let bounds = opacity_layer_shape(tree, node_id, scale, x, y);
                painter.push_layer(BlendMode::Normal, opacity, node_transform, &bounds);
            }

            let visible = !matches!(
                node.computed_style.visibility,
                VisibilityValue::Hidden | VisibilityValue::Collapse
            );
            if visible {
                // Opaque black over the whole box, then the frame fitted inside
                // it — so the letterbox bars are black, which is what a browser
                // paints for `<video>` (issue #354's software half).
                //
                // The black is paint's to draw, not the element's `background`:
                // rinch-video flips that to `transparent` the moment a frame
                // arrives, because the GPU backend composites video *under* the
                // UI and an opaque element background would hide it. rinch-video
                // cannot know which backend it is running on, so the backend
                // that paints the frame is the one that owns its backdrop.
                //
                // Rounded like any other background: a `border-radius` on the
                // viewport must not leave square black corners poking out of
                // the shape the author asked for, and the frame is clipped to
                // the same shape.
                let (backdrop, has_radius) = {
                    let cs = &node.computed_style;
                    let resolve_size = node.layout.width.min(node.layout.height);
                    let tl =
                        cs.border_radius_top_left.resolve(resolve_size).max(0.0) as f64 * scale;
                    let tr =
                        cs.border_radius_top_right.resolve(resolve_size).max(0.0) as f64 * scale;
                    let br =
                        cs.border_radius_bottom_right.resolve(resolve_size).max(0.0) as f64 * scale;
                    let bl =
                        cs.border_radius_bottom_left.resolve(resolve_size).max(0.0) as f64 * scale;
                    if tl > 0.0 || tr > 0.0 || br > 0.0 || bl > 0.0 {
                        let radii = RoundedRectRadii::new(tl, tr, br, bl);
                        (RoundedRect::from_rect(rect, radii).into(), true)
                    } else {
                        (painter::PaintShape::from(rect), false)
                    }
                };
                // Except over a hole. A viewport that punches one — a
                // `GameViewport`, which stamps no `data-viewport-ready` and is
                // active whenever it has a frame — has had its ancestors'
                // backgrounds cut away under it, and its letterbox *is* that
                // hole: see-through on a transparent window, as it was when
                // the frame was blitted over the finished pixels (#361).
                let viewport_name = node.attributes.get("data-viewport").map(String::as_str);
                if !viewport_name.is_some_and(|name| viewport_punches(node, name)) {
                    painter.fill_color(
                        Fill::NonZero,
                        node_transform,
                        AlphaColor::<Srgb>::BLACK,
                        &backdrop,
                    );
                }

                if has_radius {
                    painter.push_clip(Fill::NonZero, node_transform, &backdrop);
                }
                VIEWPORT_PIXELS.with(|vp| {
                    let guard = vp.borrow();
                    // The guard above already proved this entry exists and is
                    // drawable; `viewport_frame_bytes` re-derives the exact
                    // slice length the painter needs, because a buffer longer
                    // than `w * h * 4` would be rejected outright.
                    let Some((pixels, bytes)) = guard
                        .as_ref()
                        .and_then(|map| map.get(node.attributes.get("data-viewport")?))
                        .and_then(|pixels| Some((pixels, viewport_frame_bytes(pixels)?)))
                    else {
                        return;
                    };
                    image::paint_frame_data(
                        painter,
                        &pixels.data[..bytes],
                        pixels.width,
                        pixels.height,
                        pixels.opaque,
                        rect,
                        scale,
                        crate::computed_style::ObjectFitValue::Contain,
                        node_transform,
                    );
                });
                if has_radius {
                    painter.pop_layer();
                }
            }

            if opacity < 1.0 {
                painter.pop_layer();
            }
        }
        // Inline painting for render surfaces — draws pixels at the element's
        // position like <img>, participating in normal stacking and clipping.
        NodeKind::Element(_) if node.attributes.contains_key("data-render-surface") => {
            // The frame is this box's own content, like an `<img>`'s, so a
            // hidden surface draws neither it nor its background (#829).
            let surface_visible = !matches!(
                node.computed_style.visibility,
                VisibilityValue::Hidden | VisibilityValue::Collapse
            );
            let surface_painted = node
                .attributes
                .get("data-render-surface")
                .and_then(|id_str| id_str.parse::<usize>().ok())
                .map(|surface_id| {
                    SURFACE_PIXELS.with(|sp| {
                        let guard = sp.borrow();
                        if let Some(map) = guard.as_ref() {
                            if let Some(pixels) = map.get(&surface_id) {
                                if pixels.width > 0 && pixels.height > 0 && !pixels.data.is_empty()
                                {
                                    let rect = Rect::new(x, y, x + w, y + h);
                                    let opacity = node.computed_style.opacity;
                                    if opacity < 1.0 {
                                        let bounds =
                                            opacity_layer_shape(tree, node_id, scale, x, y);
                                        painter.push_layer(
                                            BlendMode::Normal,
                                            opacity,
                                            node_transform,
                                            &bounds,
                                        );
                                    }

                                    // Paint background behind the surface if any
                                    if surface_visible
                                        && let BackgroundValue::Color(bg_color) =
                                            &node.computed_style.background
                                    {
                                        painter.fill_color(
                                            Fill::NonZero,
                                            node_transform,
                                            *bg_color,
                                            &rect.into(),
                                        );
                                    }

                                    // Paint the surface pixels inline, like an
                                    // image — over the borrowed buffer. A live
                                    // frame source must not be cloned into a
                                    // `DecodedImage` first: that is a whole
                                    // frame of memcpy per frame, for nothing.
                                    if surface_visible {
                                        image::paint_frame_data(
                                            painter,
                                            &pixels.data,
                                            pixels.width,
                                            pixels.height,
                                            pixels.opaque,
                                            rect,
                                            scale,
                                            crate::computed_style::ObjectFitValue::Contain,
                                            node_transform,
                                        );
                                    }

                                    if opacity < 1.0 {
                                        painter.pop_layer();
                                    }
                                    return true;
                                }
                            }
                        }
                        false
                    })
                })
                .unwrap_or(false);

            if !surface_painted {
                // No pixel data yet — paint as a normal element (shows background)
                // Fall through to generic element painting below
            } else {
                return; // Surface painted inline, done
            }

            // Fall through: paint as generic element if no surface data
            let rect = Rect::new(x, y, x + w, y + h);
            let visible = !matches!(
                node.computed_style.visibility,
                VisibilityValue::Hidden | VisibilityValue::Collapse
            );
            let opacity = node.computed_style.opacity;
            if opacity < 1.0 {
                let bounds = opacity_layer_shape(tree, node_id, scale, x, y);
                painter.push_layer(BlendMode::Normal, opacity, node_transform, &bounds);
            }
            if visible {
                if let BackgroundValue::Color(bg_color) = &node.computed_style.background {
                    let radius =
                        node.computed_style.border_radius_top_left.to_px().max(0.0) as f64 * scale;
                    if radius > 0.0 {
                        let rrect = rect.to_rounded_rect(radius);
                        painter.fill_color(Fill::NonZero, node_transform, *bg_color, &rrect.into());
                    } else {
                        painter.fill_color(Fill::NonZero, node_transform, *bg_color, &rect.into());
                    }
                }
            }
            if opacity < 1.0 {
                painter.pop_layer();
            }
        }
        NodeKind::Element(_) => {
            let rect = Rect::new(x, y, x + w, y + h);
            let visible = !matches!(
                node.computed_style.visibility,
                VisibilityValue::Hidden | VisibilityValue::Collapse
            );

            // Get border-radius from computed style, resolving percentages
            // against the element's shorter side. `clip::border_radii` is the
            // one definition, so the clip shape and the background it clips
            // cannot round differently.
            let radii = border_radii(node, scale);
            // Uniform radius for code paths that don't support per-corner yet
            let radius =
                (radii.top_left + radii.top_right + radii.bottom_right + radii.bottom_left) / 4.0;

            // Get opacity from computed style and push layer if needed
            // The layer's bounds are the union of what the subtree
            // actually paints, not this element's border box —
            // [`opacity_layer_bounds`]. tiny-skia ignores the shape; Vello
            // clips every command in the layer to it, and a stacking context
            // does not clip its descendants, so a box shadow or an overflowing
            // child used to be drawn by one painter and thrown away by the
            // other (card K36).
            let opacity = node.computed_style.opacity;
            let has_opacity = opacity < 1.0;
            if has_opacity {
                let bounds = opacity_layer_shape(tree, node_id, scale, x, y);
                painter.push_layer(BlendMode::Normal, opacity, node_transform, &bounds);
            }

            // Handle overflow clipping — detect early so we can cut holes
            // in the background for viewport descendants. The two enums are
            // captured here for the scrollbar overlays at the bottom, which ask
            // a different question (is this a *scroll* container) than the clip
            // bracket does.
            let overflow_y = node.computed_style.overflow_y;
            let overflow_x = node.computed_style.overflow_x;
            // The clip shape comes from `clip::clip_shape` so that everything
            // that needs to know where this box clips asks one function:
            // `layer_bounds` and the dirty-region prune today, a hoisted
            // entry's clip chain in #324's second stage.
            let clip = clip_shape(node, scale, x, y);
            let clips = clip.is_some();

            // Whether the background below is going to write a pixel at all.
            //
            // Usually it is not, because the colour is fully transparent, which
            // is the case for most elements on most pages. `background-color`'s
            // initial value is `transparent`, and Stylo hands that back as a
            // real colour
            // rather than as an absence, so `from_stylo` turns it into
            // `BackgroundValue::Color(rgba(0, 0, 0, 0))` — a value this code
            // then dutifully filled. A `SourceOver` fill at alpha 0 writes no
            // pixel, so every one of those was a rasterisation of the element's
            // whole box for no output at all, and it cost in proportion to the
            // box: on the moto g stylus 5G a full-screen one measured 11–12ms,
            // and a single frame of the library screen spent about 70ms on
            // eight of them. See card K24.
            //
            // The skip is here, in paint, and deliberately not in `from_stylo`:
            // `transition/diff.rs` interpolates a `background-color` transition
            // only when both ends are `Color`, so collapsing transparent to
            // `None` in the computed style would silently stop
            // `transparent → red` from animating. Paint is the layer that gets
            // to decide something is not worth drawing; the style has to keep
            // the colour it was given.
            let paints_a_background = visible
                && match &node.computed_style.background {
                    BackgroundValue::None => false,
                    BackgroundValue::Color(c) => c.components[3] > 0.0,
                    _ => true,
                };

            // **Two ways a clip layer can be certain to clip nothing, and
            // neither of them is free to push.**
            //
            // Vello implements every clip as a blend-stack layer: for each
            // 16x16 tile the clip's bounding box touches, the fine stage saves
            // the tile's pixels on the way in and blends them back on the way
            // out, whatever is drawn between. That is two extra passes over
            // the clipped area, paid whether or not the clip removes a single
            // pixel. The software painter pays a comparable price for the same
            // reason. So a clip that provably removes nothing is worth not
            // pushing, and on this app's library screen 39 of the 41 clips in
            // a frame were exactly that.
            //
            // The first case is a clip that contains the whole render target.
            // It can only remove pixels that are outside the window and are
            // discarded anyway. On the library screen that is the app shell's
            // own root, `overflow: hidden` over the full 1080x2460 — 1.2ms of
            // a 17.8ms frame on the moto g stylus 5G, for a layer whose entire
            // effect was to copy the screen out and copy it back.
            //
            // The second case is a box nothing inside reaches past; see
            // [`clip_cuts_nothing`], which is `layer_bounds`' own walk asked to
            // stop one intersection short. Every row title on that screen is
            // one: `overflow: hidden` with `text-overflow: ellipsis`, where the
            // ellipsis has already shortened the string so that it fits, so
            // the clip has nothing left to cut. Eighteen of those, plus the
            // chips and the search field, were 2.8ms.
            //
            // It is that walk and not a local one because the local one was
            // wrong: it measured an IFC root's text from the border-box origin
            // while paint draws it at the content origin, so a padded box
            // elided a clip that its own text was entirely outside — 268 ink
            // pixels past the clip on the measured fixture. `layer_bounds`
            // mirrors paint node for node and already had that right, along
            // with a visit budget, `MAX_DEPTH`, and the not-knowing states.
            //
            // Together with the off-window cull in
            // [`intersects_dirty_region`] this takes the frame from 17.8ms of
            // GPU to 12.0ms. See card K43 for the full per-class table.
            //
            // **The suppression applies to the clip layer and deliberately not
            // to `clips`.** The two are one variable in the commit this came
            // from, and that was wrong even there: `clips` is also the cheap
            // gate on the viewport hole-punch walk below, and a box whose clip
            // is elided still has to cut holes for any compositor viewport
            // underneath it — eliding a layer that removes no pixels must not
            // also stop the background being cut. So the shape goes to `None`,
            // the fact stays true, and the pop at the bottom follows the push
            // rather than the fact.
            //
            // **Both cases require a square clip, and that is not a detail.**
            // A rounded clip removes the corners *of its own box*, so "nothing
            // inside reaches past the box" does not mean "nothing is cut" — the
            // clip shape is smaller than the box it was derived from. The
            // commit this came from tested `radius` in the first case only,
            // where it reads as being about the bounding box, and left the
            // second uncovered: a `border-radius: 40px; overflow: hidden` box
            // with a child exactly its own size had its corners stop being cut.
            // Caught by stage A's `a_rounded_clip_cuts_its_corners`, which did
            // not exist when this was written, so the guard is now on the whole
            // suppression rather than on one arm of it.
            let mut useless = false;
            if clips && radius <= 0.0 {
                // A rotated or skewed clip is not its own bounding box, so
                // only an axis-aligned one may be tested this way.
                let m = node_transform.as_coeffs();
                let axis_aligned = m[1].abs() < 1e-9 && m[2].abs() < 1e-9;
                let bbox = node_transform.transform_rect_bbox(rect);
                let covers_target = axis_aligned
                    && VIEWPORT.with(|v| match v.get() {
                        None => false,
                        Some(vp) => {
                            bbox.x0 <= vp.target.x0 + 0.5
                                && bbox.y0 <= vp.target.y0 + 0.5
                                && bbox.x1 >= vp.target.x1 - 0.5
                                && bbox.y1 >= vp.target.y1 - 0.5
                        }
                    });
                if covers_target || clip_cuts_nothing(tree, node_id, scale, x, y, rect) {
                    useless = true;
                }
            }
            let clip = if useless { None } else { clip };
            let pushes_clip = clip.is_some();

            // Find viewport descendants — their rects will be cut out of
            // the background fill so the compositor layer shows through.
            //
            // Only worth walking the subtree for when there *is* a background
            // fill or an inset shadow to cut them out of (#974): the holes
            // have no other consumer, and `clips` is true of every
            // `overflow: hidden` box on the page.
            let mut viewport_holes = Vec::new();
            let has_inset_shadow =
                visible && node.computed_style.box_shadow.iter().any(|s| s.inset);
            if clips && (paints_a_background || has_inset_shadow) {
                find_viewport_rects(
                    tree,
                    node_id,
                    scale,
                    offset_x,
                    offset_y,
                    &mut viewport_holes,
                );
            }

            // Only paint this element's own visuals if visible
            // (children may override with visibility: visible)
            if visible {
                // Render box-shadow from computed style
                if !node.computed_style.box_shadow.is_empty() {
                    paint_box_shadow(
                        painter,
                        &node.computed_style.box_shadow,
                        x,
                        y,
                        w,
                        h,
                        scale,
                        node,
                        node_transform,
                    );
                }

                // Get background from computed style (solid color or gradient).
                // When viewport_holes is non-empty, we paint the background with
                // holes cut out (EvenOdd fill) so compositor layers show through.
                // `viewport_holes` is collected for a background that will be
                // painted or an inset shadow (#974), so the background arm
                // asks `paints_a_background` too.
                if paints_a_background && !viewport_holes.is_empty() {
                    // Build a compound path: outer shape + inner holes (wound opposite)
                    let bg_path = build_background_with_holes(rect, radii, radius, &viewport_holes);
                    match &node.computed_style.background {
                        BackgroundValue::Color(bg_color) => {
                            painter.fill_color(
                                Fill::EvenOdd,
                                node_transform,
                                *bg_color,
                                &bg_path.into(),
                            );
                        }
                        BackgroundValue::LinearGradient {
                            angle_degrees,
                            stops,
                        } => {
                            let brush = build_linear_gradient_brush(*angle_degrees, stops, &rect);
                            painter.fill(Fill::EvenOdd, node_transform, &brush, &bg_path.into());
                        }
                        BackgroundValue::RadialGradient { stops } => {
                            let brush = build_radial_gradient_brush(stops, &rect);
                            painter.fill(Fill::EvenOdd, node_transform, &brush, &bg_path.into());
                        }
                        BackgroundValue::Image { url } => {
                            if let Some(decoded) = tree.image_cache.get(url) {
                                image::paint_image(
                                    painter,
                                    decoded,
                                    rect,
                                    scale,
                                    crate::computed_style::ObjectFitValue::Fill,
                                    node_transform,
                                );
                            }
                        }
                        BackgroundValue::None => {}
                    }
                } else if paints_a_background {
                    match &node.computed_style.background {
                        BackgroundValue::Color(bg_color) => {
                            if radius > 0.0 {
                                let rrect = rect.to_rounded_rect(radii);
                                painter.fill_color(
                                    Fill::NonZero,
                                    node_transform,
                                    *bg_color,
                                    &rrect.into(),
                                );
                            } else {
                                painter.fill_color(
                                    Fill::NonZero,
                                    node_transform,
                                    *bg_color,
                                    &rect.into(),
                                );
                            }
                        }
                        BackgroundValue::LinearGradient {
                            angle_degrees,
                            stops,
                        } => {
                            let brush = build_linear_gradient_brush(*angle_degrees, stops, &rect);
                            if radius > 0.0 {
                                let rrect = rect.to_rounded_rect(radii);
                                painter.fill(Fill::NonZero, node_transform, &brush, &rrect.into());
                            } else {
                                painter.fill(Fill::NonZero, node_transform, &brush, &rect.into());
                            }
                        }
                        BackgroundValue::RadialGradient { stops } => {
                            let brush = build_radial_gradient_brush(stops, &rect);
                            if radius > 0.0 {
                                let rrect = rect.to_rounded_rect(radii);
                                painter.fill(Fill::NonZero, node_transform, &brush, &rrect.into());
                            } else {
                                painter.fill(Fill::NonZero, node_transform, &brush, &rect.into());
                            }
                        }
                        BackgroundValue::Image { url } => {
                            if let Some(decoded) = tree.image_cache.get(url) {
                                image::paint_image(
                                    painter,
                                    decoded,
                                    rect,
                                    scale,
                                    crate::computed_style::ObjectFitValue::Fill,
                                    node_transform,
                                );
                            }
                        }
                        BackgroundValue::None => {}
                    }
                }

                // Inset shadows: above the background, below the border
                // (css-backgrounds-3 §7.1; #974).
                if !node.computed_style.box_shadow.is_empty() {
                    borders::paint_inset_box_shadow(
                        painter,
                        &node.computed_style.box_shadow,
                        x,
                        y,
                        w,
                        h,
                        scale,
                        radii,
                        node,
                        node_transform,
                        &viewport_holes,
                        &tree.perf,
                    );
                }

                // Render borders per-side with style support
                paint_borders(painter, node, scale, x, y, w, h, radii, node_transform);

                // Render outline (drawn outside the box model)
                paint_outline(painter, node, scale, x, y, w, h, radii, node_transform);

                // Render input element value
                if matches!(node.tag(), Some("input" | "textarea")) {
                    paint_input_value(
                        node,
                        &tree.perf,
                        painter,
                        scale,
                        x,
                        y,
                        w,
                        h,
                        font_cx,
                        layout_cx,
                        node_transform,
                    );
                } else if node.tag() == Some("select") {
                    // The closed control's label + arrow. Options are display:none
                    // (they don't lay out); the selected label is painted here.
                    select::paint_select_value(
                        tree,
                        node_id,
                        painter,
                        scale,
                        x,
                        y,
                        w,
                        h,
                        font_cx,
                        layout_cx,
                        node_transform,
                    );
                }
            }

            // Built once and kept, because `paint_children_with_stacking` has
            // to *lift* this exact bracket around a `position: fixed` entry
            // (#545) and put it back afterwards. Re-deriving the shape there
            // would be a second derivation to drift out of step with this one.
            let root_clip: Option<PaintShape> = clip.map(|(clip_rect, clip_radii)| {
                if radius > 0.0 {
                    clip_rect.to_rounded_rect(clip_radii).into()
                } else {
                    clip_rect.into()
                }
            });
            if let Some(shape) = &root_clip {
                painter.push_clip(Fill::NonZero, node_transform, shape);
            }

            // Render read-only text selection highlight (user-select: text).
            // It highlights this box's own text, so it goes with it (#829).
            if visible
                && node
                    .attributes
                    .get("data-text-sel")
                    .map(|s| s == "true")
                    .unwrap_or(false)
            {
                let sel_start = node
                    .attributes
                    .get("data-text-sel-start")
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);
                let sel_end = node
                    .attributes
                    .get("data-text-sel-end")
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);

                if sel_start != sel_end {
                    if let Some(ref inline_layout) = node.text_layout {
                        let cs = &node.computed_style;
                        let (off_x, off_y) = ifc_root_content_origin(node);
                        let pad_x = off_x as f64 * scale;
                        let pad_y = off_y as f64 * scale;
                        let text_x = x + pad_x;
                        let text_y = y + pad_y;
                        let text_len = inline_layout.text_content.len();
                        let content_width = node.layout.width as f64 * scale
                            - (cs.padding_left.to_px() + cs.padding_right.to_px()) as f64 * scale;
                        paint_text_selection_highlight(
                            node,
                            painter,
                            scale,
                            text_x,
                            text_y,
                            &inline_layout.layout,
                            text_len,
                            sel_start,
                            sel_end,
                            content_width,
                            node_transform,
                        );
                    }
                }
            }

            // Check if this is an IFC root with a cached inline layout
            if let Some(inline_layout) = &node.text_layout {
                // Paint inline content at the content-box origin (inside padding+border),
                // accounting for scroll offset.
                let (off_x, off_y) = ifc_root_content_origin(node);
                let scroll_x = node.scroll_offset.0 * scale;
                let scroll_y = node.scroll_offset.1 * scale;
                let content_x = x + off_x as f64 * scale - scroll_x;
                let content_y = y + off_y as f64 * scale - scroll_y;
                let ifc_text_shadows = node.computed_style.text_shadow.as_slice();
                paint_inline_layout(
                    tree,
                    painter,
                    scale,
                    content_x,
                    content_y,
                    inline_layout,
                    font_cx,
                    layout_cx,
                    ifc_text_shadows,
                    node_transform,
                    // The root's own visibility answers only for text no range
                    // maps to; each text run follows its own element (#829).
                    !visible,
                );

                // Still paint non-inline (block) children normally
                paint_children_with_stacking(
                    tree,
                    node_id,
                    painter,
                    scale,
                    x - scroll_x,
                    y - scroll_y,
                    font_cx,
                    layout_cx,
                    node_transform,
                    root_clip.as_ref(),
                    useless,
                );
            } else {
                // Normal paint path: recurse into all children
                let scroll_x = node.scroll_offset.0 * scale;
                let scroll_y = node.scroll_offset.1 * scale;
                paint_children_with_stacking(
                    tree,
                    node_id,
                    painter,
                    scale,
                    x - scroll_x,
                    y - scroll_y,
                    font_cx,
                    layout_cx,
                    node_transform,
                    root_clip.as_ref(),
                    useless,
                );
            }

            // Follows the *push*, not `clips` — a clip elided above as
            // provably removing nothing was never pushed, and popping it would
            // take a layer off the stack that belongs to somebody else.
            if pushes_clip {
                painter.pop_layer();
            }

            // Paint scrollbar overlays for scroll containers.
            //
            // Both axes, from one set of metrics (#178): thickness 6, margin 2,
            // minimum thumb 20, 40% black, fully rounded — the numbers the
            // vertical bar has always used, so the two read as one feature.
            //
            // The geometry itself lives in `paint::scrollbar`, shared with the
            // desktop input path, because the two used to derive it separately
            // and had drifted apart (#400): a drag did not move the thumb the
            // distance the pointer moved. Anything about *where* a bar is
            // belongs there; what is left here is how it is drawn.
            //
            // Gated up front on the two overflow enums captured before the
            // children were painted, so a node that scrolls on neither axis —
            // almost every node — pays two enum checks and nothing else.
            //
            // A scrollbar is the container's own visual, so a hidden container
            // draws none (#829) — hit testing already offers none to press.
            if visible
                && (matches!(overflow_y, OverflowValue::Scroll | OverflowValue::Auto)
                    || matches!(overflow_x, OverflowValue::Scroll | OverflowValue::Auto))
            {
                let node = tree.get(node_id).unwrap(); // re-borrow after children done
                let bars = scrollbar::scrollbars(tree, node_id, scale);
                let thickness = bars.thickness;
                let margin = bars.margin;

                // The thumb's colour follows `--rinch-scrollbar-color`, or the
                // container's own palette when that says `auto` (#416).
                // `--rinch-scrollbar-width: none` reaches here as "no bars",
                // so nothing below runs for it.
                let mut fill_rounded = |rect: Rect, colour: AlphaColor<Srgb>| {
                    let shape = RoundedRect::from_rect(rect, thickness * 0.5);
                    painter.fill(
                        Fill::NonZero,
                        node_transform,
                        &Brush::Solid(colour),
                        &shape.into(),
                    );
                };

                if let Some(track) = bars.vertical {
                    let scrollbar_x = x + w - thickness - margin;
                    // A track is only painted when asked for: rinch's bar is an
                    // overlay, and a track under it would change the look of
                    // every existing app.
                    if let Some(track_color) = bars.track_color {
                        fill_rounded(
                            Rect::new(
                                scrollbar_x,
                                y + track.track_start,
                                scrollbar_x + thickness,
                                y + track.track_start + track.track_len,
                            ),
                            track_color,
                        );
                    }
                    let thumb_y = y + track.thumb_start(node.scroll_offset.1 * scale);
                    fill_rounded(
                        Rect::new(
                            scrollbar_x,
                            thumb_y,
                            scrollbar_x + thickness,
                            thumb_y + track.thumb_len,
                        ),
                        bars.thumb_color,
                    );
                }

                if let Some(track) = bars.horizontal {
                    let scrollbar_y = y + h - thickness - margin;
                    if let Some(track_color) = bars.track_color {
                        fill_rounded(
                            Rect::new(
                                x + track.track_start,
                                scrollbar_y,
                                x + track.track_start + track.track_len,
                                scrollbar_y + thickness,
                            ),
                            track_color,
                        );
                    }
                    let thumb_x = x + track.thumb_start(node.scroll_offset.0 * scale);
                    fill_rounded(
                        Rect::new(
                            thumb_x,
                            scrollbar_y,
                            thumb_x + track.thumb_len,
                            scrollbar_y + thickness,
                        ),
                        bars.thumb_color,
                    );
                }
            }

            // Apply CSS filter approximations (after content is painted, before opacity pop)
            let cs = &tree.get(node_id).unwrap().computed_style;
            // Gated on the box's own visibility (#829): the approximation is an
            // overlay over this box's border box, so a hidden box would still
            // darken or grey whatever is behind it. (The real `filter` would
            // apply to a `visibility: visible` descendant; the overlay cannot
            // express that either way.)
            let has_filter = visible && (cs.filter_brightness != 1.0 || cs.filter_grayscale > 0.0);

            if has_filter {
                // Brightness: overlay black (darken) or white (brighten) with calculated alpha
                if cs.filter_brightness != 1.0 {
                    let brightness = cs.filter_brightness;
                    if brightness < 1.0 {
                        // Darken: overlay black with alpha = 1.0 - brightness.
                        // Hand the painter the float and let it do the single
                        // 8-bit quantisation, by rounding — `x as u8` truncated,
                        // biasing every filtered element one level light (#260).
                        let dark = AlphaColor::<Srgb>::BLACK
                            .with_alpha((1.0 - brightness).clamp(0.0, 1.0));
                        if radius > 0.0 {
                            let rrect = rect.to_rounded_rect(radii);
                            painter.fill_color(Fill::NonZero, node_transform, dark, &rrect.into());
                        } else {
                            painter.fill_color(Fill::NonZero, node_transform, dark, &rect.into());
                        }
                    } else if brightness > 1.0 {
                        // Brighten: overlay white with alpha proportional to
                        // excess brightness. Same rounding note as above.
                        let light = AlphaColor::<Srgb>::WHITE
                            .with_alpha((brightness - 1.0).clamp(0.0, 1.0));
                        if radius > 0.0 {
                            let rrect = rect.to_rounded_rect(radii);
                            painter.fill_color(Fill::NonZero, node_transform, light, &rrect.into());
                        } else {
                            painter.fill_color(Fill::NonZero, node_transform, light, &rect.into());
                        }
                    }
                }

                // Grayscale approximation: desaturation effect
                if cs.filter_grayscale > 0.0 {
                    let grayscale = cs.filter_grayscale.clamp(0.0, 1.0);
                    // Push a saturation layer: gray rect with Saturation blend at grayscale alpha
                    painter.push_layer(
                        BlendMode::Saturation,
                        grayscale,
                        node_transform,
                        &rect.into(),
                    );
                    // Fill with neutral gray
                    let gray = AlphaColor::<Srgb>::from_rgba8(128, 128, 128, 255);
                    if radius > 0.0 {
                        let rrect = rect.to_rounded_rect(radii);
                        painter.fill_color(Fill::NonZero, node_transform, gray, &rrect.into());
                    } else {
                        painter.fill_color(Fill::NonZero, node_transform, gray, &rect.into());
                    }
                    painter.pop_layer();
                }
            }

            if has_opacity {
                painter.pop_layer();
            }
        }

        NodeKind::Text(text_data) => {
            if text_data.content.is_empty() {
                return;
            }

            // Check visibility (inherited from parent)
            let parent_visibility = node
                .parent
                .and_then(|p| tree.get(p))
                .map(|p| &p.computed_style.visibility);
            if matches!(
                parent_visibility,
                Some(VisibilityValue::Hidden | VisibilityValue::Collapse)
            ) {
                return;
            }

            // Use cached layout if available (built after Taffy layout with final widths)
            if let Some(cached_layout) = &node.cached_text_parley {
                // Layout is already aligned during caching, use it directly
                let text_shadows = node
                    .parent
                    .and_then(|p| tree.get(p))
                    .map(|p| p.computed_style.text_shadow.as_slice())
                    .unwrap_or(&[]);
                // Painted in the parent's **current** colour, not the brush
                // the layout was shaped with (#904): a leaf's layout is rebuilt
                // only by a compute, and a colour-only change — a hover, every
                // frame of a `transition: color` — must reach the glyphs
                // without one. The same default as the fallback below.
                let color = node
                    .parent
                    .and_then(|p| tree.get(p))
                    .and_then(|p| p.computed_style.color)
                    .unwrap_or_else(|| AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 255));
                render_text_with_shadow(
                    painter,
                    cached_layout,
                    x,
                    y,
                    text_shadows,
                    parent_transform,
                    scale,
                    None,
                    Some(color),
                    None,
                );
                return;
            }

            // Fallback: build layout on demand (should not happen with caching)
            let parent_node = node.parent.and_then(|p| tree.get(p));
            let parent_computed = parent_node.map(|p| &p.computed_style);

            let font_size = parent_computed.map(|s| s.font_size).unwrap_or(16.0);
            let font_weight = parent_computed.map(|s| s.font_weight).unwrap_or(400.0);
            let font_family = parent_computed
                .map(|s| {
                    if s.font_family.is_empty() {
                        "sans-serif".to_string()
                    } else {
                        s.font_family.clone()
                    }
                })
                .unwrap_or_else(|| "sans-serif".to_string());

            let color = parent_computed
                .and_then(|s| s.color)
                .unwrap_or_else(|| AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 255));

            // Build the Parley layout at the *logical* font size (scale 1.0) — the
            // same space `measure_inline_blocks` sized the box in and the same space
            // the cached IFC path (`build_inline_layout(..., 1.0, ...)`) uses. The
            // painter re-applies `scale` in `render_text`. (Building at `font_size *
            // scale` here would both double-scale the glyphs at render *and*, because
            // glyph advances don't scale perfectly linearly, make this layout's width
            // diverge from the width the box was measured at — breaking the wrap
            // constraint below at fractional scale factors.)
            tree.perf.bump(crate::perf::Counter::ShapePaint);
            let mut builder = layout_cx.ranged_builder(font_cx, &text_data.content, 1.0, true);
            builder.push_default(parley::style::StyleProperty::FontSize(font_size));
            builder.push_default(parley::style::StyleProperty::Brush(Brush::Solid(color)));
            builder.push_default(parley::style::StyleProperty::FontFamily(
                parley::style::FontFamily::Source(std::borrow::Cow::Owned(font_family)),
            ));
            if (font_weight - 400.0).abs() > 1.0 {
                builder.push_default(parley::style::StyleProperty::FontWeight(
                    parley::style::FontWeight::new(font_weight),
                ));
            }
            if let Some(lh) = parent_computed.and_then(|s| s.line_height.to_parley()) {
                builder.push_default(parley::style::StyleProperty::LineHeight(lh));
            }
            // letter-/word-spacing (#698) — in CSS pixels, like everything else
            // on this builder, which was made at scale 1.0 for the reason above.
            // Without these two the box this fallback paints into was measured
            // with the spacing and the glyphs would be drawn without it.
            let (letter_spacing, word_spacing) = parent_computed
                .map(|s| (s.letter_spacing, s.word_spacing))
                .unwrap_or((0.0, 0.0));
            builder.push_default(parley::style::StyleProperty::LetterSpacing(letter_spacing));
            builder.push_default(parley::style::StyleProperty::WordSpacing(word_spacing));

            let mut text_layout = builder.build(&text_data.content);

            // Lay out unwrapped first to get the natural (single-line) width & height.
            text_layout.break_all_lines(None);

            // This fallback fires for text that is not part of a cached IFC layout —
            // notably the sole text child of an `inline-block`, which is not an IFC
            // root (so it never gets a `cached_text_parley`). Such text was measured
            // at the parent's max-content width, so for an *auto*-width box painting
            // it unwrapped is correct. But once the box is clamped
            // (`width`/`max-width`/`min-width` narrower than the content) layout wraps
            // the text and sizes the box for the *wrapped* height, while these glyphs
            // would still paint one overflowing line (#127).
            //
            // We can't decide "is it clamped?" by comparing this layout's width to the
            // box: this on-demand layout's width drifts a pixel or two from the width
            // the box was measured at (a different Parley build path), so a width test
            // spuriously wraps auto-width boxes. Instead compare *heights*: layout
            // records this text node's wrapped height, and line counts are discrete —
            // if the node is more than ~1.5 single lines tall, layout wrapped it, so
            // the box is clamped and paint must wrap to the content box too.
            let one_line_h = text_layout.height();
            let clamped = one_line_h > 0.0 && node.layout.height > one_line_h * 1.5;
            if clamped && let Some(p) = parent_node {
                let cs = &p.computed_style;
                let padding_h = cs.padding_left.to_px() + cs.padding_right.to_px();
                let border_h = cs.border_left_width.to_px() + cs.border_right_width.to_px();
                let content_width = p.layout.width - padding_h - border_h;
                if content_width > 0.0 {
                    // Wrap to the content box, but 2% wider. This on-demand layout is a
                    // hair wider per glyph run than the one layout wrapped the box with,
                    // so breaking at exactly `content_width` fits fewer words per line
                    // and can spill one extra line past the box's reserved height. The
                    // small proportional slack reproduces layout's line breaks; any
                    // residual right overflow stays within the element's padding.
                    text_layout.break_all_lines(Some(content_width * 1.02));
                }
            }

            // Read text-align from parent's computed style
            let alignment = parent_computed
                .map(|s| s.text_align.to_parley())
                .unwrap_or(parley::layout::Alignment::Start);
            text_layout.align(alignment, parley::layout::AlignmentOptions::default());

            // Render text glyphs
            let text_shadows = parent_computed
                .map(|s| s.text_shadow.as_slice())
                .unwrap_or(&[]);
            render_text_with_shadow(
                painter,
                &text_layout,
                x,
                y,
                text_shadows,
                parent_transform,
                scale,
                None,
                None,
                None,
            );
        }

        _ => {} // Document, Comment -- invisible
    }
}

#[cfg(test)]
mod tests {
    use crate::RinchDocument;
    use peniko::kurbo::Affine;
    use rinch_core::dom::DomDocument;

    // The correctness criterion for every test here: `painted_border_box`
    // returns the box the node is *painted* in. So the expected rects are
    // hand-computed from the CSS — from the padding, the transform, the
    // hoisting rule — never from another geometry helper, or the test would
    // just re-derive whatever bug is in one.
    //
    // Each test also asserts the *wrong* answer explicitly: the box the
    // unhardened walk produced. A positive assertion alone survives most of
    // these regressions, because the two boxes overlap.

    fn child_of(doc: &mut RinchDocument, parent: rinch_core::dom::NodeId, style: &str) -> usize {
        let el = doc.create_element("div");
        doc.set_attribute(el, "style", style);
        doc.append_child(parent, el);
        el.0
    }

    /// The plain parent-chain sum: layout offsets minus ancestor scroll, with
    /// no transform and no IFC content-box offset. This is what every walk in
    /// the codebase used to do, and it is the box the assertions below must
    /// *not* return.
    fn untransformed_origin(doc: &RinchDocument, node_id: usize) -> (f32, f32) {
        let (mut x, mut y) = (0.0_f32, 0.0_f32);
        let mut cur = Some(node_id);
        while let Some(id) = cur {
            let Some(n) = doc.tree.get(id) else { break };
            x += n.layout.x;
            y += n.layout.y;
            if let Some(p) = n.parent.and_then(|pid| doc.tree.get(pid)) {
                x -= p.scroll_offset.0 as f32;
                y -= p.scroll_offset.1 as f32;
            }
            cur = n.parent;
        }
        (x, y)
    }

    /// A plain translated ancestor: the painted box is the layout box shifted
    /// by the ancestor's translate. This is the case `Drag::percent()` gets
    /// wrong on a slider inside a centred modal (#203).
    #[test]
    fn an_ancestor_translate_moves_the_painted_box() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let panel = child_of(
            &mut doc,
            body,
            "position: relative; width: 400px; height: 300px; \
             transform: translate(-50px, -30px)",
        );
        let track = child_of(
            &mut doc,
            rinch_core::dom::NodeId(panel),
            "position: absolute; left: 100px; top: 60px; width: 200px; height: 20px",
        );
        doc.resolve_layout(800.0, 600.0);

        // Layout box (100,60)-(300,80); translate(-50,-30) paints it at
        // (50,30)-(250,50).
        let r = super::painted_border_box(&doc.tree, track, 1.0);
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (50.0, 30.0, 250.0, 50.0));
        assert_eq!(
            untransformed_origin(&doc, track),
            (100.0, 60.0),
            "the untransformed sum is the box this must NOT report"
        );
    }

    /// A `scale()` ancestor changes the painted *size*, not just the origin —
    /// which is why an AABB helper cannot just offset the layout box.
    #[test]
    fn an_ancestor_scale_resizes_the_painted_box() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        // transform-origin at the top-left keeps the arithmetic hand-checkable.
        let zoom = child_of(
            &mut doc,
            body,
            "position: relative; width: 400px; height: 300px; \
             transform: scale(2); transform-origin: 0 0",
        );
        let track = child_of(
            &mut doc,
            rinch_core::dom::NodeId(zoom),
            "position: absolute; left: 40px; top: 25px; width: 100px; height: 10px",
        );
        doc.resolve_layout(800.0, 600.0);

        // Layout box (40,25)-(140,35), doubled about (0,0): (80,50)-(280,70).
        let r = super::painted_border_box(&doc.tree, track, 1.0);
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (80.0, 50.0, 280.0, 70.0));
        assert_eq!(
            (r.width(), r.height()),
            (200.0, 20.0),
            "a scaled box is painted at twice its layout size"
        );
    }

    /// Gap 1. A `display: contents` node generates no box, so `paint_node`'s
    /// zero-size branch passes the parent transform straight to the children —
    /// a transform declared on it is never applied. The forward walk used to
    /// compose it anyway and moved the child somewhere paint does not draw it.
    #[test]
    fn a_display_contents_ancestor_contributes_no_transform() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let container = child_of(
            &mut doc,
            body,
            "position: relative; width: 400px; height: 300px; padding: 30px 20px",
        );
        let ghost = child_of(
            &mut doc,
            rinch_core::dom::NodeId(container),
            "display: contents; transform: translate(50px, 20px)",
        );
        let child = child_of(
            &mut doc,
            rinch_core::dom::NodeId(ghost),
            "width: 80px; height: 40px",
        );
        doc.resolve_layout(800.0, 600.0);

        // The transform is really declared — otherwise this test proves nothing.
        assert!(
            !doc.tree
                .get(ghost)
                .unwrap()
                .computed_style
                .transform
                .is_identity,
            "the display:contents node must actually carry a transform"
        );

        // The child is flattened into the container's content box: (20,30),
        // and nothing translates it.
        let (_, _, t) = super::compute_absolute_position_and_transform(&doc.tree, child, 1.0);
        assert_eq!(t, Affine::IDENTITY, "no transform reaches the child");
        let r = super::painted_border_box(&doc.tree, child, 1.0);
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (20.0, 30.0, 100.0, 70.0));
        assert_ne!(
            (r.x0, r.y0),
            (70.0, 50.0),
            "the display:contents translate must not displace the child"
        );
    }

    /// Gap 2. A hoisted `position: fixed` box drops every ancestor transform
    /// but keeps the body's: `paint_children_with_stacking` paints a fixed entry
    /// under [`body_paint_transform`], whichever stacking context hoisted it
    /// (#545). Hit testing already agrees
    /// (`body_transform_applies_to_a_hoisted_fixed_descendant`); this walk
    /// started from the identity, so the two disagreed.
    #[test]
    fn a_hoisted_fixed_box_paints_under_the_body_transform() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        doc.set_attribute(body, "style", "transform: translate(40px, 30px)");
        let outer = child_of(
            &mut doc,
            body,
            "position: relative; width: 400px; height: 300px; \
             transform: translate(200px, 100px)",
        );
        let fixed = child_of(
            &mut doc,
            rinch_core::dom::NodeId(outer),
            "position: fixed; left: 50px; top: 60px; width: 100px; height: 40px; z-index: 5",
        );
        doc.resolve_layout(800.0, 600.0);

        // Viewport box (50,60)-(150,100), shifted by body's translate only:
        // (90,90)-(190,130). `outer`'s translate must not reach it.
        let r = super::painted_border_box(&doc.tree, fixed, 1.0);
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (90.0, 90.0, 190.0, 130.0));
        assert_ne!(
            (r.x0, r.y0),
            (50.0, 60.0),
            "the body's transform does reach a hoisted fixed box"
        );
        assert_ne!(
            (r.x0, r.y0),
            (290.0, 190.0),
            "the transformed ancestor's translate must not reach it"
        );
    }

    /// Gap 3. An inline-block an IFC positions stores `layout.x`/`layout.y`
    /// relative to the IFC root's *content* box, while the parent chain sums
    /// border-box origins. Paint bridges the two by handing
    /// `paint_inline_layout` the content-box origin, so a walk that omits
    /// `ifc_content_box_offset` reports the box one padding+border up-left of
    /// where it is drawn.
    #[test]
    fn an_inline_block_is_placed_against_its_ifc_content_box() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let container = doc.create_element("div");
        doc.set_attribute(
            container,
            "style",
            "width: 400px; padding: 20px; border: 4px solid black; font-size: 16px",
        );
        doc.append_child(body, container);
        let before = doc.create_text("Click the ");
        doc.append_child(container, before);
        let button = doc.create_element("button"); // inline-block by default
        doc.set_attribute(button, "style", "width: 60px; height: 24px");
        doc.append_child(container, button);
        let label = doc.create_text("OK");
        doc.append_child(button, label);
        doc.resolve_layout(800.0, 600.0);

        assert!(
            doc.tree.get(container.0).unwrap().text_layout.is_some(),
            "the container must be the IFC root"
        );
        // The button's own layout.x/y are Parley's, relative to the content
        // box; the chain sum lands on the border-box origin. The painted origin
        // is that sum plus padding (20) + border (4) on each axis.
        let (sum_x, sum_y) = untransformed_origin(&doc, button.0);
        let r = super::painted_border_box(&doc.tree, button.0, 1.0);
        assert_eq!(
            (r.x0 as f32, r.y0 as f32),
            (sum_x + 24.0, sum_y + 24.0),
            "the inline-block paints inside the IFC root's padding and border"
        );
        assert_ne!(
            (r.x0 as f32, r.y0 as f32),
            (sum_x, sum_y),
            "the plain border-box sum is one padding+border off"
        );
    }

    /// Gaps 2 and 3 meeting. A hoisted `position: fixed` box is painted by
    /// `paint_node` from the body's sequence at a zero offset, never by
    /// `paint_inline_layout`, so no IFC content-box offset may reach it even
    /// when it is written inline in a padded text flow. `hit_test_node` spends
    /// an explicit branch on this; the walk here does not need one, because
    /// Stylo blockifies an out-of-flow box and `ifc_content_box_offset` then
    /// answers `(0, 0)` on its own — which the assertion below states, so that
    /// a Stylo change removing that blockification fails here instead of
    /// silently displacing every fixed overlay written inside a paragraph.
    #[test]
    fn a_fixed_inline_block_keeps_its_viewport_origin() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let container = doc.create_element("div");
        doc.set_attribute(
            container,
            "style",
            "width: 400px; padding: 20px; border: 4px solid black; font-size: 16px",
        );
        doc.append_child(body, container);
        let before = doc.create_text("Overlay: ");
        doc.append_child(container, before);
        // Two `<button>`s, inline-block by tag, in the same IFC. The in-flow one
        // is the control: it must take the 24px offset, or the fixed one taking
        // none proves nothing about the exemption.
        let inflow = doc.create_element("button");
        doc.set_attribute(inflow, "style", "width: 60px; height: 24px");
        doc.append_child(container, inflow);
        let overlay = doc.create_element("button");
        doc.set_attribute(
            overlay,
            "style",
            "position: fixed; left: 30px; top: 40px; width: 60px; height: 24px",
        );
        doc.append_child(container, overlay);
        doc.resolve_layout(800.0, 600.0);

        let control = doc.tree.get(inflow.0).unwrap();
        assert_eq!(
            super::ifc_content_box_offset(&doc.tree, control),
            (24.0, 24.0),
            "the control really is an inline-block this IFC displaces"
        );
        let (cx, _) = untransformed_origin(&doc, inflow.0);
        assert_eq!(
            super::painted_border_box(&doc.tree, inflow.0, 1.0).x0 as f32,
            cx + 24.0,
            "the in-flow inline-block paints inside the padding and border"
        );

        // Why the fixed one takes none: `<button>` is inline-block by tag, but
        // out-of-flow blockifies it, so `ifc_content_box_offset` already
        // answers zero and the walk needs no branch of its own.
        let n = doc.tree.get(overlay.0).unwrap();
        assert_eq!(
            super::ifc_content_box_offset(&doc.tree, n),
            (0.0, 0.0),
            "an out-of-flow box takes no IFC content-box offset"
        );

        let r = super::painted_border_box(&doc.tree, overlay.0, 1.0);
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (30.0, 40.0, 90.0, 64.0));
        assert_ne!(
            (r.x0, r.y0),
            (54.0, 64.0),
            "the IFC root's padding+border must not displace a hoisted fixed box"
        );
    }

    /// The consumer this walk already had. `compute_dirty_region` narrows a
    /// software repaint to the changed nodes' rects, so a rect computed
    /// somewhere other than where the node paints leaves part of the real draw
    /// outside the region and unpainted — a defect that shows on a cold full
    /// repaint and not on a partial one, which is the shape #369 describes.
    ///
    /// The oracle is `painted_border_box` because that is now the same walk
    /// `paint_node` follows; the assertion is containment, which is the
    /// property the region has to have.
    #[test]
    fn the_dirty_region_covers_where_an_inline_block_paints() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let container = doc.create_element("div");
        doc.set_attribute(
            container,
            "style",
            "width: 400px; padding: 40px; border: 6px solid black; font-size: 16px",
        );
        doc.append_child(body, container);
        let before = doc.create_text("Press ");
        doc.append_child(container, before);
        let button = doc.create_element("button");
        doc.set_attribute(button, "style", "width: 60px; height: 24px");
        doc.append_child(container, button);
        doc.resolve_layout(800.0, 600.0);

        doc.tree.paint_dirty_nodes.clear();
        doc.tree.paint_dirty_nodes.push(button.0);
        let region =
            super::compute_dirty_region(&doc.tree, 1.0, 800.0, 600.0).expect("a dirty node");
        let painted = super::painted_border_box(&doc.tree, button.0, 1.0);

        // The 46px content-box offset is real, so the two candidate rects are
        // disjoint enough for containment to be a meaningful test.
        let (sum_x, sum_y) = untransformed_origin(&doc, button.0);
        assert_eq!(
            (painted.x0 as f32, painted.y0 as f32),
            (sum_x + 46.0, sum_y + 46.0)
        );

        assert!(
            region.x0 <= painted.x0
                && region.y0 <= painted.y0
                && region.x1 >= painted.x1
                && region.y1 >= painted.y1,
            "the dirty region {region:?} must cover the painted box {painted:?}"
        );
        // And the box it would have covered instead does not reach the draw:
        // 46px of offset against a 4px anti-aliasing margin.
        assert!(
            (sum_x as f64) + 60.0 + 4.0 < painted.x1,
            "the un-offset rect stops short of the painted box's right edge"
        );
    }

    /// The inverse direction, and why it is a separate helper. Under `scale(2)`
    /// a point 120 painted pixels right of the box's painted left edge is 60
    /// pixels into the box; subtracting the painted AABB's origin would answer
    /// 120 and pick the wrong character, the wrong scrollbar position, the wrong
    /// pixel of a render surface.
    #[test]
    fn a_point_maps_into_a_scaled_box_divided_not_merely_offset() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let zoom = child_of(
            &mut doc,
            body,
            "position: relative; width: 400px; height: 300px; \
             transform: scale(2); transform-origin: 0 0",
        );
        let field = child_of(
            &mut doc,
            rinch_core::dom::NodeId(zoom),
            "position: absolute; left: 30px; top: 20px; width: 200px; height: 40px",
        );
        doc.resolve_layout(800.0, 600.0);

        // Layout origin (30,20) → painted (60,40).
        let r = super::painted_border_box(&doc.tree, field, 1.0);
        assert_eq!((r.x0, r.y0), (60.0, 40.0));

        let local = super::point_in_painted_box(&doc.tree, field, 1.0, 180.0, 100.0)
            .expect("an invertible transform");
        assert_eq!(local, (60.0, 30.0), "(180-60)/2 and (100-40)/2");
        assert_ne!(
            local,
            (120.0, 60.0),
            "subtracting the painted origin without dividing is the wrong answer"
        );

        // And back out again.
        assert_eq!(
            super::point_from_painted_box(&doc.tree, field, 1.0, 60.0, 30.0),
            (180.0, 100.0),
            "the forward map is the inverse of the backward one"
        );
    }

    /// A `scale(0)` subtree paints to zero area, so no screen point corresponds
    /// to anything inside it — the same exit `hit_test`'s `local_point` takes.
    #[test]
    fn a_degenerate_transform_has_no_local_point() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let collapsed = child_of(
            &mut doc,
            body,
            "position: relative; width: 200px; height: 100px; transform: scale(0)",
        );
        let inner = child_of(
            &mut doc,
            rinch_core::dom::NodeId(collapsed),
            "width: 50px; height: 20px",
        );
        doc.resolve_layout(800.0, 600.0);

        assert_eq!(
            super::point_in_painted_box(&doc.tree, inner, 1.0, 10.0, 10.0),
            None
        );
    }

    /// A `position: fixed` box maps points against its viewport box, not against
    /// one displaced by a scrolled ancestor — the exception seven of the walks
    /// this PR converted never had.
    #[test]
    fn a_fixed_box_maps_points_against_its_viewport_origin() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let page = child_of(
            &mut doc,
            body,
            "position: absolute; left: 0; top: 0; width: 400px; height: 300px; overflow: auto",
        );
        child_of(
            &mut doc,
            rinch_core::dom::NodeId(page),
            "width: 100%; height: 2000px",
        );
        let overlay = child_of(
            &mut doc,
            rinch_core::dom::NodeId(page),
            "position: fixed; left: 40px; top: 60px; width: 120px; height: 50px",
        );
        doc.resolve_layout(800.0, 600.0);
        doc.tree.nodes[page].scroll_offset.1 = 150.0;

        assert_eq!(
            super::point_in_painted_box(&doc.tree, overlay, 1.0, 100.0, 80.0),
            Some((60.0, 20.0)),
            "measured from the viewport box (40,60)"
        );
        assert_ne!(
            super::point_in_painted_box(&doc.tree, overlay, 1.0, 100.0, 80.0),
            Some((60.0, 170.0)),
            "the page's scroll must not be added back in"
        );
    }
}
