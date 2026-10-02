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

/// #505 — the full audit of `ComputedStyle::for_anonymous_box`'s inherited
/// copy list. Each field below is set to a value that differs from
/// `ComputedStyle::default()`, so dropping any one of them from the
/// function's literal (falling through to `..Self::default()`) makes this
/// test fail on that field specifically — the failure message names which.
///
/// `text_decoration` and `user_select` have their own behavioural fixtures
/// (`crates/rinch-dom/tests/ifc_anonymous_box_tests.rs`'s paint-ink witness,
/// `crates/rinch/src/app/text_selection.rs`'s `find_selectable_ifc` witness)
/// because the issue asked for consumer-level proof on those two; they are
/// included here too so this one test is a complete map of the list — a
/// future addition to the list that forgets a line here is caught by the
/// same mechanism as the other eighteen.
///
/// `display` is handled separately by `for_anonymous_box` (it is
/// definitionally `block`, except under a `display: contents` parent) and is
/// deliberately not part of this sweep.
#[test]
fn for_anonymous_box_copies_every_entry_on_its_inherited_list() {
    let parent = ComputedStyle {
        scrollbar_color: ScrollbarColorValue {
            thumb: Some(peniko::Color::from_rgba8(1, 2, 3, 255)),
            track: Some(peniko::Color::from_rgba8(4, 5, 6, 255)),
        },
        color: Some(peniko::Color::from_rgba8(7, 8, 9, 255)),
        visibility: VisibilityValue::Hidden,
        text_shadow: vec![TextShadowValue {
            offset_x: 1.0,
            offset_y: 2.0,
            blur_radius: 3.0,
            color: Some(peniko::Color::from_rgba8(10, 11, 12, 255)),
        }],
        cursor: CursorValue::Pointer,
        pointer_events: PointerEventsValue::None,
        user_select: UserSelectValue::Text,
        font_size: 33.0,
        font_weight: 733.0,
        font_family: "Comic Sans MS".to_string(),
        font_style: FontStyleValue::Italic,
        line_height: LineHeightValue::Absolute(41.0),
        letter_spacing: 5.0,
        word_spacing: 6.0,
        text_align: TextAlignValue::Center,
        text_decoration: TextDecorationValue {
            underline: true,
            strikethrough: true,
            style: TextDecorationStyleValue::Dashed,
            color: Some(peniko::Color::from_rgba8(13, 14, 15, 255)),
        },
        text_transform: TextTransformValue::Uppercase,
        text_underline_offset: Some(7.0),
        white_space: WhiteSpaceValue::Pre,
        overflow_wrap: OverflowWrapValue::BreakWord,
        ..Default::default()
    };

    // Every value above must actually differ from the default, or a dropped
    // field would read back equal to the parent's by coincidence rather than
    // by being copied (CLAUDE.md "fixed-point blindness").
    let default = ComputedStyle::default();
    assert_ne!(parent.scrollbar_color, default.scrollbar_color);
    assert_ne!(parent.color, default.color);
    assert_ne!(parent.visibility, default.visibility);
    assert_ne!(parent.text_shadow, default.text_shadow);
    assert_ne!(parent.cursor, default.cursor);
    assert_ne!(parent.pointer_events, default.pointer_events);
    assert_ne!(parent.user_select, default.user_select);
    assert_ne!(parent.font_size, default.font_size);
    assert_ne!(parent.font_weight, default.font_weight);
    assert_ne!(parent.font_family, default.font_family);
    assert_ne!(parent.font_style, default.font_style);
    assert!(
        !matches!(parent.line_height, LineHeightValue::Normal),
        "line_height must differ from the default `Normal`"
    );
    assert!(matches!(default.line_height, LineHeightValue::Normal));
    assert_ne!(parent.letter_spacing, default.letter_spacing);
    assert_ne!(parent.word_spacing, default.word_spacing);
    assert_ne!(parent.text_align, default.text_align);
    assert_ne!(parent.text_decoration, default.text_decoration);
    assert_ne!(parent.text_transform, default.text_transform);
    assert_ne!(parent.text_underline_offset, default.text_underline_offset);
    assert_ne!(parent.white_space, default.white_space);
    assert_ne!(parent.overflow_wrap, default.overflow_wrap);

    let anon = ComputedStyle::for_anonymous_box(&parent);
    assert_eq!(
        anon.scrollbar_color, parent.scrollbar_color,
        "scrollbar_color"
    );
    assert_eq!(anon.color, parent.color, "color");
    assert_eq!(anon.visibility, parent.visibility, "visibility");
    assert_eq!(anon.text_shadow, parent.text_shadow, "text_shadow");
    assert_eq!(anon.cursor, parent.cursor, "cursor");
    assert_eq!(anon.pointer_events, parent.pointer_events, "pointer_events");
    assert_eq!(anon.user_select, parent.user_select, "user_select");
    assert_eq!(anon.font_size, parent.font_size, "font_size");
    assert_eq!(anon.font_weight, parent.font_weight, "font_weight");
    assert_eq!(anon.font_family, parent.font_family, "font_family");
    assert_eq!(anon.font_style, parent.font_style, "font_style");
    assert!(
        matches!(anon.line_height, LineHeightValue::Absolute(v) if (v - 41.0).abs() < 1e-6),
        "line_height: expected {:?}, got {:?}",
        parent.line_height,
        anon.line_height
    );
    assert_eq!(anon.letter_spacing, parent.letter_spacing, "letter_spacing");
    assert_eq!(anon.word_spacing, parent.word_spacing, "word_spacing");
    assert_eq!(anon.text_align, parent.text_align, "text_align");
    assert_eq!(
        anon.text_decoration, parent.text_decoration,
        "text_decoration"
    );
    assert_eq!(anon.text_transform, parent.text_transform, "text_transform");
    assert_eq!(
        anon.text_underline_offset, parent.text_underline_offset,
        "text_underline_offset"
    );
    assert_eq!(anon.white_space, parent.white_space, "white_space");
    assert_eq!(anon.overflow_wrap, parent.overflow_wrap, "overflow_wrap");
}
