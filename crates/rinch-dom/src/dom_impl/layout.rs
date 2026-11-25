//! [`DomLayout`] for [`RinchDocument`]: layout results, node boxes and
//! text geometry.

use rinch_core::dom::{CaretAffinity, DomContent, DomLayout, DomTree, NodeId};

use super::RinchDocument;

mod offset;

impl DomLayout for RinchDocument {
    fn layout_size(&self, node: NodeId) -> Option<(f32, f32)> {
        let n = self.tree.nodes.get(node.0)?;
        Some((n.layout.width, n.layout.height))
    }

    /// The `offsetParent` chain from `node` to `container` and the offsets
    /// along it, from the layout (`dom_impl/layout/offset.rs`), so both hosts
    /// answer the same question the same way.
    fn layout_box_in(
        &self,
        node: NodeId,
        container: NodeId,
    ) -> Option<rinch_core::reactive::ElementBounds> {
        offset::layout_box_in(&self.tree, node.0, container.0)
    }

    /// The inline formatting context lays a `<br>` out as `"\n"`
    /// (`ifc.rs`), one byte of the flat offsets every text query here takes.
    fn line_break_flat_bytes(&self) -> usize {
        1
    }

    /// The inline formatting context lays a preserved tab out as one space
    /// widened to its tab stop (`ifc::TabStops`, #1122), one byte of the
    /// same flat offsets — as the DOM counts it. (It was four spaces, four
    /// bytes, until then: #1109.)
    fn tab_flat_bytes(&self) -> usize {
        1
    }

    /// U+2028, U+2029, U+0085 and U+000C as the inline formatting context
    /// lays them out in preserved text (`ifc::PRESERVED_SUBSTITUTES`, #1181).
    fn substituted_char_flat_bytes(&self) -> &'static [(char, usize)] {
        &crate::ifc::PRESERVED_SUBSTITUTE_FLAT_BYTES
    }

    /// The text ranges of `node_id`'s inline layout whose text node is a
    /// pseudo-element's (#891): a `::before`/`::after` box's only child is
    /// its text (`style_resolution/pseudo.rs`). A list marker is such a box
    /// too, and counts as a `::before`. And the bidi controls an element's
    /// `unicode-bidi` put around its content (#2588), which no text node owns
    /// either: an opening one as a `::before` (a caret there is drawn inside
    /// the element), a closing one as an `::after`.
    fn generated_flat_ranges(&self, node_id: u64) -> Vec<rinch_core::dom::GeneratedFlatRange> {
        let Some(il) = self
            .tree
            .nodes
            .get(node_id)
            .and_then(|n| n.text_layout.as_ref())
        else {
            return Vec::new();
        };
        let mut found: Vec<rinch_core::dom::GeneratedFlatRange> = il
            .bidi_control_ranges
            .iter()
            .map(|&(start, end, after)| rinch_core::dom::GeneratedFlatRange { start, end, after })
            .collect();
        for r in il.text_ranges.iter().filter(|r| !r.is_br) {
            let Some(pseudo) = self
                .tree
                .nodes
                .get_live(r.node_id)
                .and_then(|t| t.parent)
                .and_then(|p| self.tree.nodes.get_live(p))
                .filter(|p| p.is_pseudo_element)
            else {
                continue;
            };
            found.push(rinch_core::dom::GeneratedFlatRange {
                start: r.flat_start,
                end: r.flat_end,
                after: pseudo.is_generated_after,
            });
        }
        found.sort_by_key(|g| g.start);
        // One text node split over several style spans is several ranges,
        // end to end; so are controls beside each other.
        let mut out: Vec<rinch_core::dom::GeneratedFlatRange> = Vec::new();
        for g in found.into_iter().filter(|g| g.end > g.start) {
            match out.last_mut() {
                Some(last) if last.end == g.start && last.after == g.after => last.end = g.end,
                _ => out.push(g),
            }
        }
        out
    }

    /// Relative to the node's border box, as the trait says: Parley's
    /// layout-local caret moved by where that layout is drawn
    /// (`Self::text_layout_origin`, #1667).
    fn query_caret_position(&self, node_id: u64, byte_offset: usize) -> Option<(f32, f32)> {
        self.query_caret_position_with_affinity(node_id, byte_offset, CaretAffinity::Downstream)
    }

    fn query_caret_rect(&self, node_id: u64, byte_offset: usize) -> Option<(f32, f32, f32)> {
        self.query_caret_rect_with_affinity(node_id, byte_offset, CaretAffinity::Downstream)
    }

    /// From the same Parley layout [`Self::query_caret_position_with_affinity`]
    /// reads, moved to the node's border box the same way
    /// (`Self::text_layout_origin`).
    fn query_line_box(
        &self,
        node_id: u64,
        byte_offset: usize,
        affinity: CaretAffinity,
    ) -> Option<(f32, f32)> {
        let node = self.tree.nodes.get(node_id)?;
        let layout = node
            .text_layout
            .as_ref()
            .map(|l| &l.layout)
            .or(node.cached_text_parley.as_deref())
            .or_else(|| {
                node.children
                    .iter()
                    .filter_map(|&c| self.tree.nodes.get_live(c))
                    .find_map(|c| c.cached_text_parley.as_deref())
            })?;
        let (top, height) = crate::text_query::line_box_for_offset(layout, byte_offset, affinity)?;
        // The line as the text at `byte_offset` is drawn: moved by a relative
        // inline element (#1425), as the caret there is.
        let dy = node
            .text_layout
            .as_ref()
            .map_or(0.0, |l| l.relative_offset_at(byte_offset).1);
        let (_, oy) = self.text_layout_origin(node_id);
        Some((oy + top + dy, height))
    }

    /// What the next `resolve_layout` would act on: a style change, a box
    /// change, a structural change, or text whose inline layout was dropped.
    /// A paint-only change (`mark_paint_dirty`, a scroll offset) is not
    /// pending: it moves no line.
    /// The stamp of the text layout [`Self::query_line_box`] would read for
    /// `node_id` (its own, else a text child's), `0` with none.
    fn text_layout_generation(&self, node_id: u64) -> Option<u64> {
        let id = node_id;
        let node = self.tree.nodes.get(id)?;
        let holder = if node.text_layout.is_some() || node.cached_text_parley.is_some() {
            Some(id)
        } else {
            node.children.iter().copied().find(|&c| {
                self.tree
                    .nodes
                    .get(c)
                    .is_some_and(|c| c.cached_text_parley.is_some())
            })
        };
        Some(
            holder
                .and_then(|h| self.tree.text_layout_stamps.get(&h).copied())
                .unwrap_or(0),
        )
    }

    fn layout_pending(&self) -> bool {
        self.tree.styles_dirty
            || self.tree.layout_dirty
            || self.tree.ifc_dirty
            || !self.tree.dirty_ifc_text_roots.is_empty()
    }

    fn query_caret_position_with_affinity(
        &self,
        node_id: u64,
        byte_offset: usize,
        affinity: CaretAffinity,
    ) -> Option<(f32, f32)> {
        let (x, y) = crate::text_query::caret_position_for_offset_with_affinity(
            self,
            node_id,
            byte_offset,
            affinity,
        )?;
        let (ox, oy) = self.text_layout_origin(node_id);
        Some((ox + x, oy + y))
    }

    /// [`crate::text_query::caret_box_for_offset_with_affinity`], from the
    /// border box like every text query here.
    fn query_caret_box_with_affinity(
        &self,
        node_id: u64,
        byte_offset: usize,
        affinity: CaretAffinity,
    ) -> Option<(f32, f32, f32)> {
        let (x, y, h) = crate::text_query::caret_box_for_offset_with_affinity(
            self,
            node_id,
            byte_offset,
            affinity,
        )?;
        let (ox, oy) = self.text_layout_origin(node_id);
        Some((ox + x, oy + y, h))
    }

    /// From the IFC root's Parley layout: the box's positioned item is keyed by
    /// its node id (`ifc.rs` pushes `InlineBox { id: child_id }`).
    fn query_inline_box_caret(
        &self,
        node_id: u64,
        box_id: u64,
        after: bool,
    ) -> Option<(f32, f32, f32)> {
        let inline = self.tree.nodes.get(node_id)?.text_layout.as_ref()?;
        let (x, y, h) = crate::text_query::inline_box_caret(&inline.layout, box_id, after)?;
        // Beside the box where it is drawn: moved by the relative inline
        // elements around it (#1425).
        let (dx, dy) = inline.inline_box_relative_offset(box_id, |id| self.tree.nodes.get(id));
        let (ox, oy) = self.text_layout_origin(node_id);
        Some((ox + x + dx, oy + y + dy, h))
    }

    fn query_inline_box_caret_rect(
        &self,
        node_id: u64,
        box_id: u64,
        after: bool,
    ) -> Option<(f32, f32, f32)> {
        self.inline_box_caret_window_rect(node_id, box_id, after)
    }

    /// The element's edge box (`ifc::EDGE_BOX`, #1426), answered as an
    /// atomic inline's box is — and its generated content on that edge
    /// (#891): an `::after`'s text is laid out inside the element, before its
    /// end edge box, and a `::before`'s after its start edge box, so the
    /// element's own content stops at the generated text, and the edge runs
    /// from there out past the box. Its inner side (inside the element at its
    /// end, or at its start) is the caret at the generated text's inner end.
    /// Its outer side is the box's; with no box, the end edge's outer side
    /// is the inner caret — before the `::after` text, as Chrome draws the
    /// caret after a link with an arrow (#2035's review, F2) — and the start
    /// edge's is before the `::before` text. An end box on another line than
    /// the inner caret (the `::after` text wrapped) also answers the inner
    /// caret, so the caret for "after the link" stays on the link's line, as
    /// Chrome's does (F1).
    fn query_inline_edge_caret(
        &self,
        node_id: u64,
        element: u64,
        end: bool,
        after: bool,
    ) -> Option<(f32, f32, f32)> {
        let boxed =
            || self.query_inline_box_caret(node_id, crate::ifc::edge_box_id(element, end), after);
        let Some((start, stop)) = self.generated_edge_range(node_id, element, end) else {
            return boxed();
        };
        // The inner side: the leading side of the end edge, the trailing
        // side of the start edge.
        let inner = end != after;
        let caret_at = |byte: usize, affinity: CaretAffinity| {
            let (x, _) = self.query_caret_position_with_affinity(node_id, byte, affinity)?;
            let (top, height) = self.query_line_box(node_id, byte, affinity)?;
            Some((x, top, height))
        };
        match (inner, end) {
            // Inside at the end: after the element's own last character,
            // on its line.
            (true, true) => caret_at(start, CaretAffinity::Upstream),
            (true, false) => caret_at(stop, CaretAffinity::Downstream),
            (false, true) => {
                let inside = caret_at(start, CaretAffinity::Upstream);
                match (boxed(), inside) {
                    (Some(b), Some(i)) if (b.1 - i.1).abs() < 0.5 => Some(b),
                    (b, None) => b,
                    (_, i) => i,
                }
            }
            (false, false) => boxed().or_else(|| caret_at(start, CaretAffinity::Downstream)),
        }
    }

    fn query_inline_edge_caret_rect(
        &self,
        node_id: u64,
        element: u64,
        end: bool,
        after: bool,
    ) -> Option<(f32, f32, f32)> {
        let (x, y, h) = self.query_inline_edge_caret(node_id, element, end, after)?;
        self.caret_to_window(node_id, x, y, h)
    }

    fn query_caret_rect_with_affinity(
        &self,
        node_id: u64,
        byte_offset: usize,
        affinity: CaretAffinity,
    ) -> Option<(f32, f32, f32)> {
        let id = node_id;
        // The caret as the editor draws it, over its text's glyphs
        // (`query_caret_box_with_affinity`), in the window: a collapsed
        // `Range`'s rect, as on the web.
        if let Some((x, y, h)) = self.query_caret_box_with_affinity(id, byte_offset, affinity)
            && let Some(rect) = self.caret_to_window(id, x, y, h)
        {
            return Some(rect);
        }
        // No text geometry. For an element with no text at all (an empty
        // paragraph) that is its nature: the editor view paints the caret at the
        // block's content-box origin, one line tall (clamped the same way,
        // `RinchDomEditorView::empty_block_caret`). Text without geometry is
        // text not laid out yet (edited since the last layout), and has no
        // answer until the next one.
        if !self.text_content(NodeId(id)).unwrap_or_default().is_empty() {
            return None;
        }
        let node = self.tree.nodes.get(id)?;
        // A box 0 wide and 0 tall has not been laid out either: the empty
        // paragraph Enter just made is a fresh element whose origin is its
        // parent's until the next layout (the view's `empty_block_caret` makes
        // the same call).
        if node.layout.width == 0.0 && node.layout.height == 0.0 {
            return None;
        }
        // Its content box (#1667): a padded block's text would start there,
        // one line tall.
        let (l, t, _, b) = self.content_box_inset(node_id);
        let bh = self
            .line_box_height(node_id)
            .unwrap_or(node.layout.height - t - b);
        let h = if (4.0..=80.0).contains(&bh) { bh } else { 18.0 };
        let (l, t) = (l as f64, t as f64);
        let (x, y) = crate::paint::point_from_painted_box(&self.tree, id, 1.0, l, t);
        let (_, y2) = crate::paint::point_from_painted_box(&self.tree, id, 1.0, l, t + h as f64);
        Some((x as f32, y as f32, (y2 - y).abs() as f32))
    }

    fn query_selection_rects(
        &self,
        node_id: u64,
        byte_a: usize,
        byte_b: usize,
    ) -> Vec<(f32, f32, f32, f32)> {
        let Some(node) = self.tree.nodes.get(node_id) else {
            return Vec::new();
        };
        let layout = match (&node.text_layout, &node.cached_text_parley) {
            (Some(inline), _) => &inline.layout,
            (None, Some(layout)) => layout,
            (None, None) => return Vec::new(),
        };
        let (ox, oy) = self.text_layout_origin(node_id);
        // An IFC root's pieces are where its text is drawn: moved by a
        // relative inline element (#1425).
        let rects_of = |s: usize, e: usize| match &node.text_layout {
            Some(inline) => crate::text_query::selection_rects_for_inline(inline, s, e),
            None => crate::text_query::selection_rects_for_layout(layout, s, e),
        };
        let rects = rects_of(byte_a, byte_b);
        self.cut_generated(node_id, byte_a, byte_b, rects, rects_of)
            .into_iter()
            .map(|(x, y, w, h)| (ox + x, oy + y, w, h))
            .collect()
    }

    fn query_selection_rects_with_boxes(
        &self,
        node_id: u64,
        byte_a: usize,
        byte_b: usize,
        selected_boxes: &[u64],
    ) -> Vec<(f32, f32, f32, f32)> {
        let Some(node) = self.tree.nodes.get(node_id) else {
            return Vec::new();
        };
        let layout = match (&node.text_layout, &node.cached_text_parley) {
            (Some(inline), _) => &inline.layout,
            (None, Some(layout)) => layout,
            (None, None) => return Vec::new(),
        };
        let (ox, oy) = self.text_layout_origin(node_id);
        // Where the text and the boxes are drawn: moved by a relative inline
        // element (#1425).
        let get = |id| self.tree.nodes.get(id);
        let rects = match &node.text_layout {
            Some(inline) => crate::text_query::selection_rects_with_boxes_for_inline(
                inline,
                byte_a,
                byte_b,
                selected_boxes,
                get,
            ),
            None => crate::text_query::selection_rects_with_boxes(
                layout,
                byte_a,
                byte_b,
                selected_boxes,
            ),
        };
        let rects_of = |s: usize, e: usize| match &node.text_layout {
            Some(inline) => crate::text_query::selection_rects_for_inline(inline, s, e),
            None => crate::text_query::selection_rects_for_layout(layout, s, e),
        };
        self.cut_generated(node_id, byte_a, byte_b, rects, rects_of)
            .into_iter()
            .map(|(x, y, w, h)| (ox + x, oy + y, w, h))
            .collect()
    }

    fn query_glyph_bounds(
        &self,
        node_id: u64,
        byte_offset: usize,
    ) -> Option<rinch_core::dom::GlyphBounds> {
        use crate::text_query::glyph_bounds_for_offset;
        let bounds = glyph_bounds_for_offset(self, node_id, byte_offset)?;
        let (ox, oy) = self.text_layout_origin(node_id);
        Some(rinch_core::dom::GlyphBounds {
            x: ox + bounds.x,
            y: oy + bounds.y,
            width: bounds.width,
            height: bounds.height,
        })
    }

    fn resolve_layout(&mut self, width: f32, height: f32) {
        // Delegate to the existing implementation method
        RinchDocument::resolve_layout(self, width, height);
    }

    fn query_node_layout(&self, node_id: u64) -> Option<(f32, f32, f32, f32)> {
        let node = self.tree.nodes.get(node_id)?;
        Some((
            node.layout.x,
            node.layout.y,
            node.layout.width,
            node.layout.height,
        ))
    }

    /// The viewport of the last layout (issue #1764); `None` before the first.
    fn viewport_size(&self) -> Option<(f32, f32)> {
        let vp = self.tree.viewport;
        (vp.width > 0.0 && vp.height > 0.0).then_some((vp.width, vp.height))
    }

    /// The painted border box, as `ClickContext` and the bounds signals report
    /// it (`paint::painted_border_box` at scale 1), from the last layout
    /// (issue #1779); for a flowed inline element, the union of its line
    /// fragments (`RinchDocument::inline_element_rect`).
    fn bounding_rect(&mut self, node: NodeId) -> Option<rinch_core::reactive::ElementBounds> {
        use crate::computed_style::values::DisplayValue;
        let n = self.tree.get(node.0)?;
        if !n.is_element() || !self.is_connected(node) {
            return None;
        }
        // An element that generates no box: Chrome answers all zeros.
        let boxless = n.computed_style.display == DisplayValue::Contents
            || std::iter::successors(Some(node.0), |&id| self.tree.get(id)?.parent).any(|id| {
                self.tree
                    .get(id)
                    .is_some_and(|a| a.computed_style.display == DisplayValue::None)
            });
        if boxless {
            return Some(rinch_core::reactive::ElementBounds::default());
        }
        // A flowed inline element owns no box (its layout is a 0x0 ghost):
        // the union of its line fragments, as a browser answers (#1779).
        if n.is_flowed_inline_element()
            && let Some((x, y, w, h)) = self.inline_element_rect(node.0)
        {
            return Some(rinch_core::reactive::ElementBounds {
                x: x as f32,
                y: y as f32,
                width: w as f32,
                height: h as f32,
            });
        }
        let r = crate::paint::painted_border_box(&self.tree, node.0, 1.0);
        Some(rinch_core::reactive::ElementBounds {
            x: r.x0 as f32,
            y: r.y0 as f32,
            width: r.width() as f32,
            height: r.height() as f32,
        })
    }

    /// The node's left and top border widths, as Taffy laid them out
    /// (`taffy_conversion` hands it the computed widths). A child's
    /// [`LayoutResult`](crate::node::LayoutResult) origin is relative to its parent's
    /// border box, but an absolutely-positioned child's `left: 0; top: 0` sits
    /// at the parent's padding box, inside the border. The editor's caret and
    /// selection overlays are such children, placed from summed layout
    /// origins, so without this they were drawn one border width right of and
    /// below the glyphs (1px in the default editor style).
    fn content_origin_inset(&self, node_id: u64) -> (f32, f32) {
        let Some(node) = self.tree.nodes.get(node_id) else {
            return (0.0, 0.0);
        };
        let cs = &node.computed_style;
        (
            cs.border_left_width.to_px().max(0.0),
            cs.border_top_width.to_px().max(0.0),
        )
    }

    /// The computed `line-height` against the node's `font-size` (`normal`
    /// is rinch's 1.2, `ComputedStyle::line_height_px`).
    fn line_box_height(&self, node_id: u64) -> Option<f32> {
        let node = self.tree.nodes.get(node_id)?;
        Some(node.computed_style.line_height_px())
    }

    /// Padding plus border from the computed style, as Taffy laid the box
    /// out with them.
    fn content_box_inset(&self, node_id: u64) -> (f32, f32, f32, f32) {
        let Some(node) = self.tree.nodes.get(node_id) else {
            return (0.0, 0.0, 0.0, 0.0);
        };
        let cs = &node.computed_style;
        let side = |p: f32, b: f32| p.max(0.0) + b.max(0.0);
        (
            side(cs.padding_left.to_px(), cs.border_left_width.to_px()),
            side(cs.padding_top.to_px(), cs.border_top_width.to_px()),
            side(cs.padding_right.to_px(), cs.border_right_width.to_px()),
            side(cs.padding_bottom.to_px(), cs.border_bottom_width.to_px()),
        )
    }
}
