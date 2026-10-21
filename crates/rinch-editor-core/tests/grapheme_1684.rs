//! #1684: the editor's arrows, Delete and Backspace step over a grapheme
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

const FAMILY: &str = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
const HEART: &str = "\u{2764}\u{FE0F}";
const FLAG: &str = "\u{1F1EB}\u{1F1F7}";
const KEYCAP: &str = "1\u{FE0F}\u{20E3}";
const THUMB_TONE: &str = "\u{1F44D}\u{1F3FD}";
const E_ACUTE: &str = "e\u{301}";

fn extended(notation: &str, motion: CursorMotion) -> Selection {
    let (state, pos) = editor(notation);
    resolve_cursor_motion(&state.doc, pos, pos, motion, true).expect("a textblock")
}

#[test]
fn arrows_cross_a_whole_cluster_1684() {
    for c in [FAMILY, HEART, FLAG, KEYCAP, THUMB_TONE, E_ACUTE] {
        assert_eq!(
            moved(&format!("a|{c}b"), CursorMotion::CharRight),
            format!("a{c}|b")
        );
        assert_eq!(
            moved(&format!("a{c}|b"), CursorMotion::CharLeft),
            format!("a|{c}b")
        );
        let n = c.chars().count();
        assert_eq!(
            extended(&format!("a|{c}b"), CursorMotion::CharRight).head(),
            Pos(2 + n)
        );
    }
    assert_eq!(
        moved(&format!("a{FLAG}{FLAG}|b"), CursorMotion::CharLeft),
        format!("a{FLAG}|{FLAG}b")
    );
    // An image and a hard break are still one step each.
    assert_eq!(moved("a@|b", CursorMotion::CharLeft), "a|@b");
    assert_eq!(moved("a/|b", CursorMotion::CharLeft), "a|/b");
}

#[test]
fn delete_removes_a_whole_cluster_1684() {
    for c in [FAMILY, HEART, FLAG, KEYCAP, THUMB_TONE, E_ACUTE] {
        assert_eq!(
            deleted(&format!("a|{c}b"), "deleteCharForward"),
            "ab",
            "{c:?}"
        );
    }
}

#[test]
fn backspace_removes_an_emoji_sequence_whole_1684() {
    for c in [FAMILY, HEART, FLAG, KEYCAP, THUMB_TONE] {
        assert_eq!(
            deleted(&format!("a{c}|b"), "deleteCharBackward"),
            "ab",
            "{c:?}"
        );
    }
    assert_eq!(
        deleted(&format!("a{FLAG}{FLAG}|b"), "deleteCharBackward"),
        format!("a{FLAG}b")
    );
}

#[test]
fn backspace_takes_one_mark_off_other_clusters_1684() {
    assert_eq!(
        deleted(&format!("a{E_ACUTE}|b"), "deleteCharBackward"),
        "aeb"
    );
    assert_eq!(
        deleted("a\u{915}\u{94D}\u{937}\u{93F}|b", "deleteCharBackward"),
        "a\u{915}\u{94D}\u{937}b"
    );
}

/// A cluster that spans two text nodes (a mark change inside it) is still one
/// step: the boundaries are the textblock's, not a text node's.
#[test]
fn a_cluster_across_a_mark_boundary_is_one_step_1684() {
    let s = Rc::new(Schema::starter_kit());
    let slice = slice_from_html(&s, "<p>a\u{2764}<b>\u{FE0F}b</b></p>").expect("parses");
    let doc = s.branch("doc", slice.content).expect("a document");
    let sel = resolve_cursor_motion(&doc, Pos(2), Pos(2), CursorMotion::CharRight, false).unwrap();
    assert_eq!(sel.head(), Pos(4));
    let _: &Node = &doc;
}
