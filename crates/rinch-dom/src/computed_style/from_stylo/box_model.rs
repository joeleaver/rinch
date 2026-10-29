//! Box model Stylo conversion functions: lengths, margins, gaps, insets, borders.

use super::calc::split_length_percentage;
use crate::computed_style::values::*;

/// The one `LengthPercentage` → `LengthPercentageValue` conversion. A mixed
/// `calc()` — which `to_length()`/`to_percentage()` both refuse — becomes a
/// `Calc { px, pct }` pair instead of silently degrading to `Zero` (#278).
pub(super) fn lp_value_from_stylo(
    lp: &style::values::computed::LengthPercentage,
) -> LengthPercentageValue {
    if let Some(len) = lp.to_length() {
        LengthPercentageValue::Length(len.px())
    } else if let Some(pct) = lp.to_percentage() {
        LengthPercentageValue::Percent(pct.0)
    } else {
        let (px, pct) = split_length_percentage(lp);
        LengthPercentageValue::Calc { px, pct }
    }
}

pub(super) fn length_percentage_from_stylo(
    lp: &style::values::computed::NonNegativeLengthPercentage,
) -> LengthPercentageValue {
    lp_value_from_stylo(&lp.0)
}

/// `vertical-align` (#724). Since stylo 0.21 it is a shorthand
/// (css-inline-3) of `alignment-baseline`, `baseline-shift` and
/// `baseline-source`, so the value is read back from the first two: the eight
/// keywords and a `<length-percentage>` stylo 0.11 computed land as they did
/// (`baseline`, `middle`, `text-top` and `text-bottom` in `alignment-baseline`;
/// `sub`, `super`, `top`, `bottom` and a length in `baseline-shift`).
///
/// Two inputs that 0.11 rejected now parse: `center` (a `baseline-shift`
/// keyword), which folds to `Baseline`, the value an invalid declaration left
/// before; and a combination such as `middle 4px`, where the
/// `alignment-baseline` keyword wins, a combination 0.11 could not express.
/// `vertical-align: 0` is now `baseline-shift: 0`, the initial value, and
/// reads as `Baseline`, which lays out the same as a zero length.
pub(super) fn vertical_align_from_stylo(
    alignment: style::values::computed::AlignmentBaseline,
    shift: &style::values::computed::BaselineShift,
) -> VerticalAlignValue {
    use style::values::computed::AlignmentBaseline;
    use style::values::generics::box_::{BaselineShiftKeyword, GenericBaselineShift};
    match alignment {
        AlignmentBaseline::Middle => return VerticalAlignValue::Middle,
        AlignmentBaseline::TextTop => return VerticalAlignValue::TextTop,
        AlignmentBaseline::TextBottom => return VerticalAlignValue::TextBottom,
        _ => {}
    }
    match shift {
        GenericBaselineShift::Keyword(BaselineShiftKeyword::Sub) => VerticalAlignValue::Sub,
        GenericBaselineShift::Keyword(BaselineShiftKeyword::Super) => VerticalAlignValue::Super,
        GenericBaselineShift::Keyword(BaselineShiftKeyword::Top) => VerticalAlignValue::Top,
        GenericBaselineShift::Keyword(BaselineShiftKeyword::Bottom) => VerticalAlignValue::Bottom,
        GenericBaselineShift::Keyword(BaselineShiftKeyword::Center) => VerticalAlignValue::Baseline,
        GenericBaselineShift::Length(lp) if style::Zero::is_zero(lp) => {
            VerticalAlignValue::Baseline
        }
        GenericBaselineShift::Length(lp) => {
            VerticalAlignValue::LengthPercentage(lp_value_from_stylo(lp))
        }
    }
}

pub(super) fn margin_from_stylo_generic(
    margin: &style::values::computed::Margin,
) -> LengthPercentageAutoValue {
    use style::values::generics::length::GenericMargin;
    match margin {
        GenericMargin::Auto => LengthPercentageAutoValue::Auto,
        GenericMargin::LengthPercentage(lp) => lp_auto_value_from_stylo(lp),
        _ => LengthPercentageAutoValue::Auto,
    }
}

/// The one `LengthPercentage` → `LengthPercentageAutoValue` conversion. A
/// mixed `calc()` becomes `Calc { px, pct }` instead of `Length(0.0)` — the
/// exact defect of #278 (`left: calc(50% - 10px)` sat at 0).
fn lp_auto_value_from_stylo(
    lp: &style::values::computed::LengthPercentage,
) -> LengthPercentageAutoValue {
    if let Some(len) = lp.to_length() {
        LengthPercentageAutoValue::Length(len.px())
    } else if let Some(pct) = lp.to_percentage() {
        LengthPercentageAutoValue::Percent(pct.0)
    } else {
        let (px, pct) = split_length_percentage(lp);
        LengthPercentageAutoValue::Calc { px, pct }
    }
}

pub(super) fn gap_from_stylo(
    gap: &style::values::computed::length::NonNegativeLengthPercentageOrNormal,
) -> LengthPercentageValue {
    use style::values::computed::length::NonNegativeLengthPercentageOrNormal;
    match gap {
        NonNegativeLengthPercentageOrNormal::Normal => LengthPercentageValue::Zero,
        NonNegativeLengthPercentageOrNormal::LengthPercentage(lp) => {
            length_percentage_from_stylo(lp)
        }
    }
}

pub(super) fn inset_from_stylo_generic(
    inset: &style::values::computed::Inset,
) -> LengthPercentageAutoValue {
    use style::values::generics::position::GenericInset;
    match inset {
        GenericInset::Auto => LengthPercentageAutoValue::Auto,
        GenericInset::LengthPercentage(lp) => lp_auto_value_from_stylo(lp),
        _ => LengthPercentageAutoValue::Auto,
    }
}

pub(super) fn border_radius_from_stylo(
    radius: &style::values::computed::BorderCornerRadius,
) -> LengthPercentageValue {
    // BorderCornerRadius is a Size: `width` is the horizontal radius and
    // `height` the vertical one (#977; [`border_radius_y_from_stylo`]).
    // Percentages (and the percentage part of a calc) are resolved at paint
    // time when dimensions are known, each against its own axis.
    lp_value_from_stylo(&radius.0.width.0)
}

/// The **vertical** radius of a corner — `BorderCornerRadius`'s `height`
/// (#977). See [`border_radius_from_stylo`] for the horizontal one.
pub(super) fn border_radius_y_from_stylo(
    radius: &style::values::computed::BorderCornerRadius,
) -> LengthPercentageValue {
    lp_value_from_stylo(&radius.0.height.0)
}

/// Check if a border style is 'none' or 'hidden' (meaning no border should be painted).
pub(super) fn border_style_is_none(style: &style::values::computed::BorderStyle) -> bool {
    use style::values::computed::BorderStyle;
    matches!(style, BorderStyle::None | BorderStyle::Hidden)
}

pub(super) fn border_style_from_stylo(
    bs: &style::values::computed::BorderStyle,
) -> BorderStyleValue {
    use style::values::computed::BorderStyle;
    match *bs {
        BorderStyle::None => BorderStyleValue::None,
        BorderStyle::Solid => BorderStyleValue::Solid,
        BorderStyle::Dashed => BorderStyleValue::Dashed,
        BorderStyle::Dotted => BorderStyleValue::Dotted,
        BorderStyle::Double => BorderStyleValue::Double,
        BorderStyle::Hidden => BorderStyleValue::Hidden,
        BorderStyle::Inset => BorderStyleValue::Inset,
        BorderStyle::Outset => BorderStyleValue::Outset,
        BorderStyle::Groove => BorderStyleValue::Groove,
        BorderStyle::Ridge => BorderStyleValue::Ridge,
    }
}

pub(super) fn border_style_from_stylo_outline(
    os: &style::values::computed::OutlineStyle,
) -> BorderStyleValue {
    use style::values::computed::OutlineStyle;
    match *os {
        OutlineStyle::Auto => BorderStyleValue::Solid,
        OutlineStyle::BorderStyle(ref bs) => border_style_from_stylo(bs),
    }
}

pub(super) fn border_style_is_none_val(style: &BorderStyleValue) -> bool {
    matches!(style, BorderStyleValue::None | BorderStyleValue::Hidden)
}
