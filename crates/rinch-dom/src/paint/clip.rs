//! One answer to "does this box clip, and to what shape".
//!
//! [`Node::clips_overflow`](crate::node::Node::clips_overflow) is the predicate
//! and [`clip_shape`] is the geometry, and everything that needs either asks
//! here: paint's clip bracket, its dirty-region subtree prune, the layer-bounds
//! walk, the hoisted entry's clip chain in [`crate::stacking`], and — across
//! the crate boundary — hit testing's `check_children` gate and `RinchApp`'s
//! two viewport clip walks.
//!
//! `creates_stacking_context` was on that list until #324 **stage B**, and is
//! deliberately not any more: clipping and stacking are separate questions now,
//! and the chain is what replaced the coupling. See
//! [`Node::creates_stacking_context`](crate::node::Node::creates_stacking_context).
//!
//! Those seven sites used to hold **four** different predicates (#324), which
//! was more than a tidiness complaint. Paint, `layer_bounds` and
//! `creates_stacking_context` matched `overflow_y` against
//! `Hidden | Scroll | Auto`; the dirty-region prune matched the same three on
//! either axis; hit testing and `viewport_clip_rect` asked `!= Visible` on
//! either axis; `viewport_rect_with_radius` asked `!= Visible` on `overflow_y`
//! alone. They part company exactly where one axis is `clip`, so an
//! `overflow: clip` box **clipped clicks and painted unclipped** — content
//! drawn and not clickable.
//!
//! ## Why both axes, and why `clip` is the case that reaches it
//!
//! CSS forces a `visible` to compute to `auto` when the other axis is neither
//! `visible` nor `clip` (css-overflow-3 §3), and Stylo's style adjuster does
//! implement that — measured, not assumed, and pinned by
//! `clip_predicate_tests::stylo_pairs_a_non_visible_axis_with_auto`. So for
//! `hidden`/`scroll`/`auto` the asymmetric case is unreachable and the two
//! spellings could never actually disagree.
//!
//! `clip` is the exception the spec carves out: `overflow-x: clip;
//! overflow-y: visible` is a legal computed pair and stays asymmetric. That is
//! the whole of the reachable defect, and it is why the shared predicate reads
//! both axes rather than mirroring the old `overflow_y`-only one.
//!
//! The same probe pins the other half of that rule, which narrows the surface
//! further than "`clip` can be asymmetric": Stylo also **rewrites `clip` to
//! `hidden`** when the other axis holds a scrolling value, so
//! `overflow-x: clip; overflow-y: auto` computes to `Hidden`/`Auto`. Between
//! the two rules, `Clip` reaches a `ComputedStyle` in exactly two shapes —
//! `Clip`/`Clip` and `Clip`/`Visible` — and never beside `Hidden`, `Scroll` or
//! `Auto` at all.
//!
//! ## The deviation this does not fix
//!
//! CSS clips per axis; rinch clips both axes with one rect. So an
//! `overflow-x: clip; overflow-y: visible` box clips vertically too, which CSS
//! would not — the direction hit testing has always erred in, now matched by
//! paint rather than contradicted by it. Tracked as #535 and pinned as the
//! known-wrong answer in `clip_predicate_tests`.

use peniko::kurbo::{Rect, RoundedRectRadii};

use crate::computed_style::LengthPercentageValue;
use crate::node::Node;

/// This box's `border-radius`, resolved and scaled to painter units.
///
/// The percentage basis is the shorter side of the border box, which is what
/// every corner of every rounded box in `paint_node` already resolved against.
pub fn border_radii(node: &Node, scale: f64) -> RoundedRectRadii {
    let cs = &node.computed_style;
    let basis = node.layout.width.min(node.layout.height);
    let r = |v: &LengthPercentageValue| v.resolve(basis).max(0.0) as f64 * scale;
    RoundedRectRadii::new(
        r(&cs.border_radius_top_left),
        r(&cs.border_radius_top_right),
        r(&cs.border_radius_bottom_right),
        r(&cs.border_radius_bottom_left),
    )
}

/// The shape `node` clips its content to, or `None` if it clips nothing.
///
/// `x`/`y` is the node's painted origin and `scale` the DPI scale, i.e. exactly
/// what `paint_node` holds when it opens the clip bracket.
///
/// The rect is the **border** box — the box paint has always clipped to, and
/// the one `layer_bounds` and the viewport hole punch intersect against, so
/// this preserves their agreement rather than choosing anew. Note that CSS
/// clips to the *padding* box (css-overflow-3 §3), so a clipping container with
/// a non-zero `border-width` lets its content paint over its own border. That
/// deviation predates this function and is #536, not something it introduces;
/// with no border the two boxes coincide, which is every clipping box in the
/// component library.
///
/// The radii come back alongside rather than baked in because the caller picks
/// the shape: `paint_node` pushes a `RoundedRect` when any corner is rounded
/// and a plain `Rect` otherwise, which is the branch it has always taken.
pub fn clip_shape(node: &Node, scale: f64, x: f64, y: f64) -> Option<(Rect, RoundedRectRadii)> {
    if !node.clips_overflow() {
        return None;
    }
    let w = node.layout.width as f64 * scale;
    let h = node.layout.height as f64 * scale;
    Some((Rect::new(x, y, x + w, y + h), border_radii(node, scale)))
}
