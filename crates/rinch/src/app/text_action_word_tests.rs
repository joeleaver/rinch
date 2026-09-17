//! A long press on Android selects the word under the finger (issue #813).
//!
//! `word_around` is the boundary rule and `select_word_at_caret` applies it to
//! the focused field. The cases below are chosen off the fixed points: a caret
//! in the *middle* of a word is where every plausible rule agrees, so the
//! pins sit at word starts, word ends, whitespace, and a non-ASCII word.

use super::*;
use rinch_core::events::{InputCallback, register_input_handler};
use std::cell::Cell;

// ── the boundary rule ───────────────────────────────────────────────────────

#[test]
fn a_caret_inside_a_word_selects_that_word() {
    assert_eq!(word_around("hello world", 8), Some((6, 11)));
}

/// The case `MoveWordLeft` + `SelectWordRight` gets wrong: from the start of
/// "world" a word-left walks into "hello".
#[test]
fn a_caret_at_the_start_of_a_word_selects_that_word_not_the_previous_one() {
    assert_eq!(word_around("hello world", 6), Some((6, 11)));
}

/// And the mirror: a caret at the end of "hello" (before the space) belongs
/// to "hello", not to the whitespace after it.
#[test]
fn a_caret_at_the_end_of_a_word_selects_that_word() {
    assert_eq!(word_around("hello world", 5), Some((0, 5)));
    assert_eq!(
        word_around("hello world", 11),
        Some((6, 11)),
        "end of text too"
    );
}

#[test]
fn a_caret_touching_no_word_selects_nothing() {
    assert_eq!(word_around("a  b", 2), None, "between two spaces");
    assert_eq!(word_around("", 0), None, "empty field");
    assert_eq!(
        word_around("hello, world", 6),
        None,
        "between a comma and a space"
    );
    assert_eq!(word_around("   ", 1), None);
}

#[test]
fn an_underscore_joins_a_word_and_punctuation_ends_one() {
    assert_eq!(word_around("foo_bar baz", 2), Some((0, 7)));
    assert_eq!(word_around("foo-bar", 1), Some((0, 3)));
    assert_eq!(word_around("say(hi)", 5), Some((4, 6)));
}

/// Offsets are bytes; a multi-byte word must come back on char boundaries,
/// and a caret handed in past the end or off a boundary is snapped, not
/// panicked on.
#[test]
fn non_ascii_words_and_off_boundary_carets_are_handled() {
    let text = "héllo wörld";
    let w = text.find('w').unwrap();
    assert_eq!(word_around(text, w + 2), Some((w, text.len())));
    assert_eq!(word_around(text, 0), Some((0, w - 1)));
    // Inside `ö` (2 bytes): snapped back to its boundary, same word.
    let o = text.find('ö').unwrap();
    assert_eq!(word_around(text, o + 1), Some((w, text.len())));
    assert_eq!(
        word_around(text, 999),
        Some((w, text.len())),
        "clamped to the end"
    );
}

// ── applied to the focused field ────────────────────────────────────────────

/// One `<input>` holding `value`, with an `oninput` so the shell will claim
/// it. Returns the app and the input's node id.
fn app_with_input(value: &'static str) -> (RinchApp, usize) {
    let oninput = register_input_handler(InputCallback::new(|_| {}));
    let id: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let id_in = id.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let input = scope.create_element("input");
        input.set_attribute(
            "style",
            "width: 300px; height: 30px; font-size: 16px; line-height: 20px",
        );
        input.set_attribute("value", value);
        input.set_attribute("data-oninput", &oninput.0.to_string());
        root.append_child(&input);
        id_in.set(Some(input.node_id().0));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    (app, id.get().expect("node id captured at mount"))
}

fn press(app: &mut RinchApp, x: f32, y: f32, button: MouseButton) {
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, (800, 600), 1.0);
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, (800, 600), 1.0);
}

fn selection(app: &RinchApp) -> (usize, usize) {
    let s = &app
        .focused_input_state
        .as_ref()
        .expect("a focused input")
        .selection;
    (s.start().offset(), s.end().offset())
}

/// The Android shell's sequence: the long press arrives as a right-button
/// press, which claims the field and places the caret, and the word around
/// that caret is then selected. The DOM sees the selection too, which is
/// what paints the highlight.
#[test]
fn the_word_under_a_right_press_is_selected_and_synced_to_the_dom() {
    let (mut app, input) = app_with_input("hello world");
    press(&mut app, 5.0, 15.0, MouseButton::Right);
    assert!(app.has_focused_input(), "the press claims the field");
    // Wherever the hit test put the caret, force it into "world" so the
    // assertion is about the selection rule rather than glyph metrics.
    app.focused_input_state.as_mut().unwrap().selection = Selection::cursor(8);

    assert!(app.select_word_at_caret());
    assert_eq!(selection(&app), (6, 11));

    let doc = app.doc.clone().unwrap();
    let d = doc.borrow();
    let node = d.tree.get(input).unwrap();
    assert_ne!(
        node.attributes.get("data-selection-start"),
        node.attributes.get("data-cursor-pos"),
        "the DOM carries a non-collapsed selection: {:?}",
        node.attributes
    );
}

/// An empty field keeps its caret and reports no word — the shape that shows
/// only Paste.
#[test]
fn an_empty_field_selects_nothing_and_says_so() {
    let (mut app, _) = app_with_input("");
    press(&mut app, 5.0, 15.0, MouseButton::Right);
    assert!(app.has_focused_input());
    assert!(!app.select_word_at_caret());
    assert_eq!(selection(&app), (0, 0));
}

#[test]
fn with_no_focused_field_there_is_nothing_to_select() {
    let (mut app, _) = app_with_input("hello");
    assert!(!app.has_focused_input());
    assert!(!app.select_word_at_caret());
}

/// A long press inside a selection acts on it: Select all, then the press,
/// must not shrink "hello world" to one word.
#[test]
fn an_existing_selection_is_kept_and_still_counts_as_something_to_act_on() {
    let (mut app, _) = app_with_input("hello world");
    press(&mut app, 5.0, 15.0, MouseButton::Right);
    app.focused_input_state.as_mut().unwrap().selection = Selection::new(0, 11);

    assert!(app.select_word_at_caret());
    assert_eq!(selection(&app), (0, 11));
}
