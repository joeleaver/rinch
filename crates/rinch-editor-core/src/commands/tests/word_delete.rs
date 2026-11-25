//! Word delete (issue #303).

use super::*;

// ── word delete (issue #303) ─────────────────────────────────────────────

/// An editor over `html`, its nodes minted by the state's own schema (node
/// and mark types compare by identity, #217), with a caret at `caret`.
fn html_editor(html: &str, caret: usize) -> EditorState {
    let s = Rc::new(Schema::starter_kit());
    let slice = crate::serialize::slice_from_html(&s, html).expect("parses");
    let doc = s.branch("doc", slice.content).expect("a document");
    let mut state = EditorState::create(s, doc, vec![Rc::new(BaseCommandsPlugin)]);
    state.selection = Selection::cursor(Pos(caret));
    state
}

/// The document as `html`, for comparing a whole result at once.
fn html_of(state: &EditorState) -> String {
    crate::serialize::node_to_html(&state.doc)
}

/// Inside a word the delete stops at the word's start / end — off the
/// word's own edge, so a delete-to-the-block-edge would be told apart.
/// "hello world": "hello" 1..6, " " 6..7, "world" 7..12; the caret sits
/// in "wor|ld" at 10.
#[test]
fn word_delete_mid_word_stops_at_the_words_edge() {
    let back = html_editor("<p>hello world</p>", 10)
        .run("deleteWordBackward")
        .expect("applies");
    assert_eq!(all_text(&back.doc), "hello ld");
    assert_eq!(back.selection, Selection::cursor(Pos(7)));
    let fwd = html_editor("<p>hello world</p>", 10)
        .run("deleteWordForward")
        .expect("applies");
    assert_eq!(all_text(&fwd.doc), "hello wor");
    assert_eq!(fwd.selection, Selection::cursor(Pos(10)));
}

/// The whitespace between the caret and the word goes with the word, as
/// the word motion skips it (Ctrl+Left/Right land on the same boundaries).
#[test]
fn word_delete_takes_the_whitespace_before_the_word() {
    // "hello  world": caret after the two spaces, at 8.
    let back = html_editor("<p>hello  world</p>", 8)
        .run("deleteWordBackward")
        .unwrap();
    assert_eq!(all_text(&back.doc), "world");
    // Caret right after "hello", at 6: forward eats the spaces and "world".
    let fwd = html_editor("<p>hello  world</p>", 6)
        .run("deleteWordForward")
        .unwrap();
    assert_eq!(all_text(&fwd.doc), "hello");
}

/// The deleted range is exactly the word motion's: whatever
/// `resolve_cursor_motion(WordLeft/WordRight)` selects is what goes.
#[test]
fn word_delete_deletes_what_the_word_motion_selects() {
    use crate::motion::{CursorMotion, resolve_cursor_motion};
    let text = "one two  three four";
    for caret in 1..=(text.chars().count() + 1) {
        for (name, motion) in [
            ("deleteWordBackward", CursorMotion::WordLeft),
            ("deleteWordForward", CursorMotion::WordRight),
        ] {
            let state = html_editor(&format!("<p>{text}</p>"), caret);
            let sel = resolve_cursor_motion(&state.doc, Pos(caret), Pos(caret), motion, true)
                .expect("a textblock");
            let Some(after) = state.run(name) else {
                assert!(sel.is_empty(), "{name} at {caret}: the motion moved");
                continue;
            };
            let (from, to) = (sel.from().0, sel.to().0);
            let chars: Vec<char> = text.chars().collect();
            let expect: String = chars[..from - 1]
                .iter()
                .chain(chars[to - 1..].iter())
                .collect();
            assert_eq!(all_text(&after.doc), expect, "{name} at {caret}");
        }
    }
}

/// At a textblock's start / end the word motion has nowhere to go, and a
/// word delete means what Backspace / Delete mean there: join the blocks.
#[test]
fn word_delete_at_a_block_edge_joins_like_backspace_and_delete() {
    // "ab" 1..3, second paragraph's content opens at 5.
    let back = html_editor("<p>ab</p><p>cd</p>", 5)
        .run("deleteWordBackward")
        .expect("joins backward");
    assert_eq!(back.doc.child_count(), 1);
    assert_eq!(all_text(&back.doc), "abcd");
    assert_eq!(
        html_of(&back),
        html_of(
            &html_editor("<p>ab</p><p>cd</p>", 5)
                .run("deleteCharBackward")
                .unwrap()
        )
    );
    let fwd = html_editor("<p>ab</p><p>cd</p>", 3)
        .run("deleteWordForward")
        .expect("joins forward");
    assert_eq!(fwd.doc.child_count(), 1);
    assert_eq!(all_text(&fwd.doc), "abcd");
    // A list item's start lifts, as Backspace does there.
    let li = html_editor("<p>x</p><ul><li><p>item</p></li></ul>", 6);
    assert_eq!(
        li.run("deleteWordBackward").map(|s| html_of(&s)),
        li.run("deleteCharBackward").map(|s| html_of(&s)),
    );
    // At the very start / end of the document both are inapplicable alike.
    let start = html_editor("<p>ab</p>", 1);
    assert!(!start.can_run("deleteWordBackward"));
    assert_eq!(
        start.can_run("deleteWordBackward"),
        start.can_run("deleteCharBackward")
    );
    let end = html_editor("<p>ab</p>", 3);
    assert!(!end.can_run("deleteWordForward"));
}

/// A non-empty selection is deleted as it stands, not widened to words.
#[test]
fn word_delete_with_a_selection_deletes_the_selection() {
    for name in ["deleteWordBackward", "deleteWordForward"] {
        let mut state = html_editor("<p>hello world</p>", 1);
        state.selection = Selection::text(Pos(3), Pos(5)); // "ll"
        let after = state.run(name).expect("applies");
        assert_eq!(all_text(&after.doc), "heo world", "{name}");
    }
}

/// A mark boundary is not a word boundary: the word runs straight across
/// it. "foo " 1..5, bold "bar" 5..8, "baz" 8..11.
#[test]
fn word_delete_runs_across_a_mark_boundary() {
    let html = "<p>foo <strong>bar</strong>baz</p>";
    let back = html_editor(html, 11).run("deleteWordBackward").unwrap();
    assert_eq!(all_text(&back.doc), "foo ");
    assert!(!doc_has_mark(&back.doc, "bold"));
    let fwd = html_editor(html, 5).run("deleteWordForward").unwrap();
    assert_eq!(all_text(&fwd.doc), "foo ");
    // From the seam itself (8), backward takes the bold part only.
    let seam = html_editor(html, 8).run("deleteWordBackward").unwrap();
    assert_eq!(all_text(&seam.doc), "foo baz");
    assert!(!doc_has_mark(&seam.doc, "bold"));
}

fn count_type(doc: &Node, name: &str) -> usize {
    let mut n = 0;
    doc.nodes_between(0, doc.content_size(), &mut |node, _, _| {
        if node.type_name() == name {
            n += 1;
        }
        true
    });
    n
}

/// An inline atom (an image, a hard break) is a word of its own (#938): an
/// adjacent atom goes on its own, like Backspace / Delete would take it, and
/// a word beyond one stays.
/// `foo<img>bar`: "foo" 1..4, the image 4..5, "bar" 5..8.
#[test]
fn word_delete_stops_at_an_inline_atom() {
    for (atom, html) in [
        ("image", r#"<p>foo<img src="x.png">bar</p>"#),
        ("hard_break", "<p>foo<br>bar</p>"),
    ] {
        // Right after the atom: the atom alone.
        let back = html_editor(html, 5).run("deleteWordBackward").unwrap();
        assert_eq!(count_type(&back.doc, atom), 0, "{atom}: backward");
        assert_eq!(all_text(&back.doc), "foobar", "{atom}: backward");
        // Right before it, forward: the atom alone.
        let fwd = html_editor(html, 4).run("deleteWordForward").unwrap();
        assert_eq!(count_type(&fwd.doc, atom), 0, "{atom}: forward");
        assert_eq!(all_text(&fwd.doc), "foobar", "{atom}: forward");
        // Past the atom by a word: the word only, the atom stays.
        let word = html_editor(html, 8).run("deleteWordBackward").unwrap();
        assert_eq!(count_type(&word.doc, atom), 1, "{atom}: word before");
        assert_eq!(all_text(&word.doc), "foo", "{atom}: word before");
        let word = html_editor(html, 1).run("deleteWordForward").unwrap();
        assert_eq!(count_type(&word.doc, atom), 1, "{atom}: word after");
        assert_eq!(all_text(&word.doc), "bar", "{atom}: word after");
    }
    // Whitespace between the caret and an atom goes with the atom, as it
    // goes with a word (#938, Chrome 153): "foo" 1..4, the image 4..5,
    // "  " 5..7, caret at 7.
    let gap = html_editor(r#"<p>foo<img src="x.png">  </p>"#, 7)
        .run("deleteWordBackward")
        .unwrap();
    assert_eq!(count_type(&gap.doc, "image"), 0);
    assert_eq!(all_text(&gap.doc), "foo");
}

/// An atom further away than the word boundary does not move the delete:
/// only one *between* the caret and the boundary clamps it. Off the fixed
/// point where the atom sits exactly at the boundary (review of #930).
#[test]
fn word_delete_ignores_an_atom_beyond_the_word() {
    // "a<img>b c": "a" 1, the image 2, "b" 3, " " 4, "c" 5, end 6.
    let back = html_editor(r#"<p>a<img src="x.png">b c</p>"#, 6)
        .run("deleteWordBackward")
        .unwrap();
    assert_eq!(
        html_of(&back),
        html_of(&html_editor(r#"<p>a<img src="x.png">b </p>"#, 1))
    );
    // "foo bar baz<img>": caret after "foo" (4); forward takes " bar" only.
    let fwd = html_editor(r#"<p>foo bar baz<img src="x.png"></p>"#, 4)
        .run("deleteWordForward")
        .unwrap();
    assert_eq!(
        html_of(&fwd),
        html_of(&html_editor(r#"<p>foo baz<img src="x.png"></p>"#, 1))
    );
}

/// The default keymap binds the word deletes to the platform's chord:
/// Ctrl on Windows / Linux, Alt (Option) on macOS; plain Backspace /
/// Delete stay the char deletes.
#[test]
fn word_delete_is_bound_to_the_platforms_chord() {
    let state = editor("x");
    let bound = |k: &str| {
        state
            .keymap()
            .command_for(&KeyBinding::parse(k).unwrap())
            .map(str::to_string)
    };
    let (word, other) = if cfg!(target_os = "macos") {
        ("Alt", "Mod")
    } else {
        ("Mod", "Alt")
    };
    assert_eq!(
        bound(&format!("{word}-Backspace")).as_deref(),
        Some("deleteWordBackward")
    );
    assert_eq!(
        bound(&format!("{word}-Delete")).as_deref(),
        Some("deleteWordForward")
    );
    assert_eq!(bound(&format!("{other}-Backspace")), None);
    assert_eq!(bound(&format!("{other}-Delete")), None);
    assert_eq!(bound("Backspace").as_deref(), Some("deleteCharBackward"));
    assert_eq!(bound("Delete").as_deref(), Some("deleteCharForward"));
}
