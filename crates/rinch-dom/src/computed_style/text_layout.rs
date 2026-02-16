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
        use parley::style::{FontStack, FontWeight as ParleyFontWeight, StyleProperty};
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
        builder.push_default(StyleProperty::FontStack(FontStack::Source(Cow::Owned(
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
}
