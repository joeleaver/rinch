//! What is left of the string-parsing layer's own tests.
//!
//! These cover [`DisplayValue::parse`] and [`DimensionValue::parse`] directly.
//! They are kept, rather than removed with the rest of this module, for one
//! reason: those methods are still `pub`, and deleting the only tests of
//! surviving public API is a coverage regression however unreachable that API
//! is.
//!
//! One test here is **not** about string parsing and must not go when the rest
//! does: `inline_grid_builds_a_taffy_grid_like_grid_does` covers
//! [`DisplayValue::to_taffy`], which every real style resolution calls on every
//! element. It lives here because it is a statement about the same enum (#607).
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
    assert_eq!(DisplayValue::parse("grid"), DisplayValue::Grid);
    assert_eq!(DisplayValue::parse("inline-grid"), DisplayValue::InlineGrid);
}

/// `inline-grid` had **no arm at all** in [`DisplayValue::parse`] before #607,
/// so it fell through to `Self::default()` — which is `Flex`, not even `Grid`.
/// The fallback is what makes this worth its own assertion: a missing arm here
/// is silent, and the wrong answer is a *plausible* display value rather than an
/// error.
///
/// The unparsed spellings are the control: `parse` answers `Flex` for anything
/// it does not know, so "every input gives Flex" had to be excluded before the
/// two real arms above mean anything.
#[test]
fn an_unknown_display_string_is_what_falls_back_to_flex_not_inline_grid() {
    assert_eq!(DisplayValue::parse("ruby-text"), DisplayValue::Flex);
    assert_eq!(DisplayValue::parse(""), DisplayValue::Flex);
    assert_ne!(DisplayValue::parse("inline-grid"), DisplayValue::Flex);
    assert_ne!(DisplayValue::parse("inline-grid"), DisplayValue::Grid);
}

/// The **inside** of each display value, which is the half
/// [`crate::node::DisplayMode`] does not carry: `inline-grid` must build a
/// Taffy **grid**, exactly as `grid` does.
///
/// This is the unit-level form of the interior fixture in
/// `tests/atomic_inline_tests.rs`: a fix that made `inline-grid` inline-level by
/// routing it through the `inline-block` or `inline-flex` machinery would answer
/// `taffy::Display::Flex` here, and a grid's children would then be laid out in
/// a flex row.
#[test]
fn inline_grid_builds_a_taffy_grid_like_grid_does() {
    assert_eq!(DisplayValue::InlineGrid.to_taffy(), taffy::Display::Grid);
    assert_eq!(DisplayValue::Grid.to_taffy(), taffy::Display::Grid);
    // The control: the other two atomic inlines are *not* grids, so "every
    // inline-* value gives Grid" cannot pass.
    assert_eq!(DisplayValue::InlineFlex.to_taffy(), taffy::Display::Flex);
    assert_eq!(DisplayValue::InlineBlock.to_taffy(), taffy::Display::Flex);
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
