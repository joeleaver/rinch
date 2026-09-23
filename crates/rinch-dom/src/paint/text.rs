//! Text rendering functions.

use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, Stroke};
use peniko::{Brush, Fill};

use super::painter::{PaintGlyph, Painter};
use crate::computed_style::{TextShadowValue, VisibilityValue};
use crate::node::{Node, NodeTree};

use super::paint_node;

/// Paint inline content from a cached InlineLayout (IFC root).
///
/// Renders glyph runs directly from the Parley layout. Inline boxes
/// (inline-block elements) are painted by looking up the child node
/// and painting it at the Parley-computed position.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_inline_layout(
    tree: &NodeTree,
    painter: &mut dyn Painter,
    scale: f64,
    parent_x: f64,
    parent_y: f64,
    inline_layout: &crate::node::InlineLayout,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<Brush>,
    text_shadows: &[TextShadowValue],
    transform: Affine,
    root_hidden: bool,
) {
    // Which of the laid-out text is `visibility: hidden` (#829). `None` — the
    // common case — means all of it is shown and nothing below changes.
    let mask = TextMask::for_ifc(tree, inline_layout, root_hidden);
    let mask = mask.as_ref();

    // Paint inline element backgrounds BEFORE text so text renders on top
    if !inline_layout.background_spans.is_empty() {
        paint_inline_backgrounds(tree, painter, parent_x, parent_y, inline_layout, transform);
    }

    render_text_with_shadow(
        painter,
        &inline_layout.layout,
        parent_x,
        parent_y,
        text_shadows,
        transform,
        scale,
        mask,
    );

    // Wavy underlines (`text-decoration-style: wavy` — the spellcheck squiggle)
    // paint AFTER the text, so the wave reads over the glyph descenders rather
    // than being hidden by them.
    if !inline_layout.decoration_spans.is_empty() {
        paint_wavy_decorations(
            painter,
            parent_x,
            parent_y,
            inline_layout,
            transform,
            scale,
            mask,
        );
    }

    // Paint inline-block boxes by looking them up in tree and painting
    for line in inline_layout.layout.lines() {
        for item in line.items() {
            if let parley::layout::PositionedLayoutItem::InlineBox(positioned_box) = item {
                let child_id = positioned_box.id as usize;
                paint_node(
                    tree, child_id, painter, scale, parent_x, parent_y, font_cx, layout_cx,
                    transform,
                );
            }
        }
    }
}

/// Paint background rectangles for inline elements (code, mark, etc.).
///
/// Uses Parley's cluster-based byte-to-position mapping to determine the
/// exact visual bounds of each inline background span. Handles multi-line
/// spans by iterating lines and computing per-line background rectangles.
fn paint_inline_backgrounds(
    tree: &NodeTree,
    painter: &mut dyn Painter,
    parent_x: f64,
    parent_y: f64,
    inline_layout: &crate::node::InlineLayout,
    css_transform: Affine,
) {
    use parley::layout::Cluster;
    use peniko::kurbo::{Rect, RoundedRect};

    let layout = &inline_layout.layout;
    let transform = css_transform * Affine::translate((parent_x, parent_y));

    for bg_span in &inline_layout.background_spans {
        // The background is the inline element's own visual, so it follows
        // that element's visibility — not the visibility of the text inside
        // it, which a `visibility: visible` child can override (#829).
        if tree.get(bg_span.owner).is_some_and(is_hidden) {
            continue;
        }
        let brush = Brush::Solid(bg_span.color);

        // Iterate lines to find those overlapping this background span
        for line in layout.lines() {
            let line_range = line.text_range();

            // Skip lines that don't overlap the background span
            if line_range.end <= bg_span.start || line_range.start >= bg_span.end {
                continue;
            }

            let line_metrics = line.metrics();
            let baseline = line_metrics.baseline as f64;
            let ascent = line_metrics.ascent as f64;
            let descent = line_metrics.descent as f64;

            // Clamp background span to this line's text range
            let overlap_start = bg_span.start.max(line_range.start);
            let overlap_end = bg_span.end.min(line_range.end);

            // Use Parley's cluster API for exact byte-to-position mapping
            let Some(sc) = Cluster::from_byte_index(layout, overlap_start) else {
                continue;
            };
            let end_byte = overlap_end.saturating_sub(1).max(overlap_start);
            let Some(ec) = Cluster::from_byte_index(layout, end_byte) else {
                continue;
            };

            let Some(x_start) = sc.visual_offset() else {
                continue;
            };
            let Some(ec_offset) = ec.visual_offset() else {
                continue;
            };
            let x_end = ec_offset + ec.advance();

            // Determine if this line segment gets padding
            let is_first_line = overlap_start == bg_span.start;
            let is_last_line = overlap_end == bg_span.end;
            let pl = if is_first_line {
                bg_span.padding_left as f64
            } else {
                0.0
            };
            let pr = if is_last_line {
                bg_span.padding_right as f64
            } else {
                0.0
            };
            let pt = bg_span.padding_top as f64;
            let pb = bg_span.padding_bottom as f64;

            let rect = Rect::new(
                x_start as f64 - pl,
                baseline - ascent - pt,
                x_end as f64 + pr,
                baseline + descent + pb,
            );

            if bg_span.border_radius > 0.0 {
                let r = bg_span.border_radius as f64;
                let rrect = RoundedRect::from_rect(rect, r);
                painter.fill(Fill::NonZero, transform, &brush, &rrect.into());
            } else {
                painter.fill(Fill::NonZero, transform, &brush, &rect.into());
            }
        }
    }
}

/// Paint the wavy underlines recorded as [`InlineDecorationSpan`]s.
///
/// Parley's decoration styles are a straight line of a given brush, offset and
/// size — there is no wavy variant — so `text-decoration-style: wavy` is carried
/// out of the cascade as a byte range (see
/// `RinchDocument::push_inline_spans`) and drawn here as a zigzag under the
/// run, the same way an inline background is drawn as a rectangle under one.
///
/// The wave is a polyline rather than a curve: at a ~1px amplitude over a ~5px
/// period the difference is below a pixel, and a polyline costs no curve
/// flattening in the software painter, where this runs on every repaint of a
/// document with squiggles in it. Geometry is in the same **scaled** space
/// [`render_text`] draws glyphs in, so the underline tracks the text on a HiDPI
/// display.
///
/// `mask` cuts the wave under hidden text cluster by cluster, as
/// [`render_text`] cuts the straight underline under hidden glyphs (#829).
///
/// [`InlineDecorationSpan`]: crate::node::InlineDecorationSpan
#[allow(clippy::too_many_arguments)]
fn paint_wavy_decorations(
    painter: &mut dyn Painter,
    parent_x: f64,
    parent_y: f64,
    inline_layout: &crate::node::InlineLayout,
    css_transform: Affine,
    scale: f64,
    mask: Option<&TextMask>,
) {
    use peniko::kurbo::BezPath;

    let sf = scale as f32;
    let layout = &inline_layout.layout;
    let transform = css_transform * Affine::translate((parent_x, parent_y));
    // CSS px, scaled with everything else below.
    const PERIOD: f64 = 5.0;
    const AMPLITUDE: f64 = 1.0;
    const THICKNESS: f64 = 1.0;

    for span in &inline_layout.decoration_spans {
        let brush = Brush::Solid(span.color);
        for line in layout.lines() {
            let line_range = line.text_range();
            if line_range.end <= span.start || line_range.start >= span.end {
                continue;
            }
            let start = span.start.max(line_range.start);
            let end = span.end.min(line_range.end);
            let segments = visual_segments(&line, start, end, mask);
            if segments.is_empty() {
                continue;
            }
            let metrics = line.metrics();
            let baseline = (metrics.baseline * sf) as f64;
            // Parley's `underline_offset` is measured **up** from the baseline
            // (the sign `render_text` subtracts with), and is per-run, so take it
            // from the line's first run and fall back to a fraction of the
            // descent for a line that somehow has none.
            let offset = line
                .runs()
                .next()
                .map(|r| r.metrics().underline_offset)
                .unwrap_or(-metrics.descent * 0.4);
            // Sit the *top* of the wave where a straight underline would be, so a
            // squiggle never rides up into the glyphs.
            let mid = baseline - (offset * sf) as f64 + AMPLITUDE * scale;
            let amp = AMPLITUDE * scale;
            let half = (PERIOD * scale) / 2.0;

            let mut path = BezPath::new();
            for (x0, x1) in segments {
                let (x0, x1) = ((x0 * sf) as f64, (x1 * sf) as f64);
                path.move_to((x0, mid + amp));
                let mut x = x0;
                let mut up = true;
                while x < x1 {
                    x = (x + half).min(x1);
                    path.line_to((x, if up { mid - amp } else { mid + amp }));
                    up = !up;
                }
            }
            painter.stroke(
                &Stroke::new(THICKNESS * scale),
                transform,
                &brush,
                &path.into(),
            );
        }
    }
}

/// Whether `node` is `visibility: hidden` or `collapse` — nothing of its own is
/// drawn. `visibility` inherits, so an element under a hidden ancestor answers
/// `true` too unless it declared `visible` itself.
pub(super) fn is_hidden(node: &Node) -> bool {
    matches!(
        node.computed_style.visibility,
        VisibilityValue::Hidden | VisibilityValue::Collapse
    )
}

/// Which bytes of an IFC's laid-out text belong to a `visibility: hidden`
/// element (#829).
///
/// A text node has no computed style of its own — it is drawn with its
/// parent element's — so each [`crate::node::IfcTextRange`] takes the
/// visibility of its text node's **DOM parent**. That is per element, not per
/// Parley run: two adjacent elements that differ only in visibility share one
/// style, so Parley shapes them into one glyph run, and the split has to be
/// made glyph by glyph (see [`GlyphCursor`]). Bytes no range covers —
/// the ellipsis layout records none — take the IFC root's visibility, which
/// is why a hidden span inside an ellipsis line still draws (#853).
pub(super) struct TextMask {
    /// `(start, end, hidden)` per text range, in layout byte offsets.
    ranges: Vec<(usize, usize, bool)>,
    /// The answer for bytes no range covers.
    default_hidden: bool,
}

impl TextMask {
    /// `None` when every byte is shown, which is the fast path: the painter
    /// then does exactly what it did before visibility was consulted.
    pub(super) fn for_ifc(
        tree: &NodeTree,
        inline_layout: &crate::node::InlineLayout,
        root_hidden: bool,
    ) -> Option<Self> {
        let mut ranges: Vec<(usize, usize, bool)> = inline_layout
            .text_ranges
            .iter()
            .filter(|r| !r.is_br)
            .map(|r| {
                let hidden = tree
                    .get(r.node_id)
                    .and_then(|t| t.parent)
                    .and_then(|p| tree.get(p))
                    .map(is_hidden)
                    .unwrap_or(root_hidden);
                (r.flat_start, r.flat_end, hidden)
            })
            .collect::<Vec<_>>();
        if !root_hidden && ranges.iter().all(|&(_, _, h)| !h) {
            return None;
        }
        // Already in text order; the sort is a linear pass that keeps
        // `hidden_at`'s binary search honest if that ever changes.
        ranges.sort_by_key(|&(s, _, _)| s);
        Some(Self {
            ranges,
            default_hidden: root_hidden,
        })
    }

    /// Whether the cluster starting at layout byte `byte` is hidden.
    ///
    /// A binary search: the ranges are pushed in text order by
    /// `walk_inline_children` (`flat_pos` only grows) and never overlap. A
    /// linear scan made a paint of one IFC quadratic in its spans (PR #844
    /// review, F2: 2000 spans, one hidden, 596 -> 1780 ms).
    fn hidden_at(&self, byte: usize) -> bool {
        let i = self.ranges.partition_point(|&(_, e, _)| e <= byte);
        match self.ranges.get(i) {
            Some(&(s, _, h)) if s <= byte => h,
            _ => self.default_hidden,
        }
    }
}

/// The **visual** x extents, in unscaled layout px, of the clusters of `line`
/// whose text lies in the byte range `start..end` — one `(x0, x1)` per stretch
/// that is contiguous on screen, left to right.
///
/// A byte range is contiguous in *logical* order only. In right-to-left text its
/// first cluster is the rightmost, so reading x0 off the first cluster and x1 off
/// the last gives `x1 <= x0` and no wave at all; in mixed-direction text the
/// range can be split across runs laid out in the other order, so the two ends
/// span too little (review of #836, finding 5). Walking each run's clusters in
/// visual order and merging what touches answers both.
fn visual_segments(
    line: &parley::layout::Line<'_, Brush>,
    start: usize,
    end: usize,
    mask: Option<&TextMask>,
) -> Vec<(f32, f32)> {
    let mut segments: Vec<(f32, f32)> = Vec::new();
    for run in line.runs() {
        let range = run.text_range();
        if range.end <= start || range.start >= end {
            continue;
        }
        // One O(line) offset lookup per run, then accumulate: `visual_offset`
        // per cluster would make a long line quadratic.
        let Some(mut x) = run.visual_clusters().next().and_then(|c| c.visual_offset()) else {
            continue;
        };
        for cluster in run.visual_clusters() {
            let text = cluster.text_range();
            let advance = cluster.advance();
            // A hidden cluster takes no wave (#829); the stretch breaks there.
            if text.start < end
                && text.end > start
                && !mask.is_some_and(|m| m.hidden_at(text.start))
            {
                match segments.last_mut() {
                    Some(last) if (last.1 - x).abs() < 0.01 => last.1 = x + advance,
                    _ => segments.push((x, x + advance)),
                }
            }
            x += advance;
        }
    }
    segments.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f32, f32)> = Vec::with_capacity(segments.len());
    for (x0, x1) in segments {
        match merged.last_mut() {
            Some(last) if x0 <= last.1 + 0.01 => last.1 = last.1.max(x1),
            _ => merged.push((x0, x1)),
        }
    }
    merged.retain(|&(x0, x1)| x1 > x0);
    merged
}

/// Lines a hidden mask up with Parley's glyph runs.
///
/// A line item is one `Run` view, which Parley splits into consecutive glyph
/// runs wherever the glyphs' `style_index` changes, and keeps each glyph run's
/// start private. So the cursor walks the run's visual-order glyphs **once**
/// per line item, recording each glyph's style index and whether its cluster
/// is hidden, and then hands out consecutive slices of that walk, splitting
/// them by style index exactly as Parley's own iterator does. Each glyph is
/// visited a constant number of times however many glyph runs its run holds —
/// re-walking the run per glyph run was the other half of F2.
#[derive(Default)]
struct GlyphCursor {
    key: Option<(usize, usize)>,
    next: usize,
    /// `(style_index, hidden)` per glyph of the current run, visual order.
    glyphs: Vec<(usize, bool)>,
}

impl GlyphCursor {
    /// This glyph run's hidden flags, or `None` when none of its glyphs is
    /// hidden; advances past it either way.
    fn take(
        &mut self,
        glyph_run: &parley::layout::GlyphRun<'_, Brush>,
        mask: &TextMask,
    ) -> Option<Vec<bool>> {
        let run = glyph_run.run();
        let key = (run.index(), run.cluster_range().start);
        if self.key != Some(key) {
            self.key = Some(key);
            self.next = 0;
            self.glyphs.clear();
            for cluster in run.visual_clusters() {
                let hidden = mask.hidden_at(cluster.text_range().start);
                self.glyphs
                    .extend(cluster.glyphs().map(|g| (g.style_index(), hidden)));
            }
        }
        let rest = self.glyphs.get(self.next..).unwrap_or(&[]);
        let style = rest.first()?.0;
        let count = rest.iter().take_while(|&&(si, _)| si == style).count();
        let slice = &rest[..count];
        self.next += count;
        slice
            .iter()
            .any(|&(_, h)| h)
            .then(|| slice.iter().map(|&(_, h)| h).collect())
    }
}

/// A glyph run's hidden flags, or `None` when it is entirely shown (always,
/// without a mask).
fn run_flags(
    cursor: &mut GlyphCursor,
    glyph_run: &parley::layout::GlyphRun<'_, Brush>,
    mask: Option<&TextMask>,
) -> Option<Vec<bool>> {
    cursor.take(glyph_run, mask?)
}

/// Render a Parley text layout using a Painter.
///
/// `scale` scales glyph positions and font sizes from logical to physical pixels
/// so text rasterizes crisply on HiDPI displays. Pass 1.0 for no scaling.
///
/// `mask` drops the glyphs of hidden elements, and their share of the
/// underline and line-through, while every shown glyph keeps the position it
/// was laid out at (#829).
#[allow(clippy::too_many_arguments)]
pub(super) fn render_text(
    painter: &mut dyn Painter,
    layout: &parley::layout::Layout<Brush>,
    x: f64,
    y: f64,
    css_transform: Affine,
    scale: f64,
    mask: Option<&TextMask>,
) {
    let sf = scale as f32;
    let transform = css_transform * Affine::translate((x, y));
    for line in layout.lines() {
        let mut cursor = GlyphCursor::default();
        for item in line.items() {
            let parley::layout::PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                continue;
            };
            let flags = run_flags(&mut cursor, &glyph_run, mask);
            if flags.as_ref().is_some_and(|f| f.iter().all(|&h| h)) {
                continue;
            }
            let mut gx = glyph_run.offset() * sf;
            let gy = glyph_run.baseline() * sf;
            let run = glyph_run.run();
            let font = run.font();
            let font_size = run.font_size() * sf;
            let synthesis = run.synthesis();
            let glyph_xform = synthesis
                .skew()
                .map(|angle| Affine::skew(angle.to_radians().tan() as f64, 0.0));
            let style = glyph_run.style();
            let brush = style.brush.clone();

            // The x extents of the shown stretches of this run, for the
            // decorations: the whole run when nothing in it is hidden.
            let mut segments: Vec<(f32, f32)> = Vec::new();
            let mut glyphs: Vec<PaintGlyph> = Vec::new();
            for (i, glyph) in glyph_run.glyphs().enumerate() {
                let start_x = gx;
                let px = gx + glyph.x * sf;
                let py = gy + glyph.y * sf;
                gx += glyph.advance * sf;
                if flags.as_ref().is_some_and(|f| f[i]) {
                    continue;
                }
                match segments.last_mut() {
                    Some(seg) if seg.1 == start_x => seg.1 = gx,
                    _ => segments.push((start_x, gx)),
                }
                glyphs.push(PaintGlyph {
                    id: glyph.id,
                    x: px,
                    y: py,
                });
            }
            painter.draw_glyphs(
                font,
                font_size,
                transform,
                glyph_xform,
                &brush,
                true,
                run.normalized_coords(),
                &glyphs,
            );

            // Draw underline decoration
            if let Some(underline) = &style.underline {
                let run_metrics = run.metrics();
                let offset = underline.offset.unwrap_or(run_metrics.underline_offset) * sf;
                let size = underline.size.unwrap_or(run_metrics.underline_size) * sf;
                let dec_brush = &underline.brush;
                let line_y = (gy - offset) as f64;
                let stroke = Stroke::new(size.max(1.0) as f64);
                for &(x0, x1) in &segments {
                    let line = peniko::kurbo::Line::new((x0 as f64, line_y), (x1 as f64, line_y));
                    painter.stroke(&stroke, transform, dec_brush, &line.into());
                }
            }

            // Draw strikethrough decoration
            if let Some(strikethrough) = &style.strikethrough {
                let run_metrics = run.metrics();
                let offset = strikethrough
                    .offset
                    .unwrap_or(run_metrics.strikethrough_offset)
                    * sf;
                let size = strikethrough.size.unwrap_or(run_metrics.strikethrough_size) * sf;
                let dec_brush = &strikethrough.brush;
                let line_y = (gy - offset) as f64;
                let stroke = Stroke::new(size.max(1.0) as f64);
                for &(x0, x1) in &segments {
                    let line = peniko::kurbo::Line::new((x0 as f64, line_y), (x1 as f64, line_y));
                    painter.stroke(&stroke, transform, dec_brush, &line.into());
                }
            }
        }
    }
}

/// Render a single shadow pass of a Parley text layout (glyphs only, no decorations).
///
/// Draws all glyph runs at the given offset with the specified shadow color,
/// ignoring the original brush from the layout styles. A hidden glyph casts no
/// shadow (`mask`, #829).
pub(super) fn render_text_shadow_pass(
    painter: &mut dyn Painter,
    layout: &parley::layout::Layout<Brush>,
    x: f64,
    y: f64,
    shadow_color: AlphaColor<Srgb>,
    css_transform: Affine,
    mask: Option<&TextMask>,
) {
    let transform = css_transform * Affine::translate((x, y));
    let shadow_brush = Brush::Solid(shadow_color);
    for line in layout.lines() {
        let mut cursor = GlyphCursor::default();
        for item in line.items() {
            let parley::layout::PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                continue;
            };
            let flags = run_flags(&mut cursor, &glyph_run, mask);
            let mut gx = glyph_run.offset();
            let gy = glyph_run.baseline();
            let run = glyph_run.run();
            let font = run.font();
            let font_size = run.font_size();
            let synthesis = run.synthesis();
            let glyph_xform = synthesis
                .skew()
                .map(|angle| Affine::skew(angle.to_radians().tan() as f64, 0.0));

            let glyphs: Vec<PaintGlyph> = glyph_run
                .glyphs()
                .enumerate()
                .filter_map(|(i, glyph)| {
                    let px = gx + glyph.x;
                    let py = gy + glyph.y;
                    gx += glyph.advance;
                    if flags.as_ref().is_some_and(|f| f[i]) {
                        return None;
                    }
                    Some(PaintGlyph {
                        id: glyph.id,
                        x: px,
                        y: py,
                    })
                })
                .collect();
            if glyphs.is_empty() {
                continue;
            }
            painter.draw_glyphs(
                font,
                font_size,
                transform,
                glyph_xform,
                &shadow_brush,
                true,
                run.normalized_coords(),
                &glyphs,
            );
        }
    }
}

/// Render text with optional text-shadow effects.
///
/// Draws shadow passes (in reverse order so first shadow renders on top of later ones)
/// at the specified offsets, then draws the normal text on top. `mask` is
/// [`render_text`]'s, applied to every pass.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_text_with_shadow(
    painter: &mut dyn Painter,
    layout: &parley::layout::Layout<Brush>,
    x: f64,
    y: f64,
    text_shadows: &[TextShadowValue],
    css_transform: Affine,
    scale: f64,
    mask: Option<&TextMask>,
) {
    if text_shadows.is_empty() {
        render_text(painter, layout, x, y, css_transform, scale, mask);
        return;
    }

    // Render shadows in reverse order (first shadow = topmost, drawn last before main text)
    for shadow in text_shadows.iter().rev() {
        let shadow_color = shadow.color.unwrap_or_else(|| {
            AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 255) // default: black
        });
        let sx = x + shadow.offset_x as f64;
        let sy = y + shadow.offset_y as f64;
        render_text_shadow_pass(painter, layout, sx, sy, shadow_color, css_transform, mask);
    }

    // Render the main text on top
    render_text(painter, layout, x, y, css_transform, scale, mask);
}
