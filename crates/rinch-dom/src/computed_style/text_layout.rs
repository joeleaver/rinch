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

        // Set letter spacing if not zero
        if self.letter_spacing != 0.0 {
            builder.push_default(StyleProperty::LetterSpacing(self.letter_spacing));
        }

        // Set word spacing if not zero
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
    /// **The field list is the union of every style a text layout is built
    /// from**, and it is one list with three consumers — this file's
    /// [`Self::build_parley_layout`], `RinchDocument::build_inline_layout`'s
    /// `root_text_style`, and the `TextMeasure` context
    /// `RinchDocument::sync_text_contexts` fills. A property that reaches any of
    /// those and is missing here is a style change that silently keeps the old
    /// glyphs; **a property listed here that reaches none of them costs only a
    /// spare rebuild**, so when in doubt, list it. `overflow_x` is in the list
    /// for that reason and is not a typography property: it is half the
    /// `text-overflow: ellipsis` condition, which decides whether the layout is
    /// rebuilt truncated.
    ///
    /// Hand-written rather than `PartialEq` for the same reason
    /// `RinchDocument::same_inline_text_style` is —
    /// [`super::values::LineHeightValue`] carries an `f32` and does not derive
    /// it — and that neighbour is a **different, smaller** list (the properties
    /// an inline *span* contributes), not this one.
    pub fn same_text_layout_inputs(&self, other: &ComputedStyle) -> bool {
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
            && self.color == other.color
            && self.text_align == other.text_align
            && self.text_decoration.underline == other.text_decoration.underline
            && self.text_decoration.strikethrough == other.text_decoration.strikethrough
            && self.text_transform == other.text_transform
            && self.text_underline_offset == other.text_underline_offset
            && self.white_space == other.white_space
            && self.overflow_wrap == other.overflow_wrap
            && self.text_overflow == other.text_overflow
            && self.overflow_x == other.overflow_x
    }
}
