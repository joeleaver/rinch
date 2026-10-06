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
    root: &Node,
    root_hidden: bool,
) {
    // Which of the laid-out text is `visibility: hidden` (#829). `None` — the
    // common case — means all of it is shown and nothing below changes.
    let mask = TextMask::for_ifc(tree, inline_layout, root_hidden);
    let mask = mask.as_ref();
    // Which of it is no longer the colour it was shaped in (#679). `None` —
    // the common case — means the layout's own brushes are current.
    let live = LiveColours::for_ifc(tree, inline_layout, root);
    let live = live.as_ref();

    // Paint inline element backgrounds BEFORE text so text renders on top
    if !inline_layout.background_spans.is_empty() {
        paint_inline_backgrounds(tree, painter, parent_x, parent_y, inline_layout, transform);
    }

    // Each text run casts the `text-shadow` its own element computed (#1048);
    // `None` — the common case — means every run casts the root's list.
    match ShadowGroup::for_ifc(tree, inline_layout, text_shadows, root_hidden) {
        None => render_text_shadows(
            painter,
            &inline_layout.layout,
            parent_x,
            parent_y,
            text_shadows,
            transform,
            scale,
            mask,
            Some(inline_layout),
        ),
        Some(groups) => {
            // Every shadow under every glyph, as for one list: each group's
            // passes through a mask that keeps them to the text casting them.
            for group in &groups {
                render_text_shadows(
                    painter,
                    &inline_layout.layout,
                    parent_x,
                    parent_y,
                    group.shadows,
                    transform,
                    scale,
                    Some(&group.mask),
                    Some(inline_layout),
                );
            }
        }
    }
    draw_text(
        painter,
        &inline_layout.layout,
        transform * Affine::translate((parent_x, parent_y)),
        scale,
        mask,
        None,
        None,
        Some(inline_layout),
        live,
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
            WavyBrush::Live(tree),
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
        // The rectangle is the element's as it is styled **now**: a
        // transition or animation frame writes its colour, padding and radius
        // with no re-shape (#679). A background faded out to nothing draws
        // nothing; only an owner that is gone falls back to what was recorded.
        let live;
        let bg_span = match tree.get(bg_span.owner) {
            Some(owner) => {
                match crate::ifc::inline_background_span(owner, bg_span.start, bg_span.end) {
                    Some(span) => {
                        live = span;
                        &live
                    }
                    None => continue,
                }
            }
            None => bg_span,
        };
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
/// `brush` says whose colour the wave takes: each span's own element's, read
/// from the tree as it is now (#679), or one brush for all of them — a
/// `text-shadow` pass draws the wave in the shadow's colour (#981).
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
    brush: WavyBrush<'_>,
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
        let brush = match brush {
            WavyBrush::All(brush) => brush.clone(),
            WavyBrush::Live(tree) => Brush::Solid(
                tree.get(span.owner)
                    .map(|o| crate::ifc::wavy_underline_color(&o.computed_style))
                    .unwrap_or(span.color),
            ),
        };
        for line in layout.lines() {
            let line_range = line.text_range();
            if line_range.end <= span.start
                || line_range.start >= span.end
                || mask.is_some_and(|m| !m.may_show(line_range.clone()))
            {
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

/// The colour [`paint_wavy_decorations`] draws in.
#[derive(Clone, Copy)]
enum WavyBrush<'a> {
    /// Each span in its own element's current colour.
    Live(&'a NodeTree),
    /// Every span in this brush.
    All(&'a Brush),
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

/// The element whose computed style a text range is drawn with: its text
/// node's DOM parent.
fn range_element<'a>(tree: &'a NodeTree, range: &crate::node::IfcTextRange) -> Option<&'a Node> {
    tree.get(range.node_id)
        .and_then(|t| t.parent)
        .and_then(|p| tree.get(p))
}

/// One `text-shadow` list an IFC's text casts, and the mask that keeps its
/// passes to the text that casts it (#1048).
///
/// `text-shadow` is inherited, so a run casts its own element's list — the
/// IFC root's unless an inline element on the way down declared another
/// (`none` included). Like `visibility` ([`TextMask`]), that is per element
/// and not per Parley run: text that differs only in its shadow shares one
/// glyph run, so the mask is made glyph by glyph. A group's mask also hides
/// what the visibility mask hides.
pub(super) struct ShadowGroup<'a> {
    pub(super) shadows: &'a [TextShadowValue],
    pub(super) mask: TextMask,
}

impl<'a> ShadowGroup<'a> {
    /// The non-empty lists `inline_layout`'s text casts, in the order their
    /// text first appears; `None` when every run casts `root_shadows` — the
    /// fast path, drawn exactly as before per-run shadows were consulted.
    /// Bytes no range covers cast the root's list. A `text-overflow: ellipsis`
    /// layout records no ranges at all, so there every run casts the root's
    /// list and a span's own is not drawn — as `visibility` is not honoured
    /// there either (#853, #1065).
    pub(super) fn for_ifc(
        tree: &'a NodeTree,
        inline_layout: &crate::node::InlineLayout,
        root_shadows: &'a [TextShadowValue],
        root_hidden: bool,
    ) -> Option<Vec<Self>> {
        // Asked of every IFC on every paint: answer the common case without
        // allocating, and without walking the runs at all while no inline
        // element has ever cast a list of its own.
        if !tree.inline_text_shadows
            || inline_layout
                .text_ranges
                .iter()
                .filter(|r| !r.is_br)
                .all(|r| {
                    range_element(tree, r)
                        .is_none_or(|e| e.computed_style.text_shadow.as_slice() == root_shadows)
                })
        {
            return None;
        }
        let ranges: Vec<(usize, usize, bool, &'a [TextShadowValue])> = inline_layout
            .text_ranges
            .iter()
            .filter(|r| !r.is_br)
            .map(|r| {
                let el = range_element(tree, r);
                (
                    r.flat_start,
                    r.flat_end,
                    el.map(is_hidden).unwrap_or(root_hidden),
                    el.map_or(root_shadows, |e| e.computed_style.text_shadow.as_slice()),
                )
            })
            .collect();
        let mut lists: Vec<&'a [TextShadowValue]> = Vec::new();
        for list in ranges
            .iter()
            .map(|&(_, _, _, s)| s)
            .chain(std::iter::once(root_shadows))
        {
            if !list.is_empty() && !lists.contains(&list) {
                lists.push(list);
            }
        }
        Some(
            lists
                .into_iter()
                .map(|list| {
                    let mut mask_ranges: Vec<(usize, usize, bool)> = ranges
                        .iter()
                        .map(|&(s, e, hidden, shadows)| (s, e, hidden || shadows != list))
                        .collect();
                    mask_ranges.sort_by_key(|&(s, _, _)| s);
                    let default_hidden = root_hidden || root_shadows != list;
                    // Uncovered bytes shown: no narrower span to promise.
                    let shown_span = default_hidden.then(|| {
                        mask_ranges
                            .iter()
                            .filter(|&&(_, _, hidden)| !hidden)
                            .fold((usize::MAX, 0), |(s, e), &(rs, re, _)| {
                                (s.min(rs), e.max(re))
                            })
                    });
                    Self {
                        shadows: list,
                        mask: TextMask {
                            ranges: mask_ranges,
                            default_hidden,
                            shown_span,
                        },
                    }
                })
                .collect(),
        )
    }
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
    /// The byte span every shown byte lies in, when that is narrower than the
    /// whole text: a line outside it draws nothing and is not walked
    /// ([`Self::may_show`]). `None` means any line may.
    shown_span: Option<(usize, usize)>,
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
                let hidden = range_element(tree, r).map(is_hidden).unwrap_or(root_hidden);
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
            shown_span: None,
        })
    }

    /// Whether anything in the byte range `range` (a line's) may be shown.
    ///
    /// A shadow group's mask shows only its own text (#1048), and walking a
    /// line's items costs parley a scan of the run per glyph run it yields —
    /// so each group's pass skips the lines its text is not on. Without that,
    /// a paragraph of N spans with N distinct lists cost N passes over every
    /// line: 298 ms at N = 400 (review of #1063).
    pub(super) fn may_show(&self, range: std::ops::Range<usize>) -> bool {
        self.shown_span
            .is_none_or(|(s, e)| range.start < e && range.end > s)
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

/// [`GlyphCursor`]'s shape, for `vertical-align` (#724) instead of
/// visibility: a per-glyph shift in unscaled layout px rather than a bool.
/// Needed because [`crate::node::InlineLayout::vertical_align_shift_at`]
/// answers by **byte**, and a glyph carries no byte offset of its own — only
/// its cluster does — so the shift has to be looked up once per cluster and
/// expanded to one entry per glyph, exactly as the hidden flag is.
#[derive(Default)]
struct ValignCursor {
    key: Option<(usize, usize)>,
    next: usize,
    /// `(style_index, shift_px)` per glyph of the current run, visual order.
    glyphs: Vec<(usize, f32)>,
}

impl ValignCursor {
    /// This glyph run's per-glyph shifts, or `None` when every one of them is
    /// `0.0` (the common case, including whenever `inline_layout` has no
    /// `vertical_align_spans` at all); advances past it either way.
    fn take(
        &mut self,
        glyph_run: &parley::layout::GlyphRun<'_, Brush>,
        inline_layout: &crate::node::InlineLayout,
    ) -> Option<Vec<f32>> {
        let run = glyph_run.run();
        let key = (run.index(), run.cluster_range().start);
        if self.key != Some(key) {
            self.key = Some(key);
            self.next = 0;
            self.glyphs.clear();
            for cluster in run.visual_clusters() {
                let shift = inline_layout.vertical_align_shift_at(cluster.text_range().start);
                self.glyphs
                    .extend(cluster.glyphs().map(|g| (g.style_index(), shift)));
            }
        }
        let rest = self.glyphs.get(self.next..).unwrap_or(&[]);
        let style = rest.first()?.0;
        let count = rest.iter().take_while(|&&(si, _)| si == style).count();
        let slice = &rest[..count];
        self.next += count;
        slice
            .iter()
            .any(|&(_, s)| s != 0.0)
            .then(|| slice.iter().map(|&(_, s)| s).collect())
    }
}

/// What a stretch of text is drawn in instead of the brush it was shaped
/// with (#679).
#[derive(Clone, Copy, PartialEq)]
struct Recolour {
    /// The colour its element computes now; `None` keeps the layout's brush.
    color: Option<AlphaColor<Srgb>>,
    /// Whether its underline is in the text's colour (`currentcolor`) and so
    /// takes `color` too, rather than a declared `text-decoration-color`.
    underline: bool,
    /// The same for its line-through.
    strikethrough: bool,
}

impl Recolour {
    const KEEP: Self = Self {
        color: None,
        underline: false,
        strikethrough: false,
    };
}

/// Which of an IFC's laid-out text is no longer the colour it was shaped in,
/// and what colour it is now (#679).
///
/// A Parley layout carries each run's colour as a brush, fixed when the text
/// was shaped. The cascade drops the layout when a colour changes, but a
/// transition or animation frame writes `computed_style` with no cascade, and
/// re-shaping a paragraph for every frame of a fade would be the wrong price
/// for a change that moves no glyph. So paint asks each text range for its
/// colour again ([`crate::ifc::text_color`], the function the build recorded
/// [`crate::node::IfcTextRange::color`] with) and draws the ranges that
/// answer differently in the new one — the #904 rule for a text leaf, per
/// range. Like visibility ([`TextMask`]) that is per element and not per
/// Parley run.
///
/// Bytes no range covers — a `text-overflow: ellipsis` rebuild records none —
/// were shaped in the root's own colour and follow that.
pub(super) struct LiveColours {
    /// `(start, end, recolour)` per text range, in layout byte offsets.
    ranges: Vec<(usize, usize, Recolour)>,
    /// The answer for bytes no range covers.
    default: Recolour,
}

impl LiveColours {
    /// `None` when every range still computes the colour it was shaped in,
    /// which is the fast path: the painter then draws the layout's brushes.
    pub(super) fn for_ifc(
        tree: &NodeTree,
        inline_layout: &crate::node::InlineLayout,
        root: &Node,
    ) -> Option<Self> {
        let root_now = crate::ifc::root_text_color(&root.computed_style);
        let now = |r: &crate::node::IfcTextRange| crate::ifc::text_color(&tree.nodes, r.node_id);
        if root_now == inline_layout.root_color
            && inline_layout
                .text_ranges
                .iter()
                .all(|r| r.is_br || now(r) == r.color)
        {
            return None;
        }
        // The last element a run's decoration colour can come from: the
        // root, or for an anonymous box the container its text belongs to.
        let stop = if root.is_anonymous_block_box {
            root.parent.unwrap_or(root.id)
        } else {
            root.id
        };
        let recolour = |from: usize, color: AlphaColor<Srgb>| {
            let (underline, strikethrough) = decorations_follow_color(tree, from, stop);
            Recolour {
                color: Some(color),
                underline,
                strikethrough,
            }
        };
        let mut ranges: Vec<(usize, usize, Recolour)> = inline_layout
            .text_ranges
            .iter()
            .filter(|r| !r.is_br)
            .map(|r| {
                let color = now(r);
                let recolour = match tree.get(r.node_id).and_then(|t| t.parent) {
                    Some(element) if color != r.color => recolour(element, color),
                    _ => Recolour::KEEP,
                };
                (r.flat_start, r.flat_end, recolour)
            })
            .collect();
        ranges.sort_by_key(|&(s, _, _)| s);
        let default = if root_now == inline_layout.root_color {
            Recolour::KEEP
        } else {
            recolour(root.id, root_now)
        };
        Some(Self { ranges, default })
    }

    /// The recolour of the cluster starting at layout byte `byte`.
    fn at(&self, byte: usize) -> Recolour {
        let i = self.ranges.partition_point(|&(_, e, _)| e <= byte);
        match self.ranges.get(i) {
            Some(&(s, _, r)) if s <= byte => r,
            _ => self.default,
        }
    }
}

/// Whether the underline and the line-through of text in element `from` are
/// drawn in the text's own colour: `(underline, line-through)`.
///
/// The build pushes a decoration brush only for an element that declares a
/// `text-decoration-color` (`inline_style_props`; a wavy element's is its
/// squiggle's and not an underline's), a span inherits the one around it, and
/// Parley falls back to the text brush where none was pushed. So a decoration
/// is `currentcolor` exactly when no element from `from` up to `stop` — the
/// last one whose style the layout was built from — declares one.
fn decorations_follow_color(tree: &NodeTree, from: usize, stop: usize) -> (bool, bool) {
    let (mut underline, mut strikethrough) = (true, true);
    let mut cur = Some(from);
    while let Some(node) = cur.and_then(|id| tree.get(id)) {
        let decoration = &node.computed_style.text_decoration;
        if decoration.color.is_some() {
            strikethrough = false;
            if !decoration.is_wavy_underline() {
                underline = false;
            }
        }
        if node.id == stop || !(underline || strikethrough) {
            break;
        }
        cur = node.parent;
    }
    (underline, strikethrough)
}

/// [`GlyphCursor`]'s shape, for [`LiveColours`]: a [`Recolour`] per glyph.
#[derive(Default)]
struct ColourCursor {
    key: Option<(usize, usize)>,
    next: usize,
    /// `(style_index, recolour)` per glyph of the current run, visual order.
    glyphs: Vec<(usize, Recolour)>,
}

impl ColourCursor {
    /// This glyph run's recolours, or `None` when none of its glyphs is
    /// recoloured; advances past it either way.
    fn take(
        &mut self,
        glyph_run: &parley::layout::GlyphRun<'_, Brush>,
        live: &LiveColours,
    ) -> Option<Vec<Recolour>> {
        let run = glyph_run.run();
        let key = (run.index(), run.cluster_range().start);
        if self.key != Some(key) {
            self.key = Some(key);
            self.next = 0;
            self.glyphs.clear();
            for cluster in run.visual_clusters() {
                let recolour = live.at(cluster.text_range().start);
                self.glyphs
                    .extend(cluster.glyphs().map(|g| (g.style_index(), recolour)));
            }
        }
        let rest = self.glyphs.get(self.next..).unwrap_or(&[]);
        let style = rest.first()?.0;
        let count = rest.iter().take_while(|&&(si, _)| si == style).count();
        let slice = &rest[..count];
        self.next += count;
        slice
            .iter()
            .any(|&(_, r)| r != Recolour::KEEP)
            .then(|| slice.iter().map(|&(_, r)| r).collect())
    }
}

/// [`run_flags`]'s shape for [`ValignCursor`].
///
/// Short-circuits on an empty `vertical_align_spans` **before** touching the
/// cursor, which is the fast path for the overwhelming majority of
/// documents (nothing declares a non-`baseline` `vertical-align`): without
/// this check every glyph run would re-walk its clusters for nothing, since
/// `ValignCursor::take` has no cheaper way to answer "no span starts between
/// here and the next style change" than building the per-glyph list and
/// finding it all-zero.
fn run_shifts(
    cursor: &mut ValignCursor,
    glyph_run: &parley::layout::GlyphRun<'_, Brush>,
    inline_layout: Option<&crate::node::InlineLayout>,
) -> Option<Vec<f32>> {
    let inline_layout = inline_layout?;
    if inline_layout.vertical_align_spans.is_empty() {
        return None;
    }
    cursor.take(glyph_run, inline_layout)
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
///
/// `color` overrides the brush baked into the layout for the **glyphs**; a
/// decoration keeps its own brush, which is `text-decoration-color` and not
/// `color`. A text leaf passes its parent's **current** computed
/// colour (#904): its cached layout is rebuilt only by a layout compute, so a
/// colour-only change — a hover, a transition frame — is applied here rather
/// than by re-shaping.
///
/// `valign` is the IFC whose `vertical-align` spans (#724) shift this text's
/// glyphs; `None` — the common case, including every render with no
/// `InlineLayout` in scope — shifts nothing.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_text(
    painter: &mut dyn Painter,
    layout: &parley::layout::Layout<Brush>,
    x: f64,
    y: f64,
    css_transform: Affine,
    scale: f64,
    mask: Option<&TextMask>,
    color: Option<AlphaColor<Srgb>>,
    valign: Option<&crate::node::InlineLayout>,
) {
    let transform = css_transform * Affine::translate((x, y));
    let color_brush = color.map(Brush::Solid);
    draw_text(
        painter,
        layout,
        transform,
        scale,
        mask,
        color_brush.as_ref(),
        None,
        valign,
        None,
    );
}

/// Draw `layout`'s glyphs and its straight decorations (underline,
/// line-through) through `transform`, which already carries the text's
/// origin. `glyph_brush` overrides the layout's brush for the glyphs and
/// `decoration_brush` for the decorations; `None` keeps each one's own.
/// `valign` is [`render_text`]'s.
///
/// [`render_text`] and every `text-shadow` pass draw through this one
/// function, so a shadow is the main pass's glyphs **and** decorations
/// (#981), segment for segment — a hidden stretch that draws no underline
/// casts no underline shadow either (#829), and a shifted stretch casts its
/// shadow at the shifted position.
///
/// `live` recolours the text whose element no longer computes the colour it
/// was shaped in (#679), where `glyph_brush` does not override every glyph
/// anyway. A glyph run is then drawn as one stretch per colour: text that
/// differs only in a colour written since the build shares a run.
#[allow(clippy::too_many_arguments)]
fn draw_text(
    painter: &mut dyn Painter,
    layout: &parley::layout::Layout<Brush>,
    transform: Affine,
    scale: f64,
    mask: Option<&TextMask>,
    glyph_brush: Option<&Brush>,
    decoration_brush: Option<&Brush>,
    valign: Option<&crate::node::InlineLayout>,
    live: Option<&LiveColours>,
) {
    let sf = scale as f32;
    let live = live.filter(|_| glyph_brush.is_none());
    for line in layout.lines() {
        if mask.is_some_and(|m| !m.may_show(line.text_range())) {
            continue;
        }
        let mut cursor = GlyphCursor::default();
        let mut vcursor = ValignCursor::default();
        let mut ccursor = ColourCursor::default();
        for item in line.items() {
            let parley::layout::PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                continue;
            };
            // Before the hidden test's `continue`: the cursor hands out a
            // run's glyph runs in order and has to see each of them.
            let recolours = live.and_then(|l| ccursor.take(&glyph_run, l));
            let flags = run_flags(&mut cursor, &glyph_run, mask);
            if flags.as_ref().is_some_and(|f| f.iter().all(|&h| h)) {
                continue;
            }
            let shifts = run_shifts(&mut vcursor, &glyph_run, valign);
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
            let brush = glyph_brush.unwrap_or(&style.brush);
            // The shift this whole run's decorations (underline/line-through)
            // sit at: the first glyph's, which is exact whenever the run is
            // wholly inside one `vertical-align` span — the common case, since
            // `<sub>`/`<sup>` also change `font-size` (#674), which already
            // forces a run split at the same boundary.
            let decoration_shift = shifts
                .as_ref()
                .and_then(|s| s.first())
                .copied()
                .unwrap_or(0.0)
                * sf;

            let run_metrics = run.metrics();
            // Underline, then line-through: `(offset, size, brush)` each.
            let decorations = [
                style.underline.as_ref().map(|d| {
                    (
                        d.offset.unwrap_or(run_metrics.underline_offset),
                        d.size.unwrap_or(run_metrics.underline_size),
                        &d.brush,
                    )
                }),
                style.strikethrough.as_ref().map(|d| {
                    (
                        d.offset.unwrap_or(run_metrics.strikethrough_offset),
                        d.size.unwrap_or(run_metrics.strikethrough_size),
                        &d.brush,
                    )
                }),
            ];
            // Draw one stretch of the run: its glyphs, then its share of the
            // decorations. `recolour` is the stretch's; a decoration takes
            // the new colour only where its own was the text's (the
            // underline, then the line-through).
            let mut draw = |glyphs: &[PaintGlyph], segments: &[(f32, f32)], recolour: Recolour| {
                let recoloured = recolour.color.map(Brush::Solid);
                painter.draw_glyphs(
                    font,
                    font_size,
                    transform,
                    glyph_xform,
                    recoloured.as_ref().unwrap_or(brush),
                    true,
                    run.normalized_coords(),
                    glyphs,
                );
                let follows = [recolour.underline, recolour.strikethrough];
                for ((offset, size, own_brush), follows) in decorations
                    .iter()
                    .zip(follows)
                    .filter_map(|(d, f)| d.map(|d| (d, f)))
                {
                    let dec_brush = decoration_brush.unwrap_or(match &recoloured {
                        Some(now) if follows => now,
                        _ => own_brush,
                    });
                    let line_y = (gy + decoration_shift - offset * sf) as f64;
                    let stroke = Stroke::new((size * sf).max(1.0) as f64);
                    for &(x0, x1) in segments {
                        let line =
                            peniko::kurbo::Line::new((x0 as f64, line_y), (x1 as f64, line_y));
                        painter.stroke(&stroke, transform, dec_brush, &line.into());
                    }
                }
            };

            // The x extents of the shown stretches of this run, for the
            // decorations: the whole run when nothing in it is hidden.
            let mut segments: Vec<(f32, f32)> = Vec::new();
            let mut glyphs: Vec<PaintGlyph> = Vec::new();
            let mut stretch = Recolour::KEEP;
            for (i, glyph) in glyph_run.glyphs().enumerate() {
                let start_x = gx;
                let shift = shifts.as_ref().map(|s| s[i]).unwrap_or(0.0) * sf;
                let px = gx + glyph.x * sf;
                let py = gy + glyph.y * sf + shift;
                gx += glyph.advance * sf;
                if flags.as_ref().is_some_and(|f| f[i]) {
                    continue;
                }
                if let Some(recolours) = &recolours
                    && recolours[i] != stretch
                {
                    if !glyphs.is_empty() {
                        draw(&glyphs, &segments, stretch);
                        glyphs.clear();
                        segments.clear();
                    }
                    stretch = recolours[i];
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
            draw(&glyphs, &segments, stretch);
        }
    }
}

/// One copy of a shadow: [`draw_text`] and the wavy underlines, all in `brush`.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_shadow_copy(
    painter: &mut dyn Painter,
    layout: &parley::layout::Layout<Brush>,
    x: f64,
    y: f64,
    brush: &Brush,
    css_transform: Affine,
    scale: f64,
    mask: Option<&TextMask>,
    wavy: Option<&crate::node::InlineLayout>,
) {
    let transform = css_transform * Affine::translate((x, y));
    draw_text(
        painter,
        layout,
        transform,
        scale,
        mask,
        Some(brush),
        Some(brush),
        wavy,
        None,
    );
    if let Some(inline_layout) = wavy
        && !inline_layout.decoration_spans.is_empty()
    {
        paint_wavy_decorations(
            painter,
            x,
            y,
            inline_layout,
            css_transform,
            scale,
            mask,
            WavyBrush::All(brush),
        );
    }
}

/// Render text with optional text-shadow effects.
///
/// Draws shadow passes (in reverse order so first shadow renders on top of later ones)
/// at the specified offsets, then draws the normal text on top. `mask` is
/// [`render_text`]'s, applied to every pass. `wavy` is the IFC whose wavy
/// underlines the shadows cast too (#981); the main pass's own wavy
/// underlines are the caller's, drawn after the text.
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
    color: Option<AlphaColor<Srgb>>,
    wavy: Option<&crate::node::InlineLayout>,
) {
    render_text_shadows(
        painter,
        layout,
        x,
        y,
        text_shadows,
        css_transform,
        scale,
        mask,
        wavy,
    );

    // Render the main text on top
    render_text(
        painter,
        layout,
        x,
        y,
        css_transform,
        scale,
        mask,
        color,
        wavy,
    );
}

/// The shadow passes of [`render_text_with_shadow`], without the text: every
/// shadow in `text_shadows`, the first on top, each through `mask`.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_text_shadows(
    painter: &mut dyn Painter,
    layout: &parley::layout::Layout<Brush>,
    x: f64,
    y: f64,
    text_shadows: &[TextShadowValue],
    css_transform: Affine,
    scale: f64,
    mask: Option<&TextMask>,
    wavy: Option<&crate::node::InlineLayout>,
) {
    // Render shadows in reverse order (first shadow = topmost, drawn last before main text)
    for shadow in text_shadows.iter().rev() {
        let shadow_color = shadow.color.unwrap_or_else(|| {
            AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 255) // default: black
        });
        // The offset and the blur are CSS px and `x`/`y` are physical: scale
        // them like every other length on its way to the painter (#409).
        let sx = x + shadow.offset_x as f64 * scale;
        let sy = y + shadow.offset_y as f64 * scale;
        super::text_shadow::render_text_shadow_pass(
            painter,
            layout,
            sx,
            sy,
            shadow_color,
            shadow.blur_radius.max(0.0) as f64 * scale,
            css_transform,
            scale,
            mask,
            wavy,
        );
    }
}
