//! Typography-related Stylo conversion functions.

use crate::computed_style::values::*;

/// The computed `font-family` as a CSS list parley's `parse_css_list` reads
/// back as the same families (#1223).
///
/// Every family *name* is written quoted, so a name with spaces or a comma
/// stays one name, a name spelled like a generic (`"serif"`) stays a name,
/// and a name that starts with a quote is not an unterminated string. Bare
/// are stylo's generics and an unquoted name parley parses as a generic
/// stylo lacks (`ui-monospace`, `emoji`, `math`, `fangsong`, ...). An empty
/// name matches no font and would be a parse error that loses every family
/// after it, so it is left out; so is a name holding both `"` and `'`, which
/// parley's parser (no escapes) cannot read.
pub(super) fn font_family_from_stylo(family: &style::values::computed::font::FontFamily) -> String {
    use style::values::computed::font::{
        FontFamilyNameSyntax, GenericFontFamily, SingleFontFamily,
    };
    let mut result = String::new();
    for f in family.families.iter() {
        let item = match f {
            SingleFontFamily::FamilyName(family_name) => {
                let name: &str = family_name.name.as_ref();
                if name.is_empty() {
                    continue;
                }
                // A CSS Fonts 4 generic the servo build of stylo does not know
                // (`ui-monospace`, `emoji`, `math`, `fangsong`, ...) arrives as
                // an unquoted name; it stays bare so parley reads it as the
                // generic. Quoted, it is a family name, as CSS says.
                if family_name.syntax == FontFamilyNameSyntax::Identifiers
                    && parley::fontique::GenericFamily::parse(name).is_some()
                {
                    std::borrow::Cow::Borrowed(name)
                } else {
                    match crate::fonts::quote_family(name) {
                        Some(quoted) => std::borrow::Cow::Owned(quoted),
                        None => continue,
                    }
                }
            }
            SingleFontFamily::Generic(generic) => std::borrow::Cow::Borrowed(match *generic {
                GenericFontFamily::None => "sans-serif",
                GenericFontFamily::Serif => "serif",
                GenericFontFamily::SansSerif => "sans-serif",
                GenericFontFamily::Monospace => "monospace",
                GenericFontFamily::Cursive => "cursive",
                GenericFontFamily::Fantasy => "fantasy",
                GenericFontFamily::SystemUi => "system-ui",
            }),
        };
        if !result.is_empty() {
            result.push_str(", ");
        }
        result.push_str(&item);
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

/// `letter-spacing`'s percentage resolves against the element's own
/// `font-size` (#743) — measured in Chrome 153: `letter-spacing: 50%` at
/// `font-size: 20px` adds exactly 10px per affected cluster, and at
/// `font-size: 40px` exactly 20px, so the basis scales with `font-size` and
/// not with any glyph's measured advance (the font's own glyphs, including
/// the space, average well under 1em wide in that same probe, which is what
/// rules out "the space glyph's advance" as the basis for either property —
/// see `word_spacing_from_stylo` below). `font_size_px` is the caller's
/// already-computed `font.font_size.computed_size().px()`, the same value
/// that lands in `ComputedStyle::font_size`, so the two can never disagree
/// about which font-size the percentage was resolved against — and a later
/// `font-size` change re-shapes this text because `same_text_layout_inputs`/
/// `same_measured_text_inputs` compare `font_size` directly, not just the
/// resolved spacing.
pub(super) fn letter_spacing_from_stylo(
    ls: &style::values::computed::text::LetterSpacing,
    font_size_px: f32,
) -> f32 {
    let (px, pct) = super::calc::split_length_percentage(&ls.0);
    px + pct * font_size_px
}

/// `word-spacing`'s percentage resolves against the element's own
/// `font-size` too — **not** against the space glyph's advance width, which
/// is what css-text-4 §10 describes and what the comment this replaces
/// assumed. Measured in Chrome 153, `20px/40px monospace`, `white-space:
/// pre`, `a a a` (one character per 12.04375px, i.e. about 0.6 of the
/// 20px font-size): `word-spacing: 50%` widens the box by 20px over the
/// unspaced line — 10px per space, exactly 50% of the 20px font-size and
/// not 50% of the ~12px glyph advance a space-relative basis would give.
/// Doubling `font-size` to 40px doubles the added width to 40px (20px per
/// space), confirming the basis scales with `font-size` and not with a
/// fixed glyph metric. See `letter_spacing_from_stylo` for the shared shape.
pub(super) fn word_spacing_from_stylo(
    ws: &style::values::computed::text::WordSpacing,
    font_size_px: f32,
) -> f32 {
    let (px, pct) = super::calc::split_length_percentage(ws);
    px + pct * font_size_px
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
        // `break-spaces` preserves spaces and newlines, as `pre-wrap` /
        // `pre` do; that its spaces never hang (#1043) is not modelled.
        (WSCollapse::BreakSpaces, TWMode::Wrap) => WhiteSpaceValue::PreWrap,
        (WSCollapse::BreakSpaces, TWMode::Nowrap) => WhiteSpaceValue::Pre,
        _ => WhiteSpaceValue::Normal,
    }
}

/// `text-decoration-line` + `-style` + `-color` (the three longhands the
/// `text-decoration` shorthand expands to) as one [`TextDecorationValue`].
///
/// `current` is the element's own resolved `color`, which is what
/// `text-decoration-color`'s initial value — `currentcolor` — means. It resolves
/// to `None` here rather than to that colour, because Parley already falls back
/// to the text brush when no decoration brush is pushed; carrying `Some(text
/// colour)` instead would only defeat the "did this element change the text
/// style at all" comparison in `same_inline_text_style`.
pub(super) fn text_decoration_from_stylo(
    line: &style::values::specified::TextDecorationLine,
    style: style::properties::longhands::text_decoration_style::computed_value::T,
    color: &style::values::computed::Color,
    current: &style::color::AbsoluteColor,
) -> TextDecorationValue {
    use style::properties::longhands::text_decoration_style::computed_value::T as Style;
    TextDecorationValue {
        underline: line.contains(style::values::specified::TextDecorationLine::UNDERLINE),
        strikethrough: line.contains(style::values::specified::TextDecorationLine::LINE_THROUGH),
        style: match style {
            Style::Solid => TextDecorationStyleValue::Solid,
            Style::Double => TextDecorationStyleValue::Double,
            Style::Dotted => TextDecorationStyleValue::Dotted,
            Style::Dashed => TextDecorationStyleValue::Dashed,
            Style::Wavy => TextDecorationStyleValue::Wavy,
            // `-moz-none` is the Gecko-internal "suppress the line" value; nothing
            // in the servo build's UA sheet sets it, and it is not a *style*.
            Style::MozNone => TextDecorationStyleValue::Solid,
        },
        color: if color.is_currentcolor() {
            None
        } else {
            super::color::color_from_computed(color, current)
        },
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
