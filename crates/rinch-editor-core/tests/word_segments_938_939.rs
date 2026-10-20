//! Word motion, word delete and double-click word selection share one word
//! segmentation (#938, #939), and it is Chrome's where Chrome was measured
//! (Chrome 153, contenteditable, Linux; the tables in the two issues).
//!
//! - #938: an inline atom (an image, a hard break) is a word of its own. The
//!   motion used to read it as a space and step over it with the word beyond.
//! - #939: a word is not only "not whitespace": a run of punctuation is a word
//!   apart from the letters beside it (`foo.bar` is `foo` `.` `bar`), an
//!   apostrophe between letters stays in the word (`don't`), and — with the
//!   `word-dictionary` feature, which the desktop build turns on — a run of
//!   CJK text is split by ICU's dictionary (`漢字です` is `漢字` `です`).
//!
//! Notation: `|` is the caret, `@` an image. The paragraph's content starts at
//! position 1 and every char and atom is one position.

use rinch_editor_core::motion::{CursorMotion, resolve_cursor_motion, word_range_at};
use rinch_editor_core::serialize::slice_from_html;
use rinch_editor_core::{BaseCommandsPlugin, EditorState, Node, Pos, Schema, Selection};
use std::rc::Rc;

/// The paragraph `notation` describes (without its caret) and the caret.
fn editor(notation: &str) -> (EditorState, Pos) {
    let caret = notation
        .chars()
        .position(|c| c == '|')
        .expect("a caret in the notation");
    let html: String = notation
        .chars()
        .filter(|&c| c != '|')
        .map(|c| match c {
            '@' => r#"<img src="x.png">"#.to_string(),
            c => c.to_string(),
        })
        .collect();
    let s = Rc::new(Schema::starter_kit());
    let slice = slice_from_html(&s, &format!("<p>{html}</p>")).expect("parses");
    let doc = s.branch("doc", slice.content).expect("a document");
    let mut state = EditorState::create(s, doc, vec![Rc::new(BaseCommandsPlugin)]);
    let pos = Pos(1 + caret);
    state.selection = Selection::cursor(pos);
    (state, pos)
}

/// The first paragraph's content in the notation (`@` for an image), with `|`
/// at `caret` when given.
fn notation_of(doc: &Node, caret: Option<Pos>) -> String {
    let p = doc.child(0);
    let mut out = String::new();
    let mut pos = 1;
    for i in 0..p.child_count() {
        let child = p.child(i);
        let units: Vec<char> = match child.text() {
            Some(t) => t.chars().collect(),
            None => vec!['@'],
        };
        for c in units {
            if caret == Some(Pos(pos)) {
                out.push('|');
            }
            out.push(c);
            pos += 1;
        }
    }
    if caret == Some(Pos(pos)) {
        out.push('|');
    }
    out
}

fn moved(notation: &str, motion: CursorMotion) -> String {
    let (state, pos) = editor(notation);
    let sel = resolve_cursor_motion(&state.doc, pos, pos, motion, false).expect("a textblock");
    notation_of(&state.doc, Some(sel.head()))
}

fn deleted(notation: &str, command: &str) -> String {
    let (state, _) = editor(notation);
    let after = state.run(command).expect("the delete applies");
    notation_of(&after.doc, None)
}

/// #938's table, measured in Chrome 153: the image is a stop of its own.
#[test]
fn word_motion_stops_at_an_inline_atom_938() {
    for (start, motion, chrome) in [
        ("foo@|bar", CursorMotion::WordLeft, "foo|@bar"),
        ("foo@bar|", CursorMotion::WordLeft, "foo@|bar"),
        ("foo |@ bar", CursorMotion::WordRight, "foo @| bar"),
        ("foo|@bar", CursorMotion::WordRight, "foo@|bar"),
    ] {
        assert_eq!(moved(start, motion), chrome, "{start} {motion:?}");
    }
}

/// #938's related measurements: whitespace between the caret and an image goes
/// with the image, as it goes with a word.
#[test]
fn word_delete_takes_the_whitespace_and_the_atom_beyond_it_938() {
    assert_eq!(deleted("foo@  |", "deleteWordBackward"), "foo");
    assert_eq!(deleted("foo @ |bar", "deleteWordBackward"), "foo bar");
    // The mirror, forward.
    assert_eq!(deleted("foo|  @bar", "deleteWordForward"), "foobar");
}

/// #939's table, measured in Chrome 153 (Ctrl+Backspace).
#[test]
fn word_delete_splits_punctuation_from_letters_939() {
    for (start, chrome) in [
        ("foo.bar| baz", "foo. baz"),
        ("foo...|", "foo"),
        ("don't|", ""),
        ("a 👨\u{200d}👩\u{200d}👧| b", "a  b"),
    ] {
        assert_eq!(deleted(start, "deleteWordBackward"), chrome, "{start}");
    }
    // Forward and the motion, off the table: the same segments from the
    // other side.
    assert_eq!(deleted("foo|.bar", "deleteWordForward"), "foobar");
    assert_eq!(deleted("|foo.bar", "deleteWordForward"), ".bar");
    assert_eq!(moved("foo.bar|", CursorMotion::WordLeft), "foo.|bar");
    assert_eq!(moved("foo.|bar", CursorMotion::WordLeft), "foo|.bar");
    assert_eq!(moved("|foo...bar", CursorMotion::WordRight), "foo|...bar");
    assert_eq!(moved("foo|...bar", CursorMotion::WordRight), "foo...|bar");
}

/// A combining mark belongs to the letter before it, not to a punctuation run.
#[test]
fn a_combining_mark_stays_with_its_letter_939() {
    // "cafe" + U+0301, then "!".
    assert_eq!(deleted("cafe\u{301}|!", "deleteWordBackward"), "!");
    assert_eq!(
        deleted("cafe\u{301}!|", "deleteWordBackward"),
        "cafe\u{301}"
    );
}

/// The delete removes exactly what the motion selects, for every caret in a
/// paragraph mixing words, punctuation, atoms and whitespace (the clamp #930
/// added for atoms is gone; the motion and the delete cannot disagree).
#[test]
fn word_delete_is_the_motions_range_everywhere_938() {
    let text = "ab @ cd@ef..gh  @@ ij,k";
    let n = text.chars().count();
    for caret in 0..=n {
        let mut notation: String = text.chars().take(caret).collect();
        notation.push('|');
        notation.extend(text.chars().skip(caret));
        for (command, motion) in [
            ("deleteWordBackward", CursorMotion::WordLeft),
            ("deleteWordForward", CursorMotion::WordRight),
        ] {
            let (state, pos) = editor(&notation);
            let sel = resolve_cursor_motion(&state.doc, pos, pos, motion, true).unwrap();
            let chars: Vec<char> = text.chars().collect();
            let (from, to) = (sel.from().0 - 1, sel.to().0 - 1);
            let expect: String = chars[..from].iter().chain(chars[to..].iter()).collect();
            match state.run(command) {
                Some(after) => {
                    assert!(!sel.is_empty(), "{notation} {command}: motion did not move");
                    assert_eq!(
                        notation_of(&after.doc, None),
                        expect,
                        "{notation} {command}"
                    );
                }
                None => assert!(sel.is_empty(), "{notation} {command}"),
            }
        }
    }
}

/// Double-click selects the same segments.
#[test]
fn double_click_selects_one_segment_939() {
    let at = |notation: &str| {
        let (state, pos) = editor(notation);
        let (a, b) = word_range_at(&state.doc, pos);
        let mut s = notation_of(&state.doc, None);
        let chars: Vec<char> = s.chars().collect();
        s = chars[a.0 - 1..b.0 - 1].iter().collect();
        s
    };
    assert_eq!(at("foo.b|ar"), "bar");
    assert_eq!(at("fo|o.bar"), "foo");
    assert_eq!(at("foo.|..bar"), "...");
    assert_eq!(at("do|n't"), "don't");
    assert_eq!(at("ab|@cd"), "@");
}

/// With ICU's dictionary (the desktop build), a CJK run splits into words.
#[cfg(feature = "word-dictionary")]
#[test]
fn a_cjk_run_is_split_by_the_dictionary_939() {
    assert_eq!(
        deleted("漢字です| テスト", "deleteWordBackward"),
        "漢字 テスト"
    );
    assert_eq!(
        moved("漢字です| テスト", CursorMotion::WordLeft),
        "漢字|です テスト"
    );
    assert_eq!(
        moved("|漢字です テスト", CursorMotion::WordRight),
        "漢字|です テスト"
    );
    // A Thai run too (ICU's own example: ทุก สอง สัปดาห์).
    assert_eq!(deleted("ทุกสองสัปดาห์|", "deleteWordBackward"), "ทุกสอง");
}

/// Without the dictionary (a build that does not ask for it: the web's default),
/// a CJK run is one word, as it was before #939.
#[cfg(not(feature = "word-dictionary"))]
#[test]
fn without_the_dictionary_a_cjk_run_is_one_word_939() {
    assert_eq!(deleted("漢字です| テスト", "deleteWordBackward"), " テスト");
    // But it still ends at punctuation and at a change of script.
    assert_eq!(deleted("abc漢字です|", "deleteWordBackward"), "abc");
    assert_eq!(deleted("漢字。です|", "deleteWordBackward"), "漢字。");
}
