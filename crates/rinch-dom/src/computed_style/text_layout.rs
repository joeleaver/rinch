//! Parley text layout building from ComputedStyle.

use super::ComputedStyle;
use super::values::*;

impl ComputedStyle {
    /// Build a Parley text layout from this style's typography fields.
    ///
    /// This is the SINGLE SOURCE OF TRUTH for text layout configuration,
    /// used by measurement, painting, and hit testing.
    ///
    /// # Arguments
    /// * `text` - The text content to lay out
    /// * `scale` - DPI scale factor
    /// * `font_cx` - Parley font context
    /// * `layout_cx` - Parley layout context
    /// * `max_width` - Optional maximum width for line breaking
    pub fn build_parley_layout(
        &self,
        text: &str,
        scale: f32,
        font_cx: &mut parley::FontContext,
        layout_cx: &mut parley::LayoutContext<peniko::Brush>,
        max_width: Option<f32>,
    ) -> parley::layout::Layout<peniko::Brush> {
        use parley::style::{FontFamily, FontWeight as ParleyFontWeight, StyleProperty};
        use std::borrow::Cow;

        let scaled_font_size = self.font_size * scale;

        let mut builder = layout_cx.ranged_builder(font_cx, text, 1.0, true);

        // Set font size (scaled for DPI)
        builder.push_default(StyleProperty::FontSize(scaled_font_size));

        // Set font family (default to sans-serif if empty)
        let font_family = if self.font_family.is_empty() {
            "sans-serif"
        } else {
            &self.font_family
        };
        builder.push_default(StyleProperty::FontFamily(FontFamily::Source(Cow::Owned(
            font_family.to_string(),
        ))));

        // Set font weight if not normal (400)
        if (self.font_weight - 400.0).abs() > 1.0 {
            builder.push_default(StyleProperty::FontWeight(ParleyFontWeight::new(
                self.font_weight,
            )));
        }

        // Set font style if not normal
        if self.font_style != FontStyleValue::Normal {
            builder.push_default(StyleProperty::FontStyle(self.font_style.to_parley()));
        }

        // Set line height if not normal
        if let Some(line_height) = self.line_height.to_parley() {
            builder.push_default(StyleProperty::LineHeight(line_height));
        }

        // Set letter/word spacing if not zero.
        //
        // **The `!= 0.0` guard is right here and wrong at every other producer
        // in the table below**, so do not port it. This builder shapes one text
        // node under one style: there is no enclosing span whose inherited
        // spacing a zero would have to override, so skipping the push and
        // pushing parley's own default are the same thing. The IFC producers
        // push a style span *inside* an inherited one, where
        // `letter-spacing: normal` computes to 0 and that 0 is the whole of the
        // reset — guarding there drops it (#698).
        if self.letter_spacing != 0.0 {
            builder.push_default(StyleProperty::LetterSpacing(self.letter_spacing));
        }

        if self.word_spacing != 0.0 {
            builder.push_default(StyleProperty::WordSpacing(self.word_spacing));
        }

        // Set overflow-wrap for emergency line-breaking
        if self.overflow_wrap != OverflowWrapValue::Normal {
            builder.push_default(StyleProperty::OverflowWrap(self.overflow_wrap.to_parley()));
        }

        let mut layout = builder.build(text);
        layout.break_all_lines(max_width);

        // Debug-only verification: catch layout drift
        #[cfg(debug_assertions)]
        {
            // Verify layout was built with expected parameters
            if let Some(line) = layout.lines().next() {
                for item in line.items() {
                    if let parley::layout::PositionedLayoutItem::GlyphRun(glyph_run) = item {
                        let run = glyph_run.run();
                        let actual_size = run.font_size();
                        let expected_size = scaled_font_size;
                        debug_assert!(
                            (actual_size - expected_size).abs() < 0.1,
                            "Text layout drift: expected font_size={}, got={}",
                            expected_size,
                            actual_size
                        );
                        break;
                    }
                }
            }
        }

        layout
    }

    /// Whether `self` and `other` would shape the same text into the same
    /// Parley layout — i.e. whether a layout built from one is still valid
    /// after a recascade produced the other (issue #654).
    ///
    /// A `Node`'s `text_layout` is a **derived** value: every typography
    /// property it was built from is baked into the shaped glyphs, the line
    /// breaks and the brush. Nothing about the layout records which style
    /// produced it, and `build_ifc_layouts` re-serves an existing layout to any
    /// IFC root whose `max_width` is unchanged — so a style change that nobody
    /// pairs with an invalidation is invisible until someone measures the
    /// glyphs. That is exactly how a subtree moved under a new parent came back
    /// reading `font-family: monospace` from its computed style while painting
    /// the sans-serif glyphs it was shaped with under the old one.
    ///
    /// **The field list is the union of every style an `InlineLayout` is built
    /// from**, enumerated against its five producers rather than assembled by
    /// memory. A property one of them reads and this list misses is a style
    /// change that silently keeps the old layout; **a property listed here that
    /// none of them reads costs only a spare rebuild**, so when in doubt, list
    /// it.
    ///
    /// | producer | what it reads |
    /// |---|---|
    /// | [`Self::build_parley_layout`] | `font_size`, `font_family`, `font_weight`, `font_style`, `line_height`, `letter_spacing`, `word_spacing`, `overflow_wrap` |
    /// | `RinchDocument::build_inline_layout`'s `root_text_style` | the above plus `color`, `text_decoration`, `text_underline_offset`, `white_space` (both the collapse mode and whether `max_width` applies at all) and `text_align` |
    /// | `RinchDocument::inline_style_props`, the per-span properties | `font_size`, `font_weight`, `font_style`, `color`, `text_decoration`, `text_underline_offset`, `line_height`, `letter_spacing`, `word_spacing` |
    /// | `RinchDocument::push_inline_spans`, which builds `InlineLayout::background_spans` and `::decoration_spans` | `background_color()`, the four paddings, `border_radius_top_left`, `text_decoration` |
    /// | the `TextMeasure` context `RinchDocument::sync_text_contexts` fills | `font_size`, `font_weight`, `font_family`, `line_height`, `color`, `white_space`, `letter_spacing`, `word_spacing`, `overflow_wrap`, `text_overflow`, `overflow_x` |
    ///
    /// `letter_spacing` and `word_spacing` reached only the first of those rows
    /// until #698, whose one caller pair is the MCP debug tools — so both
    /// properties parsed, inherited and compared here while changing nothing
    /// anyone could see. They are now on every row that shapes text, which is
    /// what makes their presence in this list mean something.
    ///
    /// Four producers are **not** on this list and must not be added to it:
    /// `paint::contenteditable`'s text, `paint::select`'s label, and the two
    /// input hit-test builders in `crates/rinch/src/app/`. Those are the
    /// form-control text engine, which shapes an `<input>`/`<textarea>`/
    /// `<select>`'s own value rather than an inline formatting context, keeps
    /// no `InlineLayout` for this predicate to invalidate, and must move as one
    /// piece because paint and hit testing have to agree glyph for glyph.
    /// Unifying it onto `build_parley_layout` is #320.
    ///
    /// Plus `text_transform`, which `walk_inline_children` applies to the text
    /// before it is pushed.
    ///
    /// Three entries are not typography and are here because a producer reads
    /// them anyway. `overflow_x` is half the `text-overflow: ellipsis`
    /// condition, which decides whether the layout is rebuilt truncated, and
    /// whether `display` is a flex or grid container is another part of it
    /// (a grid's own text is an anonymous item and takes no "…", #904) — only
    /// that bit, not `display` itself, because an IFC root that flips between
    /// grid and block keeps its width and its text and nothing else would
    /// rebuild it (#1046). The
    /// background-span row is **gated on `display: inline`**, because only a
    /// non-atomic inline box contributes a span. There are **three**
    /// `push_inline_spans` call sites, all in `walk_inline_children` and
    /// all behind `DisplayMode::Inline`: the `display: inline` arm itself,
    /// which tests `child.display_mode` directly, and the two halves of the
    /// split-inline bridge — one closing the stretches a run member has left,
    /// one closing whatever is still open at the end of the run — whose owners
    /// come from `split_inline_ancestors`, i.e. from `Node::is_split_inline`,
    /// which requires `display_mode == DisplayMode::Inline` as its second
    /// clause. Without the gate a `:hover { background-color }` on a block would
    /// re-shape its own label's glyphs on every hover, which is the cost this
    /// predicate exists to avoid. The gate reads *either* side, so a box that
    /// changes into or out of `display: inline` is compared too.
    ///
    /// **The gate and those call sites read two different fields, and one line
    /// keeps them in step.** This predicate tests `ComputedStyle::display`;
    /// every call site tests `Node::display_mode`. The two agree because
    /// `RinchDocument::apply_stylo_styles_to_taffy` derives the second from the
    /// first — `DisplayValue::Inline => DisplayMode::Inline`, the only arm that
    /// produces `DisplayMode::Inline` — a few lines after it calls this
    /// predicate, in the same loop iteration and from the same `new_style`. So
    /// there is no ordering hazard and no third source of truth *for a node the
    /// cascade has reached*. `display_mode`'s other writers cannot reintroduce
    /// one: `default_display_for_tag` sets it at element creation, before any
    /// cascade, when the node has no derived layout to invalidate, and
    /// `cleanup_anonymous_block_boxes`' anonymous block box is forced to
    /// `DisplayMode::Block`, which contributes no span by construction. **A
    /// future writer that sets `display_mode` to `Inline` for a node whose
    /// computed `display` is something else would silently under-list this
    /// predicate** — the node would produce a background span that no style
    /// change ever invalidates — so such a writer needs a matching arm here,
    /// not just there.
    ///
    /// Hand-written rather than `PartialEq` for the same reason
    /// `RinchDocument::same_inline_text_style` is —
    /// [`super::values::LineHeightValue`] carries an `f32` and does not derive
    /// it — and that neighbour is a **different, smaller** list (the properties
    /// an inline *span* contributes), not this one.
    pub fn same_text_layout_inputs(&self, other: &ComputedStyle) -> bool {
        use super::values::{DisplayValue, LineHeightValue};
        let inline_level =
            self.display == DisplayValue::Inline || other.display == DisplayValue::Inline;
        let same_line_height = match (self.line_height, other.line_height) {
            (LineHeightValue::Normal, LineHeightValue::Normal) => true,
            (LineHeightValue::Absolute(x), LineHeightValue::Absolute(y)) => x == y,
            (LineHeightValue::Relative(x), LineHeightValue::Relative(y)) => x == y,
            _ => false,
        };
        self.font_size == other.font_size
            && self.font_weight == other.font_weight
            && self.font_family == other.font_family
            && self.font_style == other.font_style
            && same_line_height
            && self.letter_spacing == other.letter_spacing
            && self.word_spacing == other.word_spacing
            && self.color == other.color
            && self.text_align == other.text_align
            && self.text_decoration == other.text_decoration
            && self.text_transform == other.text_transform
            && self.text_underline_offset == other.text_underline_offset
            && self.white_space == other.white_space
            && self.overflow_wrap == other.overflow_wrap
            && self.text_overflow == other.text_overflow
            && self.overflow_x == other.overflow_x
            && self.display.is_flex_or_grid_container() == other.display.is_flex_or_grid_container()
            && (!inline_level || self.same_inline_background_inputs(other))
    }

    /// The `InlineBackgroundSpan` half of [`Self::same_text_layout_inputs`],
    /// split out so the `display: inline` gate reads as the one condition it
    /// is. See that function's table for why these six and not the rest of the
    /// box model: `push_inline_spans` reads exactly these for the background it pushes.
    fn same_inline_background_inputs(&self, other: &ComputedStyle) -> bool {
        self.background_color() == other.background_color()
            && self.padding_left.to_px() == other.padding_left.to_px()
            && self.padding_right.to_px() == other.padding_right.to_px()
            && self.padding_top.to_px() == other.padding_top.to_px()
            && self.padding_bottom.to_px() == other.padding_bottom.to_px()
            && self.border_radius_top_left.to_px() == other.border_radius_top_left.to_px()
    }

    /// The subset of [`Self::same_text_layout_inputs`] that can change a
    /// **measured size** — how wide the shaped text is, and how many line boxes
    /// it breaks into (issue #678).
    ///
    /// The larger predicate answers "must the Parley layout be rebuilt"; this
    /// one answers "must Taffy run again". They are not the same question, and
    /// the difference is the whole of the cheap path `resolve_layout` takes when
    /// `layout_dirty` is false: a `:hover { color }` re-shapes the glyphs with a
    /// new brush, which is a rebuild, and leaves every box exactly where it was,
    /// which is not a relayout. A `:hover { font-weight: bold }` is both.
    ///
    /// **A property listed here that cannot move a box costs a spare Taffy
    /// compute; one missing from here leaves the box frozen around re-wrapped
    /// text**, which is #678 — so the asymmetry is the opposite way round to the
    /// larger predicate's, and the tie-break is still "list it".
    ///
    /// Six of the larger list's entries are deliberately absent, each because
    /// the producer that reads it consumes a width rather than producing one:
    /// `color`, `text_align` (it distributes a line inside a width it is given),
    /// `text_decoration` and `text_underline_offset` (ink, no advance),
    /// `text_overflow` (the ellipsis rebuild truncates to a width already
    /// decided) and the `same_inline_background_inputs` group. `overflow_x` is
    /// absent for a different reason: it is a Taffy property, so a change in it
    /// is already caught by the Taffy style comparison that sets `layout_dirty`
    /// in the first place. The same is true of the paddings inside that group,
    /// and of the flex-or-grid bit of `display` (#1046), which only decides
    /// whether the ellipsis rebuild runs.
    ///
    /// `color` is the one pinned by a fixture
    /// (`frozen_box_remeasure_tests::a_colour_only_restyle_still_skips_taffy`),
    /// because it is the cheap path's whole reason for existing; the others are
    /// documented rather than pinned.
    pub fn same_measured_text_inputs(&self, other: &ComputedStyle) -> bool {
        use super::values::LineHeightValue;
        let same_line_height = match (self.line_height, other.line_height) {
            (LineHeightValue::Normal, LineHeightValue::Normal) => true,
            (LineHeightValue::Absolute(x), LineHeightValue::Absolute(y)) => x == y,
            (LineHeightValue::Relative(x), LineHeightValue::Relative(y)) => x == y,
            _ => false,
        };
        self.font_size == other.font_size
            && self.font_weight == other.font_weight
            && self.font_family == other.font_family
            && self.font_style == other.font_style
            && same_line_height
            && self.letter_spacing == other.letter_spacing
            && self.word_spacing == other.word_spacing
            && self.text_transform == other.text_transform
            && self.white_space == other.white_space
            && self.overflow_wrap == other.overflow_wrap
    }
}
