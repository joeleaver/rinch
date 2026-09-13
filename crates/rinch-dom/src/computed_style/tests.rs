//! The unit-level tests of [`super::values`] — the ones that need no document.
//!
//! One test, from #607: [`DisplayValue::to_taffy`], which every real style
//! resolution calls on every element.
//!
//! This file used to hold the string-parsing tests as well. Their subjects,
//! `DisplayValue::parse` and `DimensionValue::parse`, were the only two of the
//! 19 caller-less `parse` methods on these types that had any test, and they
//! were kept for one reason: those methods were still `pub`, and deleting the
//! only tests of surviving public API is a coverage regression however
//! unreachable that API is. #458 removed the methods — the rest of the
//! pre-stylo engine whose `ComputedStyle::from_props` half went in #254 — so
//! the tests went with them. Every real style resolution goes through stylo
//! (`style_resolution/` → `ComputedStyle::from_stylo`), which hands over
//! already-parsed values and never asks a `Value` type to parse a string.
//!
//! The assertions that were about *behaviour the renderer actually has* live in
//! `tests/computed_style_tests.rs`, where they run through stylo.

use super::*;

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
