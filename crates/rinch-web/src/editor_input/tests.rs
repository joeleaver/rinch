//! Unit tests for the editor input glue's pure helpers: the capture-target
//! text diff, the parked capture styles and edit intents.

use super::context_menu::{
    HIDDEN_CAPTURE_STYLE, park_around, park_right_of, parked_capture_style, select_all_chosen,
};
use super::mirror::{chars_of_utf16, text_diff, utf16_of_chars};
use super::text_input::{EditIntent, edit_intent};
use super::touch::is_tap;
use super::*;

// ── `beforeinput` → model intent ──────────────────────────────────────────
//
// This mapping is the whole soft-keyboard path: Android reports printable keys as
// `Unidentified` / 229, so `handle_keydown` never sees them and `inputType` is the
// only description of the edit we get.

#[test]
fn plain_typing_inserts_the_events_data() {
    assert_eq!(edit_intent("insertText"), EditIntent::InsertText);
}

#[test]
fn enter_and_shift_enter_split_and_break() {
    assert_eq!(edit_intent("insertParagraph"), EditIntent::Command("enter"));
    assert_eq!(
        edit_intent("insertLineBreak"),
        EditIntent::Command("insertLineBreak")
    );
}

#[test]
fn the_delete_family_maps_to_commands_or_motions() {
    assert_eq!(
        edit_intent("deleteContent"),
        EditIntent::Command("deleteSelection")
    );
    assert_eq!(
        edit_intent("deleteContentBackward"),
        EditIntent::Command("deleteCharBackward")
    );
    assert_eq!(
        edit_intent("deleteContentForward"),
        EditIntent::Command("deleteCharForward")
    );
    // Word deletes are the core's commands (#303), not a motion + delete.
    assert_eq!(
        edit_intent("deleteWordBackward"),
        EditIntent::Command("deleteWordBackward")
    );
    assert_eq!(
        edit_intent("deleteWordForward"),
        EditIntent::Command("deleteWordForward")
    );
    // A soft line is the visual line (#301) — resolved from layout, which
    // `tests/editor_soft_line.rs` drives in Chrome — and a hard line is the
    // textblock, the model's line edge.
    assert_eq!(
        edit_intent("deleteSoftLineBackward"),
        EditIntent::DeleteToVisualLine { forward: false }
    );
    assert_eq!(
        edit_intent("deleteSoftLineForward"),
        EditIntent::DeleteToVisualLine { forward: true }
    );
    assert_eq!(
        edit_intent("deleteHardLineBackward"),
        EditIntent::DeleteTo(CursorMotion::LineStart)
    );
    assert_eq!(
        edit_intent("deleteHardLineForward"),
        EditIntent::DeleteTo(CursorMotion::LineEnd)
    );
}

#[test]
fn history_gestures_reach_the_history_plugin() {
    assert_eq!(edit_intent("historyUndo"), EditIntent::Command("undo"));
    assert_eq!(edit_intent("historyRedo"), EditIntent::Command("redo"));
}

#[test]
fn edits_another_handler_already_applied_are_ignored() {
    // Composition: the composition events show the preedit and commit it. Applying
    // this too would type every composed word twice.
    assert_eq!(edit_intent("insertCompositionText"), EditIntent::Ignore);
    // Clipboard: `on_cut` / `on_paste` ran on the native ClipboardEvent.
    for t in [
        "deleteByCut",
        "insertFromPaste",
        "insertFromPasteAsQuotation",
    ] {
        assert_eq!(edit_intent(t), EditIntent::Ignore);
    }
}

#[test]
fn autocorrect_is_reconciled_rather_than_inserted() {
    // The event says what to insert but not what it replaces, so it is applied to
    // the mirrored textarea and read back off the diff. Inserting `data` at the
    // caret would leave both the typo and the correction.
    assert_eq!(edit_intent("insertReplacementText"), EditIntent::Reconcile);
}

#[test]
fn an_unknown_input_type_is_a_no_op() {
    assert_eq!(edit_intent("insertFromDrop"), EditIntent::Ignore);
    assert_eq!(edit_intent("formatBold"), EditIntent::Ignore);
    assert_eq!(edit_intent(""), EditIntent::Ignore);
}

// ── The mirror diff ───────────────────────────────────────────────────────
//
// What the browser did to the mirrored textarea has to be recovered from the text
// alone: a keyboard's replacement range never reaches us (`getTargetRanges()` is
// empty on a textarea, and a composition's region is not exposed at all).

/// `(from, to, inserted)` rendered as a readable expectation.
fn diff(base: &str, now: &str) -> Option<(usize, usize, String)> {
    text_diff(base, now)
}

#[test]
fn an_unchanged_field_is_no_edit() {
    assert_eq!(diff("hello", "hello"), None);
    assert_eq!(diff("", ""), None);
}

#[test]
fn a_suggestion_narrows_to_the_characters_that_actually_changed() {
    // The reported bug: "word" + a tap on the "world" suggestion. The keyboard
    // replaces the whole word, but the minimal edit is one inserted `l` — which is
    // what keeps the surrounding marks intact and undo granular.
    assert_eq!(diff("word", "world"), Some((3, 3, "l".into())));
    // Mid-block, with text either side that must not be touched.
    assert_eq!(
        diff("hello word here", "hello world here"),
        Some((9, 9, "l".into()))
    );
}

#[test]
fn a_replacement_sharing_no_edges_spans_the_whole_word() {
    assert_eq!(diff("teh", "the"), Some((1, 3, "he".into())));
    // The transposition is the only thing that moved: "ht" → "th", with the
    // sentence either side untouched.
    assert_eq!(
        diff("say hte thing", "say the thing"),
        Some((4, 6, "th".into()))
    );
}

#[test]
fn plain_typing_and_deleting_are_ordinary_diffs() {
    assert_eq!(diff("wor", "word"), Some((3, 3, "d".into())));
    assert_eq!(diff("word", "wor"), Some((3, 4, String::new())));
    assert_eq!(diff("", "a"), Some((0, 0, "a".into())));
    assert_eq!(diff("a", ""), Some((0, 1, String::new())));
}

#[test]
fn a_word_delete_reports_an_empty_insertion() {
    assert_eq!(
        diff("hello world here", "hello  here"),
        Some((6, 11, String::new()))
    );
}

#[test]
fn an_emptied_field_behind_a_live_mirror_reads_as_deleting_the_block() {
    // Why `clear_mirror` empties the field and drops the mirror *together*, and why
    // `reconcile_mirror` re-syncs when the block no longer holds `mirror.text`:
    // a field cleared behind a standing mirror is indistinguishable, at this layer,
    // from the user selecting the paragraph and hitting Backspace. The diff is
    // honest — there is no guard to add here — so the pairing has to hold upstream,
    // in the only two places that write the field.
    assert_eq!(
        diff("a whole paragraph of prose", ""),
        Some((0, 26, String::new()))
    );
}

#[test]
fn the_prefix_and_suffix_scans_never_overlap() {
    // A repeated run is where a naive two-ended scan double-counts and reports a
    // negative-width range. "aaa" → "aa" must be one deletion, not two.
    assert_eq!(diff("aaa", "aa"), Some((2, 3, String::new())));
    assert_eq!(diff("aa", "aaa"), Some((2, 2, "a".into())));
    let (from, to, _) = diff("aaaa", "aa").unwrap();
    assert!(from <= to, "range inverted: {from}..{to}");
}

#[test]
fn offsets_are_chars_so_multibyte_text_maps_correctly() {
    // Char indices, not bytes: the caller turns them into byte offsets against the
    // same string. An accent or an emoji must not shift the range.
    assert_eq!(diff("café", "cafés"), Some((4, 4, "s".into())));
    assert_eq!(diff("naïve", "native"), Some((2, 3, "ti".into())));
    assert_eq!(diff("hi 👋", "hi 👋!"), Some((4, 4, "!".into())));
}

#[test]
fn utf16_lengths_are_what_the_textarea_counts_in() {
    assert_eq!(utf16_of_chars("abc", 3), 3);
    assert_eq!(utf16_of_chars("café", 4), 4);
    // Astral characters are surrogate pairs — two units, one char.
    assert_eq!(utf16_of_chars("👋", 1), 2);
    assert_eq!(utf16_of_chars("hi 👋!", 4), 5);
    // Past the end counts the whole text, so a stale mirror cannot panic.
    assert_eq!(utf16_of_chars("hi 👋!", 99), 6);
    assert_eq!(utf16_of_chars("hi 👋!", 0), 0);
}

#[test]
fn textarea_offsets_convert_back_to_chars() {
    assert_eq!(chars_of_utf16("hi 👋!", 3), 3);
    assert_eq!(chars_of_utf16("hi 👋!", 5), 4);
    // Inside the surrogate pair: the whole character, never half of it.
    assert_eq!(chars_of_utf16("hi 👋!", 4), 4);
    assert_eq!(chars_of_utf16("hi 👋!", 99), 5);
    for text in ["", "plain", "café", "hi 👋 there", "a\nb\u{FFFC}c"] {
        let n = text.chars().count();
        assert_eq!(chars_of_utf16(text, utf16_of_chars(text, n)), n, "{text:?}");
    }
}

#[test]
fn a_textarea_caret_offset_converts_back_to_a_byte_offset() {
    // The shared converter from `event_delegation` — the same one the pointer
    // hit-test uses to read a browser selection offset.
    use crate::event_delegation::utf16_offset_to_utf8_bytes;
    assert_eq!(utf16_offset_to_utf8_bytes("abc", 0), 0);
    assert_eq!(utf16_offset_to_utf8_bytes("abc", 3), 3);
    // `é` is one UTF-16 unit but two bytes.
    assert_eq!(utf16_offset_to_utf8_bytes("café!", 4), 5);
    // `👋` is two UTF-16 units and four bytes: offset 3 is before it.
    assert_eq!(utf16_offset_to_utf8_bytes("hi 👋!", 3), 3);
    assert_eq!(utf16_offset_to_utf8_bytes("hi 👋!", 5), 7);
    // Past the end clamps — a selection read after the field moved on must not
    // panic.
    assert_eq!(utf16_offset_to_utf8_bytes("hi 👋!", 99), "hi 👋!".len());
}

#[test]
fn utf16_offsets_round_trip_through_bytes() {
    use crate::event_delegation::utf16_offset_to_utf8_bytes;
    for text in ["", "plain", "café", "hi 👋 there", "aaa"] {
        let units = utf16_of_chars(text, text.chars().count());
        assert_eq!(
            utf16_offset_to_utf8_bytes(text, units),
            text.len(),
            "{text:?}"
        );
        assert_eq!(utf16_offset_to_utf8_bytes(text, 0), 0, "{text:?}");
    }
}

// ── The context-menu cycle (issue #814) ───────────────────────────────────

#[test]
fn select_all_is_the_whole_field_from_offset_zero() {
    // The menu's Select All over sentinel + 9 characters + sentinel.
    assert!(select_all_chosen(0, 11, 11));
    // The editor's own write: selected between the sentinels.
    assert!(!select_all_chosen(1, 10, 11));
    // A partial selection the user could not have made from the menu — or a
    // word selection taking one sentinel with the text.
    assert!(!select_all_chosen(0, 10, 11));
    assert!(!select_all_chosen(1, 11, 11));
    assert!(!select_all_chosen(2, 7, 11));
    // An empty field selects nothing.
    assert!(!select_all_chosen(0, 0, 0));
    // The empty-selection cycle: the field is the two sentinels, and a word
    // selection there is one of them.
    assert!(select_all_chosen(0, 2, 2));
    assert!(!select_all_chosen(0, 1, 2));
    assert!(!select_all_chosen(1, 2, 2));
    assert!(!select_all_chosen(1, 1, 2));
}

#[test]
fn a_park_is_centred_on_the_pointer_and_starts_at_the_caret() {
    // Off the origin, so a spot that forgot the offset would show.
    assert_eq!(park_around(100.0, 40.0), (85.0, 25.0));
    // The menu key: left edge at the caret x, centred on the line's middle.
    assert_eq!(park_right_of(100.0, 50.0), (100.0, 35.0));
}

#[test]
fn the_parked_style_is_a_hittable_box_at_its_corner() {
    let style = parked_capture_style(85.0, 25.0);
    assert!(style.contains("left: 85px;"), "{style}");
    assert!(style.contains("top: 25px;"), "{style}");
    assert!(style.contains("width: 30px; height: 30px;"), "{style}");
    // Hittable and on top — what the hidden style is not.
    assert!(style.contains("pointer-events: auto;"), "{style}");
    assert!(!style.contains("pointer-events: none"), "{style}");
    assert!(!style.contains("z-index: -1"), "{style}");
    // And still invisible.
    assert!(style.contains("opacity: 0;"), "{style}");
    assert!(HIDDEN_CAPTURE_STYLE.contains("pointer-events: none;"));
}

// ── Tap vs scroll ─────────────────────────────────────────────────────────

#[test]
fn a_still_contact_is_a_tap_and_a_swipe_is_not() {
    assert!(is_tap((100.0, 200.0), (100.0, 200.0)));
    // Fingers wobble; a few px is still a tap.
    assert!(is_tap((100.0, 200.0), (104.0, 197.0)));
    // Right up to the slop on one axis is still a tap.
    let slop = drag_machine::TOUCH_MOVE_SLOP;
    assert!(is_tap((100.0, 200.0), (100.0 + slop, 200.0)));
    assert!(is_tap((100.0, 200.0), (100.0, 200.0 - slop)));
    // A scroll must not raise the keyboard, in either direction or axis.
    assert!(!is_tap((100.0, 200.0), (100.0, 260.0)));
    assert!(!is_tap((100.0, 200.0), (100.0, 140.0)));
    assert!(!is_tap((100.0, 200.0), (160.0, 200.0)));
}
