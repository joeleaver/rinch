//! Review of #1613, F2: a word motion, word delete or double-click segments
//! only the stretch between the whitespace around the caret, not the whole
//! textblock: in spaced text its work is the word's, not the paragraph's. In
//! text written without spaces (CJK, Thai) that stretch is the whole paragraph,
//! and the second test says so.
//! Counted by `motion::segmented_units` (the test-only `cost-counters`
//! feature, which `cargo test --workspace` turns on through
//! rinch-editor-collab's dev-dependency):
//!
//! ```text
//! cargo test -p rinch-editor-core --features cost-counters --test word_segment_cost_1613
//! ```
#![cfg(feature = "cost-counters")]

use rinch_editor_core::motion::{
    CursorMotion, resolve_cursor_motion, segmented_units, word_range_at,
};
use rinch_editor_core::{BaseCommandsPlugin, EditorState, Fragment, Pos, Schema, Selection};
use std::rc::Rc;

/// Units segmented by one Ctrl+Right, one Ctrl+Left, one Ctrl+Backspace and
/// one double-click from the middle of a paragraph of `reps` copies of
/// "lorem ipsum, dolor. ".
fn units_per_round(reps: usize) -> usize {
    let s = Rc::new(Schema::starter_kit());
    let text = "lorem ipsum, dolor. ".repeat(reps);
    let p = s
        .branch("paragraph", Fragment::from_node(s.text(&text).unwrap()))
        .unwrap();
    let doc = s.branch("doc", Fragment::from_node(p)).unwrap();
    // Inside "ipsum" of the middle copy.
    let mid = Pos(1 + 20 * (reps / 2) + 8);
    let before = segmented_units();
    for motion in [CursorMotion::WordRight, CursorMotion::WordLeft] {
        resolve_cursor_motion(&doc, mid, mid, motion, false).unwrap();
    }
    word_range_at(&doc, mid);
    let mut state = EditorState::create(s, doc, vec![Rc::new(BaseCommandsPlugin)]);
    state.selection = Selection::cursor(mid);
    state.run("deleteWordBackward").unwrap();
    segmented_units() - before
}

#[test]
fn word_segmentation_work_does_not_grow_with_the_paragraph() {
    let small = units_per_round(50);
    let large = units_per_round(5000);
    // Each of the four segments "ipsum," (6 units): 24.
    assert_eq!(small, 24);
    assert_eq!(large, small, "the work is the word's, not the block's");
}

/// The limit, stated: in a CJK paragraph with no whitespace the stretch is the
/// whole paragraph (round-2 review of #1613, D1).
#[test]
fn in_text_without_spaces_the_stretch_is_the_paragraph() {
    let s = Rc::new(Schema::starter_kit());
    let text = "漢字です".repeat(150);
    let p = s
        .branch("paragraph", Fragment::from_node(s.text(&text).unwrap()))
        .unwrap();
    let doc = s.branch("doc", Fragment::from_node(p)).unwrap();
    let before = segmented_units();
    resolve_cursor_motion(&doc, Pos(301), Pos(301), CursorMotion::WordRight, false).unwrap();
    assert_eq!(segmented_units() - before, 600);
}
