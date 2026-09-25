//! Home / End in a text field (issue #933).
//!
//! In a `<textarea>` Home and End go to the start and end of the caret's
//! **visual** line — a soft-wrapped line counts — over the same Parley layout
//! ArrowUp/ArrowDown (#307) and a click use (`RinchApp::input_text_layout`).
//! Ctrl+Home / Ctrl+End go to the ends of the whole value, Shift extends from
//! the anchor, and without Shift a selection collapses and moves from its
//! **head** (Chrome 153 on Linux, measured). A single-line `<input>` is
//! unchanged: Home/End go to the ends of its value.
//!
//! Every expected offset here was measured in Chrome 153 on the same text,
//! except where a test says otherwise (End at a soft wrap, #941).
//!
//! The text is set in the bundled Inter at a declared 16px / 20px so the
//! soft-wrap points do not depend on the host's fonts. `abcd abcd abcd` in a
//! 70px field is one word per visual line: `abcd ` (0..5), `abcd ` (5..10),
//! `abcd` (10..14) — the same fixture #307's soft-wrap test relies on.

use super::*;
use rinch_core::events::{InputCallback, register_input_handler};
use std::cell::Cell;

const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

fn mount(tag: &'static str, width: u32) -> (RinchApp, usize) {
    let input_id = register_input_handler(InputCallback::new(|_v: String| {}));
    let id: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let id_in = id.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let field = scope.create_element(tag);
        field.set_attribute(
            "style",
            &format!(
                "width: {width}px; height: 160px; padding: 0; font-family: sans-serif; \
                 font-size: 16px; line-height: 20px"
            ),
        );
        field.set_attribute("data-oninput", &input_id.0.to_string());
        root.append_child(&field);
        id_in.set(Some(field.node_id().0));
        root
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let field_id = id.get().expect("node id captured at mount");
    focus(&mut app, field_id);
    (app, field_id)
}

fn key_mods(app: &mut RinchApp, key: KeyCode, text: Option<&str>, modifiers: Modifiers) {
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: text.map(str::to_string),
            modifiers,
            repeat: KeyRepeat::Unknown,
        },
        (800, 600),
        1.0,
    );
    app.resolve_and_repaint(800.0, 600.0);
}

fn key(app: &mut RinchApp, key: KeyCode, shift: bool) {
    key_mods(
        app,
        key,
        None,
        Modifiers {
            shift,
            ..Modifiers::default()
        },
    );
}

fn ctrl_key(app: &mut RinchApp, key: KeyCode, shift: bool) {
    key_mods(
        app,
        key,
        None,
        Modifiers {
            shift,
            ctrl: true,
            // `primary()` is Cmd on macOS.
            meta: cfg!(target_os = "macos"),
            ..Modifiers::default()
        },
    );
}

fn type_str(app: &mut RinchApp, text: &str) {
    for ch in text.chars() {
        if ch == '\n' {
            key(app, KeyCode::Enter, false);
        } else {
            key_mods(
                app,
                KeyCode::KeyA,
                Some(&ch.to_string()),
                Modifiers::default(),
            );
        }
    }
}

fn focus(app: &mut RinchApp, id: usize) {
    let (cx, cy) = {
        let d = app.doc.as_ref().unwrap().borrow();
        let (ax, ay, w, h) = painted_element_box(&d.tree, id);
        (ax + w / 2.0, ay + h / 2.0)
    };
    for ev in [
        PlatformEvent::MouseDown {
            x: cx,
            y: cy,
            button: MouseButton::Left,
        },
        PlatformEvent::MouseUp {
            x: cx,
            y: cy,
            button: MouseButton::Left,
        },
    ] {
        app.handle_event(ev, (800, 600), 1.0);
    }
    assert_eq!(app.focused_input_node_id, Some(id), "the field took focus");
}

fn attr(app: &RinchApp, id: usize, name: &str) -> String {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree
        .get(id)
        .and_then(|n| n.attributes.get(name).cloned())
        .unwrap_or_default()
}

fn value(app: &RinchApp, id: usize) -> String {
    attr(app, id, "value")
}

/// `(anchor, head)` as written to the DOM for the paint layer.
fn sel(app: &RinchApp, id: usize) -> (usize, usize) {
    (
        attr(app, id, "data-selection-start").parse().unwrap(),
        attr(app, id, "data-cursor-pos").parse().unwrap(),
    )
}

fn caret(app: &RinchApp, id: usize) -> usize {
    sel(app, id).1
}

/// Put a collapsed caret at byte `at` from the start of the value.
fn caret_to(app: &mut RinchApp, at: usize) {
    ctrl_key(app, KeyCode::Home, false);
    for _ in 0..at {
        key(app, KeyCode::ArrowRight, false);
    }
}

fn caret_rect(app: &mut RinchApp, id: usize, offset: usize) -> (f32, f32, f32, f32) {
    let doc = app.doc.clone().unwrap();
    let d = doc.borrow();
    RinchApp::input_caret_rect_for_offset(
        &d.tree,
        &mut app.hit_test_font_cx,
        &mut app.hit_test_layout_cx,
        id,
        offset,
    )
    .expect("a laid-out value has a caret rect")
}

/// The issue's own case: `abc`⏎`abc`⏎`abc`, caret at the end of line 2, Home
/// goes to the start of line 2 (4), not of the value (0). End from mid line 2
/// stops before its `\n` (7), not at the end of the value (11).
#[test]
fn home_and_end_stop_at_a_hard_line_break() {
    let (mut app, id) = mount("textarea", 200);
    type_str(&mut app, "abc\nabc\nabc");
    caret_to(&mut app, 7);
    key(&mut app, KeyCode::Home, false);
    assert_eq!(caret(&app, id), 4, "Home: start of line 2");
    key(&mut app, KeyCode::ArrowRight, false);
    key(&mut app, KeyCode::End, false);
    assert_eq!(caret(&app, id), 7, "End: before line 2's line break");
}

/// Empty lines, and the empty line after a trailing `\n`: Home and End both
/// stay on them (Chrome 153: `abc⏎⏎abc⏎`, Home from 8 is 5, End from 4 is 4,
/// and both are 9 from 9).
#[test]
fn empty_lines_are_lines() {
    let (mut app, id) = mount("textarea", 200);
    type_str(&mut app, "abc\n\nabc\n");
    caret_to(&mut app, 8);
    key(&mut app, KeyCode::Home, false);
    assert_eq!(caret(&app, id), 5, "Home from the end of line 3");
    caret_to(&mut app, 4);
    key(&mut app, KeyCode::End, false);
    assert_eq!(caret(&app, id), 4, "End on the empty line 2 stays");
    key(&mut app, KeyCode::Home, false);
    assert_eq!(caret(&app, id), 4, "Home on the empty line 2 stays");
    ctrl_key(&mut app, KeyCode::End, false);
    assert_eq!(caret(&app, id), 9);
    key(&mut app, KeyCode::Home, false);
    assert_eq!(caret(&app, id), 9, "Home on the trailing empty line stays");
    key(&mut app, KeyCode::End, false);
    assert_eq!(caret(&app, id), 9, "End on the trailing empty line stays");
}

/// A soft-wrapped line is a line: Home from mid line 2 goes to its start (5),
/// End from the last line to the end of the text.
#[test]
fn home_goes_to_the_start_of_a_soft_wrapped_line() {
    let (mut app, id) = mount("textarea", 70);
    type_str(&mut app, "abcd abcd abcd");
    let (_, first_y, _, _) = caret_rect(&mut app, id, 0);
    let (_, second_y, _, _) = caret_rect(&mut app, id, 5);
    assert!(
        second_y > first_y,
        "the fixture wraps: {first_y} -> {second_y}"
    );
    caret_to(&mut app, 7);
    key(&mut app, KeyCode::Home, false);
    assert_eq!(caret(&app, id), 5, "the visual line's start");
    caret_to(&mut app, 12);
    key(&mut app, KeyCode::Home, false);
    assert_eq!(caret(&app, id), 10, "the last visual line's start");
    key(&mut app, KeyCode::End, false);
    assert_eq!(
        caret(&app, id),
        14,
        "the last visual line's end is the text's"
    );
}

/// End on a soft-wrapped line. Chrome puts the caret at the wrap offset (10)
/// with an *upstream* affinity, so it paints at the end of line 2. rinch's
/// caret has no affinity (#941) and an offset-10 caret paints at the start of
/// line 3, so End stops one character short — before the space the line broke
/// at (9) — which is the same rule ArrowUp/ArrowDown follow (#307). The caret
/// paints on line 2, a second End does not walk on to line 3, and Home comes
/// back to line 2's start.
#[test]
fn end_on_a_soft_wrapped_line_stays_on_it() {
    let (mut app, id) = mount("textarea", 70);
    type_str(&mut app, "abcd abcd abcd");
    caret_to(&mut app, 7);
    key(&mut app, KeyCode::End, false);
    assert_eq!(
        caret(&app, id),
        9,
        "before the wrap's space (Chrome: 10, #941)"
    );
    let (_, end_y, _, h) = caret_rect(&mut app, id, 9);
    let (_, line2_y, _, _) = caret_rect(&mut app, id, 5);
    assert!(
        (end_y - line2_y).abs() < h * 0.5,
        "the caret paints on line 2: {end_y} vs {line2_y}"
    );
    key(&mut app, KeyCode::End, false);
    assert_eq!(caret(&app, id), 9, "a second End stays on line 2");
    key(&mut app, KeyCode::Home, false);
    assert_eq!(caret(&app, id), 5, "and Home returns to line 2's start");
}

/// The cost of #941, pinned so it is seen when affinity lands: a character
/// typed after End at a soft wrap goes before the wrap's space. Chrome 153
/// types `abcd abcd Xabcd`.
#[test]
fn typing_after_end_at_a_soft_wrap_lands_before_the_space() {
    let (mut app, id) = mount("textarea", 70);
    type_str(&mut app, "abcd abcd abcd");
    caret_to(&mut app, 7);
    key(&mut app, KeyCode::End, false);
    type_str(&mut app, "X");
    assert_eq!(value(&app, id), "abcd abcdX abcd");
}

/// Ctrl+Home / Ctrl+End go to the ends of the whole value, from mid line 2.
#[test]
fn ctrl_home_and_ctrl_end_go_to_the_ends_of_the_text() {
    let (mut app, id) = mount("textarea", 70);
    type_str(&mut app, "abcd abcd abcd");
    caret_to(&mut app, 7);
    ctrl_key(&mut app, KeyCode::End, false);
    assert_eq!(caret(&app, id), 14);
    caret_to(&mut app, 7);
    ctrl_key(&mut app, KeyCode::Home, false);
    assert_eq!(caret(&app, id), 0);
    caret_to(&mut app, 7);
    ctrl_key(&mut app, KeyCode::End, true);
    assert_eq!(sel(&app, id), (7, 14), "Ctrl+Shift+End extends");
}

/// Shift extends from the anchor: Shift+Home from 7 selects back to line 2's
/// start, then Shift+End moves the head to line 2's end — one short of the
/// wrap, per #941. (Chrome 153 on a wider fixture: Shift+Home from mid-line
/// selects back to the line's start, Shift+End from there to its end.)
#[test]
fn shift_extends_the_selection() {
    let (mut app, id) = mount("textarea", 70);
    type_str(&mut app, "abcd abcd abcd");
    caret_to(&mut app, 7);
    key(&mut app, KeyCode::Home, true);
    assert_eq!(sel(&app, id), (7, 5));
    key(&mut app, KeyCode::End, true);
    assert_eq!(sel(&app, id), (7, 9));
}

/// Without Shift a selection collapses and moves from its **head**, not its
/// start or end — unlike ArrowUp/ArrowDown. Chrome 153: a forward selection
/// `3..16` goes to 10 on Home and 20 on End; backwards (head 3) to 0 and 10.
/// Here the selection spans lines 1 and 2, so "from the start/end" answers
/// differently from "from the head" for each key.
#[test]
fn a_selection_moves_from_its_head() {
    let (mut app, id) = mount("textarea", 70);
    type_str(&mut app, "abcd abcd abcd");
    // Forward, 2 -> 7: head on line 2.
    let forward = |app: &mut RinchApp| {
        caret_to(app, 2);
        for _ in 0..5 {
            key(app, KeyCode::ArrowRight, true);
        }
        assert_eq!(sel(app, id), (2, 7));
    };
    forward(&mut app);
    key(&mut app, KeyCode::Home, false);
    assert_eq!(sel(&app, id), (5, 5), "Home from the head's line");
    forward(&mut app);
    key(&mut app, KeyCode::End, false);
    assert_eq!(sel(&app, id), (9, 9), "End from the head's line");

    // Backward, 7 -> 2: head on line 1.
    let backward = |app: &mut RinchApp| {
        caret_to(app, 7);
        for _ in 0..5 {
            key(app, KeyCode::ArrowLeft, true);
        }
        assert_eq!(sel(app, id), (7, 2));
    };
    backward(&mut app);
    key(&mut app, KeyCode::Home, false);
    assert_eq!(sel(&app, id), (0, 0), "Home from the head's line");
    backward(&mut app);
    key(&mut app, KeyCode::End, false);
    assert_eq!(sel(&app, id), (4, 4), "End from the head's line");
}

/// Home drops the vertical goal. `abcdef` x3: Up from the end (goal column 6)
/// lands at 13, Home at 7, and Down then goes to column 0 of line 3 (14), not
/// back to the goal (20). Chrome 153 answers 14.
#[test]
fn home_drops_the_vertical_goal() {
    let (mut app, id) = mount("textarea", 200);
    type_str(&mut app, "abcdef\nabcdef\nabcdef");
    key(&mut app, KeyCode::ArrowUp, false);
    assert_eq!(caret(&app, id), 13);
    key(&mut app, KeyCode::Home, false);
    assert_eq!(caret(&app, id), 7);
    key(&mut app, KeyCode::ArrowDown, false);
    assert_eq!(caret(&app, id), 14, "column 0, not the stale column 6");
}

/// End drops it too: `abcdef`⏎`abc`⏎`abcdef`, Up from the end clamps to the end
/// of `abc` (10) with the goal at column 6; Left, End (back to 10), Up goes to
/// column 3, not 6.
#[test]
fn end_drops_the_vertical_goal() {
    let (mut app, id) = mount("textarea", 200);
    type_str(&mut app, "abcdef\nabc\nabcdef");
    key(&mut app, KeyCode::ArrowUp, false);
    assert_eq!(caret(&app, id), 10);
    key(&mut app, KeyCode::ArrowLeft, false);
    key(&mut app, KeyCode::End, false);
    assert_eq!(caret(&app, id), 10);
    key(&mut app, KeyCode::ArrowUp, false);
    assert_eq!(caret(&app, id), 3, "column 3, not the stale column 6");
}

/// A Home or End that does not move the caret keeps the goal, as in Chrome 153:
/// `abcdef`⏎⏎`abcdef`, Up from the end lands on the empty line (7) with the goal
/// at column 6; Home and End there change nothing, and the next Up still goes
/// to column 6.
#[test]
fn a_home_or_end_that_moves_nothing_keeps_the_goal() {
    let (mut app, id) = mount("textarea", 200);
    type_str(&mut app, "abcdef\n\nabcdef");
    key(&mut app, KeyCode::ArrowUp, false);
    assert_eq!(caret(&app, id), 7);
    key(&mut app, KeyCode::Home, false);
    key(&mut app, KeyCode::End, false);
    assert_eq!(caret(&app, id), 7);
    key(&mut app, KeyCode::ArrowUp, false);
    assert_eq!(caret(&app, id), 6, "the goal survived: column 6");
}

/// A single-line `<input>` is unchanged: Home/End go to the ends of the value.
/// The field is narrow enough that the shared layout breaks the value into
/// several lines, so a `<textarea>` rule applied here answers mid-value.
#[test]
fn a_single_line_input_goes_to_the_ends_of_its_value() {
    let (mut app, id) = mount("input", 70);
    type_str(&mut app, "abcd abcd abcd");
    caret_to(&mut app, 7);
    key(&mut app, KeyCode::Home, false);
    assert_eq!(caret(&app, id), 0);
    caret_to(&mut app, 7);
    key(&mut app, KeyCode::End, false);
    assert_eq!(caret(&app, id), 14);
    caret_to(&mut app, 7);
    key(&mut app, KeyCode::Home, true);
    assert_eq!(sel(&app, id), (7, 0));
}
