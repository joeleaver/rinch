//! The questions paint asks of the walk besides a layer's bounds: where a
//! subtree paints, for damage; whether a clip cuts nothing; whether a subtree
//! lies entirely outside a region, for the cull; and an opacity layer's shape.

use peniko::kurbo::{Affine, Rect};

use super::{Extent, MAX_VISITS, UNBOUNDED, Walk, opacity_layer_extent};
use crate::node::{NodeTree, RawNodeId};
use crate::paint::painter::PaintShape;

/// Where `node_id`'s subtree paints, for a damage computation: `Some(rect)`,
/// `Some` of an empty rect when it provably paints nothing, `None` when the
/// walk cannot bound it.
///
/// [`opacity_layer_bounds`](super::opacity_layer_bounds) folds "nothing" into [`UNBOUNDED`], which is right
/// for a layer about to be drawn into and wrong for damage: a box-less block
/// whose only content was just hidden paints nothing, and reading that as
/// unbounded repainted the whole surface.
pub(in crate::paint) fn subtree_paint_extent(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    x: f64,
    y: f64,
) -> Option<Rect> {
    let node = tree.get(node_id)?;
    let offset_x = x - node.layout.x as f64 * scale;
    let offset_y = y - node.layout.y as f64 * scale;
    let mut walk = Walk {
        tree,
        scale,
        budget: MAX_VISITS,
        skip_root_clip: false,
        absolutes_escape_clips: false,
        skip_fixed: false,
        measuring: false,
        snap_gain: [0.0, 0.0],
        clipper_below_cb: false,
        clipper_below_fixed_cb: false,
    };
    match walk.node(node_id, offset_x, offset_y, Affine::IDENTITY, true, 0) {
        Extent::Nothing => Some(Rect::ZERO),
        Extent::Within(r) => Some(r),
        Extent::Unknown | Extent::Escapes => None,
    }
}

/// Would `node_id`'s clip bracket cut anything that is actually drawn?
///
/// `false` means "it might, or I could not tell", and is the answer this
/// returns for every kind of not-knowing — which is what makes it safe to elide
/// a clip on a `true`: a wrong `true` is a rendering bug, a wrong `false` costs
/// one clip layer that was already being pushed. Card K43.
///
/// `rect` is the clip's own rect in the same space [`opacity_layer_bounds`](super::opacity_layer_bounds)
/// answers in — physical pixels, before the node's CSS transform, i.e. exactly
/// the `Rect` `clip_shape` handed the caller.
///
/// # Why this is a walk and not four lines of arithmetic
///
/// "Does anything inside reach past the box" is the question this module
/// already answers, and answering it again separately is how the two drift.
/// The first draft of this optimisation did re-derive it, and got the one thing
/// wrong that a re-derivation always gets wrong: it compared an IFC root's text
/// against the **border-box** origin, while `paint_node` draws that text at the
/// **content** origin (`ifc_root_content_origin`, padding + border). Measured,
/// on a `width: 40px; padding-left: 100px; overflow: hidden` box holding a 75px
/// run: the clip was elided as having nothing to cut, and 268 ink pixels landed
/// outside it. The walk gets that right at both places it matters — the root's
/// own text and an inline-block an IFC positions — because it mirrors paint
/// rather than paraphrasing it, and it also brings the visit budget,
/// `MAX_DEPTH`, and the [`Extent::Escapes`] discipline for free.
///
/// # The trap, which is that this looks right and is vacuous without it
///
/// [`Walk::node`] narrows a clipping node's children by that node's own clip.
/// Asking [`opacity_layer_bounds`](super::opacity_layer_bounds) about a clipping box therefore returns that
/// box — every time, whatever is inside it — so the obvious spelling,
/// `opacity_layer_bounds(...) ⊆ rect`, is **true for every clipping box in the
/// document** and elides all of them. `skip_root_clip` is what suspends that
/// one intersection for the root, and only for the root. If this function ever
/// starts eliding everything, that flag is the first thing to check.
///
/// A sticky descendant's [`Extent::Unknown`] rides on the same point: it is
/// unnarrowed here on purpose, so it reaches the top as a not-knowing and
/// answers `false`, rather than being narrowed to the clip that is under
/// question and answering `true`.
pub(in crate::paint) fn clip_cuts_nothing(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    x: f64,
    y: f64,
    rect: Rect,
) -> bool {
    let Some(node) = tree.get(node_id) else {
        return false;
    };
    let offset_x = x - node.layout.x as f64 * scale;
    let offset_y = y - node.layout.y as f64 * scale;

    let mut walk = Walk {
        tree,
        scale,
        budget: MAX_VISITS,
        skip_root_clip: true,
        absolutes_escape_clips: false,
        skip_fixed: false,
        measuring: false,
        snap_gain: [0.0, 0.0],
        clipper_below_cb: false,
        clipper_below_fixed_cb: false,
    };
    match walk.node(node_id, offset_x, offset_y, Affine::IDENTITY, true, 0) {
        // Half a device pixel of slack, matching the tolerance every other
        // geometric comparison in paint carries: a box whose right edge is its
        // container's right edge must read as fitting, and layout arithmetic
        // does not land on the exact same float twice.
        Extent::Within(r) => {
            r.x0 >= rect.x0 - 0.5
                && r.y0 >= rect.y0 - 0.5
                && r.x1 <= rect.x1 + 0.5
                && r.y1 <= rect.y1 + 0.5
        }
        // Paint draws nothing at all in here, so there is nothing for the clip
        // to cut. Note this is *not* the same as `Within` of a zero-area rect,
        // which is why the enum has both.
        Extent::Nothing => true,
        // `Unknown` (a sticky descendant) and `Escapes` (a fixed one, the visit
        // budget, `MAX_DEPTH`) are both "I could not place it", and a clip is
        // never elided on a not-knowing.
        Extent::Unknown | Extent::Escapes => false,
    }
}

/// Is everything this subtree paints **certainly** outside the region
/// `region_hits` describes?
///
/// The off-window cull's question (#562), and it is deliberately not spelled as
/// a rect: only a `true` from this may be acted on, so the not-knowing states
/// have to be answered here rather than smuggled out as [`UNBOUNDED`] and
/// re-decided by a caller. `false` means "it might reach the region, or I could
/// not tell", which is the answer for [`Extent::Unknown`] (a sticky descendant
/// whose position comes from an ancestor walk), [`Extent::Escapes`] (a
/// `position: fixed` descendant, the visit budget, `MAX_DEPTH`) and for a root
/// this walk cannot find.
///
/// `transform` is the node's composed CSS transform, exactly as `paint_node`
/// hands it to the painter: this walk answers in the node's own untransformed
/// space (see [`opacity_layer_bounds`](super::opacity_layer_bounds)), and the region is in screen space.
///
/// # Why the cull cannot ask a syntactic question instead
///
/// `paint_node` used to decline the shortcut for every stacking context with
/// children, on the grounds that one may own a hoisted `position: fixed` entry
/// painted in viewport space — where its own box says nothing about whether it
/// is on screen (#561). That is correct and very expensive: an `opacity < 1`
/// stacking context parked below the fold then allocates, fills and composites
/// a whole-surface pixmap every frame for a picture nobody sees.
///
/// The narrowing proposed for it — decline only for a stacking context that
/// *also* clips — is **not** lossless, measured: the skip-draw-and-recurse arm
/// it hands the non-clipping ones to sits before every `push_layer` in
/// `paint_node`, so an off-window `opacity: 0.5` sheet's on-screen fixed
/// descendant came back at full strength instead of half. Asking *whether*
/// something is painted was never the question; asking what the subtree paints,
/// and where, is.
pub(in crate::paint) fn subtree_is_entirely_outside(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    x: f64,
    y: f64,
    transform: Affine,
    region_hits: impl Fn(Rect) -> bool,
) -> bool {
    let Some(node) = tree.get(node_id) else {
        return false;
    };
    let offset_x = x - node.layout.x as f64 * scale;
    let offset_y = y - node.layout.y as f64 * scale;

    let mut walk = Walk {
        tree,
        scale,
        budget: MAX_VISITS,
        skip_root_clip: false,
        absolutes_escape_clips: true,
        skip_fixed: false,
        measuring: false,
        snap_gain: [0.0, 0.0],
        clipper_below_cb: false,
        clipper_below_fixed_cb: false,
    };
    match walk.node(node_id, offset_x, offset_y, Affine::IDENTITY, true, 0) {
        Extent::Within(r) => !region_hits(transform.transform_rect_bbox(r)),
        // Paint draws nothing at all in here, so it certainly draws nothing in
        // the region. It is the honest arm and not a fall-through, and it is
        // unreachable from the one caller — but **not** because the walk unions
        // the root's own box onto every answer, which it does not: the four
        // early exits below (the tag list, `estimated_height`, `opacity <= 0.0`,
        // `display: none`) return `Nothing` above that union, and the zero-area
        // branch returns the children's extent with no root box on it at all.
        //
        // The reason is on the caller's side. `paint_node` makes those same four
        // refusals, and takes its own zero-area branch, *above* the cull — so a
        // node that would answer `Nothing` for any of those five reasons has
        // already returned and never reaches this call. Worth stating precisely
        // rather than plausibly: the wrong reason is the one a future editor
        // would re-check when adding a fifth exit to `Walk::node`, and it would
        // tell them the union covers it.
        Extent::Nothing => true,
        Extent::Unknown | Extent::Escapes => false,
    }
}

/// The bounds of the layer an element with `opacity` below 1 is painted in,
/// as the [`PaintShape`] the call sites push.
///
/// A subtree the walk proves paints **nothing** gets an empty rect at the
/// element's own origin, where [`opacity_layer_bounds`](super::opacity_layer_bounds) answers [`UNBOUNDED`]:
/// Vello allocates a layer's tiles over its whole bounds, so a dimmed element
/// that draws nothing would cost a window of tiles. Nothing is drawn into the
/// layer, so an empty clip cuts nothing away.
pub(in crate::paint) fn opacity_layer_shape(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    x: f64,
    y: f64,
) -> PaintShape {
    match opacity_layer_extent(tree, node_id, scale, x, y) {
        Extent::Nothing => Rect::new(x, y, x, y).into(),
        Extent::Within(r) if r.width() > 0.0 && r.height() > 0.0 => r.into(),
        _ => UNBOUNDED.into(),
    }
}
