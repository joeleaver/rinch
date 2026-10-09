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

/// What `build_ellipsis_layout` rebuilds from (#1091).
pub(crate) enum EllipsisSource<'a> {
    /// The original layout, whose lines are kept and cut one by one.
    Lines(&'a InlineLayout),
    /// The whole text, cut to one prefix (a `nowrap`/`pre` root whose inline
    /// content the flat rebuild cannot represent — the behaviour before
    /// #1091).
    Whole(&'a str),
}

/// The IFC root's own text style, as the flat ellipsis rebuild applies it to
/// every character.
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
    alignment: parley::layout::Alignment,
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

/// The background span inline element `owner` draws over `start..end`, from
/// its style as it is now; `None` when it has no background to draw.
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
    if !has_inline_background(cs) {
        return None;
    }
    Some(crate::node::InlineBackgroundSpan {
        owner: owner.id,
        start,
        end,
        color: cs.background_color()?,
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
            alignment: cs.text_align.to_parley(),
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
}

impl HangStats {
    /// Sum `other` into these, field by field.
    pub(crate) fn add(&mut self, other: HangStats) {
        self.passes += other.passes;
        self.lines += other.lines;
        self.phantom_rebreaks += other.phantom_rebreaks;
        self.unglue_rebreaks += other.unglue_rebreaks;
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
    }
}

/// Break `layout` into lines at `max_width`, hanging preserved spaces at a soft
/// wrap the way CSS does.
///
/// CSS Text 3 §4.1.3: with `white-space: pre-wrap`, a sequence of preserved
/// spaces and tabs at the end of a line **hangs** — it may overflow the line
/// and never starts the next one. Parley 0.11.1 hangs only the *first* space
/// that overflows and then commits the line, so
///
/// - the rest of the sequence starts the next line (`"ab  "` at the width of
///   `"ab"` puts the second space on a line of its own), and
/// - when nothing but the hanging space is left (`"ab "` at the width of
///   `"ab"`), the breaker still emits one more, empty line, which counts in the
///   layout's height.
///
/// Either way the text is one line taller than it is. That is what made an
/// editor list item's paragraph two lines tall while its text ended in a space:
/// the flex item was sized to the text without its trailing space and then laid
/// out at that width. Upstream parley reworked hanging (#790, #785, unreleased
/// at 0.11.1); this puts the sequence back on the line it hangs from.
///
/// A line needs it when parley ended it at the hang ([`BreakReason::Regular`])
/// and what follows is a space or tab, a newline, or the end of the text. Such
/// a line is broken again — [`BreakLines::revert_to`] the state it started
/// from — with just enough room for its content, the following spaces and
/// tabs, and 0.01px (no following word fits in that), and then
/// [`set_prior_line_width`] puts its alignment width back at `max_width`, so
/// `text-align: right`/`center` still line the text up against the box edge
/// with the spaces hanging past it. The run's last space is left out of that
/// room when anything but a newline follows it, so it is parley that hangs it
/// and commits the line: widened for the whole run, the next thing to overflow
/// would be whatever follows, and parley hangs an NBSP as readily as a space.
/// NBSP does not hang (it is not white space to §4.1.3): it stays glued to the
/// word it belongs to, and after a hung run it starts the next line.
///
/// **Linear in the paragraph.** The common case — no line needs it — costs the
/// one ordinary break and a scan of the lines. Otherwise the paragraph is
/// broken once more, line by line, and each line that needs it is broken a
/// second time on the spot, so the whole thing is at most two breaks of the
/// paragraph plus one extra break per fixed line. (The first version restarted
/// the break from line 0 for every line it fixed, which is quadratic: seconds
/// for a few thousand double-spaced lines.) The breaker gives no access to a
/// line's text while it holds the layout, so where each line ends is followed
/// through a table of the paragraph's clusters and inline boxes in logical
/// order, advanced by each committed line's advance ([`line_end`]). If that
/// ever disagrees with the breaker, fixing stops for the rest of the paragraph
/// and it is broken exactly as parley breaks it.
///
/// Collapsible white space (`normal`, `nowrap`) has no trailing spaces left to
/// hang, and an unconstrained layout has nothing to wrap, so both are broken
/// exactly as before.
///
/// Whatever the white space, a line parley ended by hanging a no-break space
/// — which it hangs like a space, though UAX #14 allows no break after one —
/// is broken again where CSS breaks it ([`unglue`], #1218), in the same pass;
/// and the pass then hangs preserved spaces on every line, since moving an
/// NBSP can leave a line ending in spaces parley did not hang.
///
/// And whatever the white space, a paragraph that ends in the empty line parley
/// commits after an overflowing inline box or a final forced break
/// ([`phantom_last_line`], #1050, #1172) is broken once more, the same way,
/// without it.
///
/// [`BreakReason::Regular`]: parley::layout::BreakReason::Regular
/// [`BreakLines::revert_to`]: parley::layout::BreakLines::revert_to
/// [`set_prior_line_width`]: parley::layout::BreakLines::set_prior_line_width
pub(crate) fn break_lines_hanging_spaces(
    layout: &mut parley::Layout<Brush>,
    text: &str,
    max_width: Option<f32>,
    preserves_spaces: bool,
    unhung: &[std::ops::Range<usize>],
) -> HangStats {
    layout.break_all_lines(max_width);
    let max = max_width.filter(|max| max.is_finite());
    // The pass hangs preserved spaces on every line it breaks, whichever
    // test sent the paragraph through it: a line [`unglue`] moves an NBSP onto
    // can come out ending in spaces parley did not hang.
    let fix = max.filter(|&max| {
        (preserves_spaces
            && (any_unhung_line(layout, text) || any_unhung_trailing_line(layout, text, unhung)))
            || any_hung_nbsp_line(layout, text, max)
    });
    let fix = fix.map(|max| (max, preserves_spaces));
    let mut stats = match fix {
        Some((max, hang_spaces)) => hang_pass(layout, text, max, None, unhung, hang_spaces),
        None => HangStats::default(),
    };
    // #1050, #1172: parley's trailing empty line after an overflowing inline
    // box or a final forced break. Broken again, the same way, up to the line
    // before it.
    if let Some(keep) = phantom_last_line(layout) {
        match fix {
            Some((max, hang_spaces)) => {
                stats = hang_pass(layout, text, max, Some(keep), unhung, hang_spaces)
            }
            None => break_lines_up_to(layout, max_width, keep),
        }
        stats.phantom_rebreaks = 1;
    }
    stats
}


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
    pub min_content: &'a HashMap<usize, (f32, f32)>,
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
        let Some(&(min_width, min_height)) = self.min_content.get(&id) else {
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
            Some(fit) if fit == min_width => (fit + margins, min_height),
            // Its height at a width it has not been laid out at is not
            // known; the one it has stands in until
            // `resolve_percentage_inline_blocks` lays it out there.
            Some(fit) => (fit + margins, layout.1),
        }
    }
}

/// Where the first line of `layout` sits its baseline, from the top of the
/// layout — the content-box top of the box it was measured for. `None` when it
/// laid out no line.
pub(crate) fn first_line_baseline(layout: &parley::layout::Layout<Brush>) -> Option<f32> {
    layout.lines().next().map(|line| line.metrics().baseline)
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

/// Break a text **leaf**'s layout — a flex or grid item's own text, measured
/// through `NodeContext::Text` rather than an IFC — at `max_width`, without
/// the empty line parley commits after a final newline ([`phantom_last_line`],
/// #1172), and without a break after an NBSP parley hung ([`unglue`], #1218).
/// A leaf hangs no spaces (it never has), so this is
/// [`break_lines_hanging_spaces`] without its spaces.
pub(crate) fn break_leaf_lines(
    layout: &mut parley::Layout<Brush>,
    text: &str,
    max_width: Option<f32>,
) -> HangStats {
    layout.break_all_lines(max_width);
    let glue = max_width.filter(|max| max.is_finite() && any_hung_nbsp_line(layout, text, *max));
    let mut stats = match glue {
        Some(max) => hang_pass(layout, text, max, None, &[], false),
        None => HangStats::default(),
    };
    if let Some(keep) = phantom_last_line(layout) {
        match glue {
            Some(max) => stats = hang_pass(layout, text, max, Some(keep), &[], false),
            None => break_lines_up_to(layout, max_width, keep),
        }
        stats.phantom_rebreaks = 1;
    }
    stats
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
/// Text under `pre` or `pre-wrap`, and all the text of a `contenteditable`
/// root, is pushed verbatim, a tab as [`TAB_SPACES`] and the characters
/// above as their substitutes. It is content to the collapsible text around
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
    /// `pre`, `pre-wrap`: everything is kept.
    Preserve,
}

impl SpaceCollapse {
    pub(crate) fn of(white_space: crate::computed_style::WhiteSpaceValue) -> Self {
        use crate::computed_style::WhiteSpaceValue;
        match white_space {
            WhiteSpaceValue::Normal | WhiteSpaceValue::NoWrap => Self::Collapse,
            WhiteSpaceValue::PreLine => Self::PreserveBreaks,
            WhiteSpaceValue::Pre | WhiteSpaceValue::PreWrap => Self::Preserve,
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
        self.ops.push(IfcOp::InlineBox(inline_box));
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
            let mut out = String::with_capacity(raw.len() + 3 * TAB_SPACES.len());
            for (i, c) in raw.char_indices() {
                note(&mut map, out.len(), i);
                if c == '\t' {
                    out.push_str(TAB_SPACES);
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

    /// Remove a space held at the IFC's end and replay the ops into
    /// `builder`, all under `Preserve`: the text is already collapsed.
    ///
    /// With `emoji_family`, each emoji-presentation cluster of a text is
    /// pushed in a span of that family ([`crate::fonts::TextFamily`], #1204).
    /// A cluster is found within one text op; one split across two (an emoji
    /// and its U+FE0F in two text nodes) is not.
    pub(crate) fn finish(
        &mut self,
        builder: &mut parley::TreeBuilder<'_, Brush>,
        emoji_family: Option<parley::style::FontFamily<'static>>,
    ) {
        self.drop_pending();
        builder.set_white_space_mode(parley::style::WhiteSpaceCollapse::Preserve);
        let emoji_span = emoji_family.map(|f| [parley::style::StyleProperty::FontFamily(f)]);
        for op in self.ops.drain(..) {
            match op {
                IfcOp::Span(props) => builder.push_style_modification_span(props.iter()),
                IfcOp::Pop => builder.pop_style_span(),
                IfcOp::Text(text) => match &emoji_span {
                    Some(span) => {
                        let mut at = 0;
                        for range in crate::fonts::emoji_presentation_ranges(&text) {
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
                    None => builder.push_text(&text),
                },
                IfcOp::InlineBox(b) => builder.push_inline_box(b),
            }
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

/// The hanging-space re-break of [`break_lines_hanging_spaces`], over a layout
/// already broken at `max`, committing at most `limit` lines when given one.
/// Spaces are hung only with `hang_spaces`; a line parley ended by hanging a
/// no-break space is broken again ([`unglue`]) either way.
fn hang_pass(
    layout: &mut parley::Layout<Brush>,
    text: &str,
    max: f32,
    limit: Option<usize>,
    unhung: &[std::ops::Range<usize>],
    hang_spaces: bool,
) -> HangStats {
    use parley::layout::{BreakReason, YieldData};
    let mut stats = HangStats::default();
    let units = logical_units(layout, text, unhung);
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
        let YieldData::LineBreak(data) = yielded else {
            continue;
        };
        committed += 1;
        if !in_step {
            continue;
        }
        // The last line is committed as it is (`BreakReason::None`; an empty
        // one after a final newline even copies the previous line's advance),
        // but its unhung spaces still count for its alignment (#1212).
        if data.reason == BreakReason::None {
            if let Some(end) = line_end(&units, cursor, data.advance) {
                align_unhung_trailing(&mut breaker, &units[cursor..end], max);
            }
            continue;
        }
        // A line that ends at the hang is broken again, from the state it
        // started from, with room for its content and every space and tab
        // after it. That can end at a hang again — an NBSP after the spaces
        // overflows too, and parley hangs NBSP itself — so until it does not.
        let (mut reason, mut advance) = (data.reason, data.advance);
        let mut line_max = max;
        let mut widened_from = f32::NEG_INFINITY;
        let mut unglued = false;
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
            let hanging = match reason {
                // The hang branch is the one Regular break that overflows.
                BreakReason::Regular if hang_spaces && advance > line_max => {
                    hanging_after(&units, end)
                }
                _ => None,
            };
            let Some(hanging) = hanging else {
                align_unhung_trailing(&mut breaker, &units[cursor..end], max);
                cursor = end;
                line_start = breaker.state().clone();
                break;
            };
            // Each round takes in more of the paragraph, which is what ends
            // the loop. One that hangs again having taken in nothing would
            // repeat forever: stop fixing instead (the table would be wrong).
            if advance <= widened_from {
                in_step = false;
                break;
            }
            widened_from = advance;
            breaker.revert_to(line_start.clone());
            // Room for every space and tab but the last, which is left to
            // overflow: parley then hangs a real space and commits the line,
            // rather than hanging whatever overflows after the run (an NBSP,
            // which does not hang).
            let run = units[end..].iter().take_while(|u| u.hangs).count();
            let last = match units.get(end + run) {
                // Something follows the run on this line's paragraph: leave the
                // run's last space to overflow.
                Some(u) if run > 0 && !u.newline => units[end + run - 1].advance,
                // The end of the text or a newline: nothing can start the next
                // line, so the whole run fits.
                _ => 0.0,
            };
            line_max = advance + hanging - last + 0.01;
            let state = breaker.state_mut();
            state.set_layout_max_advance(line_max);
            state.set_line_max_advance(line_max);
            let Some(YieldData::LineBreak(again)) = breaker.break_next() else {
                in_step = false;
                break;
            };
            breaker.set_prior_line_width(max);
            stats.lines += 1;
            (reason, advance) = (again.reason, again.advance);
        }
    }
    breaker.finish();
    stats
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
            YieldData::LineBreak(d) => Some((d.reason, d.advance)),
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
            let n = before.iter().filter(|u| u.counted).count();
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

/// One cluster or inline box of a paragraph, in logical order: what the line
/// breaker adds to a line's advance, and what [`hanging_after`] needs to know.
struct LineUnit {
    advance: f32,
    /// A space or tab that hangs at a soft wrap: one whose element wraps
    /// (not NBSP, and not a `pre` element's).
    hangs: bool,
    /// A space or tab of a `pre` element (#1212): preserved, and CSS Text 3
    /// §4.1.3 hangs a preserved space only where the text wraps, so the hang
    /// pass does not widen a line for it and a line ending in it is aligned
    /// with it as content. (Parley itself still hangs the one that overflows
    /// right after `pre-wrap` spaces — tracked in #1243.)
    unhung: bool,
    newline: bool,
    /// A no-break space (U+00A0), which parley hangs and CSS glues (#1218).
    nbsp: bool,
    /// Counted by [`parley::layout::BreakLines::break_next_with_length`]:
    /// every cluster and in-flow inline box, not an out-of-flow box.
    counted: bool,
    /// An in-flow inline box.
    inline_box: bool,
    /// A hyphen or another UAX #14 class HY/BA character that is not white
    /// space: LB12a allows a break between it and an NBSP after it.
    breaks_before_glue: bool,
}

/// Re-set the alignment width of the line just committed, `line` its units,
/// when it ends in [`LineUnit::unhung`] spaces: parley counts every trailing
/// space as hanging when it aligns a line, so a `pre` root's spaces after a
/// wrapping span let an overflowing line right-align as if it fitted. With
/// the width narrowed by those spaces, parley's free space is the box's width
/// less the whole line, and an overflowing line is start-aligned — Chrome
/// 153's answer (#1212).
fn align_unhung_trailing(
    breaker: &mut parley::layout::BreakLines<'_, Brush>,
    line: &[LineUnit],
    max: f32,
) {
    let unhung: f32 = line
        .iter()
        .rev()
        .skip_while(|u| u.newline)
        .take_while(|u| u.unhung)
        .map(|u| u.advance)
        .sum();
    if unhung > 0.0 {
        breaker.set_prior_line_width(max - unhung);
    }
}

/// Whether `byte` is in one of the sorted, disjoint `ranges`.
fn in_ranges(ranges: &[std::ops::Range<usize>], byte: usize) -> bool {
    let i = ranges.partition_point(|r| r.end <= byte);
    ranges.get(i).is_some_and(|r| r.contains(&byte))
}

/// Whether some line ends (before any newline) in a space or tab of an
/// `unhung` range — the cheap test that sends a paragraph through
/// [`hang_pass`] for [`align_unhung_trailing`].
fn any_unhung_trailing_line(
    layout: &parley::Layout<Brush>,
    text: &str,
    unhung: &[std::ops::Range<usize>],
) -> bool {
    !unhung.is_empty()
        && layout.lines().any(|line| {
            let r = line.text_range();
            let body = text.get(r.clone()).unwrap_or("");
            let trimmed = body.trim_end_matches(['\n', '\r']);
            trimmed.ends_with([' ', '\t']) && in_ranges(unhung, r.start + trimmed.len() - 1)
        })
}

/// Every cluster and inline box of `layout`, in the logical order the line
/// breaker walks them. Read from an already broken layout: each cluster is on
/// exactly one line. A space in an `unhung` range is [`LineUnit::unhung`].
fn logical_units(
    layout: &parley::Layout<Brush>,
    text: &str,
    unhung: &[std::ops::Range<usize>],
) -> Vec<LineUnit> {
    // (byte, 0 = inline box / 1 = cluster, index within its kind) → unit.
    let mut keyed: Vec<((usize, u8, usize), LineUnit)> = Vec::new();
    for line in layout.lines() {
        for run in line.runs() {
            for c in run.clusters() {
                let range = c.text_range();
                let n = keyed.len();
                let space = is_hanging_space(text, &c);
                let pre = space && in_ranges(unhung, range.start);
                keyed.push((
                    (range.start, 1, n),
                    LineUnit {
                        advance: c.advance(),
                        hangs: space && !pre,
                        unhung: pre,
                        newline: c.is_hard_line_break(),
                        nbsp: c.is_space_or_nbsp()
                            && text
                                .get(range.clone())
                                .is_some_and(|t| t.starts_with('\u{a0}')),
                        counted: true,
                        inline_box: false,
                        breaks_before_glue: text
                            .get(range.clone())
                            .and_then(|t| t.chars().next_back())
                            .is_some_and(breaks_before_glue),
                    },
                ));
            }
        }
    }
    for (i, b) in layout.inline_boxes().iter().enumerate() {
        let in_flow = matches!(b.kind, parley::InlineBoxKind::InFlow);
        let advance = if in_flow { b.width } else { 0.0 };
        keyed.push((
            (b.index, 0, i),
            LineUnit {
                advance,
                hangs: false,
                unhung: false,
                newline: false,
                nbsp: false,
                counted: in_flow,
                inline_box: in_flow,
                breaks_before_glue: false,
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

/// The width of the spaces and tabs a line ending before `units[end]` has to
/// keep, or `None` when a word (or box) follows it, which is an ordinary break.
fn hanging_after(units: &[LineUnit], end: usize) -> Option<f32> {
    match units.get(end) {
        // Nothing follows: the empty line after the hang is spurious.
        None => Some(0.0),
        Some(u) if u.newline => Some(0.0),
        Some(u) if u.hangs => Some(
            units[end..]
                .iter()
                .take_while(|u| u.hangs)
                .map(|u| u.advance)
                .sum(),
        ),
        Some(_) => None,
    }
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

/// Whether some line parley ended by hanging a no-break space (#1218): a
/// regular break past `max` right after an U+00A0. The cheap test that sends a
/// paragraph through [`hang_pass`] for [`unglue`].
fn any_hung_nbsp_line(layout: &parley::Layout<Brush>, text: &str, max: f32) -> bool {
    use parley::layout::BreakReason;
    layout.lines().any(|line| {
        line.break_reason() == BreakReason::Regular
            && line.metrics().advance > max
            && text
                .get(..line.text_range().end)
                .is_some_and(|t| t.ends_with('\u{a0}'))
    })
}

/// Whether some line parley ended by hanging one space while more hangable
/// white space, a newline or nothing at all followed it. The cheap test that
/// lets the common case skip [`break_lines_hanging_spaces`]'s second break.
fn any_unhung_line(layout: &parley::Layout<Brush>, text: &str) -> bool {
    use parley::layout::{BreakReason, Cluster};
    layout.lines().any(|line| {
        line.break_reason() == BreakReason::Regular
            && match Cluster::from_byte_index(layout, line.text_range().end) {
                None => true,
                Some(c) => c.is_hard_line_break() || is_hanging_space(text, &c),
            }
    })
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
            if crate::form_control::is_value_control(&self.tree.nodes[root_id])
                || crate::replaced::is_replaced_without_content(&self.tree.nodes[root_id])
            {
                self.tree.nodes[root_id].text_layout = None;
                continue;
            }

            let max_width = self.ifc_paint_max_width(root_id);
            let node = &self.tree.nodes[root_id];

            // Skip Parley rebuild if text hasn't changed and the layout already
            // exists with the same max_width. The existing text_layout is still valid.
            if !self.tree.dirty_ifc_text_roots.contains(&root_id) {
                if let Some(existing) = &node.text_layout {
                    let old_max_width = existing.max_width;
                    let new_max_width = max_width.unwrap_or(f32::INFINITY);
                    // `==` first: an unconstrained root stores `INFINITY`, and
                    // `INFINITY - INFINITY` is NaN, which no tolerance admits, so
                    // without it every such root was reshaped on every layout
                    // pass that changed no text (ifc_unconstrained_width_skip_tests).
                    if old_max_width == new_max_width
                        || (old_max_width - new_max_width).abs() < 0.01
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

    /// The width an IFC root's paint layout is broken at: its content-box
    /// width, with the auto-width slack below. `None` for a root with no
    /// positive content width (unconstrained).
    fn ifc_paint_max_width(&self, root_id: usize) -> Option<f32> {
        let node = &self.tree.nodes[root_id];
        // Use content-box width (subtract padding+border) for line breaking.
        // For auto-width elements, don't re-constrain text to measured width
        // as floating-point precision can cause unwanted line breaks.
        let cs = &node.computed_style;
        let padding_h = cs.padding_left.to_px() + cs.padding_right.to_px();
        let border_h = cs.border_left_width.to_px() + cs.border_right_width.to_px();
        let content_width = node.layout.width - padding_h - border_h;
        if content_width > 0.0 {
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
        let (mut inline_layout, font_family_resolves) = Self::build_inline_layout(
            &self.tree.nodes,
            root_id,
            max_width,
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
        // container is cut and ends in "…" (#1091). CSS Overflow 3 §3.2 asks
        // the question per line box, after wrapping, and so does Chrome 153:
        // an unbreakable word under `white-space: normal`, a `nowrap` span
        // inside a wrapping root and each long line of `pre` text all
        // overflow a line.
        {
            use crate::computed_style::{OverflowValue, TextOverflowValue, WhiteSpaceValue};
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
                let root_is_nowrap = matches!(
                    cs.white_space,
                    WhiteSpaceValue::NoWrap | WhiteSpaceValue::Pre
                );
                // The "…" is drawn by rebuilding the text flat in the root's
                // own style (#1100 is keeping the original layout instead).
                // Per line, that is taken only where the flat text is the
                // same picture: otherwise one long word would strip the
                // colours, chips and hidden spans from every other line of
                // the paragraph, which main left clipped and styled.
                let source = if Self::ellipsis_rebuild_is_faithful(
                    &self.tree.nodes,
                    root_id,
                    &inline_layout,
                ) {
                    Some(EllipsisSource::Lines(&inline_layout))
                } else if root_is_nowrap
                    && !Self::wraps_anywhere(&self.tree.nodes, root_id, &inline_layout.text_ranges)
                {
                    // As before #1091: the whole text, cut to one prefix.
                    // Not when a descendant wraps (#1212): that paragraph has
                    // lines of its own, and one cut prefix would drop them.
                    Some(EllipsisSource::Whole(&inline_layout.text_content))
                } else {
                    None
                };
                if let Some(source) = source {
                    let built = Self::build_ellipsis_layout(
                        &self.tree.nodes,
                        root_id,
                        source,
                        container_width,
                        1.0,
                        &mut self.font_cx,
                        paint_layout_cx,
                    );
                    if let Some((layout, shapes)) = built {
                        self.tree.perf.bump(crate::perf::Counter::EllipsisBuilds);
                        self.tree
                            .perf
                            .add(crate::perf::Counter::EllipsisShapes, shapes);
                        inline_layout = layout;
                    }
                }
            }
        }

        // Write positions from Parley layout back to child nodes
        // Walk the layout lines to find positioned inline boxes and text runs
        self.write_inline_positions(root_id, &inline_layout);

        self.tree.nodes[root_id].text_layout = Some(Box::new(inline_layout));
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
    fn cleanup_anonymous_block_boxes(&mut self, scope: Option<&crate::ifc_scope::IfcScope>) {
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
        if anon_ids.is_empty() {
            return;
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
            if let Some(p) = anon.parent
                && !parents_affected.contains(&p)
                && self
                    .tree
                    .nodes
                    .get(p)
                    .is_some_and(|pn| pn.run_boxes.contains(&anon_id))
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
            if let Some(anon_taffy) = self.tree.nodes[anon_id].taffy_id {
                self.tree.taffy_map.remove(&anon_taffy);
                let _ = self.tree.taffy.remove(anon_taffy);
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
    fn create_anonymous_block_boxes(&mut self, scope: Option<&crate::ifc_scope::IfcScope>) {
        // Phase 1: Detect mixed-content block containers
        let mut containers: Vec<(usize, Vec<Vec<usize>>)> = Vec::new();
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
            let mut runs: Vec<Vec<usize>> = Vec::new();
            let mut current_run: Vec<usize> = Vec::new();

            for &child_id in &effective {
                match role_of(child_id) {
                    // A comment has no Taffy node and no box. An out-of-flow
                    // box neither joins a run nor ends one (#406): ending the
                    // current run on it made `a<abs/>b` two runs, two
                    // anonymous boxes, two lines — a split `has_block` alone
                    // cannot prevent when a real block sibling is present. A
                    // `display: none` child is boxless too (#366) and used to
                    // end the run the same wrong way.
                    InlineFlowRole::Comment | InlineFlowRole::NoBox | InlineFlowRole::OutOfFlow => {
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
                            runs.push(std::mem::take(&mut current_run));
                        }
                    }
                }
            }
            if !current_run.is_empty() {
                runs.push(current_run);
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

            for run in runs {
                // Create anonymous block box DOM node
                let anon_id = self.tree.nodes.vacant_key();
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

                // Insert anonymous node into slab
                self.tree.nodes.insert(anon_node);

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
        self.cleanup_anonymous_block_boxes(scope);
        self.restore_split_inlines(was_split);
        // Before the boxes are minted, like the split restore, so a host's
        // rebuilt list names members rather than boxes that are about to exist
        // (#591).
        self.rehome_hoisted_out_of_flow(hoist_changes);
        self.create_anonymous_block_boxes(scope);
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
            let hollow_with_children = !own_children.is_empty()
                && (crate::form_control::is_value_control(node)
                    || crate::replaced::is_replaced_without_content(node));
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
            } else if crate::form_control::is_value_control(&self.tree.nodes[root_id])
                || crate::replaced::is_replaced_without_content(&self.tree.nodes[root_id])
            {
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
                        }),
                        Some(NodeContext::Replaced {
                            width: bw,
                            height: bh,
                            ratio: br,
                        }),
                    ) => aw == bw && ah == bh && ar == br,
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
            if !self.tree.taffy.dirty(taffy_id).unwrap_or(true) {
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
        if !self.tree.atomic_min_content.is_empty() {
            for &(_, id, _) in &targets {
                self.tree.atomic_min_content.remove(&id);
            }
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
                if !self.tree.atomic_min_content.is_empty() {
                    self.tree.atomic_min_content.remove(&id);
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
            if !self.tree.atomic_min_content.is_empty() {
                self.tree.atomic_min_content.remove(&id);
            }
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
            if (now.0 - before.0).abs() <= 0.5 && (now.1 - before.1).abs() <= 0.5 {
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
            self.tree.forget_ifc_measures(root_id);
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
                    inputs,
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
                                taffy::Size {
                                    width: known_dims
                                        .width
                                        .unwrap_or(inline_layout.measured_width()),
                                    height: known_dims
                                        .height
                                        .unwrap_or(inline_layout.layout.height()),
                                }
                            }
                            Some(NodeContext::Text(text)) => {
                                if text.content.is_empty() {
                                    return taffy::Size::ZERO;
                                }
                                perf.bump(crate::perf::Counter::ShapeAtomicInline);
                                let font_family = crate::fonts::parley_text_family(
                                    font_cx,
                                    &text.font_family,
                                    &text.content,
                                );
                                let mut builder =
                                    layout_cx.ranged_builder(font_cx, &text.content, 1.0, true);
                                builder.push_default(parley::style::StyleProperty::FontSize(
                                    text.font_size,
                                ));
                                if (text.font_weight - 400.0).abs() > 1.0 {
                                    builder.push_default(parley::style::StyleProperty::FontWeight(
                                        parley::style::FontWeight::new(text.font_weight),
                                    ));
                                }
                                if let Some(lh) =
                                    layout::css_line_height_to_parley(&text.line_height_css)
                                {
                                    builder
                                        .push_default(parley::style::StyleProperty::LineHeight(lh));
                                }
                                font_family.push_to(&mut builder);
                                // Apply overflow-wrap for emergency line-breaking
                                builder.push_default(parley::style::StyleProperty::OverflowWrap(
                                    text.overflow_wrap.to_parley(),
                                ));
                                // letter-/word-spacing (#698). The builder's scale is
                                // 1.0 here, so these are CSS pixels either way.
                                builder.push_default(parley::style::StyleProperty::LetterSpacing(
                                    text.letter_spacing,
                                ));
                                builder.push_default(parley::style::StyleProperty::WordSpacing(
                                    text.word_spacing,
                                ));
                                let mut layout = builder.build(&text.content);
                                // If no_wrap is set (white-space: nowrap), don't constrain width
                                let wrap_width = if text.no_wrap {
                                    None
                                } else {
                                    known_dims.width.or(max_width)
                                };
                                break_leaf_lines(&mut layout, &text.content, wrap_width)
                                    .record(perf);
                                let size = taffy::Size {
                                    width: known_dims.width.unwrap_or(layout.width()),
                                    height: known_dims.height.unwrap_or(layout.height()),
                                };
                                // Keyed exactly as the root compute keys its own text
                                // leaves, so `copy_cached_text_layouts` picks between
                                // them by the same rule.
                                let wrap_bits = wrap_width.map(|w| w.to_bits()).unwrap_or(u32::MAX);
                                first_baseline = first_line_baseline(&layout);
                                leaf_layouts.insert((text.node_id, wrap_bits), layout);
                                size
                            }
                            Some(NodeContext::Image {
                                width,
                                height,
                                hint_ratio,
                                ..
                            }) => {
                                // A loaded image's own ratio; before it loads (or when
                                // it failed), the one its `width` and `height`
                                // attributes map to (#684); with neither, nothing.
                                let (natural, ratio) = crate::replaced::image_natural_size(
                                    *width,
                                    *height,
                                    *hint_ratio,
                                );
                                let Some(ratio) = ratio else {
                                    return taffy::Size::ZERO;
                                };
                                // As the root compute measures one (#788, #1150).
                                crate::replaced::measure(
                                    natural,
                                    Some(ratio),
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
                            }) => crate::replaced::measure(
                                (*width, *height),
                                *ratio,
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
                if let Some(nid) = node_id
                    && let Some(node) = self.tree.nodes.get_mut(nid)
                {
                    node.layout.width = layout_size.width;
                    node.layout.height = layout_size.height;
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
            let max = measure(this, taffy::AvailableSpace::MaxContent);
            this_max.set(max + 0.01);
            if atomic_inline_fits(max, stretch) {
                return max + 0.01;
            }
            let known = node_id
                .filter(|_| auto)
                .and_then(|n| this.tree.atomic_min_content.get(&n))
                .map(|m| m.0);
            let min = known.unwrap_or_else(|| {
                let mut asked = this.tree.atomic_min_requests.borrow().len();
                let mut min = measure(this, taffy::AvailableSpace::MinContent);
                for _ in 0..8 {
                    let inner = this.tree.atomic_min_requests.borrow_mut().split_off(asked);
                    if inner.is_empty() {
                        break;
                    }
                    this.measure_atomic_min_contents(inner, false);
                    asked = this.tree.atomic_min_requests.borrow().len();
                    min = measure(this, taffy::AvailableSpace::MinContent);
                }
                if auto && let Some(n) = node_id {
                    let height = this
                        .tree
                        .taffy
                        .layout(taffy_id)
                        .map_or(0.0, |l| l.size.height);
                    this.tree.atomic_min_content.insert(n, (min, height));
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
            for (inner_depth, inner) in self.reset_resolved_atomic_inlines_inside(id, depth, true)
            {
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
        // percentage-sized chip moved by half a pixel.
        self.tree.forget_ifc_measures(root_id);
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
        let mut retried: std::collections::HashSet<usize> = std::collections::HashSet::new();
        let mut changed = false;
        while let Some(id) = stack.pop() {
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
            let min = (
                self.tree.taffy.unrounded_layout(taffy_id).size.width,
                self.tree
                    .taffy
                    .layout(taffy_id)
                    .map_or(0.0, |l| l.size.height),
            );
            let _ = self.tree.taffy.mark_dirty(taffy_id);
            self.measure_inline_blocks(&[(taffy_id, None)]);
            // The boxes inside first, then this one again — once: a box
            // that cannot be measured (it left the document) is not asked
            // for twice.
            if !inner.is_empty() && retried.insert(id) {
                stack.push(id);
                stack.extend(inner);
                continue;
            }
            self.tree.atomic_min_content.insert(id, min);
            let natural = self.tree.nodes[id].layout.width;
            let resized = self.atomic_inline_resized(id, root_id, before);
            if !resized && min.0 + 1.0 >= natural {
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
    pub(crate) fn build_inline_layout(
        nodes: &slab::Slab<Node>,
        root_id: usize,
        max_width: Option<f32>,
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
        let mut builder = layout_cx.tree_builder(font_cx, scale, true, &root_text_style);
        ifc_text.finish(&mut builder, emoji_family);
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

        let (text_layout, text_content) = builder.build();
        let mut text_layout = text_layout;
        // A paragraph none of whose text may wrap is broken unconstrained:
        // only forced breaks end its lines. One that holds wrapping text — a
        // wrapping root, or a `normal`/`pre-wrap`/`pre-line` element inside a
        // `nowrap`/`pre` root (#1212) — is broken at the available width, and
        // the per-cluster `TextWrapMode` keeps the rest of it on its lines.
        let effective_max_width = if Self::wraps_anywhere(nodes, root_id, &text_ranges) {
            max_width
        } else {
            None
        };
        // Only preserved spaces can hang past a soft wrap or end a line as
        // content; collapsible ones are single, and gone at a forced break.
        let preserves_spaces = ifc_text.preserved();
        // The text of `pre` elements, whose preserved spaces must not hang
        // (`LineUnit::unhung`; the mixed seams left are tracked in #1243).
        let unhung: Vec<std::ops::Range<usize>> = if preserves_spaces {
            text_ranges
                .iter()
                .filter(|r| !r.is_br && r.flat_end > r.flat_start)
                .filter(|r| {
                    nodes
                        .get(r.node_id)
                        .and_then(|t| t.parent)
                        .and_then(|el| nodes.get(el))
                        .is_some_and(|el| {
                            matches!(
                                el.computed_style.white_space,
                                crate::computed_style::WhiteSpaceValue::Pre
                            )
                        })
                })
                .map(|r| r.flat_start..r.flat_end)
                .collect()
        } else {
            Vec::new()
        };
        let hang = break_lines_hanging_spaces(
            &mut text_layout,
            &text_content,
            effective_max_width,
            preserves_spaces,
            &unhung,
        );

        // Apply text-align from computed style
        let alignment = root_computed.text_align.to_parley();
        text_layout.align(alignment, parley::layout::AlignmentOptions::default());

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
                preserves_spaces,
                hang,
                span_fragments: Default::default(),
            },
            font_family_resolves.get(),
        )
    }

    /// Whether anything in an IFC may soft-wrap: the root itself (between its
    /// own text and its atomic inlines alike), or the nearest element of some
    /// text run inside it (#1212). `false` only for a `nowrap`/`pre` root none
    /// of whose text belongs to a wrapping element.
    pub(crate) fn wraps_anywhere(
        nodes: &slab::Slab<Node>,
        root_id: usize,
        text_ranges: &[crate::node::IfcTextRange],
    ) -> bool {
        let wraps = |id: usize| {
            nodes.get(id).is_some_and(|n| {
                Self::text_wrap_mode(&n.computed_style) == parley::style::TextWrapMode::Wrap
            })
        };
        wraps(root_id)
            || text_ranges.iter().filter(|r| !r.is_br).any(|r| {
                nodes
                    .get(r.node_id)
                    .and_then(|t| t.parent)
                    .is_some_and(wraps)
            })
    }

    /// Whether rebuilding `il` as flat text in the root's own style draws
    /// the same thing as `il` (#1091): no inline box (a chip would be dropped
    /// and left at its old position), no inline background or decoration
    /// span, and every text run's nearest element carries the root's text
    /// style, its `text-shadow` list and `visibility: visible`. Inherited
    /// properties are what an element passes to its text, so the nearest
    /// element answers for each run; the non-inherited ones that reach text
    /// arrive as `background_spans` / `decoration_spans`. `white-space` is
    /// not compared: it decides only where lines break, and the rebuild keeps
    /// the lines it was given.
    fn ellipsis_rebuild_is_faithful(
        nodes: &slab::Slab<Node>,
        root_id: usize,
        il: &InlineLayout,
    ) -> bool {
        use crate::computed_style::VisibilityValue;
        if !il.layout.inline_boxes().is_empty()
            || !il.background_spans.is_empty()
            || !il.decoration_spans.is_empty()
        {
            return false;
        }
        let root = &nodes[root_id].computed_style;
        // The kept lines are joined by hard breaks, and a line ended by one is
        // not justified: `text-align: justify` would be lost on every one.
        if matches!(
            root.text_align,
            crate::computed_style::TextAlignValue::Justify
        ) {
            return false;
        }
        il.text_ranges.iter().filter(|r| !r.is_br).all(|r| {
            let Some(el) = nodes.get(r.node_id).and_then(|t| t.parent) else {
                return false;
            };
            let s = &nodes[el].computed_style;
            Self::same_inline_text_style_but_wrap(s, root)
                && s.text_shadow == root.text_shadow
                && matches!(s.visibility, VisibilityValue::Visible)
        })
    }

    /// The byte offsets in `text` where each prefix of `line` that fits
    /// `target` ends, shortest first — the line's start (the empty prefix)
    /// and then one per cluster. Its clusters' advances are summed in
    /// **logical** order, so the cut is found in the layout already shaped,
    /// without shaping anything. `Line::runs` yields runs in *visual* order:
    /// summed that way, a line mixing directions reaches its logical end
    /// first and is not cut at all (#1103's second review).
    fn ellipsis_cut_ends(line: &parley::layout::Line<'_, Brush>, target: f32) -> Vec<usize> {
        let mut clusters: Vec<(std::ops::Range<usize>, f32)> = line
            .runs()
            .flat_map(|run| {
                run.clusters()
                    .map(|c| (c.text_range(), c.advance()))
                    .collect::<Vec<_>>()
            })
            .collect();
        clusters.sort_by_key(|(r, _)| r.start);
        let mut ends = vec![line.text_range().start];
        let mut x = 0.0;
        for (range, advance) in clusters {
            x += advance;
            if x > target {
                break;
            }
            ends.push(range.end);
        }
        ends
    }

    /// Build an IFC layout with ellipsis truncation (#1091), and the number
    /// of parley layouts shaped to do it; `None` when there is no text.
    ///
    /// From [`EllipsisSource::Lines`], every line of the original layout whose
    /// content (its advance less its trailing white space, as parley's own
    /// `Layout::width` measures it) overflows `container_width` is cut at the
    /// last cluster that fits together with a "…" ([`Self::ellipsis_cut_ends`] —
    /// nothing is shaped for the search), every other line is kept, and the
    /// lines are joined by hard breaks and laid out again: the same lines,
    /// with a "…" on exactly the ones that overflowed, where Chrome 153 draws
    /// them. Only taken where [`Self::ellipsis_rebuild_is_faithful`].
    ///
    /// From [`EllipsisSource::Whole`], the whole text is cut to the longest
    /// prefix that fits (a binary search, shaping each candidate) and laid out
    /// unconstrained — what a `nowrap`/`pre` root always got. That rebuild is
    /// flat text in the root's own style: inline styling, inline boxes and
    /// text-node ranges are lost (#1100).
    #[allow(clippy::too_many_arguments)]
    fn build_ellipsis_layout(
        nodes: &slab::Slab<Node>,
        root_id: usize,
        source: EllipsisSource<'_>,
        container_width: f32,
        scale: f32,
        font_cx: &mut parley::FontContext,
        layout_cx: &mut parley::LayoutContext<Brush>,
    ) -> Option<(InlineLayout, u64)> {
        let style = EllipsisStyle::of(&nodes[root_id].computed_style, scale);
        let ellipsis = "\u{2026}";
        let mut shapes = 1u64;
        let ellipsis_width = {
            let mut l = style.shape(layout_cx, font_cx, ellipsis, scale, false);
            l.break_all_lines(None);
            l.width()
        };
        let target_width = container_width - ellipsis_width;
        let cut = |prefix: &str| prefix.trim_end().to_string() + ellipsis;

        // Per cut line: its start and its candidate prefix ends (see
        // [`Self::ellipsis_cut_ends`]), so an overshoot can step back.
        let mut cuts: Vec<Option<(usize, Vec<usize>)>> = Vec::new();
        let mut lines: Vec<String> = Vec::new();
        let source_text = match &source {
            EllipsisSource::Lines(il) => il.text_content.as_str(),
            EllipsisSource::Whole(t) => t,
        };
        let (mut lines, whole) = match source {
            EllipsisSource::Lines(il) => {
                let text = &il.text_content;
                for line in il.layout.lines() {
                    let m = line.metrics();
                    let range = line.text_range();
                    if m.advance - m.trailing_whitespace > container_width {
                        let ends = if target_width > 0.0 {
                            Self::ellipsis_cut_ends(&line, target_width)
                        } else {
                            vec![range.start]
                        };
                        let end = *ends.last().unwrap_or(&range.start);
                        lines.push(cut(text.get(range.start..end).unwrap_or("")));
                        cuts.push(Some((range.start, ends)));
                    } else {
                        lines.push(text.get(range).unwrap_or("").trim_end().to_string());
                        cuts.push(None);
                    }
                }
                (lines, false)
            }
            EllipsisSource::Whole(full_text) => {
                if full_text.is_empty() {
                    return None;
                }
                let chars: Vec<char> = full_text.chars().collect();
                let mut best_len = 0;
                if target_width > 0.0 {
                    let (mut lo, mut hi) = (1usize, chars.len());
                    while lo <= hi {
                        let mid = (lo + hi) / 2;
                        let prefix: String = chars[..mid].iter().collect();
                        let mut l = style.shape(layout_cx, font_cx, &prefix, scale, false);
                        shapes += 1;
                        l.break_all_lines(None);
                        if l.width() <= target_width {
                            best_len = mid;
                            lo = mid + 1;
                        } else {
                            hi = mid - 1;
                        }
                    }
                }
                (
                    vec![cut(&chars[..best_len].iter().collect::<String>())],
                    true,
                )
            }
        };
        if lines.iter().all(|l| l.is_empty()) {
            return None;
        }
        let (mut layout, mut text, built) = if whole {
            let text = lines.concat();
            let mut layout = style.shape(layout_cx, font_cx, &text, scale, true);
            layout.break_all_lines(None);
            layout.align(style.alignment, parley::layout::AlignmentOptions::default());
            (layout, text, 1)
        } else {
            Self::layout_ellipsis_lines(&style, &lines, container_width, scale, font_cx, layout_cx)
        };
        shapes += built;

        // A cluster's advance carries its kerning against the character that
        // followed it, which the "…" replaces, so a cut line shaped again can
        // come out a little wider than the box (up to 1.26px measured, #1103's
        // second review). Step such a line back one cluster and lay out again;
        // it takes a shape only when a line actually overshoots.
        for _ in 0..3 {
            if whole || layout.len() != lines.len() {
                break;
            }
            let mut changed = false;
            for (i, line) in layout.lines().enumerate() {
                let m = line.metrics();
                if m.advance - m.trailing_whitespace <= container_width + 0.01 {
                    continue;
                }
                if let Some(Some((start, ends))) = cuts.get_mut(i) {
                    if ends.len() > 1 {
                        ends.pop();
                        let end = *ends.last().unwrap();
                        lines[i] = cut(source_text.get(*start..end).unwrap_or(""));
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
            let (l, t, built) = Self::layout_ellipsis_lines(
                &style,
                &lines,
                container_width,
                scale,
                font_cx,
                layout_cx,
            );
            layout = l;
            text = t;
            shapes += built;
        }

        Some((
            InlineLayout {
                layout,
                text_content: text,
                child_positions: Vec::new(),
                text_ranges: Vec::new(),
                root_color: style.color,
                background_spans: Vec::new(),
                decoration_spans: Vec::new(),
                vertical_align_spans: Vec::new(),
                max_width: container_width,
                // Every line fits: nothing hangs.
                preserves_spaces: false,
                hang: HangStats::default(),
                span_fragments: Default::default(),
            },
            shapes,
        ))
    }

    /// Lay `lines` out one per line, joined by hard breaks, and the number of
    /// layouts shaped. Broken at `container_width` so `text-align` lines each
    /// line up against the box, as the layout it replaces did. Every line fits
    /// by construction, but a cut line is shaped whole here where its prefix
    /// and "…" were measured apart; if that wraps anything, the text is broken
    /// only at the hard breaks instead, so a line is never split in two.
    pub(crate) fn layout_ellipsis_lines(
        style: &EllipsisStyle,
        lines: &[String],
        container_width: f32,
        scale: f32,
        font_cx: &mut parley::FontContext,
        layout_cx: &mut parley::LayoutContext<Brush>,
    ) -> (parley::layout::Layout<Brush>, String, u64) {
        let text = lines.join("\n");
        let mut layout = style.shape(layout_cx, font_cx, &text, scale, true);
        layout.break_all_lines(Some(container_width));
        let mut shapes = 1;
        if layout.len() != lines.len() {
            layout = style.shape(layout_cx, font_cx, &text, scale, true);
            layout.break_all_lines(None);
            shapes += 1;
        }
        layout.align(style.alignment, parley::layout::AlignmentOptions::default());
        (layout, text, shapes)
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
        background_spans: &mut Vec<crate::node::InlineBackgroundSpan>,
        decoration_spans: &mut Vec<crate::node::InlineDecorationSpan>,
        vertical_align_spans: &mut Vec<crate::node::InlineVerticalAlignSpan>,
    ) {
        if end <= start {
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

        for child_id in children {
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
                        // Collapsed (#1180), or verbatim with a tab as
                        // `TAB_SPACES` (Parley has no tab stops), by the
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

                    builder.pop_span();

                    // Record background and/or vertical-align span if the
                    // inline element has one. Through the shared helper, so
                    // this and the split-inline bridge below cannot drift
                    // about padding, radius, or the vertical shift.
                    if has_bg || has_valign {
                        Self::push_inline_spans(
                            nodes,
                            child,
                            bg_start,
                            *flat_pos,
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
                    builder.push_inline_box(parley::InlineBox {
                        id: child_id as u64,
                        index: 0, // will be set by builder
                        width: width * scale,
                        height: height * scale,
                        kind: parley::InlineBoxKind::InFlow,
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

        // Close whatever is still open at the end of the run.
        for (owner_id, start) in open.drain(..).rev() {
            if let Some(owner) = nodes.get(owner_id) {
                Self::push_inline_spans(
                    nodes,
                    owner,
                    start,
                    *flat_pos,
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
    /// around. The root's own `estimated_height` (a virtualized, collapsed
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
            for (index, &child_id) in parent.ifc_children().iter().enumerate() {
                let Some(child) = nodes.get(child_id) else {
                    continue;
                };
                let Some(root) = child.ifc_root else {
                    continue;
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
            if entry.signature == Some(sig) {
                continue;
            }
            let seen_before = entry.signature.is_some();
            entry.signature = Some(sig);
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

#[cfg(test)]
#[path = "hang_differential_tests.rs"]
mod hang_differential_tests;

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

#[cfg(test)]
mod ellipsis_layout_tests {
    use crate::RinchDocument;

    /// The hard-break fallback of `layout_ellipsis_lines` (#1091, #1103's
    /// review Mx2): handed a line wider than the box, the break at the box's
    /// width would split it in two, and the fallback keeps it one line. The
    /// faithful per-line path cannot hand it one by construction, so the line
    /// is handed in directly.
    #[test]
    fn a_line_wider_than_the_box_is_never_split_by_the_rebuild() {
        let mut doc = RinchDocument::new();
        let cs = crate::computed_style::ComputedStyle::default();
        let style = super::EllipsisStyle::of(&cs, 1.0);
        let lines = vec!["ab cd ef gh ij kl mn op".to_string(), "x".to_string()];
        let mut cx = parley::LayoutContext::new();
        let (layout, text, shapes) = RinchDocument::layout_ellipsis_lines(
            &style,
            &lines,
            20.0,
            1.0,
            &mut doc.font_cx,
            &mut cx,
        );
        assert_eq!(text, "ab cd ef gh ij kl mn op\nx");
        assert_eq!(layout.len(), 2, "one layout line per given line");
        assert_eq!(shapes, 2, "the break at the box width was tried first");
        // Positive control: the same lines at a width they fit take one shape.
        let (layout, _, shapes) = RinchDocument::layout_ellipsis_lines(
            &style,
            &lines,
            1000.0,
            1.0,
            &mut doc.font_cx,
            &mut cx,
        );
        assert_eq!((layout.len(), shapes), (2, 1));
    }
}

/// What the inline formatting context lays a tab (`'\t'`) out as: four spaces,
/// since Parley has no tab stops. So a tab is `TAB_SPACES.len()` bytes of the
/// flat offsets every text query takes (`text_query`'s DOM↔flat maps,
/// `DomDocument::tab_flat_bytes`), and they must all read this one constant.
pub(crate) const TAB_SPACES: &str = "    ";
