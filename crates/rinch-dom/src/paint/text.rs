//! Text rendering functions.

use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, Rect, Stroke, Vec2};
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
    // Every pass below draws from the same lines (#1066). Set up as shared
    // below when there are passes per fragment or per span list.
    let text = PaintText::of_ifc(inline_layout);

    // Paint inline element backgrounds BEFORE text so text renders on top
    if !inline_layout.background_spans.is_empty() {
        paint_inline_backgrounds(
            tree,
            painter,
            scale,
            parent_x,
            parent_y,
            inline_layout,
            transform,
        );
    }

    // Chrome paints each text fragment's shadows and then its text before
    // the next fragment's (#1060). `None` — the common case — is where that
    // order and the one-pass order below paint the same pixels.
    if let Some(fragments) = TextFragment::in_paint_order(
        tree,
        inline_layout,
        text_shadows,
        root_hidden,
        crate::ifc::root_text_color(&root.computed_style),
    ) {
        text.share();
        for fragment in &fragments {
            render_text_shadows(
                painter,
                &text,
                parent_x,
                parent_y,
                fragment.shadows,
                transform,
                scale,
                Some(&fragment.mask),
                Some(inline_layout),
            );
            draw_text(
                painter,
                &text,
                transform * Affine::translate((parent_x, parent_y)),
                scale,
                Some(&fragment.mask),
                None,
                None,
                Some(inline_layout),
                live,
            );
            if !inline_layout.decoration_spans.is_empty() {
                paint_wavy_decorations(
                    painter,
                    parent_x,
                    parent_y,
                    inline_layout,
                    transform,
                    scale,
                    Some(&fragment.mask),
                    WavyBrush::Live(tree),
                );
            }
        }
    } else {
        // Each text run casts the `text-shadow` its own element computed
        // (#1048); `None` — the common case — means every run casts the
        // root's list.
        match ShadowGroup::for_ifc(tree, inline_layout, text_shadows, root_hidden) {
            None => render_text_shadows(
                painter,
                &text,
                parent_x,
                parent_y,
                text_shadows,
                transform,
                scale,
                mask,
                Some(inline_layout),
            ),
            Some(groups) => {
                text.share();
                // Every shadow under every glyph, as for one list: each
                // group's passes through a mask that keeps them to the text
                // casting them.
                for group in &groups {
                    render_text_shadows(
                        painter,
                        &text,
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
            &text,
            transform * Affine::translate((parent_x, parent_y)),
            scale,
            mask,
            None,
            None,
            Some(inline_layout),
            live,
        );

        // Wavy underlines (`text-decoration-style: wavy` — the spellcheck
        // squiggle) paint AFTER the text, so the wave reads over the glyph
        // descenders rather than being hidden by them.
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
    }

    // The "…" of each line `text-overflow: ellipsis` cuts (#1100), after the
    // text it ends, in the style of the block container whose ellipsis it is:
    // an anonymous block box's own style is a copy of its container's
    // inherited properties, made when the box was (#1071).
    if let Some(cut) = inline_layout.ellipsis.as_deref()
        && !root_hidden
    {
        let owner = root
            .parent
            .filter(|_| root.is_anonymous_block_box)
            .and_then(|p| tree.get(p))
            .unwrap_or(root);
        paint_ellipsis(
            painter,
            cut,
            parent_x,
            parent_y,
            transform,
            scale,
            &owner.computed_style.text_shadow,
            crate::ifc::root_text_color(&owner.computed_style),
        );
    }

    // Paint inline-block boxes by looking them up in tree and painting
    let hidden_boxes = inline_layout
        .ellipsis
        .as_deref()
        .map_or(&[][..], |c| c.hidden_boxes.as_slice());
    // Walking a line's items walks its clusters; a layout with no inline box
    // has nothing to find.
    let lines = if inline_layout.layout.inline_boxes().len() == 0 {
        None
    } else {
        Some(inline_layout.layout.lines())
    };
    for line in lines.into_iter().flatten() {
        for item in line.items() {
            if let parley::layout::PositionedLayoutItem::InlineBox(positioned_box) = item {
                if crate::ifc::is_edge_box(positioned_box.id) {
                    continue;
                }
                let child_id = positioned_box.id as usize;
                // An atomic inline past an ellipsis cut is hidden whole.
                if hidden_boxes.contains(&child_id) {
                    continue;
                }
                paint_node(
                    tree, child_id, painter, scale, parent_x, parent_y, font_cx, layout_cx,
                    transform,
                );
            }
        }
    }
}

/// Draw the "…" of every line a `text-overflow: ellipsis` cut ends (#1100):
/// the root's `text-shadow` list under it, then the glyph in the root's
/// colour as it is now (a transition frame writes it with no re-shape).
#[allow(clippy::too_many_arguments)]
fn paint_ellipsis(
    painter: &mut dyn Painter,
    cut: &crate::node::EllipsisCut,
    parent_x: f64,
    parent_y: f64,
    transform: Affine,
    scale: f64,
    text_shadows: &[TextShadowValue],
    color: peniko::Color,
) {
    let text = PaintText::new(&cut.glyph);
    let brush = Brush::Solid(color);
    for mark in &cut.marks {
        let x = parent_x + mark.x as f64 * scale;
        let y = parent_y + mark.y as f64 * scale;
        render_text_shadows(
            painter,
            &text,
            x,
            y,
            text_shadows,
            transform,
            scale,
            None,
            None,
        );
        draw_text(
            painter,
            &text,
            transform * Affine::translate((x, y)),
            scale,
            None,
            Some(&brush),
            None,
            None,
            None,
        );
    }
}

/// Paint the background and border of inline elements (code, mark, etc.).
///
/// The rectangles are [`inline_background_rects`]'s, which the clip proof
/// (`layer_bounds`, #1762) reads too: they reach past the line box by the
/// element's padding and border, so a box whose text fits can still have
/// something to clip. The background covers the border box; the border is
/// drawn solid (any style but `none`/`hidden`), its left and right sides on
/// the fragments that close them (#1426).
fn paint_inline_backgrounds(
    tree: &NodeTree,
    painter: &mut dyn Painter,
    scale: f64,
    parent_x: f64,
    parent_y: f64,
    inline_layout: &crate::node::InlineLayout,
    css_transform: Affine,
) {
    use peniko::kurbo::RoundedRect;

    let transform = css_transform * Affine::translate((parent_x, parent_y));
    inline_background_rects(
        |id| tree.get(id),
        inline_layout,
        scale,
        |frag| {
            let bg_span = frag.span;
            let rect = frag.rect;
            if bg_span.color.components[3] > 0.0 {
                let brush = Brush::Solid(bg_span.color);
                if bg_span.border_radius > 0.0 {
                    let r = bg_span.border_radius as f64 * scale;
                    let rrect = RoundedRect::from_rect(rect, r);
                    painter.fill(Fill::NonZero, transform, &brush, &rrect.into());
                } else {
                    painter.fill(Fill::NonZero, transform, &brush, &rect.into());
                }
            }
            // The border: top and bottom on every fragment, the left and right
            // on the fragments that close them.
            let Some(cs) = tree.get(bg_span.owner).map(|o| &o.computed_style) else {
                return;
            };
            let [bt, br, bb, bl] = inline_border_widths(cs).map(|w| w * scale);
            let current = cs.color.unwrap_or(peniko::Color::BLACK);
            let mut strip = |r: Rect, c: Option<peniko::Color>| {
                if r.width() > 0.0 && r.height() > 0.0 {
                    let brush = Brush::Solid(c.unwrap_or(current));
                    painter.fill(Fill::NonZero, transform, &brush, &r.into());
                }
            };
            strip(
                Rect::new(rect.x0, rect.y0, rect.x1, rect.y0 + bt),
                cs.border_top_color,
            );
            strip(
                Rect::new(rect.x0, rect.y1 - bb, rect.x1, rect.y1),
                cs.border_bottom_color,
            );
            if frag.closes_left {
                strip(
                    Rect::new(rect.x0, rect.y0, rect.x0 + bl, rect.y1),
                    cs.border_left_color,
                );
            }
            if frag.closes_right {
                strip(
                    Rect::new(rect.x1 - br, rect.y0, rect.x1, rect.y1),
                    cs.border_right_color,
                );
            }
        },
    );
}

/// An inline element's border widths, `[top, right, bottom, left]` in CSS
/// px: 0 for a side styled `none` or `hidden`.
fn inline_border_widths(cs: &crate::computed_style::ComputedStyle) -> [f64; 4] {
    use crate::computed_style::BorderStyleValue as S;
    let border = |w: f32, s: S| {
        if matches!(s, S::None | S::Hidden) {
            0.0
        } else {
            w.max(0.0) as f64
        }
    };
    [
        border(cs.border_top_width.to_px(), cs.border_top_style),
        border(cs.border_right_width.to_px(), cs.border_right_style),
        border(cs.border_bottom_width.to_px(), cs.border_bottom_style),
        border(cs.border_left_width.to_px(), cs.border_left_style),
    ]
}

/// One line's piece of an inline element's box ([`inline_background_rects`]).
pub(super) struct InlineBoxFragment<'a> {
    /// The element's span, as it is styled now.
    pub span: &'a crate::node::InlineBackgroundSpan,
    /// The fragment's border box, in device px relative to the IFC root's
    /// content origin.
    pub rect: Rect,
    /// The fragment holds the element's left edge (its start on a
    /// left-to-right line, its end on a right-to-left one).
    pub closes_left: bool,
    /// The fragment holds the element's right edge.
    pub closes_right: bool,
}

/// Every fragment [`paint_inline_backgrounds`] fills, in device px relative
/// to the IFC root's content origin (before its CSS transform).
///
/// One per line the element is on: its glyphs' extent, by Parley's cluster
/// byte-to-position mapping, joined with its edge boxes on that line less
/// their margins (#1426) — the start padding and border on its first line,
/// the end ones on its last (CSS `box-decoration-break: slice`) — grown
/// vertically by its padding and border, and moved by the `vertical-align`
/// shift its text is drawn with ([`crate::node::InlineLayout::element_shift`],
/// #1817): a `<sup>`'s background is around the raised text.
///
/// The layout is built in CSS px, so everything here is multiplied by
/// `scale`, as [`render_text`] scales the glyphs drawn over it (#1762).
///
/// `get` looks a node up (the tree's own, or a slab borrowed elsewhere).
pub(super) fn inline_background_rects<'a>(
    get: impl Fn(crate::node::RawNodeId) -> Option<&'a Node>,
    inline_layout: &crate::node::InlineLayout,
    scale: f64,
    mut f: impl FnMut(InlineBoxFragment<'_>),
) {
    use parley::layout::{Cluster, PositionedLayoutItem};

    let layout = &inline_layout.layout;
    if inline_layout.background_spans.is_empty() {
        return;
    }
    // Each line's edge boxes (#1426), `(owner, is end, x, width)`, read in one
    // pass over the lines: asking every line's items again for every span
    // made a paragraph of many pills quadratic (500 pills in 1500 runs took
    // a second a paint).
    let ranges: Vec<std::ops::Range<usize>> = layout.lines().map(|l| l.text_range()).collect();
    let edges: Vec<Vec<(crate::node::RawNodeId, bool, f64, f64)>> = layout
        .lines()
        .map(|line| {
            line.items()
                .filter_map(|item| match item {
                    PositionedLayoutItem::InlineBox(b) => crate::ifc::edge_box_owner(b.id)
                        .map(|(o, end)| (o, end, b.x as f64, b.width as f64)),
                    _ => None,
                })
                .collect()
        })
        .collect();

    for bg_span in &inline_layout.background_spans {
        // The background is the inline element's own visual, so it follows
        // that element's visibility — not the visibility of the text inside
        // it, which a `visibility: visible` child can override (#829).
        let owner = get(bg_span.owner);
        if owner.is_some_and(is_hidden) {
            continue;
        }
        // The rectangle is the element's as it is styled **now**: a
        // transition or animation frame writes its colour, padding and radius
        // with no re-shape (#679). A background faded out to nothing draws
        // nothing; only an owner that is gone falls back to what was recorded.
        let live;
        let bg_span = match owner {
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
        let style = owner.map(|o| &o.computed_style);
        let ltr = owner.is_none_or(crate::ifc::element_is_ltr);
        let px = |v: crate::computed_style::LengthPercentageAutoValue| match v {
            crate::computed_style::LengthPercentageAutoValue::Length(px) => px as f64,
            _ => 0.0,
        };
        let (margin_left, margin_right) =
            style.map_or((0.0, 0.0), |cs| (px(cs.margin_left), px(cs.margin_right)));
        let [bt, _, bb, _] = style.map_or([0.0; 4], inline_border_widths);
        let shift = inline_layout.element_shift(bg_span.owner, &get) as f64;

        let on = lines_of_span(&ranges, bg_span.start, bg_span.end, true);
        note_span_line_visits(on.len());
        for (line, line_edges) in on.filter_map(|i| Some((layout.get(i)?, &edges[i]))) {
            let line_range = line.text_range();

            // The element's edge boxes on this line, as `(x, width)`.
            let (mut start_box, mut end_box) = (None, None);
            for &(o, end, x, w) in line_edges {
                if o == bg_span.owner {
                    let slot = if end { &mut end_box } else { &mut start_box };
                    *slot = Some((x, w));
                }
            }

            // The element's text on this line, as its glyphs' extent.
            let mut text_extent = None;
            if bg_span.end > bg_span.start
                && line_range.end > bg_span.start
                && line_range.start < bg_span.end
            {
                let overlap_start = bg_span.start.max(line_range.start);
                let overlap_end = bg_span.end.min(line_range.end);
                let end_byte = overlap_end.saturating_sub(1).max(overlap_start);
                if let (Some(sc), Some(ec)) = (
                    Cluster::from_byte_index(layout, overlap_start),
                    Cluster::from_byte_index(layout, end_byte),
                ) && let (Some(x_start), Some(ec_offset)) =
                    (sc.visual_offset(), ec.visual_offset())
                {
                    let x_end = ec_offset + ec.advance();
                    text_extent = Some((x_start.min(x_end) as f64, x_start.max(x_end) as f64));
                }
            }
            if start_box.is_none() && end_box.is_none() && text_extent.is_none() {
                continue;
            }

            // Each edge box without its margin, which is on its outer side:
            // the left of a left-to-right start box and of a right-to-left
            // end box.
            let inner = |(x, w): (f64, f64), outer_left: bool| {
                if outer_left {
                    (x + margin_left, x + w)
                } else {
                    (x, x + w - margin_right)
                }
            };
            let mut pieces: Vec<(f64, f64)> = Vec::with_capacity(3);
            pieces.extend(text_extent);
            pieces.extend(start_box.map(|b| inner(b, ltr)));
            pieces.extend(end_box.map(|b| inner(b, !ltr)));
            let left = pieces.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
            let right = pieces.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
            let (closes_left, closes_right) = if ltr {
                (start_box.is_some(), end_box.is_some())
            } else {
                (end_box.is_some(), start_box.is_some())
            };

            let line_metrics = line.metrics();
            let baseline = line_metrics.baseline as f64;
            let ascent = crate::ifc::line_ascent(line_metrics) as f64;
            let descent = crate::ifc::line_descent(line_metrics) as f64;
            let pt = bg_span.padding_top as f64;
            let pb = bg_span.padding_bottom as f64;

            // The border box: the content area grown by the padding and the
            // border (those of the sides it closes are in its edge boxes).
            let rect = Rect::new(
                left * scale,
                (baseline - ascent - pt - bt + shift) * scale,
                right.max(left) * scale,
                (baseline + descent + pb + bb + shift) * scale,
            );
            f(InlineBoxFragment {
                span: bg_span,
                rect,
                closes_left,
                closes_right,
            });
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
            let (mid, amp) = wavy_wave(&line, scale);
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

// A wavy underline's geometry, in CSS px scaled with everything else.
const PERIOD: f64 = 5.0;
const AMPLITUDE: f64 = 1.0;
const THICKNESS: f64 = 1.0;

/// Where [`paint_wavy_decorations`] draws a line's wave: its centre line and
/// its amplitude, in device px relative to the content origin. The stroke
/// reaches `amp` and half a [`THICKNESS`] either side of the centre.
fn wavy_wave(line: &parley::layout::Line<'_, Brush>, scale: f64) -> (f64, f64) {
    let sf = scale as f32;
    let metrics = line.metrics();
    let baseline = (metrics.baseline * sf) as f64;
    // Parley's `underline_offset` is measured **up** from the baseline (the
    // sign `render_text` subtracts with), and is per-run, so take it from the
    // line's first run and fall back to a fraction of the descent for a line
    // that somehow has none.
    let offset = line
        .runs()
        .next()
        .map(|r| r.font_metrics().underline_offset)
        .unwrap_or(-crate::ifc::line_descent(metrics) * 0.4);
    // Sit the *top* of the wave where a straight underline would be, so a
    // squiggle never rides up into the glyphs.
    let mid = baseline - (offset * sf) as f64 + AMPLITUDE * scale;
    (mid, AMPLITUDE * scale)
}

thread_local! {
    /// (span, line) pairs the inline-ink walks looked at
    /// ([`inline_background_rects`], [`inline_ink_past_lines`]), folded into
    /// a tree's `inline_span_line_visits` counter by [`take_span_line_visits`].
    static SPAN_LINE_VISITS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The (span, line) pairs the inline-ink walks looked at on this thread since
/// the last call, which it resets.
pub(crate) fn take_span_line_visits() -> u64 {
    SPAN_LINE_VISITS.with(|v| v.replace(0))
}

fn note_span_line_visits(n: usize) {
    if n > 0 {
        SPAN_LINE_VISITS.with(|v| v.set(v.get() + n as u64));
    }
}

/// The indices of the lines (whose text ranges are `ranges`, in order) that a
/// span over bytes `start..end` is on: those whose range overlaps it, and
/// with `edges`, also those that merely touch it at either end — where its
/// edge boxes, which take no byte, can sit (#1426). A binary search, so a
/// span costs the lines it is on and not every line of the paragraph (#1837
/// review: one text edit in a paragraph of 500 pills took 500 ms).
fn lines_of_span(
    ranges: &[std::ops::Range<usize>],
    start: usize,
    end: usize,
    edges: bool,
) -> std::ops::Range<usize> {
    let first = if edges {
        ranges.partition_point(|r| r.end < start)
    } else {
        ranges.partition_point(|r| r.end <= start)
    };
    let past = first
        + ranges[first..].partition_point(|r| if edges { r.start <= end } else { r.start < end });
    first..past
}

/// A line's horizontal reach, in CSS px from the content origin: its line box
/// (`inline_min_coord..inline_max_coord`) and its content where that is
/// placed (from `offset`, `advance` wide). The two differ for a line wider
/// than its box — the alignment width a paragraph that does not wrap is
/// given is the box's (#1237, review of #1868), and an unbreakable word
/// overflows a wrapping one — and for a right-to-left line offset left by
/// its trailing white space.
fn line_reach(m: &parley::layout::LineMetrics) -> (f64, f64) {
    let start = (m.inline_min_coord + m.offset) as f64;
    (
        (m.inline_min_coord as f64).min(start),
        (m.inline_max_coord as f64).max(start + m.advance as f64),
    )
}

/// What an IFC root draws past its **unshifted** line boxes, in device px
/// relative to its content origin, or `None` when it draws nothing there:
/// text moved off its line by `vertical-align` (#724), the inline
/// elements' backgrounds and borders ([`inline_background_rects`], which
/// reach past the line by their padding and border and move with their
/// text), and wavy underlines ([`paint_wavy_decorations`]).
///
/// The clip proof (`layer_bounds`, #1762, #1817) unions this with the line
/// boxes, and the damage ink of the root (`paint::ink_outsets_in`, #1815)
/// grows its box by it, so both measure the geometry paint draws with. Like
/// the line boxes, a shifted line is its whole line box moved by the shift
/// (the glyphs' own ink is not measured). `get` looks a node up.
pub(crate) fn inline_ink_past_lines<'a>(
    get: impl Fn(crate::node::RawNodeId) -> Option<&'a Node>,
    inline_layout: &crate::node::InlineLayout,
    scale: f64,
) -> Option<Rect> {
    if inline_layout.vertical_align_spans.is_empty()
        && inline_layout.background_spans.is_empty()
        && inline_layout.decoration_spans.is_empty()
    {
        return None;
    }
    let layout = &inline_layout.layout;
    let mut ink: Option<Rect> = None;
    let mut add = |r: Rect| ink = Some(ink.map_or(r, |i| i.union(r)));
    let ranges: Vec<std::ops::Range<usize>> = if inline_layout.vertical_align_spans.is_empty()
        && inline_layout.decoration_spans.is_empty()
    {
        Vec::new()
    } else {
        layout.lines().map(|l| l.text_range()).collect()
    };

    for span in &inline_layout.vertical_align_spans {
        let shift = span.shift_px as f64;
        if shift == 0.0 {
            continue;
        }
        let on = lines_of_span(&ranges, span.start, span.end, false);
        note_span_line_visits(on.len());
        for line in on.filter_map(|i| layout.get(i)) {
            let m = line.metrics();
            let top = m.block_min_coord.min(m.content_block_min_coord) as f64;
            let bottom = m.block_max_coord.max(m.content_block_max_coord) as f64;
            let (x0, x1) = line_reach(m);
            add(Rect::new(
                x0 * scale,
                (top + shift) * scale,
                x1 * scale,
                (bottom + shift) * scale,
            ));
        }
    }

    if !inline_layout.background_spans.is_empty() {
        inline_background_rects(&get, inline_layout, scale, |frag| add(frag.rect));
    }

    for span in &inline_layout.decoration_spans {
        let on = lines_of_span(&ranges, span.start, span.end, false);
        note_span_line_visits(on.len());
        for line in on.filter_map(|i| layout.get(i)) {
            // The wave runs under the span's clusters, which are on the
            // line: its extent is the line's, plus the stroke's half width.
            let m = line.metrics();
            let (x0, x1) = line_reach(m);
            let (mid, amp) = wavy_wave(&line, scale);
            let reach = amp + THICKNESS * scale / 2.0;
            let half = THICKNESS * scale / 2.0;
            add(Rect::new(
                x0 * scale - half,
                mid - reach,
                x1 * scale + half,
                mid + reach,
            ));
        }
    }
    ink
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
    /// Bytes no range covers cast the root's list. The "…" of a
    /// `text-overflow: ellipsis` cut is drawn apart and casts the root's
    /// list too (#1065, #1100).
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
                    let mut mask = TextMask {
                        ranges: mask_ranges,
                        default_hidden,
                        shown_span,
                        limits: Vec::new(),
                    };
                    mask.cut(inline_layout);
                    Self {
                        shadows: list,
                        mask,
                    }
                })
                .collect(),
        )
    }
}

/// One text fragment — one text node's stretch of one line — with the
/// `text-shadow` list it casts and a mask that shows only its own text
/// (#1060).
///
/// Chrome paints an inline formatting context fragment by fragment, each
/// one's shadows and then its text, so a later fragment's shadow reaching back
/// is drawn over earlier text: a span's, and one line's over the line above
/// (Chrome 155, measured). Within one fragment every shadow is under the text.
/// Chrome also splits a fragment at a bidi boundary and orders a line's
/// fragments visually; these fragments do neither (#1675).
/// The one-pass order — every shadow, then all of the text — is kept unless a
/// later fragment's shadow can change a pixel of an earlier fragment's ink
/// (text or decorations), or (with shadows of more than one colour) of its
/// shadows, by more than [`super::text_shadow::ORDER_TOLERANCE_LEVELS`]. That
/// is about order only: the one-pass path blurs a list's fragments as one
/// mask, so where adjacent blurs overlap it differs from the per-fragment
/// composite by up to about 10 levels either way (as on main);
/// only then are the fragments drawn one by one, which costs a shadow pass
/// and a text pass per fragment (a glyph run two fragments share is drawn in
/// two calls, and a blurred shadow is a mask per fragment, as wide as the
/// fragment).
pub(super) struct TextFragment<'a> {
    pub(super) shadows: &'a [TextShadowValue],
    pub(super) mask: TextMask,
}

/// How many fragment pairs [`TextFragment::in_paint_order`] compares before
/// it stops looking and draws fragment by fragment, which is always right.
const FRAGMENT_PAIR_BUDGET: usize = 65_536;

/// A text range as [`TextFragment::in_paint_order`] reads it.
struct FragmentRange<'a> {
    start: usize,
    end: usize,
    hidden: bool,
    shadows: &'a [TextShadowValue],
    colour: AlphaColor<Srgb>,
}

/// One shown fragment: its bytes, its list, its text colour and the box of
/// its ink-bearing clusters.
struct Piece<'a> {
    start: usize,
    end: usize,
    shadows: &'a [TextShadowValue],
    colour: AlphaColor<Srgb>,
    /// Whether an underline, line-through or wavy underline is drawn across
    /// it: ink in a colour of its own, so a shadow over it is compared with
    /// every colour.
    decorated: bool,
    text: Option<Rect>,
}

/// [`super::text_shadow::visible_reach`] by `(blur, alpha)`, for one
/// [`TextFragment::in_paint_order`]: a page holds few distinct shadows and
/// colours, and the reach is a root search.
#[derive(Default)]
struct ReachMemo(std::cell::RefCell<Vec<(u64, u32, f64)>>);

impl ReachMemo {
    fn get(&self, blur: f64, alpha: f32) -> f64 {
        let key = (blur.to_bits(), alpha.to_bits());
        let mut memo = self.0.borrow_mut();
        if let Some(&(_, _, r)) = memo.iter().find(|m| (m.0, m.1) == key) {
            return r;
        }
        let r = super::text_shadow::visible_reach(blur, alpha);
        if memo.len() < 64 {
            memo.push((key.0, key.1, r));
        }
        r
    }
}

/// The colour a `text-shadow` is drawn in ([`render_text_shadows`]'s).
fn shadow_colour(s: &TextShadowValue) -> AlphaColor<Srgb> {
    s.color
        .unwrap_or_else(|| AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 255))
}

/// The box within which shadow `s` of text boxed by `text` can change a
/// pixel of something in colour `over` (any colour when `None`) by more than
/// [`super::text_shadow::ORDER_TOLERANCE_LEVELS`]:
/// moved by its offset, grown by its visible reach. `None` when it can change
/// no pixel of that colour by more than that at all — the shadow painted over a pixel of colour
/// `over` or under it differs by at most its alpha times the colours'
/// largest channel difference.
fn shadow_rect(
    reaches: &ReachMemo,
    s: &TextShadowValue,
    text: Rect,
    over: Option<AlphaColor<Srgb>>,
) -> Option<Rect> {
    let c = shadow_colour(s);
    let diff = over.map_or(1.0, |o| {
        (0..3)
            .map(|k| (c.components[k] - o.components[k]).abs())
            .fold(0.0_f32, f32::max)
    });
    let alpha = c.components[3] * diff;
    if alpha as f64 * 255.0 <= super::text_shadow::ORDER_TOLERANCE_LEVELS {
        return None;
    }
    let reach = reaches.get(s.blur_radius.max(0.0) as f64, alpha);
    Some((text + Vec2::new(s.offset_x as f64, s.offset_y as f64)).inflate(reach, reach))
}

impl<'a> TextFragment<'a> {
    /// `inline_layout`'s shown text fragments in paint order — lines in order,
    /// each line's fragments in text order — or `None` when the one-pass order
    /// paints the same pixels: no shadow at all, a single fragment, or no
    /// fragment's shadow overlapping an earlier fragment's ink.
    ///
    /// The overlap is decided on boxes: a fragment's text is the advance
    /// extent of its ink-bearing clusters (a space only under a decoration)
    /// by its line's ascent and descent, moved with any `vertical-align`
    /// shift; its shadow is that box moved by each offset and grown by the
    /// reach at which the blur can still change a pixel of the colour under it
    /// by more than the order tolerance ([`shadow_rect`]). Ink past a glyph's advance or below the
    /// descent (an italic overhang) is not in the box, so a
    /// shadow touching only that keeps the one-pass order.
    pub(super) fn in_paint_order(
        tree: &'a NodeTree,
        inline_layout: &crate::node::InlineLayout,
        root_shadows: &'a [TextShadowValue],
        root_hidden: bool,
        root_colour: AlphaColor<Srgb>,
    ) -> Option<Vec<Self>> {
        // Asked of every IFC on every paint: answer the common cases without
        // allocating — no shadow anywhere, or one line holding one text node.
        if root_shadows.is_empty() && !tree.inline_text_shadows {
            return None;
        }
        let layout = &inline_layout.layout;
        if layout.len() <= 1
            && inline_layout
                .text_ranges
                .iter()
                .filter(|r| !r.is_br)
                .count()
                <= 1
        {
            return None;
        }
        if root_shadows.is_empty()
            && inline_layout
                .text_ranges
                .iter()
                .filter(|r| !r.is_br)
                .all(|r| {
                    range_element(tree, r).is_none_or(|e| e.computed_style.text_shadow.is_empty())
                })
        {
            return None;
        }
        let mut ranges: Vec<FragmentRange<'a>> = inline_layout
            .text_ranges
            .iter()
            .filter(|r| !r.is_br)
            .map(|r| {
                let el = range_element(tree, r);
                FragmentRange {
                    start: r.flat_start,
                    end: r.flat_end,
                    hidden: el.map(is_hidden).unwrap_or(root_hidden),
                    shadows: el.map_or(root_shadows, |e| e.computed_style.text_shadow.as_slice()),
                    colour: crate::ifc::text_color(&tree.nodes, r.node_id),
                }
            })
            .collect();
        ranges.sort_by_key(|r| r.start);

        // Each shown fragment: its byte range, its list, its text box.
        let mut pieces: Vec<Piece<'a>> = Vec::new();
        let styles = layout.styles();
        // A space has ink only through a decoration drawn across it.
        let wavy = !inline_layout.decoration_spans.is_empty();
        let mut next = 0;
        for line in layout.lines() {
            let lr = line.text_range();
            let first = pieces.len();
            while next < ranges.len() && ranges[next].end <= lr.start {
                next += 1;
            }
            let mut push = |start: usize, end: usize, r: &FragmentRange<'a>| {
                if start < end && !r.hidden {
                    pieces.push(Piece {
                        start,
                        end,
                        shadows: r.shadows,
                        colour: r.colour,
                        decorated: false,
                        text: None,
                    });
                }
            };
            let root_range = FragmentRange {
                start: 0,
                end: 0,
                hidden: root_hidden,
                shadows: root_shadows,
                colour: root_colour,
            };
            let (mut at, mut i) = (lr.start, next);
            while at < lr.end {
                match ranges.get(i) {
                    Some(r) if r.start < lr.end => {
                        if r.start > at {
                            push(at, r.start, &root_range);
                        }
                        push(r.start.max(at), r.end.min(lr.end), r);
                        at = r.end.max(at);
                        i += 1;
                    }
                    _ => {
                        push(at, lr.end, &root_range);
                        at = lr.end;
                    }
                }
            }
            // One pass over the line's clusters gives every fragment on it
            // the extent of its ink-bearing clusters: a walk per fragment
            // re-walks a glyph run its fragments share, quadratic in spans
            // on one line.
            let line_pieces = &mut pieces[first..];
            if line_pieces.is_empty() {
                continue;
            }
            let m = line.metrics();
            let (top, bottom) = (
                m.content_block_min_coord as f64,
                m.content_block_max_coord as f64,
            );
            for run in line.runs() {
                let Some(mut x) = run.visual_clusters().next().and_then(|c| c.visual_offset())
                else {
                    continue;
                };
                for cluster in run.visual_clusters() {
                    let byte = cluster.text_range().start;
                    let advance = cluster.advance() as f64;
                    let decorated = (cluster.glyphs().next().is_some()
                        && styles
                            .get(usize::from(cluster.style_index()))
                            .is_some_and(|st| {
                                st.underline.is_some() || st.strikethrough.is_some()
                            }))
                        || (wavy
                            && inline_layout
                                .decoration_spans
                                .iter()
                                .any(|d| d.start <= byte && byte < d.end));
                    let inked = !cluster.is_space_or_nbsp() || decorated;
                    let k = line_pieces.partition_point(|p| p.end <= byte);
                    if inked
                        && let Some(p) = line_pieces.get_mut(k)
                        && p.start <= byte
                    {
                        p.decorated |= decorated;
                        // A `vertical-align` shift moves the glyphs off the
                        // line's ascent and descent (#724; the line does not
                        // grow for it, #1357): the box goes with them.
                        let shift = if inline_layout.vertical_align_spans.is_empty() {
                            0.0
                        } else {
                            inline_layout.vertical_align_shift_at(byte) as f64
                        };
                        let r =
                            Rect::new(x as f64, top + shift, x as f64 + advance, bottom + shift);
                        p.text = Some(p.text.map_or(r, |q| q.union(r)));
                    }
                    x += cluster.advance();
                }
            }
        }
        if pieces.len() <= 1 {
            return None;
        }

        let first = pieces
            .iter()
            .flat_map(|p| p.shadows)
            .next()
            .map(shadow_colour);
        let one_colour = pieces
            .iter()
            .flat_map(|p| p.shadows)
            .all(|s| Some(shadow_colour(s)) == first);
        let overlaps =
            |a: Rect, b: Rect| a.x0.max(b.x0) < a.x1.min(b.x1) && a.y0.max(b.y0) < a.y1.min(b.y1);
        let reaches = ReachMemo::default();
        // The most a fragment's shadows can reach: each one's box at the
        // reach where it can change a pixel of any colour.
        let shadow_max = |p: &Piece<'_>| {
            let text = p.text?;
            p.shadows
                .iter()
                .filter_map(|s| shadow_rect(&reaches, s, text, None))
                .reduce(|a, b| a.union(b))
        };
        let inks: Vec<(Option<Rect>, Option<Rect>)> =
            pieces.iter().map(|p| (p.text, shadow_max(p))).collect();
        // An earlier fragment can be hit only if its ink reaches below the
        // shadow's top: scanning back, stop where no earlier ink does.
        let mut reach_below = Vec::with_capacity(inks.len());
        let mut lowest = f64::NEG_INFINITY;
        for &(text, shadow) in &inks {
            for r in text.into_iter().chain(shadow.filter(|_| !one_colour)) {
                lowest = lowest.max(r.y1);
            }
            reach_below.push(lowest);
        }
        // Whether drawing `later`'s shadows over `earlier` changes a pixel:
        // over its text, the shadow painted over a glyph or under it differs
        // by at most the shadow's coverage times its alpha times the two
        // colours' difference; over its shadows, by the same against that
        // shadow's colour.
        let reaches_back = |later: &Piece<'_>, earlier: &Piece<'_>| {
            let (Some(text), Some(earlier_text)) = (later.text, earlier.text) else {
                return false;
            };
            later.shadows.iter().any(|s| {
                shadow_rect(
                    &reaches,
                    s,
                    text,
                    (!earlier.decorated).then_some(earlier.colour),
                )
                .is_some_and(|r| overlaps(r, earlier_text))
                    || (!one_colour
                        && earlier.shadows.iter().any(|e| {
                            shadow_rect(&reaches, s, text, Some(shadow_colour(e))).is_some_and(
                                |r| {
                                    shadow_rect(&reaches, e, earlier_text, None)
                                        .is_some_and(|er| overlaps(r, er))
                                },
                            )
                        }))
            })
        };
        let mut compared = 0usize;
        let changes_pixels = 'search: {
            for (j, &(_, shadow)) in inks.iter().enumerate() {
                let Some(shadow) = shadow else { continue };
                for i in (0..j).rev() {
                    if reach_below[i] <= shadow.y0 {
                        break;
                    }
                    let (text, earlier_shadow) = inks[i];
                    if !(text.is_some_and(|t| overlaps(shadow, t))
                        || (!one_colour && earlier_shadow.is_some_and(|s| overlaps(shadow, s))))
                    {
                        continue;
                    }
                    compared += 1;
                    if compared > FRAGMENT_PAIR_BUDGET || reaches_back(&pieces[j], &pieces[i]) {
                        break 'search true;
                    }
                }
            }
            false
        };
        changes_pixels.then(|| {
            pieces
                .into_iter()
                .map(|p| {
                    let mut mask = TextMask {
                        ranges: vec![(p.start, p.end, false)],
                        default_hidden: true,
                        shown_span: Some((p.start, p.end)),
                        limits: Vec::new(),
                    };
                    mask.cut(inline_layout);
                    Self {
                        shadows: p.shadows,
                        mask,
                    }
                })
                .collect()
        })
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
/// made glyph by glyph (see [`GlyphCursor`]). Bytes no range covers take
/// the IFC root's visibility. What a `text-overflow: ellipsis` cut hides is
/// hidden too ([`Self::cut`], #1100).
pub(super) struct TextMask {
    /// `(start, end, hidden)` per text range, in layout byte offsets.
    ranges: Vec<(usize, usize, bool)>,
    /// The answer for bytes no range covers.
    default_hidden: bool,
    /// The byte span every shown byte lies in, when that is narrower than the
    /// whole text: a line outside it draws nothing and is not walked
    /// ([`Self::may_show`]). `None` means any line may.
    shown_span: Option<(usize, usize)>,
    /// The lines an ellipsis cut hides everything of past an x
    /// ([`ellipsis_line_limits`]): a walk of their clusters stops there.
    limits: Vec<(usize, f32)>,
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
        if !root_hidden && inline_layout.ellipsis.is_none() && ranges.iter().all(|&(_, _, h)| !h) {
            return None;
        }
        // Already in text order; the sort is a linear pass that keeps
        // `hidden_at`'s binary search honest if that ever changes.
        ranges.sort_by_key(|&(s, _, _)| s);
        let mut mask = Self {
            ranges,
            default_hidden: root_hidden,
            shown_span: None,
            limits: Vec::new(),
        };
        mask.cut(inline_layout);
        Some(mask)
    }

    /// Hide, as well, what `inline_layout`'s `text-overflow: ellipsis` cut
    /// hides (#1100): its bytes are split out of the ranges that hold them
    /// and marked hidden, and those no range holds are added hidden. Every
    /// mask an IFC paint draws through takes the cut, so each pass — text,
    /// decorations, shadows, wavy underlines — stops at it.
    pub(super) fn cut(&mut self, inline_layout: &crate::node::InlineLayout) {
        let Some(cut) = inline_layout.ellipsis.as_ref() else {
            return;
        };
        self.limits = ellipsis_line_limits(inline_layout);
        if cut.hidden.is_empty() {
            return;
        }
        let mut out: Vec<(usize, usize, bool)> =
            Vec::with_capacity(self.ranges.len() + 2 * cut.hidden.len());
        // The ranges, split by the cut: both lists are sorted and disjoint.
        let mut c = 0;
        for &(s, e, h) in &self.ranges {
            while c < cut.hidden.len() && cut.hidden[c].end <= s {
                c += 1;
            }
            let mut at = s;
            let mut k = c;
            while at < e && k < cut.hidden.len() && cut.hidden[k].start < e {
                let r = &cut.hidden[k];
                if r.start > at {
                    out.push((at, r.start, h));
                }
                let end = r.end.min(e);
                out.push((r.start.max(at), end, true));
                at = end;
                if r.end <= e {
                    k += 1;
                } else {
                    break;
                }
            }
            if at < e {
                out.push((at, e, h));
            }
        }
        // The cut's bytes no range holds.
        let mut i = 0;
        for r in &cut.hidden {
            let mut at = r.start;
            while i < self.ranges.len() && self.ranges[i].1 <= at {
                i += 1;
            }
            let mut j = i;
            while at < r.end {
                match self.ranges.get(j) {
                    Some(&(s, e, _)) if s < r.end => {
                        if s > at {
                            out.push((at, s, true));
                        }
                        at = at.max(e);
                        j += 1;
                    }
                    _ => {
                        out.push((at, r.end, true));
                        at = r.end;
                    }
                }
            }
        }
        out.retain(|&(s, e, _)| s < e);
        out.sort_by_key(|&(s, _, _)| s);
        self.ranges = out;
    }

    /// Whether anything in the byte range `range` (a line's) may be shown.
    ///
    /// A shadow group's mask shows only its own text (#1048), so each group's
    /// pass skips the lines its text is not on. Without that, a paragraph of
    /// N spans with N distinct lists cost N passes over every line: 298 ms at
    /// N = 400 (review of #1063, before [`PaintText`] collected each line's
    /// items once per paint, #1066).
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
    /// Whether every byte of `range` is shown and lies in one recorded
    /// range: the whole of a glyph run a fragment's mask holds, measured
    /// without a walk of its clusters.
    pub(super) fn shows_all(&self, range: std::ops::Range<usize>) -> bool {
        let i = self.ranges.partition_point(|&(_, e, _)| e <= range.start);
        matches!(self.ranges.get(i), Some(&(s, e, false)) if s <= range.start && range.end <= e)
    }

    /// The x past which everything on line `line` is hidden by an ellipsis
    /// cut, if any.
    pub(super) fn limit_for_line(&self, line: usize) -> Option<f32> {
        limit_for_line(&self.limits, line)
    }

    pub(super) fn hidden_at(&self, byte: usize) -> bool {
        let i = self.ranges.partition_point(|&(_, e, _)| e <= byte);
        match self.ranges.get(i) {
            Some(&(s, _, h)) if s <= byte => h,
            _ => self.default_hidden,
        }
    }
}

/// `(line index, x)` for each line of `inline_layout` an ellipsis cut hides
/// everything of past `x` in visual order (a cut at the line's right,
/// #1100), sorted by line. By index, not by byte: parley's line and run text
/// ranges wrap past 65,535 bytes (#1862's text).
pub(super) fn ellipsis_line_limits(inline_layout: &crate::node::InlineLayout) -> Vec<(usize, f32)> {
    inline_layout
        .ellipsis
        .as_ref()
        .map_or_else(Vec::new, |cut| {
            cut.marks
                .iter()
                .filter_map(|m| m.hidden_past.map(|x| (m.line, x)))
                .collect()
        })
}

/// The limit of [`ellipsis_line_limits`] for line `line`.
fn limit_for_line(limits: &[(usize, f32)], line: usize) -> Option<f32> {
    limits
        .binary_search_by_key(&line, |&(l, _)| l)
        .ok()
        .map(|i| limits[i].1)
}

/// `run`'s clusters in visual order, up to the first that starts past
/// `limit` — everything from there on is hidden by an ellipsis cut, so a
/// paint does not walk it (review of #1860: walking it made a paint of long
/// cut lines grow with the hidden text).
pub(super) fn shown_clusters<'r>(
    run: &'r parley::layout::Run<'r, Brush>,
    limit: Option<f32>,
) -> impl Iterator<Item = parley::layout::Cluster<'r, Brush>> + 'r {
    let mut x = match limit {
        Some(_) => run
            .visual_clusters()
            .next()
            .and_then(|c| c.visual_offset())
            .unwrap_or(0.0),
        None => 0.0,
    };
    run.visual_clusters().take_while(move |c| {
        let x0 = x;
        x += c.advance();
        limit.is_none_or(|l| x0 <= l)
    })
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
    /// The line's ellipsis limit: the walk stops where [`PaintLine::collect`]
    /// stopped collecting (#1100).
    limit: Option<f32>,
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
            for cluster in shown_clusters(run, self.limit) {
                let hidden = mask.hidden_at(cluster.text_range().start);
                self.glyphs.extend(
                    cluster
                        .glyphs()
                        .map(|_| (usize::from(cluster.style_index()), hidden)),
                );
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
                self.glyphs.extend(
                    cluster
                        .glyphs()
                        .map(|_| (usize::from(cluster.style_index()), shift)),
                );
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
/// Bytes no range covers were shaped in the root's own colour and follow
/// that. The "…" of a `text-overflow: ellipsis` cut is drawn apart, in the
/// root's colour as it is now (#1100).
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
                self.glyphs.extend(
                    cluster
                        .glyphs()
                        .map(|_| (usize::from(cluster.style_index()), recolour)),
                );
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
    text: &PaintText<'_>,
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
        text,
        transform,
        scale,
        mask,
        color_brush.as_ref(),
        None,
        valign,
        None,
    );
}

/// A text layout as a paint walks it: each line's positioned items and their
/// glyphs, collected once per line and shared by every pass that draws the
/// line (#1066).
///
/// Parley's own walk is quadratic in the glyph runs of a parley `Run`:
/// `Line::items` re-walks the run's clusters from its start for every glyph
/// run it yields, and so does `GlyphRun::glyphs`. A paragraph's spans of one
/// font are one `Run` per line, split into glyph runs by style, so a line of
/// many styled spans cost glyphs × glyph runs per walk — and an IFC paint
/// walks it once per pass: every `text-shadow` list a span casts of its own
/// (#1048), every text fragment (#1060), every blurred mask's key and
/// rasterisation. Here each glyph run's glyphs are sliced out of one walk of
/// its run's clusters, split by style index exactly as `GlyphRunIter` splits
/// them.
///
/// What is kept is bounded by what the passes need to share (review of #1700:
/// keeping every line copied a 8000-line `<pre>`'s glyphs, 12 MB, on every
/// paint). A line is kept only when a **masked** pass of a [`Self::share`]d
/// text draws it — a span's own list, a fragment: the passes that are many —
/// and an unmasked pass reuses a kept line or walks the line into one reused
/// buffer. A text drawn by a fixed number of unmasked passes ([`Self::new`])
/// keeps nothing, and pays the walk per pass as before.
pub(super) struct PaintText<'a> {
    layout: &'a parley::layout::Layout<Brush>,
    shared: std::cell::Cell<bool>,
    /// The lines kept, by index.
    kept: std::cell::RefCell<std::collections::HashMap<usize, PaintLine<'a>>>,
    /// The line an unkept walk is collected into, reused line after line.
    scratch: std::cell::RefCell<Option<PaintLine<'a>>>,
    /// One parley run's glyphs, reused by every collection.
    run_glyphs: std::cell::RefCell<Vec<(u16, parley::layout::Glyph)>>,
    glyph_count: std::cell::OnceCell<usize>,
    /// [`ellipsis_line_limits`]: a line's glyphs past its cut are not
    /// collected.
    limits: Vec<(usize, f32)>,
}

/// One line of a [`PaintText`].
pub(super) struct PaintLine<'a> {
    /// The line's index in the layout.
    pub(super) index: usize,
    pub(super) line: parley::layout::Line<'a, Brush>,
    /// The line's items in Parley's order, each glyph run with the range of
    /// its glyphs in `glyphs`.
    pub(super) items: Vec<(
        parley::layout::PositionedLayoutItem<'a, Brush>,
        std::ops::Range<usize>,
    )>,
    glyphs: Vec<parley::layout::Glyph>,
}

impl<'a> PaintLine<'a> {
    /// The line's glyph runs, in order, each with its glyphs.
    pub(super) fn glyph_runs(
        &self,
    ) -> impl Iterator<
        Item = (
            &parley::layout::GlyphRun<'a, Brush>,
            &[parley::layout::Glyph],
        ),
    > + '_ {
        self.items.iter().filter_map(|(item, range)| match item {
            parley::layout::PositionedLayoutItem::GlyphRun(run) => {
                Some((run, &self.glyphs[range.clone()]))
            }
            parley::layout::PositionedLayoutItem::InlineBox(_) => None,
        })
    }

    /// Collect `line`'s items and glyphs into `self`, reusing its buffers.
    fn collect(
        &mut self,
        index: usize,
        line: parley::layout::Line<'a, Brush>,
        run_glyphs: &mut Vec<(u16, parley::layout::Glyph)>,
        limit: Option<f32>,
    ) {
        LINE_WALKS.with(|w| w.set(w.get() + 1));
        self.index = index;
        let reference = REFERENCE_GLYPHS.with(std::cell::Cell::get);
        self.line = line;
        self.items.clear();
        self.glyphs.clear();
        // The parley run the last glyph run came from, and where its next
        // glyph run starts in `run_glyphs`.
        let mut key = None;
        let mut next = 0;
        for item in self.line.items() {
            let start = self.glyphs.len();
            if let parley::layout::PositionedLayoutItem::GlyphRun(glyph_run) = &item {
                if reference {
                    self.glyphs.extend(glyph_run.glyphs());
                } else {
                    let run = glyph_run.run();
                    let k = (run.index(), run.cluster_range().start);
                    if key != Some(k) {
                        key = Some(k);
                        next = 0;
                        run_glyphs.clear();
                        // parley 0.12 keeps a style index per cluster, not per glyph:
                        // every glyph of a cluster has the cluster's style.
                        run_glyphs.extend(shown_clusters(run, limit).flat_map(|c| {
                            let style = c.style_index();
                            c.glyphs().map(move |g| (style, g))
                        }));
                        GLYPHS_COLLECTED.with(|g| g.set(g.get() + run_glyphs.len() as u64));
                    }
                    let rest = run_glyphs.get(next..).unwrap_or(&[]);
                    let count = rest.first().map_or(0, |&(first, _)| {
                        rest.iter()
                            .take_while(|&&(style, _)| style == first)
                            .count()
                    });
                    self.glyphs.extend(rest[..count].iter().map(|&(_, g)| g));
                    next += count;
                }
            }
            let end = self.glyphs.len();
            self.items.push((item, start..end));
        }
    }
}

thread_local! {
    /// How many times paint collected a line's items ([`PaintLine::collect`]).
    static LINE_WALKS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// How many glyphs paint collected out of its runs ([`PaintLine::collect`]).
    static GLYPHS_COLLECTED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Collect each glyph run's glyphs with Parley's own `GlyphRun::glyphs`
    /// rather than slicing one walk of the run (#1066). For the oracle test.
    static REFERENCE_GLYPHS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// How many times paint has walked a line's positioned items on this thread
/// since the last call, which it resets. For tests (#1066).
#[doc(hidden)]
pub fn take_text_line_walks() -> u64 {
    LINE_WALKS.with(|w| w.replace(0))
}

/// How many glyphs paint has collected out of parley runs on this thread
/// since the last call, which it resets. A line an ellipsis cut ends is
/// collected only up to its cut (#1100). For tests.
#[doc(hidden)]
pub fn take_text_glyphs_collected() -> u64 {
    GLYPHS_COLLECTED.with(|g| g.replace(0))
}

/// Collect glyph runs' glyphs through Parley's own `GlyphRun::glyphs`, the
/// walk used before #1066, on this thread until set back. Returns the
/// previous setting. For the pixel oracle.
#[doc(hidden)]
pub fn set_reference_text_walk(on: bool) -> bool {
    REFERENCE_GLYPHS.with(|r| r.replace(on))
}

impl<'a> PaintText<'a> {
    /// A text drawn by a fixed number of unmasked passes: nothing is kept.
    pub(super) fn new(layout: &'a parley::layout::Layout<Brush>) -> Self {
        Self {
            layout,
            shared: std::cell::Cell::new(false),
            kept: Default::default(),
            scratch: Default::default(),
            run_glyphs: Default::default(),
            glyph_count: Default::default(),
            limits: Vec::new(),
        }
    }

    /// An IFC's text: a line an ellipsis cut ends is walked only up to its
    /// cut (#1100).
    pub(super) fn of_ifc(inline_layout: &'a crate::node::InlineLayout) -> Self {
        let mut text = Self::new(&inline_layout.layout);
        text.limits = ellipsis_line_limits(inline_layout);
        text
    }

    /// Mark the text as drawn by a pass per span list or per fragment: from
    /// now on, the lines a masked pass draws are kept for the passes after it.
    pub(super) fn share(&self) {
        self.shared.set(true);
    }

    /// The layout itself.
    pub(super) fn layout(&self) -> &'a parley::layout::Layout<Brush> {
        self.layout
    }

    /// Call `f` with each line `mask` may show ([`TextMask::may_show`]), in
    /// order.
    pub(super) fn for_each_line(&self, mask: Option<&TextMask>, mut f: impl FnMut(&PaintLine<'a>)) {
        let shared = self.shared.get();
        let keep = shared && mask.is_some();
        for (index, line) in self.layout.lines().enumerate() {
            if mask.is_some_and(|m| !m.may_show(line.text_range())) {
                continue;
            }
            if keep || (shared && self.kept.borrow().contains_key(&index)) {
                if !self.kept.borrow().contains_key(&index) {
                    let mut collected = PaintLine {
                        index,
                        line: line.clone(),
                        items: Vec::new(),
                        glyphs: Vec::new(),
                    };
                    collected.collect(
                        index,
                        line,
                        &mut self.run_glyphs.borrow_mut(),
                        limit_for_line(&self.limits, index),
                    );
                    self.kept.borrow_mut().insert(index, collected);
                }
                f(&self.kept.borrow()[&index]);
            } else {
                let mut scratch = self.scratch.borrow_mut();
                let scratch = scratch.get_or_insert_with(|| PaintLine {
                    index,
                    line: line.clone(),
                    items: Vec::new(),
                    glyphs: Vec::new(),
                });
                scratch.collect(
                    index,
                    line,
                    &mut self.run_glyphs.borrow_mut(),
                    limit_for_line(&self.limits, index),
                );
                f(scratch);
            }
        }
    }

    /// How many glyphs the whole text draws, counted once.
    pub(super) fn glyph_count(&self) -> usize {
        *self.glyph_count.get_or_init(|| {
            let mut n = 0;
            self.for_each_line(None, |line| {
                n += line.glyph_runs().map(|(_, g)| g.len()).sum::<usize>();
            });
            n
        })
    }
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
pub(super) fn draw_text(
    painter: &mut dyn Painter,
    text: &PaintText<'_>,
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
    text.for_each_line(mask, |line| {
        let mut cursor = GlyphCursor {
            limit: mask.and_then(|m| m.limit_for_line(line.index)),
            ..Default::default()
        };
        let mut vcursor = ValignCursor::default();
        let mut ccursor = ColourCursor::default();
        for (glyph_run, run_glyphs) in line.glyph_runs() {
            // Before the hidden test's `continue`: the cursor hands out a
            // run's glyph runs in order and has to see each of them.
            let recolours = live.and_then(|l| ccursor.take(glyph_run, l));
            let shifts = run_shifts(&mut vcursor, glyph_run, valign);
            let flags = run_flags(&mut cursor, glyph_run, mask);
            // A glyph run past an ellipsis cut has no glyphs collected.
            if run_glyphs.is_empty() || flags.as_ref().is_some_and(|f| f.iter().all(|&h| h)) {
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

            let run_metrics = run.font_metrics();
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
                    bytemuck::cast_slice(run.normalized_coords()),
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
            for (i, glyph) in run_glyphs.iter().enumerate() {
                let start_x = gx;
                let shift = shifts.as_ref().map(|s| s[i]).unwrap_or(0.0) * sf;
                let px = gx + glyph.x * sf;
                let py = gy + glyph.y * sf + shift;
                gx += glyph.advance * sf;
                if flags
                    .as_ref()
                    .is_some_and(|f| f.get(i).copied().unwrap_or(true))
                {
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
    });
}

/// One copy of a shadow: [`draw_text`] and the wavy underlines, all in `brush`.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_shadow_copy(
    painter: &mut dyn Painter,
    text: &PaintText<'_>,
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
        text,
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
    let text = PaintText::new(layout);
    render_text_shadows(
        painter,
        &text,
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
        &text,
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
    text: &PaintText<'_>,
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
            text,
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
