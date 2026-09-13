//! The unit-level tests of [`super::values`] — the ones that need no document.
//!
//! One test, from #607 and widened by #592: [`DisplayValue::to_taffy`], which
//! every real style resolution calls on every element.
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
/// [`crate::node::DisplayMode`] does not carry: each `inline-*` value must build
/// the same Taffy container its block-level partner does, because that pair
/// differs only in its *outside*.
///
/// This is the unit-level form of the interior fixtures in
/// `tests/atomic_inline_tests.rs` and `tests/inline_block_block_container_tests.rs`:
/// a fix that made `inline-grid` inline-level by routing it through the
/// `inline-block` or `inline-flex` machinery would answer `taffy::Display::Flex`
/// for it, and a grid's children would then be laid out in a flex row.
///
/// `inline-block` gives `Block` since #592, where it used to give `Flex`, and
/// that one line is half of that fix: an `inline-block`'s inside is a **block
/// container**, so its children stack and its inline runs get anonymous block
/// boxes. Under `Flex` they were laid out in a row.
#[test]
fn each_inline_value_builds_the_same_taffy_container_as_its_block_partner() {
    assert_eq!(DisplayValue::InlineGrid.to_taffy(), taffy::Display::Grid);
    assert_eq!(DisplayValue::Grid.to_taffy(), taffy::Display::Grid);
    assert_eq!(DisplayValue::InlineFlex.to_taffy(), taffy::Display::Flex);
    assert_eq!(DisplayValue::Flex.to_taffy(), taffy::Display::Flex);
    assert_eq!(DisplayValue::InlineBlock.to_taffy(), taffy::Display::Block);
    assert_eq!(DisplayValue::Block.to_taffy(), taffy::Display::Block);
    // The control: the three atomic inlines do **not** all answer the same
    // thing, so "every inline-* value gives X" cannot pass for any X.
    assert_ne!(
        DisplayValue::InlineBlock.to_taffy(),
        DisplayValue::InlineFlex.to_taffy()
    );
    assert_ne!(
        DisplayValue::InlineFlex.to_taffy(),
        DisplayValue::InlineGrid.to_taffy()
    );
}
