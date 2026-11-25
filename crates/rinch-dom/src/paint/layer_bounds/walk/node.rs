//! [`Walk::node`] and the child descent it drives: one node's extent, and
//! every child `paint_node` would descend into.

use peniko::kurbo::{Affine, Rect, Vec2};

use super::{Walk, text_shadow_reach};
use crate::computed_style::{DisplayValue, PositionValue};
use crate::node::{Node, NodeKind, RawNodeId};
use crate::paint::layer_bounds::{Extent, MAX_DEPTH, placed_subtree_reach};

impl Walk<'_> {
    /// The extent of everything `paint_node` would draw for this node and its
    /// subtree, in the layer's coordinate space.
    ///
    /// `is_root` marks the node the layer belongs to. It changes two things:
    /// the node's own CSS transform is *not* applied (the painter applies it to
    /// the bounds shape itself), and the escape hatches for boxes that paint
    /// somewhere other than where this walk would put them do not fire — the
    /// root is by definition being painted right here.
    pub(in crate::paint::layer_bounds) fn node(
        &mut self,
        node_id: RawNodeId,
        offset_x: f64,
        offset_y: f64,
        parent_transform: Affine,
        is_root: bool,
        depth: u32,
    ) -> Extent {
        // Giving up answers [`Extent::Escapes`], not `Unknown`, and the
        // distinction is the whole of #547 F7: "I did not look" includes "there
        // may be a box in here that escapes your clip". An `Unknown` is narrowed
        // by the first clipping ancestor on the way back up, so a fixed box past
        // the budget — or deeper than `MAX_DEPTH` — inside a clipper produced a
        // layer smaller than what paint draws, which is the F2 defect reached by
        // a second route.
        //
        // This restores what [`MAX_VISITS`]'s own documentation already promises
        // ("past it the answer is `UNBOUNDED` … so the fallback costs a
        // full-target clip and never a wrong picture") — that was simply untrue
        // whenever a clipping ancestor sat between here and the layer root.
        //
        // Worth keeping the shape of that in mind, because it generalises: the
        // promise was not deleted as over-claiming, it was **made true**, and
        // the history stayed here at the guard that fixed it rather than in the
        // promise. A doc comment that says something false is usually saying
        // what the code was meant to do, and the cheaper repair — softening the
        // claim — throws away the specification and leaves the defect. Check
        // which of the two is wrong before assuming it is the prose.
        //
        // It is also what lets [`Extent::is_final`] be the simple predicate it
        // is: this guard runs *before* every other arm, so once the budget is
        // gone every remaining sibling answers `Escapes` and the loop stops on
        // the next one.
        if self.budget == 0 || depth > MAX_DEPTH {
            return Extent::Escapes;
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
        //
        // **Why `Nothing` is safe here and `Escapes` is not needed — and why two
        // of these four are safe only by accident.** `Nothing` is the `union`
        // identity, so a subtree that answers it vanishes silently rather than
        // being narrowed: it is the *worse* failure of the two if it is ever
        // wrong. Each refusal has to mean "paint draws nothing here, hoisted
        // descendants included", and that is a stronger claim than "paint skips
        // this node" — `stacking::Collector::collect_hoisted` reaches a hoisted
        // entry directly, not through its parent, so a parent paint refuses does
        // not refuse what was hoisted out of it.
        //
        // - `display: none` and `opacity <= 0.0` are safe **by construction**.
        //   A `display: none` node is not in layout at all, so nothing under it
        //   is hoisted anywhere. `opacity < 1` creates a stacking context, so a
        //   fixed descendant is hoisted no further than this node (#545) and
        //   paint refuses the node itself — nothing escapes. Note what that
        //   rests on: if `Node::creates_stacking_context` ever stopped answering
        //   to `opacity`, this exit would start losing boxes silently.
        // - The tag list and `estimated_height` are the same asymmetry as F7 and
        //   are **latent rather than safe**: neither has a matching guard in
        //   `collect_hoisted`, so a hoisted descendant of one would be painted
        //   while this walk answered `Nothing`. Neither is reachable today — the
        //   tags get `display: none` from the UA stylesheet and would be caught
        //   by the exit below anyway, and a `position: fixed` box inside a
        //   virtualized editor block is not something the editor model can
        //   produce. If either ever becomes reachable, the answer is `Escapes`,
        //   for exactly the reason the give-up guard above returns it — and
        //   since #562 that is worth more than it was: a `Nothing` this walk
        //   should not have given used to shrink an opacity layer on the Vello
        //   path, and now also lets the off-window cull `return` on a subtree
        //   with something painted in it, on every backend.
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
            // A `position: fixed` box is painted *inside* this layer, so it does
            // widen it — but at coordinates this walk cannot produce. Since #545
            // a fixed box is hoisted only to its nearest ancestor stacking
            // context, and every layer root is one (`opacity < 1` and a
            // transform both create one), so a fixed descendant is an entry of
            // this layer's own sequence or of one nested inside it. Its
            // `layout.x`/`layout.y` are viewport coordinates, though, and this
            // walk accumulates offsets from the layer root — so the subtree
            // alone does not contain the answer and `Unknown` is the honest one,
            // exactly as for `sticky` below.
            //
            // It used to answer `Nothing` for a non-body root, on the grounds
            // that the body reached past every intervening context and painted
            // the box outside this layer. That stopped being true with #545.
            //
            // [`Extent::Escapes`] and not [`Extent::Unknown`], and the
            // difference is not cosmetic: an `Unknown` is narrowed by the first
            // clipping ancestor on the way back up, and a clipping ancestor
            // *inside* this layer does not clip a fixed box — its entry carries
            // an empty clip chain and paint lifts the collecting root's own
            // bracket around it. Collapsing to `Unknown` therefore returned a
            // layer smaller than what it paints, which tiny-skia ignores (it
            // never reads layer bounds) and Vello enforces (it clips
            // `push_layer` to them). That is a **backend divergence**, the
            // single failure this module exists to end, and no pixel assertion
            // against the software painter can see it — so
            // `a_fixed_descendant_is_not_narrowed_by_a_clipper_it_escapes`
            // asserts on the bounds themselves.
            if node.box_position() == PositionValue::Fixed {
                return if self.skip_fixed {
                    Extent::Nothing
                } else {
                    Extent::Escapes
                };
            }
            // `position: absolute` escapes the clippers **below its containing
            // block** — `Collector::span` truncates its chain at `cb_depth` —
            // while this walk narrows it at every clipping ancestor it descends
            // through. That is #550, named in the module doc as a known hole,
            // and it is the one hole a *cull* cannot live with: an under-measure
            // here does not shrink a layer, it deletes a box on every backend.
            //
            // The narrowing is only wrong when there is something to escape, so
            // the answer is conditioned on that rather than blanket: with no
            // clipper in scope below the containing block, the ordinary descent
            // places the box exactly where paint does and the extent is honest.
            // A sheet holding an absolutely positioned close button — the shape
            // this cull is for — therefore still prunes.
            //
            // A *partial* escape (clipped by the chain above the containing
            // block, not by the clips below it) is what would be exactly right,
            // and is what `Extent::Escapes` cannot express; this is the
            // conservative whole-escape instead.
            if self.absolutes_escape_clips
                && node.box_position() == PositionValue::Absolute
                && if node.is_contained_fixed() {
                    self.clipper_below_fixed_cb
                } else {
                    self.clipper_below_cb
                }
            {
                return Extent::Escapes;
            }
            // `position: sticky` is painted at a position `paint_node` derives
            // by walking *up* to the nearest scroll ancestor — which may well be
            // above the element this layer belongs to. The subtree alone does not
            // contain the answer, and duplicating that ancestor walk here would
            // create a second implementation of it to drift out of step with the
            // first. That drift is precisely the class of bug this module exists
            // to close, so a sticky descendant answers `Unknown`.
            if node.box_position() == PositionValue::Sticky {
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
                return self.scoped_children(
                    node,
                    offset_x,
                    offset_y,
                    parent_transform,
                    depth,
                    is_root,
                );
            }

            // A box collapsed to zero in one dimension still keeps its origin
            // and its transform, and paint still walks into it (#142); so does
            // a box collapsed in *both* since #1696 (a `0x0` stacking context's
            // descendants are entries of its own sequence, and an in-flow
            // child of a `0x0` box overflows it). The walk here does not apply
            // the box's own zero-area clip, so it can only enlarge the result,
            // which is the right way to be wrong.
            //
            // Read that "only enlarge" narrowly: it is measured against *not*
            // recursing, not against what this branch used to return. When the
            // collapsed box is the layer root, `paint_node` passed `UNBOUNDED`
            // here, so any extent computed below — however carefully — is a
            // smaller shape than the one this branch shipped before, and it is
            // the only branch of which that is true. The module doc's safety
            // property is stated with this exception; see "The one rule".
            //
            // `paint_node`'s zero-area arm draws the box's own inline content,
            // which `children` skips as drawn by the IFC.
            let transform = self.own_transform(node, x, y, parent_transform, is_root);
            // Its inline content's fragments are drawn snapped (#2607).
            self.note_snap_gain(transform, [0.0, 0.0]);
            let inline = self.inline_extent(node, x, y, scroll, transform);
            return inline.union(self.scoped_children(
                node,
                x - scroll.x,
                y - scroll.y,
                transform,
                depth,
                is_root,
            ));
        }

        let x = offset_x + layout.x as f64 * self.scale;
        let y = offset_y + layout.y as f64 * self.scale;
        let w = layout.width as f64 * self.scale;
        let h = layout.height as f64 * self.scale;
        let transform = self.own_transform(node, x, y, parent_transform, is_root);
        // A descendant whose subtree reach the last paint recorded is that
        // reach, placed at its box and through its own transform: what this
        // walk answered for it as a root then, and nothing under it has
        // changed since (`NodeTree::consume_paint_dirty` drops the reach of
        // every ancestor of a changed node).
        if self.measuring
            && !is_root
            && let Some(reach) = node.painted.as_ref().and_then(|p| p.subtree_reach)
        {
            return match reach {
                crate::node::SubtreeReach::Within { rect, snap_gain } => {
                    // Unsnapped, as this walk measures (#2368), its own
                    // snapped self left to the reach this one records; its
                    // gain, carried through the transform that places it.
                    self.note_snap_gain(transform, snap_gain.map(f64::from));
                    Extent::Within(transform.transform_rect_bbox(placed_subtree_reach(
                        rect,
                        [0.0, 0.0],
                        self.scale,
                        x,
                        y,
                    )))
                }
                crate::node::SubtreeReach::Unknown => Extent::Unknown,
                crate::node::SubtreeReach::Escapes => Extent::Escapes,
            };
        }
        // The border box and its snapped self (#2368): paint draws the box
        // on the device grid, up to half a device pixel past its layout box.
        // Measuring a reach, the box as laid out, and the gain that bounds
        // its snapped self at whatever scale the reach is read at.
        let rect = if self.measuring {
            self.note_snap_gain(transform, [0.0, 0.0]);
            Rect::new(x, y, x + w, y + h)
        } else {
            crate::paint::pixel_snap::box_reach(x, y, w, h)
        };

        // What this node draws for itself, in its own untransformed space,
        // starting from the border box every arm of `paint_node` fills.
        let mut own = rect;

        // An outset `box-shadow` reaches `offset ± (blur + spread)` from the
        // border box. `paint_box_shadow` actually stops at `blur * 0.5 + spread`
        // — the empirical match to Chrome's visible extent — so the whole blur
        // radius is a deliberate half-blur of slack, cheap insurance against
        // that approximation being retuned outward later. Inset shadows are
        // painted inside the box (`paint_inset_box_shadow` clips them to the
        // padding box), so they reach nothing past it.
        for shadow in &cs.box_shadow {
            if shadow.inset {
                continue;
            }
            let reach = (shadow.blur_radius.abs() + shadow.spread_radius.abs()) as f64 * self.scale;
            let dx = shadow.offset_x as f64 * self.scale;
            let dy = shadow.offset_y as f64 * self.scale;
            own = own.union(Rect::new(
                rect.x0 + dx - reach,
                rect.y0 + dy - reach,
                rect.x1 + dx + reach,
                rect.y1 + dy + reach,
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
        extent = extent.union(self.inline_extent(node, x, y, scroll, transform));

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
            let styled = node
                .parent
                .and_then(|p| self.tree.get_live(p))
                .unwrap_or(node);
            // And so does its stroke (#2719).
            let stroke = crate::paint::text_stroke_reach(&styled.computed_style.text_paint) as f64
                * self.scale;
            extent = extent.union(Extent::Within(
                transform.transform_rect_bbox(
                    text_shadow_reach(text, &styled.computed_style.text_shadow, self.scale)
                        .union(text.inflate(stroke, stroke)),
                ),
            ));
        }

        let mut children =
            self.scoped_children(node, x - scroll.x, y - scroll.y, transform, depth, is_root);

        // Where the subtree is genuinely clipped, the bounds shrink. The
        // predicate has to be the one `paint_node` opens its clip bracket with
        // and not an approximation of it, so it is literally that function —
        // `Node::clips_overflow`, shared since #324, where this walk used to
        // carry its own copy of paint's `overflow_y`-only spelling and a
        // paragraph explaining that it was deliberately mirroring a deviation.
        //
        // The rect is [`crate::paint::clip_shape`]'s own — the **padding** box
        // (#536), not `rect` above (the border box `own`'s box-shadow/outline
        // reach is measured against, which is unaffected: neither is clipped
        // by this bracket, since both paint before it opens). Square, radii
        // dropped: a rounded clip is inside its own square, so intersecting
        // with the square is the conservative answer and the one an *extent*
        // wants.
        //
        // Paint also sometimes decides not to push a clip it is entitled to
        // (card K43: a clip that covers the render target, or one nothing
        // reaches past). Intersecting anyway stays correct in both of those
        // cases — the first only drops content that is off-window, the second
        // drops nothing at all.
        //
        // `skip_root_clip` is the one exception, and it applies to the **root
        // only**: [`clip_cuts_nothing`] asks what the root's clip would have to
        // cut, so narrowing by that very clip first would answer "nothing" for
        // every clipping box in the document.
        //
        // Nor is the root narrowed by its own clip when an absolute box inside
        // escapes it (#2815): one whose containing block is above the root,
        // which paint lifts the root's bracket around. The whole subtree is
        // then left unnarrowed there — wider than exact, never narrower.
        // Measuring a reach, the clip as laid out: the box's gain, noted
        // above, bounds its snapped self as it bounds the box's.
        let clip = if self.measuring {
            crate::paint::clip::unsnapped_clip_shape(node, self.scale, x, y)
        } else {
            crate::paint::clip_shape(node, self.scale, x, y)
        };
        if let Some((clip_rect, _)) = clip
            && !(is_root
                && (self.skip_root_clip
                    || crate::paint::holds_escaping_absolute(self.tree, node_id)))
        {
            children = children.clipped_to(transform.transform_rect_bbox(clip_rect));
        }

        let extent = extent.union(children);
        // A `filter` that moves pixels spreads everything the box and its
        // stacking context draw by its reach (#2719), through the transform.
        if !cs.filter.is_empty()
            && cs.has_raster_filter()
            && let Extent::Within(r) = extent
        {
            let [l, t, rr, b] = crate::paint::filter::list_outsets(&cs.filter);
            let c = transform.as_coeffs();
            let k = c[0].hypot(c[1]).max(c[2].hypot(c[3])) * self.scale;
            return Extent::Within(Rect::new(
                r.x0 - l as f64 * k,
                r.y0 - t as f64 * k,
                r.x1 + rr as f64 * k,
                r.y1 + b as f64 * k,
            ));
        }
        extent
    }

    /// [`Self::children`] with [`Self::clipper_below_cb`] updated for this node
    /// and restored afterwards — the walk's half of `Collector::descend`.
    ///
    /// Three answers, and the order of the arms is the whole of it:
    ///
    /// - **The root: `false`, unless an absolute inside escapes a clip.** The
    ///   root of this walk is the stacking context that collects the sequence
    ///   its absolutes are entries of. An absolute whose containing block is
    ///   above the root escapes the root's own clip and the clips paint has
    ///   open around the root that belong to that block's descendants: paint
    ///   lifts them around its entry (#2815). So the flag starts `true` when
    ///   the root is not that block, holds such an absolute
    ///   (`holds_escaping_absolute`) and either clips itself or has such a
    ///   clip open around it (`open_clips_an_absolute_escapes`).
    /// - **A node that establishes a containing block: `false`.** `cb_depth` is
    ///   counted *after* that node's own clip is pushed, so its clip is in the
    ///   chain and nothing is escaped yet.
    /// - **A node that clips and does not: `true`**, and it stays true for the
    ///   rest of the descent until a containing block clears it.
    fn scoped_children(
        &mut self,
        node: &Node,
        offset_x: f64,
        offset_y: f64,
        transform: Affine,
        depth: u32,
        is_root: bool,
    ) -> Extent {
        let outer = self.clipper_below_cb;
        let outer_fixed = self.clipper_below_fixed_cb;
        // The root's own clip, and the clips paint has open around it, are
        // escaped by an absolute inside whose containing block is above the
        // root (#2815). Asked of the cull only, the one caller that reads
        // these flags, and only once an absolute is known to be there.
        let root_escaped = is_root
            && self.absolutes_escape_clips
            && crate::paint::holds_escaping_absolute(self.tree, node.id)
            && (node.clips_overflow()
                || crate::paint::open_clips_an_absolute_escapes(self.tree, node.id));
        self.clipper_below_cb = if is_root {
            root_escaped && !node.establishes_abs_containing_block()
        } else if node.establishes_abs_containing_block() {
            false
        } else {
            outer || node.clips_overflow()
        };
        self.clipper_below_fixed_cb = if is_root {
            root_escaped && !node.establishes_fixed_containing_block()
        } else if node.establishes_fixed_containing_block() {
            false
        } else {
            outer_fixed || node.clips_overflow()
        };
        let extent = self.children(node, offset_x, offset_y, transform, depth);
        self.clipper_below_cb = outer;
        self.clipper_below_fixed_cb = outer_fixed;
        extent
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

        // The box tree, not the element tree (#566).
        for &child_id in crate::RinchDocument::box_tree_children(&self.tree.nodes, node.id).iter() {
            let Some(child) = self.tree.get_live(child_id) else {
                continue;
            };
            if ifc_root && child.ifc_root == Some(node.id) && !child.creates_stacking_context() {
                continue;
            }
            acc = acc.union(self.node(child_id, offset_x, offset_y, transform, false, depth + 1));
            // Only `Escapes` may stop the loop — see [`Extent::is_final`].
            if acc.is_final() {
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
        // one place, for a child that is an **atomic inline**
        // (`DisplayMode::is_atomic_inline` — `inline-block`, `inline-flex` or
        // `inline-grid`; #595, #607), and it
        // records every inline child it was told about in `child_positions` —
        // text runs and `<span>`s included. Scanning that list for an
        // atomic inline is a handful of slab lookups, and it is `false` for
        // every IFC that is only text.
        let has_inline_boxes = |inline: &crate::node::InlineLayout| {
            inline.child_positions.iter().any(|(id, _)| {
                self.tree
                    .get(*id)
                    .is_none_or(|n| n.display_mode.is_atomic_inline())
            })
        };
        if let Some(inline) = node.text_layout.as_ref().filter(|l| has_inline_boxes(l)) {
            let (off_x, off_y) = crate::paint::ifc_root_content_origin(node);
            let content_x = offset_x + off_x as f64 * self.scale;
            let content_y = offset_y + off_y as f64 * self.scale;
            for line in inline.layout.lines() {
                // Lines are charged to the same budget as nodes: a very long
                // article inside a translucent element is a walk like any other.
                //
                // `Escapes` for the same reason as the guard at the top of
                // `node` — this is the same give-up, and the lines not looked at
                // may hold an inline-block whose subtree holds a fixed box. (Not
                // a fixed box *directly*: Stylo blockifies an out-of-flow box,
                // so one is never an inline item. The route is one level down,
                // and `an_unvisited_inline_line_may_hide_a_fixed_box` walks it.)
                if self.budget == 0 {
                    return Extent::Escapes;
                }
                self.budget -= 1;
                for item in line.items() {
                    let parley::layout::PositionedLayoutItem::InlineBox(positioned) = item else {
                        continue;
                    };
                    if crate::ifc::is_edge_box(positioned.id) {
                        continue;
                    }
                    acc = acc.union(self.node(
                        positioned.id as RawNodeId,
                        content_x,
                        content_y,
                        transform,
                        false,
                        depth + 1,
                    ));
                    if acc.is_final() {
                        return acc;
                    }
                }
            }
        }

        acc
    }
}
