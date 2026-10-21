//! Review of #1687 (copied helpers from grapheme_1684.rs). #1684: the editor's arrows, Delete and Backspace step over a grapheme
//! cluster as Chrome 155 does in a `contenteditable` (Linux, trusted keys over
//! the DevTools protocol), so the selection never stands inside an emoji ZWJ
//! sequence, a flag, a keycap or a base with its marks. Arrows and Delete take
//! the whole cluster; Backspace takes an emoji sequence whole and one code
//! point off any other cluster.
//!
//! Notation: `|` is the caret, `@` an image, `/` a hard break. The paragraph's
//! content starts at position 1 and every char and atom is one position.

use rinch_editor_core::motion::{CursorMotion, resolve_cursor_motion};
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
            '/' => "<br>".to_string(),
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
            None if child.type_name() == "hard_break" => vec!['/'],
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

/// A combining mark right after a hard break is its own cluster: the break is
/// one step (kills `grapheme_text` mapping a hard break to U+FFFC).
#[test]
fn review_1687_mark_after_a_hard_break() {
    assert_eq!(moved("a|/\u{301}b", CursorMotion::CharRight), "a/|\u{301}b");
    assert_eq!(moved("a/\u{301}|b", CursorMotion::CharLeft), "a/|\u{301}b");
}

/// A combining mark right after an image: in a browser the image is a
/// replaced element and the mark starts a cluster of its own, so ArrowRight
/// from before the image stops right after it.
#[test]
#[ignore = "#1693: a mark after an inline image is grouped with it"]
fn review_1687_mark_after_an_image() {
    assert_eq!(moved("a|@\u{301}b", CursorMotion::CharRight), "a@|\u{301}b");
    assert_eq!(moved("a@\u{301}|b", CursorMotion::CharLeft), "a@|\u{301}b");
}

/// A caret set programmatically inside a cluster: the arrows leave it to the
/// cluster's edges, Backspace deletes back to the cluster start.
#[test]
fn review_1687_caret_inside_a_cluster() {
    let fam = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    // caret after the first ZWJ (3 chars into the 5-char cluster? use 2)
    let n = "a\u{1F468}\u{200D}|\u{1F469}\u{200D}\u{1F467}b".to_string();
    assert_eq!(moved(&n, CursorMotion::CharRight), format!("a{fam}|b"));
    assert_eq!(moved(&n, CursorMotion::CharLeft), format!("a|{fam}b"));
    assert_eq!(
        deleted(&n, "deleteCharBackward"),
        "a\u{1F469}\u{200D}\u{1F467}b"
    );
}

/// Per-keystroke cost of the grapheme step on a long paragraph (release, run
/// by hand): CharRight and Backspace at the middle of an N-char paragraph.
#[test]
#[ignore]
fn review_1687_perf_long_paragraph() {
    for n in [1_000usize, 10_000, 100_000, 1_000_000] {
        let text: String = "abcdefghi ".repeat(n / 10);
        let s = Rc::new(Schema::starter_kit());
        let slice = slice_from_html(&s, &format!("<p>{text}</p>")).expect("parses");
        let doc = s.branch("doc", slice.content).expect("a document");
        let mut state = EditorState::create(s, doc, vec![Rc::new(BaseCommandsPlugin)]);
        let mid = Pos(1 + n / 2);
        state.selection = Selection::cursor(mid);
        let reps = 200;
        let t = std::time::Instant::now();
        for _ in 0..reps {
            let _ = resolve_cursor_motion(&state.doc, mid, mid, CursorMotion::CharRight, false);
        }
        let right = t.elapsed() / reps;
        let t = std::time::Instant::now();
        for _ in 0..reps {
            let _ = state.run("deleteCharBackward");
        }
        let bs = t.elapsed() / reps;
        let t = std::time::Instant::now();
        for _ in 0..reps {
            let _ = resolve_cursor_motion(&state.doc, mid, mid, CursorMotion::WordRight, false);
        }
        let word = t.elapsed() / reps;
        println!("PERF n={n}: CharRight {right:?}, deleteCharBackward {bs:?}, WordRight {word:?}");
    }
}
