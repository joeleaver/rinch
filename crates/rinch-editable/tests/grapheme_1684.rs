//! #1684: a desktop text field's arrows, Delete and Backspace step over a
//! grapheme cluster as Chrome 155 does, so no caret stands inside an emoji
//! ZWJ sequence, a flag, a keycap or a base with its marks.
//!
//! Every row is Chrome 155's answer (Linux, trusted keys over the DevTools
//! protocol into a `<textarea>`, caret placed with `setSelectionRange`; a
//! `contenteditable` answered the same). Notation: `|` is the caret.

use rinch_editable::{EditCommand, EditableDocument, EditableState, Selection, StringDocument};

fn parse(s: &str) -> (String, usize) {
    let at = s.find('|').expect("a caret");
    (s.replacen('|', "", 1), at)
}

fn run(before: &str, command: EditCommand) -> String {
    let (text, caret) = parse(before);
    let mut state = EditableState::new(StringDocument::with_text(&text));
    state.selection = Selection::cursor(caret);
    state.execute(command);
    let mut text = state.document.to_text();
    assert_eq!(state.selection.anchor, state.selection.head, "collapsed");
    text.insert(state.selection.head.0, '|');
    text
}

const FAMILY: &str = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
const HEART: &str = "\u{2764}\u{FE0F}";
const FLAG: &str = "\u{1F1EB}\u{1F1F7}";
const KEYCAP: &str = "1\u{FE0F}\u{20E3}";
const THUMB_TONE: &str = "\u{1F44D}\u{1F3FD}";
const E_ACUTE: &str = "e\u{301}";

#[test]
fn arrows_cross_a_whole_cluster_1684() {
    for c in [FAMILY, HEART, FLAG, KEYCAP, THUMB_TONE, E_ACUTE] {
        assert_eq!(run(&format!("a|{c}b"), EditCommand::MoveRight), format!("a{c}|b"));
        assert_eq!(run(&format!("a{c}|b"), EditCommand::MoveLeft), format!("a|{c}b"));
    }
    // Two flags are two clusters.
    assert_eq!(run(&format!("a{FLAG}{FLAG}|b"), EditCommand::MoveLeft), format!("a{FLAG}|{FLAG}b"));
}

#[test]
fn shift_arrows_extend_by_a_cluster_1684() {
    let (text, _) = parse(&format!("a|{FAMILY}b"));
    let mut state = EditableState::new(StringDocument::with_text(&text));
    state.selection = Selection::cursor(1);
    state.execute(EditCommand::SelectRight);
    assert_eq!(state.selection.head.0, 1 + FAMILY.len());
    state.execute(EditCommand::SelectLeft);
    assert_eq!(state.selection.head.0, 1);
}

#[test]
fn delete_removes_a_whole_cluster_1684() {
    for c in [FAMILY, HEART, FLAG, KEYCAP, THUMB_TONE, E_ACUTE] {
        assert_eq!(run(&format!("a|{c}b"), EditCommand::DeleteForward), "a|b");
    }
}

#[test]
fn backspace_removes_an_emoji_sequence_whole_1684() {
    for c in [FAMILY, HEART, FLAG, KEYCAP, THUMB_TONE] {
        assert_eq!(run(&format!("a{c}|b"), EditCommand::DeleteBackward), "a|b", "{c:?}");
    }
    assert_eq!(run(&format!("a{FLAG}{FLAG}|b"), EditCommand::DeleteBackward), format!("a{FLAG}|b"));
}

/// Chrome's Backspace takes one code point off a cluster that is not an emoji
/// sequence: an accent, a Hangul jamo, a Devanagari or Thai sign.
#[test]
fn backspace_takes_one_mark_off_other_clusters_1684() {
    assert_eq!(run(&format!("a{E_ACUTE}|b"), EditCommand::DeleteBackward), "ae|b");
    assert_eq!(run("a\u{1100}\u{1161}\u{11A8}|b", EditCommand::DeleteBackward), "a\u{1100}\u{1161}|b");
    assert_eq!(run("a\u{915}\u{94D}\u{937}\u{93F}|b", EditCommand::DeleteBackward), "a\u{915}\u{94D}\u{937}|b");
    assert_eq!(run("a\u{E01}\u{E34}\u{E48}|b", EditCommand::DeleteBackward), "a\u{E01}\u{E34}|b");
}
