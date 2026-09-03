//! Layout-related Stylo conversion functions: display, position, overflow, size, flex, alignment.

use super::calc::split_length_percentage;
use crate::computed_style::values::*;

/// A size-flavored `LengthPercentage` → `DimensionValue`. A mixed `calc()`
/// becomes `Calc { px, pct }` instead of silently degrading to `Auto` (#278
/// family: `width: calc(50% + 25px)` used to lay out as content-sized).
fn dimension_from_lp(lp: &style::values::computed::LengthPercentage) -> DimensionValue {
    if let Some(len) = lp.to_length() {
        DimensionValue::Length(len.px())
    } else if let Some(pct) = lp.to_percentage() {
        DimensionValue::Percent(pct.0)
    } else {
        let (px, pct) = split_length_percentage(lp);
        DimensionValue::Calc { px, pct }
    }
}

pub(super) fn display_from_stylo(display: &style::values::computed::Display) -> DisplayValue {
    use style::values::specified::box_::{DisplayInside, DisplayOutside};

    if display.is_none() {
        return DisplayValue::None;
    }
    if display.is_contents() {
        return DisplayValue::Contents;
    }

    let outside = display.outside();
    let inside = display.inside();

    match (outside, inside) {
        (DisplayOutside::Inline, DisplayInside::Flow) => DisplayValue::Inline,
        (DisplayOutside::Inline, DisplayInside::FlowRoot) => DisplayValue::InlineBlock,
        (DisplayOutside::Inline, DisplayInside::Flex) => DisplayValue::InlineFlex,
        (DisplayOutside::Block, DisplayInside::Flow) => DisplayValue::Block,
        (DisplayOutside::Block, DisplayInside::FlowRoot) => DisplayValue::Block,
        (DisplayOutside::Block, DisplayInside::Flex) => DisplayValue::Flex,
        (DisplayOutside::Block, DisplayInside::Grid) => DisplayValue::Grid,
        (DisplayOutside::Inline, DisplayInside::Grid) => DisplayValue::Grid, // inline-grid
        _ => DisplayValue::Flex, // Default to flex for unknown
    }
}

pub(super) fn position_from_stylo(
    pos: &style::values::computed::PositionProperty,
) -> PositionValue {
    use style::values::computed::PositionProperty;
    match *pos {
        PositionProperty::Static => PositionValue::Static,
        PositionProperty::Relative => PositionValue::Relative,
        PositionProperty::Absolute => PositionValue::Absolute,
        PositionProperty::Fixed => PositionValue::Fixed,
        PositionProperty::Sticky => PositionValue::Sticky,
    }
}

pub(super) fn overflow_from_stylo(overflow: &style::values::computed::Overflow) -> OverflowValue {
    use style::values::computed::Overflow;
    match *overflow {
        Overflow::Visible => OverflowValue::Visible,
        Overflow::Hidden => OverflowValue::Hidden,
        Overflow::Scroll => OverflowValue::Scroll,
        Overflow::Auto => OverflowValue::Auto,
        Overflow::Clip => OverflowValue::Clip,
    }
}

pub(super) fn size_from_stylo(size: &style::values::computed::Size) -> DimensionValue {
    use style::values::computed::Size;
    match size {
        Size::Auto => DimensionValue::Auto,
        Size::LengthPercentage(lp) => dimension_from_lp(&lp.0),
        Size::MaxContent | Size::MinContent | Size::FitContent | Size::Stretch => {
            DimensionValue::Auto
        }
        _ => DimensionValue::Auto,
    }
}

pub(super) fn max_size_from_stylo(size: &style::values::computed::MaxSize) -> DimensionValue {
    use style::values::computed::MaxSize;
    match size {
        MaxSize::None => DimensionValue::Auto,
        MaxSize::LengthPercentage(lp) => dimension_from_lp(&lp.0),
        MaxSize::MaxContent | MaxSize::MinContent | MaxSize::FitContent | MaxSize::Stretch => {
            DimensionValue::Auto
        }
        _ => DimensionValue::Auto,
    }
}

pub(super) fn flex_direction_from_stylo(
    dir: &style::properties::longhands::flex_direction::computed_value::T,
) -> FlexDirectionValue {
    use style::properties::longhands::flex_direction::computed_value::T as FlexDir;
    match *dir {
        FlexDir::Row => FlexDirectionValue::Row,
        FlexDir::RowReverse => FlexDirectionValue::RowReverse,
        FlexDir::Column => FlexDirectionValue::Column,
        FlexDir::ColumnReverse => FlexDirectionValue::ColumnReverse,
    }
}

pub(super) fn flex_wrap_from_stylo(
    wrap: &style::properties::longhands::flex_wrap::computed_value::T,
) -> FlexWrapValue {
    use style::properties::longhands::flex_wrap::computed_value::T as FlexWr;
    match *wrap {
        FlexWr::Nowrap => FlexWrapValue::NoWrap,
        FlexWr::Wrap => FlexWrapValue::Wrap,
        FlexWr::WrapReverse => FlexWrapValue::WrapReverse,
    }
}

pub(super) fn flex_basis_from_stylo(basis: &style::values::computed::FlexBasis) -> DimensionValue {
    use style::values::computed::FlexBasis;
    match basis {
        FlexBasis::Content => DimensionValue::Auto,
        FlexBasis::Size(size) => size_from_stylo(size),
    }
}

pub(super) fn align_items_from_stylo(
    align: &style::values::computed::ItemPlacement,
) -> Option<AlignItemsValue> {
    use style::values::specified::align::AlignFlags;
    let flags = align.0.value();
    if flags == AlignFlags::FLEX_START
        || flags == AlignFlags::START
        || flags == AlignFlags::SELF_START
    {
        Some(AlignItemsValue::FlexStart)
    } else if flags == AlignFlags::FLEX_END
        || flags == AlignFlags::END
        || flags == AlignFlags::SELF_END
    {
        Some(AlignItemsValue::FlexEnd)
    } else if flags == AlignFlags::CENTER {
        Some(AlignItemsValue::Center)
    } else if flags == AlignFlags::BASELINE {
        Some(AlignItemsValue::Baseline)
    } else if flags == AlignFlags::STRETCH || flags == AlignFlags::NORMAL {
        Some(AlignItemsValue::Stretch) // Normal defaults to stretch for flex items
    } else {
        None
    }
}

pub(super) fn align_self_from_stylo(
    align: &style::values::computed::SelfAlignment,
) -> Option<AlignSelfValue> {
    use style::values::specified::align::AlignFlags;
    let flags = align.0.value();
    if flags == AlignFlags::AUTO {
        None // Auto means inherit from align-items
    } else if flags == AlignFlags::FLEX_START
        || flags == AlignFlags::START
        || flags == AlignFlags::SELF_START
    {
        Some(AlignSelfValue::FlexStart)
    } else if flags == AlignFlags::FLEX_END
        || flags == AlignFlags::END
        || flags == AlignFlags::SELF_END
    {
        Some(AlignSelfValue::FlexEnd)
    } else if flags == AlignFlags::CENTER {
        Some(AlignSelfValue::Center)
    } else if flags == AlignFlags::BASELINE {
        Some(AlignSelfValue::Baseline)
    } else if flags == AlignFlags::STRETCH {
        Some(AlignSelfValue::Stretch)
    } else {
        None
    }
}

pub(super) fn justify_content_from_stylo(
    justify: &style::values::computed::ContentDistribution,
) -> Option<JustifyContentValue> {
    use style::values::specified::align::AlignFlags;
    let flags = justify.primary();
    let value = flags.value();
    if value == AlignFlags::SPACE_BETWEEN {
        Some(JustifyContentValue::SpaceBetween)
    } else if value == AlignFlags::SPACE_AROUND {
        Some(JustifyContentValue::SpaceAround)
    } else if value == AlignFlags::SPACE_EVENLY {
        Some(JustifyContentValue::SpaceEvenly)
    } else if value == AlignFlags::FLEX_START || value == AlignFlags::START {
        Some(JustifyContentValue::FlexStart)
    } else if value == AlignFlags::FLEX_END || value == AlignFlags::END {
        Some(JustifyContentValue::FlexEnd)
    } else if value == AlignFlags::CENTER {
        Some(JustifyContentValue::Center)
    } else {
        None
    }
}
