//! Box model Stylo conversion functions: lengths, margins, gaps, insets, borders.

use crate::computed_style::values::*;

pub(super) fn length_percentage_from_stylo(
    lp: &style::values::computed::NonNegativeLengthPercentage,
) -> LengthPercentageValue {
    if let Some(len) = lp.0.to_length() {
        LengthPercentageValue::Length(len.px())
    } else if let Some(pct) = lp.0.to_percentage() {
        LengthPercentageValue::Percent(pct.0)
    } else {
        LengthPercentageValue::Zero
    }
}

pub(super) fn margin_from_stylo_generic(
    margin: &style::values::computed::Margin,
) -> LengthPercentageAutoValue {
    use style::values::generics::length::GenericMargin;
    match margin {
        GenericMargin::Auto => LengthPercentageAutoValue::Auto,
        GenericMargin::LengthPercentage(lp) => {
            if let Some(len) = lp.to_length() {
                LengthPercentageAutoValue::Length(len.px())
            } else if let Some(pct) = lp.to_percentage() {
                LengthPercentageAutoValue::Percent(pct.0)
            } else {
                LengthPercentageAutoValue::Length(0.0)
            }
        }
        _ => LengthPercentageAutoValue::Auto,
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
        GenericInset::LengthPercentage(lp) => {
            if let Some(len) = lp.to_length() {
                LengthPercentageAutoValue::Length(len.px())
            } else if let Some(pct) = lp.to_percentage() {
                LengthPercentageAutoValue::Percent(pct.0)
            } else {
                LengthPercentageAutoValue::Length(0.0)
            }
        }
        _ => LengthPercentageAutoValue::Auto,
    }
}

pub(super) fn border_radius_from_stylo(
    radius: &style::values::computed::BorderCornerRadius,
) -> LengthPercentageValue {
    // BorderCornerRadius is a Size with width and height for elliptical radii
    // We just take the width (horizontal radius) for simplicity
    let width = &radius.0.width;
    if let Some(len) = width.0.to_length() {
        LengthPercentageValue::Length(len.px())
    } else if let Some(pct) = width.0.to_percentage() {
        // Store the percentage to be resolved at paint time when dimensions are known
        LengthPercentageValue::Percent(pct.0)
    } else {
        LengthPercentageValue::Zero
    }
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
        // groove, ridge, inset, outset -> render as solid
        _ => BorderStyleValue::Solid,
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
