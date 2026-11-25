//! [`DomLayout`] for [`WebDocument`]: layout results, node boxes and text
//! geometry, measured by the browser.

use super::*;

use rinch_core::dom::{CaretAffinity, DomLayout, GlyphBounds, NodeId};
use wasm_bindgen::JsCast;

impl DomLayout for WebDocument {
    fn layout_size(&self, node: NodeId) -> Option<(f32, f32)> {
        let el: &web_sys::HtmlElement = self.nodes.get(&node.0)?.dyn_ref()?;
        Some((el.offset_width() as f32, el.offset_height() as f32))
    }

    /// `offsetLeft`/`offsetTop` summed up the `offsetParent` chain to
    /// `container`, each `offsetParent` passed on the way adding its border
    /// (`clientLeft`/`clientTop`: an offset is from its parent's padding
    /// edge). Untransformed, where `getBoundingClientRect` (and so
    /// `query_node_layout`) is not.
    fn layout_box_in(
        &self,
        node: NodeId,
        container: NodeId,
    ) -> Option<rinch_core::reactive::ElementBounds> {
        let target: &web_sys::Element = self.nodes.get(&container.0)?.dyn_ref()?;
        let el: &web_sys::HtmlElement = self.nodes.get(&node.0)?.dyn_ref()?;
        let (width, height) = (el.offset_width() as f32, el.offset_height() as f32);
        let mut at = el.clone();
        let (mut x, mut y) = (0.0f32, 0.0f32);
        loop {
            x += at.offset_left() as f32;
            y += at.offset_top() as f32;
            let parent = at.offset_parent()?;
            if &parent == target {
                break;
            }
            x += parent.client_left() as f32;
            y += parent.client_top() as f32;
            at = parent.dyn_into().ok()?;
        }
        Some(rinch_core::reactive::ElementBounds {
            x,
            y,
            width,
            height,
        })
    }

    /// A `<br>`, an `<img>` or a node view's host
    /// (`WebDocument::br_caret_viewport_rect`).
    fn query_inline_box_caret(
        &self,
        node_id: u64,
        box_id: u64,
        after: bool,
    ) -> Option<(f32, f32, f32)> {
        let (x, y, h) = self.br_caret_viewport_rect(node_id, box_id, after)?;
        let block = self.nodes.get(&node_id)?;
        let r = block
            .dyn_ref::<web_sys::Element>()?
            .get_bounding_client_rect();
        Some((x - r.x() as f32, y - r.y() as f32, h))
    }

    fn query_inline_box_caret_rect(
        &self,
        node_id: u64,
        box_id: u64,
        after: bool,
    ) -> Option<(f32, f32, f32)> {
        self.br_caret_viewport_rect(node_id, box_id, after)
    }

    /// Downstream at a soft wrap, as on desktop — see
    /// [`Self::query_caret_position_with_affinity`].
    fn query_caret_position(&self, node_id: u64, byte_offset: usize) -> Option<(f32, f32)> {
        self.query_caret_position_with_affinity(node_id, byte_offset, CaretAffinity::Downstream)
    }

    fn query_caret_position_with_affinity(
        &self,
        node_id: u64,
        byte_offset: usize,
        affinity: CaretAffinity,
    ) -> Option<(f32, f32)> {
        let n = self.nodes.get(&node_id)?;
        let (x, y, _) = self.text_caret_viewport_rect(n, byte_offset, affinity)?;
        let el: web_sys::Element = n.clone().dyn_into().ok()?;
        let block_rect = el.get_bounding_client_rect();
        Some((x - block_rect.x() as f32, y - block_rect.y() as f32))
    }

    /// [`Self::query_line_boxes`] for one caret.
    fn query_line_box(
        &self,
        node_id: u64,
        byte_offset: usize,
        affinity: CaretAffinity,
    ) -> Option<(f32, f32)> {
        self.query_line_boxes(node_id, &[(byte_offset, affinity)])
            .pop()
            .flatten()
    }

    /// The browser exposes no line box, so it is rebuilt the way CSS builds
    /// it: the union of the boxes of what is on a caret's line. A piece of
    /// text contributes its inline box — its content area grown (or shrunk)
    /// to **its own element's** computed `line-height`, the smaller whole-px
    /// half of the leading above as Chrome splits it; under `line-height:
    /// normal`, the content area itself (so a line of mixed sizes there is
    /// within a pixel) — and a picture or a node view's host its own box.
    /// What is "on a caret's line" is every piece whose content area overlaps
    /// the line's caret rect by more than half the smaller height: a picture
    /// taller than the text, a `<sup>`, a `code` run, and no piece of a
    /// neighbouring line (#1677).
    ///
    /// The block is read **once** for all of `carets`: one walk of its text
    /// (each text node's string fetched once, each one's client rects once),
    /// then for each caret one range rect — the character after it, which is
    /// on the line a downstream caret is drawn on; the last character's end
    /// at the block's end. `affinity` is not read: a margin anchor's line is
    /// its downstream one.
    fn query_line_boxes(
        &self,
        node_id: u64,
        carets: &[(usize, CaretAffinity)],
    ) -> Vec<Option<(f32, f32)>> {
        let none = || vec![None; carets.len()];
        let Some(block) = self.nodes.get(&node_id) else {
            return none();
        };
        let Some(el) = block.dyn_ref::<web_sys::Element>() else {
            return none();
        };
        let Ok(range) = self.browser_doc.create_range() else {
            return none();
        };
        let window = web_sys::window();
        // A piece of the line: its content area, and its inline box.
        struct Piece {
            top: f32,
            bottom: f32,
            box_top: f32,
            box_bottom: f32,
        }
        let mut pieces: Vec<Piece> = Vec::new();
        // Text nodes in flat-byte order: (node, its text, its first flat byte).
        let mut texts: Vec<(web_sys::Node, String, usize)> = Vec::new();
        let mut line_heights: Vec<(web_sys::Element, Option<f32>)> = Vec::new();
        let mut flat = 0usize;
        let mut stack: Vec<web_sys::Node> = Vec::new();
        let kids = block.child_nodes();
        for i in (0..kids.length()).rev() {
            stack.extend(kids.item(i));
        }
        while let Some(node) = stack.pop() {
            if node.node_type() == web_sys::Node::TEXT_NODE {
                let text = node.text_content().unwrap_or_default();
                let lh = node.parent_element().and_then(|parent| {
                    if let Some((_, lh)) = line_heights.iter().find(|(e, _)| *e == parent) {
                        return *lh;
                    }
                    let lh = window
                        .as_ref()
                        .and_then(|w| w.get_computed_style(&parent).ok().flatten())
                        .and_then(|cs| cs.get_property_value("line-height").ok())
                        .and_then(|v| v.trim().strip_suffix("px")?.parse::<f32>().ok())
                        .filter(|h| *h > 0.0);
                    line_heights.push((parent, lh));
                    lh
                });
                if range.select_node_contents(&node).is_ok()
                    && let Some(rects) = range.get_client_rects()
                {
                    for j in 0..rects.length() {
                        let Some(r) = rects.item(j) else { continue };
                        let (t, b) = (r.top() as f32, r.bottom() as f32);
                        if b <= t {
                            continue;
                        }
                        let (box_top, box_bottom) = match lh {
                            Some(lh) => {
                                let half = (lh - (b - t)) / 2.0;
                                let above = if leading_floors() { half.floor() } else { half };
                                (t - above, t - above + lh)
                            }
                            None => (t, b),
                        };
                        pieces.push(Piece {
                            top: t,
                            bottom: b,
                            box_top,
                            box_bottom,
                        });
                    }
                }
                let len = text.len();
                texts.push((node, text, flat));
                flat += len;
                continue;
            }
            let Some(child) = node.dyn_ref::<web_sys::Element>() else {
                continue;
            };
            let tag = child.tag_name().to_ascii_lowercase();
            if tag == "br" {
                continue;
            }
            let atomic = matches!(
                tag.as_str(),
                "img" | "svg" | "video" | "canvas" | "iframe" | "input" | "button"
            ) || child.has_attribute(rinch_editor_view::NODE_VIEW_ATTR);
            if atomic {
                let r = child.get_bounding_client_rect();
                let (t, b) = (r.top() as f32, r.bottom() as f32);
                if b > t {
                    pieces.push(Piece {
                        top: t,
                        bottom: b,
                        box_top: t,
                        box_bottom: b,
                    });
                }
                continue;
            }
            let kids = node.child_nodes();
            for i in (0..kids.length()).rev() {
                stack.extend(kids.item(i));
            }
        }
        let block_top = el.get_bounding_client_rect().y() as f32;
        // Each caret's UTF-16 offset, found walking the text once in byte
        // order (from the start of its text node for each caret was quadratic
        // in a long paragraph's anchors).
        let mut order: Vec<usize> = (0..carets.len()).collect();
        order.sort_by_key(|&i| carets[i].0);
        let mut at: Vec<Option<(usize, u32, u32)>> = vec![None; carets.len()];
        let (mut ti, mut byte_in, mut utf16_in) = (0usize, 0usize, 0u32);
        for i in order {
            let byte = carets[i].0;
            // The text node holding the character that starts at `byte`.
            while ti < texts.len() && byte >= texts[ti].2 + texts[ti].1.len() {
                ti += 1;
                byte_in = 0;
                utf16_in = 0;
            }
            let (t, local) = match texts.get(ti) {
                Some((_, text, start)) => (ti, (byte - start).min(text.len())),
                None => match texts.last() {
                    Some((_, text, _)) => (texts.len() - 1, text.len()),
                    None => continue,
                },
            };
            if t != ti {
                // Past the end: the last node, from its start.
                let text = &texts[t].1;
                let off = utf8_byte_to_utf16_offset(text, local) as u32;
                at[i] = Some((t, off, off));
                continue;
            }
            let text = &texts[t].1;
            for ch in text[byte_in..].chars() {
                if byte_in >= local {
                    break;
                }
                byte_in += ch.len_utf8();
                utf16_in += ch.len_utf16() as u32;
            }
            let end = text[byte_in..]
                .chars()
                .next()
                .map_or(utf16_in, |ch| utf16_in + ch.len_utf16() as u32);
            at[i] = Some((t, utf16_in, end));
        }
        at.into_iter()
            .map(|at| {
                let (t, off, end) = at?;
                let node = &texts[t].0;
                range.set_start(node, off).ok()?;
                range.set_end(node, end).ok()?;
                let r = range.get_bounding_client_rect();
                let (cy, ch) = (r.top() as f32, r.height() as f32);
                if ch <= 0.0 {
                    return None;
                }
                let (mut top, mut bottom) = (f32::INFINITY, f32::NEG_INFINITY);
                for p in &pieces {
                    let overlap = p.bottom.min(cy + ch) - p.top.max(cy);
                    if overlap > 0.0 && overlap > 0.5 * (p.bottom - p.top).min(ch) {
                        top = top.min(p.box_top);
                        bottom = bottom.max(p.box_bottom);
                    }
                }
                top.is_finite().then_some((top - block_top, bottom - top))
            })
            .collect()
    }

    fn query_caret_rect_with_affinity(
        &self,
        node_id: u64,
        byte_offset: usize,
        affinity: CaretAffinity,
    ) -> Option<(f32, f32, f32)> {
        let block = self.nodes.get(&node_id)?;
        if let Some(rect) = self.text_caret_viewport_rect(block, byte_offset, affinity) {
            return Some(rect);
        }
        // No text node (an empty block): the block's content box, one line
        // high — where a padded block's text would start (#1667).
        let el = block.dyn_ref::<web_sys::Element>()?;
        let r = el.get_bounding_client_rect();
        let (l, t, _, b) = self.content_box_inset(node_id);
        let ch = self
            .line_box_height(node_id)
            .unwrap_or(r.height() as f32 - t - b);
        let h = if ch > 0.0 { ch } else { 18.0 };
        Some((r.x() as f32 + l, r.y() as f32 + t, h))
    }

    /// Downstream at a soft wrap, as on desktop — see
    /// [`Self::query_caret_rect_with_affinity`].
    fn query_caret_rect(&self, node_id: u64, byte_offset: usize) -> Option<(f32, f32, f32)> {
        self.query_caret_rect_with_affinity(node_id, byte_offset, CaretAffinity::Downstream)
    }

    fn query_glyph_bounds(&self, node_id: u64, byte_offset: usize) -> Option<GlyphBounds> {
        let n = self.nodes.get(&node_id)?;
        let (text_node, utf16_offset) = find_text_node_at_byte_offset(n, byte_offset)?;
        let text_content = text_node.text_content().unwrap_or_default();
        let text_utf16_len: usize = text_content.encode_utf16().count();

        let range = self.browser_doc.create_range().ok()?;
        // If we're at the end of text, use the last character's bounds
        if utf16_offset as usize >= text_utf16_len {
            if text_utf16_len == 0 {
                return None;
            }
            range
                .set_start(&text_node, (text_utf16_len - 1) as u32)
                .ok()?;
            range.set_end(&text_node, text_utf16_len as u32).ok()?;
        } else {
            range.set_start(&text_node, utf16_offset).ok()?;
            range.set_end(&text_node, utf16_offset + 1).ok()?;
        }

        let rect = range.get_bounding_client_rect();
        let el: web_sys::Element = n.clone().dyn_into().ok()?;
        let block_rect = el.get_bounding_client_rect();

        Some(GlyphBounds {
            x: (rect.x() - block_rect.x()) as f32,
            y: (rect.y() - block_rect.y()) as f32,
            width: rect.width() as f32,
            height: rect.height() as f32,
        })
    }

    fn resolve_layout(&mut self, _width: f32, _height: f32) {
        // No-op: browser handles layout natively.
    }

    /// Refresh `node`'s bounds signals from the browser from now on (issue
    /// #1781, `crate::bounds`). A text node has no box and is not watched.
    fn watch_bounds(&mut self, node: NodeId) {
        if let Some(el) = self
            .nodes
            .get(&node.0)
            .and_then(|n| n.dyn_ref::<web_sys::Element>())
        {
            crate::bounds::watch(self.doc_key, node.0, el);
        }
    }

    /// `documentElement.clientWidth/clientHeight` (issue #1764): the layout
    /// viewport less a classic scrollbar, which is the space a dropdown can be
    /// placed in without being hidden under one.
    fn viewport_size(&self) -> Option<(f32, f32)> {
        let el = web_sys::window()?.document()?.document_element()?;
        let (w, h) = (el.client_width(), el.client_height());
        (w > 0 && h > 0).then_some((w as f32, h as f32))
    }

    /// `getBoundingClientRect()` (issue #1779). A disconnected element answers
    /// `None`, as rinch-dom does; the browser gives a zero rect there.
    fn bounding_rect(&mut self, node: NodeId) -> Option<rinch_core::reactive::ElementBounds> {
        let n = self.nodes.get(&node.0)?;
        if !n.is_connected() {
            return None;
        }
        let el: &web_sys::Element = n.dyn_ref()?;
        let r = el.get_bounding_client_rect();
        Some(rinch_core::reactive::ElementBounds {
            x: r.x() as f32,
            y: r.y() as f32,
            width: r.width() as f32,
            height: r.height() as f32,
        })
    }

    fn query_node_layout(&self, node_id: u64) -> Option<(f32, f32, f32, f32)> {
        let n = self.nodes.get(&node_id)?;
        let el: web_sys::Element = n.clone().dyn_into().ok()?;
        let rect = el.get_bounding_client_rect();
        // The trait contract is **immediate-parent-relative** `(x, y)` (the desktop
        // Taffy backend returns this, and the editor's `block_offset_in_container`
        // sums it up the ancestor chain). `offsetLeft/offsetTop` would be
        // *offsetParent*-relative, which double-counts for a nested block (a
        // paragraph inside a blockquote/list, whose offsetParent is the editor
        // container, not its immediate parent). Use border-box differences instead.
        let (px, py) = match el.parent_element() {
            Some(parent) => {
                let pr = parent.get_bounding_client_rect();
                (pr.x(), pr.y())
            }
            None => (0.0, 0.0),
        };
        Some((
            (rect.x() - px) as f32,
            (rect.y() - py) as f32,
            rect.width() as f32,
            rect.height() as f32,
        ))
    }

    fn content_origin_inset(&self, node_id: u64) -> (f32, f32) {
        // clientLeft/clientTop = the node's left/top border width. The editor's overlay
        // container is position:relative with a border, so an absolutely-positioned
        // overlay anchors to its padding box; the block offset (computed from border-box
        // getBoundingClientRect differences) must subtract this to land on the glyphs.
        let Some(n) = self.nodes.get(&node_id) else {
            return (0.0, 0.0);
        };
        // Less its own scroll: an absolute child of a scrolled box is placed
        // against its *scrolled* content, so an editor container that is
        // itself the scroller (`overflow: auto` on the `Editor`) puts its
        // overlays at the unscrolled offset, where the desktop's parent-
        // relative layout already is (#1677).
        match n.clone().dyn_into::<web_sys::Element>() {
            Ok(el) => (
                el.client_left() as f32 - el.scroll_left() as f32,
                el.client_top() as f32 - el.scroll_top() as f32,
            ),
            Err(_) => (0.0, 0.0),
        }
    }

    /// From `getComputedStyle`: a px `line-height`; `None` for `normal`.
    fn line_box_height(&self, node_id: u64) -> Option<f32> {
        let el = self
            .nodes
            .get(&node_id)?
            .clone()
            .dyn_into::<web_sys::Element>()
            .ok()?;
        let cs = web_sys::window()?.get_computed_style(&el).ok().flatten()?;
        let v = cs.get_property_value("line-height").ok()?;
        v.trim()
            .strip_suffix("px")?
            .parse::<f32>()
            .ok()
            .filter(|h| *h > 0.0)
    }

    /// From `getComputedStyle`: the used padding and border widths, in px.
    fn content_box_inset(&self, node_id: u64) -> (f32, f32, f32, f32) {
        let zero = (0.0, 0.0, 0.0, 0.0);
        let Some(el) = self
            .nodes
            .get(&node_id)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
        else {
            return zero;
        };
        let Some(cs) = web_sys::window().and_then(|w| w.get_computed_style(&el).ok().flatten())
        else {
            return zero;
        };
        let px = |p: &str| -> f32 {
            cs.get_property_value(p)
                .ok()
                .and_then(|v| v.trim().trim_end_matches("px").parse::<f32>().ok())
                .unwrap_or(0.0)
                .max(0.0)
        };
        let side = |s: &str| px(&format!("padding-{s}")) + px(&format!("border-{s}-width"));
        (side("left"), side("top"), side("right"), side("bottom"))
    }

    fn node_font(&self, node_id: u64) -> Option<rinch_core::dom::NodeFont> {
        // `getComputedStyle` so the IME preedit overlay (an absolutely-positioned span
        // in the editor container, which inherits only the container's default font)
        // can match the block it composes into — e.g. a 32px heading, not 16px body.
        let el: web_sys::Element = self.nodes.get(&node_id)?.clone().dyn_into().ok()?;
        let cs = web_sys::window()?.get_computed_style(&el).ok().flatten()?;
        let get = |p: &str| cs.get_property_value(p).unwrap_or_default();
        Some(rinch_core::dom::NodeFont {
            family: get("font-family"),
            size: get("font-size"),
            weight: get("font-weight"),
            style: get("font-style"),
        })
    }

    /// The range starts at the last position at `byte_a` (past an unselected
    /// image there) or before the first selected box, whichever comes first,
    /// and ends at the first position at `byte_b` or after the last selected
    /// box, whichever comes last (#1120): an image has no bytes here either,
    /// so the byte range alone started a selection from right after an image
    /// at the end of the text before it, over the image, and drew an image
    /// selected alone not at all.
    fn query_selection_rects_with_boxes(
        &self,
        node_id: u64,
        byte_a: usize,
        byte_b: usize,
        selected_boxes: &[u64],
    ) -> Vec<(f32, f32, f32, f32)> {
        if byte_a >= byte_b && selected_boxes.is_empty() {
            return Vec::new();
        }
        let Some(n) = self.nodes.get(&node_id) else {
            return Vec::new();
        };
        let Some(range) = selection_range(
            &self.browser_doc,
            &self.nodes,
            n,
            byte_a,
            byte_b,
            selected_boxes,
        ) else {
            return Vec::new();
        };
        let Some(rects) = range.get_client_rects() else {
            return Vec::new();
        };
        self.block_local_rects(n, &rects)
    }

    fn query_selection_rects(
        &self,
        node_id: u64,
        byte_a: usize,
        byte_b: usize,
    ) -> Vec<(f32, f32, f32, f32)> {
        // Per-line selection rectangles for the flat UTF-8 byte range `[a, b)` within
        // the block's concatenated inline text, returned **block-local** (relative to
        // the block element's top-left) — the editor view adds the block's container
        // offset itself. Mirrors `query_caret_position`'s coordinate convention.
        if byte_a >= byte_b {
            return Vec::new();
        }
        let Some(n) = self.nodes.get(&node_id) else {
            return Vec::new();
        };
        let resolve = || -> Option<web_sys::DomRectList> {
            let (start_node, start_off) = find_text_node_at_byte_offset(n, byte_a)?;
            let (end_node, end_off) = find_text_node_at_byte_offset(n, byte_b)?;
            let range = self.browser_doc.create_range().ok()?;
            range.set_start(&start_node, start_off).ok()?;
            range.set_end(&end_node, end_off).ok()?;
            range.get_client_rects()
        };
        let Some(rects) = resolve() else {
            return Vec::new();
        };
        self.block_local_rects(n, &rects)
    }
}
