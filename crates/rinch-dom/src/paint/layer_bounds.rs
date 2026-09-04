//! How large a layer has to be so that it does not cut off what it composites.
//!
//! An element with `opacity < 1` is composited through a group layer, and every
//! `push_layer` in this crate is handed a *bounds* shape along with the opacity.
//! The two painters read that shape very differently.
//! [`TinySkiaPainter::push_layer`](super::skia_painter::TinySkiaPainter) names
//! the parameter `_bounds` and never looks at it — its layer is a pixmap the
//! size of the whole surface, composited back whole — while
//! [`VelloPainter::push_layer`](super::vello_painter::VelloPainter) passes it
//! straight to `vello::Scene::push_layer`, which *clips* every command inside
//! the layer to it.
//!
//! Paint used to hand both of them the element's border box. That is a
//! reasonable thing to give a compositor as a hint and a wrong thing to give it
//! as a clip: CSS is explicit that a stacking context does not clip its
//! descendants, so a `box-shadow`, an overflowing absolutely-positioned child,
//! or a `transform` that carries a box outside its parent is drawn by the
//! software painter and silently thrown away by the Vello one. The two
//! renderers disagreed about the same document, and that disagreement is what
//! kept the GPU path from becoming the default. See card K36.
//!
//! The cheap fix — pass a rect so large it clips nothing, the way the
//! zero-area path in `paint/mod.rs` already does — makes the two agree by
//! giving Vello no information at all. What this module does instead is keep
//! the bounds *meaningful*: [`opacity_layer_bounds`] walks the subtree and
//! returns the union of what it will actually paint, so Vello's clip becomes an
//! optimisation hint rather than a lie, and the software painter — which
//! ignores it — is unaffected either way. "Rather than a lie" is the honest
//! strength of that claim: it holds absolutely for every layer root with a box,
//! and with one stated exception for a root collapsed to zero area. Both are
//! spelled out under "The one rule" below, and the exception is the first thing
//! to read if something in this module ever appears to cut content off.
//!
//! # The one rule
//!
//! **A rect that is too large costs a little GPU fill. A rect that is too small
//! is the bug this module exists to fix.** Every judgement below is therefore
//! made in the direction of a larger rect, and anything this walk cannot answer
//! with certainty answers [`Extent::Unknown`], which surfaces as [`UNBOUNDED`]
//! — the same ±1e7 rect the zero-area path has always used. It is never a
//! guess: it is either mirrored from what `paint_node` does, or it is unknown.
//!
//! A useful safety property falls out of that, and it is worth stating with its
//! exception rather than without, because the exception is where the next bug
//! will be. For a layer root with a **non-degenerate box**, the walk starts
//! from that box and only ever unions onto it — the intersection in
//! [`Walk::node`] applies to *descendants* of a clipping box, never to the
//! subtree root — so the result contains the border-box rect `paint_node` used
//! to pass, and for those roots this change cannot make a layer smaller than it
//! was.
//!
//! The **zero-area root is not covered by that**, and it is the one place this
//! change can clip something that was not clipped before. `paint_node`'s
//! `(width == 0) != (height == 0)` branch paints only children, so the walk's
//! matching branch returns the children's extent with no root box unioned onto
//! it — and that branch used to pass [`UNBOUNDED`] unconditionally, precisely
//! because a bounds that is wrong there blanks the subtree. A computed answer
//! is therefore a real trade, not a free tightening: everything paint draws
//! outside a descendant's layout box that this walk does not model is now
//! clipped under such a root. The known items are an on-demand text layout
//! wider than the box Taffy sized (the fallback arm in `paint_node`'s text arm,
//! #127), `paint_input_value`, which pushes no clip of its own at all, an
//! inline span's `background` padding reaching above its line box, and the
//! glyph ink noted below.
//!
//! It is kept rather than reverted because the alternative — answer
//! [`UNBOUNDED`] for every zero-area root and keep the absolute invariant —
//! throws away the branch's whole benefit to protect a case no one has been
//! able to construct an actual lost pixel for, and because the branch is a
//! container collapsed on one axis, which is rare and rarely holds an
//! unmodelled overhang. **If you are here because something inside a collapsed
//! container is being cut off on the GPU path, this paragraph is the reason and
//! returning [`UNBOUNDED`] at that branch is the fix.**
//!
//! # Mirroring, not re-deriving
//!
//! The walk's arithmetic is copied from `paint_node`'s, node for node: the same
//! `offset + layout.x * scale`, the same scroll subtraction, the same
//! [`compose_node_transform`], the same order of early returns. That is
//! deliberate. The question this module answers is not "where does CSS say this
//! box is" but "where will *this painter* put it", and the only way to be sure
//! of the second is to do the same sums. Where paint's placement is unusual —
//! an inline-block positioned by an inline formatting context, a `position:
//! fixed` box hoisted to the body — the walk reproduces the unusual thing
//! rather than the tidy one.

use peniko::kurbo::{Affine, Rect, Vec2};

use super::{compose_node_transform, painter::PaintShape};
use crate::computed_style::{DisplayValue, OverflowValue, PositionValue};
use crate::node::{DisplayMode, Node, NodeKind, NodeTree, RawNodeId};

/// A rect no clip can cut anything out of, at any scale a real window reaches.
///
/// This is the value `paint/mod.rs` has been passing for the zero-area layer
/// case since long before this module existed, and it stays the answer whenever
/// the walk below is not certain. tiny-skia ignores layer bounds entirely;
/// Vello clips to them, and clipping to this is clipping to nothing.
pub const UNBOUNDED: Rect = Rect::new(-1e7, -1e7, 1e7, 1e7);

/// How many nodes the walk will look at before it gives up and says
/// [`Extent::Unknown`].
///
/// This runs once per frame per translucent element, on a phone, in a frame
/// budget of 8.3ms — cards K42 and K43 spent a lot of effort getting this app
/// to 120fps on a moto g stylus 5G and this must not be where it goes back.
///
/// Measured on the developer laptop, in release, the walk costs about 20ns a
/// node and allocates nothing: a 181-node subtree of rows, cells and labels —
/// the shape of this app's library screen — comes to 3.4us. The cap is
/// therefore not there to make the common case fast; it is there so that no
/// document can make it slow. At 512 the worst case is roughly 10us of walk on
/// a laptop and well under a tenth of a millisecond on the phone, and past it
/// the answer is `UNBOUNDED` — which is exactly the bounds every one of these
/// layers effectively had before this module existed, so the fallback costs a
/// full-target clip and never a wrong picture.
///
/// There is deliberately no cache. A cached bounds would have to be invalidated
/// on every layout change, every style change, and every scroll of every
/// ancestor inside the subtree — and a bounds that is stale by one frame is a
/// clip that cuts off content, which is the bug this module was written to fix.
/// A correct cache is not cheap to prove, and 20ns a node does not need one.
const MAX_VISITS: u32 = 512;

/// How deep the walk will go. A subtree deeper than this is not worth
/// descending for a bounds hint; the same "give up conservatively" rule applies.
const MAX_DEPTH: u32 = 32;

/// What a subtree paints, as far as this walk can tell.
///
/// The three cases are distinct on purpose. `Nothing` is not `Within` a
/// zero-area rect: unioning a rect with a degenerate rect at the origin would
/// drag the result all the way to (0, 0), which is how a "conservative" bounds
/// function quietly becomes a full-screen one. And `Unknown` is not `Within`
/// [`UNBOUNDED`] either, because a `Unknown` subtree under an `overflow: hidden`
/// ancestor is still bounded by that ancestor's clip — see [`Extent::clipped_to`].
#[derive(Clone, Copy, Debug, PartialEq)]
enum Extent {
    /// Provably nothing is drawn.
    Nothing,
    /// Everything drawn lies inside this rect.
    Within(Rect),
    /// Not known. Treat as covering the plane.
    Unknown,
}

impl Extent {
    fn union(self, other: Extent) -> Extent {
        match (self, other) {
            (Extent::Unknown, _) | (_, Extent::Unknown) => Extent::Unknown,
            (Extent::Nothing, e) | (e, Extent::Nothing) => e,
            (Extent::Within(a), Extent::Within(b)) => Extent::Within(a.union(b)),
        }
    }

    /// What is left of this extent once it is clipped to `clip`.
    ///
    /// The `Unknown` arm is the interesting one and the reason this type has
    /// three cases: content whose extent could not be worked out is still
    /// bounded by an ancestor that clips it, so an `overflow: hidden` box
    /// containing something unanalysable contributes the box, not the plane.
    fn clipped_to(self, clip: Rect) -> Extent {
        match self {
            Extent::Nothing => Extent::Nothing,
            Extent::Unknown => Extent::Within(clip),
            Extent::Within(r) => {
                let hit = r.intersect(clip);
                // kurbo's `intersect` returns an inverted rect when the two do
                // not overlap; that is emptiness, not a rect to union with.
                if hit.width() > 0.0 && hit.height() > 0.0 {
                    Extent::Within(hit)
                } else {
                    Extent::Nothing
                }
            }
        }
    }
}

/// The bounds to hand `push_layer` for the layer opened around `node_id`.
///
/// `x`/`y` are the node's painted origin in physical pixels — the same `x`/`y`
/// `paint_node` computed for its own border-box rect, *after* any `position:
/// sticky` adjustment and *before* the node's own CSS transform, because the
/// transform is passed to the painter separately and applies to this shape too.
/// The returned rect is in that same space, so it is a drop-in replacement for
/// the `Rect::new(x, y, x + w, y + h)` that used to be passed.
///
/// The result contains the node's own border box in every case, and contains
/// every descendant this walk is certain about. When it is not certain it
/// returns [`UNBOUNDED`], which is what every one of these layers effectively
/// had before — a bounds that clips nothing.
pub fn opacity_layer_bounds(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    x: f64,
    y: f64,
) -> Rect {
    let Some(node) = tree.get(node_id) else {
        return UNBOUNDED;
    };
    // The walk re-derives every node's position as `offset + layout.x * scale`,
    // including the root's, so hand it the offset that reproduces the `x`/`y`
    // paint already computed. Going the other way — trusting the walk to
    // recompute the root's position — would lose the sticky adjustment paint
    // applied above the call site.
    let offset_x = x - node.layout.x as f64 * scale;
    let offset_y = y - node.layout.y as f64 * scale;

    let mut walk = Walk {
        tree,
        scale,
        budget: MAX_VISITS,
        root_is_body: node_id == tree.body_id,
    };
    match walk.node(node_id, offset_x, offset_y, Affine::IDENTITY, true, 0) {
        // A zero-area answer is not worth trusting even when it is arrived at
        // honestly: the layer is being pushed because paint is about to draw
        // something into it, and a degenerate clip would blank whatever that
        // is. The zero-area branch in `paint_node` reaches this with an element
        // that has no box of its own and no children, and `UNBOUNDED` is
        // exactly what that branch passed before this function existed.
        Extent::Within(r) if r.width() > 0.0 && r.height() > 0.0 => r,
        _ => UNBOUNDED,
    }
}

/// Convenience for the call sites, which all want a [`PaintShape`].
pub(super) fn opacity_layer_shape(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    x: f64,
    y: f64,
) -> PaintShape {
    opacity_layer_bounds(tree, node_id, scale, x, y).into()
}

struct Walk<'a> {
    tree: &'a NodeTree,
    scale: f64,
    /// Nodes left to look at before the walk gives up. Shared across the whole
    /// walk, not per level, so the cost of one call is bounded whatever shape
    /// the subtree has.
    budget: u32,
    /// Whether the layer being measured belongs to the body. `position: fixed`
    /// descendants are hoisted out of every other stacking context and painted
    /// at the body, so only the body's own layer has to account for them.
    root_is_body: bool,
}

impl Walk<'_> {
    /// The extent of everything `paint_node` would draw for this node and its
    /// subtree, in the layer's coordinate space.
    ///
    /// `is_root` marks the node the layer belongs to. It changes two things:
    /// the node's own CSS transform is *not* applied (the painter applies it to
    /// the bounds shape itself), and the escape hatches for boxes that paint
    /// somewhere other than where this walk would put them do not fire — the
    /// root is by definition being painted right here.
    fn node(
        &mut self,
        node_id: RawNodeId,
        offset_x: f64,
        offset_y: f64,
        parent_transform: Affine,
        is_root: bool,
        depth: u32,
    ) -> Extent {
        if self.budget == 0 || depth > MAX_DEPTH {
            return Extent::Unknown;
        }
        self.budget -= 1;

        let Some(node) = self.tree.get(node_id) else {
            return Extent::Nothing;
        };

        // The same refusals `paint_node` makes, in the same order. Anything it
        // declines to draw contributes nothing to the layer's extent — and the
        // `opacity <= 0.0` one is worth more than it looks, because it is the
        // always-mounted scrim from card K24: a full-screen element that is not
        // there, whose subtree this walk would otherwise measure every frame.
        if let NodeKind::Element(ref el) = node.kind
            && matches!(
                el.tag.as_str(),
                "style" | "script" | "head" | "meta" | "link"
            )
        {
            return Extent::Nothing;
        }
        if node.estimated_height.is_some() {
            return Extent::Nothing;
        }
        let cs = &node.computed_style;
        if cs.opacity <= 0.0 && cs.display != DisplayValue::Contents {
            return Extent::Nothing;
        }
        if cs.display == DisplayValue::None {
            return Extent::Nothing;
        }

        if !is_root {
            // A `position: fixed` box is viewport content that happens to live
            // in this markup. `stacking::collect_hoisted` drops it from every
            // sequence but the body's, and the body reaches past intervening
            // stacking contexts to collect it, so it is painted *outside* this
            // layer and must not widen it. The one layer that does own its
            // fixed descendants is the body's own, and that case — a translucent
            // `<body>` with fixed children — is rare enough to answer with
            // `Unknown` rather than to reproduce the offset-zeroing the body's
            // sequence does.
            if cs.position == PositionValue::Fixed {
                return if self.root_is_body {
                    Extent::Unknown
                } else {
                    Extent::Nothing
                };
            }
            // `position: sticky` is painted at a position `paint_node` derives
            // by walking *up* to the nearest scroll ancestor — which may well be
            // above the element this layer belongs to. The subtree alone does not
            // contain the answer, and duplicating that ancestor walk here would
            // create a second implementation of it to drift out of step with the
            // first. That drift is precisely the class of bug this module exists
            // to close, so a sticky descendant answers `Unknown`.
            if cs.position == PositionValue::Sticky {
                return Extent::Unknown;
            }
        }

        let layout = &node.layout;
        let scroll = Vec2::new(
            node.scroll_offset.0 * self.scale,
            node.scroll_offset.1 * self.scale,
        );

        if layout.width == 0.0 || layout.height == 0.0 {
            let x = offset_x + layout.x as f64 * self.scale;
            let y = offset_y + layout.y as f64 * self.scale;

            // `display: contents` has no box at all: its children are laid out
            // in the grandparent's space, so paint recurses with the offsets and
            // transform it was given, unchanged.
            if cs.display == DisplayValue::Contents {
                return self.children(node, offset_x, offset_y, parent_transform, depth);
            }

            // A box collapsed to zero in one dimension still keeps its origin
            // and its transform, and paint still walks into it (#142). A box
            // collapsed in *both* is one paint returns from — but the stacking
            // walk descends *through* it to hoist positioned descendants out,
            // and those are painted. Recursing into it costs a level and can
            // only enlarge the result, which is the right way to be wrong.
            //
            // Read that "only enlarge" narrowly: it is measured against *not*
            // recursing, not against what this branch used to return. When the
            // collapsed box is the layer root, `paint_node` passed `UNBOUNDED`
            // here, so any extent computed below — however carefully — is a
            // smaller shape than the one this branch shipped before, and it is
            // the only branch of which that is true. The module doc's safety
            // property is stated with this exception; see "The one rule".
            let transform = self.own_transform(node, x, y, parent_transform, is_root);
            return self.children(node, x - scroll.x, y - scroll.y, transform, depth);
        }

        let x = offset_x + layout.x as f64 * self.scale;
        let y = offset_y + layout.y as f64 * self.scale;
        let w = layout.width as f64 * self.scale;
        let h = layout.height as f64 * self.scale;
        let transform = self.own_transform(node, x, y, parent_transform, is_root);
        let rect = Rect::new(x, y, x + w, y + h);

        // What this node draws for itself, in its own untransformed space,
        // starting from the border box every arm of `paint_node` fills.
        let mut own = rect;

        // An outset `box-shadow` reaches `offset ± (blur + spread)` from the
        // border box. `paint_box_shadow` actually stops at `blur * 0.5 + spread`
        // — the empirical match to Chrome's visible extent — so the whole blur
        // radius is a deliberate half-blur of slack, cheap insurance against
        // that approximation being retuned outward later. Inset shadows are
        // painted inside the box, and `paint_box_shadow` skips them anyway.
        for shadow in &cs.box_shadow {
            if shadow.inset {
                continue;
            }
            let reach = (shadow.blur_radius.abs() + shadow.spread_radius.abs()) as f64 * self.scale;
            let dx = shadow.offset_x as f64 * self.scale;
            let dy = shadow.offset_y as f64 * self.scale;
            own = own.union(Rect::new(
                x + dx - reach,
                y + dy - reach,
                x + w + dx + reach,
                y + h + dy + reach,
            ));
        }

        // An outline is stroked outside the border box, centred on
        // `outline-offset` out, so it reaches `offset + width` — and a negative
        // `outline-offset` only pulls it inward, which the `max` keeps from
        // shrinking the rect below the border box.
        if cs.outline_width > 0.0 {
            let reach = (cs.outline_width + cs.outline_offset.max(0.0)) as f64 * self.scale;
            own = own.inset(reach);
        }

        // Inline content: the text this node lays out as an inline formatting
        // context root, drawn from its content-box origin. `InlineLayout::layout`
        // is built in CSS px and `render_text` re-applies `scale`, so its width
        // and height scale here too. A line that overflows its box — a long
        // unbreakable word — is measured by Parley and so is caught by this.
        //
        // What is *not* caught is glyph ink that reaches past its own line box:
        // an italic's overhang, a tall diacritic, a glyph whose ink exceeds its
        // advance. Parley will not hand that back cheaply — the only route is
        // per-glyph outline bounds, which is a per-frame cost this walk cannot
        // take on — so the layout box is what it uses. That is a slice of ink
        // the border-box bounds this function replaces was already cutting, and
        // the enclosing element boxes are in the union, so it is a pre-existing
        // approximation left where it was rather than a new one introduced here.
        let mut extent = Extent::Within(transform.transform_rect_bbox(own));
        if let Some(inline) = &node.text_layout {
            let (off_x, off_y) = super::ifc_root_content_origin(node);
            let content_x = x + off_x as f64 * self.scale - scroll.x;
            let content_y = y + off_y as f64 * self.scale - scroll.y;
            let text = Rect::new(
                content_x,
                content_y,
                content_x + inline.layout.width() as f64 * self.scale,
                content_y + inline.layout.height() as f64 * self.scale,
            );
            extent = extent.union(Extent::Within(
                transform.transform_rect_bbox(text_shadow_reach(text, node, self.scale)),
            ));
        }

        // A text node paints its cached Parley layout at its own origin. The
        // layout can be a little wider than the box Taffy sized for it (see the
        // on-demand fallback in `paint_node`'s text arm and #127), which is
        // exactly the sort of overhang the border box would have hidden.
        if let (NodeKind::Text(_), Some(cached)) = (&node.kind, &node.cached_text_parley) {
            let text = Rect::new(
                x,
                y,
                x + cached.width() as f64 * self.scale,
                y + cached.height() as f64 * self.scale,
            );
            // Text shadows on a text node come from its parent's style.
            let styled = node.parent.and_then(|p| self.tree.get(p)).unwrap_or(node);
            extent = extent.union(Extent::Within(
                transform.transform_rect_bbox(text_shadow_reach(text, styled, self.scale)),
            ));
        }

        let mut children = self.children(node, x - scroll.x, y - scroll.y, transform, depth);

        // Where the subtree is genuinely clipped, the bounds shrink. Note the
        // predicate: `paint_node` decides to clip on `overflow_y` alone and then
        // clips both axes with it, so a box with `overflow-x: hidden` and
        // `overflow-y: visible` is *not* clipped by this painter however much
        // CSS would like it to be, and this walk must not pretend otherwise.
        //
        // Paint also sometimes decides not to push a clip it is entitled to
        // (card K43: a clip that covers the render target, or one nothing
        // reaches past). Intersecting anyway stays correct in both of those
        // cases — the first only drops content that is off-window, the second
        // drops nothing at all.
        if matches!(
            cs.overflow_y,
            OverflowValue::Hidden | OverflowValue::Scroll | OverflowValue::Auto
        ) {
            children = children.clipped_to(transform.transform_rect_bbox(rect));
        }

        extent.union(children)
    }

    /// Every child `paint_node` would descend into, at the offsets it would use.
    fn children(
        &mut self,
        node: &Node,
        offset_x: f64,
        offset_y: f64,
        transform: Affine,
        depth: u32,
    ) -> Extent {
        let mut acc = Extent::Nothing;

        // An inline formatting context root paints its inline-block children
        // from the inline layout, at the *content*-box origin, and skips them in
        // its ordinary child walk (`already_drawn_inline`). Reproducing that
        // split matters for more than a padding's worth of offset: an
        // inline-block nested inside a `<span>` is positioned by the IFC that
        // owns it, not by the span, and the ordinary walk never reaches it at
        // all — the span itself is skipped as already drawn.
        let ifc_root = node.text_layout.is_some();

        for &child_id in &node.children {
            let Some(child) = self.tree.get(child_id) else {
                continue;
            };
            if ifc_root && child.ifc_root == Some(node.id) && !child.creates_stacking_context() {
                continue;
            }
            acc = acc.union(self.node(child_id, offset_x, offset_y, transform, false, depth + 1));
            if acc == Extent::Unknown {
                // Nothing below can make the answer narrower again — a caller
                // that clips will still clip it, and one that does not will get
                // `UNBOUNDED` whatever else is found.
                return acc;
            }
        }

        // The inline boxes, if this IFC has any — and most have none.
        //
        // The gate is worth its own paragraph, because walking a Parley layout
        // to find out is not cheap: `lines()` materialises a `Line` per line,
        // and on a 181-node subtree where 80 of the nodes were a div holding a
        // two-line label, doing it unconditionally was four fifths of the whole
        // walk (16.6us against 3.9us) to find no inline boxes at all. So ask
        // the cheap question first. `ifc.rs` pushes an inline box in exactly
        // one place, for a child whose `display_mode` is `InlineBlock`, and it
        // records every inline child it was told about in `child_positions` —
        // text runs and `<span>`s included. Scanning that list for an
        // inline-block is a handful of slab lookups, and it is `false` for
        // every IFC that is only text.
        let has_inline_boxes = |inline: &crate::node::InlineLayout| {
            inline.child_positions.iter().any(|(id, _)| {
                self.tree
                    .get(*id)
                    .is_none_or(|n| n.display_mode == DisplayMode::InlineBlock)
            })
        };
        if let Some(inline) = node.text_layout.as_ref().filter(|l| has_inline_boxes(l)) {
            let (off_x, off_y) = super::ifc_root_content_origin(node);
            let content_x = offset_x + off_x as f64 * self.scale;
            let content_y = offset_y + off_y as f64 * self.scale;
            for line in inline.layout.lines() {
                // Lines are charged to the same budget as nodes: a very long
                // article inside a translucent element is a walk like any other.
                if self.budget == 0 {
                    return Extent::Unknown;
                }
                self.budget -= 1;
                for item in line.items() {
                    let parley::layout::PositionedLayoutItem::InlineBox(positioned) = item else {
                        continue;
                    };
                    acc = acc.union(self.node(
                        positioned.id as RawNodeId,
                        content_x,
                        content_y,
                        transform,
                        false,
                        depth + 1,
                    ));
                    if acc == Extent::Unknown {
                        return acc;
                    }
                }
            }
        }

        acc
    }

    fn own_transform(
        &self,
        node: &Node,
        x: f64,
        y: f64,
        parent_transform: Affine,
        is_root: bool,
    ) -> Affine {
        if is_root {
            // The painter applies the layer's transform to the bounds shape, and
            // that transform already carries this node's own. Composing it again
            // here would rotate the rect twice.
            parent_transform
        } else {
            compose_node_transform(node, x, y, self.scale, parent_transform)
        }
    }
}

/// `rect` grown by however far `node`'s `text-shadow` carries its glyphs.
///
/// `render_text_with_shadow` offsets the shadow pass in *unscaled* layout units
/// while the main text is drawn scaled, so the reach is taken at whichever of
/// the two is larger; a shadow drawn nearer than the rect allows is not a
/// problem, one drawn further is. The blur radius is included even though this
/// painter draws shadow text without blurring it, because the day it does the
/// glyphs will spread by it.
fn text_shadow_reach(rect: Rect, node: &Node, scale: f64) -> Rect {
    let shadows = &node.computed_style.text_shadow;
    if shadows.is_empty() {
        return rect;
    }
    let unit = scale.max(1.0);
    let mut grown = rect;
    for shadow in shadows {
        let blur = shadow.blur_radius.abs() as f64 * unit;
        let dx = shadow.offset_x as f64 * unit;
        let dy = shadow.offset_y as f64 * unit;
        grown = grown.union(Rect::new(
            rect.x0 + dx - blur,
            rect.y0 + dy - blur,
            rect.x1 + dx + blur,
            rect.y1 + dy + blur,
        ));
    }
    grown
}
