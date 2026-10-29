//! Inline Formatting Context (IFC): Parley-based inline layout for text and inline elements.

use std::collections::HashMap;

use peniko::Brush;

use crate::RinchDocument;
use crate::computed_style::VerticalAlignValue;
use crate::layout;
use crate::node::{
    DisplayMode, InlineFlowRole, InlineLayout, LayoutResult, Node, NodeContext, NodeKind,
};

/// What the post-layout tree-check sweep may fail on, and what it may not.
///
/// See [`RinchDocument::tree_check_verdict`], which is the only thing that
/// builds one.
#[derive(Debug, Default, Clone)]
pub struct TreeCheckVerdict {
    /// Violations the sweep fails on: every Taffy-tree line and every DOM-tree
    /// line. There is no waived arm any more — see `tree_check_verdict` for the
    /// one it had while #513 and then #591 were open, and for how to bring one
    /// back if a future open defect needs it.
    pub fatal: Vec<String>,
}

/// The IFC root's own text style, which the `text-overflow: ellipsis` "…"
/// is shaped in (#1100).
pub(crate) struct EllipsisStyle {
    font_size: f32,
    font_family: std::borrow::Cow<'static, str>,
    font_weight: parley::style::FontWeight,
    font_style: parley::style::FontStyle,
    color: peniko::Color,
    line_height: Option<parley::style::LineHeight>,
    letter_spacing: f32,
    word_spacing: f32,
    underline: bool,
    strikethrough: bool,
    decoration_color: Option<peniko::Color>,
}

/// The colour an IFC root's own style gives its text: the default brush of
/// its layout, and of the flat `text-overflow: ellipsis` rebuild.
pub(crate) fn root_text_color(cs: &crate::computed_style::ComputedStyle) -> peniko::Color {
    cs.color.unwrap_or(peniko::Color::BLACK)
}

/// The colour a text node's glyphs are drawn in: its element's `color`.
///
/// A text node has no style of its own. `walk_inline_children` pushes each
/// inline element's colour as a span around its text, a styled `display:
/// contents` wrapper's (#574) and the DOM parent's for a member of an
/// anonymous box, so in every case the brush a run gets is its text node's DOM
/// parent's colour — which is what this answers, at the build (recorded as
/// [`crate::node::IfcTextRange::color`]) and again at every paint (#679).
pub(crate) fn text_color(nodes: &slab::Slab<Node>, text_node: usize) -> peniko::Color {
    let mut cur = nodes.get(text_node).and_then(|t| t.parent);
    while let Some(node) = cur.and_then(|id| nodes.get(id)) {
        if let Some(color) = node.computed_style.color {
            return color;
        }
        cur = node.parent;
    }
    peniko::Color::BLACK
}

/// The colour of an element's wavy underline: its `text-decoration-color`,
/// or its `color` for `currentcolor`.
pub(crate) fn wavy_underline_color(cs: &crate::computed_style::ComputedStyle) -> peniko::Color {
    cs.text_decoration
        .color
        .or(cs.color)
        .unwrap_or(peniko::Color::BLACK)
}

/// Whether an inline element has a background to draw: a colour that is not
/// fully transparent. The one test [`inline_background_span`] makes, asked
/// on its own by the transition and animation ticks (#679).
pub(crate) fn has_inline_background(cs: &crate::computed_style::ComputedStyle) -> bool {
    cs.background_color().is_some_and(|c| c.components[3] > 0.0)
}

/// The bit an **edge box**'s parley id carries (#1426).
///
/// parley 0.11.1 has no per-span padding, so the room a `display: inline`
/// element's inline-start margin, border and padding take on its first line,
/// and its inline-end ones on its last, is an `InlineBox` of that width and
/// no height pushed before and after its content
/// ([`IfcText::push_edge_box`]). Such a box holds no node: its id is this bit,
/// [`EDGE_BOX_END`] for the end box, and the owner's node id, all inside 32
/// bits so that `id as usize` cannot name a real node on a 32-bit target
/// either. Every walker of a layout's inline boxes that maps a box to a node
/// asks [`is_edge_box`] first.
pub const EDGE_BOX: u64 = 1 << 31;
/// Set with [`EDGE_BOX`] on an element's inline-end box.
pub const EDGE_BOX_END: u64 = 1 << 30;

/// A line's typographic extent above its baseline: the tallest ascent of its
/// glyphs and the part of its inline boxes over the baseline. parley 0.12
/// dropped `LineMetrics::ascent` (linebender/parley#697); its
/// `content_block_min_coord` is the top of that extent.
pub(crate) fn line_ascent(m: &parley::LineMetrics) -> f32 {
    m.baseline - m.content_block_min_coord
}

/// The top of what a line covers: its line box's (`block_min_coord`), or its
/// glyphs' when they reach higher — under a negative half-leading, where they
/// stand out of a line box shorter than they are (Chrome highlights and
/// hit-tests that content area). The glyphs are the line's text runs, each
/// its font's ascent above the baseline: what parley 0.11.1 folded into
/// `block_min_coord`. Not 0.12's `content_block_min_coord`, which also holds
/// the strut — the block's own font — so a 16px run in a 32px block reached
/// 8px above its line (`review_1587_tests`, `span32`).
pub(crate) fn line_top(line: &parley::layout::Line<'_, Brush>) -> f32 {
    let m = line.metrics();
    // Rounded as parley 0.11.1 rounded them in a quantized layout (every
    // IFC's), so a highlight stays on whole pixels.
    line.runs()
        .map(|run| m.baseline - run.font_metrics().ascent.round())
        .fold(m.block_min_coord, f32::min)
}

/// [`line_top`]'s counterpart at the bottom: the line box's, or its runs'
/// descent below the baseline.
pub(crate) fn line_bottom(line: &parley::layout::Line<'_, Brush>) -> f32 {
    let m = line.metrics();
    line.runs()
        .map(|run| m.baseline + run.font_metrics().descent.round())
        .fold(m.block_max_coord, f32::max)
}

/// [`line_ascent`]'s counterpart below the baseline (`LineMetrics::descent`
/// before parley 0.12).
pub(crate) fn line_descent(m: &parley::LineMetrics) -> f32 {
    m.content_block_max_coord - m.baseline
}

/// The id of the zero-size out-of-flow box an IFC with no content at all
/// (no text, no inline box) is handed: parley 0.12 gives a layout with no
/// items one line holding the root's strut, kept for a caret, where CSS 2.1
/// §9.4.2 makes such a line box zero-height (Chrome 153: a block holding only
/// collapsible spaces, a comment, an empty `<span>` or an out-of-flow box is
/// 0 tall). With one item that is no content parley makes the line invisible.
/// It reads as an edge box ([`is_edge_box`]) of an owner no node has, so
/// every walker skips it.
pub const EMPTY_IFC_BOX: u64 = EDGE_BOX | EDGE_BOX_END | (EDGE_BOX_END - 1);

/// Whether an inline box id is an edge box ([`EDGE_BOX`]) rather than an
/// atomic inline's node id.
pub fn is_edge_box(id: u64) -> bool {
    id & EDGE_BOX != 0
}

/// The owner of an edge box and whether it is the inline-end one.
pub(crate) fn edge_box_owner(id: u64) -> Option<(usize, bool)> {
    is_edge_box(id).then_some(((id & (EDGE_BOX_END - 1)) as usize, id & EDGE_BOX_END != 0))
}

/// An edge box's id ([`EDGE_BOX`]).
pub(crate) fn edge_box_id(owner: usize, end: bool) -> u64 {
    debug_assert!((owner as u64) < EDGE_BOX_END);
    EDGE_BOX | if end { EDGE_BOX_END } else { 0 } | (owner as u64 & (EDGE_BOX_END - 1))
}

/// The room a `display: inline` element takes before its content on its
/// first line and after it on its last (#1426): its inline-start and
/// inline-end margin + border + padding, in CSS px, `(start, end)`. With
/// `ltr` false the start is the right side. A percentage margin or padding is
/// taken as 0 (its basis is the containing block's width, which the IFC does
/// not know when it is built), and a negative total as 0.
pub(crate) fn inline_edge_widths(
    cs: &crate::computed_style::ComputedStyle,
    ltr: bool,
) -> (f32, f32) {
    use crate::computed_style::BorderStyleValue as S;
    use crate::computed_style::LengthPercentageAutoValue as M;
    let margin = |m: M| match m {
        M::Length(px) => px,
        _ => 0.0,
    };
    let border = |w: f32, s: S| {
        if matches!(s, S::None | S::Hidden) {
            0.0
        } else {
            w
        }
    };
    let left = margin(cs.margin_left)
        + border(cs.border_left_width.to_px(), cs.border_left_style)
        + cs.padding_left.to_px();
    let right = margin(cs.margin_right)
        + border(cs.border_right_width.to_px(), cs.border_right_style)
        + cs.padding_right.to_px();
    let (start, end) = if ltr { (left, right) } else { (right, left) };
    (start.max(0.0), end.max(0.0))
}

/// Whether an element's own direction is left to right.
pub(crate) fn element_is_ltr(node: &Node) -> bool {
    node.stylo_element_data
        .borrow()
        .as_ref()
        .and_then(|d| d.styles.get_primary().map(|p| p.writing_mode.is_bidi_ltr()))
        .unwrap_or(true)
}

/// Whether an inline element draws a border of its own on its fragments.
pub(crate) fn has_inline_border(cs: &crate::computed_style::ComputedStyle) -> bool {
    use crate::computed_style::BorderStyleValue as S;
    let on = |w: f32, s: S| w > 0.0 && !matches!(s, S::None | S::Hidden);
    on(cs.border_left_width.to_px(), cs.border_left_style)
        || on(cs.border_right_width.to_px(), cs.border_right_style)
        || on(cs.border_top_width.to_px(), cs.border_top_style)
        || on(cs.border_bottom_width.to_px(), cs.border_bottom_style)
}

/// The background span inline element `owner` draws over `start..end`, from
/// its style as it is now; `None` when it has neither a background nor a
/// border to draw (a border alone gives a span of transparent colour, #1426).
///
/// Called at the build, which records one span per element that has one, and
/// by paint for each recorded span, so the rectangle follows a colour,
/// padding or radius written with no re-shape (#679).
pub(crate) fn inline_background_span(
    owner: &Node,
    start: usize,
    end: usize,
) -> Option<crate::node::InlineBackgroundSpan> {
    let cs = &owner.computed_style;
    let color = cs
        .background_color()
        .filter(|_| has_inline_background(cs))
        .or_else(|| has_inline_border(cs).then_some(peniko::Color::TRANSPARENT))?;
    Some(crate::node::InlineBackgroundSpan {
        owner: owner.id,
        start,
        end,
        color,
        padding_left: cs.padding_left.to_px(),
        padding_right: cs.padding_right.to_px(),
        padding_top: cs.padding_top.to_px(),
        padding_bottom: cs.padding_bottom.to_px(),
        border_radius: cs.border_radius_top_left.to_px(),
    })
}

impl EllipsisStyle {
    pub(crate) fn of(cs: &crate::computed_style::ComputedStyle, scale: f32) -> Self {
        Self {
            font_size: cs.font_size * scale,
            font_family: if cs.font_family.is_empty() {
                "sans-serif".into()
            } else {
                cs.font_family.clone().into()
            },
            font_weight: parley::style::FontWeight::new(cs.font_weight),
            font_style: cs.font_style.to_parley(),
            color: root_text_color(cs),
            line_height: cs.line_height.to_parley(),
            letter_spacing: cs.letter_spacing,
            word_spacing: cs.word_spacing,
            underline: cs.text_decoration.underline && !cs.text_decoration.is_wavy_underline(),
            strikethrough: cs.text_decoration.strikethrough,
            decoration_color: cs.text_decoration.color,
        }
    }

    /// `text` shaped in this style, not yet broken into lines. `paint` adds
    /// what only the drawn layout needs (brush, line height, decorations);
    /// a measurement leaves them out.
    pub(crate) fn shape(
        &self,
        layout_cx: &mut parley::LayoutContext<Brush>,
        font_cx: &mut parley::FontContext,
        text: &str,
        scale: f32,
        paint: bool,
    ) -> parley::layout::Layout<Brush> {
        use parley::style::StyleProperty as P;
        let font_family = crate::fonts::parley_text_family(font_cx, &self.font_family, text);
        let mut b = layout_cx.ranged_builder(font_cx, text, scale, true);
        b.push_default(P::FontSize(self.font_size));
        b.push_default(P::FontWeight(self.font_weight));
        b.push_default(P::FontStyle(self.font_style));
        font_family.push_to(&mut b);
        push_spacing(&mut b, self.letter_spacing, self.word_spacing);
        if paint {
            b.push_default(P::Brush(Brush::Solid(self.color)));
            if let Some(lh) = self.line_height {
                b.push_default(P::LineHeight(lh));
            }
            b.push_default(P::Underline(self.underline));
            b.push_default(P::Strikethrough(self.strikethrough));
            if let Some(c) = self.decoration_color {
                b.push_default(P::UnderlineBrush(Some(Brush::Solid(c))));
                b.push_default(P::StrikethroughBrush(Some(Brush::Solid(c))));
            }
        }
        b.build(text)
    }
}

/// Push the inherited `letter-spacing` / `word-spacing` onto a ranged builder,
/// in CSS pixels (#698).
///
/// The `text-overflow: ellipsis` rebuild shapes the ellipsis glyph, each
/// binary-search prefix of a whole-text cut and the final truncated layout,
/// and all of them have to shape the text the way the untruncated line was
/// shaped, or the prefix that "fits" is measured under one set of advances and
/// painted under another. They all go through [`EllipsisStyle::shape`], so
/// they cannot drift apart.
///
/// Unconditional, like the sibling pushes in
/// [`RinchDocument::inline_style_props`]: 0 is parley's own default, so a zero
/// push costs nothing and there is no guard to forget.
fn push_spacing(b: &mut parley::RangedBuilder<'_, Brush>, letter_spacing: f32, word_spacing: f32) {
    b.push_default(parley::style::StyleProperty::LetterSpacing(letter_spacing));
    b.push_default(parley::style::StyleProperty::WordSpacing(word_spacing));
}

/// The width a paragraph that does not wrap is aligned within (#1237): the
/// root's content width, or, for an auto-width root (`up_to`, the width its
/// lines are broken at, is the content width plus `ifc_paint_max_width`'s
/// pixel of slack) whose paragraph is within a pixel of that width, the
/// paragraph's own width. A root sized from its text (shrink-to-fit:
/// `inline-block`, an absolute box, `max-content`, `fit-content`) is as wide
/// as its widest line rounded to a whole pixel, and its lines are aligned
/// within the widest line itself, as in Chrome 155 (review of #1868). Not
/// Chrome's: an auto-width block that stretches to a width its text happens
/// to be within a pixel of is aligned within the text's width too (off by
/// that fraction). For a declared width `up_to` is `width`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AlignWidth {
    /// The content-box width.
    pub width: f32,
    /// The width the paint layout is broken at.
    pub up_to: f32,
}

/// What [`break_lines_hanging_spaces`] had to do to one layout, for the
/// `ifc_hang_*` perf counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HangStats {
    /// Whole-paragraph re-breaks: 0 when no line needed its spaces hung or
    /// a hung NBSP undone (#1218), else exactly 1 however many lines did.
    pub passes: u32,
    /// Lines broken a second time: at a widened width to keep their spaces,
    /// or to undo a hung NBSP ([`unglue`]).
    pub lines: u32,
    /// Extra breaks of the whole paragraph to drop parley's empty line after
    /// an overflowing last inline box (#1050) or a final forced break
    /// (#1172): 0 or 1.
    pub phantom_rebreaks: u32,
    /// Breaks of one line [`unglue`] made to move a break parley put after
    /// a hung NBSP (#1218): a few per such line, logarithmic in the length
    /// of a glued chain.
    pub unglue_rebreaks: u32,
    /// Builds of a paragraph after the first, made to put its preserved
    /// tabs at their stops (#1122): one for a line of tabs that do not wrap,
    /// at most [`MAX_TAB_BUILDS`]` - 1`.
    pub tab_rebuilds: u32,
    /// Lines [`css_line`] was asked about (#1426, #1238, #1243): one scan of
    /// the line's units each, whether or not the line was broken again.
    pub css_scans: u32,
}

impl HangStats {
    /// Sum `other` into these, field by field.
    pub(crate) fn add(&mut self, other: HangStats) {
        self.passes += other.passes;
        self.lines += other.lines;
        self.phantom_rebreaks += other.phantom_rebreaks;
        self.unglue_rebreaks += other.unglue_rebreaks;
        self.tab_rebuilds += other.tab_rebuilds;
        self.css_scans += other.css_scans;
    }

    /// Add these to `perf`'s `ifc_hang_*` counters.
    pub(crate) fn record(&self, perf: &crate::perf::PerfCounters) {
        perf.add(crate::perf::Counter::IfcHangPasses, u64::from(self.passes));
        perf.add(crate::perf::Counter::IfcHangLines, u64::from(self.lines));
        perf.add(
            crate::perf::Counter::IfcPhantomRebreaks,
            u64::from(self.phantom_rebreaks),
        );
        perf.add(
            crate::perf::Counter::IfcUnglueRebreaks,
            u64::from(self.unglue_rebreaks),
        );
        perf.add(
            crate::perf::Counter::IfcTabRebuilds,
            u64::from(self.tab_rebuilds),
        );
        perf.add(
            crate::perf::Counter::IfcCssLineScans,
            u64::from(self.css_scans),
        );
    }
}

/// Break `layout` into lines at `max_width`, then commit again the lines
/// parley ends where CSS does not.
///
/// Since parley 0.12 the white space itself is parley's: preserved spaces at
/// a soft wrap hang, those before a forced break or at the end hang only as
/// far as they overflow, a `break-spaces` space never hangs
/// ([`white_space_hanging`]), an NBSP never does, and a justified line's
/// opportunities are CSS's. rinch's own fix-ups for those in 0.11.1 — a
/// hang pass for preserved spaces, NBSPs hung at a soft wrap, alignment
/// widths narrowed by trailing white space that does not hang (#1042) and
/// justification opportunities recounted (#1056) — are gone.
///
/// What is left is a second break of the paragraph ([`hang_pass`]) when a line
/// may end where CSS does not end it: next to an inline element's edge box
/// (#1426), at a seam between atomic inlines or `pre` spaces (#1238, #1243),
/// right after an inline box parley placed by its emergency branch (#1081),
/// at an overflowing NBSP ([`unglue`], #1218) and in overflowing
/// `break-spaces` spaces (#1043). It follows each line's end through a table
/// of the paragraph's clusters and inline boxes in logical order, advanced by
/// each committed line's advance ([`line_end`]); if that ever disagrees with
/// the breaker, fixing stops for the rest of the paragraph and it is broken
/// exactly as parley breaks it. A paragraph needing none of it costs the one
/// ordinary break and a scan of its lines.
///
/// And whatever the white space, a paragraph that ends in the empty line parley
/// commits after an overflowing inline box or a final forced break
/// ([`phantom_last_line`], #1050, #1172) is broken once more, the same way,
/// without it.
///
/// And with `align_width`, a paragraph broken unconstrained (`max_width` is
/// `None`: none of its text may wrap) is aligned within that width ([`AlignWidth`]) rather
/// than within its own widest line, which is what parley does with no width
/// (#1237): it is the line box CSS aligns a line in. Its lines are those of
/// the unconstrained break — nothing wraps, not even after an inline box,
/// where parley would break a `nowrap` paragraph given a width (#1238) — and
/// each line's alignment width is set to `align_width` as it is committed
/// ([`break_unwrapped`]). A line wider than that overflows and parley starts
/// it at the start edge, as CSS does. The caller passes `align_width` only
/// for an alignment that moves a line from its start in a left-to-right
/// block; parley infers the paragraph's direction from its text, so a
/// right-to-left one is then still lined up by parley's rules for it.
///
/// [`set_prior_line_width`]: parley::layout::BreakLines::set_prior_line_width
pub(crate) fn break_lines_hanging_spaces(
    layout: &mut parley::Layout<Brush>,
    text: &str,
    max_width: Option<f32>,
    preserves_spaces: bool,
    unhung: &UnhungSpaces,
    align_width: Option<AlignWidth>,
) -> HangStats {
    // #1237: a paragraph that does not wrap, aligned within the box.
    if let Some(align) =
        align_width.filter(|a| max_width.is_none() && a.width.is_finite() && a.width > 0.0)
    {
        return break_unwrapped_aligned(layout, text, align, preserves_spaces, unhung);
    }
    layout.break_all_lines(max_width);
    let max = max_width.filter(|max| max.is_finite());
    // #1243: whether the seams are in force for `pre` spaces depends on
    // where parley broke the paragraph.
    unhung
        .pre_seams
        .set(unhung.pre_candidate && max.is_some() && pre_seams_needed(layout, text, unhung));
    let fix = max.filter(|_| {
        any_emergency_box_line_before_white_space(layout, text)
            || any_line_split_at_edge(layout)
            || unhung.active_seams().is_some()
    });
    let mut stats = match fix {
        Some(max) => hang_pass(layout, text, max, None, unhung),
        None => HangStats::default(),
    };
    // #1050, #1172: parley's trailing empty line after an overflowing inline
    // box or a final forced break. Broken again, the same way, up to the line
    // before it.
    if let Some(keep) = phantom_last_line(layout) {
        match fix {
            Some(max) => stats = hang_pass(layout, text, max, Some(keep), unhung),
            None => break_lines_up_to(layout, max_width, keep),
        }
        stats.phantom_rebreaks = 1;
    }
    stats
}

/// How many times `resolve_layout` runs `resolve_percentage_inline_blocks`
/// and the root compute after it, at most (#1476). A pass is followed by
/// another only when it found a min-content request, so the rounds end when
/// no measure asks any more; the limit is a guard against a cycle, not a
/// budget. A layout that stops at it is not a fixpoint: the next one
/// carries on.
pub(crate) const MAX_INLINE_ROUNDS: u32 = 64;

/// Whether an atomic inline whose max-content width is `max` fits a line
/// `stretch` wide — the first half of its shrink-to-fit width
/// `min(max-content, max(min-content, stretch))` (#658). See
/// `RinchDocument::resolve_root_width_keyword` for the pixel of tolerance.
pub(crate) fn atomic_inline_fits(max: f32, stretch: f32) -> bool {
    max <= stretch + 1.0
}

/// The second half: the width a box that does not fit `stretch` is capped
/// at, or `None` when that is within a pixel of its max-content width (it
/// then keeps that).
pub(crate) fn atomic_inline_capped_width(min: f32, max: f32, stretch: f32) -> Option<f32> {
    let fit = stretch.max(min);
    (fit + 1.0 < max).then_some(fit)
}

/// Whether an atomic inline's width is the plain shrink-to-fit of its
/// content, which an IFC measure can work out for any line width from the
/// box's min- and max-content sizes (#1476): an `auto` width, no percentage
/// `min-`/`max-width` (whose basis is the containing block), and margins that
/// are lengths. Returns the horizontal margins.
pub(crate) fn plain_shrink_to_fit_margins(
    style: &crate::computed_style::ComputedStyle,
) -> Option<f32> {
    use crate::computed_style::DimensionValue::{Auto, Percent};
    use crate::computed_style::LengthPercentageAutoValue as M;
    if !matches!(style.width, Auto)
        || matches!(style.min_width, Percent(_))
        || matches!(style.max_width, Percent(_))
    {
        return None;
    }
    let margin = |m: M| match m {
        M::Auto => Some(0.0),
        M::Length(px) => Some(px),
        _ => None,
    };
    Some(margin(style.margin_left)? + margin(style.margin_right)?)
}

/// What an IFC **measure** needs to size the atomic inlines on its lines
/// (#1476).
///
/// The paint layout lines an atomic inline up at the size it has
/// (`Node::layout`). A measure answers a question about a width the box may
/// never be laid out at — "how narrow can this be", "how wide would it like
/// to be", "how tall is it at 400px" — and CSS answers each from what the box
/// *contributes* at that width: its min-content size, its max-content size,
/// or its shrink-to-fit size `min(max-content, max(min-content, width))`.
/// Lined up at its current size instead, a box made every container sized
/// from that answer (a flex item, a grid `auto` track, a shrink-to-fit
/// absolute box, another `auto` atomic inline) as wide as itself, so the cap
/// `resolve_percentage_inline_blocks` puts on it could never bind — and once
/// one was capped, the container stayed that narrow when there was room
/// again.
///
/// Only for a box whose width is the plain shrink-to-fit of its content
/// ([`plain_shrink_to_fit_margins`]); any other is lined up at the size it
/// has, as before.
pub(crate) struct AtomicContributions<'a> {
    /// The width the measure breaks lines at: `None` for max-content,
    /// `Some(0.0)` for min-content.
    pub wrap_width: Option<f32>,
    /// `NodeTree::atomic_min_content`.
    pub min_content: &'a HashMap<usize, f32>,
    /// `NodeTree::keyword_inline_cb_width`: the containing-block width each
    /// resolved box was last resolved against.
    pub resolved_at: &'a HashMap<usize, f32>,
    /// `NodeTree::atomic_min_requests`.
    pub requests: &'a std::cell::RefCell<Vec<usize>>,
}

impl AtomicContributions<'_> {
    /// The size to line atomic inline `id` up at.
    fn size(&self, id: usize, node: &Node) -> (f32, f32) {
        let layout = (node.layout.width, node.layout.height);
        // Where the box was last resolved, if it has been since it was last
        // measured as `auto`. Only then can it have another size than its
        // natural one, and only then is the recorded one current: a box
        // that became an atomic inline by a restyle has never been measured
        // as one, and the size it was laid out at stands.
        let resolved_at = if self.resolved_at.is_empty() {
            None
        } else {
            self.resolved_at.get(&id).copied()
        };
        let natural = match resolved_at {
            Some(_) => node.natural_inline_size,
            None => layout,
        };
        // A box that has the width asked for is lined up as it is: its
        // height is then the real one (a box laid out again around a capped
        // one inside it keeps its width and grows).
        let at = |width: f32, height: f32| {
            if (width - layout.0).abs() <= 0.5 {
                layout
            } else {
                (width, height)
            }
        };
        let Some(wrap) = self.wrap_width else {
            // Max-content: the box's own max-content size, whatever it has
            // been capped at since.
            return if natural == layout
                || plain_shrink_to_fit_margins(&node.computed_style).is_none()
            {
                layout
            } else {
                at(natural.0, natural.1)
            };
        };
        // Resolved against exactly this width: the size it has is the answer
        // (and the one the paint layout will use).
        if resolved_at.is_some_and(|w| (w - wrap).abs() < 0.01) {
            // Capped: its margin box, as below.
            return match plain_shrink_to_fit_margins(&node.computed_style) {
                Some(margins) if layout.0 + 0.5 < natural.0 => (layout.0 + margins, layout.1),
                _ => layout,
            };
        }
        let Some(margins) = plain_shrink_to_fit_margins(&node.computed_style) else {
            return layout;
        };
        let stretch = (wrap - margins).max(0.0);
        if atomic_inline_fits(natural.0, stretch) {
            return at(natural.0, natural.1);
        }
        let Some(&min_width) = self.min_content.get(&id) else {
            // Not measured yet, and a measure cannot start a compute: line
            // it up at its max-content size, as before #1476, and ask.
            self.requests.borrow_mut().push(id);
            return at(natural.0, natural.1);
        };
        // A capped box is lined up at its **margin box**: the line is then
        // as wide as what the box takes of it, so a container sized from
        // this answer leaves the box the width it was capped at here once
        // its margins come out again. (A box that fits is lined up without
        // them, as the paint layout lines every box up.)
        match atomic_inline_capped_width(min_width, natural.0, stretch) {
            None => at(natural.0, natural.1),
            // Its height at a width it has not been laid out at is not
            // known; the one it has stands in until
            // `resolve_percentage_inline_blocks` lays it out there — and is
            // the answer when it already has that width. Not the height of
            // its min-content compute, which breaks its text at every
            // opportunity: at its min-content *width* the short words share
            // lines (review of #1488).
            //
            // At the width its layout will round that to (a box laid out as
            // a root is `round(width)` wide), which is what the branch above
            // answers once it has been: a measure at this width then answers
            // the same before and after, so a container sized from it does
            // not move by the rounding between an incremental layout and a
            // fresh one, and is not a pixel narrower than the box it holds
            // (reviews of #1510).
            Some(fit) => (fit.round() + margins, layout.1),
        }
    }
}

/// Where the first line of `layout` sits its baseline, from the top of the
/// layout — the content-box top of the box it was measured for. `None` when it
/// laid out no line.
pub(crate) fn first_line_baseline(layout: &parley::layout::Layout<Brush>) -> Option<f32> {
    layout.lines().next().map(|line| line.metrics().baseline)
}

/// Its last line's baseline, from the layout's top (`None` when it has no
/// line): an `inline-block`'s baseline is its last line box's (CSS 2.1
/// §10.8.1, #663).
pub(crate) fn last_line_baseline(layout: &parley::layout::Layout<Brush>) -> Option<f32> {
    layout.lines().last().map(|line| line.metrics().baseline)
}

/// Atomic inline `id`'s baseline from its border-box top, read from the
/// Taffy layout its detached compute just made (#663): what parley 0.12 lines
/// the box up by (`InlineBox::baseline`) against the line's strut.
///
/// - an `inline-flex` / `inline-grid` takes its **first** baseline, the first
///   in-flow item's (css-flexbox-1 §8.5, css-grid-1 §11.8; approximated: no
///   baseline alignment groups, no reordering);
/// - any other atomic inline (an `inline-block`, a `<button>`) its **last**
///   line box's, found through its last in-flow block child that has one, or
///   none — lined up by its bottom edge — when it has no line box or its
///   `overflow` is neither `visible` nor `clip` (CSS 2.1 §10.8.1);
/// - a replaced element and a form control with no line of its own, none.
pub(crate) fn atomic_inline_baseline(tree: &crate::node::NodeTree, id: usize) -> Option<f32> {
    use crate::computed_style::{DisplayValue, OverflowValue};
    let node = tree.nodes.get(id)?;
    let taffy_id = node.taffy_id?;
    let style = &node.computed_style;
    let first = matches!(
        style.display,
        DisplayValue::InlineFlex | DisplayValue::InlineGrid
    );
    let clips = |o: OverflowValue| !matches!(o, OverflowValue::Visible | OverflowValue::Clip);
    if let Some(NodeContext::FormControl { .. }) = tree.taffy.get_node_context(taffy_id) {
        let height = tree.taffy.unrounded_layout(taffy_id).size.height;
        return crate::form_control::input_baseline(node, height);
    }
    if !first && (clips(style.overflow_x) || clips(style.overflow_y)) {
        return None;
    }
    let r = taffy_baseline(tree, taffy_id, first, 0);
    if std::env::var_os("ZZ_PROBE_BASELINE").is_some() {
        eprintln!(
            "ZZ atomic_inline_baseline({id}) = {r:?} lines={:?}",
            tree.atomic_line_baselines
        );
    }
    r
}

/// [`atomic_inline_baseline`]'s walk over the Taffy subtree at `taffy_id`:
/// its first (`first`) or last line's baseline, from its border-box top.
fn taffy_baseline(
    tree: &crate::node::NodeTree,
    taffy_id: taffy::NodeId,
    first: bool,
    depth: usize,
) -> Option<f32> {
    // Deep enough for any real tree; a cycle cannot happen, a pathological
    // nesting just lines the box up by its bottom.
    if depth > 256 {
        return None;
    }
    let taffy = &tree.taffy;
    let layout = taffy.unrounded_layout(taffy_id);
    let top = layout.padding.top + layout.border.top;
    let leaf = match taffy.get_node_context(taffy_id) {
        Some(NodeContext::InlineRoot(root)) => Some(*root),
        Some(NodeContext::Text(text)) => Some(text.node_id),
        _ => None,
    };
    if let Some(leaf) = leaf {
        let width = layout.size.width
            - layout.padding.left
            - layout.padding.right
            - layout.border.left
            - layout.border.right;
        // The measure at the width it has; the closest one when Taffy clamped
        // the measured width (a box `380` wide whose line measured 380.86),
        // the newest of equally close ones.
        let height = layout.size.height
            - layout.padding.top
            - layout.padding.bottom
            - layout.border.top
            - layout.border.bottom;
        let entry = tree
            .atomic_line_baselines
            .get(&leaf)?
            .iter()
            .rev()
            .min_by(|a, b| {
                let off = |e: &(f32, f32, Option<f32>, Option<f32>)| {
                    (e.0 - width).abs() + (e.1 - height).abs()
                };
                off(a).total_cmp(&off(b))
            })?;
        // A last line keeps its distance from the content bottom when the
        // nearest measure was of another height (Taffy answered the layout
        // from a cache the records no longer match exactly).
        return if first {
            entry.2
        } else {
            entry.3.map(|b| height - (entry.1 - b))
        }
        .map(|b| top + b);
    }
    let count = taffy::TraversePartialTree::child_count(taffy, taffy_id);
    let in_flow = |child: taffy::NodeId| {
        let style = taffy.style(child).ok();
        style.is_some_and(|s| {
            s.display != taffy::Display::None && s.position != taffy::Position::Absolute
        })
    };
    let at = |i: usize| -> Option<f32> {
        let child = taffy.child_at_index(taffy_id, i).ok()?;
        if !in_flow(child) {
            return None;
        }
        let y = taffy.unrounded_layout(child).location.y;
        taffy_baseline(tree, child, first, depth + 1).map(|b| y + b)
    };
    if first {
        (0..count).find_map(at)
    } else {
        (0..count).rev().find_map(at)
    }
}

/// The `white-space-collapse` parley 0.12 is handed for an element's text:
/// which of its spaces hang at a line's end (CSS Text 4 §4.3). rinch
/// collapses white space itself ([`IfcText`]), so everything parley sees is
/// preserved — `Preserve` (hangs at a soft wrap, and only as far as it
/// overflows before a forced break or the end) — except `break-spaces`, whose
/// spaces never hang (`BreakSpaces`, #1043).
pub(crate) fn white_space_hanging(
    white_space: crate::computed_style::WhiteSpaceValue,
) -> parley::style::WhiteSpaceCollapse {
    match white_space {
        crate::computed_style::WhiteSpaceValue::BreakSpaces => {
            parley::style::WhiteSpaceCollapse::BreakSpaces
        }
        _ => parley::style::WhiteSpaceCollapse::Preserve,
    }
}

/// An atomic inline's `vertical-align` as parley 0.12 lines a box up by it
/// (CSS 2.1 §10.8.1; #663): every keyword, and a length or percentage (of the
/// box's own `line-height`) raising it. Before 0.12 an atomic inline sat on
/// the baseline whatever it declared. A text span's `vertical-align` is still
/// rinch's own glyph shift ([`RinchDocument::vertical_align_shift_px`]).
pub(crate) fn atomic_vertical_align(
    style: &crate::computed_style::ComputedStyle,
    scale: f32,
) -> parley::style::VerticalAlign {
    use parley::style::VerticalAlign as V;
    match style.vertical_align {
        VerticalAlignValue::Baseline => V::BASELINE,
        VerticalAlignValue::Sub => V::SUB,
        VerticalAlignValue::Super => V::SUPER,
        VerticalAlignValue::Top => V::TOP,
        VerticalAlignValue::Bottom => V::BOTTOM,
        VerticalAlignValue::Middle => V::MIDDLE,
        VerticalAlignValue::TextTop => V::TEXT_TOP,
        VerticalAlignValue::TextBottom => V::TEXT_BOTTOM,
        VerticalAlignValue::LengthPercentage(lp) => {
            V::length(lp.resolve(style.line_height_px()) * scale)
        }
    }
}

/// A measure's `LayoutOutput` with its first baseline (#1013).
///
/// `compute_leaf_layout` reports `Baselines::NONE` whatever the measure did, so
/// Taffy synthesized every text leaf's and IFC root's baseline from its bottom
/// border edge, and `align-items: baseline` in a flex or grid row lined items
/// up by their bottoms. Taffy measures a baseline from the **border-box** top
/// (`flexbox.rs` adds only the item's top margin), so the content-box baseline
/// gets the box's top padding and border, resolved exactly as
/// `compute_leaf_layout` resolves them. A block container above the leaf needs
/// nothing: Taffy's block algorithm propagates its first in-flow child's.
pub(crate) fn with_first_baseline(
    mut out: taffy::LayoutOutput,
    inputs: &taffy::LayoutInput,
    style: &taffy::Style,
    content_baseline: Option<f32>,
) -> taffy::LayoutOutput {
    if let Some(baseline) = content_baseline {
        use taffy::{CoreStyle, ResolveOrZero};
        let pw = inputs.parent_size.width;
        let padding = style.padding().resolve_or_zero(pw, |_, _| 0.0);
        let border = style.border().resolve_or_zero(pw, |_, _| 0.0);
        out.baselines.first = Some(padding.top + border.top + baseline);
    }
    out
}

/// A text **leaf**'s text — a flex or grid item's own text, measured through
/// `NodeContext::Text` rather than an IFC — as parley lays it out (#1279,
/// #1672): its white space processed by its element's `white-space` exactly
/// as an IFC processes a text node's ([`IfcText::push_text_node`]: collapsed
/// for `normal`/`nowrap`, spaces collapsed and newlines kept for `pre-line`,
/// a held space at the end removed, U+2028/U+2029/U+0085 substituted). A
/// preserved tab stays a tab: a leaf's tabs are not set to their stops
/// (#1585).
///
/// Offsets into a leaf's layout are offsets into this text, as an IFC
/// root's are into its flat text.
pub(crate) struct LeafText<'a> {
    pub text: std::borrow::Cow<'a, str>,
    /// Its spaces are preserved: only then can they hang or end a line as
    /// content ([`break_lines_hanging_spaces`],
    /// [`crate::node::measured_width_of`]).
    pub preserved: bool,
    /// The `pre` and `break-spaces` text, whose spaces do not hang.
    pub unhung: UnhungSpaces,
}

impl<'a> LeafText<'a> {
    pub(crate) fn of(raw: &'a str, white_space: crate::computed_style::WhiteSpaceValue) -> Self {
        use crate::computed_style::WhiteSpaceValue;
        let mut ifc = IfcText::new(false);
        ifc.push_text_node(
            std::borrow::Cow::Borrowed(raw),
            SpaceCollapse::of(white_space),
        );
        ifc.finish();
        let preserved = ifc.preserved();
        let tabs = ifc.tab_offsets();
        let mut text = ifc.into_text();
        if !tabs.is_empty() {
            let t = text.to_mut();
            for at in tabs {
                t.replace_range(at..at + 1, "\t");
            }
        }
        let mut unhung = UnhungSpaces::default();
        if !text.is_empty() {
            match white_space {
                WhiteSpaceValue::Pre => unhung.pre.push(0..text.len()),
                WhiteSpaceValue::BreakSpaces => unhung.breaking.push(0..text.len()),
                _ => {}
            }
        }
        Self {
            text,
            preserved,
            unhung,
        }
    }
}

/// A text leaf shaped and broken at `wrap_width`, and the size its measure
/// reports ([`crate::node::measured_width_of`]). The one place a leaf's
/// layout is built: the root compute's measure and an atomic inline's
/// (`measure_inline_blocks`) both call it.
pub(crate) fn shape_text_leaf(
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<Brush>,
    text: &crate::node::TextMeasure,
    wrap_width: Option<f32>,
) -> LeafShape {
    let leaf = LeafText::of(&text.content, text.white_space);
    let font_family = crate::fonts::parley_text_family(font_cx, &text.font_family, &leaf.text);
    let mut builder = layout_cx.ranged_builder(font_cx, &leaf.text, 1.0, true);
    use parley::style::StyleProperty as P;
    builder.push_default(P::FontSize(text.font_size));
    if (text.font_weight - 400.0).abs() > 1.0 {
        builder.push_default(P::FontWeight(parley::style::FontWeight::new(
            text.font_weight,
        )));
    }
    if let Some(lh) = layout::css_line_height_to_parley(&text.line_height_css) {
        builder.push_default(P::LineHeight(lh));
    }
    font_family.push_to(&mut builder);
    // So the cached layout can be rendered with a colour (paint draws a leaf
    // in its parent's current one, #904).
    builder.push_default(P::Brush(Brush::Solid(text.color)));
    // Emergency line breaking.
    builder.push_default(P::OverflowWrap(text.overflow_wrap.to_parley()));
    // letter-/word-spacing (#698). The builder's scale is 1.0, so these are
    // CSS pixels either way.
    builder.push_default(P::LetterSpacing(text.letter_spacing));
    builder.push_default(P::WordSpacing(text.word_spacing));
    builder.push_default(P::WhiteSpaceCollapse(white_space_hanging(text.white_space)));
    let mut layout = builder.build(&leaf.text);
    let hang = break_lines_hanging_spaces(
        &mut layout,
        &leaf.text,
        wrap_width,
        leaf.preserved,
        &leaf.unhung,
        None,
    );
    // A flex or grid container's text that is all white space is not
    // rendered, whatever its `white-space` (css-flexbox-1 §4, css-grid-1
    // §6): Chrome 155 lays `"   "` out as nothing under `pre-wrap` too. Here
    // it measures 0x0 with no baseline, but is still an item (#1760).
    let blank = text
        .content
        .bytes()
        .all(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'));
    let (width, height) = if blank {
        (0.0, 0.0)
    } else {
        let width = crate::node::measured_width_of(
            &layout,
            &leaf.text,
            leaf.preserved,
            wrap_width.unwrap_or(f32::INFINITY),
            &leaf.unhung.breaking,
        );
        (width, layout.height())
    };
    LeafShape {
        layout,
        hang,
        width,
        height,
        blank,
    }
}

/// What [`shape_text_leaf`] answers.
pub(crate) struct LeafShape {
    pub layout: parley::Layout<Brush>,
    pub hang: HangStats,
    /// The width the measure reports.
    pub width: f32,
    /// The height the measure reports.
    pub height: f32,
    /// All white space: measured 0x0, and reports no first baseline, so it
    /// moves no baseline-aligned item. It is still an item (a `gap` is laid
    /// out beside it), where Chrome makes none (#1760).
    pub blank: bool,
}

impl LeafShape {
    /// Its first line's baseline, from the content-box top; `None` when blank.
    pub(crate) fn first_baseline(&self) -> Option<f32> {
        if self.blank {
            None
        } else {
            first_line_baseline(&self.layout)
        }
    }
}

/// The line count to keep when `layout` ends in an empty line parley 0.11.1
/// commits where CSS has no line box: after an inline box it placed by an
/// **emergency** break (#1050), or after a forced break that ends the text
/// (#1172).
///
/// **After an overflowing box.** At the start of a line, a box too wide for
/// any line is consumed and the line committed on the spot
/// (`BreakReason::Emergency`). When that box was the paragraph's last item the
/// breaker is not marked done, so its next `break_next` commits one more line
/// holding nothing. When text came before the box that line still carries an
/// empty text-run item, so the breaker's own rule that an empty last line adds
/// no height does not apply, and it is counted at the box's line height: a
/// 300x80 box wrapped under `"x "` in a 200px paragraph made it 180px tall
/// where Chrome makes it 105. Parley's `main` still has the same branch
/// (checked 2026-09-26).
///
/// **After a final forced break.** A `<br>` is pushed to parley as `"\n"`, and
/// parley always lays out one more line after a text-final newline, with an
/// empty text run in it (its "copy metrics from previous line" hack) — so it
/// is counted too. A line box after the last forced break exists in CSS only
/// when something comes after the break: `<div>a<br></div>` and `<pre>a\n</pre>`
/// are one line in Chrome 153 (rinch made two), `<div><br></div>` one,
/// `<div>a<br><br></div>` two. What "something" is has been settled before
/// parley sees the text: a collapsed space, an empty element and an
/// out-of-flow box put nothing into it ([`IfcText`]), while a preserved space,
/// an NBSP, and any in-flow atomic inline (even a 0x0 one) do.
///
/// Either way it is that line exactly: the last line, ended by the end of the
/// text (`BreakReason::None`), right after an emergency break with nothing to
/// paint or hit (no glyph run, no box), or right after a forced break
/// (`BreakReason::Explicit`) with no text and no box. A line of real content —
/// even a single space — has a glyph run or text and is kept.
///
/// **This is not a `<textarea>`'s rule**, whose value ending in `"\n"` does
/// show an empty last line in Chrome (where its caret goes). A textarea is laid
/// out by the form-control text path, not by an IFC, and keeps it. Nor is it
/// the rich-text editor's, which needs the line for a caret after a trailing
/// hard break and renders a second `<br>` for it, as ProseMirror does.
fn phantom_last_line(layout: &parley::Layout<Brush>) -> Option<usize> {
    use parley::PositionedLayoutItem;
    use parley::layout::BreakReason;
    let n = layout.len();
    if n < 2 {
        return None;
    }
    let last = layout.get(n - 1)?;
    let prev = layout.get(n - 2)?;
    if last.break_reason() != BreakReason::None {
        return None;
    }
    let phantom = match prev.break_reason() {
        BreakReason::Emergency => last.items().next().is_none(),
        BreakReason::Explicit => {
            last.text_range().is_empty()
                && !last
                    .items()
                    .any(|item| matches!(item, PositionedLayoutItem::InlineBox(_)))
        }
        _ => false,
    };
    phantom.then_some(n - 1)
}

/// One step of an IFC's parley tree-builder program, recorded by
/// [`IfcText`] and replayed once the whole IFC has been walked (#1180).
enum IfcOp<'a> {
    // 'static, not 'a: every property `inline_style_props` pushes is a Copy
    // value or (since #677) an owned `FontFamily` resolved through
    // `fonts::parley_font_family` — nothing in a span borrows from the node
    // slab's `'a`. `parley::TreeBuilder::push_style_modification_span` takes
    // its own independent `'s: 'iter` generic pair and resolves each property
    // within the call, so a `'static` item costs nothing at the replay site
    // in [`IfcText::finish`].
    Span(Vec<parley::style::StyleProperty<'static, Brush>>),
    Pop,
    Text(std::borrow::Cow<'a, str>),
    InlineBox(parley::InlineBox),
}

/// The text of one inline formatting context on its way to parley, with its
/// white space collapsed by rinch rather than by parley (#1180).
///
/// **Why rinch collapses.** parley 0.11.1's `WhiteSpaceCollapse::Collapse`
/// collapses and trims per *style span*: it trims a span's start and its end
/// (`is_span_first` / `is_span_last` in `resolve/tree.rs`). rinch pushes one
/// span per inline element, so a collapsible space at an element's edge was
/// removed — `a<span> b</span>` laid out `ab`, `rsx! { p { "Hello" b { "
/// world" } } }` `Helloworld`. CSS Text 3 §4.1.1 collapses across the whole
/// inline formatting context: phase I removes a collapsible space that
/// follows another one, "even one outside the boundary of the inline
/// containing that space", and phase II removes the spaces at a *line's*
/// start and end. And its trims took every Unicode `White_Space` character,
/// where CSS collapses only spaces, tabs and segment breaks — an NBSP at an
/// edge went too (#1154).
///
/// **What this does instead.** The walk hands every text node to
/// [`Self::push_text_node`] with the collapse mode of the element the text is
/// in ([`SpaceCollapse`], #1192 — not the root's: CSS applies
/// `white-space-collapse` per character), which applies both phases in
/// document order across element boundaries: each run of spaces,
/// tabs and segment breaks (LF, CR) becomes one space — not U+000C FORM FEED,
/// which Chrome draws as a character (#1181) — a space after another collapsible space is
/// removed wherever an element boundary falls between them, and a space at
/// the start of the IFC or after a `<br>` is removed. A space at the *end*
/// of the IFC or before a `<br>` cannot be known to be one until the walk
/// gets there, so the last kept space is held as `pending` and removed then
/// ([`Self::push_forced_break`], [`Self::finish`]); anything that is
/// content — any other character, an atomic inline —
/// confirms it. An out-of-flow box or an empty element is not content. A
/// space at a *soft* wrap is left to parley, which hangs it (a collapsible
/// space is never next to another now). The walk's ops are recorded rather than pushed,
/// because a removed trailing space may sit in a span that has already been
/// closed, and every op is replayed under `Preserve`, so parley trims
/// nothing: what it lays out is exactly this text.
///
/// **The flat offsets are offsets into that text.** They used to count the
/// text as pushed, so after any collapsed run every recorded offset — the
/// DOM↔flat caret maps, an inline background's range, the visibility mask —
/// was late by what parley had removed. [`Self::len`] is the text as kept so
/// far, with a held space counted; [`Self::remap`] takes such an offset to
/// the final text once `finish` has removed the held spaces. A text node's
/// own DOM↔flat correspondence is its `offset_map` (see
/// [`crate::node::IfcTextRange`]).
///
/// U+2028 LINE SEPARATOR, U+2029 PARAGRAPH SEPARATOR and U+0085 NEXT LINE,
/// in any mode, and U+000C FORM FEED in preserved text, are handed to parley
/// as substitutes that lay out the way Chrome 153 draws them, `letter-spacing`
/// and `word-spacing` aside ([`laid_out_as`], #1181): parley reads
/// the first two as forced line breaks, where Chrome draws an ordinary
/// space-wide character with a break opportunity after it (`x&#x2028;y` is
/// one line). The substitute's length differs from the character's, which
/// the `offset_map` records like any collapsed run.
///
/// Text under `pre`, `pre-wrap` or `break-spaces`, and all the text of a `contenteditable`
/// root, is pushed verbatim, a tab as one space that the replay widens to
/// its tab stop ([`TabStops`], #1122) and the characters above as their
/// substitutes. It is content to the collapsible text around
/// it: a preserved space confirms a held space before
/// it and is not collapsible, so a collapsible space right after it is kept,
/// and a preserved newline is a forced break, as a `<br>` is. `pre-line`
/// collapses spaces and tabs and keeps each segment break as a forced break.
pub(crate) struct IfcText<'a> {
    /// Every text node is pushed verbatim, whatever its own `white-space`: a
    /// `contenteditable` root, whose caret mapping needs the DOM text.
    preserve_all: bool,
    /// Some text was pushed with its spaces preserved ([`SpaceCollapse::Preserve`]).
    preserved: bool,
    ops: Vec<IfcOp<'a>>,
    len: usize,
    /// Nothing has been kept on this line yet: the IFC's start, or after a
    /// forced break (a `<br>`, a preserved newline).
    line_start: bool,
    /// The last thing kept was a collapsible space.
    prev_space: bool,
    /// That space, while it could still turn out to end a line: `(op, byte
    /// in the op's text, flat offset)`.
    pending: Option<(usize, usize, usize)>,
    /// Flat offsets (in [`Self::len`]'s terms) of the held spaces removed,
    /// ascending.
    dropped: Vec<usize>,
    /// The last atomic inline pushed and the flat offset it was pushed at.
    last_box: Option<(usize, usize)>,
    /// The out-of-flow boxes met ([`Self::note_out_of_flow`]), their offsets
    /// in [`Self::len`]'s terms and no position yet.
    out_of_flow: Vec<crate::node::OutOfFlowMark>,
    /// Every preserved tab, in order: `(op, byte in the op's text, flat
    /// offset in [`Self::len`]'s terms)`. A tab is pushed as one space and
    /// widened to its tab stop when replayed ([`Self::replay`], #1122).
    tabs: Vec<(usize, usize, usize)>,
    /// An edge box was pushed ([`Self::push_edge_box`], #1426).
    edge_boxes: bool,
}

/// Start a new stretch of a DOM↔flat `offset_map` at `(flat, dom)` when the
/// two have fallen out of step there ([`crate::node::IfcTextRange::offset_map`]).
fn offset_map_note(map: &mut Vec<(usize, usize)>, flat: usize, dom: usize) {
    let (f, d) = map.last().copied().unwrap_or((0, 0));
    if d.wrapping_sub(f) != dom.wrapping_sub(flat) {
        map.push((flat, dom));
    }
}

/// What parley is handed for a character it would lay out differently from
/// Chrome 153 (#1181), or `None` for one it gets right. `preserved`: the
/// text's white space is preserved (`pre`, `pre-wrap`).
///
/// - U+2028 LINE SEPARATOR and U+2029 PARAGRAPH SEPARATOR: parley reads
///   either as a forced break (`Whitespace::Newline`, and line-break class BK
///   in the analysis). Chrome draws a space-wide character under every
///   `white-space` that is never collapsed, never trimmed or hung at a
///   line's edge, and has a break opportunity after it. That is an NBSP (a
///   space's advance, no break before it, kept at a line's end by
///   [`InlineLayout::measured_width`]) followed by a ZERO WIDTH SPACE (the
///   break opportunity after it).
/// - U+0085 NEXT LINE: parley draws a glyph; Chrome draws nothing, with a
///   break opportunity — a ZERO WIDTH SPACE.
/// - U+000C FORM FEED under preserved white space: zero-width in Chrome, with
///   no break opportunity and no `letter-spacing` (`x\fx` at 10px spacing is
///   37.47px, as `xx`) — so it is removed. Where white space collapses
///   Chrome draws it as a character, which parley does too (13.8px in
///   Chrome against parley's 10.3px for the bundled Inter: the glyph differs).
///
/// Preserved text takes [`PRESERVED_SUBSTITUTES`], which is also what the
/// rich-text editor's caret map is told
/// ([`DomDocument::substituted_char_flat_bytes`]), so the two count the same
/// flat bytes for each.
///
/// [`InlineLayout::measured_width`]: crate::node::InlineLayout::measured_width
/// [`DomDocument::substituted_char_flat_bytes`]: rinch_core::dom::DomDocument::substituted_char_flat_bytes
fn laid_out_as(c: char, preserved: bool) -> Option<&'static str> {
    if !preserved && c == '\u{c}' {
        return None;
    }
    PRESERVED_SUBSTITUTES
        .iter()
        .find(|&&(k, _)| k == c)
        .map(|&(_, sub)| sub)
}

/// What [`laid_out_as`] hands parley for each character in preserved text.
pub(crate) const PRESERVED_SUBSTITUTES: [(char, &str); 4] = [
    ('\u{2028}', "\u{a0}\u{200b}"),
    ('\u{2029}', "\u{a0}\u{200b}"),
    ('\u{85}', "\u{200b}"),
    ('\u{c}', ""),
];

/// The flat bytes each of [`PRESERVED_SUBSTITUTES`] occupies.
pub(crate) const PRESERVED_SUBSTITUTE_FLAT_BYTES: [(char, usize); 4] = {
    let mut out = [('\0', 0); 4];
    let mut i = 0;
    while i < PRESERVED_SUBSTITUTES.len() {
        out[i] = (PRESERVED_SUBSTITUTES[i].0, PRESERVED_SUBSTITUTES[i].1.len());
        i += 1;
    }
    out
};

/// Whether preserved text may hold a tab or a character [`laid_out_as`]
/// substitutes: a vectorisable scan for their UTF-8 lead bytes (`\t`, `\f`,
/// `0xC2` for U+0085, `0xE2` for U+2028/U+2029), so ordinary text is pushed
/// as is at about memchr's cost. A hit is checked char by char.
fn may_rewrite_preserved(raw: &str) -> bool {
    raw.as_bytes().chunks(32).any(|chunk| {
        chunk
            .iter()
            .fold(false, |a, &b| a | matches!(b, b'\t' | 0x0C | 0xC2 | 0xE2))
    }) && raw.contains(|c| c == '\t' || laid_out_as(c, true).is_some())
}

/// Push `sub`, what [`laid_out_as`] lays one DOM character out as, onto
/// `out`, the character's DOM text ending at `dom_end`. The map's stretch
/// already starts at the first character; a position after it, inside the
/// substitute, maps to the DOM character's end — never into its UTF-8 bytes.
fn push_substitute(out: &mut String, map: &mut Vec<(usize, usize)>, sub: &str, dom_end: usize) {
    for (k, ch) in sub.char_indices() {
        if k > 0 {
            offset_map_note(map, out.len(), dom_end);
        }
        out.push(ch);
    }
}

/// How a text node's white space is processed: CSS Text 3
/// `white-space-collapse`, read from the text's own element (#1192).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SpaceCollapse {
    /// `normal`, `nowrap`: spaces, tabs and segment breaks collapse.
    Collapse,
    /// `pre-line`: spaces and tabs collapse, a segment break is a forced
    /// break and removes the collapsible spaces on both sides of it.
    PreserveBreaks,
    /// `pre`, `pre-wrap`, `break-spaces`: everything is kept.
    Preserve,
}

impl SpaceCollapse {
    pub(crate) fn of(white_space: crate::computed_style::WhiteSpaceValue) -> Self {
        use crate::computed_style::WhiteSpaceValue;
        match white_space {
            WhiteSpaceValue::Normal | WhiteSpaceValue::NoWrap => Self::Collapse,
            WhiteSpaceValue::PreLine => Self::PreserveBreaks,
            WhiteSpaceValue::Pre | WhiteSpaceValue::PreWrap | WhiteSpaceValue::BreakSpaces => {
                Self::Preserve
            }
        }
    }
}

impl<'a> IfcText<'a> {
    /// `preserve_all`: push every text node verbatim (a `contenteditable`
    /// root); otherwise each is collapsed by the mode it is pushed with.
    pub(crate) fn new(preserve_all: bool) -> Self {
        Self {
            preserve_all,
            preserved: preserve_all,
            ops: Vec::new(),
            len: 0,
            line_start: true,
            prev_space: false,
            pending: None,
            dropped: Vec::new(),
            last_box: None,
            out_of_flow: Vec::new(),
            tabs: Vec::new(),
            edge_boxes: false,
        }
    }

    /// Whether any text was pushed with its spaces preserved: only then can a
    /// line end in spaces that are content ([`InlineLayout::measured_width`],
    /// [`break_lines_hanging_spaces`]).
    pub(crate) fn preserved(&self) -> bool {
        self.preserved
    }

    /// The flat length so far, a held space included.
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn push_span(&mut self, props: Vec<parley::style::StyleProperty<'static, Brush>>) {
        self.ops.push(IfcOp::Span(props));
    }

    pub(crate) fn pop_span(&mut self) {
        self.ops.push(IfcOp::Pop);
    }

    /// An atomic inline is content: a held space before it stays, and a
    /// space after it is not at a line's start.
    pub(crate) fn push_inline_box(&mut self, inline_box: parley::InlineBox) {
        self.pending = None;
        self.line_start = false;
        self.prev_space = false;
        self.last_box = Some((inline_box.id as usize, self.len));
        self.ops.push(IfcOp::InlineBox(inline_box));
    }

    /// An inline element's start or end edge box (#1426): `width` CSS px
    /// (already scaled) of room and no height. Not content to white-space
    /// processing — CSS collapses spaces across an inline box's padding
    /// (css-text-3 §4.1.1: "even one outside the boundary of the inline"),
    /// and removes a space at a line's start whatever padding precedes it —
    /// so the held space, the line start and the last atomic inline are left
    /// as they were.
    ///
    /// An end box after a forced break that ends the element's content (a
    /// `<br>`, a preserved newline, with only closing spans after it) goes
    /// before that break: the element's last fragment is the line the break
    /// ends, and what follows starts the next line at its edge, as in Chrome
    /// 155 (review of #1823).
    pub(crate) fn push_edge_box(&mut self, owner: usize, end: bool, width: f32) {
        let edge = IfcOp::InlineBox(parley::InlineBox {
            id: edge_box_id(owner, end),
            index: 0,
            width,
            height: 0.0,
            kind: parley::InlineBoxKind::InFlow,
            baseline: None,
            vertical_align: parley::style::VerticalAlign::BASELINE,
        });
        self.edge_boxes = true;
        if end {
            // The last text op, past closing spans and end boxes only.
            let at = self.ops.iter().rposition(|op| match op {
                IfcOp::Pop => false,
                IfcOp::InlineBox(b) => !(is_edge_box(b.id) && b.id & EDGE_BOX_END != 0),
                _ => true,
            });
            if let Some(at) = at
                && let IfcOp::Text(text) = &self.ops[at]
                && text.ends_with('\n')
            {
                if text.len() > 1 {
                    // Split the newline off; tabs recorded in this op are
                    // before it and keep their op and byte.
                    let IfcOp::Text(text) = &mut self.ops[at] else {
                        unreachable!()
                    };
                    let mut prefix = std::mem::take(text).into_owned();
                    prefix.pop();
                    *text = prefix.into();
                    self.ops.insert(at + 1, IfcOp::Text("\n".into()));
                    self.shift_ops_after(at + 1);
                    self.ops.insert(at + 1, edge);
                    self.shift_ops_after(at + 1);
                } else {
                    self.ops.insert(at, edge);
                    self.shift_ops_after(at);
                }
                return;
            }
        }
        self.ops.push(edge);
    }

    /// An op was inserted at `at`: the recorded op indices from there on
    /// move up one.
    fn shift_ops_after(&mut self, at: usize) {
        for tab in &mut self.tabs {
            if tab.0 >= at {
                tab.0 += 1;
            }
        }
        if let Some(p) = &mut self.pending
            && p.0 >= at
        {
            p.0 += 1;
        }
    }

    /// Whether any edge box ([`Self::push_edge_box`]) was pushed.
    pub(crate) fn has_edge_boxes(&self) -> bool {
        self.edge_boxes
    }

    /// The node ids of the in-flow atomic inlines pushed, in order (#1238).
    pub(crate) fn atomic_box_ids(&self) -> impl Iterator<Item = usize> + '_ {
        self.ops.iter().filter_map(|op| match op {
            IfcOp::InlineBox(b)
                if !is_edge_box(b.id) && b.kind == parley::InlineBoxKind::InFlow =>
            {
                Some(b.id as usize)
            }
            _ => None,
        })
    }

    /// An out-of-flow box stands here in the content. It is not content — a
    /// held space before it is neither confirmed nor dropped, and it takes
    /// no room — but its static position is where it would have been
    /// (#632): remember the place.
    pub(crate) fn note_out_of_flow(&mut self, id: usize) {
        self.out_of_flow.push(crate::node::OutOfFlowMark {
            id,
            flat: self.len,
            after_box: self.last_box,
            inline: (0.0, 0.0),
            block_y: 0.0,
        });
    }

    /// The out-of-flow boxes met, with their offsets in the final text
    /// (after [`Self::finish`]) and no position yet
    /// ([`resolve_out_of_flow_marks`]).
    pub(crate) fn out_of_flow_marks(&self) -> Vec<crate::node::OutOfFlowMark> {
        self.out_of_flow
            .iter()
            .map(|mark| crate::node::OutOfFlowMark {
                flat: self.remap(mark.flat),
                after_box: mark.after_box.map(|(b, f)| (b, self.remap(f))),
                ..*mark
            })
            .collect()
    }

    /// A `<br>`: one `"\n"`, a forced break. The held space before it ends
    /// a line and goes, and the next line starts afresh.
    pub(crate) fn push_forced_break(&mut self) {
        self.break_line();
        self.ops.push(IfcOp::Text("\n".into()));
        self.len += 1;
    }

    /// A forced break: the held space before it ends a line and goes, and
    /// the next line starts afresh.
    fn break_line(&mut self) {
        self.drop_pending();
        self.line_start = true;
        self.prev_space = false;
    }

    fn drop_pending(&mut self) {
        if let Some((op, byte, flat)) = self.pending.take() {
            if let IfcOp::Text(text) = &mut self.ops[op] {
                text.to_mut().remove(byte);
            }
            self.dropped.push(flat);
        }
    }

    /// Push one text node's text (after `text-transform`), white space
    /// processed by `mode` — its own element's (#1192). Returns its flat
    /// length — the held space included — and its DOM↔flat `offset_map`
    /// (empty when the text is pushed byte for byte).
    ///
    /// Modes meet at the boundaries CSS Text 3 §4.1 draws. A preserved space
    /// is not collapsible, so a collapsible space right after one is kept
    /// (phase I removes a collapsible space only after another *collapsible*
    /// one), and a collapsible space held before preserved text is not at a
    /// line's end. A preserved newline, and a `pre-line` one, is a forced
    /// break, exactly as a `<br>` is: the held space before it goes and the
    /// collapsible spaces after it start a line.
    pub(crate) fn push_text_node(
        &mut self,
        raw: std::borrow::Cow<'a, str>,
        mode: SpaceCollapse,
    ) -> (usize, Vec<(usize, usize)>) {
        use offset_map_note as note;
        let mut map = Vec::new();
        let mode = if self.preserve_all {
            SpaceCollapse::Preserve
        } else {
            mode
        };
        if mode == SpaceCollapse::Preserve {
            if raw.is_empty() {
                return (0, map);
            }
            self.preserved = true;
            if !self.preserve_all {
                // Preserved text is content, a space included: it confirms a
                // held space before it — unless it starts with a newline,
                // which ends that space's line — and leaves nothing
                // collapsible behind it. Each newline starts a line.
                if raw.starts_with('\n') {
                    self.drop_pending();
                }
                self.pending = None;
                self.prev_space = false;
                self.line_start = raw.ends_with('\n');
            }
            if !may_rewrite_preserved(raw.as_ref()) {
                let n = raw.len();
                self.ops.push(IfcOp::Text(raw));
                self.len += n;
                return (n, map);
            }
            let op = self.ops.len();
            let mut out = String::with_capacity(raw.len() + 6);
            for (i, c) in raw.char_indices() {
                note(&mut map, out.len(), i);
                if c == '\t' {
                    // One space, one flat byte, as the DOM's tab is one
                    // byte; widened to its stop at replay.
                    self.tabs.push((op, out.len(), self.len + out.len()));
                    out.push(' ');
                } else if let Some(sub) = laid_out_as(c, true) {
                    push_substitute(&mut out, &mut map, sub, i + c.len_utf8());
                } else {
                    out.push(c);
                }
            }
            let n = out.len();
            self.ops.push(IfcOp::Text(out.into()));
            self.len += n;
            return (n, map);
        }
        let keep_breaks = mode == SpaceCollapse::PreserveBreaks;
        let op = self.ops.len();
        // Allocated at the first byte the kept text differs from `raw` in;
        // until then the kept text is `raw[..i]`.
        let mut out: Option<String> = None;
        for (i, c) in raw.char_indices() {
            if keep_breaks && c == '\n' {
                // `pre-line`: a segment break is kept, as a forced break.
                match self.pending {
                    // The held space is this node's own last kept byte:
                    // removed here, so no later offset counts it. A map
                    // entry noted for it now starts an empty stretch, which
                    // maps nothing differently.
                    Some((pop, byte, _)) if pop == op => {
                        let o = out.get_or_insert_with(|| raw[..i].to_string());
                        debug_assert_eq!(o.len(), byte + 1);
                        o.truncate(byte);
                        self.pending = None;
                        self.line_start = true;
                        self.prev_space = false;
                    }
                    _ => self.break_line(),
                }
                let flat = out.as_ref().map_or(i, String::len);
                note(&mut map, flat, i);
                if let Some(o) = out.as_mut() {
                    o.push('\n');
                }
                continue;
            }
            // CSS Text 3 §4.1.1 collapses spaces, tabs and segment breaks (LF,
            // and CR, which HTML folds into one). Not U+000C FORM FEED, which
            // `is_ascii_whitespace` includes: Chrome 153 draws it (#1181).
            let space = matches!(c, ' ' | '\t' | '\n' | '\r');
            if space && (self.line_start || self.prev_space) {
                out.get_or_insert_with(|| raw[..i].to_string());
                continue;
            }
            let sub = laid_out_as(c, false);
            if space {
                self.prev_space = true;
            } else {
                self.prev_space = false;
                self.line_start = false;
                self.pending = None;
            }
            if out.is_none() && ((space && c != ' ') || sub.is_some()) {
                out = Some(raw[..i].to_string());
            }
            let flat = out.as_ref().map_or(i, String::len);
            note(&mut map, flat, i);
            if space {
                self.pending = Some((op, flat, self.len + flat));
            }
            if let Some(o) = out.as_mut() {
                match sub {
                    Some(sub) => push_substitute(o, &mut map, sub, i + c.len_utf8()),
                    None if space => o.push(' '),
                    None => o.push(c),
                }
            }
        }
        let text: std::borrow::Cow<'a, str> = match out {
            Some(o) => o.into(),
            None => raw,
        };
        let n = text.len();
        if n > 0 {
            self.ops.push(IfcOp::Text(text));
        }
        self.len += n;
        (n, map)
    }

    /// Remove a space held at the IFC's end. Called once, after the walk
    /// and before [`Self::replay`].
    pub(crate) fn finish(&mut self) {
        self.drop_pending();
    }

    /// The flat offsets of the preserved tabs in the final text (after
    /// [`Self::finish`]), ascending.
    pub(crate) fn tab_offsets(&self) -> Vec<usize> {
        self.tabs.iter().map(|&(_, _, f)| self.remap(f)).collect()
    }

    /// Replay the ops into `builder`, all under `Preserve`: the text is
    /// already collapsed. May be called again with other `tab_spacing`.
    ///
    /// With `emoji_family`, each emoji-presentation cluster of a text is
    /// pushed in a span of that family ([`crate::fonts::TextFamily`], #1204).
    /// A cluster is found within one text op; one split across two (an emoji
    /// and its U+FE0F in two text nodes) is not.
    ///
    /// Each preserved tab is its one space in a span of its own, with no
    /// `word-spacing` and `tab_spacing[i]` (CSS px) of `letter-spacing` for
    /// the `i`th tab ([`set_tab_stops`], #1122): what widens it to its stop.
    pub(crate) fn replay(
        &self,
        builder: &mut parley::TreeBuilder<'_, Brush>,
        emoji_family: Option<&parley::style::FontFamily<'static>>,
        tab_spacing: &[f32],
    ) {
        // parley 0.12 made `white-space-collapse` a style (`TextStyle::white_space_collapse`),
        // `Preserve` by default and set by nothing in rinch: the builder collapses nothing.
        use parley::style::StyleProperty as P;
        let emoji_span = emoji_family.map(|f| [P::FontFamily(f.clone())]);
        let push_plain =
            |builder: &mut parley::TreeBuilder<'_, Brush>, text: &str| match &emoji_span {
                Some(span) => {
                    let mut at = 0;
                    for range in crate::fonts::emoji_presentation_ranges(text) {
                        if range.start > at {
                            builder.push_text(&text[at..range.start]);
                        }
                        builder.push_style_modification_span(span.iter());
                        builder.push_text(&text[range.clone()]);
                        builder.pop_style_span();
                        at = range.end;
                    }
                    if at < text.len() {
                        builder.push_text(&text[at..]);
                    }
                }
                None => builder.push_text(text),
            };
        let mut tab = 0usize;
        for (i, op) in self.ops.iter().enumerate() {
            match op {
                IfcOp::Span(props) => builder.push_style_modification_span(props.iter()),
                IfcOp::Pop => builder.pop_style_span(),
                IfcOp::Text(text) => {
                    let mut at = 0;
                    while let Some(&(op, byte, _)) = self.tabs.get(tab) {
                        if op != i {
                            break;
                        }
                        if byte > at {
                            push_plain(builder, &text[at..byte]);
                        }
                        let spacing = [
                            P::LetterSpacing(tab_spacing.get(tab).copied().unwrap_or(0.0)),
                            P::WordSpacing(0.0),
                        ];
                        builder.push_style_modification_span(spacing.iter());
                        builder.push_text(&text[byte..byte + 1]);
                        builder.pop_style_span();
                        at = byte + 1;
                        tab += 1;
                    }
                    if at < text.len() || at == 0 {
                        push_plain(builder, &text[at..]);
                    }
                }
                IfcOp::InlineBox(b) => builder.push_inline_box(b.clone()),
            }
        }
        let has_content = self.ops.iter().any(|op| match op {
            IfcOp::Text(text) => !text.is_empty(),
            IfcOp::InlineBox(_) => true,
            _ => false,
        });
        if !has_content {
            builder.push_inline_box(parley::InlineBox {
                id: EMPTY_IFC_BOX,
                index: 0,
                width: 0.0,
                height: 0.0,
                kind: parley::InlineBoxKind::OutOfFlow,
                baseline: None,
                vertical_align: parley::style::VerticalAlign::BASELINE,
            });
        }
    }

    /// Whether any text pushed so far could hold an emoji-presentation
    /// cluster: a byte at or above 0xE2, the UTF-8 lead byte of U+231A, the
    /// first `Emoji_Presentation` character (see
    /// [`crate::fonts::emoji_presentation_ranges`]).
    pub(crate) fn may_hold_emoji(&self) -> bool {
        self.ops.iter().any(|op| match op {
            IfcOp::Text(text) => text.bytes().any(|b| b >= 0xE2),
            _ => false,
        })
    }

    /// A flat offset recorded during the walk, in the final text: less the
    /// held spaces [`Self::finish`] removed before it.
    pub(crate) fn remap(&self, flat: usize) -> usize {
        flat - self.dropped.partition_point(|&d| d < flat)
    }

    /// The final length, after [`Self::finish`].
    pub(crate) fn final_len(&self) -> usize {
        self.len - self.dropped.len()
    }

    /// The final text, after [`Self::finish`]: its text ops end to end. A
    /// single text node's is the node's own text, borrowed, when nothing was
    /// collapsed.
    pub(crate) fn into_text(self) -> std::borrow::Cow<'a, str> {
        let mut texts = self.ops.into_iter().filter_map(|op| match op {
            IfcOp::Text(t) => Some(t),
            _ => None,
        });
        let first = texts.next().unwrap_or_default();
        texts.fold(first, |mut all, t| {
            all.to_mut().push_str(&t);
            all
        })
    }
}

/// Find, in `layout`'s lines, where each out-of-flow box of `marks` would
/// have sat — its static position (CSS 2.1 §10.3.7, §10.6.4; issues #632 and
/// #634). `text` is the flat text and `scale` what the layout was built at.
///
/// Measured in Chrome 153 (`tests/static_position_tests.rs`):
///
/// - a box that was **inline-level** sits where the content before it ends
///   on its line, at the line box's top — whatever makes that line tall. At
///   a soft wrap it stays on the line the content before it is on, at the end
///   of that line's content (a space hung there is not counted); after a
///   forced break it starts the next line; with nothing before it, it starts
///   the first line. `text-align` moves it with its line;
/// - a **block-level** box goes at the content box's left edge, below the
///   line holding the content before it — or at the first line's top when
///   nothing precedes it. (After a forced break that is the next line's top:
///   the same number.)
///
/// "The content before it" is the last atomic inline pushed before the box
/// when no text followed that one ([`crate::node::OutOfFlowMark::after_box`]),
/// else the cluster of the byte before it.
pub(crate) fn resolve_out_of_flow_marks(
    layout: &parley::Layout<Brush>,
    text: &str,
    scale: f32,
    marks: &mut [crate::node::OutOfFlowMark],
) {
    use parley::layout::{Cluster, PositionedLayoutItem};
    if marks.is_empty() {
        return;
    }
    let after_box = |m: &crate::node::OutOfFlowMark| {
        m.after_box
            .filter(|&(_, at)| at == m.flat)
            .map(|(id, _)| id)
    };
    // The atomic inlines some box stands right after: line and right edge.
    let mut boxes: HashMap<usize, (usize, f32)> = HashMap::new();
    if marks.iter().any(|m| after_box(m).is_some()) {
        for (index, line) in layout.lines().enumerate() {
            for item in line.items() {
                if let PositionedLayoutItem::InlineBox(b) = item {
                    boxes.insert(b.id as usize, (index, b.x + b.width));
                }
            }
        }
    }
    // Each line box's top: the lines stack by their line heights. (Not
    // `block_min_coord`, which leaves a negative half-leading out — the top
    // of the glyphs' own box, above the line box's when a 32px run stands in
    // a 20px line.)
    let mut tops: Vec<f32> = Vec::with_capacity(layout.len() + 1);
    let mut y = 0.0;
    for line in layout.lines() {
        tops.push(y);
        y += line.metrics().line_height;
    }
    tops.push(y);
    let top = |index: usize| tops.get(index).copied().unwrap_or(y);
    for mark in marks.iter_mut() {
        // (x, the line's top, the line's bottom): the end of the content
        // before the box, or — with the two the same — the start of a line
        // nothing precedes it on.
        let at_line_start = |index: usize| {
            layout
                .get(index)
                .map(|line| (line.metrics().offset, top(index), top(index)))
        };
        let found = if let Some(&(index, right)) = after_box(mark).and_then(|id| boxes.get(&id)) {
            Some((right, top(index), top(index + 1)))
        } else if mark.flat == 0 {
            at_line_start(0)
        } else {
            Cluster::from_byte_index(layout, mark.flat - 1).map(|cluster| {
                let index = cluster.path().line_index();
                let line = cluster.line();
                let m = line.metrics();
                if text.as_bytes().get(mark.flat - 1) == Some(&b'\n') {
                    // A forced break: the box starts the next line. A text-
                    // final break has none (`phantom_last_line`); the box
                    // still goes below this one.
                    return at_line_start(index + 1).unwrap_or((
                        m.offset,
                        top(index + 1),
                        top(index + 1),
                    ));
                }
                let start = cluster.visual_offset().unwrap_or(m.offset);
                let x = if cluster.is_rtl() {
                    start
                } else {
                    // Not past the line's content: a space hung at a soft
                    // wrap takes no room there.
                    (start + cluster.advance()).min(m.offset + m.advance - m.hanging_advance)
                };
                (x, top(index), top(index + 1))
            })
        };
        let (x, top, bottom) = found.unwrap_or((0.0, 0.0, 0.0));
        mark.inline = (x / scale, top / scale);
        mark.block_y = bottom / scale;
    }
}

/// Break `layout` at `max_width` as `break_all_lines` does, but commit only its
/// first `keep` lines — what [`phantom_last_line`] leaves.
fn break_lines_up_to(layout: &mut parley::Layout<Brush>, max_width: Option<f32>, keep: usize) {
    use parley::layout::YieldData;
    let max = max_width.unwrap_or(f32::MAX);
    let mut breaker = layout.break_lines();
    let state = breaker.state_mut();
    state.set_layout_max_advance(max);
    state.set_line_max_advance(max);
    let mut committed = 0usize;
    while committed < keep {
        match breaker.break_next() {
            Some(YieldData::LineBreak(_)) => committed += 1,
            Some(_) => {}
            None => break,
        }
    }
    breaker.finish();
}

/// [`break_lines_hanging_spaces`] for a paragraph none of whose text may
/// wrap, aligned within `align` (#1237): broken unconstrained, each line
/// given the content width as its alignment width ([`break_unwrapped`]).
/// When the paragraph's own width (its widest line, as a measure counts it:
/// [`crate::node::measured_width_of`]) turns out to lie between that and
/// `align.up_to` — a shrink-to-fit box floored from it — the lines are
/// aligned within that width instead; and when parley left an empty line
/// after a final forced break ([`phantom_last_line`]): for either the
/// paragraph is broken once more, for both at once.
fn break_unwrapped_aligned(
    layout: &mut parley::Layout<Brush>,
    text: &str,
    align: AlignWidth,
    preserves_spaces: bool,
    unhung: &UnhungSpaces,
) -> HangStats {
    break_unwrapped(layout, align.width, None);
    let mut stats = HangStats::default();
    let own = crate::node::measured_width_of(
        layout,
        text,
        preserves_spaces,
        f32::INFINITY,
        &unhung.breaking,
    );
    // A box sized from the paragraph's own width is that width rounded to
    // a whole pixel, either way (`a\nbbb`, 29.39 wide, is a 29px
    // inline-block; `a\nbbbbbb`, 58.78, a 59px one). Only an auto-width box
    // has the slack (`up_to > width`); a declared width is aligned within.
    let widened =
        align.up_to > align.width && own != align.width && (own - align.width).abs() < 1.0;
    let width = if widened { own } else { align.width };
    let keep = phantom_last_line(layout);
    if widened || keep.is_some() {
        break_unwrapped(layout, width, keep);
        stats.passes = u32::from(widened);
        stats.phantom_rebreaks = u32::from(keep.is_some());
    }
    stats
}

/// Break `layout` unconstrained — only forced breaks end its lines — giving
/// each line `width` as its alignment width, so that `align` lines it up
/// within `width` and not within the widest line (#1237); with `limit`, only
/// that many lines are committed (see [`phantom_last_line`]).
fn break_unwrapped(layout: &mut parley::Layout<Brush>, width: f32, limit: Option<usize>) {
    use parley::layout::YieldData;
    let mut breaker = layout.break_lines();
    let mut committed = 0usize;
    while limit != Some(committed) {
        let state = breaker.state_mut();
        state.set_layout_max_advance(f32::MAX);
        state.set_line_max_advance(f32::MAX);
        let Some(yielded) = breaker.break_next() else {
            break;
        };
        let YieldData::LineBreak(_) = yielded else {
            continue;
        };
        committed += 1;
        breaker.set_prior_line_width(width);
    }
    breaker.finish();
}

/// The re-break of [`break_lines_hanging_spaces`], over a layout already
/// broken at `max`, committing at most `limit` lines when given one: lines
/// parley ends where CSS does not — next to an edge box (#1426), at a seam
/// (#1238, #1243), after an overflowing box (#1081), at an overflowing NBSP
/// ([`unglue`], #1218) or in overflowing `break-spaces` spaces (#1043) — are
/// committed again where CSS ends them. (Hanging spaces, trailing white space
/// in alignment and justification opportunities were also fixed up here
/// until parley 0.12 got them right itself.)
fn hang_pass(
    layout: &mut parley::Layout<Brush>,
    text: &str,
    max: f32,
    limit: Option<usize>,
    unhung: &UnhungSpaces,
) -> HangStats {
    use parley::layout::{BreakReason, YieldData};
    let mut stats = HangStats::default();
    let units = logical_units(layout, text, unhung);
    let has_edges = units.iter().any(|u| u.edge != 0);
    // #1238: parley's breaks next to atomic inlines are not CSS's here.
    let seams = unhung.active_seams().is_some();
    let pre_seams = seams && unhung.pre_seams.get();
    let near_pre = pre_seams.then(|| NearPre::new(&units));
    let breaks = TextBreaks::new(text);
    stats.passes = 1;
    let mut breaker = layout.break_lines();
    // The first unit of the line being broken, and the state it starts from.
    let mut cursor = 0usize;
    let mut line_start = breaker.state().clone();
    let mut in_step = true;
    // Every turn of this loop leaves exactly one line committed: a line broken
    // again replaces itself.
    let mut committed = 0usize;
    loop {
        if limit == Some(committed) {
            break;
        }
        let state = breaker.state_mut();
        state.set_layout_max_advance(max);
        state.set_line_max_advance(max);
        let Some(yielded) = breaker.break_next() else {
            break;
        };
        // Only a committed line ends one; nothing else is yielded here.
        let YieldData::LineBreak(mut data) = yielded else {
            continue;
        };
        committed += 1;
        if !in_step {
            continue;
        }
        // #1426: a break parley made next to an edge box where CSS does
        // not break is moved to where it does. #1238: with seams, so is
        // every soft wrap — parley breaks after every box whatever
        // `white-space` says, and misses the seams' own opportunities, so it
        // stops at an earlier one that fits (#1243) — and a line that
        // overflows — parley appends a box after text that does not wrap
        // even where CSS breaks before it — is ended where CSS ends it,
        // whatever ended it. Not an `overflow-wrap` line.
        let soft = matches!(data.reason, BreakReason::Regular | BreakReason::Emergency);
        let emergency = data.reason == BreakReason::Emergency;
        if (has_edges && soft) || seams {
            let Some(end) = line_end(&units, cursor, data.metrics.advance) else {
                in_step = false;
                continue;
            };
            let wrong = (has_edges
                && soft
                && !edge_break_allowed(&units, &breaks, end, emergency, false, pre_seams))
                || (seams
                    && !emergency
                    && ((soft
                        && (!wrap_allowed_after(&units, end)
                            || near_pre
                                .as_ref()
                                .is_some_and(|n| n.touches(&units, cursor, end, max))))
                        || line_overflows(&units[cursor..end], max)))
                // parley's emergency break after a box too wide for the
                // line, where `white-space` glues what follows to it.
                || (seams
                    && emergency
                    && end > 0
                    && units[end - 1].inline_box
                    && !wrap_allowed_after(&units, end));
            let css = wrong
                .then(|| {
                    stats.css_scans += 1;
                    css_line(&units, &breaks, cursor, max, pre_seams)
                })
                .filter(|css| !same_end(&units, css.0, end));

            if let Some(css) = css {
                let Some((reason, advance)) =
                    commit_css_line(&mut breaker, &units, cursor, css, &line_start, max)
                else {
                    in_step = false;
                    continue;
                };
                stats.lines += 1;
                data.reason = reason;
                data.metrics.advance = advance;
            }
        }
        // The last line is committed as it is (`BreakReason::None`; an empty
        // one after a final newline even copies the previous line's advance).
        if data.reason == BreakReason::None {
            continue;
        }
        // #1081: the white space or forced break after a box parley placed
        // by its emergency branch belongs on the box's line.
        if data.reason == BreakReason::Emergency {
            match fold_after_overflowing_box(
                &mut breaker,
                &units,
                cursor,
                data.metrics.advance,
                &line_start,
                max,
            ) {
                Ok(Some(end)) => {
                    stats.lines += 1;
                    cursor = end;
                    line_start = breaker.state().clone();
                    continue;
                }
                Ok(None) => {}
                Err(()) => {
                    in_step = false;
                    continue;
                }
            }
        }
        // A line that ends at the hang is broken again, from the state it
        // started from, with room for its content and every space and tab
        // after it. That can end at a hang again — an NBSP after the spaces
        // overflows too, and parley hangs NBSP itself — so until it does not.
        let (mut reason, mut advance) = (data.reason, data.metrics.advance);
        let mut line_max = max;
        let mut unglued = false;
        let mut spaces_broken = false;
        loop {
            let Some(end) = line_end(&units, cursor, advance) else {
                in_step = false;
                break;
            };
            // #1218: parley hangs an overflowing NBSP as it hangs a space, and
            // commits the line after it, where there is no break opportunity.
            if !unglued
                && reason == BreakReason::Regular
                && advance > line_max
                && end > cursor
                && units[end - 1].nbsp
            {
                unglued = true;
                let rebreaks = std::cell::Cell::new(0);
                let again = unglue(
                    &mut breaker,
                    &units,
                    cursor,
                    end,
                    &line_start,
                    max,
                    advance,
                    &rebreaks,
                );
                stats.unglue_rebreaks += rebreaks.get();
                let Some(again) = again else {
                    in_step = false;
                    break;
                };
                stats.lines += 1;
                (reason, advance) = again;
                line_max = max;
                continue;
            }
            // #1043: `break-spaces` spaces never hang. A line parley ended
            // with such spaces past the room is broken where CSS breaks it.
            if !spaces_broken && reason == BreakReason::Regular && advance > max + 0.005 {
                spaces_broken = true;
                if let Some(again) =
                    break_overflowing_spaces(&mut breaker, &units, cursor, end, &line_start, max)
                {
                    stats.lines += 1;
                    (reason, advance) = again;
                    line_max = max;
                    continue;
                }
            }
            cursor = end;
            line_start = breaker.state().clone();
            break;
        }
    }
    breaker.finish();
    stats
}

/// Break again a line that ends in an inline box parley placed by its
/// emergency branch, so that the spaces and tabs after the box, or a newline
/// right after it, end the box's line (#1081). Returns the unit the next line
/// starts at, `None` when nothing of that kind follows the box (the line is
/// kept as it is), and `Err` when the unit table and the breaker disagree.
///
/// At the start of a line parley 0.11.1 consumes a box too wide for any line
/// and commits the line on the spot, so whatever follows starts the next
/// line. CSS puts a forced break on the line it ends (`x <ib><br>` is two
/// lines in Chrome 155, not three), and white space at a soft wrap hangs —
/// a collapsible space is removed there — so neither makes a line of its
/// own. The line is committed again by length: the box and either the run of
/// spaces and tabs after it ([`LineUnit::hangs`]: not an NBSP, not a `pre` or
/// `break-spaces` space) or a newline right after it. Spaces that a newline
/// follows are not folded: Chrome 155 puts them and the break on a line of
/// their own (`x <ib>  \n` is three lines there, the box's line ending at
/// the box). A word right after the box, or after the spaces, still starts
/// the next line.
fn fold_after_overflowing_box(
    breaker: &mut parley::layout::BreakLines<'_, Brush>,
    units: &[LineUnit],
    cursor: usize,
    advance: f32,
    line_start: &parley::layout::BreakerState,
    max: f32,
) -> Result<Option<usize>, ()> {
    let end = line_end(units, cursor, advance).ok_or(())?;
    if end == cursor || !units[end - 1].inline_box {
        return Ok(None);
    }
    let spaces = units[end..].iter().take_while(|u| u.hangs).count();
    let ends_in_newline = units.get(end + spaces).is_some_and(|u| u.newline);
    // Spaces that a forced break follows do not hang after the box: Chrome 155
    // puts them, and the break, on a line of their own (`x <ib>  \n` is three
    // lines). Only a newline right after the box joins its line.
    if spaces > 0 && ends_in_newline {
        return Ok(None);
    }
    let newline = ends_in_newline;
    let take = end + spaces + usize::from(newline);
    if take == end {
        return Ok(None);
    }
    // The table's cursor can sit on the zero-width newline that ended the
    // line before; a newline is never the first unit of a line.
    let first = cursor + units[cursor..end].iter().take_while(|u| u.newline).count();
    let n = units[first..take]
        .iter()
        .map(|u| u.counted as usize)
        .sum::<usize>();
    breaker.revert_to(line_start.clone());
    breaker
        .break_next_with_length(u32::try_from(n).map_err(|_| ())?)
        .ok_or(())?;
    breaker.set_prior_line_width(max);
    Ok(Some(take))
}

/// Break again a line parley ended by hanging a no-break space (#1218), and
/// return the reason and advance of the line that replaces it.
///
/// U+00A0 is UAX #14 class GL: no break after it, and none before it but
/// after a space, tab or hyphen (LB12, LB12a). parley 0.11.1's breaker hangs an overflowing one like a space
/// (`is_space_or_nbsp` in its hang branch) and commits the line right after
/// it. The line, from `line_start` to `units[end]`, ends in a run of NBSPs
/// that starts `run_x` along it; CSS puts the break where parley would have
/// without that branch:
///
/// - **Right after an inline box or a hyphen** the run follows: Chrome 153
///   breaks between an atomic inline and an NBSP, where UAX #14 alone would
///   glue them, and LB12a allows a break after HY and BA.
/// - **At the last opportunity before the run.** Broken with room for all
///   but the unit before the run, that unit overflows and parley takes the
///   opportunity it last passed — the same one, since an NBSP offers none.
/// - **Right before the run**, when that is an `overflow-wrap` (emergency)
///   opportunity and there is no other: the run then starts the next line,
///   as in Chrome 153. parley says so by answering the first re-break with
///   an emergency break one unit early, and the line is committed by length.
/// - **At the first opportunity after the glued word**, when there is none
///   before the run: the word overflows, to the FIRST opportunity after it.
///   Found by a galloping search over the units after the run (see the
///   code): O(log d) breaks of a line d units long, plus one per NBSP in the
///   gap after the glued word — not one per glued word, which is quadratic
///   in a chain.
///
/// parley's `main` hangs no NBSP since linebender/parley#762 (merged
/// 2026-09-07, after 0.11.1): this can go with the release that carries it.
///
/// Every break it makes is counted in `rebreaks`. `None` when the unit table
/// and the breaker disagree.
#[allow(clippy::too_many_arguments)]
fn unglue(
    breaker: &mut parley::layout::BreakLines<'_, Brush>,
    units: &[LineUnit],
    cursor: usize,
    end: usize,
    line_start: &parley::layout::BreakerState,
    max: f32,
    advance: f32,
    rebreaks: &std::cell::Cell<u32>,
) -> Option<(parley::layout::BreakReason, f32)> {
    use parley::layout::{BreakReason, YieldData};
    // No following word fits in this much room: it only decides which side
    // of a width boundary a unit falls.
    const ROOM: f32 = 0.01;
    let run = units[cursor..end]
        .iter()
        .rev()
        .take_while(|u| u.nbsp)
        .count();
    let run_start = end - run;
    let run_x = advance - units[run_start..end].iter().map(|u| u.advance).sum::<f32>();
    type Breaker<'b, 'l> = &'b mut parley::layout::BreakLines<'l, Brush>;
    let rebreak = |breaker: Breaker<'_, '_>, line_max: f32| -> Option<(BreakReason, f32)> {
        rebreaks.set(rebreaks.get() + 1);
        breaker.revert_to(line_start.clone());
        let state = breaker.state_mut();
        state.set_layout_max_advance(line_max);
        state.set_line_max_advance(line_max);
        match breaker.break_next()? {
            YieldData::LineBreak(d) => Some((d.reason, d.metrics.advance)),
            _ => None,
        }
    };
    let ends_in_hung_nbsp = |(reason, adv): (BreakReason, f32), line_max: f32| {
        reason == BreakReason::Regular
            && adv > line_max
            && line_end(units, cursor, adv).is_some_and(|e| e > cursor && units[e - 1].nbsp)
    };
    // Commit the line up to the run, by length: the line breaker places no
    // break of its own right before an NBSP.
    let before_run =
        |breaker: Breaker<'_, '_>, reason: BreakReason| -> Option<(BreakReason, f32)> {
            // The table's cursor can sit on the zero-width newline that ended the
            // line before; a newline is never the first unit of a line.
            let first = cursor
                + units[cursor..run_start]
                    .iter()
                    .take_while(|u| u.newline)
                    .count();
            let before = &units[first..run_start];
            let width: f32 = before.iter().map(|u| u.advance).sum();
            if (width - run_x).abs() > 0.005 {
                return None;
            }
            let n = before.iter().map(|u| u.counted as usize).sum::<usize>();
            rebreaks.set(rebreaks.get() + 1);
            breaker.revert_to(line_start.clone());
            breaker.break_next_with_length(u32::try_from(n).ok()?)?;
            breaker.set_prior_line_width(max);
            Some((reason, run_x))
        };
    // Right after an inline box (Chrome 153 breaks between an atomic inline
    // and the NBSP after it, as parley does after any box) or a hyphen
    // (LB12a: no break before GL but after a space, BA or HY).
    if run_start > cursor && {
        let u = &units[run_start - 1];
        u.inline_box || u.breaks_before_glue
    } {
        return before_run(breaker, BreakReason::Regular);
    }
    if run_start > cursor && run_x > ROOM {
        let line_max = run_x - ROOM;
        let a = rebreak(breaker, line_max)?;
        match a.0 {
            // Text's emergency break (`overflow-wrap`), one unit early: the
            // last emergency opportunity is right before the run.
            BreakReason::Emergency if a.1 <= line_max => {
                return before_run(breaker, BreakReason::Emergency);
            }
            // A regular break, or a box too wide for any line placed alone at
            // the line's start: the last opportunity before the run.
            _ if !ends_in_hung_nbsp(a, line_max) => {
                breaker.set_prior_line_width(max);
                return Some(a);
            }
            // Hung again: there is none.
            _ => {}
        }
    }
    // No opportunity before the run: the glued word overflows, up to the
    // first opportunity after the run. Where that is, the breaker answers.
    // Broken with room for everything before a unit `u` (not an NBSP, space,
    // tab or newline, and with width), `u` is the first unit to overflow, and
    // parley breaks at the last opportunity at or before it — so the line
    // comes back fitting the room exactly when there is an opportunity between
    // the run and `u`. That is monotone in `u`: a galloping search finds the
    // first `u` that has one, in O(log d) breaks of a line d units long (a
    // probe can walk up to about twice the line, where the gallop overshoots)
    // — not one break per glued word, which is quadratic in a chain of them
    // (review of #1257). Where in the gap before that `u` the line breaks is
    // decided after the search, below.
    let first = cursor + units[cursor..end].iter().take_while(|u| u.newline).count();
    // (unit index, x where it starts) of every candidate after the run, up to
    // the forced break that ends the line anyway; extended as the search goes.
    let mut candidates: Vec<(usize, f32)> = Vec::new();
    let mut scan = first;
    let mut scan_x = 0.0f32;
    let mut more = |upto: usize, candidates: &mut Vec<(usize, f32)>| {
        while candidates.len() <= upto && scan < units.len() {
            let u = &units[scan];
            if u.newline && scan >= end {
                scan = units.len();
                break;
            }
            if scan >= end && !u.nbsp && !u.hangs && !u.unhung && u.advance > ROOM {
                candidates.push((scan, scan_x));
            }
            scan_x += u.advance;
            scan += 1;
        }
    };
    // Whether the line broken with room up to candidate `k` fits that room.
    let fits = |breaker: Breaker<'_, '_>, x: f32| -> Option<bool> {
        let room = x + ROOM;
        let c = rebreak(breaker, room)?;
        Some(!ends_in_hung_nbsp(c, room) && c.1 <= room + 0.005)
    };
    let (mut lo, mut hi) = (None::<usize>, None::<usize>);
    let mut k = 0usize;
    loop {
        more(k, &mut candidates);
        // Past the last candidate: the last one is the last to try.
        if k >= candidates.len() {
            match candidates.len().checked_sub(1) {
                Some(last) if lo.is_none_or(|l| last > l) => k = last,
                _ => break,
            }
        }
        let x = candidates[k].1;
        if fits(breaker, x)? {
            hi = Some(k);
            break;
        }
        lo = Some(k);
        k = 2 * k + 1;
    }
    // The search finds the first candidate `hi` with an opportunity before
    // it, but breaking with room up to `hi` would take the LAST opportunity
    // before it, and the gap between two candidates (spaces, NBSPs, zero-width
    // units) can hold several: CSS takes the first (review of #1257, round 2:
    // `aaaa~bbbbbb ~ cc` at 40px is `aaaa~bbbbbb ` / `~ cc` in Chrome 153).
    // There is none up to the candidate before `hi`, so with room up to where
    // it ends, the next unit overflows and parley commits at the first
    // opportunity — unless it hangs an NBSP of the gap, which takes one more
    // break per NBSP there.
    let before_hi = match hi {
        Some(mut hi) => {
            let mut lo = lo;
            while lo.map_or(0, |l| l + 1) < hi {
                let mid = (lo.map_or(0, |l| l + 1) + hi) / 2;
                if fits(breaker, candidates[mid].1)? {
                    hi = mid;
                } else {
                    lo = Some(mid);
                }
            }
            hi.checked_sub(1)
        }
        // None up to the forced break or the end of the text: start after the
        // last candidate.
        None => candidates.len().checked_sub(1),
    };
    let mut through = match before_hi {
        Some(k) => candidates[k].1 + units[candidates[k].0].advance,
        None => advance,
    };
    let result = loop {
        let c = rebreak(breaker, through + ROOM)?;
        // Nothing more taken in (a safety net: every round takes in at least
        // the NBSP it hung last time).
        if !ends_in_hung_nbsp(c, through + ROOM) || c.1 <= through {
            break c;
        }
        through = c.1;
    };
    breaker.set_prior_line_width(max);
    Some(result)
}

/// Break again a line parley ended past `max` in a run of `break-spaces`
/// spaces or tabs (#1043), and return the reason and advance of the line that
/// replaces it; `None` when the line is not one, or is right as it is.
///
/// parley 0.11.1 hangs an overflowing space whatever the `white-space`, and
/// commits the line after it. CSS Text 3 §4.1.3 hangs no `break-spaces`
/// space: one that does not fit starts the next line, and there is a break
/// opportunity after every one of them. There is none between a word and the
/// space after it (UAX #14 LB7), so the line, from `line_start` to
/// `units[end]`, ending in such a run, breaks:
///
/// - **after the last space of the run that fits**, when one does: the rest
///   start the next line (`abc` and ten spaces in 40px is `abc` and two, then
///   eight, in Chrome 155);
/// - **at the last opportunity before the word the run follows**, when not
///   even its first space fits and there is one: the word goes to the next
///   line with its space (`bullet text ` in 75px is `bullet ` / `text `);
/// - **after the run's first space**, when there is none: the word and that
///   space overflow, and the next line starts with the second space
///   (`abcdef  x` in 20px is `abcdef ` / ` x`). A run that starts the line is
///   broken after its first space the same way, however narrow the line.
///
/// A line committed here is committed by length, as [`unglue`] commits one:
/// the breaker has no opportunity of its own between two spaces.
fn break_overflowing_spaces(
    breaker: &mut parley::layout::BreakLines<'_, Brush>,
    units: &[LineUnit],
    cursor: usize,
    end: usize,
    line_start: &parley::layout::BreakerState,
    max: f32,
) -> Option<(parley::layout::BreakReason, f32)> {
    use parley::layout::{BreakReason, YieldData};
    const TOL: f32 = 0.005;
    // No following word fits in this much room: it only decides which side
    // of a width boundary a unit falls.
    const ROOM: f32 = 0.01;
    // The table's cursor can sit on the zero-width newline that ended the
    // line before; a newline is never the first unit of a line.
    let first = cursor + units[cursor..end].iter().take_while(|u| u.newline).count();
    let run = units[first..end]
        .iter()
        .rev()
        .take_while(|u| u.breaks)
        .count();
    if run == 0 {
        return None;
    }
    let run_start = end - run;
    let x = |p: usize| -> f32 { units[cursor..p].iter().map(|u| u.advance).sum() };
    let through = match (run_start + 1..=end).rev().find(|&p| x(p) <= max + TOL) {
        // It fits as it is.
        Some(p) if p == end => return None,
        Some(p) => p,
        None => {
            let word_x = x(run_start);
            let mut wraps_in_word = false;
            if run_start > first && word_x > ROOM {
                breaker.revert_to(line_start.clone());
                let room = word_x - ROOM;
                let state = breaker.state_mut();
                state.set_layout_max_advance(room);
                state.set_line_max_advance(room);
                if let Some(YieldData::LineBreak(d)) = breaker.break_next() {
                    if d.reason == BreakReason::Emergency {
                        wraps_in_word = true;
                    } else if d.metrics.advance <= room + TOL {
                        breaker.set_prior_line_width(max);
                        return Some((d.reason, d.metrics.advance));
                    }
                }
            }
            if wraps_in_word {
                run_start
            } else {
                run_start + 1
            }
        }
    };
    let n = units[first..through]
        .iter()
        .map(|u| u.counted as usize)
        .sum::<usize>();
    breaker.revert_to(line_start.clone());
    breaker.break_next_with_length(u32::try_from(n).ok()?)?;
    breaker.set_prior_line_width(max);
    Some((BreakReason::Regular, x(through)))
}

/// The preserved text of an inline formatting context whose spaces and tabs
/// do not hang at a soft wrap, as sorted, disjoint flat byte ranges.
#[derive(Default)]
pub(crate) struct UnhungSpaces {
    /// `pre` text (#1212): it wraps nowhere, so CSS Text 3 §4.1.3 hangs none
    /// of its spaces and a line ending in them is aligned with them.
    pub pre: Vec<std::ops::Range<usize>>,
    /// `break-spaces` text (#1043): its spaces never hang
    /// ([`LineUnit::breaks`]).
    pub breaking: Vec<std::ops::Range<usize>>,
    /// Text whose element does not wrap (`nowrap`, `pre`), filled only for
    /// an IFC holding edge boxes (#1426) or [`Self::seams`]: no soft wrap
    /// after it ([`LineUnit::wrap_after`]).
    pub nowrap: Vec<std::ops::Range<usize>>,
    /// Where `white-space` allows a soft wrap after a text node or an atomic
    /// inline, in a paragraph broken at a width that holds atomic inlines
    /// and some element that does not wrap (#1238); `None` otherwise, where
    /// parley's own rule around boxes is CSS's.
    pub seams: Option<WrapSeams>,
    /// [`Self::seams`] are for atomic inlines next to an element that does
    /// not wrap (#1238): always in force.
    pub box_seams: bool,
    /// The paragraph holds a `pre` element's spaces and is broken at a
    /// width (#1243): [`Self::seams`] are filled, and in force when
    /// [`pre_seams_needed`] says so of parley's lines ([`Self::pre_seams`]).
    pub pre_candidate: bool,
    /// The paragraph takes [`Self::seams`] for a `pre` element's spaces
    /// (#1243): [`pre_seam`] and the rules only such a paragraph gets. Set
    /// by [`break_lines_hanging_spaces`] once parley has broken it.
    pub pre_seams: std::cell::Cell<bool>,
    /// Text whose element collapses its spaces (`normal`, `nowrap`,
    /// `pre-line`), filled only with [`Self::pre_seams`]
    /// ([`LineUnit::collapsible`]).
    pub collapsible: Vec<std::ops::Range<usize>>,
    /// Where a text node whose spaces collapsed away entirely stands, in an
    /// element the text before it closes into, as a sorted flat byte;
    /// filled only with [`Self::pre_candidate`]
    /// ([`RinchDocument::collapsed_after_close`], [`pre_seam`]).
    pub collapsed: Vec<usize>,
}

/// Whether `white-space` allows a soft wrap after each text node and each
/// atomic inline of an inline formatting context, given what follows it
/// (#1238).
///
/// parley 0.11.1 allows a break after every inline box and before one when
/// the character before it wraps, whatever the `white-space` around the
/// box. Chrome 155 (and css-text-3 §5) asks the white space of the element
/// the box or text is in — an inline-block's own `white-space` styles its
/// contents and plays no part — and, at an element's end, that of the
/// element it closes into: Chrome's line breaker computes the opportunity
/// after an item with the `white-space` in effect there, and again at each
/// close tag after it. So the opportunity after a leaf (a text node or a box)
/// is allowed when any element from the leaf's parent up to the nearest one
/// that also contains the next leaf wraps: `aa<span normal>[1][2][3]</span>cc`
/// in a `nowrap` root breaks after `[1]`, `[2]` and `[3]` but not between
/// `aa` and `[1]` (the root holds both). Only the edge of a leaf is decided
/// here; inside a text node the break is its element's, as parley has it.
///
/// A run of spaces is different (review of #1878): Chrome 155 decides its
/// opportunity by the spaces' own `white-space`, or, when the spaces are in
/// an element the text before them closes into, by any wrapping element
/// that closes there ([`Seam::closes_into_wrap`], [`wrap_allowed_after`]).
#[derive(Default)]
pub(crate) struct WrapSeams {
    /// The flat byte each text node's run ends at, ascending, and its seam.
    text_ends: Vec<(usize, Seam)>,
    /// Each atomic inline's id, ascending, and its seam.
    boxes: Vec<(u64, Seam)>,
}

/// What [`WrapSeams`] knows about the end of one leaf.
#[derive(Clone, Copy, Default)]
pub(crate) struct Seam {
    /// A soft wrap may follow the leaf.
    wraps: bool,
    /// The next leaf's element contains this one's, and an element that
    /// closes between them (inside that one) wraps: `<span normal>hello
    /// </span>` then the root's `nowrap` space. What decides a break after
    /// a run of spaces that follows the leaf.
    closes_into_wrap: bool,
    /// An element closes between this leaf and the next into one that does
    /// not wrap: `<span pre-wrap>aa  </span>` then a `pre` root's spaces.
    /// What makes the break between a wrapping run of spaces and a `pre`
    /// one an ordinary break ([`pre_seam`], #1243).
    closes_into_nowrap: bool,
    /// Elements close between this leaf and the next, and none opens: the
    /// next leaf's element is a proper ancestor of this one's
    /// (`<span>dd</span>` then its parent's space; [`pre_seam`]).
    closes: bool,
}

impl WrapSeams {
    /// The seams of an IFC whose text runs are `text_ranges` and whose
    /// inline boxes are `layout`'s.
    pub(crate) fn of(
        nodes: &slab::Slab<Node>,
        root_id: usize,
        text_ranges: &[crate::node::IfcTextRange],
        layout: &parley::Layout<Brush>,
    ) -> Self {
        // Every leaf in logical order: (flat byte, 0 = box / 1 = text, its
        // index), its node. A box at byte `i` comes before the text at `i`.
        let mut leaves: Vec<((usize, u8, usize), usize)> = text_ranges
            .iter()
            .enumerate()
            .filter(|(_, r)| r.flat_end > r.flat_start)
            .map(|(i, r)| ((r.flat_start, 1, i), r.node_id))
            .collect();
        for (i, b) in layout.inline_boxes().enumerate() {
            if !is_edge_box(b.id) && b.kind == parley::InlineBoxKind::InFlow {
                leaves.push(((b.index, 0, i), b.id as usize));
            }
        }
        leaves.sort_unstable_by_key(|(k, _)| *k);
        let mut seams = Self::default();
        for (i, &((_, kind, index), node)) in leaves.iter().enumerate() {
            let next = leaves.get(i + 1).map(|&(_, next)| next);
            let seam = Seam {
                wraps: next.is_none_or(|next| wraps_between(nodes, root_id, node, next)),
                closes_into_wrap: next
                    .is_some_and(|next| closes_into_wrap(nodes, root_id, node, next)),
                closes_into_nowrap: next
                    .is_some_and(|next| closes_into_nowrap(nodes, root_id, node, next)),
                closes: next.is_some_and(|next| only_closes(nodes, root_id, node, next)),
            };
            if kind == 0 {
                seams.boxes.push((node as u64, seam));
            } else {
                seams.text_ends.push((text_ranges[index].flat_end, seam));
            }
        }
        seams.boxes.sort_unstable_by_key(|&(id, _)| id);
        seams
    }

    /// The seam after the text run that ends at `byte`; `None` when no run
    /// ends there.
    fn after_text(&self, byte: usize) -> Option<Seam> {
        let i = self.text_ends.partition_point(|&(b, _)| b < byte);
        self.text_ends
            .get(i)
            .filter(|&&(b, _)| b == byte)
            .map(|&(_, w)| w)
    }

    /// The seam after the atomic inline `id`.
    fn after_box(&self, id: u64) -> Option<Seam> {
        let i = self.boxes.partition_point(|&(b, _)| b < id);
        self.boxes
            .get(i)
            .filter(|&&(b, _)| b == id)
            .map(|&(_, w)| w)
    }
}

/// Whether `next`'s parent element contains `leaf`, and some element from
/// `leaf`'s parent up to (not including) it wraps ([`Seam::closes_into_wrap`]).
fn closes_into_wrap(nodes: &slab::Slab<Node>, root_id: usize, leaf: usize, next: usize) -> bool {
    let Some(target) = nodes.get(next).and_then(|n| n.parent) else {
        return false;
    };
    let mut wraps = false;
    let mut el = nodes.get(leaf).and_then(|n| n.parent);
    while let Some(id) = el {
        if id == target {
            return wraps;
        }
        if id == root_id {
            return false;
        }
        let Some(node) = nodes.get(id) else {
            return false;
        };
        wraps |= RinchDocument::text_wrap_mode(&node.computed_style)
            == parley::style::TextWrapMode::Wrap;
        el = node.parent;
    }
    false
}

/// Whether `next`'s parent element is a proper ancestor of `leaf`'s: elements
/// close between them and none opens ([`Seam::closes`]).
fn only_closes(nodes: &slab::Slab<Node>, root_id: usize, leaf: usize, next: usize) -> bool {
    let Some(target) = nodes.get(next).and_then(|n| n.parent) else {
        return false;
    };
    let mut el = nodes.get(leaf).and_then(|n| n.parent);
    if el == Some(target) {
        return false;
    }
    while let Some(id) = el {
        if id == target {
            return true;
        }
        if id == root_id {
            return false;
        }
        el = nodes.get(id).and_then(|n| n.parent);
    }
    false
}

/// Whether an element that closes between `leaf` and `next` — one from
/// `leaf`'s parent up to, not including, the nearest that also contains
/// `next` — closes into one that does not wrap
/// ([`Seam::closes_into_nowrap`]).
fn closes_into_nowrap(nodes: &slab::Slab<Node>, root_id: usize, leaf: usize, next: usize) -> bool {
    let contains_next = |el: usize| {
        std::iter::successors(Some(next), |&id| {
            (id != root_id)
                .then(|| nodes.get(id).and_then(|n| n.parent))
                .flatten()
        })
        .any(|id| id == el)
    };
    let mut el = nodes.get(leaf).and_then(|n| n.parent);
    while let Some(id) = el {
        if id == root_id || contains_next(id) {
            return false;
        }
        let Some(parent) = nodes.get(id).and_then(|n| n.parent) else {
            return false;
        };
        let Some(into) = nodes.get(parent) else {
            return false;
        };
        if RinchDocument::text_wrap_mode(&into.computed_style) != parley::style::TextWrapMode::Wrap
        {
            return true;
        }
        el = Some(parent);
    }
    false
}

/// Whether some element from `leaf`'s parent up to the nearest one that also
/// contains `next` — the IFC root at the most — wraps ([`WrapSeams`]).
fn wraps_between(nodes: &slab::Slab<Node>, root_id: usize, leaf: usize, next: usize) -> bool {
    let contains_next = |el: usize| {
        let mut n = Some(next);
        while let Some(id) = n {
            if id == el {
                return true;
            }
            if id == root_id {
                return false;
            }
            n = nodes.get(id).and_then(|x| x.parent);
        }
        false
    };
    let mut el = nodes.get(leaf).and_then(|n| n.parent);
    while let Some(id) = el {
        let Some(node) = nodes.get(id) else {
            return true;
        };
        if RinchDocument::text_wrap_mode(&node.computed_style) == parley::style::TextWrapMode::Wrap
        {
            return true;
        }
        if id == root_id || contains_next(id) {
            return false;
        }
        el = node.parent;
    }
    true
}

impl UnhungSpaces {
    /// The seams in force: [`Self::box_seams`], or [`Self::pre_seams`].
    fn active_seams(&self) -> Option<&WrapSeams> {
        self.seams
            .as_ref()
            .filter(|_| self.box_seams || self.pre_seams.get())
    }
}

/// An anonymous block box `cleanup_anonymous_block_boxes` took out of its
/// container but kept in the slab, with the run it was minted for (#964).
/// `create_anonymous_block_boxes` mints that run's box into it, or frees it.
pub(crate) struct RetiredAnonBox {
    id: usize,
    parent: usize,
    run_members: Vec<usize>,
}

/// The boxes one pass retired, found by the first member of their run — a
/// node is a member of one run at most, so the key names one box — so that
/// matching every run of a document to its old box is linear in the runs, not
/// quadratic. A box whose run lost every member (they all moved away) can match
/// nothing and waits in `unmatched` to be freed.
#[derive(Default)]
pub(crate) struct RetiredAnonBoxes {
    by_first_member: HashMap<usize, RetiredAnonBox>,
    unmatched: Vec<usize>,
}

impl RetiredAnonBoxes {
    fn push(&mut self, r: RetiredAnonBox) {
        match r.run_members.first() {
            Some(&first) => {
                if let Some(other) = self.by_first_member.insert(first, r) {
                    self.unmatched.push(other.id);
                }
            }
            None => self.unmatched.push(r.id),
        }
    }

    /// The box retired from `parent` for exactly `run`, if there is one.
    fn take(&mut self, parent: usize, run: &[usize]) -> Option<RetiredAnonBox> {
        let first = *run.first()?;
        let r = self.by_first_member.get(&first)?;
        if r.parent != parent || r.run_members != run {
            return None;
        }
        self.by_first_member.remove(&first)
    }

    fn into_ids(self) -> impl Iterator<Item = usize> {
        self.by_first_member
            .into_values()
            .map(|r| r.id)
            .chain(self.unmatched)
    }
}

/// One cluster or inline box of a paragraph, in logical order: what the line
/// breaker adds to a line's advance, and what the hang pass needs to know.
struct LineUnit {
    advance: f32,
    /// A space or tab that hangs at a soft wrap: one whose element wraps
    /// (not NBSP, and not a `pre` element's).
    hangs: bool,
    /// A space or tab of a `pre` element (#1212): preserved, and CSS Text 3
    /// §4.1.3 hangs a preserved space only where the text wraps, so the hang
    /// pass does not widen a line for it and a line ending in it is aligned
    /// with it as content. (parley hangs the one that overflows right after
    /// wrapping spaces; such a paragraph takes the seams path, #1243.) A
    /// `break-spaces` space or tab is one too: it never hangs.
    unhung: bool,
    /// A space or tab of a `break-spaces` element (#1043): it never hangs,
    /// one that does not fit starts the next line, and there is a break
    /// opportunity after it ([`break_overflowing_spaces`]).
    breaks: bool,
    newline: bool,
    /// A no-break space (U+00A0), which parley hangs and CSS glues (#1218).
    nbsp: bool,
    /// A space or NBSP cluster (parley's `is_space_or_nbsp`). A preserved tab
    /// is one too: it is pushed to parley as a space (#1122).
    space: bool,
    /// What [`parley::layout::BreakLines::break_next_with_length`] counts
    /// for it: a cluster's characters — parley 0.12's clusters are grapheme
    /// clusters, `ง่` is one cluster of two — an in-flow inline box 1, an
    /// out-of-flow box 0.
    counted: u32,
    /// An in-flow inline box.
    inline_box: bool,
    /// A hyphen or another UAX #14 class HY/BA character that is not white
    /// space: LB12a allows a break between it and an NBSP after it.
    breaks_before_glue: bool,
    /// An inline element's edge box (#1426): [`EDGE_START`] or [`EDGE_END`],
    /// else 0. Not an [`Self::inline_box`].
    edge: u8,
    /// `white-space` allows a soft wrap after it: a cluster of text whose
    /// element wraps, and at the end of a text run or after an atomic
    /// inline what [`WrapSeams`] answers (#1238). Always for an edge box,
    /// whose run [`edge_break_allowed`] looks past.
    wrap_after: bool,
    /// A cluster of text whose own element wraps (not in
    /// [`UnhungSpaces::nowrap`]); what decides a break after a space
    /// ([`wrap_allowed_after`]).
    own_wrap: bool,
    /// [`Seam::closes_into_wrap`] at the end of a text run or after an
    /// atomic inline.
    closes_into_wrap: bool,
    /// [`Seam::closes_into_nowrap`] at the end of a text run or after an
    /// atomic inline.
    closes_into_nowrap: bool,
    /// The last cluster of a text node, known only with [`WrapSeams`]
    /// ([`pre_seam`]).
    text_end: bool,
    /// [`Seam::closes`] at the end of a text run.
    closes: bool,
    /// A cluster of text whose element collapses its spaces, known only
    /// with [`UnhungSpaces::pre_seams`] ([`pre_seam`]).
    collapsible: bool,
    /// A text node whose spaces collapsed away, in an element the text before
    /// it closes into, stands right before this cluster
    /// ([`UnhungSpaces::collapsed`], [`pre_seam`]).
    after_collapsed: bool,
    /// The byte a cluster starts at, or a box stands at.
    byte: usize,
}

/// Whether a line may end before `units[e]` as far as edge boxes go (#1426).
///
/// parley 0.11.1 allows a break after every in-flow inline box and before
/// one that does not fit, which is right for an atomic inline and wrong for
/// an edge box ([`EDGE_BOX`]): an element's start padding belongs with its
/// first content and its end padding with its last. So where edge boxes meet
/// the break, CSS's answer is asked instead: the break must fall after the
/// run's end boxes and before its start boxes, and there must be an
/// opportunity between what precedes and what follows the run — `white-space`
/// must allow one after what precedes ([`LineUnit::wrap_after`]), and then a
/// UAX #14 opportunity in the text, or an atomic inline on either side. With
/// no edge box next to the break, that is the whole question, which
/// [`css_line`] asks at every unit (#1238). With `emergency` (an
/// `overflow-wrap` break) only the side of the boxes is checked.
fn edge_break_allowed(
    units: &[LineUnit],
    breaks: &TextBreaks<'_>,
    e: usize,
    emergency: bool,
    between_spaces: bool,
    pre_seams: bool,
) -> bool {
    if e == 0 {
        return false;
    }
    if e >= units.len() {
        return true;
    }
    let mut i0 = e;
    while i0 > 0 && units[i0 - 1].edge != 0 {
        i0 -= 1;
    }
    let mut i1 = e;
    while i1 < units.len() && units[i1].edge != 0 {
        i1 += 1;
    }
    let split = i0
        + units[i0..i1]
            .iter()
            .take_while(|u| u.edge == EDGE_END)
            .count();
    if i0 < i1 && e != split {
        return false;
    }
    if emergency {
        return true;
    }
    let Some(p) = i0.checked_sub(1).map(|i| &units[i]) else {
        return false;
    };
    let Some(n) = units.get(i1) else {
        return true;
    };
    if !wrap_allowed_after(units, i0) {
        return false;
    }
    // With `between_spaces` ([`css_line`]'s scan), `break-spaces` offers an
    // opportunity after every space (css-text-3 §4.1.3), which UAX #14 does
    // not between two spaces: Chrome 155 puts trailing ones that overflow
    // before a forced break on a line of their own (not the forced break
    // itself, which ends the line the last one is on). A
    // break parley made there is still looked at again: its line has a
    // space that does not fit and does not hang (#1043).
    if p.newline || p.inline_box || n.inline_box || (between_spaces && p.breaks && !n.newline) {
        return true;
    }
    // A wrapping space then a `break-spaces` one: the first hangs, the
    // second starts the next line (Chrome 155: ` gggcccc ` / `        b`
    // for ` ggg<span normal>cccc  </span>        <span pre>b</span>` in a
    // 70px `break-spaces` root; #1243). Only in a paragraph that takes the
    // seams for `pre` spaces: elsewhere (the #1426 edge-box path) it is
    // left as it was.
    if between_spaces && pre_seams && is_plain_space(p) && p.own_wrap && n.breaks {
        return true;
    }
    // A run of spaces that does not wrap, in an element a wrapping one
    // closes into right before it, breaks before the run (Chrome 155 keeps
    // the spaces at the start of the next line), where UAX #14 has no break
    // before a space.
    if is_plain_space(n) && !n.own_wrap && !is_plain_space(p) && p.closes_into_wrap {
        return true;
    }
    // And a run of spaces that wraps then one that does not breaks between
    // them: the first hangs, the second starts the next line. Before a `pre`
    // element's spaces, [`css_line`] takes it as [`pre_seam`] says (#1243).
    if is_plain_space(n) && !n.own_wrap && is_plain_space(p) && p.own_wrap {
        return true;
    }
    breaks.at(n.byte)
}

/// How Chrome 155 takes the break between a run of spaces that wraps and a
/// `pre` element's spaces after it (#1243), which [`edge_break_allowed`]
/// allows; `None` for any other break. Chrome's line breaker finds it by
/// the text items (text nodes) the run of wrapping spaces lies in, measured
/// on `w  <span pre>  dd</span>` and its variants in a 50px box, with a
/// `w` that fits (`aaaa`) and one that overflows (`hhhhhhh`):
///
/// - the wrapping spaces' element closes there into one that does not wrap
///   ([`Seam::closes_into_nowrap`]: `<span pre-wrap>hhhhhhh  </span>` then a
///   `pre` root's spaces): an ordinary break;
/// - the run is the end of the text node of the word before it
///   (`hhhhhhh  <span pre>…`): only when what is before it fits
///   ([`PreSeam::RewindOnly`]) — Chrome rewinds to it from an overflow later
///   in the line, and does not stop at it when the word already overflows;
/// - the run is a text node of its own after the word (`hhhhhhh<span
///   pre-wrap>  </span><span pre>…`, or the word and the run split into
///   two text nodes of one element), whether the word wraps or not: only
///   when the line overflows before the `pre` spaces, in the word or in the
///   run ([`PreSeam::ForwardOnly`]); `aaaa<span pre-wrap>  </span><span
///   pre>  dd</span>` stays on one line, as does `<span pre>gggg</span><span
///   normal> </span><span pre> a</span>`;
/// - except that a run of *collapsible* spaces right after a wrapping
///   element that closes there ([`Seam::closes`]: `<span>dd</span> <span
///   pre>    dd</span>`
///   in a `normal` or `pre-line` root) is an ordinary break, taken whether
///   the word fits or overflows (review of #1897: of 100 such variants in
///   `normal` and `pre-line` roots Chrome lays out 79 as it lays out the
///   word unwrapped, and rinch now matches Chrome on 95, main on 60; a
///   preserved run, in a `pre-wrap` root, is the text node of its own
///   above);
/// - the run starts in the word's text node and goes on into another: an
///   ordinary break. A text node whose spaces collapsed away counts when it
///   is in an element the run's own element closes into
///   ([`UnhungSpaces::collapsed`]): `<span>…lvWa.   </span> <span pre>  bee…`
///   breaks before the `pre` spaces, `aaaa ` and ` ` as two text nodes of
///   one element do not (review of #1897).
///
/// A run after an inline box, or at the start of the paragraph, is an
/// ordinary break, as #1238 has it. Edge boxes at the break are looked
/// past, as [`edge_break_allowed`] looks past them. Not every case is
/// Chrome's (#1898).
fn pre_seam(units: &[LineUnit], e: usize) -> Option<PreSeam> {
    let mut i0 = e.min(units.len());
    while i0 > 0 && units[i0 - 1].edge != 0 {
        i0 -= 1;
    }
    let mut i1 = e;
    while i1 < units.len() && units[i1].edge != 0 {
        i1 += 1;
    }
    let p = &units[i0.checked_sub(1)?];
    let n = units.get(i1)?;
    if !(is_plain_space(p) && p.own_wrap && is_plain_space(n) && !n.own_wrap && n.unhung) {
        return None;
    }
    let run = units[..i0]
        .iter()
        .rev()
        .take_while(|u| is_plain_space(u) && u.own_wrap)
        .count();
    let r0 = i0 - run;
    let w = r0
        .checked_sub(1)
        .map(|i| &units[i])
        .filter(|w| !w.inline_box && w.edge == 0 && !w.newline);
    if let Some(w) = w.filter(|w| w.text_end) {
        // A run of collapsible spaces right after an element that closes
        // there (`<span>dd</span> <span pre>…` in a `normal` root) breaks
        // both ways: when the word fits, and after it when it overflows.
        return Some(if w.closes && w.own_wrap && units[r0].collapsible {
            PreSeam::Ordinary
        } else {
            PreSeam::ForwardOnly
        });
    }
    if w.is_none() || p.closes_into_nowrap {
        return Some(PreSeam::Ordinary);
    }
    // A run whose element closes into a text node whose spaces collapsed
    // away (`<span>…lvWa.   </span> <span pre>…`: the root's space between)
    // goes on into another text node, as Chrome's items have it.
    Some(
        if n.after_collapsed || units[r0..i0 - 1].iter().any(|u| u.text_end) {
            PreSeam::Ordinary
        } else {
            PreSeam::RewindOnly
        },
    )
}

/// [`pre_seam`]'s answer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PreSeam {
    /// Taken as any break is.
    Ordinary,
    /// Taken only when what is before it fits.
    RewindOnly,
    /// Taken only when the line, the run of spaces included, overflows
    /// before it.
    ForwardOnly,
}

/// Whether `white-space` allows a soft wrap after `units[..end]`, edge boxes
/// at its end looked past: the [`LineUnit::wrap_after`] of its last unit, or
/// after a newline. After a run of spaces (not `break-spaces` ones, which
/// offer an opportunity after each) it is the spaces' own `white-space`
/// ([`LineUnit::own_wrap`]), not the element they close into (review of
/// #1878, Chrome 155): `<span nowrap>abc def </span>[box]` in a `normal`
/// root has none. Spaces that do not wrap, in an element a wrapping one
/// closes into right before them, have their opportunity before the run
/// instead ([`edge_break_allowed`]): `aa<span normal>hello</span> ccc` in a
/// `nowrap` or `pre` root breaks after `hello` and keeps the space at the
/// start of the next line (but `<span normal>ccc<span nowrap> mm</span>`
/// does not break: the `nowrap` element opens there).
fn wrap_allowed_after(units: &[LineUnit], end: usize) -> bool {
    let mut i = end.min(units.len());
    while i > 0 && units[i - 1].edge != 0 {
        i -= 1;
    }
    let Some(p) = i.checked_sub(1).map(|i| &units[i]) else {
        return true;
    };
    if is_plain_space(p) {
        return p.own_wrap;
    }
    p.newline || p.wrap_after
}

/// A space or tab that is not a no-break space nor a `break-spaces` one: a
/// run of them has one opportunity ([`wrap_allowed_after`]).
fn is_plain_space(u: &LineUnit) -> bool {
    u.space && !u.nbsp && !u.breaks
}

/// The UAX #14 break opportunities of a paragraph's text, as parley's own
/// analysis finds them: the same segmenter, with the complex-script
/// dictionaries parley's `complex-scripts` feature turns on, so a line this
/// re-breaks keeps the breaks inside Thai, Lao, Khmer and Burmese words
/// (review of #1878). Segmented once, the first time a break is asked
/// about, over the whole text, as parley segments it (parley splits it
/// where `word-break` changes, which rinch does not pass it). Less one kind:
/// a break between a complex-context (`SA`) character and a letter or
/// digit of another script, which ICU4X's dictionary segmenter reports and
/// UAX #14 does not (LB1 resolves `SA` to `AL`, and LB28 keeps `AL AL`
/// together): Chrome 155 has none between `ภาษาไทยง่ายมาก` and `w`.
struct TextBreaks<'t> {
    text: &'t str,
    positions: std::cell::OnceCell<Vec<usize>>,
}

impl<'t> TextBreaks<'t> {
    fn new(text: &'t str) -> Self {
        Self {
            text,
            positions: std::cell::OnceCell::new(),
        }
    }

    /// Whether there is an opportunity at `byte`.
    fn at(&self, byte: usize) -> bool {
        if chrome_glues_ascii(self.text, byte) {
            return false;
        }
        self.positions
            .get_or_init(|| {
                let mut options = icu_segmenter::options::LineBreakOptions::default();
                options.word_option = Some(icu_segmenter::options::LineBreakWordOption::Normal);
                icu_segmenter::LineSegmenter::new_dictionary(options)
                    .segment_str(self.text)
                    .filter(|&b| !complex_context_seam(self.text, b))
                    .collect()
            })
            .binary_search(&byte)
            .is_ok()
    }
}

/// Whether `byte` of `text` lies between `!`, `/`, `|` or `}` and an ASCII
/// letter or digit, where UAX #14 (and ICU4X, parley's segmenter) has a
/// break and Chrome 155 does not: `cccc!dddd`, `cccc/dddd`, `cccc}1234` stay
/// whole in a box too narrow for them (review of #1897, measured over every
/// ASCII punctuation mark before and after a letter and a digit). Only the
/// lines [`css_line`] breaks (#1426, #1238, #1243) ask it: elsewhere
/// parley's own breaks still split there (a separate difference, #1910).
/// `?` and the other marks agree with ICU4X.
fn chrome_glues_ascii(text: &str, byte: usize) -> bool {
    let (Some(before), Some(after)) = (
        text.get(..byte).and_then(|t| t.chars().next_back()),
        text.get(byte..).and_then(|t| t.chars().next()),
    ) else {
        return false;
    };
    matches!(before, '!' | '/' | '|' | '}') && after.is_ascii_alphanumeric()
}

/// Whether `byte` of `text` lies between a complex-context (`SA`) character
/// and a letter or digit (`AL`, `HL`, `NU`) — UAX #14 has no break there
/// ([`TextBreaks`]).
fn complex_context_seam(text: &str, byte: usize) -> bool {
    use icu_properties::CodePointMapData;
    use icu_properties::props::LineBreak;
    let (Some(before), Some(after)) = (
        text.get(..byte).and_then(|t| t.chars().next_back()),
        text.get(byte..).and_then(|t| t.chars().next()),
    ) else {
        return false;
    };
    let lb = CodePointMapData::<LineBreak>::new();
    let letter = |c: char| {
        matches!(
            lb.get(c),
            LineBreak::Alphabetic | LineBreak::HebrewLetter | LineBreak::Numeric
        )
    };
    let sa = |c: char| lb.get(c) == LineBreak::ComplexContext;
    (sa(before) && letter(after)) || (letter(before) && sa(after))
}

/// Where CSS ends the line that starts at `units[cursor]`, by the
/// opportunities [`edge_break_allowed`] allows (#1426, #1238): the unit one
/// past its end, and its advance.
///
/// That is the last allowed break whose content (its hanging spaces left
/// out) fits `max`, or, when none fits, the first allowed break — the
/// unbreakable stretch overflows, as in CSS — or a forced break, whichever
/// comes first.
fn css_line(
    units: &[LineUnit],
    breaks: &TextBreaks<'_>,
    cursor: usize,
    max: f32,
    pre_seams: bool,
) -> (usize, f32) {
    // The table's cursor can sit on the zero-width newline that ended the
    // line before; a newline is never the first unit of a line.
    let first = cursor + units[cursor..].iter().take_while(|u| u.newline).count();
    let mut x = 0.0f32;
    // The run of hanging spaces the line would end in.
    let mut hanging = 0.0f32;
    let mut fit: Option<(usize, f32)> = None;
    let mut chosen = None;
    for e in first + 1..=units.len() {
        let u = &units[e - 1];
        if u.newline {
            // The forced break ends the line, unless what is before it
            // overflows and an earlier break fits.
            chosen = Some(fit.filter(|_| x - hanging > max + 0.005).unwrap_or((e, x)));
            break;
        }
        x += u.advance;
        // An element's end edge box right after hanging spaces hangs with
        // them (review of #1878: Chrome 155 fits `<span padding:0 3px>[B]q-r
        // </span>` and a forced break in 71.5px, the content less the space
        // and the end padding, 71.18px; at 71px it breaks after `q-`).
        hanging = if u.hangs || (u.edge == EDGE_END && hanging > 0.0) {
            hanging + u.advance
        } else {
            0.0
        };
        if !edge_break_allowed(units, breaks, e, false, true, pre_seams) {
            continue;
        }
        let seam = pre_seams.then(|| pre_seam(units, e)).flatten();
        if seam == Some(PreSeam::ForwardOnly) && x <= max + 0.005 {
            continue;
        }
        if x - hanging <= max + 0.005 {
            fit = Some((e, x));
        } else if seam != Some(PreSeam::RewindOnly) {
            chosen = Some(fit.unwrap_or((e, x)));
            break;
        }
    }
    chosen.or(fit).unwrap_or((units.len(), x))
}

/// Commit, in place of the line parley just ended, the line from
/// `units[cursor]` to `units[e]` (exclusive) whose advance is `advance`
/// ([`css_line`]), and return its reason and advance; `None` when the unit
/// table and the breaker disagree.
fn commit_css_line(
    breaker: &mut parley::layout::BreakLines<'_, Brush>,
    units: &[LineUnit],
    cursor: usize,
    (e, advance): (usize, f32),
    line_start: &parley::layout::BreakerState,
    max: f32,
) -> Option<(parley::layout::BreakReason, f32)> {
    use parley::layout::BreakReason;
    let first = cursor + units[cursor..].iter().take_while(|u| u.newline).count();
    let n = units[first..e]
        .iter()
        .map(|u| u.counted as usize)
        .sum::<usize>();
    breaker.revert_to(line_start.clone());
    breaker.break_next_with_length(u32::try_from(n).ok()?)?;
    breaker.set_prior_line_width(max);
    let reason = if e >= units.len() {
        BreakReason::None
    } else if units[e - 1].newline {
        BreakReason::Explicit
    } else {
        BreakReason::Regular
    };
    Some((reason, advance))
}

/// Whether a line's content — its units less the spaces that hang at its
/// end, end edge boxes after them counted as content ([`css_line`]) — is
/// wider than `max`. With seams every soft wrap is looked at again
/// (#1243); this sends the last line and one before a forced break through
/// [`css_line`] too when it overflows.
fn line_overflows(line: &[LineUnit], max: f32) -> bool {
    let advance: f32 = line.iter().map(|u| u.advance).sum();
    // The trailing run of hanging spaces and end edge boxes, from its
    // first space on (as [`css_line`] counts it).
    let run = line
        .iter()
        .rev()
        .take_while(|u| u.hangs || u.edge == EDGE_END)
        .count();
    let hanging: f32 = line[line.len() - run..]
        .iter()
        .skip_while(|u| !u.hangs)
        .map(|u| u.advance)
        .sum();
    advance - hanging > max + 0.005
}

/// Whether a paragraph that holds a `pre` element's spaces takes the seams
/// for them (#1243), asked of its text and of the lines parley broke it
/// into: a `pre` element's text that starts with a space or tab right after
/// text that wraps (a run of wrapping spaces before it is a [`pre_seam`],
/// where parley has no break; a word of a wrapping element that closes
/// there has its break before the `pre` run, [`Seam::closes_into_wrap`]),
/// or a soft-wrapped line that parley ended in a `pre` space (it hung one,
/// breaking the `pre` run) or right before `pre` text (it went back to its
/// last break when the `pre` text overflowed, where CSS may hang the spaces
/// before it, or break later). None of these, and parley's breaks stand: a
/// paragraph whose `pre` spaces start it, or follow other text that does
/// not wrap, and whose lines parley does not end next to them, pays no pass
/// for them.
fn pre_seams_needed(layout: &parley::Layout<Brush>, text: &str, unhung: &UnhungSpaces) -> bool {
    use parley::layout::BreakReason;
    let is_space = |b: usize| matches!(text.as_bytes().get(b), Some(b' ' | b'\t'));
    let pre_space = |b: usize| is_space(b) && in_ranges(&unhung.pre, b);
    let seam = unhung.pre.iter().any(|r| {
        r.start > 0
            && pre_space(r.start)
            && !in_ranges(&unhung.pre, r.start - 1)
            && !in_ranges(&unhung.nowrap, r.start - 1)
    });
    seam || layout.lines().any(|line| {
        matches!(
            line.break_reason(),
            BreakReason::Regular | BreakReason::Emergency
        ) && {
            let e = line.text_range().end;
            (e > 0 && pre_space(e - 1)) || (e < text.len() && in_ranges(&unhung.pre, e))
        }
    })
}

/// Where a paragraph's `pre` spaces are, for the cheap test that sends a
/// soft-wrapped line of it to [`css_line`] (#1243): only a line that can
/// reach a `pre` space — one on the line, or the next one after it within
/// the room — can end elsewhere for a [`pre_seam`] than where parley ended
/// it. Built once per pass.
struct NearPre {
    /// The first `pre` space at or after each unit (`units.len()` for none).
    next: Vec<usize>,
    /// The advance of `units[..i]`.
    x: Vec<f32>,
}

impl NearPre {
    fn new(units: &[LineUnit]) -> Self {
        let mut next = vec![units.len(); units.len() + 1];
        for i in (0..units.len()).rev() {
            next[i] = if units[i].unhung && !units[i].breaks {
                i
            } else {
                next[i + 1]
            };
        }
        let mut x = Vec::with_capacity(units.len() + 1);
        let mut acc = 0.0f32;
        x.push(acc);
        for u in units {
            acc += u.advance;
            x.push(acc);
        }
        Self { next, x }
    }

    /// Whether the line from `units[cursor]` to `units[end]` holds a `pre`
    /// space, or the first one after it is within `max` of the line's start,
    /// the hanging spaces right before it left out.
    fn touches(&self, units: &[LineUnit], cursor: usize, end: usize, max: f32) -> bool {
        let p = self.next[cursor.min(units.len())];
        if p >= units.len() {
            return false;
        }
        if p <= end {
            return true;
        }
        let hanging: f32 = units[cursor..p]
            .iter()
            .rev()
            .take_while(|u| u.hangs)
            .map(|u| u.advance)
            .sum();
        self.x[p] - self.x[cursor] - hanging <= max + 0.005
    }
}

/// Whether a line ending before `units[a]` ends where one ending before
/// `units[b]` does: nothing but zero-width units (a newline, an out-of-flow
/// box) lies between them.
fn same_end(units: &[LineUnit], a: usize, b: usize) -> bool {
    units[a.min(b)..a.max(b)].iter().all(|u| u.advance <= 0.005)
}

/// Whether some soft-wrapped line of `layout` begins or ends with an edge
/// box (#1426): the cheap test that sends a paragraph through [`hang_pass`]
/// for [`css_line`]. A break edge boxes are not next to is parley's
/// own, and a break next to one is always looked at again.
fn any_line_split_at_edge(layout: &parley::Layout<Brush>) -> bool {
    use parley::PositionedLayoutItem;
    use parley::layout::BreakReason;
    let edge = |item: Option<PositionedLayoutItem<'_, Brush>>| matches!(item, Some(PositionedLayoutItem::InlineBox(b)) if is_edge_box(b.id));
    if !layout.inline_boxes().any(|b| is_edge_box(b.id)) {
        return false;
    }
    // Items are in visual order: on a right-to-left line the logical end is
    // the first item, so both ends of every line next to a soft wrap are
    // looked at.
    let mut prev_soft = false;
    for line in layout.lines() {
        let soft = matches!(
            line.break_reason(),
            BreakReason::Regular | BreakReason::Emergency
        );
        if (prev_soft || soft) && (edge(line.items().next()) || edge(line.items().last())) {
            return true;
        }
        prev_soft = soft;
    }
    false
}

/// [`LineUnit::edge`] of an inline element's inline-start box.
const EDGE_START: u8 = 1;
/// [`LineUnit::edge`] of an inline element's inline-end box.
const EDGE_END: u8 = 2;

/// Whether `node`'s computed CSS `direction` is `ltr`: read from the nearest
/// node at or above it with a Stylo style (an anonymous block box has none
/// and inherits its container's). `ltr` when nothing has one.
fn css_direction_is_ltr(nodes: &slab::Slab<Node>, node: usize) -> bool {
    let mut at = Some(node);
    while let Some(id) = at {
        let Some(n) = nodes.get(id) else { break };
        let data = n.stylo_element_data.borrow();
        if let Some(primary) = data.as_ref().and_then(|d| d.styles.get_primary()) {
            return primary.writing_mode.is_bidi_ltr();
        }
        at = n.parent;
    }
    true
}

/// Whether `byte` is in one of the sorted, disjoint `ranges`.
fn in_ranges(ranges: &[std::ops::Range<usize>], byte: usize) -> bool {
    let i = ranges.partition_point(|r| r.end <= byte);
    ranges.get(i).is_some_and(|r| r.contains(&byte))
}

/// Every cluster and inline box of `layout`, in the logical order the line
/// breaker walks them. Read from an already broken layout: each cluster is on
/// exactly one line. A space in one of the `unhung` ranges is
/// [`LineUnit::unhung`], and in a `break-spaces` one [`LineUnit::breaks`] too.
fn logical_units(
    layout: &parley::Layout<Brush>,
    text: &str,
    unhung: &UnhungSpaces,
) -> Vec<LineUnit> {
    // (byte, 0 = inline box / 1 = cluster, index within its kind) → unit.
    let mut keyed: Vec<((usize, u8, usize), LineUnit)> = Vec::new();
    for line in layout.lines() {
        for run in line.runs() {
            for c in run.clusters() {
                let range = c.text_range();
                let n = keyed.len();
                let space = is_hanging_space(text, &c);
                let pre = space && in_ranges(&unhung.pre, range.start);
                let breaks = space && in_ranges(&unhung.breaking, range.start);
                let own_wrap = !in_ranges(&unhung.nowrap, range.start);
                let seam = unhung.active_seams().and_then(|s| s.after_text(range.end));
                keyed.push((
                    (range.start, 1, n),
                    LineUnit {
                        advance: c.advance(),
                        hangs: space && !pre && !breaks,
                        unhung: pre || breaks,
                        breaks,
                        newline: c.is_hard_line_break(),
                        nbsp: c.is_space_or_nbsp()
                            && text
                                .get(range.clone())
                                .is_some_and(|t| t.starts_with('\u{a0}')),
                        space: c.is_space_or_nbsp(),
                        counted: text
                            .get(range.clone())
                            .map_or(1, |t| t.chars().count() as u32),
                        inline_box: false,
                        breaks_before_glue: text
                            .get(range.clone())
                            .and_then(|t| t.chars().next_back())
                            .is_some_and(breaks_before_glue),
                        edge: 0,
                        wrap_after: seam.map_or(own_wrap, |s| s.wraps),
                        own_wrap,
                        closes_into_wrap: seam.is_some_and(|s| s.closes_into_wrap),
                        closes_into_nowrap: seam.is_some_and(|s| s.closes_into_nowrap),
                        text_end: seam.is_some(),
                        closes: seam.is_some_and(|s| s.closes),
                        collapsible: in_ranges(&unhung.collapsible, range.start),
                        after_collapsed: unhung.collapsed.binary_search(&range.start).is_ok(),
                        byte: range.start,
                    },
                ));
            }
        }
    }
    for (i, b) in layout.inline_boxes().enumerate() {
        let in_flow = matches!(b.kind, parley::InlineBoxKind::InFlow);
        let seam = unhung.active_seams().and_then(|s| s.after_box(b.id));
        let advance = if in_flow { b.width } else { 0.0 };
        keyed.push((
            (b.index, 0, i),
            LineUnit {
                advance,
                hangs: false,
                unhung: false,
                breaks: false,
                newline: false,
                nbsp: false,
                space: false,
                counted: u32::from(in_flow),
                inline_box: in_flow && !is_edge_box(b.id),
                breaks_before_glue: false,
                edge: match edge_box_owner(b.id) {
                    Some((_, false)) => EDGE_START,
                    Some((_, true)) => EDGE_END,
                    None => 0,
                },
                wrap_after: seam.is_none_or(|s| s.wraps),
                own_wrap: true,
                closes_into_wrap: seam.is_some_and(|s| s.closes_into_wrap),
                closes_into_nowrap: seam.is_some_and(|s| s.closes_into_nowrap),
                text_end: false,
                closes: false,
                collapsible: false,
                after_collapsed: false,
                byte: b.index,
            },
        ));
    }
    // A box at byte `i` comes before the text at `i` (the builder splits the
    // run there).
    keyed.sort_unstable_by_key(|(k, _)| *k);
    keyed.into_iter().map(|(_, u)| u).collect()
}

/// Where a line that starts at `units[start]` and has `advance` ends: the index
/// one past its last unit. `None` when the table cannot account for the advance
/// (it overshoots, or runs out) — the table and the breaker disagree.
///
/// The first unit at which the running sum reaches the advance is taken. Zero
/// width units right after it may belong to this line or the next; either way
/// they add nothing to the next line's sum, and a line that hangs a space ends
/// on that space, which has width.
fn line_end(units: &[LineUnit], start: usize, advance: f32) -> Option<usize> {
    const TOL: f32 = 0.005;
    if advance <= TOL {
        return Some(start);
    }
    let mut acc = 0.0f32;
    for (i, u) in units.iter().enumerate().skip(start) {
        acc += u.advance;
        if acc >= advance - TOL {
            return (acc <= advance + TOL).then_some(i + 1);
        }
    }
    None
}

/// Spaces and tabs hang; NBSP does not (§4.1.3 hangs white space, and NBSP is
/// not white space for this rule — it glues).
fn is_hanging_space(text: &str, c: &parley::layout::Cluster<'_, Brush>) -> bool {
    c.is_space_or_nbsp() && matches!(text.get(c.text_range()), Some(" " | "\t"))
}

/// Whether `c` is of UAX #14 class HY or BA and not white space — after
/// which LB12a allows a break before a no-break space (the common members;
/// spaces and tabs hang instead).
fn breaks_before_glue(c: char) -> bool {
    matches!(
        c,
        '-' | '|'
            | '\u{ad}'
            | '\u{58a}'
            | '\u{5be}'
            | '\u{2010}'
            | '\u{2012}'
            | '\u{2013}'
            | '\u{2027}'
    )
}

/// Whether some line holds an inline box parley placed by its emergency
/// branch and the next line starts with a space, tab or newline (#1081): the
/// cheap test that sends a paragraph through [`hang_pass`] for
/// [`fold_after_overflowing_box`].
fn any_emergency_box_line_before_white_space(layout: &parley::Layout<Brush>, text: &str) -> bool {
    use parley::PositionedLayoutItem;
    use parley::layout::BreakReason;
    let mut lines = layout.lines().peekable();
    while let Some(line) = lines.next() {
        if line.break_reason() != BreakReason::Emergency
            || !line.items().last().is_some_and(
                |item| matches!(item, PositionedLayoutItem::InlineBox(b) if !is_edge_box(b.id)),
            )
        {
            continue;
        }
        if let Some(next) = lines.peek()
            && text
                .get(next.text_range().start..)
                .and_then(|t| t.chars().next())
                .is_some_and(|c| matches!(c, ' ' | '\t' | '\n'))
        {
            return true;
        }
    }
    false
}

impl RinchDocument {
    /// Build inline layouts for all IFC roots after Taffy layout.
    ///
    /// Uses the computed width from Taffy as the available width for Parley line breaking.
    pub(crate) fn build_ifc_layouts(&mut self, paint_layout_cx: &mut parley::LayoutContext<Brush>) {
        // Discover all IFC roots. From `NodeTree::ifc_root_registry` rather
        // than the slab (layout audit F12): every node the structural passes
        // made a root is registered, and the test below — the one this walk
        // always applied to every slab node — decides which still are, in the
        // same ascending order. An entry that fails it is dropped; a node only
        // becomes a root again through a structural pass, which re-registers it.
        let mut ifc_roots: Vec<usize> = Vec::new();
        let mut stale: Vec<usize> = Vec::new();
        for &id in &self.tree.ifc_root_registry {
            let Some(node) = self.tree.nodes.get(id) else {
                stale.push(id);
                continue;
            };
            if !node.is_element() {
                stale.push(id);
                continue;
            }
            // Only block containers can be IFC roots — and **this gate is
            // redundant *here*** (#593), which is why removing `Inline` from it
            // killed no test and moved no pixel while the same mutation at
            // `create_anonymous_block_boxes` fails 9 tests and at
            // `setup_inline_formatting_contexts` fails 7. **Six and four of those
            // pre-date this change**, so neither site owes its witnesses to the
            // fixtures added with this comment. Both re-measured on *this* tree,
            // because #593's own 3 and 9 were taken before #513 — and they had
            // already drifted to 8 and 6 two rebases ago, which is how fast a
            // count quoted across a tree change goes stale without telling
            // anyone. A node this gate skips
            // cannot pass the `is_ifc` test below, so the test already is the
            // gate.
            //
            // The argument is closed and rests on two greppable facts.
            // `mark_inline_descendants` holds the **only** two writes of
            // `Some(_)` to `ifc_root` in the workspace (every other write clears
            // it: `cleanup_anonymous_block_boxes`,
            // `setup_inline_formatting_contexts`' reset, and
            // `clear_ifc_root_recursive`), and it only ever writes a `root_id`
            // that `setup_inline_formatting_contexts`' own root scan admitted —
            // through the **same** `is_block_container()` call. So a child carrying
            // `ifc_root == Some(id)` is itself proof that `id` is a block
            // container. A *stale* mark cannot open a hole either, and the link
            // that carries that is `layout_dirty`, not only `ifc_dirty`. The reset
            // runs every `ifc_dirty` pass — but this function has a **second call
            // site where it has not run**: `layout_engine.rs`'s text-only early
            // return (`!layout_dirty && !dirty_ifc_text_roots.is_empty()`), which
            // runs neither `setup_inline_formatting_contexts` nor
            // `recompute_contributes_in_flow_block`. What closes that path is that
            // the inline-level crossing in `apply_stylo_styles_to_taffy` sets
            // `layout_dirty` **as well as** `ifc_dirty`, so a display change can
            // never be observed by the text-only call: the pass that sees it is
            // always the full one, and the reset has fired.
            //
            // Measured as well as argued, because an argument is not a
            // measurement: instrumented to count every node that fails this gate
            // and *would* pass the test below — **mode-agnostic**, so a
            // `DisplayMode` variant added later is counted too (`InlineGrid`
            // arrived with #607 after the first sweep, which bucketed by mode and
            // would have missed it) — over 67 constructed shapes and all of
            // `-p rinch-dom -p rinch`. **Zero.**
            //
            // Kept rather than deleted: the three sites asking this question are
            // one authority since #614, and a redundant agreement costs a
            // `matches!` while a site that has stopped asking is how #518, #476
            // and #568 happened. (There were four until #296 removed the
            // one-line floor for childless blocks, `apply_empty_block_line_floor`,
            // the one site that subtracted the atomic inlines.)
            //
            // **"Redundant" means something different at each site.** The other
            // two are load-bearing with witnesses: the same mutation fails 9
            // tests at `create_anonymous_block_boxes`' phase-1 scan and 7 at
            // `setup_inline_formatting_contexts`' root scan (counts from when
            // this comment was written, before #296). So this site is the only
            // one whose arm nothing can reach in a way that matters.
            if !node.display_mode.is_block_container() {
                stale.push(id);
                continue;
            }
            // `ifc_children`, not `children`: an anonymous block box holds its
            // run in `run_members` and has no children at all (#566).
            let is_ifc = node.ifc_children().iter().any(|&child_id| {
                self.tree
                    .nodes
                    .get(child_id)
                    .map(|c| c.ifc_root == Some(id))
                    .unwrap_or(false)
            });
            if is_ifc {
                ifc_roots.push(id);
            } else {
                stale.push(id);
            }
        }
        for id in stale {
            self.tree.ifc_root_registry.remove(&id);
        }

        // Only the roots that need it are re-shaped (the expensive part): a
        // root is rebuilt when it is dirty, has no layout, or has a layout
        // built at a different width — the check after `max_width` below.
        //
        // There used to be a second, earlier skip: whenever *any* root was
        // dirty, every non-dirty root was skipped before its width was
        // looked at. So a width change with no restyle of its own — a flex
        // item narrowed because a sibling's text grew, the sibling being the
        // dirty one — kept its glyphs broken at the old width, overflowing its
        // box. A root with no layout was skipped the same way, and would have
        // had nothing to paint. The width comparison below is the whole rule.
        for root_id in ifc_roots {
            // Skip collapsed blocks (virtualized) — no Parley work needed.
            // Drop any existing text_layout to free memory.
            if self.tree.nodes[root_id].estimated_height.is_some() {
                self.tree.nodes[root_id].text_layout = None;
                continue;
            }
            // A hollow control (#1159): its children are its value, which
            // `paint_input_value` draws; nothing lays them out as content.
            // A canvas, video or iframe renders none of its children (#1173).
            if crate::replaced::ignores_inner_display(&self.tree.nodes[root_id]) {
                self.tree.nodes[root_id].text_layout = None;
                continue;
            }

            let max_width = self.ifc_paint_max_width(root_id);
            let node = &self.tree.nodes[root_id];

            // Skip Parley rebuild if text hasn't changed and the layout already
            // exists with the same max_width and alignment width (#1237: an
            // auto-width root and one declared a pixel wider break at the
            // same width and align within different ones). The existing
            // text_layout is still valid.
            if !self.tree.dirty_ifc_text_roots.contains(&root_id) {
                if let Some(existing) = &node.text_layout {
                    // `==` first: an unconstrained root stores `INFINITY`, and
                    // `INFINITY - INFINITY` is NaN, which no tolerance admits, so
                    // without it every such root was reshaped on every layout
                    // pass that changed no text (ifc_unconstrained_width_skip_tests).
                    let same = |old: f32, new: f32| old == new || (old - new).abs() < 0.01;
                    let new_align = match max_width {
                        Some(_) => self.ifc_content_width(root_id).unwrap_or(f32::INFINITY),
                        None => f32::INFINITY,
                    };
                    if same(existing.max_width, max_width.unwrap_or(f32::INFINITY))
                        && same(existing.align_width, new_align)
                    {
                        continue; // Skip — text_layout is still valid
                    }
                }
            }

            // A root outside the document is not shaped (#1069): nothing
            // paints it. `set_text_content` and `remove_child` take a subtree's
            // marks with it (#1073), but the whole-document structural pass
            // still walks the slab and marks detached subtrees again, so this
            // is where a disconnected root is turned away. Asked only of a
            // root about to be shaped, so a settled document pays nothing.
            // Its old layout is dropped and so is its registration: attaching
            // it again seeds a structural pass, which registers it, and it has
            // no layout to keep, so it is shaped then
            // (`a_reattached_orphan_ifc_root_lays_out_as_a_fresh_one`).
            if self.depth_if_connected(root_id).is_none() {
                self.tree.nodes[root_id].text_layout = None;
                self.tree.ifc_root_registry.remove(&root_id);
                continue;
            }

            self.shape_ifc_root_paint_layout(root_id, max_width, paint_layout_cx);
        }
    }

    /// An IFC root's content-box width, `None` when it is not positive.
    fn ifc_content_width(&self, root_id: usize) -> Option<f32> {
        let node = &self.tree.nodes[root_id];
        let cs = &node.computed_style;
        let padding_h = cs.padding_left.to_px() + cs.padding_right.to_px();
        let border_h = cs.border_left_width.to_px() + cs.border_right_width.to_px();
        let content_width = node.layout.width - padding_h - border_h;
        (content_width > 0.0).then_some(content_width)
    }

    /// The width an IFC root's paint layout is broken at: its content-box
    /// width, with the auto-width slack below. `None` for a root with no
    /// positive content width (unconstrained).
    fn ifc_paint_max_width(&self, root_id: usize) -> Option<f32> {
        // Use content-box width (subtract padding+border) for line breaking.
        // For auto-width elements, don't re-constrain text to measured width
        // as floating-point precision can cause unwanted line breaks.
        let cs = &self.tree.nodes[root_id].computed_style;
        if let Some(content_width) = self.ifc_content_width(root_id) {
            // An auto-width element is sized to its content: its box width is
            // the text's max-content width (+ padding/border) measured with NO
            // wrap, then ROUNDED DOWN to an integer pixel — losing strictly
            // less than 1px of the text's true width. So the paint layout here
            // must allow up to 1px more than `content_width`, or it re-wraps
            // the text at a space inside a box that was sized for one line
            // (the box says one line, the glyphs render two). A 0.5px
            // tolerance can't absorb a full 1px floor; 1.0px provably can
            // (`natural - content_width == frac(natural) < 1.0`). Explicit-
            // width elements get no tolerance — they should wrap at their width.
            //
            // The question is whether the width was *derived* rather than
            // declared, so it is `is_auto_or_keyword` (#626, #691): a
            // `width: max-content` box is measured and floored the same
            // way `auto` is and needs the same slack. Reading `is_auto()`
            // here gave such a box 0px and re-wrapped its text inside a
            // box sized for one line — the box unchanged, the glyphs on
            // two.
            let tolerance = if cs.width.is_auto_or_keyword() {
                1.0
            } else {
                0.0
            };
            Some(content_width + tolerance)
        } else {
            None
        }
    }

    /// Shape `root_id`'s paint layout at `max_width` and store it — the
    /// rebuild half of [`Self::build_ifc_layouts`], which decides *whether*.
    fn shape_ifc_root_paint_layout(
        &mut self,
        root_id: usize,
        max_width: Option<f32>,
        paint_layout_cx: &mut parley::LayoutContext<Brush>,
    ) {
        self.tree.perf.bump(crate::perf::Counter::ShapeIfcBuild);
        // A paragraph that does not wrap is aligned within the content box
        // itself, without the auto-width slack `max_width` carries (#1237).
        let align_width = self
            .ifc_content_width(root_id)
            .zip(max_width)
            .map(|(width, up_to)| AlignWidth { width, up_to });
        let (mut inline_layout, font_family_resolves) = Self::build_inline_layout(
            &self.tree.nodes,
            root_id,
            max_width,
            align_width,
            1.0,
            &mut self.font_cx,
            paint_layout_cx,
            None,
        );
        self.tree.perf.add(
            crate::perf::Counter::InlineFontFamilyResolves,
            font_family_resolves,
        );
        inline_layout.hang.record(&self.tree.perf);

        // text-overflow: ellipsis — every line whose content overflows the
        // container is cut and ends in "…" (#1091), at paint time over these
        // very lines (#1100). CSS Overflow 3 §3.2 asks
        // the question per line box, after wrapping, and so does Chrome 153:
        // an unbreakable word under `white-space: normal`, a `nowrap` span
        // inside a wrapping root and each long line of `pre` text all
        // overflow a line.
        {
            use crate::computed_style::{OverflowValue, TextOverflowValue};
            let cs = &self.tree.nodes[Self::ellipsis_style_owner(&self.tree.nodes, root_id)]
                .computed_style;
            let container_width = max_width.unwrap_or(f32::INFINITY);
            // A grid (or flex) container holding only text is laid out
            // here as an IFC root, but that text is an anonymous item of
            // the container, which does not clip — so no "…", as in
            // Chrome (#904's second review). It is the only ellipsis
            // site since #982 (`ellipsis_route_tests.rs`).
            if matches!(cs.text_overflow, TextOverflowValue::Ellipsis)
                && !cs.display.is_flex_or_grid_container()
                && matches!(cs.overflow_x, OverflowValue::Hidden | OverflowValue::Clip)
                && container_width.is_finite()
                && container_width > 0.0
                // Cheap: the widest line, which parley already knows.
                && inline_layout.layout.width() > container_width
            {
                if let Some(cut) = Self::ellipsis_cut(
                    &self.tree.nodes,
                    root_id,
                    &inline_layout,
                    container_width,
                    &mut self.font_cx,
                    paint_layout_cx,
                ) {
                    self.tree.perf.bump(crate::perf::Counter::EllipsisBuilds);
                    // The "…", shaped once; the cut itself is found in the
                    // lines already shaped.
                    self.tree.perf.add(crate::perf::Counter::EllipsisShapes, 1);
                    inline_layout.ellipsis = Some(Box::new(cut));
                }
            }
        }

        // Write positions from Parley layout back to child nodes
        // Walk the layout lines to find positioned inline boxes and text runs
        self.write_inline_positions(root_id, &inline_layout);

        // The out-of-flow boxes whose static position these lines hold
        // (#632). Each is placed again from them: it was read back before
        // they existed.
        let marked: Vec<usize> = inline_layout.out_of_flow.iter().map(|m| m.id).collect();

        self.tree.nodes[root_id].text_layout = Some(Box::new(inline_layout));
        self.tree.text_layout_epoch += 1;
        let stamp = self.tree.text_layout_epoch;
        self.tree.text_layout_stamps.insert(root_id, stamp);
        // Each box is told where its note is, last to first: a box noted
        // twice (hoisted out of an inline element in an anonymous box's run)
        // keeps the first, its real place.
        for (index, &id) in marked.iter().enumerate().rev() {
            if let Some(node) = self.tree.nodes.get_mut(id) {
                node.static_ifc_root = Some((root_id, index));
            }
        }
        for id in marked {
            crate::out_of_flow::place_static_after_lines(&mut self.tree, id);
        }
        // New glyphs are a paint change whether or not the root's box
        // moved: a span that left the line (`display: none`) or changed
        // its text reaches the screen only through this root. Paint-only:
        // the layout that asked for this rebuild has already run.
        self.tree.paint_dirty_nodes.push(root_id);
    }

    /// Shape one IFC root's paint layout as `build_ifc_layouts` would, **even
    /// though it is outside the document**, which that function declines to
    /// do (#1069). Test support only.
    ///
    /// `walk_inline_children`'s `_ => break` arm has no known connected route
    /// (#615's argument): in a document, an in-flow block inside an inline
    /// element splits it (#513), so the walk never meets the block. Its only witnesses lay a detached
    /// subtree out, where the fold that splits the inline does not reach
    /// (`ifc_classifier_tests`, #615). Once detached roots stopped being
    /// shaped they needed this to reach the walk. The marking half they also
    /// pin is still reached by the whole-document pass itself.
    #[doc(hidden)]
    pub fn shape_ifc_root_for_tests(&mut self, root: rinch_core::dom::NodeId) {
        debug_assert!(
            self.tree
                .nodes
                .get(root.0)
                .is_some_and(|n| n.ifc_children().iter().any(|&c| self
                    .tree
                    .nodes
                    .get(c)
                    .is_some_and(|c| c.ifc_root == Some(root.0)))),
            "shape_ifc_root_for_tests: node {} is not an IFC root (no child carries its mark)",
            root.0
        );
        let max_width = self.ifc_paint_max_width(root.0);
        let mut cx = std::mem::take(&mut self.layout_cx);
        self.shape_ifc_root_paint_layout(root.0, max_width, &mut cx);
        self.layout_cx = cx;
    }

    /// The node whose style decides an IFC root's `text-overflow: ellipsis`
    /// (#1071): the root itself, unless it is an anonymous block box, when it
    /// is the box's `parent` — its block container.
    ///
    /// An anonymous box carries only the inherited properties
    /// ([`crate::computed_style::ComputedStyle::for_anonymous_box`]), and
    /// `text-overflow` and `overflow` are not inherited — yet its lines are its
    /// block container's lines, clipped by that container's `overflow`, and
    /// Chrome 153 draws the container's "…" on them. The boxes of a `<span>`
    /// split around a block child (#513) are minted with the span's block
    /// container as their `parent` too, which is the box Chrome asks.
    /// `white-space` is read there as well; the box inherits it, so the answer
    /// is the same.
    ///
    /// The anonymous box holds none of what this reads, so a restyle of the
    /// container must re-shape it: `invalidate_text_measure_for_node` drops
    /// every box in the container's `run_boxes`
    /// (`ellipsis_anonymous_box_tests.rs`).
    fn ellipsis_style_owner(nodes: &slab::Slab<Node>, root_id: usize) -> usize {
        let root = &nodes[root_id];
        match root.parent {
            Some(parent) if root.is_anonymous_block_box => parent,
            _ => root_id,
        }
    }

    /// Copy cached text layouts from measurement to nodes.
    ///
    /// Uses the exact layouts built during Taffy measurement to ensure
    /// paint uses identical text shaping results.
    ///
    /// No `text-overflow: ellipsis` is built here. A text leaf is the text of
    /// a flex or grid container (behind any `display: contents` wrappers),
    /// which is an anonymous item that does not clip — Chrome 153 draws no
    /// "…" there (#904) — and a box that does clip its text with an ellipsis
    /// is an IFC root, whose "…" is built in `build_ifc_layouts`
    /// (`ellipsis_route_tests.rs`). This site had its own rebuild until #982.
    pub(crate) fn copy_cached_text_layouts(
        &mut self,
        cache: HashMap<(usize, u32), parley::layout::Layout<Brush>>,
    ) {
        // First collect node IDs and their layouts to apply. Only a node the
        // compute measured can have an entry, so walk the cache's nodes rather
        // than the slab (layout audit F12): a node absent from the cache was
        // skipped by every arm below anyway. Ascending, as the slab walk was.
        let mut measured: Vec<usize> = cache.keys().map(|&(id, _)| id).collect();
        measured.sort_unstable();
        measured.dedup();
        let updates: Vec<(usize, parley::layout::Layout<Brush>)> = measured
            .into_iter()
            .filter_map(|id| self.tree.nodes.get(id).map(|node| (id, node)))
            .filter_map(|(id, node)| {
                if node.ifc_root.is_some() {
                    return None;
                } // Skip IFC-managed nodes
                if !matches!(&node.kind, NodeKind::Text(_)) {
                    return None;
                }

                // The final width the measure was called with is the box's
                // **unrounded** width: Taffy rounds `layout` to whole pixels,
                // and a 45.34px min-content grid column rounded to 45 missed its
                // own layout and fell through to the unwrapped max-content one
                // below — one line painted across a box laid out for seven
                // (#904's review, found on an `inline-grid`).
                let unrounded = node
                    .taffy_id
                    .and_then(|t| self.tree.taffy.unrounded_layout(t).size.width.into())
                    .filter(|w: &f32| *w > 0.0);
                if let Some(layout) = unrounded.and_then(|w| cache.get(&(id, w.to_bits()))) {
                    return Some((id, layout.clone()));
                }

                let width = node.layout.width;
                let wrap_bits = if width > 0.0 {
                    width.to_bits()
                } else {
                    u32::MAX
                };

                // Try to find a cached layout for this node with the final width
                if let Some(layout) = cache.get(&(id, wrap_bits)) {
                    return Some((id, layout.clone()));
                }

                // Fallback: prefer the MaxContent layout (wrap_bits = u32::MAX) which has natural width.
                // This avoids picking a MinContent layout that was wrapped to 0 width.
                if let Some(layout) = cache.get(&(id, u32::MAX)) {
                    return Some((id, layout.clone()));
                }
                // Last resort: find any cached layout for this node (prefer widest/tallest ratio)
                let mut best: Option<&parley::layout::Layout<Brush>> = None;
                for ((nid, _), layout) in &cache {
                    if *nid == id {
                        // Prefer single-line layouts (lower height) over wrapped ones
                        if best.is_none() || layout.height() < best.unwrap().height() {
                            best = Some(layout);
                        }
                    }
                }
                best.map(|layout| (id, layout.clone()))
            })
            .collect();

        // Apply the updates with alignment
        for (id, mut layout) in updates {
            // Get text-align from parent's computed style
            let alignment = self.tree.nodes[id]
                .parent
                .and_then(|p| self.tree.nodes.get(p))
                .map(|p| p.computed_style.text_align.to_parley())
                .unwrap_or(parley::layout::Alignment::Start);
            layout.align(alignment, parley::layout::AlignmentOptions::default());
            self.tree.nodes[id].cached_text_parley = Some(Box::new(layout));
            self.tree.text_layout_epoch += 1;
            let stamp = self.tree.text_layout_epoch;
            self.tree.text_layout_stamps.insert(id, stamp);
        }
    }

    /// Write computed positions from an InlineLayout back into child node layout fields.
    pub(crate) fn write_inline_positions(&mut self, root_id: usize, inline_layout: &InlineLayout) {
        let root_layout = self.tree.nodes[root_id].layout;

        // Walk Parley layout lines to find positioned items
        for line in inline_layout.layout.lines() {
            for item in line.items() {
                match item {
                    parley::layout::PositionedLayoutItem::GlyphRun(_) => {
                        // Text runs don't map to individual child nodes
                    }
                    parley::layout::PositionedLayoutItem::InlineBox(positioned_box) => {
                        if is_edge_box(positioned_box.id) {
                            continue;
                        }
                        let child_id = positioned_box.id as usize;
                        if let Some(child) = self.tree.nodes.get_mut(child_id) {
                            // A move is a move, whoever makes it: push the box
                            // paint-dirty like any other layout change, so the
                            // region clears where it was painted and draws where
                            // it went — its own box *and* anything it carries
                            // outside the IFC root's box (an absolute child),
                            // which the root's own rect does not reach. Its
                            // `prev_layout` stays the painted box until a paint
                            // consumes it. Review of #880, probe N: unpushed, the
                            // chip's absolute kid was neither drawn at its new
                            // spot nor cleared from its old one.
                            let moved = child.layout.x != positioned_box.x
                                || child.layout.y != positioned_box.y;
                            child.layout.x = positioned_box.x;
                            child.layout.y = positioned_box.y;
                            if moved {
                                self.tree.paint_dirty_nodes.push(child_id);
                                // An absolute box placed past this one is
                                // owed a second look (`out_of_flow`).
                                self.tree.abs_late_moves = true;
                            }
                        }
                    }
                }
            }
        }

        // For text nodes that are direct children, set their layout to reflect
        // the actual IFC content extent (not the constrained container height).
        // A scroll container's range no longer reads these boxes:
        // `paint::scrollbar::content_extents` measures an IFC root's inline
        // layout itself (#396), because a text node under a `display: contents`
        // wrapper or an inline span gets no box here at all, and a skipped
        // root never rewrites this one (#873).
        let ifc_content_height = inline_layout.layout.height();
        let children: Vec<usize> = self.tree.nodes[root_id].children.clone();
        for child_id in children {
            if let Some(child) = self.tree.nodes.get(child_id)
                && child.is_text()
                && child.ifc_root == Some(root_id)
                && let Some(child) = self.tree.nodes.get_mut(child_id)
            {
                child.layout.x = 0.0;
                child.layout.y = 0.0;
                child.layout.width = root_layout.width;
                child.layout.height = ifc_content_height;
            }
        }
    }

    /// Drop the previous layout pass's anonymous block boxes.
    ///
    /// Since #566 this is a **drop**, not a dissolve: the boxes were never in
    /// the DOM tree and nothing was ever reparented, so there is nothing to put
    /// back. What used to live here — finding the box in its parent's
    /// `children`, removing it, re-inserting its adopted children at recorded
    /// slots, and the `AdoptedChild { child, origin, prev, index }` record that
    /// made those slots survive a document mutated between passes — existed
    /// only to undo a move that no longer happens.
    ///
    /// It is now the same shape as [`Self::cleanup_ifc_measure_leaves`]: forget
    /// the Taffy node, forget the slab entry, and rebuild whatever list held it.
    ///
    /// With a `scope` (a scoped pass, `crate::ifc_scope`), only the boxes the
    /// scope's containers hold are removed — plus any **orphan**: a box whose
    /// container has left the slab or no longer lists it, which a freed subtree
    /// (`set_inner_html`) leaves behind and which no container in any scope
    /// would ever reach again.
    ///
    /// **A box whose container still lists it is retired, not freed** (#964):
    /// it keeps its slab entry, its paint layout and its measured sizes (with
    /// the content signature), and is handed back in [`RetiredAnonBoxes`] for
    /// `create_anonymous_block_boxes` to mint the same run's box into. A
    /// whole-document pass re-mints every box in the document whatever changed
    /// (a theme change), and a scoped pass every box of its containers; freeing
    /// them re-shaped each box's text, because both the paint layout and the
    /// cached measures (with the content signature `refresh_ifc_signatures`
    /// compares) went with the box. An orphan is freed as before.
    fn cleanup_anonymous_block_boxes(
        &mut self,
        scope: Option<&crate::ifc_scope::IfcScope>,
    ) -> RetiredAnonBoxes {
        let all = std::mem::take(&mut self.tree.anonymous_block_boxes);
        let anon_ids = match scope {
            None => all,
            Some(scope) => {
                let nodes = &self.tree.nodes;
                let (go, keep): (Vec<usize>, Vec<usize>) = all.into_iter().partition(|&b| {
                    let Some(anon) = nodes.get(b) else {
                        return true;
                    };
                    let Some(p) = anon.parent else { return true };
                    scope.rootable.contains(&p)
                        || nodes.get(p).is_none_or(|pn| !pn.run_boxes.contains(&b))
                });
                self.tree.anonymous_block_boxes = keep;
                go
            }
        };
        let mut retired = RetiredAnonBoxes::default();
        if anon_ids.is_empty() {
            return retired;
        }

        // The containers whose Taffy child lists held these boxes — read from
        // the box's own `parent`, which it keeps (see
        // `create_anonymous_block_boxes`).
        let mut parents_affected: Vec<usize> = Vec::new();
        for &anon_id in &anon_ids {
            let Some(anon) = self.tree.nodes.get(anon_id) else {
                continue;
            };
            let members = anon.run_members.clone();
            // Listed by a live container: retired for the run's next box.
            let listed_by = anon.parent.filter(|&p| {
                self.tree
                    .nodes
                    .get(p)
                    .is_some_and(|pn| pn.run_boxes.contains(&anon_id))
            });
            if let Some(p) = listed_by
                && !parents_affected.contains(&p)
            {
                parents_affected.push(p);
            }
            for m in members {
                // Only a member that still names this box: a freed member's
                // slab id can have been handed to an unrelated node since.
                if let Some(member) = self.tree.nodes.get_mut(m)
                    && member.run_box == Some(anon_id)
                {
                    member.run_box = None;
                    member.ifc_root = None;
                }
            }

            // Remove anonymous Taffy node (children are detached, not deleted)
            if let Some(anon_taffy) = self.tree.nodes[anon_id].taffy_id.take() {
                self.tree.taffy_map.remove(&anon_taffy);
                let _ = self.tree.taffy.remove(anon_taffy);
            }

            if let Some(parent) = listed_by {
                // Retired: the entry stays, inert — no parent, no run, no Taffy
                // node — so its id is not handed to anything else before
                // `create_anonymous_block_boxes` decides whether the run is
                // still there. Its measured sizes stay in the cache under the
                // same id.
                let node = &mut self.tree.nodes[anon_id];
                node.parent = None;
                node.run_out_of_flow.clear();
                retired.push(RetiredAnonBox {
                    id: anon_id,
                    parent,
                    run_members: std::mem::take(&mut node.run_members),
                });
                continue;
            }

            // Remove anonymous DOM node from slab
            self.tree.nodes.remove(anon_id);
            // And its measured sizes, for `remove_subtree`'s reason: the slab
            // recycles the id, and the box minted next — very often for the
            // same run, so with the same content signature — would otherwise
            // be answered sizes measured under the old box's inherited style.
            // (A box keeps its container's typography, and a restyle of the
            // container after a move reaches no root the members name: the
            // move cleared their marks. Found by the scoped pass's random
            // differential; the whole-document pass hid it by re-minting every
            // box in a different id order.)
            self.tree.ifc_measure_cache.remove(&anon_id);
        }

        // Rebuild each affected parent's Taffy children through the one
        // authority for that list — `collect_effective_taffy_children` (#476,
        // #566).
        for parent_id in parents_affected {
            // Cleared with the boxes it listed: every one of them has just
            // been removed from the slab, so leaving the list populated would
            // point invariant C at freed indices — and send
            // `box_tree_children` down the substituting path for a container
            // with nothing to substitute.
            if let Some(p) = self.tree.nodes.get_mut(parent_id) {
                p.run_boxes.clear();
            }
            self.rebuild_effective_taffy_children(parent_id);
        }
        retired
    }

    /// Free the retired boxes `create_anonymous_block_boxes` did not mint a run
    /// into: their run is gone, or has changed.
    fn free_retired_anonymous_boxes(&mut self, retired: RetiredAnonBoxes) {
        for id in retired.into_ids() {
            self.tree.nodes.remove(id);
            // `remove_subtree`'s reason, as for a freed box above: the slab
            // recycles the id.
            self.tree.ifc_measure_cache.remove(&id);
        }
    }

    /// Rebuild `node_id`'s Taffy child list — and, when it is a boxless
    /// `display: contents` element, the list of whatever actually holds its
    /// boxes — from the one authority for that list (#476).
    ///
    /// See [`Self::collect_effective_taffy_children`] (the order) and
    /// [`Self::taffy_child_list_owners`] (whose list, and why bottom-up).
    fn rebuild_effective_taffy_children(&mut self, node_id: usize) {
        for owner in Self::taffy_child_list_owners(&self.tree.nodes, node_id) {
            let Some(owner_taffy) = self.tree.nodes.get(owner).and_then(|n| n.taffy_id) else {
                continue;
            };
            let children = Self::collect_effective_taffy_children(&self.tree.nodes, owner);
            let _ = self.tree.taffy.set_children(owner_taffy, &children);
        }
    }

    /// Remove the previous pass's IFC measure leaves (#466).
    ///
    /// A measure leaf is a Taffy-only node — no DOM identity, absent from
    /// `taffy_map` — created by [`Self::setup_inline_formatting_contexts`] for
    /// an IFC root whose out-of-flow children stay attached. Like anonymous
    /// block boxes they are recreated from scratch each `ifc_dirty` pass, so
    /// this runs first and unconditionally. `taffy.remove` detaches the leaf
    /// from its parent whether or not it is still attached (an interim
    /// `sync_display_contents` rebuild may already have dropped it).
    ///
    /// With a `scope`, only the leaves of roots in it — and of roots that have
    /// left the slab — are removed.
    fn cleanup_ifc_measure_leaves(&mut self, scope: Option<&crate::ifc_scope::IfcScope>) {
        let leaves: Vec<(usize, taffy::NodeId)> = match scope {
            None => std::mem::take(&mut self.tree.ifc_measure_leaves)
                .into_iter()
                .collect(),
            Some(scope) => {
                let nodes = &self.tree.nodes;
                let go: Vec<usize> = self
                    .tree
                    .ifc_measure_leaves
                    .keys()
                    .copied()
                    .filter(|r| scope.rootable.contains(r) || !nodes.contains(*r))
                    .collect();
                go.into_iter()
                    .filter_map(|r| self.tree.ifc_measure_leaves.remove(&r).map(|l| (r, l)))
                    .collect()
            }
        };
        for (root_id, leaf) in leaves {
            let _ = self.tree.taffy.remove(leaf);
            // `remove` does not dirty the old parent. The mutation that made
            // this pass run usually did, but mark it explicitly so a container
            // that stops being an IFC root altogether cannot serve a cached
            // layout that still includes the removed leaf's height.
            if let Some(root_taffy) = self.tree.nodes.get(root_id).and_then(|n| n.taffy_id) {
                let _ = self.tree.taffy.mark_dirty(root_taffy);
            }
        }
    }

    /// Mark an IFC root's measure leaf dirty in Taffy, if it has one (#466).
    ///
    /// Taffy caches layout per node and `mark_dirty` propagates *up* toward
    /// the root — so marking the IFC root's own Taffy node does **not**
    /// invalidate the measure cached on its child leaf: the stale measure
    /// would be served straight back on the next compute, and a text edit in a
    /// `text + absolute` container would never change the container's height.
    /// Every site that marks an IFC root's Taffy node dirty to force a
    /// re-measure must call this beside it. (Marking the leaf also dirties the
    /// root — propagation is upward — but the sites keep their own root mark:
    /// most roots have no leaf.)
    pub(crate) fn mark_ifc_measure_dirty(&mut self, root_id: usize) {
        if let Some(&leaf) = self.tree.ifc_measure_leaves.get(&root_id) {
            let _ = self.tree.taffy.mark_dirty(leaf);
        }
    }

    /// Collect `node_id`'s children as **run units**: the boxes it actually
    /// holds, with every `display: contents` child replaced by what it stands
    /// for, recursively.
    ///
    /// This is [`crate::RinchDocument::collect_effective_taffy_children`]'s
    /// question asked in DOM ids, and — since #568 landed on #566's redesign —
    /// asked the *same way*. That function attaches boxes and flattens
    /// unconditionally; so does this one.
    ///
    /// **A unit is therefore never a `Contents` node**, by construction: a
    /// wrapper is recursed into, never pushed. Consumers rely on that rather
    /// than re-testing for it.
    ///
    /// # It used to keep a wrapper whole, and that rule is gone
    ///
    /// A wrapper an inline run would have taken *all* of used to be pushed
    /// whole rather than flattened, so a run moved "the shallowest node
    /// standing for exactly its content, and no deeper". **That was #566's
    /// ground**: grouping ended in reparenting the DOM, so flattening every
    /// wrapper would have adopted content out of a node the author wrote, one
    /// level further from anything they could see, and `insert_before(wrapper,
    /// new, adopted)` could not then find its reference node.
    ///
    /// Since #566 a run is *recorded*, never reparented, and the rule went on
    /// three independent grounds rather than one:
    ///
    ///  1. **Its measurement no longer reproduces.** The number that justified
    ///     it — insert a block into a wrapper before its text, one pass later —
    ///     was `y = 7` kept whole against `y = 27` flattened. It is now `y = 7`
    ///     **either way**, because the adoption that produced the divergence is
    ///     gone.
    ///  2. **CSS points the other way.** A `display: contents` element
    ///     generates no box (CSS 2.1 §9.2.1.1), so the inline-level boxes in a
    ///     run *are* the wrapper's children. Flattening is the faithful
    ///     spelling; keeping whole was the workaround.
    ///  3. **Nothing else could observe it.** With the rule disabled the whole
    ///     suite passed except the single test written to watch it, and the
    ///     acceptance matrix stayed at 13/13.
    ///
    /// It cost ~60 lines of three-condition logic carrying a warning about not
    /// adding a fourth. Two of those conditions had already lost their only
    /// witnesses to #566, because the witnesses asserted DOM parentage and
    /// nothing is reparented any more.
    ///
    /// **This list has three readers and they are one authority.** The
    /// classification below groups runs out of it,
    /// [`crate::RinchDocument::box_tree_children`] emits the boxes into it, and
    /// [`crate::RinchDocument::run_bookkeeping_violations`] checks that every
    /// member is still a unit of it. Change what a unit is and all three move
    /// together — the third exists to say so out loud if they ever do not.
    pub(crate) fn collect_run_units(
        nodes: &slab::Slab<Node>,
        node_id: usize,
        out: &mut Vec<usize>,
    ) {
        Self::collect_run_units_for(nodes, node_id, node_id, out);
    }

    /// [`Self::collect_run_units`] with the **owner** threaded through the
    /// recursion: the container whose list is being built, which a recursion
    /// into a `display: contents` wrapper or a split inline must not lose sight
    /// of, because a hoisted out-of-flow box (#591) is a unit of exactly one
    /// owner and is recognised by comparing `hoisted_out_of_flow_to` with it.
    fn collect_run_units_for(
        nodes: &slab::Slab<Node>,
        node_id: usize,
        owner: usize,
        out: &mut Vec<usize>,
    ) {
        let Some(node) = nodes.get(node_id) else {
            return;
        };
        for &child_id in &node.children {
            let Some(child) = nodes.get(child_id) else {
                continue;
            };
            // A hoisted out-of-flow box is a unit of its **host** and of nothing
            // else (#591): when this walk is building some other node's list —
            // the inline element the box sits in, or a wrapper under it — the
            // box is not this node's child in the box tree. Its host's walk
            // picks it up below, right after the inline element.
            if child
                .hoisted_out_of_flow_to
                .is_some_and(|host| host != owner)
            {
                continue;
            }
            // Two kinds of child contribute their children's boxes rather than
            // one of their own, and both are recursed into rather than pushed:
            //
            //  * a `display: contents` wrapper, which generates no box at all;
            //  * a **split inline** (#513) — a `display: inline` element holding
            //    an in-flow block-level box, which CSS 2.1 §9.2.1.1 breaks
            //    around that box. Its pieces are this container's boxes, so the
            //    anonymous block boxes are minted *here* and the block becomes
            //    their sibling, which is the whole of #513's fix.
            //
            // **A unit is therefore never either of those**, by construction,
            // and the consumers below rely on that rather than re-testing.
            if child.inline_flow_role() == InlineFlowRole::Contents || child.is_split_inline() {
                Self::collect_run_units_for(nodes, child_id, owner, out);
            } else {
                out.push(child_id);
                // A third kind contributes **one** box of its own plus some of
                // its descendants' (#591): a non-atomic `display: inline`
                // element holding an out-of-flow box. CSS 2.1 §9.4.2 says the
                // box neither breaks the inline formatting context nor splits
                // the inline — so the inline stays one unit, flowed whole by
                // the IFC — and the box, which the IFC does not lay out and
                // which would otherwise leave the Taffy tree with the detached
                // inline, is a unit of this owner in its own right, emitted
                // right after the inline it sits in. It joins no run and ends
                // none (#406), so the run grouping is untouched.
                if child.hosts_hoisted_out_of_flow
                    && child.is_element()
                    && child.display_mode == DisplayMode::Inline
                {
                    Self::collect_hoisted_out_of_flow(nodes, child_id, owner, out);
                }
            }
        }
    }

    /// The out-of-flow boxes beneath the non-atomic inline element `inline_id`
    /// that are hoisted to `owner` (#591), in DOM order — walking through
    /// nested non-atomic inlines and `display: contents` wrappers, and only
    /// where `hosts_hoisted_out_of_flow` says there is something to find.
    fn collect_hoisted_out_of_flow(
        nodes: &slab::Slab<Node>,
        inline_id: usize,
        owner: usize,
        out: &mut Vec<usize>,
    ) {
        let Some(node) = nodes.get(inline_id) else {
            return;
        };
        for &child_id in &node.children {
            let Some(child) = nodes.get(child_id) else {
                continue;
            };
            if child.hoisted_out_of_flow_to == Some(owner) {
                out.push(child_id);
            } else if child.hosts_hoisted_out_of_flow
                && (child.inline_flow_role() == InlineFlowRole::Contents
                    || (child.is_element() && child.display_mode == DisplayMode::Inline))
            {
                Self::collect_hoisted_out_of_flow(nodes, child_id, owner, out);
            }
        }
    }

    /// Create anonymous block boxes for block containers with mixed content.
    ///
    /// Per CSS spec, when a block container has both inline-level and block-level
    /// children, consecutive runs of inline children are wrapped in anonymous
    /// block boxes. These boxes become IFC roots for text layout.
    fn create_anonymous_block_boxes(
        &mut self,
        scope: Option<&crate::ifc_scope::IfcScope>,
        mut retired: RetiredAnonBoxes,
    ) {
        // Phase 1: Detect mixed-content block containers
        // Each run with the out-of-flow boxes standing inside it
        // (`Node::run_out_of_flow`).
        type Run = (Vec<usize>, Vec<(usize, usize)>);
        let mut containers: Vec<(usize, Vec<Run>)> = Vec::new();
        // Reused across containers so the per-node walk allocates once.
        let mut effective: Vec<usize> = Vec::new();

        // A scoped pass looks only at its own containers: a box is minted for
        // a block container's run, and every block container that could need
        // one differently from last pass is a container of the scope.
        let candidates: Vec<usize> = match scope {
            None => self.tree.nodes.iter().map(|(id, _)| id).collect(),
            Some(scope) => scope.containers.clone(),
        };
        for id in candidates {
            let Some(node) = self.tree.nodes.get(id) else {
                continue;
            };
            if !node.is_element() || node.is_anonymous_block_box {
                continue;
            }
            // Only block containers can have anonymous boxes
            if !node.display_mode.is_block_container() {
                continue;
            }

            // A `display: contents` element generates no box, so it can never
            // be the box that holds anonymous children — its content belongs to
            // the nearest ancestor that does generate one, and the flattened
            // scan below reaches it from there (#568). `display: contents` maps
            // to `DisplayMode::Block` (`style_resolution`), so without this the
            // wrapper is classified as a container in its own right and mints a
            // box inside a boxless element. `setup_inline_formatting_contexts`
            // has carried the same guard, spelled the same way, since #61; this
            // site asking the question differently was the asymmetry.
            if node.computed_style.display == crate::computed_style::values::DisplayValue::Contents
            {
                continue;
            }

            effective.clear();
            Self::collect_run_units(&self.tree.nodes, id, &mut effective);

            let role_of = |c: usize| {
                self.tree
                    .nodes
                    .get(c)
                    .map(|n| n.inline_flow_role())
                    .unwrap_or(InlineFlowRole::NoBox)
            };

            // No `Contents` case: a unit is never a wrapper (see
            // `collect_run_units`), so there is nothing here to test for.
            let has_inline = effective
                .iter()
                .any(|&c| role_of(c) == InlineFlowRole::Inline);
            // An out-of-flow child is not block *content* (#406): per CSS 2.1
            // §9.2.1.1 an absolutely positioned box is out of flow and does not
            // force anonymous block box generation, so a container whose only
            // non-inline children are out of flow holds inline content only.
            // Counting it here minted an anonymous box CSS would never create —
            // and that box was, by accident, what kept the container's measure
            // reachable. Its principled replacement is the measure leaf that
            // `setup_inline_formatting_contexts` creates for exactly this
            // shape (see [`NodeContext::InlineRoot`]). A `display: none` child
            // generates no box either (#366): counting it minted an anonymous
            // box for a child browsers lay out nothing for, and — with the run
            // grouping below ending the run on it — split `a<none/>b` onto two
            // lines browsers render as one.
            //
            // The `display: contents` carve-out this test used to need (#518 —
            // count a wrapper only when it is **opaque**) is gone: every
            // wrapper is broken into units by `collect_run_units`, so
            // `effective` carries the block itself and the question needs no
            // special case. No wrapper survives into this list at all — the
            // collector recurses into one and never pushes it — so nothing here
            // can be `Contents`, let alone a wrapper masquerading as an
            // `InFlowBlock`. `has_block` and the run grouping read the same
            // list, which is what #518 asked for and could then only
            // approximate.
            //
            // The display-first precedence — `display: contents; position:
            // absolute` is `Contents`, never `OutOfFlow`, because Stylo does
            // not blockify contents and a boxless element has no box to take
            // out of flow — is [`Node::inline_flow_role`]'s contract (#366),
            // and is what sends such a wrapper into the unit collector rather
            // than past this scan as one out-of-flow box.
            let has_block = effective
                .iter()
                .any(|&c| role_of(c) == InlineFlowRole::InFlowBlock);

            if !(has_inline && has_block) {
                continue;
            }

            // Group consecutive inline children into runs
            let mut runs: Vec<Run> = Vec::new();
            let mut current_run: Vec<usize> = Vec::new();
            let mut current_out_of_flow: Vec<(usize, usize)> = Vec::new();

            for &child_id in &effective {
                match role_of(child_id) {
                    // A comment has no Taffy node and no box. An out-of-flow
                    // box neither joins a run nor ends one (#406): ending the
                    // current run on it made `a<abs/>b` two runs, two
                    // anonymous boxes, two lines — a split `has_block` alone
                    // cannot prevent when a real block sibling is present. A
                    // `display: none` child is boxless too (#366) and used to
                    // end the run the same wrong way.
                    //
                    // Where it stood in the run is kept, for its static
                    // position (#632) — before the run's first member too:
                    // Taffy's sibling order puts such a box at the top of
                    // the run's box, but an inline-level one starts the
                    // first line, which `text-align` moves. A block that
                    // arrives while the run is still empty takes the boxes
                    // before it out again: they stand in no line.
                    InlineFlowRole::OutOfFlow => {
                        // (A box hoisted out of an inline element (#591)
                        // is a unit right after that element, so it is kept
                        // here too; the walk of the element meets it first,
                        // in its real place, and the first note is the one
                        // read.)
                        current_out_of_flow.push((current_run.len(), child_id));
                        continue;
                    }
                    InlineFlowRole::Comment | InlineFlowRole::NoBox => {
                        continue;
                    }
                    // A wrapper never reaches here: `collect_run_units`
                    // flattens every one, so a unit is always a real box.
                    InlineFlowRole::Contents => {}
                    InlineFlowRole::Inline => current_run.push(child_id),
                    // Only an in-flow block-level box ends a run, and it is the
                    // same list `has_block` just read, so the two cannot
                    // disagree about a node (#518).
                    InlineFlowRole::InFlowBlock => {
                        if !current_run.is_empty() {
                            runs.push((
                                std::mem::take(&mut current_run),
                                std::mem::take(&mut current_out_of_flow),
                            ));
                        } else {
                            current_out_of_flow.clear();
                        }
                    }
                }
            }
            if !current_run.is_empty() {
                runs.push((current_run, current_out_of_flow));
            }

            if !runs.is_empty() {
                containers.push((id, runs));
            }
        }

        // Phase 2: mint a box per run and **record** it — nothing is reparented
        //
        // The box is deliberately outside the DOM tree (#566): in no node's
        // `children`, holding its run in `run_members` while each member points
        // back through `run_box`. It keeps a real `parent` — the edge is
        // one-way, upward only, which is the half no reader of the author's
        // tree ever follows. An anonymous box is a box-tree
        // construct (CSS 2.1 §9.2.1.1) and the DOM is the element tree; making
        // it a DOM node made it lie to every single-level read of that tree —
        // `remove_child`'s `retain`, `insert_before`'s `position()`,
        // `next_sibling`, `:nth-child`, and `find_parent_computed_style`.
        //
        // What used to be here — reparenting the run, rewriting the container's
        // `children`, and a restore path to undo both — is gone. That deletion
        // *is* the fix.
        for (parent_id, runs) in containers {
            let guard = self.tree.guard.clone();

            for (run, run_out_of_flow) in runs {
                // The box this run had last pass, retired by
                // `cleanup_anonymous_block_boxes` (#964): same container, same
                // members in the same order. Minted into its slab entry, so its
                // id — which its measures are cached under and an out-of-flow
                // box's `static_ifc_root` names — does not move. (Anything
                // else the run's lines are built from, the out-of-flow boxes
                // standing in it included, is in the content signature.)
                let kept = retired.take(parent_id, &run);
                // Create anonymous block box DOM node
                let anon_id = match &kept {
                    Some(r) => r.id,
                    None => self.tree.nodes.vacant_key(),
                };
                let mut anon_node = Node::element(anon_id, "div", guard.clone());
                anon_node.is_anonymous_block_box = true;
                anon_node.display_mode = DisplayMode::Block;
                // An anonymous block box is an in-flow block-level box, so the
                // rule on [`Node::contributes_in_flow_block`] answers `true`
                // for it. Set here rather than in the recompute pass because a
                // box is not in anybody's `children` and that pass walks
                // `children`. Nothing reads it today — a box is never a unit of
                // its container — and it is set anyway so the field stays total
                // over the slab and the differential fixture can say so.
                anon_node.contributes_in_flow_block = true;
                // **A parent, but not a child.** The box keeps an upward edge
                // to its container and appears in nobody's `children`; its run
                // lives in `run_members` and is never reparented.
                //
                // Exactly two edges caused #566, #579 and the inheritance
                // defect, and the box's own `parent` is neither: the box
                // appearing in `container.children` shifted every following
                // sibling's index, and members' `parent` pointing at the box
                // broke `remove_child`, `insert_before`, `next_sibling` and the
                // cascade. Cutting those two is the whole fix.
                //
                // Keeping the upward edge is not incidental. `parent: None` was
                // tried and is **wrong**:
                // `compute_absolute_position_and_transform` walks up via
                // `node.parent` twice and both loops end on `None`, so a
                // parentless box is summed **as if it were the root** — its
                // line painted at the viewport origin instead of inside its
                // container. Silent: no panic, no wrong number, just a box in
                // the wrong place.
                //
                // And this is what makes the safety a proof rather than a
                // survey. The line that used to follow — `child.parent =
                // Some(anon_id)` — was the **only** site in the codebase that
                // made anything a child of the box. With it gone, no node has
                // the box as its parent, so the box is a **leaf of the parent
                // relation**: reachable upward from itself, never traversed
                // *through*. Every ancestor walk in the engine is unaffected by
                // construction rather than by enumeration.
                anon_node.parent = Some(parent_id);
                anon_node.run_members = run.clone();
                anon_node.run_out_of_flow = run_out_of_flow;
                // Inherited properties only (CSS 2.1 §9.2.1.1). Cloning the
                // parent's whole style gave the anonymous box a box model its
                // Taffy style (below) does not have, and paint double-counted
                // the parent's padding+border for everything this IFC draws
                // (#319) — see [`ComputedStyle::for_anonymous_box`].
                //
                // The style parent is passed explicitly now that the box has no
                // DOM parent to read it from. It must be the parent the **run**
                // has, which on this base is the container: a box that inherits
                // from the wrong node draws its text in the wrong colour and at
                // the wrong size, and nothing else in the suite sees it.
                anon_node.computed_style = crate::computed_style::ComputedStyle::for_anonymous_box(
                    &self.tree.nodes[parent_id].computed_style,
                );

                // Create Taffy node for the anonymous box
                let anon_taffy = self
                    .tree
                    .taffy
                    .new_leaf(taffy::Style {
                        display: taffy::Display::Block,
                        ..Default::default()
                    })
                    .unwrap();
                anon_node.taffy_id = Some(anon_taffy);
                self.tree.taffy_map.insert(anon_taffy, anon_id);

                match kept {
                    Some(_) => {
                        // The old box's layout and measures are kept as they
                        // stand, exactly as a box outside a scoped pass keeps
                        // them: whatever has touched them since the last pass
                        // has already dropped them. A restyle of the container
                        // drops the layouts of every box it lists
                        // (`invalidate_text_layout_for_node`, through
                        // `run_boxes`), a change to a member drops its box's,
                        // and `refresh_ifc_signatures`, which runs after this,
                        // drops a box whose content signature moved. What it
                        // was last laid out and painted at is kept for damage
                        // (`prev_layout`, `painted`), as a box that stayed would.
                        let old = &mut self.tree.nodes[anon_id];
                        anon_node.text_layout = old.text_layout.take();
                        anon_node.layout = old.layout;
                        anon_node.prev_layout = old.prev_layout;
                        anon_node.painted = old.painted.take();
                        self.tree.nodes[anon_id] = anon_node;
                    }
                    None => {
                        // Insert anonymous node into slab
                        self.tree.nodes.insert(anon_node);
                    }
                }

                // Point every member at its box. This is what
                // `box_tree_children` reads, and it must be set before the
                // Taffy rebuild below, which consults it.
                for &child_id in &run {
                    if let Some(child) = self.tree.nodes.get_mut(child_id) {
                        child.run_box = Some(anon_id);
                    }
                }

                // The downward half of the box's edge. Recorded here, beside
                // the upward `parent` and the members' back-pointers, so all
                // three are written in one place and cannot drift — and so
                // invariant A can be stated for every node without exempting
                // this one.
                self.tree.nodes[parent_id].run_boxes.push(anon_id);

                // Track for cleanup on next layout pass
                self.tree.anonymous_block_boxes.push(anon_id);
            }

            // Rebuild the container's Taffy children — and each new box's own —
            // through the one authority for that list,
            // `collect_effective_taffy_children`, which flattens
            // `display: contents` children (#476) and now also substitutes a
            // run's box for its members (#566).
            //
            // A box's own list is built from `run_members` rather than from
            // `children`, which is empty. `mark_inline_descendants` detaches
            // those members again a few lines later — this attaches them only
            // so that the detach has something to detach *from*, which is what
            // keeps a member's Taffy node from being left parented to the
            // container.
            // The container's own `run_boxes` rather than the document-wide
            // list: the boxes whose parent is this container are exactly the
            // ones it lists, in the order they were minted, so the filter below
            // selects the same ones — without an O(every box) scan per container.
            let new_boxes: Vec<usize> = self.tree.nodes[parent_id]
                .run_boxes
                .iter()
                .copied()
                .filter(|&b| {
                    self.tree.nodes[b]
                        .run_members
                        .first()
                        .and_then(|&m| self.tree.nodes.get(m))
                        .and_then(|m| m.parent)
                        == Some(parent_id)
                })
                .collect();
            for anon_id in new_boxes {
                // Through the **one** rebuild authority (#476), not a
                // hand-rolled member → `taffy_id` map.
                //
                // The hand-rolled map was wrong while a member could be a
                // `display: contents` wrapper — briefly true on this branch,
                // under a keep-whole rule since deleted. Such a wrapper
                // generates no box, so mapping members to their own `taffy_id`
                // handed the box the *wrapper's* Taffy node, which
                // `mark_inline_descendants` never detached: it detaches the
                // wrapper's `Inline` descendants, not the wrapper. The box
                // stayed an IFC root with a Taffy child and tripped #466's
                // leaf invariant.
                //
                // **A member cannot be a wrapper today** — `collect_run_units`
                // recurses into every `Contents` child and never pushes one, so
                // no input produces such a unit. The two spellings therefore
                // agree on everything now, and this one is kept because one
                // rebuild authority beats two that happen to agree (#476).
                self.rebuild_effective_taffy_children(anon_id);
            }
            self.rebuild_effective_taffy_children(parent_id);
        }
        self.free_retired_anonymous_boxes(retired);
    }

    /// Recompute [`Node::contributes_in_flow_block`] for every node, bottom-up
    /// in one pass (#513).
    ///
    /// The field's own doc carries the rule; this is where it is applied. Three
    /// things about the shape of the pass:
    ///
    /// **It runs first, before every other pass in the `ifc_dirty` block.** The
    /// field is read by `collect_run_units` and therefore by
    /// `box_tree_children` and `collect_effective_taffy_children` — which
    /// `sync_display_contents` and `cleanup_anonymous_block_boxes` both call. A
    /// pass reading a *previous* pass's value would rebuild a Taffy child list
    /// from a classification the rest of this pass is about to contradict, and
    /// the list nothing rebuilds again is how #476 stayed invisible.
    ///
    /// **It is a whole-tree recompute, not an invalidation**, for
    /// `setup_inline_formatting_contexts`' own stated reason about `ifc_root`:
    /// derived state that only ever gets *set* goes stale in the direction
    /// nobody notices. Cost is one DFS over the DOM, replacing a per-call
    /// subtree walk at four classification sites.
    ///
    /// **A node unreachable from the document root keeps `false`.** That is the
    /// conservative direction: `false` means "not split", which means the node
    /// stays a unit of its parent, which is the behaviour a detached subtree had
    /// before this field existed. Anonymous block boxes are not in `children`
    /// and so are not reached by this walk at all — `create_anonymous_block_boxes`
    /// sets theirs when it mints them, because by the rule above an anonymous
    /// block box (an in-flow `Block`) contributes `true`.
    fn recompute_contributes_in_flow_block(
        &mut self,
        scope: Option<&crate::ifc_scope::IfcScope>,
    ) -> Vec<(usize, Option<usize>, Option<usize>)> {
        if let Some(scope) = scope {
            return self.recompute_contributes_in_flow_block_scoped(scope);
        }
        // **The clear is defence, not a fix, and that is measured**: the mutant
        // that deletes these lines survives `-p rinch-dom -p rinch` (51
        // targets), including the transition fixture written to catch exactly
        // this — derived state that is only ever set. The reason is that the
        // fold below *writes* every node the walk reaches, `false` included, so
        // the clear can only matter for the two classes it does not reach: an
        // anonymous block box, which is minted fresh with its value every pass,
        // and a node detached from the document root, which nothing reads while
        // it is detached and which the walk reaches again the moment it is
        // re-attached (attaching seeds a structural pass over the subtree).
        // A *scoped* pass (`crate::ifc_scope`) never clears it: it recomputes
        // only its own regions and leaves a detached subtree as it was.
        //
        // It is kept because "nothing reads it while detached" is an argument
        // about every current reader, and the direction it fails in is the bad
        // one: a node that carried `true` and left the tree would keep it, so a
        // list rebuilt for a detached container — `cleanup_anonymous_block_boxes`
        // takes its parents from boxes minted on the *previous* pass and can
        // reach one — would flatten a split inline that no longer is one.
        // `false` is the pre-field behaviour, and one pass over a slab this
        // function already walks is not a cost worth arguing about.
        //
        // For `hoisted_out_of_flow_to` the clear is **load-bearing** (#591): a
        // hoisted box whose inline element left the document is not reached by
        // the walk, so the clear is what turns its `Some(host)` into `None`, and
        // the change list below is what then takes it out of the host's Taffy
        // list. Snapshot first so the change can be seen.
        let mut previous: Vec<(usize, Option<usize>)> = Vec::new();
        for (id, node) in self.tree.nodes.iter_mut() {
            node.contributes_in_flow_block = false;
            if node.hoisted_out_of_flow_to.is_some() {
                previous.push((id, node.hoisted_out_of_flow_to));
            }
            node.hoisted_out_of_flow_to = None;
            node.hosts_hoisted_out_of_flow = false;
        }

        // Post-order DFS. The `bool` is "children already pushed", so a node is
        // visited twice: once to queue its children, once to fold their answers.
        // Iterative because this is arbitrary author markup and may be deep.
        //
        // The first visit also runs the **top-down** half (#591): every node
        // carries, for its children, the nearest ancestor that owns a Taffy
        // child list (`host`) and whether a non-atomic `display: inline`
        // element has been crossed on the way down from it. An out-of-flow box
        // met with `crossed == true` is hoisted to `host`. A `display: contents`
        // wrapper passes both through unchanged; a non-atomic inline element
        // sets `crossed`; anything else — a block container, an **atomic**
        // inline (its interior is a Taffy root of its own, and a box inside it
        // is its own, which is what keeps `DropdownMenu`'s panels under its
        // `inline-block` root), an out-of-flow box, a text node — becomes the
        // host for its own children with `crossed` reset.
        //
        // A split inline (#513) is treated as a plain inline here, deliberately
        // and harmlessly: the box it would hoist is already a unit of the same
        // container through `collect_run_units`' split recursion, and the
        // `hoisted_out_of_flow_to` it gets names that same container, so
        // `box_tree_parent` and the list owner agree. (`is_split_inline` cannot
        // be consulted on the way down anyway — `contributes_in_flow_block` is
        // folded on the way up.)
        let mut ctx: std::collections::HashMap<usize, (Option<usize>, bool)> =
            std::collections::HashMap::new();
        ctx.insert(self.tree.root_id, (Some(self.tree.root_id), false));
        let mut stack: Vec<(usize, bool)> = vec![(self.tree.root_id, false)];
        while let Some((id, folded)) = stack.pop() {
            if !folded {
                stack.push((id, true));
                let Some(node) = self.tree.nodes.get(id) else {
                    continue;
                };
                let (host, crossed) = node
                    .parent
                    .and_then(|p| ctx.get(&p).copied())
                    .unwrap_or((Some(id), false));
                let role = node.inline_flow_role();
                let non_atomic_inline = role == InlineFlowRole::Inline
                    && node.is_element()
                    && node.display_mode == DisplayMode::Inline;
                let hoisted_to = if role == InlineFlowRole::OutOfFlow && crossed {
                    host
                } else {
                    None
                };
                let child_ctx = if role == InlineFlowRole::Contents {
                    (host, crossed)
                } else if non_atomic_inline {
                    (host, true)
                } else {
                    (Some(id), false)
                };
                ctx.insert(id, child_ctx);
                let children: Vec<usize> = node.children.clone();
                self.tree.nodes[id].hoisted_out_of_flow_to = hoisted_to;
                for &child_id in children.iter() {
                    stack.push((child_id, false));
                }
                continue;
            }
            let Some(node) = self.tree.nodes.get(id) else {
                continue;
            };
            let role = node.inline_flow_role();
            // `DisplayMode::Inline` exactly — see `Node::is_split_inline` for
            // why an atomic inline stops the recursion rather than continuing
            // it.
            let descends = role == InlineFlowRole::Contents
                || (role == InlineFlowRole::Inline
                    && node.is_element()
                    && node.display_mode == DisplayMode::Inline);
            let value = role == InlineFlowRole::InFlowBlock
                || (descends
                    && node.children.iter().any(|&c| {
                        self.tree
                            .nodes
                            .get(c)
                            .is_some_and(|child| child.contributes_in_flow_block)
                    }));
            // `hosts` propagates up through the transparent kinds only: the
            // host itself is the last node to carry it, and its own parent does
            // not, so the O(1) gate in `box_tree_children` fires for exactly
            // the lists that must be rebuilt from units.
            let hosts = node.children.iter().any(|&c| {
                self.tree.nodes.get(c).is_some_and(|child| {
                    child.hoisted_out_of_flow_to.is_some()
                        || (child.hosts_hoisted_out_of_flow
                            && (child.inline_flow_role() == InlineFlowRole::Contents
                                || (child.is_element()
                                    && child.display_mode == DisplayMode::Inline)))
                })
            });
            let n = &mut self.tree.nodes[id];
            n.contributes_in_flow_block = value;
            n.hosts_hoisted_out_of_flow = hosts;
        }

        // The hosts that changed, for `rehome_hoisted_out_of_flow`: every box
        // that was hoisted last pass and is hoisted elsewhere or not at all now,
        // and every box hoisted for the first time.
        let mut changes: Vec<(usize, Option<usize>, Option<usize>)> = Vec::new();
        let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::new();
        for (id, old) in previous {
            seen.insert(id);
            let new = self
                .tree
                .nodes
                .get(id)
                .and_then(|n| n.hoisted_out_of_flow_to);
            if new != old {
                changes.push((id, old, new));
            }
        }
        for (id, node) in &self.tree.nodes {
            if node.hoisted_out_of_flow_to.is_some() && !seen.contains(&id) {
                changes.push((id, None, node.hoisted_out_of_flow_to));
            }
        }
        changes
    }

    /// [`Self::recompute_contributes_in_flow_block`] over a scoped pass's
    /// regions only (`crate::ifc_scope`).
    ///
    /// The whole-document walk derives every value from the node's position
    /// under the nearest formatting container, and a formatting container
    /// **resets** the context its children see — `(Some(itself), false)`,
    /// whatever lies above it. So each region is walked on its own, from its
    /// container, and produces the values the whole-document walk would:
    ///
    /// - every region node's `hoisted_out_of_flow_to` (a stop node's too — its
    ///   hoisting is decided by the region it sits in);
    /// - a transparent node's `contributes_in_flow_block` and
    ///   `hosts_hoisted_out_of_flow`, folded post-order from its children as
    ///   the full walk folds them;
    /// - a stop node's `contributes_in_flow_block`, which for a formatting
    ///   container is its role alone. Its `hosts_hoisted_out_of_flow` reads
    ///   its own children and belongs to its own region — left alone unless it
    ///   is a container of this scope too;
    /// - the container's own `hosts_hoisted_out_of_flow` and
    ///   `contributes_in_flow_block` (its own hoisting belongs to its parent's
    ///   region).
    fn recompute_contributes_in_flow_block_scoped(
        &mut self,
        scope: &crate::ifc_scope::IfcScope,
    ) -> Vec<(usize, Option<usize>, Option<usize>)> {
        use crate::ifc_scope::is_inline_transparent;
        let mut changes: Vec<(usize, Option<usize>, Option<usize>)> = Vec::new();
        let hosts_of = |nodes: &slab::Slab<Node>, id: usize| -> bool {
            nodes[id].children.iter().any(|&c| {
                nodes.get(c).is_some_and(|child| {
                    child.hoisted_out_of_flow_to.is_some()
                        || (child.hosts_hoisted_out_of_flow
                            && (child.inline_flow_role() == InlineFlowRole::Contents
                                || (child.is_element()
                                    && child.display_mode == DisplayMode::Inline)))
                })
            })
        };
        for &container in &scope.containers {
            // (node, folded, host, crossed)
            let mut stack: Vec<(usize, bool, Option<usize>, bool)> = self.tree.nodes[container]
                .children
                .iter()
                .map(|&c| (c, false, Some(container), false))
                .collect();
            while let Some((id, folded, host, crossed)) = stack.pop() {
                let Some(node) = self.tree.nodes.get(id) else {
                    continue;
                };
                let role = node.inline_flow_role();
                let transparent = is_inline_transparent(node);
                if folded {
                    let value = node.children.iter().any(|&c| {
                        self.tree
                            .nodes
                            .get(c)
                            .is_some_and(|child| child.contributes_in_flow_block)
                    });
                    let hosts = hosts_of(&self.tree.nodes, id);
                    let n = &mut self.tree.nodes[id];
                    n.contributes_in_flow_block = value;
                    n.hosts_hoisted_out_of_flow = hosts;
                    continue;
                }
                let hoisted_to = if role == InlineFlowRole::OutOfFlow && crossed {
                    host
                } else {
                    None
                };
                let old = node.hoisted_out_of_flow_to;
                if old != hoisted_to {
                    changes.push((id, old, hoisted_to));
                }
                if transparent {
                    let child_crossed = crossed || role != InlineFlowRole::Contents;
                    let children = node.children.clone();
                    self.tree.nodes[id].hoisted_out_of_flow_to = hoisted_to;
                    stack.push((id, true, host, crossed));
                    for c in children {
                        stack.push((c, false, host, child_crossed));
                    }
                } else {
                    let n = &mut self.tree.nodes[id];
                    n.hoisted_out_of_flow_to = hoisted_to;
                    n.contributes_in_flow_block = role == InlineFlowRole::InFlowBlock;
                    if n.children.is_empty() {
                        n.hosts_hoisted_out_of_flow = false;
                    }
                }
            }
            let role = self.tree.nodes[container].inline_flow_role();
            let hosts = hosts_of(&self.tree.nodes, container);
            let n = &mut self.tree.nodes[container];
            n.contributes_in_flow_block = role == InlineFlowRole::InFlowBlock;
            n.hosts_hoisted_out_of_flow = hosts;
        }
        changes
    }

    /// Move the Taffy edge of every out-of-flow box whose host changed this
    /// pass (#591) — the record-and-restore half of the hoist, and the sentence
    /// in the design that was wrong ("no record/restore list is needed").
    ///
    /// A hoisted box's Taffy parent is its host, not its DOM parent, and the
    /// one authority for a list ([`Self::collect_effective_taffy_children`])
    /// now names it there. But an authority is consulted only when a list is
    /// **rebuilt**, and the passes that rebuild lists are keyed to other
    /// departures — an anonymous box dropped, a split undone, an inline that
    /// became block-level. None of them fires when the *hoist condition* ends:
    /// the span restyled to `inline-block` (an atomic inline is still
    /// `InlineFlowRole::Inline`, so `reattach_departed_ifc_children` skips it),
    /// or the span removed from the document outright. The stale edge then
    /// made the IFC root a non-leaf carrier of `InlineRoot` and the #466 leaf
    /// invariant panicked — measured by the review of PR 2 (T8, T12, T18), all
    /// clean before the hoist existed.
    ///
    /// So: for each box whose `hoisted_out_of_flow_to` differs from last pass,
    /// rebuild the old host's list (it stops naming the box), the new host's
    /// (it starts to), and the list of whatever the box's box-tree parent now
    /// is (for a box that is no longer hoisted, its DOM-side owner takes it
    /// back). `set_children` steals, so the order is immaterial. A box whose
    /// inline element left the document has no new host and no live owner; its
    /// edge was already cut at mutation time by `taffy_detach_contribution`,
    /// and the old host's rebuild here is the second lock on the same door.
    fn rehome_hoisted_out_of_flow(&mut self, changes: Vec<(usize, Option<usize>, Option<usize>)>) {
        let mut owners: Vec<usize> = Vec::new();
        for (id, old, new) in changes {
            for host in [old, new].into_iter().flatten() {
                if self.tree.nodes.contains(host) && !owners.contains(&host) {
                    owners.push(host);
                }
            }
            if self.tree.nodes.contains(id)
                && let Some(owner) = Self::effective_taffy_owner(&self.tree.nodes, id)
                && !owners.contains(&owner)
            {
                owners.push(owner);
            }
        }
        for owner in owners {
            self.rebuild_effective_taffy_children(owner);
        }
    }

    /// Undo the previous pass's splits (#513), so this pass can decide them
    /// again from scratch.
    ///
    /// Splitting moves an inline element's boxes out of its own Taffy child list
    /// and into its block container's. Nothing else puts them back, so an
    /// element that *stops* being split — its block child hidden, removed, or
    /// restyled inline-level — would be left with a parentless, childless Taffy
    /// node and a container whose list does not name it. That is the missing
    /// direction #597 had to add for the marking pass and #520 for
    /// `display: contents`, arrived at a third time.
    ///
    /// **Two rebuilds, and the second one is not optional.** The element's own
    /// list has to come back — that is what `taffy_child_list_owners` walking
    /// past a split inline gives, bottom-up — but the *container's* does too, and
    /// nothing else provides it once the element stops being split: an element
    /// that is no longer split is not in that walk, so the walk stops at the
    /// element itself and the container is never visited.
    ///
    /// Skipping it is not a missing optimisation, it is a **`C orphan`**, and it
    /// was measured rather than argued. `<div><a><div>card</div></a></div>`
    /// restyled to `display: block`: the element's rebuild adopts the card back
    /// out of the container (`set_children` steals), so the container's list ends
    /// **empty**, the element's Taffy node is still parentless from the split, and
    /// the container measures `h = 0` with the card laid out by an orphan. Neither
    /// `cleanup_anonymous_block_boxes` nor `create_anonymous_block_boxes` covers
    /// it, because a block-only split inline mints no run on either pass.
    /// `a_block_only_split_inline_restyled_to_a_block_container_rejoins_its_parent`
    /// is the pin, and the mutant that drops this whole function survived the
    /// suite until it existed.
    ///
    /// Order matters and is the same rule as everywhere else here: the element
    /// first, its owner second, because `set_children` removes an adopted child
    /// from its previous parent. Called **before**
    /// `create_anonymous_block_boxes`, so there are no anonymous boxes yet whose
    /// members the element's rebuild could steal.
    ///
    /// Unconditional — it does not ask whether the element is still split. A
    /// gate would have to answer "will this pass split it again", and the pass
    /// that follows answers that anyway by re-splitting; a wrong gate here is
    /// the class of bug this function exists to fix.
    fn restore_split_inlines(&mut self, was_split: Vec<usize>) {
        for id in was_split {
            if !self.tree.nodes.contains(id) {
                continue;
            }
            self.rebuild_effective_taffy_children(id);
            if let Some(owner) = Self::effective_taffy_owner(&self.tree.nodes, id) {
                self.rebuild_effective_taffy_children(owner);
            }
        }
    }

    /// Take every split inline's boxes into its block container's Taffy child
    /// list, and record the split so the next pass can undo it (#513).
    ///
    /// **Only the container's list is rebuilt, never the split inline's own**,
    /// and that asymmetry is load-bearing. By the time this runs,
    /// `create_anonymous_block_boxes` has already given each anonymous box its
    /// members, so rebuilding the inline element's list from its DOM children
    /// would steal those members straight back out of the boxes that lay them
    /// out. The element's Taffy node is left childless by the rebuild itself:
    /// every one of its children is a unit of the container — an anonymous box
    /// stands in for each inline run, and a block, out-of-flow or `display: none`
    /// child appears directly — so `set_children` on the container adopts all of
    /// them and Taffy removes each from its previous parent.
    ///
    /// **This pass is needed even when no anonymous box was minted**, which is
    /// the case `create_anonymous_block_boxes`' own rebuild cannot cover:
    /// `<a><div>card</div></a>` holds no inline content, so `has_inline` is false
    /// and no run exists, and without this the block would stay a Taffy child of
    /// the `<a>`. Measured consequence of skipping it — not hypothetical, and not
    /// merely untidy: with a block sibling before the `<a>`, Taffy positions the
    /// block relative to the `<a>` while paint reaches it directly from the
    /// container (the `<a>` is not in the box tree and its `layout` is zeroed), so
    /// it is drawn over that sibling.
    ///
    /// With a `scope`, only the scope's region nodes are looked at, and the
    /// entries the pass did not reach are kept (`kept`: what
    /// `setup_inline_formatting_contexts` left in the list after taking the
    /// in-scope ones out to restore).
    fn split_inline_boxes(&mut self, scope: Option<&crate::ifc_scope::IfcScope>) {
        let split: Vec<usize> = match scope {
            None => self
                .tree
                .nodes
                .iter()
                .filter(|(_, node)| node.is_split_inline())
                .map(|(id, _)| id)
                .collect(),
            Some(scope) => scope
                .transparent_region()
                .filter(|&id| self.tree.nodes.get(id).is_some_and(|n| n.is_split_inline()))
                .collect(),
        };
        let kept: Vec<usize> = match scope {
            None => Vec::new(),
            Some(_) => std::mem::take(&mut self.tree.split_inlines),
        };
        if split.is_empty() {
            self.tree.split_inlines = kept;
            return;
        }

        let mut owners: Vec<usize> = Vec::new();
        for &id in &split {
            if let Some(owner) = Self::effective_taffy_owner(&self.tree.nodes, id)
                && !owners.contains(&owner)
            {
                owners.push(owner);
            }
        }
        for owner in owners {
            self.rebuild_effective_taffy_children(owner);
        }

        let mut all = kept;
        all.extend(split);
        self.tree.split_inlines = all;
    }

    /// Whether any ancestor of `id` (not `id` itself) has computed `display:
    /// none`.
    ///
    /// `display` does not inherit, so a node's own computed style says
    /// nothing about whether an ancestor's `display: none` keeps it from
    /// being rendered at all — the chain has to be walked, as
    /// `style_resolution::StyleResolver::ancestors_are_rendered` walks it for
    /// the transition start gate (#703). This is `setup_inline_formatting_contexts`'s
    /// own copy rather than a shared call: that one is private to
    /// `style_resolution` and additionally understands the restyle pass's
    /// "was hidden before this cascade" list, which has no analogue here —
    /// a layout pass only ever asks about the *current* tree.
    fn has_display_none_ancestor(nodes: &slab::Slab<Node>, id: usize) -> bool {
        use crate::computed_style::values::DisplayValue;
        let mut current = nodes.get(id).and_then(|n| n.parent);
        while let Some(pid) = current {
            let Some(parent) = nodes.get(pid) else {
                break;
            };
            if parent.computed_style.display == DisplayValue::None {
                return true;
            }
            current = parent.parent;
        }
        false
    }

    /// Detect IFC roots and mark inline children.
    ///
    /// An element is an IFC root if it's a block container that has any
    /// inline children (text nodes, inline elements, etc.). Even single
    /// text children use IFC — this avoids maintaining two measurement
    /// paths (standalone Taffy vs IFC) and the sync bugs that arise when
    /// elements transition between them during editing.
    ///
    /// With a `scope` (`crate::ifc_scope`) every step runs over the scope's
    /// containers and their regions only, and everything else keeps the state
    /// the last pass that reached it left: its splices, boxes, splits, leaves,
    /// marks and root contexts, and so Taffy's cached layout for it.
    pub(crate) fn setup_inline_formatting_contexts(
        &mut self,
        scope: Option<&crate::ifc_scope::IfcScope>,
    ) {
        // Before everything else: the classification every pass below consumes.
        // `collect_run_units` reads it, so `box_tree_children` and therefore
        // `collect_effective_taffy_children` do — which means the cleanups and
        // the rebuilds below all do. Reading a previous pass's value here would
        // rebuild a Taffy child list from a classification this pass is about to
        // contradict; see the function's own doc.
        let hoist_changes = self.recompute_contributes_in_flow_block(scope);

        // Clean up the previous pass's measure leaves, anonymous block boxes and
        // splits, then recreate all three for the current DOM state.
        self.cleanup_ifc_measure_leaves(scope);
        let was_split = match scope {
            None => std::mem::take(&mut self.tree.split_inlines),
            Some(scope) => {
                // Restore the in-scope entries (and any that left the slab);
                // keep the rest, which `split_inline_boxes` appends to.
                let nodes = &self.tree.nodes;
                let (go, keep): (Vec<usize>, Vec<usize>) =
                    std::mem::take(&mut self.tree.split_inlines)
                        .into_iter()
                        .partition(|&id| scope.nodes.contains(&id) || !nodes.contains(id));
                self.tree.split_inlines = keep;
                go
            }
        };
        let retired = self.cleanup_anonymous_block_boxes(scope);
        self.restore_split_inlines(was_split);
        // Before the boxes are minted, like the split restore, so a host's
        // rebuilt list names members rather than boxes that are about to exist
        // (#591).
        self.rehome_hoisted_out_of_flow(hoist_changes);
        self.create_anonymous_block_boxes(scope, retired);
        // After the boxes exist, so the container's rebuilt list names them
        // rather than their members (#513).
        self.split_inline_boxes(scope);

        // `ifc_root` is *derived* state — "this node's boxes are drawn by that
        // IFC, so the paint tree-walk must skip it" — and the marking pass below
        // only ever *sets* it. Nothing clears it when a node stops being inline
        // content: `clear_ifc_root_recursive` only fires on the subtree being
        // moved, so a `display:contents` wrapper that was IFC content in an
        // earlier pass keeps its mark forever once content is appended *into* it
        // (an `if` branch that starts hidden, a reactive component whose root
        // turns block-level, a `for` list that starts empty). Paint then skips
        // that wrapper and everything under it, which is exactly the bug this
        // module's contents handling exists to avoid. This pass recomputes every
        // mark, so reset them all first rather than trying to invalidate at each
        // mutation site.
        //
        // A scoped pass resets the marks of its regions only — which include
        // every stop node, whose mark is its region's to give — and never a
        // container's own: that one belongs to the region *above* it. The two
        // registries lose the same entries, and the marking below puts back
        // the ones that still hold.
        let candidates: Vec<usize> = match scope {
            None => {
                for (_id, node) in self.tree.nodes.iter_mut() {
                    node.ifc_root = None;
                }
                self.tree.ifc_root_registry.clear();
                self.tree.atomic_inline_registry.clear();
                self.tree.nodes.iter().map(|(id, _)| id).collect()
            }
            Some(scope) => {
                for &(id, _, stop) in &scope.region {
                    if let Some(node) = self.tree.nodes.get_mut(id) {
                        node.ifc_root = None;
                    }
                    self.tree.atomic_inline_registry.remove(&id);
                    if !stop {
                        self.tree.ifc_root_registry.remove(&id);
                    }
                }
                let mut c: Vec<usize> = Vec::new();
                for &container in &scope.containers {
                    self.tree.ifc_root_registry.remove(&container);
                    c.push(container);
                    c.extend(self.tree.nodes[container].run_boxes.iter().copied());
                }
                c
            }
        };
        // Old boxes (freed by the cleanup above) may still be registered.
        if let Some(scope) = scope {
            for &id in &scope.nodes {
                if !self.tree.nodes.contains(id) {
                    self.tree.ifc_root_registry.remove(&id);
                }
            }
        }

        let mut ifc_roots: Vec<usize> = Vec::new();
        for id in candidates {
            let Some(node) = self.tree.nodes.get(id) else {
                continue;
            };
            if !node.is_element() {
                continue;
            }
            // Only block containers can be IFC roots
            if !node.display_mode.is_block_container() {
                continue;
            }
            // A `display: contents` element generates no box, so it can never
            // establish an inline formatting context — its inline children belong
            // to the nearest real (block or flex) ancestor. In a flex container
            // its children blockify into flex items (issue #41); in a block
            // container that block establishes the IFC over the flattened inline
            // content (issue #61, handled below). Either way the contents node is
            // never an IFC root itself.
            if node.computed_style.display == crate::computed_style::values::DisplayValue::Contents
            {
                continue;
            }
            // A `display: none` element generates no box at all — not even
            // the degenerate one `Contents` leaves to its nearest real
            // ancestor — so it establishes no inline formatting context
            // over its own subtree either. Before this, a hidden block with
            // inline children (`div{display:none}<span>text</span>`) was
            // still made an IFC root: its children were detached from Taffy
            // and a Parley `InlineLayout` was built for text that can never
            // paint, on every rebuild-all pass (#509). The marking pass's
            // existing `NoBox` arm (`mark_inline_descendants`,
            // `inline_flow_role`) already detaches a `display: none`
            // *child* of some other root the same way `Contents` is
            // detached above, so skipping here loses no coverage — it only
            // stops this node from being asked to run that pass on itself.
            if node.computed_style.display == crate::computed_style::values::DisplayValue::None {
                continue;
            }

            // Classify the children once. A comment answers `is_inline()`
            // with `true` (it flows with inline content and must not split a
            // run), but it is not inline *content*: it renders nothing and has
            // no Taffy node. It therefore must not establish an IFC on its own
            // when non-inline children are present (#466): `show_dom`'s marker
            // comment next to its branch root under a block parent used to
            // make that parent an IFC root *while the branch root stayed
            // attached as a Taffy child* — a non-leaf carrying `InlineRoot`,
            // minted fresh every pass through this very loop, so the stale
            // sweep below could never touch it. (`create_anonymous_block_boxes`
            // already excludes comments from `has_inline` for the same reason.)
            //
            // With no other inline content in reach, withholding the mark is
            // observationally identical — the context was dead data Taffy's
            // block arm never consults (the leaf invariant on
            // [`NodeContext::InlineRoot`]). With contents-wrapped text beside
            // the comment it is a *fix* (#490): `mark_inline_descendants`
            // detached that text into the root whose measure could never run,
            // so the line contributed nothing to the height and painted at
            // y = 0 over the block sibling — `div { if x { "text" } Block{} }`
            // rendered corrupted while the same markup without the marker
            // comment rendered correctly. Unmarked, the text stays in Taffy
            // as an ordinary text leaf and the container matches its
            // comment-free twin exactly.
            //
            // Out-of-flow children are excluded from `has_non_comment_inline`
            // because [`Node::inline_flow_role`] classifies them `OutOfFlow`,
            // not `Inline` — an absolute box is not inline content. (Under the
            // hood that still rests on Stylo blockifying every out-of-flow box
            // — `style_adjuster.rs`, `blockify_if!(is_absolutely_positioned)`
            // — but the reliance is now stated once, in the classifier, not
            // leaned on silently per site; #406.)
            //
            // A child already claimed by an anonymous block box is **not this
            // node's inline content** — it is the box's, and the box is the
            // root that lays it out (#566). Before the box left the DOM tree
            // that was automatic, because the run had been reparented out of
            // `children`; now `children` still holds it and the classification
            // has to say so itself. Missing this makes a mixed container an IFC
            // root *as well as* its boxes, which trips the #466 leaf invariant
            // immediately: the container keeps the block's Taffy child, so its
            // `InlineRoot` context sits on a non-leaf and its measure is
            // structurally unreachable.
            //
            // An anonymous block box classifies over its **run**, not its
            // `children`, which are empty — it is not in the element tree
            // (#566). Without this it is never discovered as a root at all and
            // its whole run goes unlaid-out.
            // **Units, not children** (#568), for the same reason
            // `box_tree_children` reads them: a run is grouped over the
            // flattened list, so a wrapper whose inline content has been taken
            // into a run must not offer that content here a second time. Left
            // as `children`, the wrapper reaches the `Contents` arm below, the
            // container is judged to hold inline content it no longer owns, and
            // it becomes an IFC root *while also* parenting a box — which is
            // the #466 leaf invariant violated, measured, by
            // `a_transparent_wrappers_inline_content_stays_reachable`.
            //
            // An anonymous box still classifies over its **run**: its
            // `children` are empty because it is not in the element tree
            // (#566), and its units would be empty for the same reason.
            let mut unit_buf: Vec<usize> = Vec::new();
            let own_children: &[usize] = if node.is_anonymous_block_box {
                &node.run_members
            } else {
                Self::collect_run_units(&self.tree.nodes, id, &mut unit_buf);
                &unit_buf
            };
            let mut has_non_comment_inline = false;
            let mut all_children_are_comments = !own_children.is_empty();
            for &child_id in own_children {
                let Some(child) = self.tree.nodes.get(child_id) else {
                    continue;
                };
                if child.run_box.is_some() && !node.is_anonymous_block_box {
                    all_children_are_comments = false;
                    continue;
                }
                match child.inline_flow_role() {
                    InlineFlowRole::Comment => continue,
                    // As above: units carry no wrapper, so `Contents` rides
                    // with the non-inline arm rather than claiming to be a case.
                    InlineFlowRole::Inline => {
                        all_children_are_comments = false;
                        has_non_comment_inline = true;
                    }
                    _ => all_children_are_comments = false,
                }
            }

            // Activate IFC for any block element with inline children.
            // Even a single text child uses IFC — it's a degenerate case with one
            // text range. This avoids needing two measurement paths (standalone vs IFC)
            // and the sync bugs that arise when elements transition between them.
            //
            // A container holding nothing but comments keeps the root+measure
            // path it always had (a collapsed `show_dom` branch is exactly
            // this shape) — the comment rule above only withholds roothood
            // when a non-inline child would stay attached.
            //
            // #61's case — the only inline content lives *behind*
            // `display: contents` wrapper(s), as rsx `if`/`match` emit — needs
            // no separate test any more. It is answered by
            // `has_non_comment_inline` directly, because the scan above reads
            // the container's **units** and the collector has already flattened
            // those wrappers (#568). `contents_wraps_only_inline` supplied it
            // until #586 and is deleted; the measurement is in that issue, and
            // the short form is that it never once decided this condition
            // across the whole suite.
            // A childless block container is **not** floored at a line (#296):
            // with no in-flow content it has no line box, so its auto height is
            // 0 (CSS 2.1 §10.6.3), as in every browser. The floor that used to
            // be written here was written for `<input>`/`<textarea>`, which had
            // no content height, and also carried a blockified `<br>` and the
            // non-text input types; the line-sized ones are measured now (#297,
            // `form_control.rs`).
            //
            // A control or replaced element is a root whatever its children
            // are (#1178, #1288): the UA sheet makes every element child of one
            // `display: none`, the marking pass detaches those from Taffy — one
            // attached `None` child left the measure unreachable and the
            // element collapsed to 0x0 — and the hollow arm below gives it its
            // own measure context.
            let hollow_with_children =
                !own_children.is_empty() && crate::replaced::ignores_inner_display(node);
            if (has_non_comment_inline || all_children_are_comments || hollow_with_children)
                // A node whose own `display` isn't `none` can still sit under
                // an ancestor that is: `display` does not inherit, so this
                // node's own computed style says nothing about whether it is
                // rendered at all (css-display-3 §2.4, the "not being
                // rendered" rule #703 reads the same way for transitions).
                // Such a subtree is never painted and Taffy's own compute
                // does not descend past the `display: none` ancestor to
                // begin with, so marking a node here buys nothing but a
                // Parley `InlineLayout` that this pass rebuilds every time
                // it runs (#826 measured it growing with the hidden
                // subtree's text, unboundedly, for as long as the subtree
                // exists). Walked only here, once per node this loop has
                // already decided would otherwise be a root — not once per
                // candidate — so the O(depth) cost lands on the small set
                // that would otherwise pay far more.
                && !Self::has_display_none_ancestor(&self.tree.nodes, id)
            {
                ifc_roots.push(id);
            }
        }

        self.tree
            .ifc_root_registry
            .extend(ifc_roots.iter().copied());
        // Put back every box a past marking pass took out and that is no longer
        // the IFC's to hold (#597). After root discovery, because whether a
        // departed box stays out depends on whether its owner is a root *this*
        // pass, not merely whether it could be one: a block whose last inline
        // child went `display: none` is no root, and its hidden children must
        // rejoin its Taffy list as they would in a document built that way.
        // Before the marking pass below, so that a rebuilt list is re-detached
        // by it: the rebuild restores the owner's *whole* effective child list,
        // inline children included, and the pass that follows removes exactly
        // the ones that are still inline content. On a scoped pass the roots
        // are the scope containers' and their run boxes' — and so is every
        // candidate's owner, since the candidates are the scope's regions — so
        // the gate stays exact there too.
        self.reattach_departed_ifc_children(&ifc_roots, scope);

        for &root_id in &ifc_roots {
            let root_taffy = match self.tree.nodes[root_id].taffy_id {
                Some(t) => t,
                None => continue,
            };

            // Remove inline children from Taffy (Parley handles their layout),
            // flattening through any `display:contents` wrappers so their inline
            // grandchildren join this root's IFC (issue #61).
            self.mark_inline_descendants(root_id, root_id, root_taffy);

            // Decide which Taffy node carries `InlineRoot` — the leaf
            // invariant (#466, see [`NodeContext::InlineRoot`]): Taffy
            // consults a measure function only on a childless node, so the
            // carrier must be one. When inline detachment emptied the root's
            // Taffy node, the root itself is the carrier (the common case,
            // below). When out-of-flow children remain attached — `has_block`
            // no longer mints an anonymous box for `text + absolute` (#406) —
            // the root is a non-leaf whose measure would be structurally
            // unreachable, so a Taffy-only **measure leaf** child carries the
            // context instead, and Taffy keeps doing 100% of the out-of-flow
            // layout (containing block, inset resolution, static position).
            //
            // The decision reads the **DOM**, not the current Taffy
            // attachment. It was written because `compute_taffy_child_index`
            // counted DOM siblings blind to attachment and Taffy's
            // out-of-range error was swallowed, so an out-of-flow child
            // inserted between frames could be attached **nowhere** (#477).
            // That is fixed — the index comes from the parent's attached list
            // now, and a refused insert reports.
            //
            // Reading the DOM stays right for a reason the fix does not touch,
            // and it is about **membership, not order**: what this loop needs
            // to know is *which* of the root's children are out-of-flow, which
            // is a fact about each child's computed style
            // (`inline_flow_role`) and not about where Taffy currently holds
            // it. The attached list cannot answer it — the measure leaf in it
            // has no DOM identity at all, and a stale splice can leave ids in
            // it that name no current child. So the DOM is the only thing that
            // can be asked.
            //
            // (Do not restate this as "a late insert lands ahead of the
            // measure leaf": that is true only when the new box has no
            // attached preceding sibling. With two out-of-flow children
            // already present the insert lands *after* the leaf and in the
            // very order this canonicalization is about to impose — measured,
            // and it is what `taffy_child_index_tests`'
            // `an_insert_into_a_container_with_a_measure_leaf_...` builds.)
            let mut out_of_flow_children: Vec<taffy::NodeId> = Vec::new();
            let mut in_flow_stays_attached = false;
            // **The root's box-tree children, not its DOM `children`** (#591).
            // An out-of-flow box beneath a non-atomic inline element is a unit
            // of its host container (`collect_run_units`), so it appears here
            // as an `OutOfFlow` unit of the root that hosts it and is collected
            // like a direct one — and is absent from a root that does not host
            // it: under a mixed-content container the IFC root is an anonymous
            // block box and the box's host is the *container*, whose Taffy list
            // `collect_effective_taffy_children` names it in, which is what
            // makes the container (the CSS containing block) rather than the
            // anonymous box the Taffy parent it resolves its insets against.
            // For an anonymous box root this is its run, verbatim (#566).
            let root_scan: Vec<usize> = Self::box_tree_children(&self.tree.nodes, root_id).to_vec();
            for &child_id in &root_scan {
                let Some(child) = self.tree.nodes.get(child_id) else {
                    continue;
                };
                match child.inline_flow_role() {
                    // A comment has no Taffy node at all; a `display: none`
                    // child generates no box and was detached by the marking
                    // pass (#487); inline content was detached into this IFC.
                    InlineFlowRole::Comment | InlineFlowRole::NoBox => {}
                    // Inline content was detached into this IFC. An out-of-flow
                    // box beneath a non-atomic inline element is not reached
                    // through it here — it is a unit of its host in its own
                    // right and arrives through the `OutOfFlow` arm (#591).
                    InlineFlowRole::Inline => {}
                    InlineFlowRole::Contents => {
                        if Self::contents_is_inline_transparent(&self.tree.nodes, child_id) {
                            // A transparent wrapper's flattened *in-flow*
                            // content was detached by the recursion above —
                            // but its out-of-flow descendants (which no
                            // longer make it opaque, #289) were reparented
                            // into this root's Taffy node by
                            // `sync_display_contents` and stay attached.
                            // Collect them so the canonicalization below
                            // keeps them laid out by Taffy.
                            Self::collect_contents_out_of_flow(
                                &self.tree.nodes,
                                child_id,
                                &mut out_of_flow_children,
                            );
                        } else {
                            // An opaque one stopped the marking pass,
                            // leaving its flattened in-flow boxes (and any
                            // later siblings) attached.
                            in_flow_stays_attached = true;
                        }
                    }
                    InlineFlowRole::OutOfFlow => {
                        if let Some(child_taffy) = child.taffy_id {
                            out_of_flow_children.push(child_taffy);
                        }
                    }
                    InlineFlowRole::InFlowBlock => {
                        if child.taffy_id.is_some() {
                            in_flow_stays_attached = true;
                        }
                    }
                }
            }

            if !in_flow_stays_attached && !out_of_flow_children.is_empty() {
                // Canonicalize the root's Taffy children to exactly the
                // DOM-ordered out-of-flow children — replacing whatever
                // attachment history (including #477 damage) left behind —
                // then put the measure leaf at **index 0, deliberately**: a
                // block child's static position follows its siblings, so the
                // leaf-first order preserves today's (and browsers')
                // below-the-line static position for auto-inset absolute
                // children. Appending it would silently move them to
                // content-top.
                let _ = self
                    .tree
                    .taffy
                    .set_children(root_taffy, &out_of_flow_children);
                let Ok(leaf) = self.tree.taffy.new_leaf_with_context(
                    taffy::Style {
                        display: taffy::Display::Block,
                        ..Default::default()
                    },
                    NodeContext::InlineRoot(root_id),
                ) else {
                    continue;
                };
                let _ = self.tree.taffy.insert_child_at_index(root_taffy, 0, leaf);
                self.tree.ifc_measure_leaves.insert(root_id, leaf);

                // The context is a **move**, not a copy: the root itself has
                // children now, so a stale `InlineRoot` left on it from an
                // earlier pass must go (the sweep below skips this-pass
                // roots). In-place write — the canonicalization above already
                // dirtied the node.
                if let Some(ctx) = self.tree.taffy.get_node_context_mut(root_taffy)
                    && matches!(ctx, NodeContext::InlineRoot(_))
                {
                    *ctx = NodeContext::Element;
                }
            } else if crate::replaced::ignores_inner_display(&self.tree.nodes[root_id]) {
                // A hollow control (#1159): its children are its value, not
                // content, so it is measured exactly as a childless control is
                // — the context `sync_form_control_measure` gives one — and
                // `build_ifc_layouts` shapes nothing for it. Written only when
                // it differs, so a pass that finds it already hollow leaves
                // Taffy's cache alone.
                // A canvas, video or iframe is hollow the same way (#1173):
                // its children are fallback content a browser does not
                // render, and it keeps its natural-size context.
                let want =
                    crate::replaced::replaced_context(&self.tree.nodes[root_id]).or_else(|| {
                        crate::form_control::hollow_control_context(
                            &self.tree.nodes[root_id],
                            &mut self.font_cx,
                            &mut self.layout_cx,
                            self.tree.font_generation,
                            &self.tree.perf,
                        )
                    });
                let have = self.tree.taffy.get_node_context(root_taffy);
                let same = match (&want, have) {
                    (
                        Some(NodeContext::FormControl {
                            content_width: aw,
                            content_height: ah,
                        }),
                        Some(NodeContext::FormControl {
                            content_width: bw,
                            content_height: bh,
                        }),
                    ) => aw == bw && ah == bh,
                    (
                        Some(NodeContext::Replaced {
                            width: aw,
                            height: ah,
                            ratio: ar,
                            aspect: aa,
                        }),
                        Some(NodeContext::Replaced {
                            width: bw,
                            height: bh,
                            ratio: br,
                            aspect: ba,
                        }),
                    ) => aw == bw && ah == bh && ar == br && aa == ba,
                    (None, None) => true,
                    _ => false,
                };
                if !same {
                    let _ = self.tree.taffy.set_node_context(root_taffy, want);
                }
            } else {
                // Today's path: the root's own (now childless) Taffy node
                // carries the context so the measure function fires for it.
                // With an in-flow child still attached this leaves a non-leaf
                // carrier exactly as before — bug-for-bug; the debug validator
                // below owns flagging that shape.
                if let Some(ctx) = self.tree.taffy.get_node_context_mut(root_taffy) {
                    *ctx = NodeContext::InlineRoot(root_id);
                } else {
                    // Element nodes don't have context by default — we need to set one.
                    // Taffy only calls measure for nodes with context, so we must ensure it has one.
                    let _ = self
                        .tree
                        .taffy
                        .set_node_context(root_taffy, Some(NodeContext::InlineRoot(root_id)));
                }
            }
        }

        // Stale-context sweep (#466). The loop above only ever *sets*
        // `InlineRoot` — nothing clears one when a node stops being an IFC
        // root. A container that was all-inline (and got the context) and then
        // gained a block child is no longer a root — an anonymous box takes
        // over its inline run — but its Taffy node keeps the stale `InlineRoot`
        // while now having Taffy children. That context is dead data: Taffy
        // consults a measure function only on a childless node (the leaf
        // invariant on [`NodeContext::InlineRoot`]), so clearing it is
        // observationally identical — which is exactly why this sweep is
        // restricted to nodes whose Taffy node *has children*. A childless
        // stale carrier is deliberately left alone: its measure IS reachable,
        // so clearing it would change measure behaviour. The `children > 0`
        // guard is load-bearing — do not widen it.
        //
        // A scoped pass sweeps its own nodes: only a container of the scope can
        // have stopped being a root (its units are what changed), or a region
        // node whose own display changed — and that puts it in the scope too.
        let roots_this_pass: std::collections::HashSet<usize> = ifc_roots.iter().copied().collect();
        let sweep: Vec<usize> = match scope {
            None => self.tree.nodes.iter().map(|(id, _)| id).collect(),
            Some(scope) => {
                let mut v: Vec<usize> = scope.rootable.iter().copied().collect();
                v.sort_unstable();
                v
            }
        };
        for id in sweep {
            let Some(node) = self.tree.nodes.get(id) else {
                continue;
            };
            if roots_this_pass.contains(&id) {
                continue;
            }
            let Some(taffy_id) = node.taffy_id else {
                continue;
            };
            if !matches!(
                self.tree.taffy.get_node_context(taffy_id),
                Some(NodeContext::InlineRoot(_))
            ) {
                continue;
            }
            if self.tree.taffy.children(taffy_id).map_or(0, |c| c.len()) == 0 {
                continue;
            }
            if let Some(ctx) = self.tree.taffy.get_node_context_mut(taffy_id) {
                // In-place write, not `set_node_context`, which would
                // `mark_dirty` the node and invalidate Taffy's layout cache —
                // the sweep must be observationally neutral.
                *ctx = NodeContext::Element;
            }
        }

        #[cfg(debug_assertions)]
        {
            let mut violations = self.ifc_leaf_invariant_violations();
            // A scoped pass sets up nothing outside the document: a detached
            // subtree keeps whatever state it left with (an element emptied by
            // `set_text_content` and then removed still holds its new text as
            // a Taffy child of its `InlineRoot`), and attaching it seeds it
            // again. Everything **connected** must hold — including what the
            // scope did not reach, which is the claim this checks.
            if scope.is_some() {
                let nodes = &self.tree.nodes;
                let root = self.tree.root_id;
                violations.retain(|&id| {
                    let mut cur = Some(id);
                    while let Some(c) = cur {
                        if c == root {
                            return true;
                        }
                        cur = nodes.get(c).and_then(|n| n.parent);
                    }
                    false
                });
            }
            debug_assert!(
                violations.is_empty(),
                "IFC leaf invariant violated (#466): Taffy node(s) carrying \
                 NodeContext::InlineRoot have Taffy children after \
                 setup_inline_formatting_contexts (DOM node ids {violations:?}). \
                 Taffy consults a measure function only on a childless node, so \
                 these roots' inline measure is structurally unreachable and \
                 their auto height collapses to 0."
            );
        }
    }

    /// Every violation of the IFC leaf invariant (#466): DOM nodes whose Taffy
    /// node carries [`NodeContext::InlineRoot`] while having Taffy children.
    ///
    /// Taffy (0.12 and 0.14 alike) consults a measure function only on a node with zero
    /// children, so a non-leaf carrying `InlineRoot` can never be measured —
    /// an auto-height IFC root in that state collapses to `h = 0`. After
    /// [`Self::setup_inline_formatting_contexts`] this must be empty; a
    /// `debug_assertions` check there enforces it.
    ///
    /// Coverage note: this walks DOM-owned Taffy nodes (elements and anonymous
    /// block boxes both live in the slab) **and** the Taffy-only measure
    /// leaves in `ifc_measure_leaves` (#466), which have no DOM identity. If a
    /// future change hands the context to yet another kind of carrier, that
    /// carrier must be added to this walk. A measure-leaf violation is
    /// reported under its IFC root's DOM id.
    pub fn ifc_leaf_invariant_violations(&self) -> Vec<usize> {
        let mut violations = Vec::new();
        for (id, node) in &self.tree.nodes {
            let Some(taffy_id) = node.taffy_id else {
                continue;
            };
            if matches!(
                self.tree.taffy.get_node_context(taffy_id),
                Some(NodeContext::InlineRoot(_))
            ) && self.tree.taffy.children(taffy_id).map_or(0, |c| c.len()) > 0
            {
                violations.push(id);
            }
        }
        // Measure leaves must carry the context (a leaf without it makes its
        // root's measure unreachable just as surely as a non-leaf carrier
        // does) and must themselves be childless.
        for (&root_id, &leaf) in &self.tree.ifc_measure_leaves {
            let carries = matches!(
                self.tree.taffy.get_node_context(leaf),
                Some(NodeContext::InlineRoot(_))
            );
            if !carries || self.tree.taffy.children(leaf).map_or(0, |c| c.len()) > 0 {
                violations.push(root_id);
            }
        }
        violations
    }

    /// Violations of the **DOM** tree's own invariants — the check
    /// `taffy_tree_violations` structurally could not make (#578).
    ///
    /// That validator compares the Taffy tree against itself, which is why it
    /// answered `[]` for both #566 reconciler failures: a deleted row still
    /// painted and an inserted row never laid out were **DOM** corruption with
    /// a perfectly consistent Taffy tree underneath. It was not blind by
    /// accident; it validates a different thing.
    ///
    /// Three invariants, and the first is the one that catches that class:
    ///
    /// - **A** bidirectional: `nodes[n].parent == Some(p)` **iff**
    ///   `nodes[p].children` contains `n`.
    /// - **B** no node appears in two parents' `children`.
    /// - **C** no `parent` or `children` entry names a freed slab index.
    ///
    /// **A was not expressible before #566**, and that is the point worth
    /// keeping: the anonymous block box used to violate it deliberately — it
    /// reparented a run, so a member's `parent` disagreed with the author's
    /// tree on purpose and a check like this would have fired on legitimate
    /// state. Nothing is reparented now, so the invariant is true of every node
    /// and can be asserted. The redesign is what made the validator possible,
    /// which is why it lands with it rather than before it.
    ///
    /// The anonymous box itself is exempt from **A's backward direction** by
    /// construction: it names a `parent` and appears in nobody's `children`,
    /// which is exactly the state A forbids for an ordinary node. Only that
    /// direction is waived — see the comment on the backward pass. That
    /// exemption is the one place this check has to know the design exists,
    /// and it is stated rather than silently skipped.
    pub fn dom_tree_violations(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen_child: std::collections::HashMap<usize, usize> =
            std::collections::HashMap::new();

        for (id, node) in &self.tree.nodes {
            // C: children must name live slab entries.
            for &c in &node.children {
                if self.tree.nodes.get(c).is_none() {
                    out.push(format!(
                        "C freed child: {id} lists {c}, which is not in the slab"
                    ));
                    continue;
                }
                // B: one parent per node.
                if let Some(&other) = seen_child.get(&c) {
                    out.push(format!(
                        "B double-parent: {c} is a child of both {other} and {id}"
                    ));
                } else {
                    seen_child.insert(c, id);
                }
                // A, forward: a child must point back.
                if self.tree.nodes[c].parent != Some(id) {
                    out.push(format!(
                        "A one-way: {id} lists {c} as a child, but {c}.parent is {:?}",
                        self.tree.nodes[c].parent
                    ));
                }
                // A, exclusivity: exactly one list, never both.
                if node.run_boxes.contains(&c) {
                    out.push(format!(
                        "A both lists: {c} is in {id}'s children AND its run_boxes"
                    ));
                }
            }

            // The same three checks over the box list. A box is a child of the
            // box tree, not of the author's tree, so it lives here instead of
            // in `children` — but it is otherwise an ordinary node and A, B and
            // C say exactly the same things about it. Folding `run_boxes` into
            // all three is what makes A total; folding it into A alone would
            // leave B and C blind to the new list, which is the very shape
            // (a check weakest where it must hold) this validator exists to
            // avoid.
            for &b in &node.run_boxes {
                // C: box entries must name live slab entries.
                let Some(boxx) = self.tree.nodes.get(b) else {
                    out.push(format!(
                        "C freed run box: {id} lists {b}, which is not in the slab"
                    ));
                    continue;
                };
                // B: one parent per node, across both lists.
                if let Some(&other) = seen_child.get(&b) {
                    out.push(format!(
                        "B double-parent: {b} is listed by both {other} and {id}"
                    ));
                } else {
                    seen_child.insert(b, id);
                }
                // A, forward.
                if boxx.parent != Some(id) {
                    out.push(format!(
                        "A one-way: {id} lists {b} as a run box, but {b}.parent is {:?}",
                        boxx.parent
                    ));
                }
                // Only an anonymous box belongs in this list. Nothing else may
                // acquire a parent that does not list it in `children`.
                if !boxx.is_anonymous_block_box {
                    out.push(format!(
                        "A not a box: {id} lists {b} as a run box, but it is not an anonymous block box"
                    ));
                }
            }
            // C: a parent must name a live slab entry.
            if let Some(p) = node.parent
                && self.tree.nodes.get(p).is_none()
            {
                out.push(format!(
                    "C freed parent: {id}.parent is {p}, which is not in the slab"
                ));
            }
        }

        // A, backward: a node claiming a parent must appear in **exactly one**
        // of that parent's two lists.
        //
        // **There is no exemption, and that is the point.** The anonymous block
        // box used to need one: it carries `parent = Some(container)` and is
        // deliberately absent from `children`, which is the state A forbids for
        // an ordinary node. An invariant introduced to catch DOM corruption
        // would then have carried a carve-out for the exact construct whose old
        // shape produced the corruption — weakest precisely where it must hold.
        // Recording the downward edge in `run_boxes` removes the need: the box
        // is in one list, every other node is in the other, and A is a single
        // sentence true of every node in the slab.
        //
        // Note this direction is what would have caught §11 by itself. The
        // superseded `parent: None` build asserted the opposite — that a box
        // has no parent at all — so giving the box its parent back turned that
        // assertion from silent to firing on every box, every frame.
        for (id, node) in &self.tree.nodes {
            let Some(p) = node.parent else { continue };
            let Some(parent) = self.tree.nodes.get(p) else {
                continue; // reported as "C freed parent" above
            };
            let in_children = parent.children.contains(&id);
            let in_run_boxes = parent.run_boxes.contains(&id);
            if !in_children && !in_run_boxes {
                out.push(format!(
                    "A one-way: {id}.parent is {p}, but neither {p}'s children nor its run_boxes contain it"
                ));
            }
            // The "exactly one" half is reported from the forward pass, which
            // sees both lists of the same node together.
            let _ = in_run_boxes;
        }

        // D: the run relation is bidirectional, like A is for the DOM edge.
        // A member names its box and the box names it back, or one of the two
        // is stale — which is the corruption shape #566's own construct could
        // introduce, so it is checked rather than trusted.
        for (id, node) in &self.tree.nodes {
            if let Some(b) = node.run_box {
                match self.tree.nodes.get(b) {
                    None => out.push(format!(
                        "D freed run box: {id}.run_box is {b}, which is not in the slab"
                    )),
                    Some(boxx) if !boxx.is_anonymous_block_box => out.push(format!(
                        "D not a box: {id}.run_box is {b}, which is not an anonymous block box"
                    )),
                    Some(boxx) if !boxx.run_members.contains(&id) => out.push(format!(
                        "D one-way run: {id}.run_box is {b}, but {b}'s members do not contain it"
                    )),
                    Some(_) => {}
                }
            }
            for &m in &node.run_members {
                if self.tree.nodes.get(m).is_none() {
                    out.push(format!(
                        "D freed member: {id} lists member {m}, which is not in the slab"
                    ));
                } else if self.tree.nodes[m].run_box != Some(id) {
                    out.push(format!(
                        "D one-way run: {id} lists {m} as a member, but {m}.run_box is {:?}",
                        self.tree.nodes[m].run_box
                    ));
                }
            }
        }

        out
    }

    /// The box hangs off the container its run actually came from (#566, #578).
    ///
    /// **Separate from [`Self::dom_tree_violations`] on purpose, and the
    /// separation is the whole point.** That one holds at *any* moment — it is
    /// about the DOM tree, which every mutation keeps consistent. This one is
    /// about the **run bookkeeping**, which is rebuilt by
    /// `create_anonymous_block_boxes` and is therefore only meaningful
    /// immediately after a layout pass. Folding it into the any-time validator
    /// would make a clean run mean *less*, because a clean run would then
    /// depend on when you asked.
    ///
    /// The rule:
    ///
    /// > for an anonymous box `b` with `parent = Some(p)`, every `m` in
    /// > `b.run_members` has `m.parent == Some(p)`, and `run_members` is
    /// > non-empty.
    ///
    /// It closes the one direction the design's upward edge depends on and
    /// nothing else checks. Invariant A pins a box's `parent` to the container
    /// that *lists* it in `run_boxes`; it says nothing about whether that
    /// container is the one whose children the run was grouped from. A box
    /// could hang off `C` while its members are children of `D` and every check
    /// in `dom_tree_violations` would pass — measured, not argued.
    ///
    /// **Why it cannot be an any-time invariant**, also measured: ordinary DOM
    /// mutation desynchronises the two legitimately. `remove_child(c, m)`
    /// followed by `append_child(d, m)` between layout passes leaves `m.parent
    /// == d` while its box still hangs off `c` and still lists it — correct
    /// state, since the run is rebuilt on the next pass, and an any-time form
    /// of this rule would fire on it. That is the cry-wolf failure an invariant
    /// must not have.
    ///
    /// The non-empty half is not vacuous filler: a memberless box is
    /// **unreachable** at validator time — `create_anonymous_block_boxes` mints
    /// a box only from a non-empty run, and nothing empties `run_members`
    /// afterwards (`remove_child` does not touch it). Stating it keeps the rule
    /// from passing trivially if that ever changes, which is exactly how the
    /// first draft of this assertion would have gone quietly vacuous.
    ///
    /// # The three sites that must agree, and how a drift is caught
    ///
    /// The classification, [`crate::RinchDocument::box_tree_children`] and this
    /// check are three readings of one list, so all three call
    /// **`collect_run_units`** rather than keeping parallel spellings in step.
    /// That is deliberate: #518, #476 and #568 were each *two sites asking one
    /// question two ways*, and construction beats discipline.
    ///
    /// This check is the one that fails loudly if they ever drift, which is why
    /// it asks the strongest form — **membership of the container's unit list**
    /// rather than a property a member merely happens to have. A member that is
    /// no longer a unit of the container its box hangs off means the
    /// classification and the box disagree about the same run, and that is
    /// reported here whatever the reason.
    ///
    /// **This used to be `#568-hostile` and no longer is** — the exception is
    /// deleted rather than annotated. The old rule compared a member's
    /// `parent`, which is false by design once a run may span a
    /// `display: contents` wrapper: a member behind a broken-up wrapper is a
    /// child of the wrapper. Asking the unit list instead holds in the whole
    /// case and the split case alike, so there is nothing left to carve out.
    pub fn run_bookkeeping_violations(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (id, node) in &self.tree.nodes {
            if !node.is_anonymous_block_box {
                continue;
            }
            if node.run_members.is_empty() {
                out.push(format!("R memberless box: {id} lays out nothing"));
            }
            // The container's unit list, from the very function the
            // classification grouped this run out of.
            let mut units: Vec<usize> = Vec::new();
            if let Some(container) = node.parent {
                Self::collect_run_units(&self.tree.nodes, container, &mut units);
            }
            for &m in &node.run_members {
                if self.tree.nodes.get(m).is_none() {
                    continue; // reported as "D freed member"
                }
                if !units.contains(&m) {
                    out.push(format!(
                        "R not a unit: box {id} hangs off {:?}, but its member {m} is not a run unit of that container",
                        node.parent
                    ));
                }
            }
        }
        out
    }

    /// Both any-time validators, partitioned into the violations that are a
    /// **failure** and the ones that are a **known open defect** (#584).
    ///
    /// This exists because [`crate::RinchDocument::taffy_tree_violations`] and
    /// [`Self::dom_tree_violations`] answer *"what is wrong"* and the
    /// post-layout sweep in `resolve_layout` needs *"may I fail"*. Until #584
    /// the sweep answered the second question with an `eprintln!` — which the
    /// test harness swallows for every passing test, and every test passes when
    /// nothing asserts. A violation was reported 149 times under `--nocapture`
    /// and 0 times without it, for the whole life of the flag.
    ///
    /// **It fails on everything now.** For its first two days it could not:
    /// **#513** was open, block-level content inside an inline element was laid
    /// out by nobody, and 15 `D detached` lines were live across
    /// `-p rinch-dom -p rinch`; #513's fix left one standing — **#591**'s
    /// out-of-flow child of an inline — and #591's fix removed that. So the
    /// verdict carried a `waived` arm for those lines, and the waiver was a
    /// **shape**, not a count: a `D detached` node whose Taffy chain terminated
    /// at one of its own DOM ancestors that the inline formatting context had
    /// detached on purpose (`strandings_under_a_detached_inline`, deleted with
    /// #591). A count ("15 lines are expected") would have moved with every
    /// fixture and passed a brand-new detachment in for free; the shape survived
    /// #513's fix as a rename and retired itself on #591's — each time because
    /// `tree_check_hook_tests` carried a witness whose only job was to fail the
    /// moment the waiver waived nothing, which it did, twice. That is the
    /// pattern to reuse if a future open defect ever needs the arm back: a
    /// structural predicate, plus a fixture that fails when the defect is fixed.
    /// Never a count, never a blanket class exemption.
    ///
    /// `run_bookkeeping_violations` is deliberately absent, for the reason
    /// given at the sweep's call site: it is not any-time-true.
    pub fn tree_check_verdict(&self) -> TreeCheckVerdict {
        let mut fatal = self.taffy_tree_violations();
        // Nothing in the DOM-tree validator has a known open defect behind it —
        // it has reported 0 workspace-wide since #578 — so every line is fatal.
        fatal.extend(self.dom_tree_violations());
        TreeCheckVerdict { fatal }
    }

    /// Every way the Taffy tree disagrees with itself or loses a box (#476),
    /// as human-readable lines. Empty is the invariant.
    ///
    /// Three properties, and each one caught a real defect on the tree this
    /// landed against:
    ///
    /// - **A** no Taffy node appears in two parents' `children()` lists. Taffy's
    ///   `add_child` writes `parents[child]` and pushes **without** removing the
    ///   child from a previous parent's vector, so a clear-and-`add_child`
    ///   rebuild could leave one node in two lists — and the container would lay
    ///   out from whichever copy it happened to hold. `set_children` scrubs, so
    ///   the fix that routes both anonymous-box rebuilds through it is what
    ///   makes A hold.
    /// - **B** `taffy.parent(c)` names the parent whose list actually holds `c`.
    ///   The other half of the same inconsistency; a violation means layout and
    ///   any parent-walking consumer disagree about the tree.
    /// - **C** no **orphan**: every DOM node that generates a box and is not
    ///   claimed by an IFC has a Taffy parent. This is #476 stated directly —
    ///   an orphan is laid out by nothing and painted by nothing, and no other
    ///   assertion in the crate notices, because the `debug_assert` on
    ///   [`Self::ifc_leaf_invariant_violations`] only inspects carriers of
    ///   `InlineRoot` *that have children*.
    /// - **D** no **detached** subtree: the same node is *reachable* from a
    ///   Taffy node a compute pass actually runs on. C is the local
    ///   approximation of this and D is the property itself — see below.
    /// - **E** no **ghost box**: a node C and D exempt *because it generates no
    ///   box* carries no box. See below.
    ///
    /// # D, and why C is not enough (#589)
    ///
    /// C asks *"does this node have a Taffy parent?"*, which is **local**. The
    /// property layout actually depends on is **global**: reachable from a
    /// root Taffy computes from. A chain of perfectly valid edges hanging off
    /// nothing satisfies C at every link.
    ///
    /// That is not hypothetical. Detach a subtree at a `display: contents`
    /// node — the one node in the chain with no Taffy parent, and the one C
    /// exempts by design — and every node below it still reports a parent. A,
    /// B and `dom_tree_violations` all compare a tree against itself and the
    /// detached subtree is internally perfect. Measured (#589, #585's mutant):
    /// every validator in the crate answered `[]` while a flex column measured
    /// `0` and its whole branch was laid out nowhere.
    ///
    /// So D replaces C's final test with set membership, and C survives only
    /// as the **trivial case** of it — a node with no Taffy parent at all.
    /// Two messages rather than one because they point at different bugs (and
    /// `C orphan` is already referenced by #584), not because they are two
    /// rules.
    ///
    /// **Reachable from *which* roots.** Not the document root alone.
    /// [`Self::inline_block_measure_roots`] is a second, legitimate category:
    /// an atomic inline inside an IFC is detached from its parent's tree on
    /// purpose and `measure_inline_blocks` computes it as a Taffy **root**, so
    /// its subtree is laid out correctly while being unreachable from the
    /// document root. Seeding from it is what separates "computed by a pass of
    /// its own" from "computed by nobody".
    ///
    /// Seed from **what runs a compute pass**, never from the exemption list.
    /// Nothing computes from a `display: contents` node, so seeding one would
    /// re-admit the exact subtree #589 is about.
    ///
    /// **Only the topmost detached node in a DOM subtree is named.** A node
    /// whose DOM ancestor was already reported is suppressed: one defect
    /// strands a whole branch, and naming every node in it buries the one that
    /// needs fixing. The cost is that a second, independent detachment
    /// *inside* an already reported subtree is not separately named.
    ///
    /// Measured over `RINCH_TREE_CHECK=1 cargo test -p rinch-dom -p rinch --
    /// --nocapture` on `1a60722` — where, since #603, **C alone prints
    /// nothing at all**:
    ///
    /// | seeds | suppression | `C orphan` | `D detached` | total |
    /// |---|---|---|---|---|
    /// | root only | off | 0 | 116 | 116 |
    /// | root only | on | 0 | 77 | 77 |
    /// | root + inline-block | off | 0 | 15 | 15 |
    /// | root + inline-block | on | 0 | 15 | **15** |
    ///
    /// **Seeding is what earns its keep; suppression changed nothing when this
    /// was measured** — 15 either way — and saying so is the point. Every one of
    /// those 15 lines was a #513 shape whose stranded nodes are siblings, so
    /// there was no ancestor to suppress from. Suppression is kept for the
    /// *nested* case, which `only_the_topmost_detached_node_is_named` pins
    /// directly and which the suite last exhibited on `db9c64f` (36 lines
    /// against 26 there).
    ///
    /// **The 15 are gone**, and so is the one line #513's fix left standing —
    /// #591's out-of-flow child of an inline. The live `D` count across
    /// `-p rinch-dom -p rinch` is zero. The table above is kept as the
    /// measurement it was, at the commit it was taken on, because what it
    /// establishes — seeding matters, suppression is insurance — does not depend
    /// on which defects happened to be open.
    ///
    /// **D is not merely a #513 detector, which those 15 lines would have
    /// suggested** — and #513's fix is the proof, since D survived it with a
    /// line still to report. Disable #603's inline-level crossing trigger
    /// (leaving its heal in place) and the sweep reported **18** `D detached`
    /// lines and still **zero** `C orphan`: five of them were #597's own damage
    /// in `ifc_reattach_tests`, a defect class C is completely blind to. Two of
    /// the 15 were the reverse — created by that trigger, in the
    /// `block → inline` path #603 documented as *converging on #513*, and now
    /// clean because that convergence resolved in #513's direction.
    ///
    /// **They are ordered, not independent, and the witness is a fixture rather
    /// than a number.** Suppression applied over a wrong seed set does not
    /// merely fail to help — it **loses true positives**, because a node
    /// reported only because reachability was seeded wrongly still suppresses
    /// everything beneath it. On `db9c64f` the root-only-plus-suppression cell
    /// reported **6** `C orphan` lines where every other cell reported 16;
    /// #603 fixed all sixteen of those orphans and that demonstration
    /// evaporated within one session.
    ///
    /// Which is the general lesson, and the reason it is now built by hand in
    /// `taffy_reachability_tests::a_true_orphan_survives_under_a_legitimately_unreachable_ancestor`:
    /// **a validator's design property must not be evidenced by whichever bugs
    /// happen to exist at one commit.** That fixture puts a real orphan under an
    /// inline-block's subtree, so seeded correctly the orphan is the one line
    /// reported and seeded from the root alone it disappears behind its
    /// ancestor. It depends on no open defect and still said this after #513
    /// landed — which it has, so that is now observed rather than forecast.
    ///
    /// **Cost, measured rather than asserted.** One BFS over the
    /// `taffy.children()` reads A and B already make — kept rather than
    /// re-read — plus one membership test per DOM node. Same `O(nodes)` C
    /// already paid, and it runs only from a test or under
    /// `RINCH_TREE_CHECK=1`.
    ///
    /// The constant is **not** free, though, and the first draft of this got it
    /// badly wrong by comparing against a mutant that still paid for the BFS.
    /// On a 2203-node document, debug build, 200 iterations, best of 3 in one
    /// quiet session: the whole validator is **1.40ms** with D and E against
    /// **1.03ms** without them. Spelling the same two structures as a
    /// `HashMap` and a `HashSet` cost **2.72ms** — `SipHash` in a debug build,
    /// ~6600 probes, which is more than the rest of the check put together.
    /// Hence the `binary_search` on `parents`; do not "simplify" it back.
    /// (Re-measured after the #603 rebase at 1.41 / 1.16 on a loaded machine:
    /// the absolute figures move with load, the hashing penalty did not.)
    ///
    /// # E, and what it is for (#543)
    ///
    /// C and D exempt three kinds of node, and for two of them the exemption
    /// is *"this element generates no box"* — `display: none` and
    /// `display: contents`. That is a claim with a testable consequence, so E
    /// tests it: such a node's `layout` must be zero.
    ///
    /// A closed `DropdownMenu` that kept a `160x168` box, stayed painted and
    /// stayed clickable (#543) is exactly this shape, and it shipped with every
    /// validator here answering `[]` — because the node was exempt from the
    /// only rule that looked at it.
    ///
    /// # What C, D and E do **not** guarantee
    ///
    /// Stated because an invariant read as stronger than it is, is worse than
    /// no invariant:
    ///
    /// - **The `ifc_root` exemption is unconditional, and nothing here can see
    ///   past it.** Inline content legitimately carries a real box — the IFC
    ///   assigns it (`write_inline_positions`), not Taffy — so E cannot ask the
    ///   box question of an `ifc_root` node, and C and D exempt it outright.
    ///   Measured directly: strand a node the #589 way and D reports it; set
    ///   `ifc_root = Some(..)` on that same tree and the report goes to `[]`.
    ///   A node carrying a mark no IFC honours would therefore be invisible to
    ///   all three rules.
    ///
    ///   **This was written as "blind to #597's intermediate state", and #603
    ///   voided that.** Measured on `1a60722`, both of #597's routes — a bare
    ///   `<button>` restyled to `display: flex`, and a `<span>`/`<svg>` pair
    ///   blockified by their parent becoming flex — reach `ifc_root = None`
    ///   with a real Taffy parent on the **first** pass after the restyle.
    ///   There is no intermediate state left to be blind to. The stale mark
    ///   existed because the marking pass never ran, not because it ran and
    ///   left something behind, and #603's crossing trigger makes it run.
    ///   No other producer for the shape was found on this base — *not found*,
    ///   which is not the same as *does not exist*.
    /// - **Anonymous block boxes are not visited.** The walk is over
    ///   `node.children`, and a box lives in its container's `run_boxes`. A
    ///   detached box is reported only through its members, which are ordinary
    ///   DOM nodes and are walked.
    /// - **E says nothing about the root**, which carries the viewport box, and
    ///   nothing about nodes *inside* a `display: none` subtree — the walk
    ///   stops reporting at the subtree's top node, which is where the repair
    ///   goes.
    ///
    /// Deliberately **not** a bare `debug_assert` in `resolve_layout`, and
    /// #584 did not make it one: C is a claim about author markup as much as
    /// about the engine, and a document that legitimately holds a detached
    /// subtree should not panic. `RINCH_TREE_CHECK=1` sweeps it across a whole
    /// suite and **fails** on what it finds (debug builds only), but it fails on
    /// [`Self::tree_check_verdict`]'s list — this one plus the DOM-tree
    /// validator's, and, while #513 and then #591 were open, minus a waived arm
    /// whose history that function's docs keep. `RINCH_TREE_CHECK=warn` prints instead, which is
    /// what the flag did for its whole life before #584 — and printed to a
    /// stderr `cargo test` captures, so it showed nobody anything.
    pub fn taffy_tree_violations(&self) -> Vec<String> {
        use crate::computed_style::values::DisplayValue;
        use std::collections::HashMap;

        let mut out = Vec::new();

        // Every Taffy node we could ask about, DOM-owned or a measure leaf.
        let mut parents: Vec<taffy::NodeId> = self
            .tree
            .nodes
            .iter()
            .filter_map(|(_, n)| n.taffy_id)
            .collect();
        parents.extend(self.tree.ifc_measure_leaves.values().copied());
        parents.sort_by_key(|t| usize::from(*t));
        parents.dedup();

        let mut claims: HashMap<taffy::NodeId, Vec<taffy::NodeId>> = HashMap::new();
        // The downward edges, kept from the very same read A and B are built
        // from — so D's reachability below costs no extra `children()` call,
        // and cannot disagree with A and B about the tree. Parallel to
        // `parents` (an `Err` pushes an empty list) so it is indexed by
        // position, not hashed: see `slot_of`.
        let mut kids_of: Vec<Vec<taffy::NodeId>> = Vec::with_capacity(parents.len());
        for &p in &parents {
            match self.tree.taffy.children(p) {
                Ok(kids) => {
                    for &k in &kids {
                        claims.entry(k).or_default().push(p);
                    }
                    kids_of.push(kids);
                }
                Err(_) => kids_of.push(Vec::new()),
            }
        }

        // A Taffy node's position in `parents`, which is sorted and deduped
        // above.
        //
        // **Deliberately a binary search rather than a `HashSet`/`HashMap`.**
        // This check only ever runs in a debug build, where `SipHash` is
        // several hundred ns a probe, and D needs one membership test per DOM
        // node on top of A and B's existing work. Measured on a 2203-node
        // document, best of 3 in one quiet session: the hashed spelling took
        // the whole validator from 1.03ms to 2.72ms, this one to 1.40ms.
        // Indexing a
        // bitmap by `usize::from(id)` directly is not available — Taffy's
        // `NodeId` packs a slotmap version into the high bits, so the values
        // are around 2^32 rather than dense.
        let slot_of = |t: taffy::NodeId| -> Option<usize> {
            parents
                .binary_search_by_key(&usize::from(t), |p| usize::from(*p))
                .ok()
        };

        // D: everything a compute pass can actually reach. One BFS from the
        // roots layout computes from — the document root, plus every
        // inline-block measured standalone.
        let mut reachable = vec![false; parents.len()];
        {
            let mut frontier: Vec<taffy::NodeId> = self.inline_block_measure_roots();
            if let Some(root_taffy) = self
                .tree
                .nodes
                .get(self.tree.root_id)
                .and_then(|n| n.taffy_id)
            {
                frontier.push(root_taffy);
            }
            while let Some(t) = frontier.pop() {
                // A node outside `parents` has no DOM identity and is never
                // asked about below, so there is nothing to mark.
                let Some(slot) = slot_of(t) else { continue };
                if std::mem::replace(&mut reachable[slot], true) {
                    continue;
                }
                frontier.extend(kids_of[slot].iter().copied());
            }
        }
        // Sorted so the output is stable enough to diff between runs.
        let mut claimed: Vec<_> = claims.into_iter().collect();
        claimed.sort_by_key(|(c, _)| usize::from(*c));
        for (child, holders) in claimed {
            let dom = |t: &taffy::NodeId| self.tree.taffy_map.get(t).copied();
            if holders.len() > 1 {
                out.push(format!(
                    "A double-claim: taffy {} (dom={:?}) is in the child list of {:?}",
                    usize::from(child),
                    dom(&child),
                    holders
                        .iter()
                        .map(|p| (usize::from(*p), dom(p)))
                        .collect::<Vec<_>>()
                ));
            }
            let reported = self.tree.taffy.parent(child);
            if reported != Some(holders[0]) || holders.len() > 1 {
                out.push(format!(
                    "B parent disagreement: taffy {} is held by {:?} but parent() says {:?}",
                    usize::from(child),
                    holders.iter().map(|p| usize::from(*p)).collect::<Vec<_>>(),
                    reported.map(usize::from)
                ));
            }
        }

        // C, D and E: walk the DOM, skipping what legitimately generates no box.
        //
        // `suppressed` is set for every node under one already reported: a
        // single detachment strands a whole branch, and the topmost node is
        // the one the repair goes to.
        let mut stack = vec![(self.tree.root_id, false, false)];
        while let Some((id, hidden, suppressed)) = stack.pop() {
            let Some(node) = self.tree.nodes.get(id) else {
                continue;
            };
            let display = node.computed_style.display;
            let hidden_here = hidden || display == DisplayValue::None;
            // A **split inline** joins the boxless set (#513): CSS 2.1 §9.2.1.1
            // breaks it into fragments, rinch models the fragments' geometry
            // through anonymous block boxes and models no box for the element
            // itself, and its Taffy node legitimately holds nothing and is held
            // by nobody. So `C` and `D` have nothing to ask about it — and `E`
            // has everything to ask: this is what makes `read_layout_results`'
            // zeroing an enforced invariant instead of a coincidence, on the very
            // path (`block → inline`) where Taffy would otherwise keep serving it
            // a stale box.
            //
            // A **flowed inline element** joins it too (#591): a `<span>` that
            // an IFC lays out owns no box — its fragments are the line's —
            // and `read_layout_results` zeroes it for the same stale-detached-
            // node reason. The `ifc_root.is_some()` exemption two arms down is
            // for inline content that *does* carry a box: a direct text child
            // (stretched to the line block) and an atomic inline (measured by
            // Taffy, positioned by the IFC). `Node::is_flowed_inline_element`
            // excludes exactly those two.
            let boxless = matches!(display, DisplayValue::None | DisplayValue::Contents)
                || node.is_split_inline()
                || node.is_flowed_inline_element();
            let rect = (
                node.layout.x,
                node.layout.y,
                node.layout.width,
                node.layout.height,
            );
            let describe = |what: &str| {
                format!(
                    "{what}: dom {id} <{}> display={display:?} mode={:?} layout={rect:?}",
                    node.tag().unwrap_or("#text"),
                    node.display_mode,
                )
            };

            // `detached` drives suppression; `E` does not set it. Suppression
            // is D's mechanism — a stranded ancestor explains a stranded
            // descendant — and a `display: contents` wrapper holding a stale
            // origin explains nothing about whether its children are attached.
            // (For `display: none` the question does not arise: `hidden` skips
            // the whole subtree either way.)
            let detached = if suppressed || hidden || id == self.tree.root_id {
                false
            } else if boxless {
                // No box of its own: `none` generates nothing, `contents` is
                // flattened into an ancestor's list by design. E: an element
                // that generates no box must not be carrying one (#543).
                if rect != (0.0, 0.0, 0.0, 0.0) {
                    out.push(describe("E ghost box"));
                }
                false
            } else if node.ifc_root.is_some() {
                // Inline content is detached from Taffy on purpose — Parley
                // lays it out and the IFC root draws it. It carries a real
                // box, so E has nothing to ask here.
                false
            } else if let Some(taffy_id) = node.taffy_id {
                if self.tree.taffy.parent(taffy_id).is_none() {
                    out.push(describe("C orphan"));
                    true
                } else if slot_of(taffy_id).is_some_and(|slot| !reachable[slot]) {
                    // Every edge above it is intact and it is still laid out
                    // by nobody — the chain terminates somewhere that is not a
                    // root any compute pass runs on (#589).
                    out.push(describe("D detached"));
                    true
                } else {
                    false
                }
            } else {
                false
            };

            for &c in &node.children {
                stack.push((c, hidden_here, suppressed || detached));
            }
        }

        out
    }

    /// Whether a `display:contents` node is *transparent to the surrounding
    /// inline formatting context* — i.e. it wraps no block-level box, so every
    /// box it flattens into the ancestor belongs to that ancestor's IFC.
    ///
    /// `display:contents` is common in rsx output (`Vec<NodeHandle>` children,
    /// reactive text spans), and such a wrapper very often holds *block*
    /// content. A wrapper like that is NOT part of the ancestor's IFC: its
    /// blocks are ordinary in-flow boxes that the paint tree-walk must descend
    /// into. Marking it with `ifc_root` (which tells paint "the IFC draws this,
    /// skip it") would silently drop the whole subtree from the scene (the
    /// regression `a433811` introduced: rows kept their layout boxes but were
    /// never drawn).
    ///
    /// It does **not** require that inline content actually be found: an empty
    /// or comment-only wrapper has nothing to paint either way, and treating it
    /// as transparent keeps document order intact for the inline content around
    /// it. (A sibling predicate that *did* require it,
    /// `contents_wraps_only_inline`, was deleted in #586 once the IFC-root scan
    /// began reading units and answered its question directly.)
    ///
    /// # It is the memoized classifier now, not a per-call scan
    ///
    /// This used to walk the wrapper's subtree on every call
    /// (`scan_contents_children`, deleted with #513's split), so a chain of
    /// nested wrappers was rescanned once per ancestor. It reads
    /// [`Node::contributes_in_flow_block`] instead — the same question, answered
    /// bottom-up once per `ifc_dirty` pass by
    /// [`Self::recompute_contributes_in_flow_block`].
    ///
    /// **The two answers are not identical, and the difference is the point.**
    /// The field recurses through a `display: inline` element and the old scan
    /// did not, so a wrapper holding `<a>x<div/>y</a>` is *opaque* here and was
    /// *transparent* before. That is the correct answer — the wrapper does hold
    /// an in-flow block-level box — and it was not safe to adopt before the split
    /// existed: such a container was still an IFC root, so calling the wrapper
    /// opaque stopped the marking pass and the walk at it and dropped everything
    /// after it (`split_inline_predicate_tests`'
    /// `the_transparency_scan_is_not_switched_over_yet`, landed with the
    /// classifier for exactly this, and rewritten here).
    ///
    /// It is safe now because the container is no longer a root: the `<a>` is
    /// flattened into its units, so it holds an in-flow block-level unit, an
    /// anonymous box takes each inline run, and nothing asks this question of it
    /// at all. Measured, not argued — the fixture asserts the shape renders like
    /// its wrapper-free twin.
    ///
    /// # The *opaque* answer has no witness, and the argument is now closed
    ///
    /// The mutant that makes this return `true` unconditionally — "every wrapper
    /// is transparent" — survives `-p rinch-dom -p rinch`. It was filed with the
    /// two `break` arms in `mark_inline_descendants` and `walk_inline_children`
    /// as one cause, three arms (#615). **Those two turned out to be reachable
    /// and now have witnesses; this one is different, and the route that
    /// witnessed them cannot witness it.** Do not carry the three over as one
    /// fact any more.
    ///
    /// The induction, over the fold's own recursion. `contributes_in_flow_block`
    /// is `true` only for a node `recompute_contributes_in_flow_block` reached,
    /// i.e. an attached one. Take an attached wrapper `W` with the field set, and
    /// ask who could pass it to this function:
    ///
    /// * **a child of an IFC root `R`** (the marking pass, and the measure-leaf
    ///   decision loop) — `W`'s field is set only because some descendant reached
    ///   through boxless/`display: inline` nodes is an `InFlowBlock`, which is
    ///   exactly the chain `collect_run_units` follows, so that block **is** a
    ///   unit of `R`. Then `has_block` holds, every inline unit carries a
    ///   `run_box`, and **`R` is not a root** — which is what saves both sites.
    ///   Note the measure-leaf loop reaches its `Contents` arm over `R`'s *raw*
    ///   `children` and never calls the collector at all, so "the collector
    ///   flattens it" is not that site's reason; not being a root is.
    ///   Contradiction;
    /// * **a child of a `display: inline` element `E` the walk recursed into** —
    ///   if `W` contributes then so does `E` (the fold descends into an inline
    ///   element unconditionally), so `E` is a split inline, so `E` is never a
    ///   unit and the walk never reaches it. Contradiction;
    /// * **a child of a transparent wrapper the walk recursed into** — that
    ///   wrapper's field is `false`, so no child of it contributes.
    ///   Contradiction.
    ///
    /// And the staleness route that falsified the other two arms' version of this
    /// argument **inverts here**: a detached subtree has the field cleared to
    /// `false`, so however many blocks a wrapper in it holds, it answers
    /// *transparent*. That is the half of the argument a future reader can check.
    /// `ifc_classifier_tests`'
    /// `a_detached_contents_wrapper_answers_transparent_inside_a_detached_inline`
    /// pins it, and it pins the *answer* rather than only the field it reads: the
    /// wrapper ends up carrying `ifc_root`, and only the transparent branch marks —
    /// the opaque one `break`s first — so the mark is proof this function ran and
    /// answered `true`. (The fixture puts the wrapper inside an `<a>` for exactly
    /// that reason. Directly under the container it would never be reached, which
    /// is the induction's first case, and an earlier version of this sentence
    /// claimed a reach that was not happening.)
    ///
    /// **A fourth arm rides on the same zero** and is recorded here rather than
    /// left to be re-derived: the `else { in_flow_stays_attached = true }` branch of
    /// `setup_inline_formatting_contexts`' measure-leaf loop is entered only on an
    /// opaque answer, so it is unwitnessed by this same induction.
    ///
    /// Of 67 constructed shapes, the ones carrying a `display: contents` wrapper
    /// do reach this function — stylesheet flips, class flips,
    /// `position: absolute → static`, `display: none → block`,
    /// `inline-block → block`, `set_inner_html`, `replace_node`, nested wrappers,
    /// detached wrappers — and neither they nor the whole suite **ever** take the
    /// opaque branch. It is kept rather than collapsed to `true` because
    /// the induction assumes a fresh fold, which is guaranteed only for the
    /// marking pass — the walk may read a fold one pass old — and because the
    /// transparent answer is very much live (rsx emits a `display: contents`
    /// wrapper for every `if`/`match`/`for`). **Deleting it is a behaviour change
    /// in a direction no test can see**, which is the #585 standard for keeping
    /// it.
    fn contents_is_inline_transparent(nodes: &slab::Slab<Node>, node_id: usize) -> bool {
        nodes
            .get(node_id)
            .is_none_or(|node| !node.contributes_in_flow_block)
    }

    /// Collect, in flattened DOM order, the Taffy ids of the out-of-flow boxes
    /// a **transparent** `display:contents` wrapper flattens into its IFC root,
    /// descending through nested (necessarily transparent) wrappers.
    ///
    /// `sync_display_contents` reparented these boxes into the root's Taffy
    /// node, and — unlike the wrapper's in-flow content, which the marking pass
    /// detaches — they stay attached there: the IFC never claims an out-of-flow
    /// box (#289). The measure-leaf canonicalization must therefore count them
    /// exactly like the root's own out-of-flow children, or `set_children`
    /// silently drops them from layout.
    ///
    /// Classification is [`Node::inline_flow_role`] — display before
    /// position, always (#366): a nested `Contents` wrapper is recursed into
    /// even when it also declares `position: absolute` (boxless — no box to
    /// take out of flow), and `display:none` generates no box. A flip of that
    /// precedence here would silently drop the wrapper's out-of-flow
    /// descendants from layout. An in-flow block-level box cannot occur here:
    /// the wrapper was judged transparent, which the scan answers only when no
    /// such box exists at any depth.
    fn collect_contents_out_of_flow(
        nodes: &slab::Slab<Node>,
        wrapper_id: usize,
        out: &mut Vec<taffy::NodeId>,
    ) {
        for &child_id in &nodes[wrapper_id].children {
            let Some(child) = nodes.get(child_id) else {
                continue;
            };
            match child.inline_flow_role() {
                InlineFlowRole::Contents => {
                    Self::collect_contents_out_of_flow(nodes, child_id, out);
                }
                InlineFlowRole::OutOfFlow => {
                    if let Some(child_taffy) = child.taffy_id {
                        out.push(child_taffy);
                    }
                }
                // Comments and `display: none` have no box; inline content
                // was detached into the IFC by the marking pass — an out-of-flow
                // box beneath an inline element is a unit of its host and is
                // collected by the root loop from the units, not from here
                // (#591); an in-flow block cannot occur (see above).
                InlineFlowRole::Comment
                | InlineFlowRole::NoBox
                | InlineFlowRole::Inline
                | InlineFlowRole::InFlowBlock => {}
            }
        }
    }

    /// Restore every box a previous [`Self::mark_inline_descendants`] pass
    /// detached and that is no longer inline content (#597).
    ///
    /// # The shape of the bug
    ///
    /// The detach is correct and the *re*-attach did not exist. Inline content
    /// is laid out by Parley and drawn by its IFC root, so the marking pass
    /// removes it from the root's Taffy child list. Nothing ever put a box back
    /// when the reason for removing it went away, so an element restyled from
    /// inline-level to block-level at runtime — a bare `<button>` handed
    /// `display: flex`, a `<span>` whose parent became a flex container and
    /// blockified it — ended with **no Taffy parent at all**, its parent's child
    /// list empty, and the box it happened to have while it was inline: a stale
    /// one, or `0x0` where it had never been laid out as a box. Laid out by
    /// nobody, and drawn by nobody either once its `ifc_root` cleared.
    ///
    /// This is [`crate::node::Node::contents_spliced`]'s shape exactly (#520),
    /// one pass along: a departure the tree records so a later pass can undo it.
    ///
    /// # The gate is "will the marking pass take this back", asked of both ends
    ///
    /// [`Self::mark_inline_descendants`] detaches in two arms — inline content,
    /// and a `display: none` child whose attached Taffy node would make its
    /// root's measure structurally unreachable (#466, #487) — and both record
    /// the departure. What decides a heal is not which arm ran but whether the
    /// marking pass would detach the node **again**, and that takes **two**
    /// facts, not one:
    ///
    /// - the node's own role today. Only `Inline`, `Comment` and `NoBox` are
    ///   ever detached, so a departed node whose role is now `InFlowBlock` or
    ///   `OutOfFlow` is one no IFC will take back.
    /// - whether its owner **is** an IFC root on this pass — the `ifc_roots`
    ///   the root scan just found, which is why this runs after that scan. The
    ///   role half alone leaves a text node
    ///   stranded for ever when the **container** stops being a block container:
    ///   a text node's role is `Inline` whatever happens to it, so the gate
    ///   skipped it, while no marking pass runs for a root that is no longer
    ///   one. `<div>text</div>` restyled to `display: flex` left the text with
    ///   no Taffy parent and `ifc_root = None` — a `C orphan`, reproduced on
    ///   `main` before #592 went anywhere near it — and #592 widened the shape
    ///   to every `inline-block`, a bare `<button>` included, because an
    ///   `inline-block` is a block container now and so detaches its own text.
    ///
    /// A child that is still hidden under an owner that is still a root keeps
    /// its record and stays out.
    ///
    /// The owner half used to ask only whether the owner *could* be a root —
    /// a block container that is not `display: contents` — and that stranded a
    /// box. A block whose last rendered inline child is switched to
    /// `display: none` can still establish an IFC but no longer does, so no
    /// marking pass runs for it, and every child it had detached (the newly
    /// hidden span, and a hidden sibling detached under #487) stayed out of its
    /// Taffy list for good. The block kept the one-line box it measured as a
    /// root: 22px where the same document built hidden measures 0
    /// (`hidden_last_inline_tests`). Asking about the roots this pass actually
    /// found is exact rather than permissive. It is not free — the steady state
    /// walks to each departed node's owner and builds one set of the roots —
    /// but only on a structural pass, and on a scoped one only over its regions.
    ///
    /// **What the gate is measured to do, and what it is not.** Its first job is
    /// that the steady state costs nothing. Its second is that it does not
    /// rebuild lists it has no business rebuilding: the mutant that removes it
    /// entirely is killed across `-p rinch-dom -p rinch` by exactly one fixture,
    /// `ifc_leaf_invariant_tests`'
    /// `the_validator_fires_inside_setup_on_a_marked_root_that_kept_a_child`,
    /// whose deliberately doctored extra Taffy child an ungated heal silently
    /// removes before the leaf-invariant validator can see it.
    ///
    /// The `NoBox` arm specifically is **not** distinguished by anything in that
    /// scope — a mutant that heals hidden children survives it. That is because
    /// the placement below makes it harmless rather than because it is
    /// pointless: the heal runs before the marking pass, whose `NoBox` arm
    /// detaches such a child again on the same pass. It is written this way
    /// because the question the gate asks is "will the marking pass take this
    /// back", and for a hidden child the answer is yes; do not read the mutant's
    /// survival as licence to widen it.
    ///
    /// `Contents` is deliberately not healed here even though it is not
    /// detachable either: a boxless wrapper has no id in any effective child
    /// list, so the rebuild below could not restore it, and
    /// [`Self::sync_display_contents`]'s own departed-wrapper pass owns that
    /// case. Its record is **kept**, so the heal still fires if it later becomes
    /// a box again.
    ///
    /// # Whole-list rebuild, before the marking pass
    ///
    /// The restore rebuilds the departed node's **effective Taffy parent's**
    /// entire child list from
    /// [`Self::collect_effective_taffy_children`] — the authority for that
    /// order, and the only thing that knows about anonymous block boxes and
    /// `display: contents` flattening — rather than inserting one id at a
    /// guessed index. That list also contains the owner's *inline* children, so
    /// this must run **before** the marking pass and not after it: the pass
    /// removes them again.
    ///
    /// That ordering is measured, not argued, and the shape it shows up in is
    /// narrower than "an IFC's content would stay attached" — which is why it
    /// took a fixture of its own to see. The owner of a healed node is almost
    /// never a live IFC root: a container holding a block-level child alongside
    /// inline content is mixed content, so `create_anonymous_block_boxes` boxes
    /// the run and the container is not discovered as a root at all. The
    /// exception is `text + absolute`, which mints no anonymous box (#406) — so
    /// the container stays a root, and a root whose only attached child is
    /// out-of-flow is given a Taffy-only measure leaf (#466). Healing after the
    /// root loop rebuilds that root's list from the DOM and throws the leaf
    /// away with it, along with the detach of its text.
    /// `a_healed_out_of_flow_child_leaves_its_roots_measure_leaf_alone` is the
    /// pin; before it was written, the mutant that moves this call after the
    /// loop survived all twelve other fixtures.
    ///
    /// In the steady state it does nothing at all: a node that is still inline
    /// content fails the gate, so no list is rebuilt and no Taffy node is
    /// dirtied on a pass where nothing crossed.
    fn reattach_departed_ifc_children(
        &mut self,
        ifc_roots: &[usize],
        scope: Option<&crate::ifc_scope::IfcScope>,
    ) {
        let mut departed: Vec<usize> = Vec::new();
        let roots: std::collections::HashSet<usize> = ifc_roots.iter().copied().collect();
        let mut owners: Vec<usize> = Vec::new();
        // A node can only stop being inline content of its IFC through a
        // change to itself or to the container holding it, and either one puts
        // it in the scope of a scoped pass.
        let candidates: Vec<usize> = match scope {
            None => self.tree.nodes.iter().map(|(id, _)| id).collect(),
            Some(scope) => scope.region.iter().map(|r| r.0).collect(),
        };
        for id in candidates {
            let Some(node) = self.tree.nodes.get(id) else {
                continue;
            };
            if !node.ifc_detached {
                continue;
            }
            let owner = Self::effective_taffy_owner(&self.tree.nodes, id);
            match node.inline_flow_role() {
                // Not restorable by a list rebuild whatever the owner is: a
                // boxless wrapper has no id in any effective child list.
                InlineFlowRole::Contents => continue,
                // Still detachable — but only while something is there to
                // detach it. Keep the record and leave it alone; otherwise fall
                // through and heal.
                InlineFlowRole::Inline | InlineFlowRole::Comment | InlineFlowRole::NoBox
                    if owner.is_some_and(|o| roots.contains(&o)) =>
                {
                    continue;
                }
                _ => {}
            }
            departed.push(id);
            if let Some(owner) = owner
                && !owners.contains(&owner)
            {
                owners.push(owner);
            }
        }
        if departed.is_empty() {
            return;
        }

        for owner in owners {
            let Some(owner_taffy) = self.tree.nodes.get(owner).and_then(|n| n.taffy_id) else {
                continue;
            };
            let children = Self::collect_effective_taffy_children(&self.tree.nodes, owner);
            let _ = self.tree.taffy.set_children(owner_taffy, &children);
            // As in `sync_display_contents`: a reparented child may carry cache
            // entries from its old position, and Taffy's dirty propagation stops
            // at an already-empty ancestor. Belt and braces, and said so
            // honestly — the mutant that deletes both calls survives
            // `-p rinch-dom -p rinch`, so no fixture here is known to need them.
            for &child_taffy in &children {
                let _ = self.tree.taffy.mark_dirty(child_taffy);
            }
            let _ = self.tree.taffy.mark_dirty(owner_taffy);
        }

        // Re-derive the record from what actually happened rather than assuming
        // the rebuild reached every node: an owner with no `taffy_id`, or a node
        // whose owner's list legitimately does not name it, keeps its record and
        // is tried again next pass. Cheap accuracy rather than a fix — leaving
        // the record set forever costs a redundant rebuild per pass and nothing
        // else, and the mutant that does exactly that survives the suite.
        for id in departed {
            let still_out = self.tree.nodes[id]
                .taffy_id
                .is_none_or(|t| self.tree.taffy.parent(t).is_none());
            self.tree.nodes[id].ifc_detached = still_out;
        }
    }

    /// Detach the inline descendants of an IFC root from Taffy and mark their
    /// `ifc_root`, flattening through `display:contents` wrappers.
    ///
    /// Direct inline children are removed from the root's Taffy node (so Parley
    /// lays them out) and get `ifc_root` set. A `display:contents` wrapper
    /// generates no box, so it is transparent *when it wraps no block-level
    /// box*: it is then marked with this root's id (so IFC discovery finds this
    /// container and paint skips the wrapper in the normal tree walk) and
    /// recursed into, so its inline grandchildren — which
    /// `sync_display_contents` reparented into `root_taffy` — are detached and
    /// joined to this IFC too (issue #61). A wrapper that *does* hold a block box
    /// is left unmarked and ends the marking pass, mirroring
    /// [`Self::walk_inline_children`], which stops building the line there.
    ///
    /// The set of nodes marked here must be *exactly* the set
    /// [`Self::walk_inline_children`] flows into this IFC, so both consume
    /// [`Node::inline_flow_role`] and act on it the same way (#366): inline
    /// content joins, an out-of-flow or boxless child is walked past, an
    /// in-flow block-level child (or the opaque contents wrapper standing for
    /// one) **stops** the pass. The recursion follows the walk's rule too:
    /// down through `display: inline` elements and transparent
    /// `display:contents` wrappers, and **not** into an **atomic inline**
    /// ([`crate::node::DisplayMode::is_atomic_inline`]), which is a box the IFC
    /// only measures and places — its interior is laid out and painted by
    /// Taffy, on its own.
    fn mark_inline_descendants(
        &mut self,
        root_id: usize,
        node_id: usize,
        root_taffy: taffy::NodeId,
    ) {
        // Read the root's Taffy children once. Taffy hands back an owned `Vec`,
        // so asking per child cloned the whole list on every iteration — and the
        // recursion below multiplies that by the number of inline descendants.
        let root_taffy_children = self.tree.taffy.children(root_taffy).unwrap_or_default();
        // An anonymous block box reaches its run through `run_members`, not
        // `children` — it has none, and is not in the element tree at all
        // (#566). Every other node walks its children as before.
        let children: Vec<usize> = self.tree.nodes[node_id].ifc_children().to_vec();
        for child_id in children {
            let (role, is_inline_element, child_taffy) = match self.tree.nodes.get(child_id) {
                Some(c) => (
                    c.inline_flow_role(),
                    c.is_element() && c.display_mode == DisplayMode::Inline,
                    c.taffy_id,
                ),
                None => continue,
            };
            match role {
                InlineFlowRole::Contents => {
                    // Only a wrapper that holds *no block-level box* is part of
                    // this IFC. One that wraps blocks (an rsx `Vec<NodeHandle>`
                    // child, a component's subtree) keeps `ifc_root == None` so the
                    // paint tree-walk still descends into it — marking it would
                    // make paint skip the wrapper and every box beneath it.
                    //
                    // `break`, not `continue`: `walk_inline_children` treats such a
                    // wrapper exactly like the block it wraps and *stops* building
                    // the line at it, so anything after it never reaches Parley.
                    // Marking a later sibling would make paint skip a box that the
                    // IFC then never draws — the same silent disappearance, one
                    // sibling along. Stopping here leaves those boxes in Taffy, so
                    // they still lay out and paint (as a block would after a block).
                    if !Self::contents_is_inline_transparent(&self.tree.nodes, child_id) {
                        break;
                    }
                    if let Some(c) = self.tree.nodes.get_mut(child_id) {
                        c.ifc_root = Some(root_id);
                        if c.display_mode.is_atomic_inline() {
                            self.tree.atomic_inline_registry.insert(child_id);
                        }
                    }
                    self.mark_inline_descendants(root_id, child_id, root_taffy);
                }
                // A comment rides with the inline arm: it has no Taffy node to
                // detach and no interior to recurse into, but the mark keeps
                // IFC discovery (`build_ifc_layouts` finds roots by marked
                // children) seeing a comment-only container.
                InlineFlowRole::Comment | InlineFlowRole::Inline => {
                    // Detach from the Taffy node's *actual current parent*
                    // (`taffy.parent`), not from a test against
                    // `root_taffy_children` (#653, review of #1333). That test
                    // only ever matches a member whose own `taffy_id` is a
                    // *direct* Taffy child of the outermost root — true for a
                    // `<span>` found at this recursion's top level, never true
                    // for a text node (or anything else) nested one level
                    // deeper inside it, because *its* Taffy parent is the
                    // span's Taffy node, not the root's. Such a member used to
                    // be marked `ifc_root = Some(root_id)` with its Taffy
                    // attachment left untouched, orphaned only as a side
                    // effect of the span above it being detached — and if
                    // that attachment was ever reachable and computed again
                    // (e.g. the span briefly became its own IFC root on an
                    // intervening `display` toggle, #1333's review fixture),
                    // the stale box it picked up then survived every pass
                    // after, forever, with nothing to correct it. Asking Taffy
                    // for the actual parent instead removes the member from
                    // wherever it really is, at any nesting depth, so no
                    // lingering attachment is ever left for a later,
                    // unrelated reachability change to feed a real compute.
                    if let Some(child_taffy) = child_taffy
                        && let Some(actual_parent) = self.tree.taffy.parent(child_taffy)
                    {
                        let _ = self.tree.taffy.remove_child(actual_parent, child_taffy);
                        // Record the departure (#597). Only where the removal
                        // actually happened: a node with no current Taffy
                        // parent is already detached and is not this root's to
                        // claim.
                        if let Some(c) = self.tree.nodes.get_mut(child_id) {
                            c.ifc_detached = true;
                        }
                    }
                    if let Some(c) = self.tree.nodes.get_mut(child_id) {
                        c.ifc_root = Some(root_id);
                        // A member is no text leaf: its root lays out and
                        // paints its text. A leaf layout left from when it was
                        // a flex or grid item's text answered the caret and
                        // glyph queries (#970). Only a text node can hold one;
                        // for an element taking this arm (a span, an atomic
                        // inline) the clear is a no-op.
                        c.cached_text_parley = None;
                        if c.display_mode.is_atomic_inline() {
                            self.tree.atomic_inline_registry.insert(child_id);
                        }
                    }
                    // Everything inside a `display: inline` element belongs to this
                    // IFC too. Marking the `<a>` and stopping was enough for text —
                    // `walk_inline_children` recurses through it either way — but it
                    // left any inline-block descendant with `ifc_root == None`, so
                    // `compute_inline_block_layouts` never measured it and the
                    // `InlineBox` pushed for it read a `layout` that was still zero:
                    // an `<img>` inside a link disappeared, right `src` and computed
                    // size, 0x0 box.
                    //
                    // An `inline-block` is where the recursion stops, exactly as it
                    // does in `walk_inline_children`. Its interior is Taffy's, not
                    // this IFC's: marking it would make `read_layout_results` hold a
                    // nested inline-block at its stale x/y (it keeps the IFC's
                    // position for anything carrying an `ifc_root`), make
                    // `ifc_content_box_offset` add this root's padding to its hit
                    // rect, resolve its percentage sizes against the wrong
                    // containing block, and strip `cached_text_parley` from the text
                    // inside every `<button>`.
                    if is_inline_element {
                        self.mark_inline_descendants(root_id, child_id, root_taffy);
                    }
                }
                InlineFlowRole::NoBox => {
                    // A `display: none` child generates no box at all — which is
                    // exactly why `contributes_in_flow_block` does not count it
                    // when deciding that a contents wrapper is transparent to
                    // this IFC. But its
                    // Taffy node — reparented here by `sync_display_contents`, or
                    // a direct sibling of the wrapper — still counts toward
                    // Taffy's `has_children`, and one attached child makes this
                    // root's measure structurally unreachable (the leaf invariant
                    // on [`NodeContext::InlineRoot`]): the container collapsed to
                    // `h = 0` with its visible text laid out but never given a
                    // box. Detach it like the inline children. When it becomes
                    // visible again, the display change seeds a structural pass and
                    // `sync_display_contents`'s rebuild re-attaches it from DOM
                    // order. Not marked with `ifc_root` — it is not this IFC's
                    // content, it is nobody's content.
                    //
                    // **This comment used to claim the detach was free (#543)**
                    // — "it takes no space under any algorithm, so the only
                    // geometry this changes is the collapse itself". False.
                    // `read_layout_results` reads `taffy.layout()` per DOM node,
                    // and Taffy keeps serving a **detached** node the layout it
                    // last computed, so the box survived the detach and was
                    // written straight back onto the node every pass. A closed
                    // overlay stayed drawn, and — because `hit_test_node` reads
                    // `layout` and tests `visibility`, not `display` — stayed
                    // clickable.
                    //
                    // Nothing is zeroed *here*: the repair belongs where the box
                    // is written, beside the `Contents` branch of
                    // `read_layout_results` that already exists for exactly this
                    // hazard. Zeroing here as well would be a second authority
                    // for the same fact, and this pass does not run on every
                    // frame (only a structural change runs it) while that one does.
                    if let Some(child_taffy) = child_taffy
                        && root_taffy_children.contains(&child_taffy)
                    {
                        let _ = self.tree.taffy.remove_child(root_taffy, child_taffy);
                        // Recorded like the inline detach (#597) — *departed*,
                        // not *inline*. The heal gate reads the node's current
                        // role, not the arm that removed it, so a child that is
                        // still `display: none` is skipped and one that has
                        // become block-level is restored.
                        if let Some(c) = self.tree.nodes.get_mut(child_id) {
                            c.ifc_detached = true;
                        }
                    }
                }
                InlineFlowRole::OutOfFlow => {
                    // Left in place, unmarked, and walked past — deliberately:
                    // an absolute or fixed box keeps its Taffy node and an
                    // `ifc_root` of `None`, which is what lets it paint from
                    // its stacking root rather than from this IFC (#289), and
                    // `walk_inline_children` walks past it the same way (#406).
                    // Any future change here that starts marking block-level
                    // children must keep excluding `OutOfFlow`, or the box
                    // loses its only route to the screen.
                    //
                    // "Keeps its Taffy node" is worth something only when the
                    // node it keeps it on is computed. For a direct child of the
                    // root it is. For one nested inside an `Inline` element —
                    // reached through the recursive call above — the parent it
                    // keeps it on is the inline this very pass detached, so it
                    // is **hoisted**: `Node::hoisted_out_of_flow_to` names its
                    // host, it is a unit of that host (`collect_run_units`), the
                    // host's Taffy list names it, and the root loop collects it
                    // from the units (#591). Nothing to do here either way.
                }
                InlineFlowRole::InFlowBlock => {
                    // **Reachable, in a detached subtree — #615 is answered, and
                    // the earlier "believed unreachable" note was wrong.**
                    //
                    // The argument it rested on is sound *for an attached tree*:
                    // an IFC root's units cannot contain an `InFlowBlock`, because
                    // `has_inline && has_block` would both hold,
                    // `create_anonymous_block_boxes` would mint a run for every
                    // maximal inline stretch, every inline unit would carry a
                    // `run_box`, and the root scan skips those — so
                    // `has_non_comment_inline` is false and the container is never
                    // discovered as a root.
                    //
                    // What it missed is that the step "an inline holding a block
                    // is flattened into its container's units" reads
                    // [`Node::contributes_in_flow_block`], and
                    // `recompute_contributes_in_flow_block` walks from the
                    // document root, so **a node unreachable from it keeps
                    // `false`** — deliberately, as that function's own doc says,
                    // because `false` is the pre-#513 behaviour and the
                    // conservative direction. In a detached subtree the `<a>` of
                    // `<div><a>text<div/>tail</a></div>` is therefore *not* split,
                    // stays a unit of its container, `has_block` reads false, no
                    // anonymous box is minted, the container is an IFC root again,
                    // and this pass recurses into the `<a>` and meets the block —
                    // exactly as it did before #513. Two ordinary routes reach it:
                    // a subtree `remove_child`-ed from the document while its
                    // handles are still alive, and one built before it is
                    // appended. Both are pinned in `ifc_classifier_tests`
                    // (`a_removed_subtree_still_stops_the_mark_and_the_walk_at_a_block`,
                    // `a_never_attached_subtree_stops_them_at_a_block_too`), and
                    // the mutant that makes this arm `continue` fails both.
                    //
                    // So the arm is what makes that conservative direction *safe*:
                    // it is #366's rule doing its job in the one place that still
                    // needs it. The 67-shape sweep that found the route found no
                    // attached shape that reaches it, which is consistent with the
                    // first paragraph rather than a gap in it.
                    //
                    // An in-flow block-level child ends the marking pass —
                    // `break`, not `continue`, matching where
                    // `walk_inline_children` stops building the line (#366).
                    // Continuing used to stamp `ifc_root` on every later
                    // sibling — `<a>text<div>block</div>tail</a>` marked
                    // `tail` — so every consumer of the field (paint's
                    // `already_drawn_inline` skip, IFC invalidation routing,
                    // the inline-block special cases) believed an IFC owned a
                    // box no IFC lays out or draws. The block itself and
                    // everything after it stay in Taffy, unmarked.
                    break;
                }
            }
        }
    }

    /// The Taffy nodes a layout pass computes from **besides the document
    /// root** — every **atomic inline** that belongs to an IFC
    /// ([`DisplayMode::is_atomic_inline`]: `inline-block`, `inline-flex` and
    /// `inline-grid`).
    ///
    /// Such a node is detached from its parent's Taffy tree (the parent
    /// measures through `InlineRoot` instead) and
    /// [`Self::measure_inline_blocks`] computes it as a **root** of its own, so
    /// its whole subtree is laid out by a real compute pass while being
    /// unreachable from the document root's Taffy node.
    ///
    /// It is a function rather than a repeated `if` because two callers need
    /// the same answer and a drift between them would be invisible:
    /// `compute_inline_block_layouts` uses it to decide what to measure, and
    /// `taffy_tree_violations`' reachability rule uses it to decide which
    /// unreachable subtrees are legitimate. If they disagreed, the validator
    /// would either exempt a subtree nothing computes (blind) or report one
    /// that is computed correctly (noise) — the two failure modes the rule
    /// exists to avoid. Same list, one definition.
    ///
    /// Note what it reads: `ifc_root` and [`crate::node::DisplayMode`], the
    /// same two fields the measure pass reads. **Not**
    /// `DisplayValue::to_taffy`, which is not injective —
    /// `inline`/`block`/`inline-block` all give `taffy::Display::Block` (#592),
    /// `flex`/`inline-flex`/`contents` all give `Flex`, and `grid`/`inline-grid`
    /// both give `Grid` (#607). That aliasing is the whole of
    /// #597's second mechanism, and it is the classic way an exemption ends up
    /// wider than its author believes. This seed set cannot acquire it,
    /// because there is nothing to keep in step: one expression, two readers.
    ///
    /// It reads `NodeTree::atomic_inline_registry` rather than the slab: every
    /// node the marking pass ever gave an `ifc_root` to while it was atomic is
    /// registered, and the predicate below is applied to each entry exactly as
    /// it was applied to each slab node, so the answer is the same set at
    /// O(atomic inlines) instead of O(document).
    pub(crate) fn inline_block_measure_roots(&self) -> Vec<taffy::NodeId> {
        let mut out: Vec<(usize, taffy::NodeId)> = Vec::new();
        for &id in &self.tree.atomic_inline_registry {
            let Some(node) = self.tree.nodes.get(id) else {
                continue;
            };
            // A box outside the document is not measured (#1040): nothing
            // paints it, and attaching it again seeds a structural pass that
            // sizes it then. A detach clears the subtree's marks (#1073), but
            // this is the whole-document pass, which walks the slab and marks
            // detached subtrees again, so the registry names their atomic
            // inlines again.
            if node.ifc_root.is_some()
                && node.display_mode.is_atomic_inline()
                && let Some(taffy_id) = node.taffy_id
                && let Some(depth) = self.depth_if_connected(id)
            {
                out.push((depth, taffy_id));
            }
        }
        // **Deepest first**, and that is a correctness order rather than a
        // preference (#592). An atomic inline nested inside another is measured
        // as a root of its own, and the outer one's Parley layout sizes the
        // `InlineBox` it pushes for it from `Node::layout` — which is whatever
        // the last pass left there, `0x0` on a first layout. Slab order is
        // creation order, so `<span ib><i ib></i></span>` from one
        // `set_inner_html` measured the outer box first and gave it its padding
        // and nothing else: 14x14 where Chrome says 54x34.
        //
        // Sorting rather than recursing keeps the one-expression/two-readers
        // property below: `taffy_tree_violations` consumes this list as a *set*
        // and does not care about the order, so the two callers still cannot
        // drift.
        out.sort_by_key(|a| std::cmp::Reverse(a.0));
        out.into_iter().map(|(_, t)| t).collect()
    }

    /// The scoped pass's share of [`Self::compute_inline_block_layouts`]
    /// (`crate::ifc_scope`): size the atomic inlines the pass placed — the
    /// stop nodes of its regions that sit in an IFC — whose Taffy node is
    /// dirty. One that is clean was measured before and nothing inside it has
    /// changed since (any change inside it marks it: its subtree is a Taffy
    /// tree of its own), so its detached compute would be a cache hit.
    ///
    /// Queued first, for `remeasure_dirty_atomic_inlines` at the end of the
    /// pass: every atomic inline **at or above** a container of the scope —
    /// a change inside a container can resize every one around it, and the
    /// IFC that lines such a box up is outside this scope, so only the
    /// before/after comparison that function makes will re-measure that IFC.
    /// Measured here, a box is taken off that queue: its IFC is in this
    /// scope and its signature carries the new size.
    pub(crate) fn measure_scoped_atomic_inlines(&mut self, scope: &crate::ifc_scope::IfcScope) {
        for &c in &scope.containers {
            self.mark_atomic_inline_dirty(c);
        }
        let mut targets: Vec<(usize, usize, taffy::NodeId)> = Vec::new();
        for &(id, _, stop) in &scope.region {
            if !stop {
                continue;
            }
            let Some(node) = self.tree.nodes.get(id) else {
                continue;
            };
            let (Some(_), Some(taffy_id)) = (node.ifc_root, node.taffy_id) else {
                continue;
            };
            if !node.display_mode.is_atomic_inline() {
                continue;
            }
            // Not dirty: measured already — unless it never was as an atomic
            // inline, and so has no baseline to line it up by (#663).
            if node.atomic_baseline_known && !self.tree.taffy.dirty(taffy_id).unwrap_or(true) {
                continue;
            }
            let mut depth = 0usize;
            let mut cur = node.parent;
            while let Some(p) = cur {
                depth += 1;
                cur = self.tree.nodes.get(p).and_then(|n| n.parent);
            }
            targets.push((depth, id, taffy_id));
        }
        if targets.is_empty() {
            return;
        }
        // Innermost first, as `inline_block_measure_roots` orders them: an
        // outer box's compute reads an inner one's size.
        targets.sort_by_key(|t| std::cmp::Reverse(t.0));
        let measure: Vec<(taffy::NodeId, Option<f32>)> =
            targets.iter().map(|&(_, _, t)| (t, None)).collect();
        self.measure_inline_blocks(&measure);
        for &(_, id, _) in &targets {
            self.tree.dirty_atomic_inlines.remove(&id);
        }
    }

    /// Pre-compute layout for inline-block children that were detached from Taffy.
    ///
    /// Inline-block children are removed from their parent's Taffy tree (so the parent
    /// uses InlineRoot measurement), but they still need their own subtree computed
    /// so `walk_inline_children` can read their width/height for Parley InlineBox.
    pub(crate) fn compute_inline_block_layouts(&mut self) {
        // Anything recorded before this pass has a *stale* Taffy measure, and
        // this pass measures against Taffy's cache — so "measure everything"
        // is not enough on its own (#784). A node can land in the set before
        // `ifc_dirty` is set: the cascade records it while resolving one node's
        // style and only then reaches the `display` change that sets the flag.
        // The take is also the clear: a mark made *during* the measure below
        // would be news, and clearing the set afterwards — which is what this
        // did before #784 — would throw it away. Nothing records one today
        // (`measure_inline_blocks` reaches none of `mark_atomic_inline_dirty`'s
        // callers), so this is a statement about what the set means rather than
        // a fix.
        for id in std::mem::take(&mut self.tree.dirty_atomic_inlines) {
            if let Some(taffy_id) = self.tree.nodes.get(id).and_then(|n| n.taffy_id) {
                let _ = self.tree.taffy.mark_dirty(taffy_id);
            }
        }
        // Every box is measured again: none of the min-content sizes stands.
        self.tree.atomic_min_content.clear();
        // And the line around a box that lost one is measured again, as
        // `remeasure_dirty_atomic_inlines` has it for the boxes it measures.
        for id in std::mem::take(&mut self.tree.atomic_min_dropped) {
            if let Some(root_id) = self.tree.nodes.get(id).and_then(|n| n.ifc_root) {
                self.invalidate_ifc_of_atomic_inline(root_id);
            }
        }
        let ib_taffy_ids: Vec<(taffy::NodeId, Option<f32>)> = self
            .inline_block_measure_roots()
            .into_iter()
            .map(|t| (t, None))
            .collect();
        self.measure_inline_blocks(&ib_taffy_ids);
    }

    /// Record that `node_id`'s content or style changed, so every atomic inline
    /// that contains it — itself included — needs re-measuring (issue #661).
    ///
    /// An atomic inline is detached from its parent's Taffy child list, so the
    /// root compute cannot reach it and `run_taffy_compute` cannot notice that
    /// anything below it moved. **Three** functions size one, all of them
    /// through [`Self::measure_inline_blocks`]:
    /// [`Self::compute_inline_block_layouts`] on an `ifc_dirty` pass,
    /// [`Self::remeasure_dirty_atomic_inlines`] off this set, and
    /// [`Self::resolve_percentage_inline_blocks`] after the root compute for a
    /// box with a percentage inline size. The third is easy to forget and is
    /// the one #661 itself proposed as the hook: measured, a
    /// `display: inline-block; width: 50%` span tracks a **viewport** resize
    /// with no `ifc_dirty` pass and an **empty** `dirty_atomic_inlines` — the
    /// re-cascade a viewport change forces produces identical computed styles,
    /// so nothing marks anything. `frozen_box_remeasure_tests::
    /// a_percentage_atomic_inline_tracks_a_viewport_resize` asserts the empty
    /// set as a positive control, so it cannot start passing for this set's
    /// sake if that ever changes.
    ///
    /// **The walk does not stop at the first atomic inline it finds**, because
    /// they nest: `<span ib><i ib>…</i></span>` sizes the outer box from the
    /// inner one's `Node::layout`, so a change inside the inner box moves both.
    /// It is the same reason `inline_block_measure_roots` sorts deepest-first.
    /// `frozen_box_remeasure_tests::nested_atomic_inlines_both_regrow` is the
    /// pin on both halves — deleting the sort left the whole suite green until
    /// it existed.
    ///
    /// O(depth) per call, and called only for the two *kinds* of change that can
    /// dirty a measure without dirtying the IFC structure — a restyle that
    /// changed something, and `set_text_content` — which between them are five
    /// call sites (`dom_impl/mod.rs` twice, `style_resolution/mod.rs`,
    /// `dom_document_impl.rs`, `layout_engine.rs`). A *structural* mutation
    /// seeds a structural pass instead (`crate::ifc_scope`), which sizes what
    /// it placed and calls this itself for every container it set up (the
    /// atomic inlines around a changed container are outside its scope, and
    /// this set is how they get re-measured) — so the mutation verbs must not
    /// call it too, where it would cost an ancestor walk per `append_child` on
    /// first build and buy nothing. Only a pending **whole-document** pass
    /// (`ifc_dirty`) takes the other arm below, since that pass sizes every
    /// atomic inline anyway.
    pub(crate) fn mark_atomic_inline_dirty(&mut self, node_id: usize) {
        self.mark_atomic_inline_dirty_for(node_id, true);
    }

    /// [`Self::mark_atomic_inline_dirty`], saying whether the change can
    /// move a **size**. `sizes: false` is a change that re-shapes text
    /// without re-wrapping it — a colour — after which each box is measured
    /// again (its interior's glyphs are new) but its min-content width
    /// stands (#1476): dropping it sent a colour-only hover on a chip in a
    /// flex item through the min-content compute and a second root compute.
    pub(crate) fn mark_atomic_inline_dirty_for(&mut self, node_id: usize, sizes: bool) {
        // On an `ifc_dirty` pass there is nothing to accumulate — that pass
        // measures every atomic inline in the document — but **measuring is not
        // re-measuring** (#784). `measure_inline_blocks` asks Taffy, and an
        // atomic inline's measure is cached against a root detached from its
        // parent's child list, which the root compute never reaches; so a box
        // whose content changed in this very pass is "measured" straight out of
        // the cache, at the font it last had. That is the freeze a panel
        // crossing `none` → rendered shows: the crossing is always an
        // `ifc_dirty` pass. Marking the Taffy node instead of returning is what
        // makes the pass re-measure it.
        let ifc_dirty = self.tree.ifc_dirty;
        let mut cur = Some(node_id);
        while let Some(id) = cur {
            let Some(node) = self.tree.nodes.get(id) else {
                break;
            };
            if node.display_mode.is_atomic_inline() {
                if ifc_dirty {
                    if let Some(taffy_id) = node.taffy_id {
                        let _ = self.tree.taffy.mark_dirty(taffy_id);
                    }
                } else {
                    self.tree.dirty_atomic_inlines.insert(id);
                }
                // Its min-content size is no longer what was measured (#1476).
                if sizes
                    && !self.tree.atomic_min_content.is_empty()
                    && self.tree.atomic_min_content.remove(&id).is_some()
                {
                    self.tree.atomic_min_dropped.insert(id);
                }
            }
            cur = node.parent;
        }
    }

    /// Re-measure the atomic inlines recorded by [`Self::mark_atomic_inline_dirty`]
    /// (issue #661).
    ///
    /// Runs on a pass that computes Taffy **without** rebuilding the IFC
    /// structure, which is where the freeze lived: a `font-size` change or a
    /// `set_text_content` re-shapes the text inside an `inline-block` and leaves
    /// the box that is supposed to contain it at the size it was first measured
    /// at — measured, `225 x 20` while paint drew six lines.
    ///
    /// Must run **before** the root compute, for the same reason
    /// `compute_inline_block_layouts` does: the enclosing IFC line-breaks
    /// against `Node::layout` of the `InlineBox` it pushes for this box, so a
    /// box re-measured afterwards would be one pass behind.
    ///
    /// Returns whether anything moved, which is only used for the perf log — the
    /// invalidation the caller would otherwise do with it is done here, per
    /// affected root, rather than globally.
    pub(crate) fn remeasure_dirty_atomic_inlines(&mut self) -> bool {
        if self.tree.dirty_atomic_inlines.is_empty() {
            return false;
        }
        let dirty = std::mem::take(&mut self.tree.dirty_atomic_inlines);

        // A worklist keyed (depth, id), popped deepest first — an outer atomic
        // inline sizes its `InlineBox` from the inner one's `Node::layout`.
        // Same order, same reason, as `inline_block_measure_roots`.
        //
        // The input order this undoes is ascending node id, i.e. creation
        // order, i.e. shallowest-first for a tree built parent-first — which is
        // why `dirty_atomic_inlines` is a `BTreeSet` and must stay one. Under a
        // `HashSet` the order was per-process random and the fixture that pins
        // this line killed a sort-deleted mutant only **8** times in 25 runs —
        // it missed it the other 17. It now kills it 25 times in 25. See that
        // field's doc.
        //
        // A box no longer in the document is dropped, not measured (#1040):
        // nothing paints what it holds. A detach clears the subtree's marks
        // (#1073), so such an entry is usually skipped below for its missing
        // `ifc_root` anyway — but a whole-document pass in the same layout
        // marks a detached subtree again. Dropping the entry loses nothing: attaching
        // the box again seeds a structural pass that sizes it.
        // `a_detached_atomic_inline_is_sized_again_when_reattached` pins that
        // for a box whose content changed before the detach with no layout in
        // between, with and without a change while it was out, under a scoped
        // and a whole-document pass.
        let mut pending: std::collections::BTreeSet<(usize, usize)> = dirty
            .into_iter()
            .filter_map(|id| self.depth_if_connected(id).map(|depth| (depth, id)))
            .collect();

        // One box at a time, and each box's IFC invalidated — and every atomic
        // inline around that IFC queued — **before** the next box is measured.
        // An outer atomic inline lays its own IFC out around the inner one,
        // and that IFC's measure is cached on a Taffy node inside the outer
        // box's subtree: sized in one batch, the outer box read the measure its
        // IFC had cached against the inner box's *old* size and kept the old
        // width, and one sized earlier in the pass for another reason was never
        // sized again (both found by the scoped pass's random differential).
        let mut changed = false;
        while let Some((depth, id)) = pending.pop_last() {
            // The predicate is `inline_block_measure_roots`' — a node may have
            // been removed, or have stopped being an atomic inline, since it
            // was marked.
            let Some(node) = self.tree.nodes.get(id) else {
                continue;
            };
            let (Some(root_id), Some(taffy_id)) = (node.ifc_root, node.taffy_id) else {
                continue;
            };
            if !node.display_mode.is_atomic_inline() {
                continue;
            }
            let before = (node.layout.width, node.layout.height);
            // A box inside that was resolved against its containing block
            // (a percentage width) is lined up at the size that gave it, so
            // this box's `auto` size would depend on what its containing
            // block was before the change. Measured as `auto` first — what
            // a fresh layout has when it measures this box — and resolved
            // again after the compute, as every such box is.
            self.reset_resolved_atomic_inlines_inside(id, depth, false);
            // Taffy caches a measure per (node, available space), and the
            // previous pass measured this box at exactly the available space
            // this one will ask for — so without a mark the stale size is
            // served straight back.
            let _ = self.tree.taffy.mark_dirty(taffy_id);
            self.measure_inline_blocks(&[(taffy_id, None)]);
            let now = {
                let n = &self.tree.nodes[id];
                (n.layout.width, n.layout.height)
            };
            // A box whose min-content width was dropped contributes
            // something else to a narrower line even at the size it had
            // (review 2 of #1488): a text edit that kept the box's size left
            // the flex item around it at its old width for good, Taffy
            // serving the item's min-content answer made from the old text.
            let dropped = !self.tree.atomic_min_dropped.is_empty()
                && self.tree.atomic_min_dropped.remove(&id);
            if !dropped && (now.0 - before.0).abs() <= 0.5 && (now.1 - before.1).abs() <= 0.5 {
                continue;
            }
            // An IFC root that line-broke against the stale box has to break
            // again. `resolve_percentage_inline_blocks` does the same three
            // things for the same reason; the fourth, clearing the whole
            // `ifc_measure_cache`, is deliberately narrowed to the affected
            // roots here — this pass runs for an ordinary text edit, where
            // flushing every root's cached measure is the cost the dirty set
            // exists to avoid.
            changed = true;
            if let Some(root) = self.tree.nodes.get_mut(root_id) {
                root.text_layout = None;
            }
            self.tree.dirty_ifc_text_roots.insert(root_id);
            // The root's signature hashes the box's size, which has just moved
            // with no structural pass to sign it.
            self.tree.forget_ifc_measures_and_signature(root_id);
            if let Some(root_taffy) = self.tree.nodes.get(root_id).and_then(|n| n.taffy_id) {
                let _ = self.tree.taffy.mark_dirty(root_taffy);
            }
            // The measure may live on the root's measure leaf, which a mark on
            // the root does not reach — dirty propagates up, not down (#466).
            self.mark_ifc_measure_dirty(root_id);
            // That IFC is inside every atomic inline above the box, whose sizes
            // it feeds: those are measured again, after it.
            let mut cur = self.tree.nodes.get(id).and_then(|n| n.parent);
            while let Some(a) = cur {
                let Some(an) = self.tree.nodes.get(a) else {
                    break;
                };
                // `a` is an ancestor of a connected box, so it is connected:
                // `depth_if_connected` is asked for its depth here, not used
                // as a filter, and has no `None` to give.
                if an.display_mode.is_atomic_inline()
                    && let Some(depth) = self.depth_if_connected(a)
                {
                    pending.insert((depth, a));
                }
                cur = an.parent;
            }
        }
        changed
    }

    /// One compute of a detached atomic-inline root, with the Parley measure
    /// function these roots need (#120, #592).
    ///
    /// Split out of [`Self::measure_inline_blocks`] only so that function can run
    /// it more than once for the same node — see its doc for why a single pass
    /// cannot answer both "how wide is this box" and "lay its interior out at
    /// that width".
    fn compute_atomic_inline_root(
        tree: &mut crate::node::NodeTree,
        font_cx: &mut parley::FontContext,
        layout_cx: &mut parley::LayoutContext<Brush>,
        taffy_id: taffy::NodeId,
        avail: taffy::Size<taffy::AvailableSpace>,
    ) {
        // Split-borrow so the closure captures only `tree.nodes`, leaving
        // `tree.nodes.get_mut` free after the call returns.
        let nodes = &tree.nodes;
        let perf = &tree.perf;
        let (min_content, resolved_at, min_requests) = (
            &tree.atomic_min_content,
            &tree.keyword_inline_cb_width,
            &tree.atomic_min_requests,
        );
        // The layouts this compute's text leaves are measured with, kept for
        // paint (#904) — see `NodeTree::atomic_leaf_layouts`.
        let mut leaf_layouts: HashMap<(usize, u32), parley::layout::Layout<Brush>> = HashMap::new();
        // Each IFC root's and text leaf's line baselines, by the content
        // width it was measured at (#663): `NodeTree::atomic_line_baselines`.
        let mut baselines: Vec<(usize, f32, f32, Option<f32>, Option<f32>)> = Vec::new();
        perf.bump(crate::perf::Counter::InlineBlockComputes);
        let _ = tree.taffy.compute_layout_with_measure(
            taffy_id,
            avail,
            |inputs, _node_id, context, style| {
                // Taffy 0.14's measure returns a `LayoutOutput`; the leaf
                // algorithm (box-sizing, min/max clamps) is what Taffy 0.12's
                // `TaffyView` ran around the size this body returns, with the
                // same `0.0` calc resolver. The first line's baseline goes to
                // Taffy too (#1013): an `inline-flex` / `inline-grid` with
                // `align-items: baseline` aligns its own items by it.
                let mut first_baseline: Option<f32> = None;
                let out = taffy::compute_leaf_layout(
                    crate::out_of_flow::absolute_leaf_inputs(inputs, style),
                    style,
                    |_, _| 0.0,
                    |known_dims, avail_space| {
                        let max_width = match avail_space.width {
                            taffy::AvailableSpace::Definite(w) => Some(w),
                            taffy::AvailableSpace::MaxContent => None,
                            taffy::AvailableSpace::MinContent => Some(0.0),
                        };
                        match context {
                            Some(NodeContext::InlineRoot(root_id)) => {
                                let root_id = *root_id;
                                if let Some(est_h) = nodes[root_id].estimated_height {
                                    return taffy::Size {
                                        width: known_dims.width.unwrap_or(0.0),
                                        height: known_dims.height.unwrap_or(est_h),
                                    };
                                }
                                perf.bump(crate::perf::Counter::ShapeAtomicInline);
                                let atomic = AtomicContributions {
                                    wrap_width: max_width,
                                    min_content,
                                    resolved_at,
                                    requests: min_requests,
                                };
                                let (inline_layout, font_family_resolves) =
                                    Self::build_inline_layout(
                                        nodes,
                                        root_id,
                                        max_width,
                                        None,
                                        1.0,
                                        font_cx,
                                        layout_cx,
                                        Some(&atomic),
                                    );
                                perf.add(
                                    crate::perf::Counter::InlineFontFamilyResolves,
                                    font_family_resolves,
                                );
                                inline_layout.hang.record(perf);
                                first_baseline = first_line_baseline(&inline_layout.layout);
                                let width =
                                    known_dims.width.unwrap_or(inline_layout.measured_width());
                                let height =
                                    known_dims.height.unwrap_or(inline_layout.layout.height());
                                baselines.push((
                                    root_id,
                                    width,
                                    height,
                                    first_baseline,
                                    last_line_baseline(&inline_layout.layout),
                                ));
                                taffy::Size { width, height }
                            }
                            Some(NodeContext::Text(text)) => {
                                if text.content.is_empty() {
                                    return taffy::Size::ZERO;
                                }
                                perf.bump(crate::perf::Counter::ShapeAtomicInline);
                                // If no_wrap is set (white-space: nowrap), don't constrain width
                                let wrap_width = if text.no_wrap {
                                    None
                                } else {
                                    known_dims.width.or(max_width)
                                };
                                let leaf = shape_text_leaf(font_cx, layout_cx, text, wrap_width);
                                leaf.hang.record(perf);
                                first_baseline = leaf.first_baseline();
                                let last = if leaf.blank {
                                    None
                                } else {
                                    last_line_baseline(&leaf.layout)
                                };
                                let layout = leaf.layout;
                                let size = taffy::Size {
                                    width: known_dims.width.unwrap_or(leaf.width),
                                    height: known_dims.height.unwrap_or(leaf.height),
                                };
                                baselines.push((
                                    text.node_id,
                                    size.width,
                                    size.height,
                                    first_baseline,
                                    last,
                                ));
                                // Keyed exactly as the root compute keys its own text
                                // leaves, so `copy_cached_text_layouts` picks between
                                // them by the same rule.
                                let wrap_bits = wrap_width.map(|w| w.to_bits()).unwrap_or(u32::MAX);
                                leaf_layouts.insert((text.node_id, wrap_bits), layout);
                                size
                            }
                            Some(NodeContext::Image {
                                width,
                                height,
                                aspect,
                                ..
                            }) => {
                                // A loaded image's own ratio unless `aspect-ratio`
                                // declares one without `auto`; before it loads (or when
                                // it failed), the declared one, which is the one its
                                // `width` and `height` attributes map to (#684) unless an
                                // author rule replaced it (#1286); with neither, none.
                                let (natural, ratio) =
                                    crate::replaced::image_natural_size(*width, *height, *aspect);
                                // As the root compute measures one (#788, #1150).
                                crate::replaced::measure(
                                    natural,
                                    ratio,
                                    known_dims,
                                    style,
                                    inputs.parent_size,
                                    inputs.sizing_mode == taffy::SizingMode::InherentSize,
                                )
                            }
                            Some(NodeContext::FormControl {
                                content_width,
                                content_height,
                            }) => crate::form_control::measure(
                                *content_width,
                                *content_height,
                                known_dims,
                            ),
                            Some(NodeContext::Replaced {
                                width,
                                height,
                                ratio,
                                aspect,
                            }) => crate::replaced::measure(
                                (*width, *height),
                                aspect.used_ratio(*ratio),
                                known_dims,
                                style,
                                inputs.parent_size,
                                inputs.sizing_mode == taffy::SizingMode::InherentSize,
                            ),
                            _ => taffy::Size::ZERO,
                        }
                    },
                );
                with_first_baseline(out, &inputs, style, first_baseline)
            },
        );
        tree.atomic_leaf_layouts.extend(leaf_layouts);
        for (id, width, height, first, last) in baselines {
            let entry = tree.atomic_line_baselines.entry(id).or_default();
            entry.retain(|e| (e.0 - width).abs() > 0.01 || (e.1 - height).abs() > 0.01);
            if entry.len() >= 8 {
                entry.remove(0);
            }
            entry.push((width, height, first, last));
        }
    }

    /// Measure a set of detached atomic inlines (`inline-block`, `inline-flex`,
    /// `inline-grid`) as Taffy compute roots.
    ///
    /// `available_width` is the definite inner width of the box's containing
    /// block, where it is known. Taffy resolves a *root* node's percentage sizes
    /// against its available space — `compute_root_layout` turns `AvailableSpace`
    /// into the `parent_size` basis — so passing `Some(w)` is what lets a
    /// `width: 50%` inline-block resolve at all. `None` measures at max-content,
    /// which is right for auto and definite sizes but leaves a percentage with no
    /// basis, collapsing it to min-content (issue #120).
    ///
    /// # Why a block-mapped root needs more than one pass (#592)
    ///
    /// Since #592 an `inline-block` maps to `taffy::Display::Block`, and Taffy's
    /// root path treats a block root differently from a flex or grid one in two
    /// ways that both fight what an atomic inline is:
    ///
    /// - **A definite available space means "stretch".**
    ///   `compute_root_layout`'s block branch folds `available_space` into
    ///   `known_dimensions` (`available_space_based_size`), so `Definite(700)` —
    ///   which is only there to give percentages a basis — also makes an
    ///   auto-width box 700 wide. An atomic inline is shrink-to-fit
    ///   (CSS 2.1 §10.3.5). Measured: `min-width: 50%` of a 700px cell with the
    ///   text `"hi"` came out **700** where it must be 350.
    ///   The repair is to **pin** a width as `size.width` for the definite
    ///   pass, because `clamped_style_size` is `.or`'d ahead of
    ///   `available_space_based_size` while percentage `min-`/`max-width`
    ///   still resolve against the containing block. Since #658 that width is
    ///   the box's shrink-to-fit width, which
    ///   [`Self::resolve_root_width_keyword`] resolves for `auto` as for
    ///   `fit-content` (it used to be a separate "pass A" pinning the
    ///   max-content width, never capped); an `auto` box that fits is
    ///   measured under max-content space instead. Only for an auto-width
    ///   box: an explicit `width` is
    ///   already the answer that branch reaches for.
    /// - **A leaf's min/max clamp is applied *after* its measure ran.** So a
    ///   `max-width` that bites leaves the box narrow with an interior laid out
    ///   wide — measured: `max-width: 175px` over a sentence gave a 175px box
    ///   **one line tall**, the text unwrapped at its 400px max-content width.
    ///   The repair is a second compute pinned to the width the box actually
    ///   has. Flex and grid roots do not need it: their own algorithms
    ///   re-measure an item against the resolved container size, which is what
    ///   made this case work while `inline-block` mapped to `Display::Flex`.
    ///
    /// Both extra passes are **gated on the style that can trigger them**, so the
    /// overwhelmingly common atomic inline — a `<button>` with no `width` and no
    /// `max-width`, measured at max-content — still costs exactly one compute.
    /// The pin is written into the Taffy style and taken out again before this
    /// returns, so nothing outside this function ever sees it; the node's
    /// computed style, which is what the next pass re-derives from, is untouched.
    fn measure_inline_blocks(&mut self, targets: &[(taffy::NodeId, Option<f32>)]) {
        for &(taffy_id, available_width) in targets {
            // Measured as `auto` (no containing-block width): a keyword box's
            // cached size no longer stands (#691).
            if available_width.is_none()
                && !self.tree.keyword_inline_cb_width.is_empty()
                && let Some(&nid) = self.tree.taffy_map.get(&taffy_id)
            {
                self.tree.keyword_inline_cb_width.remove(&nid);
            }
            let definite = |w: f32| taffy::Size {
                width: taffy::AvailableSpace::Definite(w),
                height: taffy::AvailableSpace::MaxContent,
            };
            let max_content = taffy::Size {
                width: taffy::AvailableSpace::MaxContent,
                height: taffy::AvailableSpace::MaxContent,
            };

            let Ok(mut original) = self.tree.taffy.style(taffy_id).cloned() else {
                continue;
            };
            if available_width.is_none()
                && let Some(&nid) = self.tree.taffy_map.get(&taffy_id)
            {
                if original.size.width.is_auto() && self.subtree_uses_percentage(taffy_id) {
                    self.tree.auto_inline_percent_content.insert(nid);
                } else if !self.tree.auto_inline_percent_content.is_empty() {
                    self.tree.auto_inline_percent_content.remove(&nid);
                }
            }
            // A Taffy root ignores a sizing keyword on its own `size` (#691),
            // so a keyword width is resolved here and handed to the passes
            // below as the length (or `auto`) it means; `declared` is what
            // goes back afterwards. No clone for a box without one.
            let keyword_width =
                self.resolve_root_width_keyword(taffy_id, &original, available_width);
            let declared = keyword_width.map(|w| {
                let declared = original.clone();
                original.size.width = w;
                let _ = self.tree.taffy.set_style(taffy_id, original.clone());
                declared
            });
            let block_root = original.display == taffy::Display::Block;
            let clampable = !original.max_size.width.is_auto();
            let mut pinned = keyword_width.is_some();

            // There used to be a pass A here (#592): an `auto` width given a
            // containing-block width was measured at max-content and that
            // width pinned, so the definite pass below had a basis for a
            // percentage without stretching the box. An `auto` width given a
            // containing-block width is now resolved to its shrink-to-fit
            // length above (#658), which is that pin capped at the containing
            // block — unrounded, for the reason that pass gave: a 580.37px
            // max-content line pinned at 580 re-broke into two lines.

            // Pass B — the measure the caller asked for. Never narrower than
            // a width resolved above: one allowed up to a pixel past its
            // containing block (see `resolve_root_width_keyword`) re-broke
            // its last word onto a second line under the narrower space.
            let resolved_width = keyword_width
                .filter(|w| w.tag() == taffy::CompactLength::LENGTH_TAG)
                .map(|w| w.value());
            let avail = match (available_width, resolved_width) {
                (Some(w), Some(r)) => definite(w.max(r)),
                // An `auto` box that fits (see `resolve_root_width_keyword`).
                (Some(_), None) if original.size.width.is_auto() => max_content,
                (Some(w), None) => definite(w),
                (None, _) => max_content,
            };
            Self::compute_atomic_inline_root(
                &mut self.tree,
                &mut self.font_cx,
                &mut self.layout_cx,
                taffy_id,
                avail,
            );

            // Pass C — lay the interior out at the width the box actually has.
            //
            // Only a `max-width` can leave the box narrower than the width its
            // interior was measured at, so that is the gate. Widening it to
            // `clampable || pinned` was tried and **dropped**: it was written
            // for a symptom the unrounded pin above turns out to be the real
            // cure for, and with that pin in place the wider gate is killed by
            // nothing across `-p rinch-dom -p rinch` — a whole extra compute per
            // percentage-sized atomic inline, buying nothing any test can see.
            if block_root && clampable {
                // Unrounded: a rounded-down width is one the content does
                // not fit in (a 580.37px line pinned at 580 re-broke).
                let used = self.tree.taffy.unrounded_layout(taffy_id).size.width;
                let mut style = original.clone();
                style.size.width = taffy::Dimension::length(used);
                let _ = self.tree.taffy.set_style(taffy_id, style);
                pinned = true;
                Self::compute_atomic_inline_root(
                    &mut self.tree,
                    &mut self.font_cx,
                    &mut self.layout_cx,
                    taffy_id,
                    definite(used),
                );
            }

            // Read the computed layout back into the node, before the pin comes
            // out: `set_style` only invalidates Taffy's *cache*, but reading
            // first keeps the order obvious.
            if let Ok(taffy_layout) = self.tree.taffy.layout(taffy_id) {
                let layout_size = taffy_layout.size;
                let node_id = self.tree.taffy_map.get(&taffy_id).copied();
                let baseline = node_id.and_then(|nid| atomic_inline_baseline(&self.tree, nid));
                if let Some(nid) = node_id
                    && let Some(node) = self.tree.nodes.get_mut(nid)
                {
                    node.layout.width = layout_size.width;
                    node.layout.height = layout_size.height;
                    node.atomic_baseline = baseline;
                    node.atomic_baseline_known = true;
                }
            }

            if pinned {
                let _ = self
                    .tree
                    .taffy
                    .set_style(taffy_id, declared.unwrap_or(original));
            }
        }
    }

    /// The width a detached atomic-inline root takes for a **sizing keyword**
    /// on its own `width` (#691), as a length — or `auto` where the keyword
    /// needs a containing-block width this pass was not given. `None` when the
    /// width is not a keyword.
    ///
    /// Taffy lays the keywords out on a box it computes as a child, but a
    /// *root*'s own size keyword is resolved to nothing, so the root falls
    /// back to `auto` under whatever available space it is handed — and every
    /// atomic inline is a root here. So:
    ///
    /// - `max-content` and `min-content` are a compute of the box with an
    ///   `auto` width under `AvailableSpace::MaxContent` / `MinContent`. With no
    ///   containing-block width, `max-content` needs no compute at all: `auto`
    ///   under max-content space is exactly what pass B does with it.
    /// - `fit-content` is `min(max-content, max(min-content, stretch))` and
    ///   `stretch` is the containing block's inner width less the box's
    ///   margins, so both need `available_width`. Without one (the pass before
    ///   the root compute) they lay out as `auto`, and
    ///   [`Self::resolve_percentage_inline_blocks`] measures them again once
    ///   the containing block has a width.
    ///
    /// Widths are the unrounded border-box widths: a rounded-down width is one
    /// the content does not fit in (a 580.37px line pinned at 580 re-broke).
    /// (rinch hands Taffy no `box-sizing`, so every box is border-box to it.)
    fn resolve_root_width_keyword(
        &mut self,
        taffy_id: taffy::NodeId,
        declared: &taffy::Style,
        available_width: Option<f32>,
    ) -> Option<taffy::Dimension> {
        use taffy::CompactLength as C;
        use taffy::ResolveOrZero;
        let width = declared.size.width;
        // An `auto` width is shrink-to-fit on an atomic inline (CSS 2.1
        // §10.3.9), which is `fit-content` (css-sizing-3 §3.1) — but only
        // once there is a containing-block width to fit (#658). Without one
        // it is measured as Taffy measures `auto` under max-content space.
        let auto = width.is_auto();
        if auto && available_width.is_none() {
            return None;
        }
        if !auto && !width.is_sizing_keyword() {
            return None;
        }
        let measure = |this: &mut Self, space: taffy::AvailableSpace| -> f32 {
            let mut probe = declared.clone();
            probe.size.width = taffy::Dimension::auto();
            let _ = this.tree.taffy.set_style(taffy_id, probe);
            Self::compute_atomic_inline_root(
                &mut this.tree,
                &mut this.font_cx,
                &mut this.layout_cx,
                taffy_id,
                taffy::Size {
                    width: space,
                    height: taffy::AvailableSpace::MaxContent,
                },
            );
            this.tree.taffy.unrounded_layout(taffy_id).size.width
        };
        // `min(max-content, max(min-content, stretch))`. The max-content
        // width is measured first: when it fits, it is the answer and the
        // min-content compute is not needed.
        //
        // "Fits" and the result both allow one pixel: an IFC lines an atomic
        // inline up at its *rounded* `Node::layout` size, so a box whose
        // only content is another one has a min-content width (and so a
        // containing block) up to a pixel narrower than that box's unrounded
        // max-content width — capped there, the inner box wrapped its last
        // word onto a second line (an `inline-block` around an
        // `inline-block`, 544.0 against 544.3).
        //
        // The max-content answer is handed back a hundredth of a pixel
        // wider: laid out again at exactly its own max-content width, a
        // line can break (measured: `"x y z"` in a padded `inline-block`,
        // 32.64 wide, came out two lines tall), and a width that has room
        // for its content must not wrap it. It rounds to the same pixel.
        //
        // The min-content width of an `auto` box is kept
        // (`NodeTree::atomic_min_content`, #1476): an inline formatting
        // context that lines the box up needs it too, and it stands until
        // something inside the box changes. Its own min-content compute can
        // meet a box inside it whose min-content size is not known yet; those
        // are measured and the compute run again.
        let this_max = std::cell::Cell::new(f32::INFINITY);
        let node_id = self.tree.taffy_map.get(&taffy_id).copied();
        let fit_content = |this: &mut Self, stretch: f32| -> f32 {
            // From before the max-content compute: a flex item inside the
            // box is asked for its min-content size by that compute too, and
            // Taffy keeps the answer — made with a stand-in — for the
            // min-content compute below, which then asks nothing.
            let mut asked = this.tree.atomic_min_requests.borrow().len();
            // A box still resolved (it has not been measured as `auto`
            // since, so nothing inside it changed) whose min-content width
            // is known needs neither probe: its max-content width is the
            // one the last resolution measured. This is a capped chip whose
            // container was resized — one compute a pixel, not two.
            let remembered = node_id.filter(|n| {
                auto && this.tree.keyword_inline_cb_width.contains_key(n)
                    && this.tree.atomic_min_content.contains_key(n)
            });
            let max = match remembered {
                Some(n) => this.tree.nodes[n].natural_max_width,
                None => {
                    let max = measure(this, taffy::AvailableSpace::MaxContent);
                    if auto && let Some(n) = node_id {
                        this.tree.nodes[n].natural_max_width = max;
                    }
                    max
                }
            };
            this_max.set(max + 0.01);
            if atomic_inline_fits(max, stretch) {
                return max + 0.01;
            }
            let known = node_id
                .filter(|_| auto)
                .and_then(|n| this.tree.atomic_min_content.get(&n))
                .copied();
            let min = known.unwrap_or_else(|| {
                let mut min = measure(this, taffy::AvailableSpace::MinContent);
                for _ in 0..8 {
                    let inner = this.tree.atomic_min_requests.borrow_mut().split_off(asked);
                    if !inner.iter().any(|&r| this.wants_min_content(r)) {
                        break;
                    }
                    this.measure_atomic_min_contents(inner, false);
                    asked = this.tree.atomic_min_requests.borrow().len();
                    min = measure(this, taffy::AvailableSpace::MinContent);
                }
                if auto && let Some(n) = node_id {
                    this.tree.atomic_min_content.insert(n, min);
                }
                min
            });
            atomic_inline_capped_width(min, max, stretch).unwrap_or(max + 0.01)
        };
        let stretch = available_width.map(|cb| {
            let margin = declared.margin.resolve_or_zero(Some(cb), |_, _| 0.0);
            (cb - margin.left - margin.right).max(0.0)
        });
        let resolved = match width.tag() {
            // An `auto` box that fits is measured as `auto`, under
            // max-content space — exactly what a measure with no
            // containing-block width does — rather than pinned at that
            // width: a pinned line can break where the unpinned one did not
            // (an inline formatting context holding an absolute box's
            // placeholder does, pre-existing), and then a box resolved here
            // and one a whole-document pass left alone came out at two
            // heights. Not when a percentage in it, or its own, needs that
            // width as its basis (#662): that box is pinned.
            _ if auto => stretch.and_then(|stretch| {
                let w = fit_content(self, stretch);
                let max = this_max.get();
                let percent_content = self
                    .tree
                    .taffy_map
                    .get(&taffy_id)
                    .is_some_and(|n| self.tree.auto_inline_percent_content.contains(n));
                // Its own percentage `min-`/`max-width` needs the basis too.
                let own_percent = [declared.min_size.width, declared.max_size.width]
                    .iter()
                    .any(|d| d.into_raw().tag() == C::PERCENT_TAG);
                (w < max || percent_content || own_percent).then_some(w)
            }),
            C::MAX_CONTENT_TAG if available_width.is_none() => None,
            C::MAX_CONTENT_TAG => Some(measure(self, taffy::AvailableSpace::MaxContent)),
            C::MIN_CONTENT_TAG => Some(measure(self, taffy::AvailableSpace::MinContent)),
            C::FIT_CONTENT_KEYWORD_TAG => stretch.map(|stretch| fit_content(self, stretch)),
            C::STRETCH_TAG => stretch,
            // `fit-content(<length-percentage>)` — rinch never produces one.
            _ => None,
        };
        Some(resolved.map_or(taffy::Dimension::auto(), taffy::Dimension::length))
    }

    /// Whether a box in `root`'s own Taffy subtree (not `root` itself, and not
    /// inside a nested atomic inline, which is a root of its own) has a
    /// percentage that resolves against its containing block's width (#662):
    /// an inline size, a flex basis, a padding or margin on any side, a gap,
    /// or a horizontal inset. Asked when an `auto`-width atomic inline is
    /// measured with no containing-block width, so it costs one walk per
    /// measure, not one per layout pass.
    fn subtree_uses_percentage(&self, root: taffy::NodeId) -> bool {
        use taffy::CompactLength as C;
        let pct = |c: taffy::CompactLength| c.tag() == C::PERCENT_TAG;
        let mut stack = self.tree.taffy.children(root).unwrap_or_default();
        while let Some(n) = stack.pop() {
            let Ok(s) = self.tree.taffy.style(n) else {
                continue;
            };
            if pct(s.size.width.into_raw())
                || pct(s.min_size.width.into_raw())
                || pct(s.max_size.width.into_raw())
                || pct(s.flex_basis.into_raw())
                || pct(s.padding.left.into_raw())
                || pct(s.padding.right.into_raw())
                || pct(s.padding.top.into_raw())
                || pct(s.padding.bottom.into_raw())
                || pct(s.margin.left.into_raw())
                || pct(s.margin.right.into_raw())
                || pct(s.margin.top.into_raw())
                || pct(s.margin.bottom.into_raw())
                || pct(s.gap.width.into_raw())
                || pct(s.inset.left.into_raw())
                || pct(s.inset.right.into_raw())
            {
                return true;
            }
            if let Ok(children) = self.tree.taffy.children(n) {
                stack.extend(children);
            }
        }
        false
    }

    /// Whether any of this style's inline-axis sizes is a percentage.
    fn has_percentage_inline_size(style: &crate::computed_style::ComputedStyle) -> bool {
        use crate::computed_style::DimensionValue::Percent;
        matches!(style.width, Percent(_))
            || matches!(style.min_width, Percent(_))
            || matches!(style.max_width, Percent(_))
    }

    /// Whether any of this style's inline-axis sizes needs a containing-block
    /// width to resolve against: a percentage, a `fit-content` or `stretch`
    /// width (#691 — see [`Self::resolve_root_width_keyword`]), or an `auto`
    /// one, which is shrink-to-fit and so capped at the containing block
    /// (#658).
    fn needs_containing_block_width(style: &crate::computed_style::ComputedStyle) -> bool {
        use crate::computed_style::DimensionValue::{Auto, Intrinsic, Percent};
        use crate::computed_style::IntrinsicSize::{FitContent, Stretch};
        matches!(
            style.width,
            Auto | Percent(_) | Intrinsic(FitContent | Stretch)
        ) || matches!(style.min_width, Percent(_))
            || matches!(style.max_width, Percent(_))
    }

    /// Size the atomic inlines whose width needs their containing block's —
    /// a percentage (#120), a `fit-content`/`stretch` keyword (#691), or
    /// `auto`, which is shrink-to-fit (#658) — now that the containing block
    /// has a computed width.
    ///
    /// `compute_inline_block_layouts` runs *before* the root Taffy compute, so
    /// such a width has nothing to resolve against (a percentage collapses to
    /// min-content, `auto` comes out at max-content). This runs *after* that
    /// compute, when the containing block's width is real, and re-measures
    /// against it.
    ///
    /// Returns `true` if any atomic inline changed size — the caller must then
    /// re-run the root compute so the enclosing IFCs line-break against the
    /// corrected boxes. **It looks at every `auto`-width atomic inline on
    /// every pass**: one measured as `auto` that fits its block (the
    /// overwhelmingly common case — a button, a badge) costs a
    /// containing-block lookup and one comparison, and is not measured.
    ///
    /// Only the *inline* axis is corrected. A percentage height on an inline-block
    /// resolves against a containing-block height that is itself usually content-
    /// derived, so there is no non-circular basis to feed back here.
    ///
    /// Last, it measures the min-content size of every `auto` box a measure
    /// asked for since the last pass (`NodeTree::atomic_min_requests`,
    /// #1476 — see [`AtomicContributions`]) and says so in
    /// `NodeTree::atomic_contributions_changed`: the caller then runs this
    /// pass once more after the compute, because a container sized from
    /// that box can come out narrower than the box, which is only then too
    /// wide for it.
    pub(crate) fn resolve_percentage_inline_blocks(&mut self) -> bool {
        // The boxes to resolve, keyed (depth, id): popped outermost first,
        // one at a time (see the two phases below).
        let mut pending: std::collections::BTreeSet<(usize, usize)> =
            std::collections::BTreeSet::new();

        // The registry, not the slab (layout audit F8/F12): this runs after
        // every root compute, and only an atomic inline in an IFC qualifies.
        // Entries that no longer qualify are dropped here — a node only gains
        // an `ifc_root` from the marking pass, which registers it again.
        {
            let nodes = &self.tree.nodes;
            self.tree.atomic_inline_registry.retain(|&id| {
                nodes
                    .get(id)
                    .is_some_and(|n| n.ifc_root.is_some() && n.display_mode.is_atomic_inline())
            });
        }
        // Whether any percentage-sized atomic inline is in the registry.
        let mut any_percent = false;
        let percent_content = !self.tree.auto_inline_percent_content.is_empty();
        let any_keyword = !self.tree.keyword_inline_cb_width.is_empty();
        for &id in &self.tree.atomic_inline_registry {
            let Some(node) = self.tree.nodes.get(id) else {
                continue;
            };
            let (Some(root_id), Some(_)) = (node.ifc_root, node.taffy_id) else {
                continue;
            };
            let style = &node.computed_style;
            if !node.display_mode.is_atomic_inline() || !Self::needs_containing_block_width(style) {
                continue;
            }
            let percent = Self::has_percentage_inline_size(style);
            any_percent |= percent;
            let Some(inner_width) = self.containing_block_inner_width(root_id) else {
                continue;
            };
            // An `auto` width measured as `auto` since it was last resolved
            // (no `keyword_inline_cb_width` entry) is at its max-content
            // width, and one that fits is already its shrink-to-fit width
            // (#658): nothing to measure, and nothing to record — a narrower
            // containing block later finds it too wide here and resolves it
            // then. Not when a percentage inside the box needs the width it
            // has as its basis (#662). This is the path every ordinary
            // atomic inline (a `<button>`, a badge) takes on every pass, so
            // it is asked first and as cheaply as it can be: the box's own
            // margins from its computed style when they are lengths.
            if !percent
                && matches!(style.width, crate::computed_style::DimensionValue::Auto)
                && !(any_keyword && self.tree.keyword_inline_cb_width.contains_key(&id))
                && !(percent_content && self.tree.auto_inline_percent_content.contains(&id))
            {
                use crate::computed_style::LengthPercentageAutoValue as M;
                let margin = |m: M| match m {
                    M::Auto => Some(0.0),
                    M::Length(px) => Some(px),
                    _ => None,
                };
                let margins = match (margin(style.margin_left), margin(style.margin_right)) {
                    (Some(l), Some(r)) => l + r,
                    _ => {
                        use taffy::ResolveOrZero;
                        let Ok(s) = self.tree.taffy.style(node.taffy_id.unwrap()) else {
                            continue;
                        };
                        let m = s.margin.resolve_or_zero(Some(inner_width), |_, _| 0.0);
                        m.left + m.right
                    }
                };
                if node.layout.width <= inner_width - margins + 0.5 {
                    continue;
                }
            }
            // A box that needs the width only for a `fit-content`/`stretch`
            // keyword (#691) or an `auto` width (#658) is sized from that
            // width and its own content alone. Its content changing
            // re-measures it as `auto` first, which drops this entry, so an
            // entry at the same width means nothing moved: skip the two or
            // three computes. (A percentage size is left re-measured every
            // pass, as before.)
            if !percent
                && any_keyword
                && self
                    .tree
                    .keyword_inline_cb_width
                    .get(&id)
                    .is_some_and(|&w| (w - inner_width).abs() < 0.01)
            {
                continue;
            }
            // Asked last: it walks the ancestors. A box outside the document
            // is not measured (#1040).
            if let Some(depth) = self.depth_if_connected(id) {
                pending.insert((depth, id));
            }
        }
        // A fitting box that holds a box sized against its containing block
        // — one recorded in `keyword_inline_cb_width`, or one with a
        // percentage size — is resolved after all. Measured as `auto`, it
        // was lined up with that box at the size it was resolved to, not at
        // its natural contribution, so its max-content width is not its
        // natural one and the fit test proves nothing (the box inside is
        // reset first, see phase 1). Without this a capped box kept the
        // `inline-flex` around it as narrow as the cap had made it, where a
        // whole-document pass widened both again. Built only when there can
        // be such a box: usually there is none, and it costs a walk up from
        // each.
        if any_percent || any_keyword {
            let mut holds_resolved: std::collections::HashSet<usize> =
                std::collections::HashSet::new();
            {
                let nodes = &self.tree.nodes;
                let inner = self.tree.keyword_inline_cb_width.keys().copied().chain(
                    self.tree
                        .atomic_inline_registry
                        .iter()
                        .copied()
                        .filter(|&id| {
                            any_percent
                                && nodes.get(id).is_some_and(|n| {
                                    Self::has_percentage_inline_size(&n.computed_style)
                                })
                        }),
                );
                for id in inner {
                    let mut cur = nodes.get(id).and_then(|n| n.parent);
                    while let Some(a) = cur {
                        let Some(an) = nodes.get(a) else {
                            break;
                        };
                        if an.display_mode.is_atomic_inline() && !holds_resolved.insert(a) {
                            break;
                        }
                        cur = an.parent;
                    }
                }
            }
            for id in holds_resolved {
                // The boxes the fit test can have left alone: an `auto`
                // width with no entry of its own. (One already queued is
                // queued once.)
                let Some(n) = self.tree.nodes.get(id) else {
                    continue;
                };
                if n.ifc_root.is_none()
                    || n.taffy_id.is_none()
                    || !matches!(
                        n.computed_style.width,
                        crate::computed_style::DimensionValue::Auto
                    )
                    || Self::has_percentage_inline_size(&n.computed_style)
                    || self.tree.keyword_inline_cb_width.contains_key(&id)
                {
                    continue;
                }
                if let Some(depth) = self.depth_if_connected(id) {
                    pending.insert((depth, id));
                }
            }
        }

        // Two phases.
        //
        // 1. Resolve the boxes against their containing blocks **outermost
        //    first**: a box's containing block is laid out inside every
        //    atomic inline around it, so its width is only known once those
        //    are resolved. Before an outer box is resolved, every box inside
        //    it that is resolved here is measured again as `auto` (no
        //    containing-block width), so the outer box's min- and max-content
        //    probes see their natural contributions — what a fresh layout,
        //    which measures every atomic inline that way before the root
        //    compute, shows them — and not whatever an earlier pass resolved
        //    them to. Without that the answer depended on history, and the
        //    scoped and whole-document passes disagreed (#1327).
        // 2. Lay the boxes **around** a box that changed size out again,
        //    innermost first, at the width each already has (see below).
        let mut around: std::collections::BTreeSet<(usize, usize)> =
            std::collections::BTreeSet::new();
        let mut changed = false;
        // A measure that lined a box up at a stand-in size (#1476, see the
        // end of this function) left that answer in Taffy's cache and the
        // IFC's own. A box resolved below is sized by computes that would be
        // served it — the min-content width of a box around the asked one
        // came out as wide as the stand-in — so those answers go first; the
        // measures that replace them ask again, and are answered there.
        let asked: Vec<usize> = self.tree.atomic_min_requests.borrow().clone();
        for id in asked {
            if self.tree.atomic_min_content.contains_key(&id) {
                continue;
            }
            let Some(root_id) = self.tree.nodes.get(id).and_then(|n| n.ifc_root) else {
                continue;
            };
            if let Some(root_taffy) = self.tree.nodes.get(root_id).and_then(|n| n.taffy_id) {
                let _ = self.tree.taffy.mark_dirty(root_taffy);
            }
            self.mark_ifc_measure_dirty(root_id);
            self.tree.forget_ifc_measures(root_id);
        }
        while let Some((depth, id)) = pending.pop_first() {
            let Some(node) = self.tree.nodes.get(id) else {
                continue;
            };
            let (Some(root_id), Some(taffy_id)) = (node.ifc_root, node.taffy_id) else {
                continue;
            };
            if !node.display_mode.is_atomic_inline() {
                continue;
            }
            let keyword_only = !Self::has_percentage_inline_size(&node.computed_style);
            for (inner_depth, inner) in self.reset_resolved_atomic_inlines_inside(id, depth, true) {
                pending.insert((inner_depth, inner));
            }
            let Some(available) = self.containing_block_inner_width(root_id) else {
                continue;
            };
            let before = {
                let n = &self.tree.nodes[id];
                (n.layout.width, n.layout.height)
            };
            // Taffy caches per (node, available space); the first pass
            // measured these under MaxContent, so mark them dirty to force a
            // real re-measure.
            let _ = self.tree.taffy.mark_dirty(taffy_id);
            // Not resolved since it was last measured as `auto`: the size
            // it has is its natural one (#1476, `Node::natural_inline_size`).
            if keyword_only && !self.tree.keyword_inline_cb_width.contains_key(&id) {
                self.tree.nodes[id].natural_inline_size = before;
            }
            self.measure_inline_blocks(&[(taffy_id, Some(available))]);
            if keyword_only {
                self.tree.keyword_inline_cb_width.insert(id, available);
            }
            if self.atomic_inline_resized(id, root_id, before) {
                changed = true;
                self.queue_atomic_inlines_around(id, &mut around);
            }
        }
        // Phase 2. **At the width each already has**, not sized again: the
        // box inside was sized against a containing block inside them, so
        // its new width is a cyclic contribution, which CSS takes as `auto` —
        // what they were sized with (css-sizing-3 §5.2.1). Sized again, an
        // `inline-block` around a `width: 50%` child shrank to half, and
        // Chrome's does not (#662). Before #658 they were not laid out again
        // at all, and kept the height they had with the box's old size — an
        // `inline-flex` around a capped box stayed one line tall on a fresh
        // layout while a scoped pass, re-measuring it for an unrelated
        // change, got it right.
        while let Some((_, id)) = around.pop_last() {
            let Some(node) = self.tree.nodes.get(id) else {
                continue;
            };
            let (Some(root_id), Some(taffy_id)) = (node.ifc_root, node.taffy_id) else {
                continue;
            };
            let before = (node.layout.width, node.layout.height);
            let _ = self.tree.taffy.mark_dirty(taffy_id);
            self.relayout_atomic_inline_at_its_width(taffy_id);
            if self.atomic_inline_resized(id, root_id, before) {
                changed = true;
                self.queue_atomic_inlines_around(id, &mut around);
            }
        }
        // The boxes a measure — in the root compute, or in one of the
        // computes above — lined up at their max-content size for want of a
        // min-content one (#1476). One capped above has it by now.
        let asked = std::mem::take(&mut *self.tree.atomic_min_requests.borrow_mut());
        // Asked at all: some measure used a stand-in, whether its box was
        // measured here or by the capping above, so the pass has to look
        // again after the compute that uses the real size.
        self.tree.atomic_contributions_changed = !asked.is_empty();
        let measured = !asked.is_empty() && self.measure_atomic_min_contents(asked, true);
        changed || measured
    }

    /// Phase 1's reset (see `resolve_percentage_inline_blocks`): measure
    /// again as `auto`, innermost first, every atomic inline inside `outer`
    /// that is sized against its containing block, and return them (with
    /// their depths) to be resolved after `outer`. Walks `outer`'s subtree,
    /// which is only done for a box that is being resolved anyway.
    ///
    /// With `plain` false, only the boxes an IFC measure lines up at the size
    /// they have — not the plain shrink-to-fit ones, which it lines up at
    /// their own min- and max-content sizes whatever they were resolved to
    /// ([`AtomicContributions`]).
    fn reset_resolved_atomic_inlines_inside(
        &mut self,
        outer: usize,
        outer_depth: usize,
        plain: bool,
    ) -> Vec<(usize, usize)> {
        let mut found: Vec<(usize, usize)> = Vec::new();
        let mut stack: Vec<(usize, usize)> = self
            .tree
            .nodes
            .get(outer)
            .map(|n| n.children.iter().map(|&c| (c, outer_depth + 1)).collect())
            .unwrap_or_default();
        while let Some((id, depth)) = stack.pop() {
            let Some(n) = self.tree.nodes.get(id) else {
                continue;
            };
            if n.display_mode.is_atomic_inline()
                && n.ifc_root.is_some()
                && n.taffy_id.is_some()
                && Self::needs_containing_block_width(&n.computed_style)
                && (plain || plain_shrink_to_fit_margins(&n.computed_style).is_none())
            {
                found.push((depth, id));
            }
            stack.extend(n.children.iter().map(|&c| (c, depth + 1)));
        }
        found.sort_by_key(|&(d, _)| std::cmp::Reverse(d));
        for &(_, id) in &found {
            let n = &self.tree.nodes[id];
            let (Some(root_id), Some(taffy_id)) = (n.ifc_root, n.taffy_id) else {
                continue;
            };
            let before = (n.layout.width, n.layout.height);
            let _ = self.tree.taffy.mark_dirty(taffy_id);
            // Drops its `keyword_inline_cb_width` entry, so it is resolved
            // again below.
            self.measure_inline_blocks(&[(taffy_id, None)]);
            self.atomic_inline_resized(id, root_id, before);
        }
        found
    }

    /// Whether the atomic inline `id` came out at another size than
    /// `before`; if so, the IFC `root_id` it is laid out in, which line-broke
    /// against the stale box, is invalidated.
    fn atomic_inline_resized(&mut self, id: usize, root_id: usize, before: (f32, f32)) -> bool {
        let now = {
            let n = &self.tree.nodes[id];
            (n.layout.width, n.layout.height)
        };
        if (now.0 - before.0).abs() <= 0.5 && (now.1 - before.1).abs() <= 0.5 {
            return false;
        }
        self.invalidate_ifc_of_atomic_inline(root_id);
        true
    }

    /// Have IFC `root_id` measured and line-broken again: an atomic inline
    /// on its lines changed size, or what it contributes to a measure did.
    fn invalidate_ifc_of_atomic_inline(&mut self, root_id: usize) {
        if let Some(root) = self.tree.nodes.get_mut(root_id) {
            root.text_layout = None;
        }
        self.tree.dirty_ifc_text_roots.insert(root_id);
        if let Some(root_taffy) = self.tree.nodes[root_id].taffy_id {
            let _ = self.tree.taffy.mark_dirty(root_taffy);
        }
        // The measure may be cached on the root's measure leaf rather than
        // the root itself — dirty propagates up, not down (#466).
        self.mark_ifc_measure_dirty(root_id);
        // Measures cached under the stale size would be served straight
        // back. Only this root's: it used to clear the whole cache, which
        // re-shaped every IFC root in the document whenever one
        // percentage-sized chip moved by half a pixel. The signature goes
        // with them: it hashes the chip's old size.
        self.tree.forget_ifc_measures_and_signature(root_id);
    }

    /// Measure the min-content size of each `auto`-width atomic inline in
    /// `ids` that an IFC measure asked for and did not find (#1476; see
    /// [`AtomicContributions`]), and record it in
    /// `NodeTree::atomic_min_content`. Returns whether any IFC has to be
    /// measured again — a box whose min-content width is narrower than the
    /// max-content width it was lined up at in its place.
    ///
    /// The compute of one box can ask for boxes inside it; those are
    /// measured first and the box again. Afterwards the box is measured as
    /// `auto` once more, which is the state every measure with no
    /// containing-block width leaves a box in: the min-content compute left
    /// its interior laid out at that width.
    ///
    /// Costs two computes of the box, once, until something inside it
    /// changes — and only for a box some line was too narrow for.
    ///
    /// `around`: also measure again, as `auto`, every atomic inline around a
    /// box whose answer changed. The IFC that lined the box up is inside
    /// each of them, in a detached compute the root compute does not reach.
    /// Not from inside the measure of one of those boxes
    /// (`resolve_root_width_keyword`), which is laying it out anyway.
    fn measure_atomic_min_contents(&mut self, ids: Vec<usize>, around: bool) -> bool {
        let mut stack = ids;
        let mut changed = false;
        // Every round measures a box or puts strictly deeper ones ahead of
        // it, so this ends; the count is a guard, not a budget.
        let mut rounds_left = 100_000u32;
        while let Some(id) = stack.pop() {
            rounds_left -= 1;
            if rounds_left == 0 {
                break;
            }
            if self.tree.atomic_min_content.contains_key(&id) {
                continue;
            }
            let Some(node) = self.tree.nodes.get(id) else {
                continue;
            };
            let (Some(root_id), Some(taffy_id)) = (node.ifc_root, node.taffy_id) else {
                continue;
            };
            if !node.display_mode.is_atomic_inline()
                || plain_shrink_to_fit_margins(&node.computed_style).is_none()
            {
                continue;
            }
            let Some(depth) = self.depth_if_connected(id) else {
                continue;
            };
            // The boxes inside this one that some measure has asked for
            // come first: Taffy holds answers made with their stand-ins (a
            // flex item's min-content size, from any compute of this box),
            // and the compute below would be served them and ask nothing.
            let inside: Vec<usize> = {
                let asked = self.tree.atomic_min_requests.borrow();
                let mut inside: Vec<usize> = stack
                    .iter()
                    .chain(asked.iter())
                    .copied()
                    .filter(|&r| r != id && self.wants_min_content(r) && self.is_inside(r, id))
                    .collect();
                inside.sort_unstable();
                inside.dedup();
                inside
            };
            if !inside.is_empty() {
                stack.push(id);
                stack.extend(inside);
                continue;
            }
            // A box inside that is lined up at the size it was resolved to
            // (a percentage width, a `fit-content` one) is measured as
            // `auto` first, as phase 1 of `resolve_percentage_inline_blocks`
            // does and for its reason; the next pass resolves it again.
            if !self
                .reset_resolved_atomic_inlines_inside(id, depth, false)
                .is_empty()
            {
                changed = true;
            }
            let before = {
                let n = &self.tree.nodes[id];
                (n.layout.width, n.layout.height)
            };
            let asked = self.tree.atomic_min_requests.borrow().len();
            let _ = self.tree.taffy.mark_dirty(taffy_id);
            Self::compute_atomic_inline_root(
                &mut self.tree,
                &mut self.font_cx,
                &mut self.layout_cx,
                taffy_id,
                taffy::Size {
                    width: taffy::AvailableSpace::MinContent,
                    height: taffy::AvailableSpace::MaxContent,
                },
            );
            let inner = self.tree.atomic_min_requests.borrow_mut().split_off(asked);
            let min = self.tree.taffy.unrounded_layout(taffy_id).size.width;
            let _ = self.tree.taffy.mark_dirty(taffy_id);
            self.measure_inline_blocks(&[(taffy_id, None)]);
            // The boxes inside that it asked for first, then this one
            // again. Only the ones that will be measured: one that cannot
            // be would be asked for again by every compute of this box.
            let inner: Vec<usize> = inner
                .into_iter()
                .filter(|&r| r != id && self.wants_min_content(r) && self.is_inside(r, id))
                .collect();
            if !inner.is_empty() {
                stack.push(id);
                stack.extend(inner);
                continue;
            }
            self.tree.atomic_min_content.insert(id, min);
            let natural = self.tree.nodes[id].layout.width;
            let resized = self.atomic_inline_resized(id, root_id, before);
            // The `auto` measure above laid the box's interior out with no
            // containing block, so a percentage inside it lost its basis
            // (#662): the pass has to resolve the box again, whether or not
            // anything else about it changed (review of #1488).
            if self.tree.auto_inline_percent_content.contains(&id) {
                changed = true;
            }
            if !resized && min + 1.0 >= natural {
                continue;
            }
            if !resized {
                self.invalidate_ifc_of_atomic_inline(root_id);
            }
            changed = true;
            if around && let Some(parent) = self.tree.nodes[id].parent {
                self.mark_atomic_inline_dirty(parent);
            }
        }
        if around {
            self.remeasure_dirty_atomic_inlines();
        }
        changed
    }

    /// Whether `id` is a box [`Self::measure_atomic_min_contents`] measures
    /// and has not measured yet: a connected atomic inline in an IFC whose
    /// width is plain shrink-to-fit.
    fn wants_min_content(&self, id: usize) -> bool {
        !self.tree.atomic_min_content.contains_key(&id)
            && self.tree.nodes.get(id).is_some_and(|n| {
                n.ifc_root.is_some()
                    && n.taffy_id.is_some()
                    && n.display_mode.is_atomic_inline()
                    && plain_shrink_to_fit_margins(&n.computed_style).is_some()
            })
            && self.depth_if_connected(id).is_some()
    }

    /// Whether `node` is a descendant of `ancestor`.
    fn is_inside(&self, node: usize, ancestor: usize) -> bool {
        let mut cur = self.tree.nodes.get(node).and_then(|n| n.parent);
        while let Some(p) = cur {
            if p == ancestor {
                return true;
            }
            cur = self.tree.nodes.get(p).and_then(|n| n.parent);
        }
        false
    }

    /// Queue every atomic inline above `id` for phase 2 of
    /// `resolve_percentage_inline_blocks`: the IFC `id` changed in is inside
    /// each of them, and the root compute does not reach them (each is a
    /// detached root of its own).
    fn queue_atomic_inlines_around(
        &self,
        id: usize,
        around: &mut std::collections::BTreeSet<(usize, usize)>,
    ) {
        let mut cur = self.tree.nodes.get(id).and_then(|n| n.parent);
        while let Some(a) = cur {
            let Some(an) = self.tree.nodes.get(a) else {
                break;
            };
            if an.display_mode.is_atomic_inline()
                && an.ifc_root.is_some()
                && let Some(depth) = self.depth_if_connected(a)
            {
                around.insert((depth, a));
            }
            cur = an.parent;
        }
    }

    /// Lay a detached atomic inline out again at the width it already has
    /// (unrounded), so its interior and its height follow a change inside it
    /// while its width stays — see `resolve_percentage_inline_blocks`.
    fn relayout_atomic_inline_at_its_width(&mut self, taffy_id: taffy::NodeId) {
        let Ok(declared) = self.tree.taffy.style(taffy_id).cloned() else {
            return;
        };
        let w = self.tree.taffy.unrounded_layout(taffy_id).size.width;
        let mut pinned = declared.clone();
        pinned.size.width = taffy::Dimension::length(w);
        let _ = self.tree.taffy.set_style(taffy_id, pinned);
        self.measure_inline_blocks(&[(taffy_id, Some(w))]);
        let _ = self.tree.taffy.set_style(taffy_id, declared);
    }

    /// The inner (content-box) width of the box an atomic inline in
    /// `root_id`'s IFC is laid out in, from the last root compute. The IFC
    /// root is that box's containing block by construction: an IFC root is a
    /// block container, which no inline-level box and no flex container is.
    fn containing_block_inner_width(&self, root_id: usize) -> Option<f32> {
        let cb_taffy = self.tree.nodes.get(root_id)?.taffy_id?;
        // Unrounded: a box whose max-content width is its containing
        // block's (an `inline-block` around one other) would be capped at a
        // whole-pixel rounding of it, and wrap.
        let cb = self.tree.taffy.unrounded_layout(cb_taffy);
        let inner_width =
            cb.size.width - cb.padding.left - cb.padding.right - cb.border.left - cb.border.right;
        inner_width.is_finite().then_some(inner_width.max(0.0))
    }

    /// Build a Parley inline layout for an IFC root node.
    ///
    /// Walks the IFC root's children, collecting text nodes and inline elements
    /// into a single Parley TreeBuilder layout. Returns the InlineLayout.
    /// Returns the built layout and the number of per-span `font-family`
    /// resolutions `inline_style_props` actually performed (review of #1326);
    /// see the doc on [`Counter::InlineFontFamilyResolves`](crate::perf::Counter::InlineFontFamilyResolves).
    ///
    /// `align_width` is the width a paragraph none of whose text may wrap is
    /// aligned in ([`AlignWidth`]) — from the root's content width, for the paint layout — or
    /// `None` for a measure, which reads no line's position (#1237; see
    /// [`break_lines_hanging_spaces`]).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_inline_layout(
        nodes: &slab::Slab<Node>,
        root_id: usize,
        max_width: Option<f32>,
        align_width: Option<AlignWidth>,
        scale: f32,
        font_cx: &mut parley::FontContext,
        layout_cx: &mut parley::LayoutContext<Brush>,
        atomic: Option<&AtomicContributions<'_>>,
    ) -> (InlineLayout, u64) {
        // Get root style properties from typed ComputedStyle
        let root_computed = &nodes[root_id].computed_style;
        let root_font_size = root_computed.font_size * scale;
        let root_color = root_text_color(root_computed);

        let font_family = crate::fonts::parley_font_family(font_cx, &root_computed.font_family);

        let mut root_text_style = parley::style::TextStyle {
            font_size: root_font_size,
            brush: Brush::Solid(root_color),
            font_family,
            ..Default::default()
        };

        // Apply font-weight from computed style
        root_text_style.font_weight = parley::style::FontWeight::new(root_computed.font_weight);

        // Apply font-style from computed style
        root_text_style.font_style = root_computed.font_style.to_parley();

        // Apply line-height from computed style
        if let Some(lh) = root_computed.line_height.to_parley() {
            root_text_style.line_height = lh;
        }

        // Apply text-decoration from computed style. A **wavy** underline is not a
        // Parley style — it becomes a decoration span painted as a zigzag, pushed
        // over the whole flat text once the walk below knows how long that is — so
        // the straight line is suppressed here, exactly as `inline_style_props`
        // does for an inline element.
        root_text_style.has_underline = root_computed.text_decoration.underline
            && !root_computed.text_decoration.is_wavy_underline();
        root_text_style.has_strikethrough = root_computed.text_decoration.strikethrough;
        // `text-decoration-color`. Left unset for `currentcolor`, whose meaning is
        // exactly Parley's own fallback to the text brush.
        if let Some(c) = root_computed.text_decoration.color {
            root_text_style.underline_brush = Some(Brush::Solid(c));
            root_text_style.strikethrough_brush = Some(Brush::Solid(c));
        }

        // Apply text-underline-offset from computed style.
        //
        // **Negated, and that is the whole of the conversion.** The field holds
        // CSS pixels with CSS's sign — positive is *away from* the text, i.e.
        // below the baseline for horizontal text — while parley's
        // `underline_offset` is measured **up** from the baseline, which is how
        // `paint/text.rs` draws it (`gy - offset`). Both spellings replace the
        // font's own metric rather than adding to it, so nothing per-font is
        // needed here and the two agree up to this sign. See
        // [`crate::computed_style::ComputedStyle::text_underline_offset`].
        //
        // Always `None` today — no CSS reaches the field (#580) — so this line
        // and its sign exist only because
        // `crates/rinch-dom/tests/underline_offset_tests.rs` sets the field
        // directly. Without those fixtures both are equivalent mutants.
        if let Some(css_offset) = root_computed.text_underline_offset {
            root_text_style.underline_offset = Some(-css_offset);
        }

        // Apply overflow-wrap from computed style
        root_text_style.overflow_wrap = root_computed.overflow_wrap.to_parley();

        // Apply letter-/word-spacing from computed style (#698).
        //
        // **In CSS pixels, not scaled here.** `LayoutContext::tree_builder`
        // resolves the root `TextStyle` through `resolve_entire_style_set`,
        // which multiplies `letter_spacing` and `word_spacing` by the `scale`
        // it was handed — so pre-scaling them the way `root_font_size` above is
        // pre-scaled would apply the factor twice. Every caller of this function
        // passes `scale == 1.0` (layout happens in logical pixels; paint
        // re-applies the DPI factor), so the two spellings agree today and only
        // this one keeps agreeing if that ever changes.
        //
        // Both are pushed unconditionally rather than behind a `!= 0.0` test:
        // the default is exactly parley's own default, so a zero costs nothing,
        // and a conditional here would have to be mirrored in
        // `inline_style_props` to let a span reset an inherited spacing back to
        // zero.
        root_text_style.letter_spacing = root_computed.letter_spacing;
        root_text_style.word_spacing = root_computed.word_spacing;

        // The root's own `nowrap`/`pre` forbids a soft wrap in its own text,
        // while a descendant that allows wrapping still wraps (#1212). Parley
        // takes a break opportunity's wrap mode from the cluster *before* it.
        // That agrees with Chrome 153 at `pre` root text followed by a
        // `normal` span (no break at the seam) and between two `normal` spans
        // (a break, though their common ancestor is the root), and differs
        // after a `normal` span followed by the root's preserved spaces,
        // where Chrome breaks before the spaces and rinch does not.
        root_text_style.text_wrap_mode = Self::text_wrap_mode(root_computed);
        root_text_style.white_space_collapse = white_space_hanging(root_computed.white_space);

        // Apply white-space mode from computed style.
        // Contenteditable elements always use Preserve (pre-wrap) to prevent
        // Parley from collapsing trailing whitespace, which would cause cursor
        // position mismatches (the DOM text has the space but the layout doesn't).
        let is_contenteditable = {
            let mut nid = Some(root_id);
            let mut found = false;
            while let Some(id) = nid {
                if nodes[id].attributes.contains_key("contenteditable") {
                    found = true;
                    break;
                }
                nid = nodes[id].parent;
            }
            found
        };
        let mut child_positions = Vec::new();
        let mut text_ranges = Vec::new();
        let mut background_spans = Vec::new();
        let mut decoration_spans = Vec::new();
        let mut vertical_align_spans = Vec::new();
        let mut flat_pos = 0usize;

        // Walk children into an `IfcText`, which collapses white space across
        // the whole IFC (#1180), each text node by its own element's
        // `white-space` (#1192), then replay it into the Parley tree.
        //
        // `font_family_resolves` counts `inline_style_props`'s actual
        // `parley_font_family` calls (review of #1326's perf finding) — this
        // function has no `&PerfCounters` of its own (only the node slab), so
        // the three callers bump `Counter::InlineFontFamilyResolves` by the
        // second element of the returned tuple.
        let font_family_resolves = std::cell::Cell::new(0u64);
        let mut ifc_text = IfcText::new(is_contenteditable);
        Self::walk_inline_children(
            nodes,
            root_id,
            &mut ifc_text,
            &mut child_positions,
            &mut text_ranges,
            &mut background_spans,
            &mut decoration_spans,
            &mut vertical_align_spans,
            &mut flat_pos,
            scale,
            font_cx,
            &font_family_resolves,
            atomic,
        );
        // An emoji-presentation cluster is shaped with the root's stack, its
        // generics replaced by their primary face (#1204), whatever family an
        // enclosing inline element pushed for its own text (#677): emoji
        // handling is root-wide, not per-span, so one emoji family serves the
        // whole IFC even though a `<code>` or `<kbd>` span now shapes its own
        // ordinary glyphs in its own family. Asked only when some text could
        // hold one, after the walk and before the builder borrows the font
        // context.
        let emoji_family = if ifc_text.may_hold_emoji() {
            crate::fonts::parley_emoji_font_family(font_cx, &root_computed.font_family)
        } else {
            None
        };
        ifc_text.finish();
        // Every offset the walk recorded counted a held space `finish` has
        // since removed; take them all to the final text.
        for r in &mut text_ranges {
            r.flat_start = ifc_text.remap(r.flat_start);
            r.flat_end = ifc_text.remap(r.flat_end);
        }
        for b in &mut background_spans {
            b.start = ifc_text.remap(b.start);
            b.end = ifc_text.remap(b.end);
        }
        for d in &mut decoration_spans {
            d.start = ifc_text.remap(d.start);
            d.end = ifc_text.remap(d.end);
        }
        for v in &mut vertical_align_spans {
            v.start = ifc_text.remap(v.start);
            v.end = ifc_text.remap(v.end);
        }
        let flat_len = ifc_text.final_len();
        let mut out_of_flow = ifc_text.out_of_flow_marks();

        // The IFC root's own wavy underline covers everything the walk produced.
        // (An inline element's covers its own range and is pushed by the walk.)
        if root_computed.text_decoration.is_wavy_underline() && flat_len > 0 {
            decoration_spans.push(crate::node::InlineDecorationSpan {
                owner: root_id,
                start: 0,
                end: flat_len,
                color: wavy_underline_color(root_computed),
            });
        }

        // A paragraph none of whose text may wrap is broken unconstrained:
        // only forced breaks end its lines. One that holds wrapping text — a
        // wrapping root, or a `normal`/`pre-wrap`/`pre-line` element inside a
        // `nowrap`/`pre` root (#1212) — is broken at the available width, and
        // the per-cluster `TextWrapMode` keeps the rest of it on its lines.
        let boxes: Vec<usize> = ifc_text.atomic_box_ids().collect();
        let effective_max_width = if Self::wraps_anywhere(nodes, root_id, &text_ranges, &boxes) {
            max_width
        } else {
            None
        };
        // Only preserved spaces can hang past a soft wrap or end a line as
        // content; collapsible ones are single, and gone at a forced break.
        let preserves_spaces = ifc_text.preserved();
        // The text of `pre` elements, whose preserved spaces must not hang
        // (`LineUnit::unhung`; next to wrapping spaces, see `pre_seam`),
        // and of `break-spaces` ones, whose spaces never hang (#1043).
        let mut unhung = UnhungSpaces::default();
        if preserves_spaces {
            use crate::computed_style::WhiteSpaceValue;
            for r in text_ranges
                .iter()
                .filter(|r| !r.is_br && r.flat_end > r.flat_start)
            {
                let white_space = nodes
                    .get(r.node_id)
                    .and_then(|t| t.parent)
                    .and_then(|el| nodes.get(el))
                    .map(|el| el.computed_style.white_space);
                match white_space {
                    Some(WhiteSpaceValue::Pre) => unhung.pre.push(r.flat_start..r.flat_end),
                    Some(WhiteSpaceValue::BreakSpaces) => {
                        unhung.breaking.push(r.flat_start..r.flat_end)
                    }
                    _ => {}
                }
            }
        }
        // #1238: a paragraph broken at a width that holds atomic inlines and
        // some element that does not wrap is broken where CSS breaks next to
        // its boxes ([`WrapSeams`]), not where parley does. #1243: so is one
        // that holds a `pre` element's spaces (a wrapping paragraph does
        // then): parley takes no break between a wrapping run of spaces and
        // a `pre` one, and hangs a `pre` space that overflows, breaking the
        // `pre` run.
        // Whether it does is known once parley has broken it
        // ([`pre_seams_needed`]); its ranges are filled now.
        let pre_candidate =
            effective_max_width.is_some() && Self::any_pre_space(nodes, &text_ranges, &unhung.pre);
        unhung.pre_candidate = pre_candidate;
        let needs_seams = effective_max_width.is_some()
            && !boxes.is_empty()
            && Self::any_nowrap_element(nodes, root_id, &text_ranges, &boxes);
        unhung.box_seams = needs_seams;
        if pre_candidate {
            unhung.collapsed = Self::collapsed_after_close(nodes, root_id, &text_ranges);
            use crate::computed_style::WhiteSpaceValue;
            for r in text_ranges
                .iter()
                .filter(|r| !r.is_br && r.flat_end > r.flat_start)
            {
                let collapses = nodes
                    .get(r.node_id)
                    .and_then(|t| t.parent)
                    .and_then(|el| nodes.get(el))
                    .is_some_and(|el| {
                        matches!(
                            el.computed_style.white_space,
                            WhiteSpaceValue::Normal
                                | WhiteSpaceValue::NoWrap
                                | WhiteSpaceValue::PreLine
                        )
                    });
                if collapses {
                    unhung.collapsible.push(r.flat_start..r.flat_end);
                }
            }
        }
        // #1426: where an edge box's false break opportunities are undone,
        // text that may not wrap offers no opportunity either; the same where
        // a box's are (#1238).
        if ifc_text.has_edge_boxes() || needs_seams || pre_candidate {
            for r in text_ranges
                .iter()
                .filter(|r| !r.is_br && r.flat_end > r.flat_start)
            {
                let wraps = nodes
                    .get(r.node_id)
                    .and_then(|t| t.parent)
                    .and_then(|el| nodes.get(el))
                    .is_none_or(|el| {
                        Self::text_wrap_mode(&el.computed_style)
                            == parley::style::TextWrapMode::Wrap
                    });
                if !wraps {
                    unhung.nowrap.push(r.flat_start..r.flat_end);
                }
            }
        }
        // Build and break; with preserved tabs, again until each tab ends at
        // its stop (#1122): a tab's width depends on where its line puts it.
        let tabs = ifc_text.tab_offsets();
        let mut tab_spacing = vec![0.0f32; tabs.len()];
        let stops = (!tabs.is_empty()).then(|| {
            TabStops::of_block(
                font_cx,
                layout_cx,
                &root_text_style,
                root_computed.letter_spacing,
                root_computed.word_spacing,
                scale,
            )
        });
        let alignment = root_computed.text_align.to_parley();
        // A line aligned away from its start in a left-to-right block is
        // aligned within the content width even when nothing wraps (#1237):
        // rinch hands parley no CSS `direction`.
        let ltr = css_direction_is_ltr(nodes, root_id);
        let aligned_away = matches!(
            alignment,
            parley::layout::Alignment::Right
                | parley::layout::Alignment::Center
                | parley::layout::Alignment::End
        ) && ltr;
        let mut hang = HangStats::default();
        let mut builds = 0u32;
        let (text_layout, text_content) = loop {
            let mut builder = layout_cx.tree_builder(font_cx, scale, true, &root_text_style);
            // A `direction: rtl` block's paragraph is right to left whatever
            // its text (parley 0.12 takes a base direction): its trailing
            // spaces are at its visual left, inside the box (Chrome 155,
            // review of #1669). A `ltr` one — the initial value, so nearly
            // every block — is still left to parley's first-strong rule.
            if !ltr {
                builder.set_base_direction(parley::BaseDirection::Rtl);
            }
            ifc_text.replay(&mut builder, emoji_family.as_ref(), &tab_spacing);
            let (mut text_layout, text_content) = builder.build();
            builds += 1;
            if (needs_seams || pre_candidate) && unhung.seams.is_none() {
                unhung.seams = Some(WrapSeams::of(nodes, root_id, &text_ranges, &text_layout));
            }
            hang.add(break_lines_hanging_spaces(
                &mut text_layout,
                &text_content,
                effective_max_width,
                preserves_spaces,
                &unhung,
                align_width.filter(|_| aligned_away),
            ));
            let Some(stops) = &stops else {
                break (text_layout, text_content);
            };
            let deltas = stops.deltas(&text_layout, &tabs);
            if deltas.iter().all(|d| d.abs() < TAB_STOP_TOLERANCE) || builds >= MAX_TAB_BUILDS {
                break (text_layout, text_content);
            }
            for (spacing, d) in tab_spacing.iter_mut().zip(&deltas) {
                *spacing += d / scale;
            }
            hang.tab_rebuilds += 1;
        };
        let mut text_layout = text_layout;

        // Apply text-align from computed style
        text_layout.align(alignment, parley::layout::AlignmentOptions::default());
        resolve_out_of_flow_marks(&text_layout, &text_content, scale, &mut out_of_flow);

        (
            InlineLayout {
                layout: text_layout,
                text_content,
                child_positions,
                text_ranges,
                root_color,
                background_spans,
                decoration_spans,
                vertical_align_spans,
                max_width: max_width.unwrap_or(f32::INFINITY),
                align_width: align_width.map_or(f32::INFINITY, |a| a.width),
                preserves_spaces,
                break_spaces: unhung.breaking,
                hang,
                span_fragments: Default::default(),
                out_of_flow,
                ellipsis: None,
            },
            font_family_resolves.get(),
        )
    }

    /// Whether anything in an IFC may soft-wrap: the root itself (between its
    /// own text and its atomic inlines alike), an element holding some text
    /// run inside it (#1212), or one of its atomic inlines (`boxes`), however
    /// deep (#1238: a `normal` span of
    /// inline-blocks breaks between them, and a span that closes between
    /// two boxes lends them its opportunity, [`WrapSeams`]). `false` only for
    /// a `nowrap`/`pre` root none of whose content is in a wrapping element.
    pub(crate) fn wraps_anywhere(
        nodes: &slab::Slab<Node>,
        root_id: usize,
        text_ranges: &[crate::node::IfcTextRange],
        boxes: &[usize],
    ) -> bool {
        let wraps = |id: usize| {
            nodes.get(id).is_some_and(|n| {
                Self::text_wrap_mode(&n.computed_style) == parley::style::TextWrapMode::Wrap
            })
        };
        wraps(root_id)
            || text_ranges
                .iter()
                .filter(|r| !r.is_br)
                .any(|r| Self::ancestors_in_ifc(nodes, root_id, r.node_id).any(wraps))
            || boxes
                .iter()
                .any(|&b| Self::ancestors_in_ifc(nodes, root_id, b).any(wraps))
    }

    /// Where a text node whose spaces collapsed away entirely stands right
    /// after text whose element closes into the collapsed node's (#1243,
    /// review of #1897): `<span>dd </span> <span pre>…` — the root's space
    /// collapsed after the span's. Chrome 155 breaks before the `pre` spaces
    /// there as it does after a run of spaces across two text nodes; not
    /// when the collapsed space is in the same element as the text before
    /// it (`dd ` and ` ` as two text nodes of one element), in a sibling
    /// element that opens there (`dd <span> </span><span pre>`), or after a
    /// comment. Sorted flat bytes.
    fn collapsed_after_close(
        nodes: &slab::Slab<Node>,
        root_id: usize,
        text_ranges: &[crate::node::IfcTextRange],
    ) -> Vec<usize> {
        let parent = |id: usize| nodes.get(id).and_then(|n| n.parent);
        let mut out = Vec::new();
        let mut prev: Option<&crate::node::IfcTextRange> = None;
        for r in text_ranges.iter().filter(|r| !r.is_br) {
            if r.flat_end > r.flat_start {
                prev = Some(r);
                continue;
            }
            let collapsed_space = nodes
                .get(r.node_id)
                .and_then(|n| n.text_content())
                .is_some_and(|t| !t.is_empty());
            let Some(before) = prev.filter(|p| p.flat_end == r.flat_start) else {
                continue;
            };
            let (Some(into), Some(from)) = (parent(r.node_id), parent(before.node_id)) else {
                continue;
            };
            // `into` is a proper ancestor of the element the text before it is
            // in: that element closes there.
            let closes = into != from
                && std::iter::successors(parent(from), |&id| {
                    (id != root_id).then(|| parent(id)).flatten()
                })
                .chain(std::iter::once(root_id))
                .any(|id| id == into);
            if collapsed_space && closes {
                out.push(r.flat_start);
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Whether some text of a `pre` element (`pre`, the `white-space` ranges
    /// of [`UnhungSpaces::pre`]) holds a space or tab (#1243).
    fn any_pre_space(
        nodes: &slab::Slab<Node>,
        text_ranges: &[crate::node::IfcTextRange],
        pre: &[std::ops::Range<usize>],
    ) -> bool {
        !pre.is_empty()
            && text_ranges
                .iter()
                .filter(|r| !r.is_br && r.flat_end > r.flat_start && in_ranges(pre, r.flat_start))
                .any(|r| {
                    nodes
                        .get(r.node_id)
                        .and_then(|n| n.text_content())
                        .is_some_and(|t| t.contains([' ', '\t']))
                })
    }

    /// Whether some element holding content of an IFC — its root, the
    /// element of a text run, or an element holding one of its atomic
    /// inlines (`boxes`) — does not wrap (#1238).
    fn any_nowrap_element(
        nodes: &slab::Slab<Node>,
        root_id: usize,
        text_ranges: &[crate::node::IfcTextRange],
        boxes: &[usize],
    ) -> bool {
        let nowrap = |id: usize| {
            nodes.get(id).is_some_and(|n| {
                Self::text_wrap_mode(&n.computed_style) != parley::style::TextWrapMode::Wrap
            })
        };
        nowrap(root_id)
            || text_ranges
                .iter()
                .filter(|r| !r.is_br)
                .any(|r| Self::ancestors_in_ifc(nodes, root_id, r.node_id).any(nowrap))
            || boxes
                .iter()
                .any(|&b| Self::ancestors_in_ifc(nodes, root_id, b).any(nowrap))
    }

    /// The elements from `node`'s parent up to, not including, the IFC root.
    fn ancestors_in_ifc(
        nodes: &slab::Slab<Node>,
        root_id: usize,
        node: usize,
    ) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(nodes.get(node).and_then(|n| n.parent), |&id| {
            nodes.get(id).and_then(|n| n.parent)
        })
        .take_while(move |&id| id != root_id)
    }

    /// Where `text-overflow: ellipsis` cuts `il`'s lines (#1100), or `None`
    /// when no line overflows `container_width`.
    ///
    /// A line overflows when its content (its advance less its trailing white
    /// space, as parley's own `Layout::width` measures it) is wider than the
    /// box (#1091: the question is asked per line, after wrapping). On such a
    /// line everything whose **visual** extent passes the point where the "…"
    /// still fits before the line's end edge is hidden — each cluster, and
    /// each atomic inline whole — and the "…" is placed right after what is
    /// kept: the box's right edge less the "…" when the line leaves the box by
    /// its right, its left edge plus the "…" when it leaves by its left (an
    /// overflowing right-to-left line, which parley aligns to the left of the
    /// box). That is Chrome 155's cut
    /// (`ellipsis_paint_cut_1100_tests.rs`): it is made by position, so a
    /// right-to-left run in a left-to-right line is cut where it is drawn,
    /// not at its logical end. Edge boxes (#1426) draw nothing and are never
    /// hidden; one that fits moves the "…" past it.
    ///
    /// Nothing but the "…" is shaped: the lines are the ones laid out, and
    /// paint draws the cut ([`crate::node::EllipsisCut`]).
    fn ellipsis_cut(
        nodes: &slab::Slab<Node>,
        root_id: usize,
        il: &InlineLayout,
        container_width: f32,
        font_cx: &mut parley::FontContext,
        layout_cx: &mut parley::LayoutContext<Brush>,
    ) -> Option<crate::node::EllipsisCut> {
        use parley::layout::PositionedLayoutItem;
        // A tolerance for the float sums: a cluster ending this close past
        // the limit still fits.
        const EPS: f32 = 0.01;
        let overflowing = |line: &parley::layout::Line<'_, Brush>| {
            let m = line.metrics();
            m.advance - m.hanging_advance > container_width
        };
        if !il.layout.lines().any(|l| overflowing(&l)) {
            return None;
        }
        // The "…" is the root's text, undecorated: Chrome 155 draws it in the
        // block's font and colour, and draws no underline under it even where
        // the block itself is underlined.
        let mut style = EllipsisStyle::of(&nodes[root_id].computed_style, 1.0);
        style.underline = false;
        style.strikethrough = false;
        let mut glyph = style.shape(layout_cx, font_cx, "\u{2026}", 1.0, true);
        glyph.break_all_lines(None);
        let ellipsis_width = glyph.width();
        let glyph_baseline = glyph.lines().next().map_or(0.0, |l| l.metrics().baseline);
        let rtl = il.layout.is_rtl();

        let mut hidden: Vec<std::ops::Range<usize>> = Vec::new();
        let mut hidden_boxes = Vec::new();
        let mut marks = Vec::new();
        // One line's items: `(x0, x1, what)`. A cluster item is a unit that
        // is kept or hidden whole: a grapheme, and a ligature with every
        // cluster its glyph covers. Paint draws a glyph with the first
        // cluster it belongs to, so hiding part of either drew the whole
        // glyph under the "…" (review of #1860).
        enum Item {
            Cluster(Vec<std::ops::Range<usize>>, bool),
            Edge,
            Atomic(usize),
        }
        let mut items: Vec<(f32, f32, Item)> = Vec::new();
        // `(range, x0, x1, space, starts a unit)` per cluster of a line.
        let mut clusters: Vec<(std::ops::Range<usize>, f32, f32, bool, bool)> = Vec::new();
        let text = il.text_content.as_str();
        for (index, line) in il.layout.lines().enumerate() {
            if !overflowing(&line) {
                continue;
            }
            let m = line.metrics();
            items.clear();
            clusters.clear();
            for run in line.runs() {
                let Some(mut x) = run.visual_clusters().next().and_then(|c| c.visual_offset())
                else {
                    continue;
                };
                for cluster in run.visual_clusters() {
                    let x1 = x + cluster.advance();
                    let r = cluster.text_range();
                    let starts = !cluster.is_ligature_continuation()
                        && rinch_words::is_grapheme_boundary(text, r.start);
                    clusters.push((r, x, x1, cluster.is_space_or_nbsp(), starts));
                    x = x1;
                }
            }
            // Group the clusters into units in text order.
            clusters.sort_by_key(|c| c.0.start);
            for (r, x0, x1, space, starts) in clusters.drain(..) {
                match items.last_mut() {
                    Some((ux0, ux1, Item::Cluster(ranges, uspace))) if !starts => {
                        *ux0 = ux0.min(x0);
                        *ux1 = ux1.max(x1);
                        *uspace &= space;
                        ranges.push(r);
                    }
                    _ => items.push((x0, x1, Item::Cluster(vec![r], space))),
                }
            }
            for item in line.items() {
                if let PositionedLayoutItem::InlineBox(b) = item {
                    let what = if is_edge_box(b.id) {
                        Item::Edge
                    } else {
                        Item::Atomic(b.id as usize)
                    };
                    items.push((b.x, b.x + b.width, what));
                }
            }
            // The side the line's content leaves the box by is its end
            // edge: the right for a left-to-right paragraph, the left for a
            // right-to-left one, whose overflowing lines parley moves left
            // of the box when it aligns them against it. An unconstrained
            // (`nowrap`/`pre`) right-to-left line is not so moved and leaves
            // by the right, where the "…" then goes, inside the box.
            let (lo, hi) = items
                .iter()
                .filter(|(_, _, w)| !matches!(w, Item::Cluster(_, true) | Item::Edge))
                .fold(
                    (f32::INFINITY, f32::NEG_INFINITY),
                    |(lo, hi), &(a, b, _)| (lo.min(a), hi.max(b)),
                );
            let (over_left, over_right) = (lo < -EPS, hi > container_width + EPS);
            let at_left = if over_left != over_right {
                over_left
            } else {
                rtl
            };
            // Whether an extent `x0..x1` is past the point the "…" fits at.
            let past = |x0: f32, x1: f32| {
                if at_left {
                    x0 < ellipsis_width - EPS
                } else {
                    x1 > container_width - ellipsis_width + EPS
                }
            };
            // The kept content's extent at the end edge.
            let mut kept: Option<f32> = None;
            for (x0, x1, what) in items.drain(..) {
                if past(x0, x1) {
                    match what {
                        Item::Cluster(ranges, _) => {
                            hidden.extend(ranges.into_iter().filter(|r| !r.is_empty()))
                        }
                        Item::Atomic(id) => hidden_boxes.push(id),
                        _ => {}
                    }
                } else {
                    let edge = if at_left { x0 } else { x1 };
                    kept = Some(match kept {
                        None => edge,
                        Some(k) if at_left => k.min(edge),
                        Some(k) => k.max(edge),
                    });
                }
            }
            let x = match (kept, at_left) {
                (Some(edge), false) => edge,
                (Some(edge), true) => edge - ellipsis_width,
                (None, false) => m.offset.max(0.0),
                (None, true) => (m.offset + m.advance).min(container_width) - ellipsis_width,
            };
            marks.push(crate::node::EllipsisMark {
                line: index,
                x,
                y: m.baseline - glyph_baseline,
                // Past this x, in the line's visual order, everything is
                // hidden: paint stops walking the line there.
                hidden_past: (!at_left).then_some(container_width - ellipsis_width + EPS),
            });
        }
        hidden.sort_by_key(|r| r.start);
        let mut merged: Vec<std::ops::Range<usize>> = Vec::with_capacity(hidden.len());
        for r in hidden {
            match merged.last_mut() {
                Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
                _ => merged.push(r),
            }
        }
        Some(crate::node::EllipsisCut {
            hidden: merged,
            hidden_boxes,
            marks,
            glyph,
        })
    }

    /// Whether two computed styles agree on every property
    /// [`Self::inline_style_props`] reads.
    ///
    /// Inheritance means a child that declares none of them agrees with its
    /// parent on all of them, so this answers "did this element change the text
    /// style at all" — and a span that changes nothing need not be built or
    /// pushed. That matters because rsx emits a `display: contents` wrapper for
    /// every `if` / `match` / `for` and every reactive component, and virtually
    /// none of them declares anything: measured on 1000 undeclared wrappers,
    /// pushing a span for each cost **+8%** of the whole layout pass, and
    /// **+21%** with them nested three deep. With this test the same shapes are
    /// back at parity.
    ///
    /// Hand-written rather than `PartialEq`, because
    /// [`crate::computed_style::values::LineHeightValue`] does not derive it
    /// and giving an f32-carrying enum a derived equality is a worse trade than
    /// spelling out the ten fields that matter here. **If
    /// `inline_style_props` gains a property, it must gain one here too** —
    /// they are one list, and a field present there and missing here is a
    /// declaration that silently stops applying. Seven of the ten have a
    /// fixture in `contents_wrapper_inherited_style_tests` that dies when their
    /// line is deleted, `text_underline_offset` is covered elsewhere as below,
    /// and the `letter_spacing`/`word_spacing` pair added by #698 is
    /// `a_wrappers_letter_and_word_spacing_reach_its_text` in that same file —
    /// which covers **each clause separately**, over content the property under
    /// test can actually move.
    ///
    /// That last one is worth a paragraph, because two obvious fixtures do
    /// **not** cover it. This predicate gates only a boxless wrapper and the
    /// split-inline bridge; a real `display: inline` span pushes its properties
    /// unconditionally, so a span-scoped spacing test exercises
    /// `inline_style_props` and never reaches here. Measured: a mutant deleting
    /// both clauses survived the whole `rinch-dom` suite, #698's own fixtures
    /// included, until a `display: contents` fixture was written for it. And it
    /// had to measure the **line width** — spacing moves the same glyphs apart,
    /// so this file's ink and colour oracles cannot see it.
    ///
    /// Then the same trap a second time, one level down: with a single
    /// `letter-spacing` row the *word* clause was still unwitnessed across all
    /// 78 test binaries, because deleting it changes nothing in text that
    /// declares only the other property. Adjacent clauses invite a mutant that
    /// deletes both and dies on either; each needs content of its own.
    ///
    /// **`text_underline_offset` is inert but not untested** (#580). No CSS can
    /// make it `Some` — the property is gecko-only in this Stylo build, so the
    /// declaration is discarded before `ComputedStyle::from_stylo` runs, and
    /// that one line is the only place CSS could reach the field. It writes
    /// `None` unconditionally. (The field has two other writers and neither is a
    /// CSS path: `Default` holds `None`, and
    /// [`crate::computed_style::ComputedStyle::for_anonymous_box`] copies
    /// `parent.text_underline_offset` — the property inherits, so an anonymous
    /// box gets whatever its parent had, which today is `None` because the root
    /// had `None`.) So in production this comparison is always `None == None`,
    /// and the `if let Some(..)` arm in [`Self::inline_style_props`] never fires.
    ///
    /// That made a mutant deleting either one look equivalent by construction,
    /// which it is not: `crates/rinch-dom/tests/underline_offset_tests.rs` sets
    /// the field directly and kills all three — this clause, that arm, and the
    /// `root_text_style` assignment in [`Self::build_inline_layout`]. The
    /// clause's own witness is
    /// `a_contents_wrapper_is_not_skipped_when_only_its_offset_differs`, where
    /// the wrapper's offset is the only thing that differs from its parent's
    /// style, so dropping this line skips the wrapper and loses it.
    ///
    /// Kept for the one-list rule: whoever plumbs the property through gets the
    /// skip predicate already correct rather than discovering a year later that
    /// their declaration is dropped. The sign is already correct too — both
    /// consumers negate, because the field is CSS-signed and parley's offset is
    /// baseline-up. See the field's own doc.
    fn same_inline_text_style(
        a: &crate::computed_style::ComputedStyle,
        b: &crate::computed_style::ComputedStyle,
    ) -> bool {
        Self::same_inline_text_style_but_wrap(a, b)
            && Self::text_wrap_mode(a) == Self::text_wrap_mode(b)
    }

    /// [`Self::same_inline_text_style`] less the wrap mode: every property
    /// that decides how a span's glyphs look.
    fn same_inline_text_style_but_wrap(
        a: &crate::computed_style::ComputedStyle,
        b: &crate::computed_style::ComputedStyle,
    ) -> bool {
        use crate::computed_style::values::LineHeightValue;
        let same_line_height = match (a.line_height, b.line_height) {
            (LineHeightValue::Normal, LineHeightValue::Normal) => true,
            (LineHeightValue::Absolute(x), LineHeightValue::Absolute(y)) => x == y,
            (LineHeightValue::Relative(x), LineHeightValue::Relative(y)) => x == y,
            _ => false,
        };
        a.font_size == b.font_size
            && a.font_weight == b.font_weight
            && a.font_family == b.font_family
            && a.font_style == b.font_style
            && a.color == b.color
            && a.text_decoration == b.text_decoration
            && a.text_underline_offset == b.text_underline_offset
            && same_line_height
            && a.letter_spacing == b.letter_spacing
            && a.word_spacing == b.word_spacing
    }

    /// Whether an element's text may wrap: `nowrap` and `pre` forbid it
    /// (#1091). Pushed per inline element, so a `nowrap` span inside a
    /// wrapping root keeps its text on one line — and overflows that line,
    /// which is what draws its `text-overflow: ellipsis`. The IFC root's own
    /// mode is its root text style's (#1212), so a wrapping span inside a
    /// `nowrap`/`pre` root wraps while the root's own text does not.
    fn text_wrap_mode(
        computed: &crate::computed_style::ComputedStyle,
    ) -> parley::style::TextWrapMode {
        use crate::computed_style::WhiteSpaceValue;
        match computed.white_space {
            WhiteSpaceValue::NoWrap | WhiteSpaceValue::Pre => parley::style::TextWrapMode::NoWrap,
            _ => parley::style::TextWrapMode::Wrap,
        }
    }

    /// The Parley style span an element contributes to the inline formatting
    /// context it flows into — the inherited text properties, scaled.
    ///
    /// Shared by the `display: inline` arm and the `display: contents` arm of
    /// [`Self::walk_inline_children`] (#574). A boxless element generates no
    /// box but is still in the inheritance chain, so the two arms owe their
    /// children the same properties; only the *box*-shaped work (the background
    /// span) is the inline element's alone.
    ///
    /// **Includes `font-family` (#677).** Without it a span inherited the IFC
    /// root's family whatever its own computed style said — `<code>`'s
    /// `font-family: monospace` inside a paragraph rendered in the paragraph's
    /// proportional face. Resolved through [`crate::fonts::parley_font_family`],
    /// the same generic/fallback resolution the root's own family goes through
    /// in [`Self::build_inline_layout`], so a generic such as `monospace` on a
    /// span is resolved identically to one on the root. One known gap stays
    /// open: an emoji-presentation cluster inside such a span is still shaped
    /// with the *root's* emoji family ([`Self::build_inline_layout`]'s
    /// `emoji_family`, computed once from `root_computed.font_family`), not the
    /// span's — out of scope for #677, which is about text glyphs, not emoji.
    ///
    /// **`parent_family` is the cheap skip (review of #1326).** `font-family`
    /// is an inherited CSS property, so an element that does not declare its
    /// own carries its parent's value already — `resolve_property`'s
    /// `FontFamily` resolution is the expensive half of this push (a cache
    /// lookup plus an owned `Cow` clone even on a hit), and calling it for
    /// every one of a document's spans whether or not the family actually
    /// changed cost 30-50% of wall time on an adversarial 3000-span document
    /// (measured, review of #1326). `parent_family` is every call site's own
    /// `nodes[parent_id].computed_style.font_family` — the enclosing IFC
    /// context's family — so `computed.font_family == parent_family` means
    /// Parley already has the right value from whichever ancestor last
    /// changed it, and no span-level override is needed at all.
    // The leading unconditional pushes (`FontSize` through `FontStyle`) read
    // as a `vec![]` candidate in isolation, but the conditional pushes further
    // down mean the whole function cannot be one literal.
    #[allow(clippy::vec_init_then_push)]
    fn inline_style_props(
        computed: &crate::computed_style::ComputedStyle,
        parent_family: &str,
        scale: f32,
        font_cx: &mut parley::FontContext,
        font_family_resolves: &std::cell::Cell<u64>,
    ) -> Vec<parley::style::StyleProperty<'static, Brush>> {
        let mut props: Vec<parley::style::StyleProperty<'static, Brush>> = Vec::new();
        if computed.font_family != parent_family {
            props.push(parley::style::StyleProperty::FontFamily(
                crate::fonts::parley_font_family(font_cx, &computed.font_family),
            ));
            font_family_resolves.set(font_family_resolves.get() + 1);
        }
        props.push(parley::style::StyleProperty::FontSize(
            computed.font_size * scale,
        ));
        props.push(parley::style::StyleProperty::FontWeight(
            parley::style::FontWeight::new(computed.font_weight),
        ));
        props.push(parley::style::StyleProperty::FontStyle(
            computed.font_style.to_parley(),
        ));
        if let Some(color) = computed.color {
            props.push(parley::style::StyleProperty::Brush(Brush::Solid(color)));
        }
        // A wavy underline is painted by `paint::text` from a decoration span
        // (Parley has no wavy decoration), so a wavy element pushes no
        // `Underline` at all — and in particular does not turn one *off*. CSS
        // propagates an ancestor's decoration to every descendant's glyphs and a
        // descendant cannot remove it, so a misspelled word inside `<u>` or an
        // underlined link keeps its straight line and gains the squiggle, which
        // is what a browser draws (review of #836, finding 4).
        let wavy = computed.text_decoration.is_wavy_underline();
        if computed.text_decoration.underline && !wavy {
            props.push(parley::style::StyleProperty::Underline(true));
        }
        if computed.text_decoration.strikethrough {
            props.push(parley::style::StyleProperty::Strikethrough(true));
        }
        // `text-decoration-color`; see the sibling assignment in
        // `build_inline_layout` for why `currentcolor` pushes nothing. A wavy
        // element's colour belongs to its squiggle (the decoration span carries
        // it), not to an underline it inherited.
        if let Some(c) = computed.text_decoration.color {
            if !wavy {
                props.push(parley::style::StyleProperty::UnderlineBrush(Some(
                    Brush::Solid(c),
                )));
            }
            props.push(parley::style::StyleProperty::StrikethroughBrush(Some(
                Brush::Solid(c),
            )));
        }
        // Negated on the way out, for the reason spelled out at the sibling push
        // in [`Self::build_inline_layout`]: the field is CSS-signed (positive is
        // away from the text), parley's is baseline-up.
        //
        // **Always `None` in production, because no CSS path sets the field
        // today.** `text-underline-offset` is declared `engines="gecko"` at
        // `stylo-0.11.0/properties/longhands/inherited_text.mako.rs:330`, and
        // stylo's `build.rs` generates exactly one engine's property set — so
        // the servo build emits no parser entry and Stylo discards the
        // declaration as unknown. Measured in a generated `properties.rs`:
        // `text_underline_offset` 0 hits, the documented gecko-only
        // `scrollbar_color` 0 hits, and `text_decoration_line` 52 hits as the
        // positive control that the grep fires at all (#580).
        if let Some(css_offset) = computed.text_underline_offset {
            props.push(parley::style::StyleProperty::UnderlineOffset(Some(
                -css_offset,
            )));
        }
        if let Some(lh) = computed.line_height.to_parley() {
            let scaled_lh = match lh {
                parley::style::LineHeight::Absolute(v) => {
                    parley::style::LineHeight::Absolute(v * scale)
                }
                other => other,
            };
            props.push(parley::style::StyleProperty::LineHeight(scaled_lh));
        }
        // letter-/word-spacing (#698), in CSS pixels — parley scales a pushed
        // `LetterSpacing`/`WordSpacing` by the builder's own factor, so this is
        // the sibling of the unscaled assignment in `build_inline_layout` and
        // not of the pre-scaled `FontSize` above.
        //
        // Unconditional, and that is the point: both properties inherit, so a
        // span nested in a spaced ancestor carries the ancestor's value and
        // pushing it changes nothing — while a span that declares
        // `letter-spacing: normal` inside one carries 0 and must push it to get
        // the reset CSS promises. A `!= 0.0` guard would drop exactly that push.
        props.push(parley::style::StyleProperty::LetterSpacing(
            computed.letter_spacing,
        ));
        props.push(parley::style::StyleProperty::WordSpacing(
            computed.word_spacing,
        ));
        props.push(parley::style::StyleProperty::TextWrapMode(
            Self::text_wrap_mode(computed),
        ));
        props.push(parley::style::StyleProperty::WhiteSpaceCollapse(
            white_space_hanging(computed.white_space),
        ));
        props
    }

    /// The `vertical-align` shift (#724) a `vertical_align` value gives a text
    /// run, as an unscaled layout-pixel offset in rinch's Y-down convention:
    /// **positive moves the glyph down**, negative up.
    ///
    /// Three cases:
    ///
    /// - `Baseline` and the five keywords with no consumer yet
    ///   (`Top`/`TextTop`/`Middle`/`Bottom`/`TextBottom` — see
    ///   [`crate::computed_style::VerticalAlignValue`]'s doc) answer `0.0`.
    /// - `Sub`/`Super` answer a fixed fraction of `parent_font_size` — CSS 2.1
    ///   §10.8.1 says `sub`/`super` lower/raise the box "to the proper
    ///   position for subscripts/superscripts **of the parent's box**",
    ///   UA-defined, so there is no spec algorithm to match: Chrome instead
    ///   reads the OS/2 table's actual sub/superscript metrics of the
    ///   parent's font at its size, non-linearly (measured: at 24px Inter the
    ///   ratio to font-size is ~8% off the 16px ratio, almost certainly
    ///   device-pixel rounding inside Chrome's own layout, not a simpler
    ///   closed form). `SUB_RATIO`/`SUPER_RATIO` below are that *ratio*,
    ///   calibrated once against the one case this issue asks for — Chrome
    ///   153, the bundled Inter, a 16px parent (`crates/rinch-dom/tests/vertical_align_tests.rs`
    ///   has the measurement) — and applied linearly to any `parent_font_size`.
    ///   Exact only at that calibration point; elsewhere it is a documented
    ///   approximation, like the five keywords above.
    /// - A `<length>`/`<percentage>` resolves through
    ///   [`crate::computed_style::values::LengthPercentageValue::resolve`]
    ///   against **this element's own** `line-height` (its own box, not the
    ///   parent's — the one place `sub`/`super` and a length disagree about
    ///   whose metrics apply, both measured against Chrome 153 with Inter).
    ///   CSS's sign (positive raises) is the opposite of this function's, so
    ///   the resolved value is negated on the way out.
    pub(crate) fn vertical_align_shift_px(
        computed: &crate::computed_style::ComputedStyle,
        parent_font_size: f32,
    ) -> f32 {
        use crate::computed_style::VerticalAlignValue;
        // Calibration: Chrome 153, bundled Inter, 16px parent `font-size`
        // (line-height does not move it — verified at 16px/32px line-height,
        // same 16px font-size, identical shift both times).
        const SUB_RATIO: f32 = 67.0 / 256.0; // 4.1875px at a 16px parent
        const SUPER_RATIO: f32 = 405.0 / 1024.0; // 6.328125px at a 16px parent
        match computed.vertical_align {
            VerticalAlignValue::Sub => SUB_RATIO * parent_font_size,
            VerticalAlignValue::Super => -(SUPER_RATIO * parent_font_size),
            VerticalAlignValue::LengthPercentage(lp) => -lp.resolve(computed.line_height_px()),
            VerticalAlignValue::Baseline
            | VerticalAlignValue::Top
            | VerticalAlignValue::TextTop
            | VerticalAlignValue::Middle
            | VerticalAlignValue::Bottom
            | VerticalAlignValue::TextBottom => 0.0,
        }
    }

    /// Push the spans `owner` contributes over `start..end` — its background, if
    /// it has a visible one, and its wavy underline, if it has one.
    ///
    /// The one place an inline box becomes a span, so the ordinary `display:
    /// inline` arm and the split-inline bridge below cannot disagree about
    /// padding, radius, or which elements get a squiggle.
    #[allow(clippy::too_many_arguments)]
    fn push_inline_spans(
        nodes: &slab::Slab<Node>,
        owner: &Node,
        start: usize,
        end: usize,
        has_edges: bool,
        background_spans: &mut Vec<crate::node::InlineBackgroundSpan>,
        decoration_spans: &mut Vec<crate::node::InlineDecorationSpan>,
        vertical_align_spans: &mut Vec<crate::node::InlineVerticalAlignSpan>,
    ) {
        if end <= start {
            // An element with no text still has its edge boxes (#1426), and
            // draws its background and border over them.
            if has_edges && let Some(span) = inline_background_span(owner, start, end) {
                background_spans.push(span);
            }
            return;
        }
        // A wavy underline is not a Parley style, so it is recorded as a span and
        // painted as a path; `inline_style_props` correspondingly declines to push
        // the straight `Underline` for the same element.
        if owner.computed_style.text_decoration.is_wavy_underline() {
            decoration_spans.push(crate::node::InlineDecorationSpan {
                owner: owner.id,
                start,
                end,
                color: wavy_underline_color(&owner.computed_style),
            });
        }
        // `vertical-align` (#724): a post-layout glyph shift, not a Parley
        // style — recorded the same way the wavy underline above is. The
        // "parent's box" the spec shifts against (CSS 2.1 §10.8.1) is this
        // element's own DOM parent, which may sit above whatever enclosing
        // span called in once a split inline or a `display: contents` bridge
        // is in the way — so this reads `owner.parent` directly.
        if owner.computed_style.vertical_align != VerticalAlignValue::Baseline {
            let parent_font_size = owner
                .parent
                .and_then(|p| nodes.get(p))
                .map(|p| p.computed_style.font_size)
                .unwrap_or(owner.computed_style.font_size);
            let shift = Self::vertical_align_shift_px(&owner.computed_style, parent_font_size);
            if shift != 0.0 {
                vertical_align_spans.push(crate::node::InlineVerticalAlignSpan {
                    owner: owner.id,
                    start,
                    end,
                    shift_px: shift,
                });
            }
        }
        if let Some(span) = inline_background_span(owner, start, end) {
            background_spans.push(span);
        }
    }

    /// The `is_split_inline` DOM ancestors of `node_id` below `stop_at`,
    /// outermost first (#513).
    ///
    /// A split inline is flattened out of the box tree, so its run members are
    /// walked by the anonymous block box directly and the element itself is never
    /// walked — which is why anything it would have contributed to the line has to
    /// be reached from its descendants instead.
    fn split_inline_ancestors(
        nodes: &slab::Slab<Node>,
        node_id: usize,
        stop_at: Option<usize>,
    ) -> Vec<usize> {
        let mut chain = Vec::new();
        let mut cur = nodes.get(node_id).and_then(|n| n.parent);
        let cap = nodes.len() + 1;
        let mut hops = 0;
        while let Some(id) = cur {
            if Some(id) == stop_at || hops > cap {
                break;
            }
            hops += 1;
            let Some(node) = nodes.get(id) else { break };
            if node.is_split_inline() {
                chain.push(id);
            }
            cur = node.parent;
        }
        chain.reverse();
        chain
    }

    /// Recursively walk inline children, pushing text and style spans into the TreeBuilder.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn walk_inline_children<'a>(
        nodes: &'a slab::Slab<Node>,
        parent_id: usize,
        builder: &mut IfcText<'a>,
        child_positions: &mut Vec<(usize, LayoutResult)>,
        text_ranges: &mut Vec<crate::node::IfcTextRange>,
        background_spans: &mut Vec<crate::node::InlineBackgroundSpan>,
        decoration_spans: &mut Vec<crate::node::InlineDecorationSpan>,
        vertical_align_spans: &mut Vec<crate::node::InlineVerticalAlignSpan>,
        flat_pos: &mut usize,
        scale: f32,
        font_cx: &mut parley::FontContext,
        font_family_resolves: &std::cell::Cell<u64>,
        atomic: Option<&AtomicContributions<'_>>,
    ) {
        // As in `mark_inline_descendants`: an anonymous box's content is its
        // recorded run, not its (empty) `children` (#566). The two must walk
        // exactly the same set — that is [`Node::inline_flow_role`]'s contract
        // (#366) and it now includes where the set comes from.
        let children: Vec<usize> = nodes[parent_id].ifc_children().to_vec();

        // **A split inline's own background has to be reached from its members**
        // (#513). The element is flattened out of the box tree, so the arm below
        // that records an inline box's background never runs for it — and before
        // this, `<a style="background: cyan">text<div/>tail</a>` painted no cyan at
        // all, where the unsplit twin paints it and the pre-split engine painted it
        // over `text`. Measured: 436px before the split landed, 0 after, 761 on the
        // twin. That is a regression this change would otherwise introduce, not a
        // gap it inherits, which is why it is fixed here rather than filed.
        //
        // One span per ancestor per **contiguous stretch** of the run that lies
        // inside it, rather than one per member: two text members inside the same
        // `<a>` are one fragment and must be one rectangle, or the padding at the
        // seam is drawn twice. `open` holds the ancestors whose stretch is still
        // running, outermost first, with the flat offset each began at.
        //
        // What this does **not** do is model fragment identity: there is no inline
        // box per fragment, only a background rectangle per stretch, so per-fragment
        // *borders* are still unimplemented. See `paint::drawn_by_its_ifc`.
        let container = nodes[parent_id].parent;
        let bridging = nodes[parent_id].is_anonymous_block_box;
        let mut open: Vec<(usize, usize)> = Vec::new();
        // An out-of-flow box that is a child of the container itself joins
        // no run (#406), so an anonymous box's member list does not hold it.
        // The ones standing inside this run are met here, each before the
        // member it precedes (`Node::run_out_of_flow`, #632).
        let between: &[(usize, usize)] = if bridging {
            &nodes[parent_id].run_out_of_flow
        } else {
            &[]
        };

        for (member_index, child_id) in children.into_iter().enumerate() {
            for &(_, id) in between.iter().filter(|(at, _)| *at == member_index) {
                builder.note_out_of_flow(id);
            }
            let child = match nodes.get(child_id) {
                Some(c) => c,
                None => continue,
            };

            // Diff this member's split-inline ancestor chain against the open
            // stretches: close the ones it has left (emitting their span), open
            // the ones it has entered. `flat_pos` is the seam in both directions.
            if bridging {
                let chain = Self::split_inline_ancestors(nodes, child_id, container);
                let keep = open
                    .iter()
                    .zip(chain.iter())
                    .take_while(|((open_id, _), chain_id)| open_id == *chain_id)
                    .count();
                for (owner_id, start) in open.drain(keep..).rev() {
                    if let Some(owner) = nodes.get(owner_id) {
                        Self::push_inline_spans(
                            nodes,
                            owner,
                            start,
                            *flat_pos,
                            false,
                            background_spans,
                            decoration_spans,
                            vertical_align_spans,
                        );
                    }
                }
                for &owner_id in &chain[keep..] {
                    open.push((owner_id, *flat_pos));
                }
            }

            // The flow decision is [`Node::inline_flow_role`]'s — the same
            // classifier `mark_inline_descendants` consumes, which is what
            // keeps "mark exactly what this walk flows" a single rule (#366).
            // The first four arms are the dispatch *within* `Inline` (text,
            // `<br>`, inline element, atomic inline); the remaining arms map
            // one role each.
            let role = child.inline_flow_role();
            // **A member of an anonymous box did not inherit from that box.**
            // A run is grouped over the *flattened* child list (#568), so a
            // member can sit behind a `display: contents` wrapper that the run
            // took only part of — and a broken-up wrapper is never walked, so
            // the style its children inherited from it would simply be lost.
            // This is #574's rule ("a boxless element is still in the
            // inheritance chain") applied to the case where the wrapper does
            // not survive to push its own span.
            //
            // Text only, and that is not arbitrary: an element member pushes
            // its own span from its own computed style, which already carries
            // the inheritance. A text node has no arm that pushes one — it
            // takes whatever span encloses it — so it is the only member that
            // can lose it. Pushed only when it actually differs, for the same
            // reason #574 tests before pushing.
            //
            // The style comes from the member's **DOM parent**, not from the
            // text node itself: a text node renders with the style of the
            // element containing it, which is the wrapper here.
            let bridged_style = if nodes[parent_id].is_anonymous_block_box
                && matches!(child.kind, NodeKind::Text(_))
            {
                child
                    .parent
                    .filter(|&dom_parent| dom_parent != parent_id)
                    .and_then(|dom_parent| nodes.get(dom_parent))
                    .filter(|owner| {
                        !Self::same_inline_text_style(
                            &owner.computed_style,
                            &nodes[parent_id].computed_style,
                        )
                    })
                    .map(|owner| &owner.computed_style)
            } else {
                None
            };
            let bridged = bridged_style.is_some();
            if let Some(owner_style) = bridged_style {
                builder.push_span(Self::inline_style_props(
                    owner_style,
                    &nodes[parent_id].computed_style.font_family,
                    scale,
                    font_cx,
                    font_family_resolves,
                ));
            }
            match &child.kind {
                NodeKind::Text(text_data) => {
                    if !text_data.content.is_empty() {
                        let start = *flat_pos;
                        // Apply text-transform from parent's computed style
                        let parent_transform = &nodes[parent_id].computed_style.text_transform;
                        let raw: std::borrow::Cow<'a, str> =
                            match parent_transform.apply(&text_data.content) {
                                Some(transformed) => transformed.into(),
                                None => text_data.content.as_str().into(),
                            };
                        let dom_text_len = raw.len();
                        // Collapsed (#1180), or verbatim with a tab as a
                        // space widened to its stop (#1122), by the
                        // `white-space` of the element the text is in: its
                        // DOM parent, which inside an anonymous box is not
                        // `parent_id` (#1192).
                        let mode = SpaceCollapse::of(
                            child
                                .parent
                                .and_then(|dom_parent| nodes.get(dom_parent))
                                .unwrap_or(&nodes[parent_id])
                                .computed_style
                                .white_space,
                        );
                        let (flat_len, offset_map) = builder.push_text_node(raw, mode);
                        *flat_pos += flat_len;
                        debug_assert_eq!(*flat_pos, builder.len());
                        text_ranges.push(crate::node::IfcTextRange {
                            flat_start: start,
                            flat_end: *flat_pos,
                            node_id: child_id,
                            node_offset: 0,
                            is_br: false,
                            dom_text_len,
                            offset_map,
                            color: text_color(nodes, child_id),
                        });
                        // Record position placeholder — actual position comes from layout
                        child_positions.push((child_id, LayoutResult::default()));
                    }
                }
                NodeKind::Element(_)
                    if role == InlineFlowRole::Inline
                        && child.display_mode == DisplayMode::Inline
                        && child.tag() == Some("br") =>
                {
                    // <br> elements insert a hard line break.
                    let start = *flat_pos;
                    builder.push_forced_break();
                    *flat_pos += 1;
                    text_ranges.push(crate::node::IfcTextRange {
                        flat_start: start,
                        flat_end: *flat_pos,
                        node_id: child_id,
                        node_offset: 0,
                        is_br: true,
                        dom_text_len: 1,
                        offset_map: Vec::new(),
                        color: peniko::Color::BLACK,
                    });
                    child_positions.push((child_id, LayoutResult::default()));
                }
                NodeKind::Element(_)
                    if role == InlineFlowRole::Inline
                        && child.display_mode == DisplayMode::Inline =>
                {
                    // Push style span for inline element using typed ComputedStyle
                    let child_computed = &child.computed_style;

                    // Record background span start position
                    let bg_start = *flat_pos;
                    let has_bg = child_computed.background_color().is_some();
                    // #724: whether this element needs its own vertical-align
                    // span over the stretch it owns.
                    let has_valign = child_computed.vertical_align != VerticalAlignValue::Baseline;

                    builder.push_span(Self::inline_style_props(
                        child_computed,
                        &nodes[parent_id].computed_style.font_family,
                        scale,
                        font_cx,
                        font_family_resolves,
                    ));
                    child_positions.push((child_id, LayoutResult::default()));

                    // #1426: the room its inline-start margin, border and
                    // padding take before its content, and the end ones after
                    // it — as zero-height boxes, which hold no text.
                    // The direction is read only when the two sides differ.
                    let (left, right) = inline_edge_widths(child_computed, true);
                    let (edge_start, edge_end) = if left != right && !element_is_ltr(child) {
                        (right, left)
                    } else {
                        (left, right)
                    };
                    if edge_start > 0.0 {
                        builder.push_edge_box(child_id, false, edge_start * scale);
                    }

                    // Recurse into inline element's children
                    Self::walk_inline_children(
                        nodes,
                        child_id,
                        builder,
                        child_positions,
                        text_ranges,
                        background_spans,
                        decoration_spans,
                        vertical_align_spans,
                        flat_pos,
                        scale,
                        font_cx,
                        font_family_resolves,
                        atomic,
                    );

                    if edge_end > 0.0 {
                        builder.push_edge_box(child_id, true, edge_end * scale);
                    }
                    builder.pop_span();

                    // Record background and/or vertical-align span if the
                    // inline element has one. Through the shared helper, so
                    // this and the split-inline bridge below cannot drift
                    // about padding, radius, or the vertical shift. An empty
                    // element with edge boxes still draws its background and
                    // border over them (#1426).
                    let has_edges = edge_start > 0.0 || edge_end > 0.0;
                    if has_bg || has_valign || has_inline_border(child_computed) {
                        Self::push_inline_spans(
                            nodes,
                            child,
                            bg_start,
                            *flat_pos,
                            has_edges,
                            background_spans,
                            decoration_spans,
                            vertical_align_spans,
                        );
                    }
                }
                NodeKind::Element(_) if role == InlineFlowRole::Inline => {
                    // An **atomic inline** — every remaining `Inline`-role
                    // element ([`DisplayMode::is_atomic_inline`]): measure via
                    // Taffy first, then embed as an InlineBox. Which formatting
                    // context its interior gets is `DisplayValue::to_taffy`'s
                    // business, not this arm's, so `inline-grid` needed nothing
                    // here (#607).
                    //
                    // The paint layout lines the box up at the size it has.
                    // A *measure* lines an `auto`-width one up at what it
                    // contributes to the width being asked about (#1476).
                    let (width, height) = match atomic {
                        Some(atomic) => atomic.size(child_id, child),
                        None => (child.layout.width, child.layout.height),
                    };
                    // Its baseline (#663), as its compute last laid it out.
                    // Lined up at another height than that one (a measure's
                    // contribution), a last-line baseline keeps its distance
                    // from the bottom and a first-line one from the top.
                    let baseline = child.atomic_baseline.map(|b| {
                        let first = matches!(
                            child.computed_style.display,
                            crate::computed_style::DisplayValue::InlineFlex
                                | crate::computed_style::DisplayValue::InlineGrid
                        );
                        if first {
                            b
                        } else {
                            b + (height - child.layout.height)
                        }
                    });
                    builder.push_inline_box(parley::InlineBox {
                        id: child_id as u64,
                        index: 0, // will be set by builder
                        width: width * scale,
                        height: height * scale,
                        kind: parley::InlineBoxKind::InFlow,
                        baseline: baseline.map(|b| b * scale),
                        vertical_align: atomic_vertical_align(&child.computed_style, scale),
                    });
                    child_positions.push((child_id, LayoutResult::default()));
                }
                NodeKind::Element(_)
                    if role == InlineFlowRole::Contents
                        && Self::contents_is_inline_transparent(nodes, child_id) =>
                {
                    // `display:contents` generates no box — it is transparent.
                    // Recurse without pushing a style span so its inline
                    // descendants flow into this IFC in document order (#61).
                    //
                    // A wrapper holding block-level content is *not* transparent
                    // to the IFC: it falls through to the `_` arm below and
                    // breaks the inline flow, exactly as the block it wraps
                    // would if it were a direct child.
                    // A boxless element is still in the inheritance chain, so
                    // its children owe it the same text style a `display:
                    // inline` box's children owe theirs (#574) — pushed only
                    // when it actually changes something, since the wrapper's
                    // values are its parent's unless it declared otherwise and
                    // rsx emits one of these for every `if`/`match`/`for`.
                    let styled = !Self::same_inline_text_style(
                        &child.computed_style,
                        &nodes[parent_id].computed_style,
                    );
                    if styled {
                        builder.push_span(Self::inline_style_props(
                            &child.computed_style,
                            &nodes[parent_id].computed_style.font_family,
                            scale,
                            font_cx,
                            font_family_resolves,
                        ));
                    }
                    Self::walk_inline_children(
                        nodes,
                        child_id,
                        builder,
                        child_positions,
                        text_ranges,
                        background_spans,
                        decoration_spans,
                        vertical_align_spans,
                        flat_pos,
                        scale,
                        font_cx,
                        font_family_resolves,
                        atomic,
                    );
                    if styled {
                        builder.pop_span();
                    }
                }
                NodeKind::Element(_)
                    if matches!(role, InlineFlowRole::OutOfFlow | InlineFlowRole::NoBox) =>
                {
                    // An out-of-flow box does not break an inline formatting
                    // context (CSS 2.1 §9.4.2): its inline siblings carry on
                    // across it, on the same line. It is laid out by Taffy
                    // from its containing block, not by this IFC — walk past
                    // it (#406). Falling into the `break` arm below split
                    // `a<abs/>b` so that `b` never reached Parley at all:
                    // `mark_inline_descendants` had already detached and
                    // marked it, so it was neither laid out here nor by Taffy.
                    //
                    // A `display: none` child generates no box and is walked
                    // past the same way (#366). It used to fall into the
                    // `break` arm — while the marking pass detaches it and
                    // *continues* — so inline content after it was marked but
                    // never flowed: `<a>text<none/>tail</a>` silently dropped
                    // `tail`, the exact divergence class this classifier
                    // exists to kill.
                    //
                    // Classifying by role keeps this walk exactly aligned
                    // with `mark_inline_descendants`: an *opaque*
                    // `display: contents` wrapper that also declares
                    // `position: absolute` is boxless (Stylo does not
                    // blockify contents, so `is_out_of_flow()` answers true
                    // for it), its role is `Contents` — display before
                    // position — and it `break`s below, matching where the
                    // marking pass stops. Walking past it instead would flow
                    // text the mark left attached — a double draw.
                    //
                    // Its place in the content is remembered, though: that
                    // is its static position (#632).
                    if role == InlineFlowRole::OutOfFlow {
                        builder.note_out_of_flow(child_id);
                    }
                }
                NodeKind::Comment(_) => {
                    // Skip comments in inline layout
                }
                _ => {
                    // **Reachable in a detached subtree, same route and same
                    // witnesses as `mark_inline_descendants`' `InFlowBlock` arm**
                    // (#615). Read the mechanism there; the two arms are one rule
                    // and must stay one rule, and the two fixtures named there
                    // assert both halves of it on one document — the mark leaves
                    // `tail` unmarked, and this walk leaves it out of the line.
                    //
                    // Note which mutant each half kills: making the *mark*
                    // `continue` stamps `ifc_root` on `tail`, and making *this*
                    // arm `continue` flows `tail` into the line ahead of the
                    // block. Neither is caught by the other's assertion, which is
                    // why both are asserted.
                    //
                    // Since #1069 the layout pass no longer shapes a detached
                    // root, so the marking arm is still reached by the layout
                    // pass and this one only through
                    // `RinchDocument::shape_ifc_root_for_tests`, which the
                    // witnesses call after checking the pass declined.
                    //
                    // An in-flow block-level child — or the opaque
                    // `display: contents` wrapper standing for one — breaks
                    // the inline flow: stop here, exactly where
                    // `mark_inline_descendants` stops marking (#366).
                    break;
                }
            }
            if bridged {
                builder.pop_span();
            }
        }

        // (Not reached when the walk stopped at a block above; a run holds
        // none.)
        let members = nodes[parent_id].ifc_children().len();
        for &(_, id) in between.iter().filter(|(at, _)| *at >= members) {
            builder.note_out_of_flow(id);
        }

        // Close whatever is still open at the end of the run.
        for (owner_id, start) in open.drain(..).rev() {
            if let Some(owner) = nodes.get(owner_id) {
                Self::push_inline_spans(
                    nodes,
                    owner,
                    start,
                    *flat_pos,
                    false,
                    background_spans,
                    decoration_spans,
                    vertical_align_spans,
                );
            }
        }
    }
}

// ── IFC content signatures ──────────────────────────────────────────────────

/// FxHash's mixing step: fast, not cryptographic, and good enough to tell one
/// IFC's content from another's. Collisions would cost a stale measure, so
/// each member's hash is finished with a full avalanche (`finish_member`)
/// before the members are summed.
#[derive(Default)]
struct SigHasher(u64);

impl SigHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

impl std::hash::Hasher for SigHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        let (chunks, rest) = bytes.as_chunks::<8>();
        for c in chunks {
            self.add(u64::from_le_bytes(*c));
        }
        if !rest.is_empty() {
            let mut b = [0u8; 8];
            b[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(b));
        }
    }
    fn write_u8(&mut self, i: u8) {
        self.add(i as u64);
    }
    fn write_u32(&mut self, i: u32) {
        self.add(i as u64);
    }
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
    fn write_isize(&mut self, i: isize) {
        self.add(i as u64);
    }
}

/// splitmix64's finaliser, so that summing member hashes (which makes the
/// signature independent of slab order) cannot cancel structured inputs out.
#[inline]
fn finish_member(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

impl RinchDocument {
    /// The content signature of every IFC root in the document, keyed by root.
    ///
    /// A root's signature hashes everything its measure and its paint layout
    /// are built from **other than typography and the available width**:
    /// which nodes are its members (`ifc_root == Some(root)`), under which
    /// parent and at which index (so a reorder moves it), what each one is —
    /// its text, its tag, its `display` mode, its `position`, whether it is
    /// generated content — and, for an atomic inline, the size
    /// `compute_inline_block_layouts` just gave it, which the line breaks
    /// around. An **out-of-flow box** among those children is hashed the
    /// same way though it is no member: the lines keep its static position
    /// (#632), so they are built again when one comes, goes or moves. The root's own `estimated_height` (a virtualized, collapsed
    /// block) is folded in too.
    ///
    /// Typography is deliberately **not** in it: the cascade compares each
    /// re-cascaded node's text inputs and drops the IFC of the one that
    /// changed (`ComputedStyle::same_text_layout_inputs`), so hashing styles
    /// here would only repeat that at the cost of a style hash per member per
    /// structural pass. The width is the key of each cached size.
    ///
    /// O(document): one pass over the slab and a hash of every member's text.
    /// That is what the structural pass costs anyway; what this replaces was
    /// a Parley shape of every root.
    ///
    /// With a `scope`, only the roots of the scope's containers — each
    /// container and the anonymous boxes it now holds — are signed, from the
    /// parents their members can have: those same nodes and the transparent
    /// nodes of the containers' regions. A root's members are all in its
    /// container's region, so each signature is the one the whole-document
    /// walk computes.
    pub(crate) fn ifc_signatures(
        &self,
        scope: Option<&crate::ifc_scope::IfcScope>,
    ) -> HashMap<usize, u64> {
        use std::hash::{Hash, Hasher};
        let nodes = &self.tree.nodes;
        let mut sigs: HashMap<usize, u64> = HashMap::new();
        let (parents, roots): (Vec<usize>, Option<std::collections::HashSet<usize>>) = match scope {
            None => (nodes.iter().map(|(id, _)| id).collect(), None),
            Some(scope) => {
                let mut roots = std::collections::HashSet::new();
                let mut parents = Vec::new();
                for &c in &scope.containers {
                    roots.insert(c);
                    parents.push(c);
                    if let Some(cn) = nodes.get(c) {
                        roots.extend(cn.run_boxes.iter().copied());
                        parents.extend(cn.run_boxes.iter().copied());
                    }
                }
                parents.extend(scope.transparent_region());
                (parents, Some(roots))
            }
        };
        for parent_id in parents {
            let Some(parent) = nodes.get(parent_id) else {
                continue;
            };
            let children = parent.ifc_children();
            // The root of the nearest member before the child at hand, and
            // (found once, when asked) of the first member at all.
            let mut near: Option<usize> = None;
            let mut first: Option<Option<usize>> = None;
            for (index, &child_id) in children.iter().enumerate() {
                let Some(child) = nodes.get(child_id) else {
                    continue;
                };
                let root = match child.ifc_root {
                    Some(root) => {
                        near = Some(root);
                        root
                    }
                    // An out-of-flow box is no member, but the lines hold
                    // its static position (#632): one that appears among a
                    // root's content, leaves it or moves in it changes what
                    // those lines must remember. Signed with the root of
                    // the member before it (the first one, when none is).
                    // (`is_out_of_flow` first: it is one compare, and this
                    // arm is asked of every block child of every parent.)
                    None if child.is_out_of_flow()
                        && child.inline_flow_role() == InlineFlowRole::OutOfFlow =>
                    {
                        let found = near.or_else(|| {
                            *first.get_or_insert_with(|| {
                                children
                                    .iter()
                                    .find_map(|&c| nodes.get(c).and_then(|n| n.ifc_root))
                            })
                        });
                        match found {
                            Some(root) => root,
                            None => continue,
                        }
                    }
                    None => continue,
                };
                if roots.as_ref().is_some_and(|r| !r.contains(&root)) {
                    continue;
                }
                let mut h = SigHasher::default();
                child_id.hash(&mut h);
                parent_id.hash(&mut h);
                index.hash(&mut h);
                match &child.kind {
                    NodeKind::Text(t) => {
                        0u8.hash(&mut h);
                        t.content.hash(&mut h);
                    }
                    NodeKind::Element(_) => {
                        1u8.hash(&mut h);
                        child.tag().unwrap_or("").hash(&mut h);
                    }
                    _ => 2u8.hash(&mut h),
                }
                std::mem::discriminant(&child.display_mode).hash(&mut h);
                std::mem::discriminant(&child.computed_style.position).hash(&mut h);
                child.is_pseudo_element.hash(&mut h);
                if child.display_mode.is_atomic_inline() {
                    child.layout.width.to_bits().hash(&mut h);
                    child.layout.height.to_bits().hash(&mut h);
                }
                let member = finish_member(h.finish());
                let sig = sigs.entry(root).or_insert(0);
                *sig = sig.wrapping_add(member);
            }
        }
        for (&root, sig) in sigs.iter_mut() {
            let collapsed = nodes.get(root).and_then(|n| n.estimated_height);
            let mut h = SigHasher::default();
            collapsed.map(f32::to_bits).hash(&mut h);
            *sig = sig.wrapping_add(finish_member(h.finish() ^ 0x9e37_79b9_7f4a_7c15));
        }
        sigs
    }

    /// Decide, after a structural pass, which IFC roots the change reached.
    ///
    /// A root whose content signature ([`Self::ifc_signatures`]) is the one it
    /// had at the previous structural pass keeps its cached measures and its
    /// paint layout: nothing it is built from moved. A root whose signature
    /// moved — a child appended, removed or reordered anywhere inside it, a
    /// member's text or `display` changed, an atomic inline in it resized —
    /// or that is new, loses both, and is marked dirty in Taffy so the next
    /// compute measures it at all. A node that has stopped being a root loses
    /// its entry.
    ///
    /// This is what lets the structural pass stop clearing the whole measure
    /// cache, which re-shaped every paragraph in a document to measure one
    /// appended row. It is also a second, independent net under the mutation
    /// verbs' own invalidations (`invalidate_parent_ifc` and friends), which
    /// only reach the *parent* of a change: text appended to a `<span>` inside
    /// a paragraph invalidated the span, not the paragraph, and the paragraph
    /// kept painting its old line.
    ///
    /// With a `scope`, only the scope's roots are signed and only the scope's
    /// nodes can lose an entry or a layout: a root outside it was not reached
    /// by the change, keeps its signature and so keeps both.
    pub(crate) fn refresh_ifc_signatures(&mut self, scope: Option<&crate::ifc_scope::IfcScope>) {
        let sigs = self.ifc_signatures(scope);
        // Roots signed at an earlier pass that are roots no longer. Their
        // `text_layout` alone cannot say so: a move verb has usually dropped
        // it already (`invalidate_ifc_left_by`, on the IFC its node left), so
        // a block whose only span moved to another parent held no layout by
        // now, and its glyphs stayed on screen (#1064). The signature is what
        // survives every invalidation until this pass.
        let mut unrooted: Vec<usize> = Vec::new();
        match scope {
            None => self.tree.ifc_measure_cache.retain(|root, entry| {
                let keep = sigs.contains_key(root);
                if !keep && entry.signature.is_some() {
                    unrooted.push(*root);
                }
                keep
            }),
            Some(scope) => {
                for &id in &scope.rootable {
                    if !sigs.contains_key(&id)
                        && let Some(entry) = self.tree.ifc_measure_cache.remove(&id)
                        && entry.signature.is_some()
                    {
                        unrooted.push(id);
                    }
                }
            }
        }
        // One still holding a layout is pushed below, with the layout.
        for id in unrooted {
            if self
                .tree
                .nodes
                .get(id)
                .is_some_and(|n| n.text_layout.is_none())
            {
                self.tree.paint_dirty_nodes.push(id);
            }
        }
        // A node that is no longer an IFC root keeps no layout from when it
        // was one. `text_layout` is written for IFC roots only (a text leaf's
        // shaped text lives in `cached_text_parley`), and several readers take
        // its presence to mean "this is a root" — caret and selection rects,
        // layer bounds, and the ancestor walk in `invalidate_ifc_for_node`,
        // which would stop at the stale node instead of the real root.
        //
        // Its glyphs leave the screen with it (a paragraph whose only span
        // became `display: none`), so it is paint-dirty too — in both arms:
        // the scoped one is the path a `display` flip takes now.
        match scope {
            None => {
                for (id, node) in self.tree.nodes.iter_mut() {
                    if node.text_layout.is_some() && !sigs.contains_key(&id) {
                        node.text_layout = None;
                        self.tree.paint_dirty_nodes.push(id);
                    }
                }
            }
            Some(scope) => {
                for &id in &scope.rootable {
                    if let Some(node) = self.tree.nodes.get_mut(id)
                        && node.text_layout.is_some()
                        && !sigs.contains_key(&id)
                    {
                        node.text_layout = None;
                        self.tree.paint_dirty_nodes.push(id);
                    }
                }
            }
        }
        let mut changed: u64 = 0;
        let mut atomic_changed = false;
        for (root, sig) in sigs {
            let entry = self.tree.ifc_measure_cache.entry(root).or_default();
            if entry.signature == Some(sig) && !entry.stale {
                continue;
            }
            let seen_before = entry.signature.is_some();
            entry.signature = Some(sig);
            entry.stale = false;
            entry.sizes.clear();
            changed += 1;
            let Some(node) = self.tree.nodes.get_mut(root) else {
                continue;
            };
            node.text_layout = None;
            // An atomic inline is sized by its own detached compute, which
            // `compute_inline_block_layouts` ran *earlier in this pass* —
            // against the cached measures this loop is only now dropping, and
            // with no signal that its content had changed. So it is queued
            // for the same re-measure a text edit inside it gets, which runs
            // below once every changed root has been seen. A root seen for
            // the first time is not: it has no earlier measure to be stale
            // against, and re-measuring every new chip would size each one
            // twice on a first layout.
            if node.display_mode.is_atomic_inline() {
                if seen_before {
                    self.tree.dirty_atomic_inlines.insert(root);
                    atomic_changed = true;
                }
                continue;
            }
            if let Some(taffy_id) = node.taffy_id {
                let _ = self.tree.taffy.mark_dirty(taffy_id);
            }
            self.mark_ifc_measure_dirty(root);
            // The same holds one level out: an atomic inline that *contains*
            // this root — a paragraph in an `inline-block`, a flex item's IFC in
            // an `inline-flex` — was sized earlier in the pass, against the
            // measure this root had cached in Taffy before its content moved.
            // Queue every one above it. (Found by the scoped pass's random
            // differential, on the whole-document pass: a chip turned
            // `inline-block` inside a flex item of an `inline-flex` left the
            // `inline-flex` at the width it had without the chip.)
            if seen_before {
                let mut cur = self.tree.nodes.get(root).and_then(|n| n.parent);
                while let Some(a) = cur {
                    let Some(an) = self.tree.nodes.get(a) else {
                        break;
                    };
                    if an.display_mode.is_atomic_inline() {
                        self.tree.dirty_atomic_inlines.insert(a);
                        atomic_changed = true;
                    }
                    cur = an.parent;
                }
            }
        }
        self.tree
            .perf
            .add(crate::perf::Counter::IfcSignatureChanges, changed);
        if atomic_changed {
            self.remeasure_dirty_atomic_inlines();
        }
    }
}

/// `tab-size`'s initial value. Stylo parses `tab-size` for Gecko only, so
/// no other value reaches rinch-dom (#1122).
pub(crate) const TAB_SIZE: f32 = 8.0;

/// How close a tab's end has to be to its stop to count as there.
const TAB_STOP_TOLERANCE: f32 = 0.01;

/// The most builds [`RinchDocument::build_inline_layout`] makes of a
/// paragraph to put its tabs at their stops. A tab's width moves what follows
/// it on its line, and on a wrapping line can move the line break, which
/// moves the tabs after it; a line of tabs that does not wrap is settled by
/// the second build. Each build settles one more moved soft wrap, so a long
/// soft-wrapped `pre-wrap` paragraph can stop here with some tabs off their
/// stops, 3.8–5px from Chrome's, from about its fifth line on: it keeps the
/// last build (Refs #1593).
const MAX_TAB_BUILDS: u32 = 4;

/// The tab stops of one block container (css-text-3 §4.2), and how far each
/// preserved tab of a laid-out paragraph is from ending at its stop.
///
/// Measured in Chrome 155 (`tests/tab_stop_1122_tests.rs`): the stops are
/// [`TAB_SIZE`] times the advance of a space in the block's own font, its
/// `letter-spacing` and `word-spacing` included — not the font or spacing of
/// the element the tab is in — from the line's start edge, the tab's start
/// measured in logical order before `text-align` moves the line; the tab's
/// end is the first stop strictly past its start, or the one after it when
/// that is less than half a space (the font's own, unspaced) away.
pub(crate) struct TabStops {
    interval: f32,
    min_width: f32,
}

impl TabStops {
    /// The stops of a block of `style` (its `letter_spacing` and
    /// `word_spacing` in CSS px; `style` resolves them at `scale`).
    pub(crate) fn of_block(
        font_cx: &mut parley::FontContext,
        layout_cx: &mut parley::LayoutContext<Brush>,
        style: &parley::style::TextStyle<'_, '_, Brush>,
        letter_spacing: f32,
        word_spacing: f32,
        scale: f32,
    ) -> Self {
        let mut builder = layout_cx.tree_builder(font_cx, scale, true, style);
        // parley 0.12 made `white-space-collapse` a style (`TextStyle::white_space_collapse`),
        // `Preserve` by default and set by nothing in rinch: the builder collapses nothing.
        builder.push_text(" ");
        let (mut layout, _) = builder.build();
        layout.break_all_lines(None);
        let space: f32 = layout
            .lines()
            .flat_map(|line| line.runs().collect::<Vec<_>>())
            .flat_map(|run| run.clusters().map(|c| c.advance()).collect::<Vec<_>>())
            .sum();
        Self {
            interval: TAB_SIZE * space,
            min_width: ((space - (letter_spacing + word_spacing) * scale) / 2.0).max(0.0),
        }
    }

    /// How wide a tab starting `x` from its line's start edge is.
    pub(crate) fn width_at(&self, x: f32) -> f32 {
        if !self.interval.is_finite() || self.interval <= 0.0 {
            return 0.0;
        }
        let mut w = self.interval - x.rem_euclid(self.interval);
        if w < self.min_width {
            w += self.interval;
        }
        w
    }

    /// For each tab of `layout` (`tabs`: their flat offsets, ascending), how
    /// much wider it must be to end at its stop. A tab's start counts the
    /// widths the tabs before it on its line are to take.
    pub(crate) fn deltas(&self, layout: &parley::Layout<Brush>, tabs: &[usize]) -> Vec<f32> {
        use parley::PositionedLayoutItem;
        let mut deltas = vec![0.0f32; tabs.len()];
        /// (byte, 0 = inline box / 1 = cluster, order), advance, tab index.
        type Unit = ((usize, u8, usize), f32, Option<usize>);
        let mut units: Vec<Unit> = Vec::new();
        for line in layout.lines() {
            units.clear();
            for run in line.runs() {
                for c in run.clusters() {
                    let start = c.text_range().start;
                    let n = units.len();
                    units.push(((start, 1, n), c.advance(), tabs.binary_search(&start).ok()));
                }
            }
            for item in line.items() {
                if let PositionedLayoutItem::InlineBox(b) = item {
                    let Some(ib) = layout.inline_boxes().find(|ib| ib.id == b.id) else {
                        continue;
                    };
                    let advance = if matches!(ib.kind, parley::InlineBoxKind::InFlow) {
                        ib.width
                    } else {
                        0.0
                    };
                    let n = units.len();
                    units.push(((ib.index, 0, n), advance, None));
                }
            }
            // A box at byte `i` comes before the text at `i`.
            units.sort_unstable_by_key(|u| u.0);
            let mut x = 0.0f32;
            for &(_, advance, tab) in &units {
                if let Some(t) = tab {
                    let delta = self.width_at(x) - advance;
                    deltas[t] = delta;
                    x += delta;
                }
                x += advance;
            }
        }
        deltas
    }
}

#[cfg(test)]
mod atomic_leaf_layout_tests {
    use crate::RinchDocument;
    use rinch_core::dom::DomDocument;

    /// The keying rule of `NodeTree::atomic_leaf_layouts` (#904): each layout
    /// the atomic compute's text measure builds is filed under the width it
    /// was wrapped at, as the root compute files its own. Nothing behavioural
    /// pins it — `copy_cached_text_layouts` falls back to the `u32::MAX` entry,
    /// and the last measure Taffy makes is usually the final one — so filing
    /// every layout under `u32::MAX` left every other fixture green (#904's
    /// review, M6). Here the final width is a min-content grid column, 45.3px
    /// and not a whole pixel, which the box's unrounded width must name.
    #[test]
    fn a_leaf_layout_is_filed_under_the_width_it_was_wrapped_at() {
        let mut doc = RinchDocument::new();
        doc.load_css(
            "body { font-family: sans-serif; font-size: 16px; line-height: 20px; }
             .c { display: inline-grid; grid-template-columns: min-content; }",
        );
        let body = doc.body();
        let c = doc.create_element("span");
        doc.set_attribute(c, "class", "c");
        let t = doc.create_text("a long chip label text that wraps");
        doc.append_child(c, t);
        doc.append_child(body, c);
        doc.resolve_layout(800.0, 600.0);
        doc.resolve_layout(800.0, 600.0);

        let leaf_taffy = doc.tree.nodes[t.0]
            .taffy_id
            .expect("the leaf has a Taffy node");
        let final_width = doc.tree.taffy.unrounded_layout(leaf_taffy).size.width;
        assert!(
            final_width.fract() != 0.0,
            "counter-oracle: the column is not a whole pixel ({final_width})"
        );

        // Measure the atomic inline again, and read what it filed before a
        // layout pass hands it on.
        let _ = doc.tree.taffy.mark_dirty(leaf_taffy);
        doc.compute_inline_block_layouts();
        let keys: Vec<u32> = doc
            .tree
            .atomic_leaf_layouts
            .keys()
            .filter(|(id, _)| *id == t.0)
            .map(|&(_, bits)| bits)
            .collect();
        assert!(
            keys.contains(&final_width.to_bits()),
            "a layout filed under the final width {final_width}; keys were {:?}",
            keys.iter().map(|&b| f32::from_bits(b)).collect::<Vec<_>>()
        );
        let n = doc.tree.nodes[t.0].layout.height;
        let cached = doc.tree.nodes[t.0].cached_text_parley.as_ref().unwrap();
        assert_eq!(
            cached.height(),
            n,
            "the painted layout is the one laid out for the box, not the unwrapped line"
        );
    }
}
