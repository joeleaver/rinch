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
//! ## "Does it clip" is not "is the layer worth pushing"
//!
//! A third question sits on top of these two and is answered somewhere else:
//! whether the clip this module describes is worth handing to the painter at
//! all. A clip whose rect contains the whole render target, or one nothing
//! inside reaches past, removes no pixel anybody sees, and a clip layer is not
//! free — Vello implements each as a blend-stack layer, two extra passes over
//! the clipped area, paid whether or not it cuts anything. `paint_node` may
//! therefore decline to push a bracket for a box that answers `clips_overflow`,
//! using `layer_bounds::clip_cuts_nothing` and the render target (card K43).
//!
//! **That does not weaken any of the answers here, and consumers must not
//! assume it does.** The predicate and the shape are unchanged; the elision is
//! a fact about one bracket on one painter's stack. Everything that reasons
//! about *where content ends up* — the hoisted entry's clip chain, hit testing,
//! `layer_bounds`' own intersection — keeps applying this clip, because the
//! elision only ever drops a restriction that was already vacuous. What it does
//! break is the reverse inference: "this node clips, therefore a bracket is
//! open" is no longer true, which is why
//! `paint_children_with_stacking` is *told* whether one is rather than deducing
//! it. Both cases require square corners: a rounded clip cuts the corners of
//! its own box, so a subtree fitting the box does not mean the shape cuts
//! nothing.
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
//! ## What never clips at all
//!
//! A `display: contents` element answers `false` (#1038): it generates no box,
//! so there is nothing to clip to, and Chrome 153 clips nothing. Its `layout` is
//! `0x0`, so the rect the shape derived from it hid every descendant a chain or
//! hit testing's gate carried it to.
//!
//! A non-atomic `display: inline` element answers `false` to the predicate
//! whatever its `overflow` computes to (#591 PR 1). `overflow` applies to block,
//! flex and grid containers (css-overflow-3 §3); an inline *box* is none of
//! those, while an `inline-block` is a block container and still clips. The
//! guard lives in [`Node::clips_overflow`](crate::node::Node::clips_overflow)
//! rather than in [`clip_shape`] for the reason this module exists: every site
//! that asks "does this clip" asks the predicate, and a guard on the shape alone
//! would leave hit testing's gate and the dirty-region prune disagreeing with
//! the chain. A *flowed* inline element also owns no box — its `layout` is
//! zeroed, see `Node::is_flowed_inline_element` — so the rect the shape derived
//! from it was a `0x0` at its parent's origin, which `stacking::Collector::descend`
//! pushed onto the chain of a positioned box hoisted out from under it whenever
//! that box's entry carries the live chain (a `relative` box; an `absolute`
//! whose containing block is the span itself — not one truncated at a
//! containing block above the span, and not a `fixed` box). The guard is
//! deliberately **wider** than that boxless set: a split inline and an unmarked
//! inline element — one no IFC has claimed, which therefore keeps a real Taffy
//! box — are inline boxes too, and `overflow` applies to neither. (The unmarked
//! case had one natural producer, the inner `<span>` of a re-measured
//! `inline-block`; #592 made an `inline-block` a block container, so that span
//! is now its IFC content and owns no box. The guard stays wide regardless —
//! see [`crate::node::Node::clips_overflow`].)
//!
//! ## The deviation this does not fix
//!
//! CSS clips per axis; rinch clips both axes with one rect. So an
//! `overflow-x: clip; overflow-y: visible` box clips vertically too, which CSS
//! would not — the direction hit testing has always erred in, now matched by
//! paint rather than contradicted by it. Tracked as #535 and pinned as the
//! known-wrong answer in `clip_predicate_tests`.
//!
//! ## The box: padding, not border (#536)
//!
//! CSS clips overflowing content to a box's **padding** box (css-overflow-3
//! §3: "the box's content is clipped to the box's padding edge"), and
//! [`clip_shape`] now does too — it used to clip to the border box, which let
//! a clipping container with a non-zero `border-width` paint its content over
//! its own border (border paints *before* the clip bracket opens, so nothing
//! else caught it). With no border the two boxes coincide, which is every
//! clipping box in the component library, so this was correctness debt rather
//! than a live visual bug until something actually set a border.
//!
//! [`padding_box_insets`] is the one place that measures the four border
//! widths for this; [`clip_shape`] scales and applies them, and so does every
//! cross-crate consumer the module doc above names — `layer_bounds`, the
//! viewport-clip walks in `RinchApp`, and hit testing's `check_children` gate,
//! which reads [`padding_box_insets`] directly rather than re-deriving it
//! (`crates/rinch/src/app/hit_testing.rs`). The radii shrink to match:
//! [`padding_box_radii`] reduces each of [`border_radii`]'s (outer) corners by
//! the two border widths that meet there. rinch's radii are one scalar per
//! corner rather than CSS's per-axis elliptical pair (see
//! [`peniko::kurbo::RoundedRectRadii`]), so each corner is reduced by the
//! *average* of its two adjacent widths — exact when the border is uniform
//! (the arc-per-side border painter in `paint/borders.rs` makes the same
//! choice, there called `half`), an approximation otherwise.

use peniko::kurbo::{Rect, RoundedRectRadii};

use crate::computed_style::LengthPercentageValue;
use crate::node::Node;

/// This box's **outer** `border-radius` — the border box's corners — resolved
/// and scaled to painter units.
///
/// The percentage basis is the shorter side of the border box, which is what
/// every corner of every rounded box in `paint_node` already resolved against.
/// This is the radii the background and border paint with; [`clip_shape`]'s
/// own (smaller, padding-box) radii come from [`padding_box_radii`].
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

/// How far the padding box sits inside the border box on each side, in
/// logical (unscaled) CSS px — the same unit as `node.layout` — as
/// `(left, top, right, bottom)`.
///
/// Clamped so the four widths can never claim more than the box itself has:
/// a `left + right` (or `top + bottom`) past the box's own width (or height)
/// is scaled back proportionally-by-side as a hard clamp (right/bottom give
/// up whatever left/top already claimed) rather than left to produce a
/// negative-size padding box. `border-width` takes no percentage in CSS, so
/// `to_px()` — the same resolution `paint/borders.rs` uses for the border
/// stroke itself — needs no basis.
///
/// Logical units so a caller already working in CSS px (hit testing, and
/// `RinchApp`'s viewport-clip walks, neither of which carries a DPI scale)
/// uses this as-is; [`clip_shape`] is the one caller that scales it.
///
/// Clamps against `node.layout`'s **current** border-box size. A caller
/// clipping against a *different* size for the same node — `clip_chain_bounds`
/// asks about a `Frame::Painted` ancestor's `prev_layout`, since the damage
/// chain can be built against either frame — wants that size's own clamp
/// instead, which [`padding_box_insets_for_size`] gives it.
pub fn padding_box_insets(node: &Node) -> (f32, f32, f32, f32) {
    padding_box_insets_for_size(node, node.layout.width, node.layout.height)
}

/// [`padding_box_insets`], clamped against an explicit `(width, height)`
/// rather than `node.layout`'s own — for a caller clipping against a border
/// box it is *not* reading from `node.layout` itself (a painted/previous
/// frame's size, say). The border widths still come from `node`'s **current**
/// computed style either way: there is no record of a painted border width to
/// ask for instead, the same approximation [`clip_shape`]'s radii already make
/// for every caller.
pub fn padding_box_insets_for_size(node: &Node, width: f32, height: f32) -> (f32, f32, f32, f32) {
    let cs = &node.computed_style;
    let w = width.max(0.0);
    let h = height.max(0.0);
    let left = cs.border_left_width.to_px().max(0.0).min(w);
    let right = cs
        .border_right_width
        .to_px()
        .max(0.0)
        .min((w - left).max(0.0));
    let top = cs.border_top_width.to_px().max(0.0).min(h);
    let bottom = cs
        .border_bottom_width
        .to_px()
        .max(0.0)
        .min((h - top).max(0.0));
    (left, top, right, bottom)
}

/// [`border_radii`] (the border box's corners) reduced to the padding box's —
/// each corner shrunk by the average of the two border widths that meet
/// there, clamped at zero. See the module doc's "The box: padding, not
/// border" section for why an average: rinch's radii are circular, one scalar
/// per corner, where CSS reduces each of two per-corner elliptical
/// components by a single adjacent side's width.
pub fn padding_box_radii(node: &Node, scale: f64) -> RoundedRectRadii {
    let outer = border_radii(node, scale);
    let (left, top, right, bottom) = padding_box_insets(node);
    let (left, top, right, bottom) = (
        left as f64 * scale,
        top as f64 * scale,
        right as f64 * scale,
        bottom as f64 * scale,
    );
    RoundedRectRadii::new(
        (outer.top_left - (top + left) * 0.5).max(0.0),
        (outer.top_right - (top + right) * 0.5).max(0.0),
        (outer.bottom_right - (bottom + right) * 0.5).max(0.0),
        (outer.bottom_left - (bottom + left) * 0.5).max(0.0),
    )
}

/// The shape `node` clips its content to, or `None` if it clips nothing.
///
/// `x`/`y` is the node's painted origin and `scale` the DPI scale, i.e. exactly
/// what `paint_node` holds when it opens the clip bracket.
///
/// The rect is the **padding** box (#536) — CSS clips overflowing content to
/// a box's padding edge (css-overflow-3 §3), inset from the border box by
/// [`padding_box_insets`] and scaled here; with no border the two boxes
/// coincide; see the module doc. `layer_bounds` and the viewport-clip walks in
/// `RinchApp` ask this same function (or, where they work in unscaled logical
/// px, [`padding_box_insets`]/[`padding_box_radii`] directly) so nothing can
/// drift back out of agreement with it.
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
    let (left, top, right, bottom) = padding_box_insets(node);
    let (left, top, right, bottom) = (
        left as f64 * scale,
        top as f64 * scale,
        right as f64 * scale,
        bottom as f64 * scale,
    );
    let ix = x + left;
    let iy = y + top;
    let iw = (w - left - right).max(0.0);
    let ih = (h - top - bottom).max(0.0);
    let radii = padding_box_radii(node, scale);
    Some((Rect::new(ix, iy, ix + iw, iy + ih), radii))
}
