//! #1619: a desktop text field's word motion and word delete split words as
//! the rich editor does (`rinch-words`, the splitter #1613 gave the editor),
//! not on whitespace alone.
//!
//! Every row is Chrome 155's answer (Linux, chromedriver 155, trusted
//! Ctrl+key presses into an `<input>` or a `<textarea>`, caret placed with
//! `setSelectionRange`). Notation: `|` is the caret before the key.

use rinch_editable::{EditCommand, EditableDocument, EditableState, Selection, StringDocument};

/// Split `"ab|cd"` into the value and the caret's byte offset.
fn parse(s: &str) -> (String, usize) {
    let at = s.find('|').expect("a caret");
    (s.replacen('|', "", 1), at)
}

fn show(state: &EditableState<StringDocument>) -> String {
    let mut text = state.document.to_text();
    assert_eq!(state.selection.anchor, state.selection.head, "collapsed");
    text.insert(state.selection.head.0, '|');
    text
}

/// The value and caret after `command` from `before`.
fn run(before: &str, command: EditCommand) -> String {
    let (text, caret) = parse(before);
    let mut state = EditableState::new(StringDocument::with_text(&text));
    state.selection = Selection::cursor(caret);
    state.execute(command);
    show(&state)
}

fn back(before: &str) -> String {
    run(before, EditCommand::DeleteWordBackward)
}
fn fwd(before: &str) -> String {
    run(before, EditCommand::DeleteWordForward)
}
fn left(before: &str) -> String {
    run(before, EditCommand::MoveWordLeft)
}
fn right(before: &str) -> String {
    run(before, EditCommand::MoveWordRight)
}

/// The issue's table: punctuation is a word apart from the letters beside it.
#[test]
fn ctrl_backspace_stops_at_punctuation_1619() {
    assert_eq!(back("foo-bar|"), "foo-|");
    assert_eq!(back("path/to/file|"), "path/to/|");
    assert_eq!(back("hello world!!!|"), "hello world|");
    assert_eq!(back("a.b.c|"), "a.b.|");
    assert_eq!(back("foo...|"), "foo|");
    assert_eq!(back("foo--bar|"), "foo--|");
    assert_eq!(back("x foo.|"), "x foo|");
}

#[test]
fn ctrl_delete_stops_at_punctuation_1619() {
    assert_eq!(fwd("|foo-bar"), "|-bar");
    assert_eq!(fwd("x |foo."), "x |.");
}

/// A number with a separator, a contraction and `snake_case` stay one word.
#[test]
fn numbers_contractions_and_underscores_are_one_word_1619() {
    assert_eq!(back("pi 3.14|"), "pi |");
    assert_eq!(back("1,000| x"), "| x");
    assert_eq!(back("don't|"), "|");
    assert_eq!(right("|snake_case x"), "snake_case| x");
}

#[test]
fn ctrl_arrows_stop_at_punctuation_1619() {
    assert_eq!(right("|foo-bar"), "foo|-bar");
    assert_eq!(right("foo|-bar"), "foo-|bar");
    assert_eq!(left("foo-bar|"), "foo-|bar");
    assert_eq!(left("foo-|bar"), "foo|-bar");
    assert_eq!(right("|path/to/file"), "path|/to/file");
    assert_eq!(right("|a, b"), "a|, b");
    assert_eq!(left("a, b|"), "a, |b");
    // Whitespace is skipped first, then one segment.
    assert_eq!(right("foo|  -bar"), "foo  -|bar");
    assert_eq!(left("foo-  bar|"), "foo-  |bar");
    assert_eq!(left("foo |bar"), "|foo bar");
    assert_eq!(right("foo| bar"), "foo bar|");
}

/// A `<textarea>`'s line break: Ctrl+Right at a line's end moves past it and
/// no further; Ctrl+Left never leaves the caret's line, and at a line's start
/// does not move.
#[test]
fn a_line_break_bounds_the_word_motion_1619() {
    assert_eq!(right("ab cd|\nef"), "ab cd\n|ef");
    assert_eq!(right("ab|\n\ncd"), "ab\n|\ncd");
    assert_eq!(right("ab \n| cd"), "ab \n cd|");
    assert_eq!(left("ab \n |cd"), "ab \n| cd");
    assert_eq!(left("ab-\n|cd"), "ab-\n|cd");
    assert_eq!(left("ab cd\n|ef"), "ab cd\n|ef");
    assert_eq!(left("ab\n\n|cd"), "ab\n\n|cd");
    assert_eq!(left("x ab\n|cd"), "x ab\n|cd");
    assert_eq!(left("ab cd|\nef"), "ab |cd\nef");
    assert_eq!(right("ab \n|-cd"), "ab \n-|cd");
}

/// #1616's rules hold with the new words: a word delete stops at a line
/// break, and right beside one deletes just the break.
#[test]
fn a_word_delete_still_stops_at_a_line_break_1619() {
    assert_eq!(back("ab\ncd-ef|"), "ab\ncd-|");
    assert_eq!(back("ab cd\nef|"), "ab cd\n|");
    assert_eq!(fwd("ab cd|\nef"), "ab cd|ef");
}
