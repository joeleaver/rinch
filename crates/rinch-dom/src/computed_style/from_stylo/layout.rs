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
        (DisplayOutside::Inline, DisplayInside::Grid) => DisplayValue::InlineGrid,
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

/// Report a sizing value rinch does not implement — **once per property and
/// spelling per process**, on stderr, in the shape `layout_engine`'s calc-cap
/// warning already uses (rinch-dom links no logging crate).
///
/// Returns whether this was the first report, which is the half
/// `intrinsic_sizing_tests` can observe: a test cannot read another process's
/// stderr, but it can assert that the second call answers `false`.
pub(crate) fn note_unsupported_size(prop: &'static str, value: &str) -> bool {
    use std::collections::BTreeSet;
    use std::sync::Mutex;
    // `BTreeSet::new` is const, so this needs no `OnceLock`. The lock is only
    // ever taken for a value rinch cannot lay out, which is rare; the ordinary
    // `auto`/length/percentage path never reaches it.
    static REPORTED: Mutex<BTreeSet<(&'static str, String)>> = Mutex::new(BTreeSet::new());
    let Ok(mut seen) = REPORTED.lock() else {
        return false;
    };
    if !seen.insert((prop, value.to_string())) {
        return false;
    }
    eprintln!(
        "[rinch] `{prop}: {value}` is not implemented; it lays out as `auto`, which \
         matches a browser only where `auto` already gives the same used size \
         (issue #626). Reported once per property and value per process."
    );
    true
}

/// `width`/`height`/`min-width`/`min-height`, and `flex-basis` through
/// [`flex_basis_from_stylo`].
///
/// The four intrinsic keywords survive as [`DimensionValue::Intrinsic`] rather
/// than collapsing into `Auto`. That does **not** make them lay out — they
/// still reach Taffy as `auto` (see that variant) — it makes the declaration
/// visible to a reader of the computed style, and it is what lets the
/// diagnostic above fire exactly once instead of on every style resolution.
pub(super) fn size_from_stylo(
    prop: &'static str,
    size: &style::values::computed::Size,
) -> DimensionValue {
    use style::values::computed::Size;
    let keyword = |k: IntrinsicSize| {
        note_unsupported_size(prop, k.css_name());
        DimensionValue::Intrinsic(k)
    };
    match size {
        Size::Auto => DimensionValue::Auto,
        Size::LengthPercentage(lp) => dimension_from_lp(&lp.0),
        Size::MaxContent => keyword(IntrinsicSize::MaxContent),
        Size::MinContent => keyword(IntrinsicSize::MinContent),
        Size::FitContent => keyword(IntrinsicSize::FitContent),
        Size::Stretch | Size::WebkitFillAvailable => keyword(IntrinsicSize::Stretch),
        // `fit-content(<length-percentage>)` clamps between min-content and
        // max-content, so it is not the bare `fit-content` keyword and cannot
        // borrow its variant. It parses: `stylo_static_prefs`' compile-time
        // `pref!` hard-codes `layout.css.fit-content-function.enabled` to
        // `true`, as it does the `stretch` and `-webkit-fill-available` keys
        // above. `anchor-size()` is CSS anchor positioning, which rinch does not
        // implement at all. The catch-all also absorbs the gecko-only
        // `-moz-available` (cfg'd out of this build) and any variant a future
        // stylo adds.
        Size::FitContentFunction(_) => {
            note_unsupported_size(prop, "fit-content()");
            DimensionValue::Auto
        }
        other => {
            note_unsupported_size(prop, &format!("{other:?}"));
            DimensionValue::Auto
        }
    }
}

/// `max-width`/`max-height`. Same treatment as [`size_from_stylo`]; the one
/// structural difference is that the initial value is `none`, not `auto`, and
/// both spell "no constraint" as `DimensionValue::Auto` here.
pub(super) fn max_size_from_stylo(
    prop: &'static str,
    size: &style::values::computed::MaxSize,
) -> DimensionValue {
    use style::values::computed::MaxSize;
    let keyword = |k: IntrinsicSize| {
        note_unsupported_size(prop, k.css_name());
        DimensionValue::Intrinsic(k)
    };
    match size {
        MaxSize::None => DimensionValue::Auto,
        MaxSize::LengthPercentage(lp) => dimension_from_lp(&lp.0),
        MaxSize::MaxContent => keyword(IntrinsicSize::MaxContent),
        MaxSize::MinContent => keyword(IntrinsicSize::MinContent),
        MaxSize::FitContent => keyword(IntrinsicSize::FitContent),
        MaxSize::Stretch | MaxSize::WebkitFillAvailable => keyword(IntrinsicSize::Stretch),
        MaxSize::FitContentFunction(_) => {
            note_unsupported_size(prop, "fit-content()");
            DimensionValue::Auto
        }
        other => {
            note_unsupported_size(prop, &format!("{other:?}"));
            DimensionValue::Auto
        }
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
        FlexBasis::Size(size) => size_from_stylo("flex-basis", size),
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

#[cfg(test)]
mod diagnostic_tests {
    use super::note_unsupported_size;

    /// The diagnostic is once per (property, value), not once overall and not
    /// once per node — a document with a hundred `width: max-content` boxes
    /// prints one line, and a second *different* property still prints.
    ///
    /// A test cannot read the stderr of its own process, so what is asserted is
    /// the return value the `eprintln!` is gated on. The property names here are
    /// deliberately not real CSS, so no other test in this binary can have
    /// claimed them first.
    #[test]
    fn a_property_and_value_pair_is_reported_exactly_once() {
        assert!(note_unsupported_size("-test-a", "max-content"));
        assert!(!note_unsupported_size("-test-a", "max-content"));
        assert!(!note_unsupported_size("-test-a", "max-content"));
        // Same property, different value.
        assert!(note_unsupported_size("-test-a", "stretch"));
        assert!(!note_unsupported_size("-test-a", "stretch"));
        // Same value, different property.
        assert!(note_unsupported_size("-test-b", "max-content"));
        assert!(!note_unsupported_size("-test-b", "max-content"));
        // And the first pair is still spent.
        assert!(!note_unsupported_size("-test-a", "max-content"));
    }
}
