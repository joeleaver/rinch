//! Typography-related Stylo conversion functions.

use crate::computed_style::values::*;

pub(super) fn font_family_from_stylo(family: &style::values::computed::font::FontFamily) -> String {
    use style::values::computed::font::{GenericFontFamily, SingleFontFamily};
    let mut result = String::new();
    for (i, f) in family.families.iter().enumerate() {
        if i > 0 {
            result.push_str(", ");
        }
        match f {
            SingleFontFamily::FamilyName(name) => {
                result.push_str(name.name.as_ref());
            }
            SingleFontFamily::Generic(generic) => {
                result.push_str(match *generic {
                    GenericFontFamily::None => "sans-serif",
                    GenericFontFamily::Serif => "serif",
                    GenericFontFamily::SansSerif => "sans-serif",
                    GenericFontFamily::Monospace => "monospace",
                    GenericFontFamily::Cursive => "cursive",
                    GenericFontFamily::Fantasy => "fantasy",
                    GenericFontFamily::SystemUi => "system-ui",
                });
            }
        }
    }
    result
}

pub(super) fn font_style_from_stylo(
    font_style: &style::values::computed::font::FontStyle,
) -> FontStyleValue {
    use style::values::computed::font::FontStyle as StyloFontStyle;
    if *font_style == StyloFontStyle::NORMAL {
        FontStyleValue::Normal
    } else if *font_style == StyloFontStyle::ITALIC {
        FontStyleValue::Italic
    } else {
        // Oblique - any other angle is oblique
        FontStyleValue::Oblique
    }
}

pub(super) fn line_height_from_stylo(lh: &style::values::computed::LineHeight) -> LineHeightValue {
    use style::values::generics::font::LineHeight;
    match lh {
        LineHeight::Normal => LineHeightValue::Normal,
        LineHeight::Number(n) => LineHeightValue::Relative(n.0),
        LineHeight::Length(len) => {
            // NonNegativeLength - get the px value directly
            LineHeightValue::Absolute(len.0.px())
        }
    }
}

pub(super) fn letter_spacing_from_stylo(ls: &style::values::computed::text::LetterSpacing) -> f32 {
    // LetterSpacing wraps a LengthPercentage. A percentage here is
    // font-relative (resolved per glyph at used-value time — Chrome keeps
    // `calc(50% - 10px)` unresolved in the computed value), which the px-only
    // spacing rinch hands Parley cannot express. Keep the length part of a
    // mixed calc rather than dropping the whole value.
    let (px, _pct) = super::calc::split_length_percentage(&ls.0);
    px
}

pub(super) fn word_spacing_from_stylo(ws: &style::values::computed::text::WordSpacing) -> f32 {
    // Same shape as letter_spacing_from_stylo: the percentage part is
    // font-relative and not representable in px-only spacing.
    let (px, _pct) = super::calc::split_length_percentage(ws);
    px
}

pub(super) fn text_align_from_stylo(align: &style::values::computed::TextAlign) -> TextAlignValue {
    use style::values::computed::TextAlign;
    match *align {
        TextAlign::Start => TextAlignValue::Start,
        TextAlign::End => TextAlignValue::End,
        TextAlign::Left => TextAlignValue::Start,
        TextAlign::Right => TextAlignValue::End,
        TextAlign::Center => TextAlignValue::Center,
        TextAlign::Justify => TextAlignValue::Justify,
        _ => TextAlignValue::Start,
    }
}

pub(super) fn white_space_from_stylo(
    collapse: &style::properties::longhands::white_space_collapse::computed_value::T,
    wrap_mode: &style::properties::longhands::text_wrap_mode::computed_value::T,
) -> WhiteSpaceValue {
    use style::properties::longhands::text_wrap_mode::computed_value::T as TWMode;
    use style::properties::longhands::white_space_collapse::computed_value::T as WSCollapse;
    match (*collapse, *wrap_mode) {
        (WSCollapse::Collapse, TWMode::Wrap) => WhiteSpaceValue::Normal,
        (WSCollapse::Collapse, TWMode::Nowrap) => WhiteSpaceValue::NoWrap,
        (WSCollapse::Preserve, TWMode::Nowrap) => WhiteSpaceValue::Pre,
        (WSCollapse::Preserve, TWMode::Wrap) => WhiteSpaceValue::PreWrap,
        (WSCollapse::PreserveBreaks, TWMode::Wrap) => WhiteSpaceValue::PreLine,
        _ => WhiteSpaceValue::Normal,
    }
}

pub(super) fn text_decoration_from_stylo(
    line: &style::values::specified::TextDecorationLine,
) -> TextDecorationValue {
    TextDecorationValue {
        underline: line.contains(style::values::specified::TextDecorationLine::UNDERLINE),
        strikethrough: line.contains(style::values::specified::TextDecorationLine::LINE_THROUGH),
    }
}

pub(super) fn text_transform_from_stylo(
    tt: &style::values::computed::TextTransform,
) -> TextTransformValue {
    use style::values::specified::text::TextTransform;
    if tt.contains(TextTransform::UPPERCASE) {
        TextTransformValue::Uppercase
    } else if tt.contains(TextTransform::LOWERCASE) {
        TextTransformValue::Lowercase
    } else if tt.contains(TextTransform::CAPITALIZE) {
        TextTransformValue::Capitalize
    } else {
        TextTransformValue::None
    }
}

pub(super) fn overflow_wrap_from_stylo(
    ow: &style::properties::longhands::overflow_wrap::computed_value::T,
) -> OverflowWrapValue {
    use style::values::specified::text::OverflowWrap as StyloOW;
    match *ow {
        StyloOW::Normal => OverflowWrapValue::Normal,
        StyloOW::BreakWord => OverflowWrapValue::BreakWord,
        StyloOW::Anywhere => OverflowWrapValue::Anywhere,
    }
}

pub(super) fn text_overflow_from_stylo(
    to: &style::values::computed::TextOverflow,
) -> TextOverflowValue {
    use style::values::specified::text::TextOverflowSide;
    // CSS text-overflow: ellipsis sets the "second" (end) side
    match to.second {
        TextOverflowSide::Ellipsis => TextOverflowValue::Ellipsis,
        _ => TextOverflowValue::Clip,
    }
}
