//! What is left of the string-parsing layer's own tests.
//!
//! These two cover [`DisplayValue::parse`] and [`DimensionValue::parse`]
//! directly. They are kept, rather than removed with the rest of this module,
//! for one reason: those methods are still `pub`, and deleting the only tests
//! of surviving public API is a coverage regression however unreachable that
//! API is.
//!
//! Unreachable is what they now are. `ComputedStyle::from_props` — a second,
//! hand-rolled style resolver that nothing but its own tests called — was their
//! last caller, and it is gone (issue #254). Every real style resolution goes
//! through stylo (`style_resolution/` → `ComputedStyle::from_stylo`), which
//! hands over already-parsed values and never asks a `Value` type to parse a
//! string. 19 of the 20 `parse` methods on those types are now caller-less;
//! only `UserSelectValue::parse` still has one.
//!
//! Removing the other 19 — together with the parallel dead Taffy-style builder
//! in `layout.rs` — is the rest of the same cleanup, tracked separately because
//! it removes public API and deserves its own review. When that lands, this
//! file goes with it.
//!
//! The assertions that were about *behaviour the renderer actually has* moved
//! to `tests/computed_style_tests.rs`, where they run through stylo.

use super::*;
use crate::layout::Viewport;

#[test]
fn test_display_value_parse() {
    assert_eq!(DisplayValue::parse("flex"), DisplayValue::Flex);
    assert_eq!(DisplayValue::parse("block"), DisplayValue::Block);
    assert_eq!(DisplayValue::parse("none"), DisplayValue::None);
    assert_eq!(DisplayValue::parse("inline-flex"), DisplayValue::InlineFlex);
}

#[test]
fn test_dimension_value_parse() {
    let vp = Viewport {
        width: 1000.0,
        height: 800.0,
    };
    assert!(matches!(
        DimensionValue::parse("auto", &vp),
        DimensionValue::Auto
    ));
    assert!(
        matches!(DimensionValue::parse("100px", &vp), DimensionValue::Length(v) if (v - 100.0).abs() < 0.01)
    );
    assert!(
        matches!(DimensionValue::parse("50%", &vp), DimensionValue::Percent(v) if (v - 0.5).abs() < 0.01)
    );
    assert!(
        matches!(DimensionValue::parse("10vh", &vp), DimensionValue::Length(v) if (v - 80.0).abs() < 0.01)
    );
    assert!(
        matches!(DimensionValue::parse("10vw", &vp), DimensionValue::Length(v) if (v - 100.0).abs() < 0.01)
    );
    assert!(
        matches!(DimensionValue::parse("2rem", &vp), DimensionValue::Length(v) if (v - 32.0).abs() < 0.01)
    );
}
