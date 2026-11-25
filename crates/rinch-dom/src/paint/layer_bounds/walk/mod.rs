//! [`Walk`], the descent that mirrors `paint_node` to find where a subtree
//! paints: its state and flags, and the helpers its descent uses. The descent
//! itself is in `node.rs`.

use peniko::kurbo::{Affine, Rect, Vec2};

use super::{Extent, MAX_VISITS};
use crate::node::{Node, NodeTree};
use crate::paint::compose_node_transform;

mod node;

pub(super) struct Walk<'a> {
    pub(super) tree: &'a NodeTree,
    pub(super) scale: f64,
    /// Nodes left to look at before the walk gives up. Shared across the whole
    /// walk, not per level, so the cost of one call is bounded whatever shape
    /// the subtree has.
    pub(super) budget: u32,
    /// Do not narrow the **root's** children by the root's own clip.
    ///
    /// Only [`clip_cuts_nothing`](super::clip_cuts_nothing) sets this, and it is the difference between
    /// that function working and being vacuous. See its doc comment.
    /// Descendants' clips always apply, in both callers.
    pub(super) skip_root_clip: bool,
    /// Answer [`Extent::Nothing`] for a viewport-`fixed` descendant instead of
    /// [`Extent::Escapes`]: for the damage of a moved box, whose fixed
    /// descendants stay where they are (#2222). A fixed box that changed is
    /// paint-dirty itself.
    pub(super) skip_fixed: bool,
    /// Measure a subtree reach (`PaintedState::subtree_reach`): only
    /// [`measure_subtree_reach`](super::measure_subtree_reach) sets it, right after the paint, when every
    /// recorded reach describes the tree as it is. The walk then answers a
    /// descendant box from the reach its last paint recorded instead of
    /// entering it, and measures every box and clip **unsnapped** (#2368),
    /// keeping in [`Self::snap_gain`] how far snapping can take them at any
    /// scale: the reach is read back at a scale and a position this walk
    /// does not know, where the device grid is somewhere else.
    pub(super) measuring: bool,
    /// While [`Self::measuring`]: the largest snap gain
    /// ([`crate::node::SubtreeReach::Within`]) of any box visited so far,
    /// `[0, 0]` while no transform has moved one.
    pub(super) snap_gain: [f64; 2],
    /// Answer [`Extent::Escapes`] for a `position: absolute` descendant that
    /// this walk would narrow by a clip paint does not apply to it — **#550's
    /// hole, closed for the one caller that cannot survive it**.
    ///
    /// Only [`subtree_is_entirely_outside`](super::subtree_is_entirely_outside) sets this, and the asymmetry is
    /// deliberate rather than an oversight. The two older callers hand their
    /// answer to `push_layer` as *bounds*, where an under-measure costs content
    /// on the Vello path only; the cull hands it to a `return`, where an
    /// under-measure costs content on **every** path. The direction of the
    /// error is the same in both — see **The one rule** — but the price is not,
    /// and turning this on for the bounds callers would enlarge every layer
    /// holding an absolute to [`UNBOUNDED`](super::UNBOUNDED). That is a GPU cost to weigh
    /// against #550's benefit, and it is not this flag's job to decide it.
    pub(super) absolutes_escape_clips: bool,
    /// Whether a clipping ancestor sits **strictly below** the nearest ancestor
    /// that establishes a containing block for absolutes — i.e. whether there
    /// is a clip in scope that an absolute found here would escape.
    ///
    /// Mirrors `crate::stacking::Collector`'s `cb_depth`, which is the
    /// authority: an absolute's clip chain is `live[..cb_depth]`, counted
    /// *after* the containing block's own clip is pushed, so that block's clip
    /// is in the chain and every clip inside it is not.
    pub(super) clipper_below_cb: bool,
    /// [`Self::clipper_below_cb`] for a `position: fixed` box whose
    /// containing block is an ancestor (#1372): cleared by a node that
    /// establishes a containing block for fixed boxes, not by a merely
    /// positioned one.
    pub(super) clipper_below_fixed_cb: bool,
}

impl<'a> Walk<'a> {
    /// A walk at `scale` with the given flags, measuring nothing.
    pub(super) fn new(
        tree: &'a NodeTree,
        scale: f64,
        skip_root_clip: bool,
        skip_fixed: bool,
    ) -> Self {
        Walk {
            tree,
            scale,
            budget: MAX_VISITS,
            skip_root_clip,
            absolutes_escape_clips: false,
            skip_fixed,
            measuring: false,
            snap_gain: [0.0, 0.0],
            clipper_below_cb: false,
            clipper_below_fixed_cb: false,
        }
    }

    /// While measuring: fold in a box whose own space reaches the root's
    /// through `transform`, with its own subtree's gain `inner` (`[0, 0]` for
    /// a box alone, or a subtree no transform moves). Through no transform
    /// the box is snapped on the root's grid, and the gain is its subtree's;
    /// through one it is snapped on its own, half a device pixel per edge at
    /// most, its subtree's transformed boxes `inner` times that.
    fn note_snap_gain(&mut self, transform: Affine, inner: [f64; 2]) {
        if !self.measuring {
            return;
        }
        let g = if transform == Affine::IDENTITY {
            inner
        } else {
            crate::paint::pixel_snap::snap_gain_through(transform, inner.map(|g| g.max(1.0)))
        };
        self.snap_gain = [self.snap_gain[0].max(g[0]), self.snap_gain[1].max(g[1])];
    }

    /// What an IFC root's own inline content draws. A root collapsed to no
    /// area draws it too (it overflows the box), so both arms of [`Self::node`]
    /// ask.
    fn inline_extent(
        &self,
        node: &crate::Node,
        x: f64,
        y: f64,
        scroll: Vec2,
        transform: Affine,
    ) -> Extent {
        let Some(inline) = &node.text_layout else {
            return Extent::Nothing;
        };
        let (off_x, off_y) = crate::paint::ifc_root_content_origin(node);
        let content_x = x + off_x as f64 * self.scale - scroll.x;
        let content_y = y + off_y as f64 * self.scale - scroll.y;
        let mut text = Rect::new(
            content_x,
            content_y,
            content_x + inline.painted_width() as f64 * self.scale,
            content_y + inline.layout.height() as f64 * self.scale,
        );
        // What it draws past those line boxes: text `vertical-align`
        // moves off its line, inline elements' backgrounds and borders
        // (which reach past the line by their padding and border, #1762)
        // and wavy underlines (#1817) — measured by the one function the
        // damage ink reads too, from the geometry paint draws with.
        // Before #1762 and #1817 a clip was elided whenever the unshifted
        // lines fitted, and the pill, the raised text or the wave painted
        // outside the box.
        if let Some(past) = crate::paint::text::inline_ink_past_lines(
            |id| self.tree.get(id),
            inline,
            self.scale,
            self.tree.inline_outlines,
        ) {
            text = text.union(past + Vec2::new(content_x, content_y));
        }
        // The shadows its inline elements declare are drawn with it (#1048).
        let mut reach = text_shadow_reach(text, &node.computed_style.text_shadow, self.scale);
        // A `-webkit-text-stroke` reaches past the glyphs (#2719): the
        // root's own, and its inline elements' where one may differ.
        let mut stroke = crate::paint::text_stroke_reach(&node.computed_style.text_paint);
        if self.tree.inline_text_shadows {
            for list in crate::paint::member_text_shadows(node, true, |id| self.tree.get(id)) {
                reach = reach.union(text_shadow_reach(text, list, self.scale));
            }
            stroke = inline
                .text_ranges
                .iter()
                .filter(|r| !r.is_br)
                .filter_map(|r| {
                    self.tree
                        .get(r.node_id)?
                        .parent
                        .and_then(|p| self.tree.get(p))
                })
                .map(|e| crate::paint::text_stroke_reach(&e.computed_style.text_paint))
                .fold(stroke, f32::max);
        }
        if stroke > 0.0 {
            let s = stroke as f64 * self.scale;
            reach = reach.union(text.inflate(s, s));
        }
        Extent::Within(transform.transform_rect_bbox(reach))
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

/// `rect` grown by however far the `text-shadow` list `shadows` carries its
/// glyphs.
///
/// `render_text_with_shadow` draws each shadow pass `offset × scale` physical
/// px from the scaled text, at the text's own size (#409), so the reach is the
/// offset plus [`REACH_PER_BLUR`](crate::paint::text_shadow::REACH_PER_BLUR) blur
/// radii — three standard deviations, where the blur stops (#980) — times
/// `scale`.
fn text_shadow_reach(
    rect: Rect,
    shadows: &[crate::computed_style::TextShadowValue],
    scale: f64,
) -> Rect {
    if shadows.is_empty() {
        return rect;
    }
    let mut grown = rect;
    for shadow in shadows {
        let blur =
            shadow.blur_radius.abs() as f64 * crate::paint::text_shadow::REACH_PER_BLUR * scale;
        let dx = shadow.offset_x as f64 * scale;
        let dy = shadow.offset_y as f64 * scale;
        grown = grown.union(Rect::new(
            rect.x0 + dx - blur,
            rect.y0 + dy - blur,
            rect.x1 + dx + blur,
            rect.y1 + dy + blur,
        ));
    }
    grown
}
