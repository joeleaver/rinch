//! Text geometry in window coordinates, and the generated-text helpers
//! [`DomLayout`] for [`RinchDocument`] uses.

use rinch_core::dom::{CaretAffinity, DomLayout};

use crate::node::RawNodeId;

use super::RinchDocument;

impl RinchDocument {
    /// The text caret at `byte_offset` in the text-bearing element `node_id`, as
    /// `(x, y, height)` in logical window pixels: Parley's caret position and the
    /// glyph's line height, in the element's content box, pushed forward through
    /// the composed transform of its ancestors (#203), so a `scale()` stretches
    /// the height too. `None` when the element has no text layout: an empty
    /// paragraph (for which [`DomLayout::query_caret_rect`] answers the box),
    /// or text changed since the last layout.
    pub fn text_caret_window_rect(
        &self,
        node_id: RawNodeId,
        byte_offset: usize,
    ) -> Option<(f32, f32, f32)> {
        self.text_caret_window_rect_with_affinity(node_id, byte_offset, CaretAffinity::Downstream)
    }

    /// [`Self::text_caret_window_rect`] for the caret beside the inline box
    /// `box_id` (an `<img>`) in `node_id`'s inline layout — its left edge, or
    /// its right when `after` ([`DomLayout::query_inline_box_caret`], #1104).
    pub fn inline_box_caret_window_rect(
        &self,
        node_id: RawNodeId,
        box_id: RawNodeId,
        after: bool,
    ) -> Option<(f32, f32, f32)> {
        let (x, y, h) = self.query_inline_box_caret(node_id, box_id, after)?;
        self.caret_to_window(node_id, x, y, h)
    }

    /// [`Self::text_caret_window_rect`] for a caret with `affinity` at a soft
    /// wrap (#301): an `Upstream` caret there is at the end of the upper line.
    pub fn text_caret_window_rect_with_affinity(
        &self,
        node_id: RawNodeId,
        byte_offset: usize,
        affinity: CaretAffinity,
    ) -> Option<(f32, f32, f32)> {
        let (local_x, local_y) =
            self.query_caret_position_with_affinity(node_id, byte_offset, affinity)?;
        let height = self
            .query_glyph_bounds(node_id, byte_offset)
            .map(|g| g.height)
            .unwrap_or(18.0);
        self.caret_to_window(node_id, local_x, local_y, height)
    }

    /// The box of the glyph cluster at `byte_offset` in the text-bearing
    /// element `node_id`, as `(x, y, width, height)` in logical window pixels:
    /// [`DomLayout::query_glyph_bounds`] (border-box-local, so an IFC root's
    /// padding and border are in it, as paint draws the text) pushed forward
    /// through the composed transform of its ancestors (#203): the bounding box
    /// of its four corners, so a `scale()` stretches it and a `rotate()` holds
    /// the turned glyph. `None` where that query answers
    /// `None` (no text layout, an offset past the text).
    pub fn text_glyph_window_rect(
        &self,
        node_id: RawNodeId,
        byte_offset: usize,
    ) -> Option<(f32, f32, f32, f32)> {
        let g = self.query_glyph_bounds(node_id, byte_offset)?;
        let (x, y, w, h) = crate::paint::rect_from_painted_box(
            &self.tree,
            node_id,
            1.0,
            (g.x as f64, g.y as f64, g.width as f64, g.height as f64),
        );
        Some((x as f32, y as f32, w as f32, h as f32))
    }

    /// A caret `(x, y, height)` relative to `node_id`'s border box (what the
    /// `DomDocument` text queries answer), pushed through the composed
    /// transform of its ancestors into logical window pixels.
    pub(super) fn caret_to_window(
        &self,
        node_id: RawNodeId,
        local_x: f32,
        local_y: f32,
        height: f32,
    ) -> Option<(f32, f32, f32)> {
        self.tree.get(node_id)?;
        let fwd = |lx: f32, ly: f32| {
            crate::paint::point_from_painted_box(&self.tree, node_id, 1.0, lx as f64, ly as f64)
        };
        let (cx, cy) = fwd(local_x, local_y);
        let (_, cy2) = fwd(local_x, local_y + height);
        Some((cx as f32, cy as f32, (cy2 - cy).abs() as f32))
    }

    /// The flat bytes `start..end` of `element`'s generated text on its end
    /// edge (its `::after`) or its start edge (its `::before`) in the inline
    /// layout of `node_id` (#891). `None` when it has none there.
    pub(super) fn generated_edge_range(
        &self,
        node_id: RawNodeId,
        element: RawNodeId,
        end: bool,
    ) -> Option<(usize, usize)> {
        let el = self.tree.nodes.get(element)?;
        let pseudo = el.children.iter().copied().find(|&c| {
            self.tree
                .nodes
                .get_live(c)
                .is_some_and(|n| n.is_pseudo_element && n.is_generated_after == end)
        })?;
        let text = *self.tree.nodes.get_live(pseudo)?.children.first()?;
        let il = self.tree.nodes.get(node_id)?.text_layout.as_ref()?;
        let mut ranges = il.text_ranges.iter().filter(|r| r.node_id == text);
        let first = ranges.next()?;
        let stop = ranges.next_back().map_or(first.flat_end, |r| r.flat_end);
        Some((first.flat_start, stop))
    }

    /// `rects` (selection rects of `node_id` over `byte_a..byte_b`) less the
    /// generated text in that range (#891): a browser leaves a pseudo-
    /// element's text out of a selection's highlight. `holes` gives the
    /// rects of one generated stretch; each is cut out of the rects on its
    /// line.
    pub(super) fn cut_generated(
        &self,
        node_id: u64,
        byte_a: usize,
        byte_b: usize,
        mut rects: Vec<(f32, f32, f32, f32)>,
        holes: impl Fn(usize, usize) -> Vec<(f32, f32, f32, f32)>,
    ) -> Vec<(f32, f32, f32, f32)> {
        let (a, b) = (byte_a.min(byte_b), byte_a.max(byte_b));
        for g in self.generated_flat_ranges(node_id) {
            let (s, e) = (g.start.max(a), g.end.min(b));
            if s >= e {
                continue;
            }
            for (hx, hy, hw, _) in holes(s, e) {
                let (hl, hr) = (hx, hx + hw);
                rects = rects
                    .into_iter()
                    .flat_map(|(x, y, w, h)| {
                        let (l, r) = (x, x + w);
                        if (y - hy).abs() > 0.01 || hr <= l || r <= hl {
                            return vec![(x, y, w, h)];
                        }
                        [(l, hl.min(r)), (hr.max(l), r)]
                            .into_iter()
                            .filter(|(l, r)| r - l > 0.01)
                            .map(|(l, r)| (l, y, r - l, h))
                            .collect()
                    })
                    .collect();
            }
        }
        rects
    }

    /// Where the text layout the `DomDocument` text queries read for
    /// `node_id` is drawn, relative to `node_id`'s border box — the offset
    /// that turns Parley's layout-local answers into the border-box-local
    /// ones the trait promises (and rinch-web answers from
    /// `getBoundingClientRect`, #1667). The same precedence as
    /// [`crate::text_query::caret_position_for_offset_with_affinity`]:
    ///
    /// - an IFC root's inline layout is drawn at its content box, one padding
    ///   and border in ([`crate::paint::ifc_root_content_origin`], what paint
    ///   uses);
    /// - a text node's own measured layout at its own origin;
    /// - an element whose text is a measured child text node at that child's
    ///   box.
    pub(super) fn text_layout_origin(&self, node_id: RawNodeId) -> (f32, f32) {
        let Some(node) = self.tree.nodes.get(node_id) else {
            return (0.0, 0.0);
        };
        if node.text_layout.is_some() {
            return crate::paint::ifc_root_content_origin(node);
        }
        if node.cached_text_parley.is_some() {
            return (0.0, 0.0);
        }
        node.children
            .iter()
            .filter_map(|&c| self.tree.nodes.get_live(c))
            .find(|c| c.cached_text_parley.is_some())
            .map(|c| (c.layout.x, c.layout.y))
            .unwrap_or((0.0, 0.0))
    }
}
