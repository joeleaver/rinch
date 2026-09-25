//! ArrowUp / ArrowDown in a text field (issue #307).
//!
//! A `<textarea>` moves the caret one **visual** line — a soft-wrapped line
//! counts, as in a browser — over the same Parley layout a click is hit-tested
//! against (`RinchApp::input_text_layout`), and keeps the horizontal goal
//! across consecutive vertical moves. Off the first line ArrowUp goes to the
//! start of the text, off the last line ArrowDown to its end, and a single-line
//! `<input>` does the same thing from any caret (Chrome on Linux and Windows).
//! Shift extends the selection; without Shift a selection collapses and moves
//! from its start (up) or its end (down), as Blink's `SelectionModifier` does.
//!
//! The text is set in the bundled Inter so horizontal geometry (the goal x, the
//! soft-wrap point) does not depend on the host's fonts.

use super::*;
use rinch_core::events::{InputCallback, register_input_handler};
use std::cell::Cell;

const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

/// One focusable control of tag `tag` (and `type` attribute `ty`, if any),
/// `width` px wide, in Inter at 16px. Returns the app and the control's id.
fn mount(tag: &'static str, ty: Option<&'static str>, width: u32) -> (RinchApp, usize) {
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
        if let Some(ty) = ty {
            field.set_attribute("type", ty);
        }
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
    // Lay the new value out, so the next vertical move reads the layout the
    // field would paint.
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

/// The caret rect for `offset`, over the same layout paint and hit testing use.
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

/// The issue's own case: `a`⏎`b`, ArrowUp, type — the character lands on line 1.
/// Before the fix ArrowUp was an empty body and `x` went after `b`.
#[test]
fn arrow_up_returns_to_the_first_line() {
    let (mut app, id) = mount("textarea", None, 200);
    type_str(&mut app, "a\nb");
    key(&mut app, KeyCode::ArrowUp, false);
    type_str(&mut app, "x");
    assert_eq!(value(&app, id), "ax\nb");
}

/// ArrowDown is the mirror: from the end of line 1 to the end of line 2.
#[test]
fn arrow_down_moves_to_the_next_line() {
    let (mut app, id) = mount("textarea", None, 200);
    type_str(&mut app, "ab\ncd");
    key(&mut app, KeyCode::ArrowUp, false);
    assert_eq!(caret(&app, id), 2, "up: end of `ab`");
    key(&mut app, KeyCode::ArrowLeft, false);
    key(&mut app, KeyCode::ArrowDown, false);
    assert_eq!(
        caret(&app, id),
        4,
        "down keeps column 1: between `c` and `d`"
    );
}

/// Off the first line ArrowUp goes to the start of the text, off the last line
/// ArrowDown goes to its end — Chrome on Linux and Windows. Sampled mid-line, so
/// "stay put" and "go to the line's start/end" both fail.
#[test]
fn past_the_first_or_last_line_goes_to_the_ends_of_the_text() {
    let (mut app, id) = mount("textarea", None, 200);
    type_str(&mut app, "abc\nabcd");
    key(&mut app, KeyCode::ArrowLeft, false);
    key(&mut app, KeyCode::ArrowLeft, false);
    assert_eq!(caret(&app, id), 6);
    key(&mut app, KeyCode::ArrowDown, false);
    assert_eq!(caret(&app, id), 8, "down on the last line: end of text");
    // The jump to the end is still a vertical move, so the goal from column 2
    // survives it (Blink keeps its x across the edge too).
    key(&mut app, KeyCode::ArrowUp, false);
    assert_eq!(caret(&app, id), 2, "back on line 1 at the goal, mid-line");
    key(&mut app, KeyCode::ArrowUp, false);
    assert_eq!(caret(&app, id), 0, "up on the first line: start of text");
}

/// The goal x survives a short line: from the end of `abc`, down through `d`
/// (clamped to its end) and down again lands at the end of the second `abc`,
/// not at column 1. And back up the same way.
#[test]
fn consecutive_vertical_moves_keep_the_goal_x() {
    let (mut app, id) = mount("textarea", None, 200);
    type_str(&mut app, "abc\nd\nabc");
    // Up twice from the end: `d`'s end, then `abc`'s end.
    key(&mut app, KeyCode::ArrowUp, false);
    assert_eq!(caret(&app, id), 5, "clamped to the end of `d`");
    key(&mut app, KeyCode::ArrowUp, false);
    assert_eq!(
        caret(&app, id),
        3,
        "the goal x is column 3, not `d`'s column 1"
    );
    key(&mut app, KeyCode::ArrowDown, false);
    key(&mut app, KeyCode::ArrowDown, false);
    assert_eq!(caret(&app, id), 9, "and back down to column 3 of line 3");
}

/// Any other caret move sets a new goal: after Up to `d`'s end, Left to its
/// start, the next Up starts from column 0.
#[test]
fn a_horizontal_move_resets_the_goal_x() {
    let (mut app, id) = mount("textarea", None, 200);
    type_str(&mut app, "abc\nd\nabc");
    key(&mut app, KeyCode::ArrowUp, false);
    key(&mut app, KeyCode::ArrowLeft, false);
    assert_eq!(caret(&app, id), 4);
    key(&mut app, KeyCode::ArrowUp, false);
    assert_eq!(caret(&app, id), 0);
}

/// A soft-wrapped line is a line: in one long paragraph with no `\n`, ArrowUp
/// from the end moves one visual line up — neither to the start of the text
/// (what a logical-line model does with a one-line value) nor nowhere.
#[test]
fn a_soft_wrapped_line_is_a_line() {
    let (mut app, id) = mount("textarea", None, 120);
    let text = "alpha beta gamma delta epsilon zeta eta theta";
    type_str(&mut app, text);
    let end = text.len();
    assert_eq!(caret(&app, id), end);
    let (end_x, end_y, _, _) = caret_rect(&mut app, id, end);

    key(&mut app, KeyCode::ArrowUp, false);
    let up = caret(&app, id);
    assert!(
        up > 0 && up < end,
        "one visual line up, not the start of the text: {up}"
    );
    let (_, up_y, _, line_h) = caret_rect(&mut app, id, up);
    let rise = end_y - up_y;
    assert!(
        rise > line_h * 0.5 && rise < line_h * 1.5,
        "one line up, not zero or two: {end_y} -> {up_y} (line {line_h})"
    );

    key(&mut app, KeyCode::ArrowDown, false);
    let down = caret(&app, id);
    let (down_x, down_y, _, _) = caret_rect(&mut app, id, down);
    assert_eq!(down_y, end_y, "back on the last visual line");
    assert!(
        (down_x - end_x).abs() < 0.5,
        "the goal x brought it back where it started"
    );
}

/// Shift extends the selection from the anchor; Shift+Down back collapses it.
#[test]
fn shift_extends_the_selection() {
    let (mut app, id) = mount("textarea", None, 200);
    type_str(&mut app, "ab\ncd");
    key(&mut app, KeyCode::ArrowLeft, false);
    key(&mut app, KeyCode::ArrowUp, true);
    assert_eq!(sel(&app, id), (4, 1), "anchor stays, head moves up");
    key(&mut app, KeyCode::ArrowDown, true);
    assert_eq!(sel(&app, id), (4, 4), "and back");
    key(&mut app, KeyCode::ArrowDown, true);
    assert_eq!(
        sel(&app, id),
        (4, 5),
        "shift+down off the last line: to the end"
    );
}

/// Without Shift a selection collapses and moves from its start (up) or its end
/// (down), not from its head. The selection runs forwards here, so its head is
/// its end and "from the head" answers differently for ArrowUp.
#[test]
fn a_selection_moves_from_its_start_up_and_its_end_down() {
    let (mut app, id) = mount("textarea", None, 200);
    type_str(&mut app, "abc\nabc\nabc");
    // Line 2: anchor at column 0, head at column 2.
    // (Left rather than Home: Home in a `<textarea>` goes to the start of the
    // whole value on desktop, #933.)
    key(&mut app, KeyCode::ArrowUp, false);
    for _ in 0..3 {
        key(&mut app, KeyCode::ArrowLeft, false);
    }
    key(&mut app, KeyCode::ArrowRight, true);
    key(&mut app, KeyCode::ArrowRight, true);
    assert_eq!(sel(&app, id), (4, 6));
    key(&mut app, KeyCode::ArrowUp, false);
    assert_eq!(
        sel(&app, id),
        (0, 0),
        "up from the start: column 0 of line 1"
    );

    // Line 2 again, a *backwards* selection: anchor column 2, head column 0.
    key(&mut app, KeyCode::ArrowDown, false);
    key(&mut app, KeyCode::ArrowRight, false);
    key(&mut app, KeyCode::ArrowRight, false);
    key(&mut app, KeyCode::ArrowLeft, true);
    key(&mut app, KeyCode::ArrowLeft, true);
    assert_eq!(sel(&app, id), (6, 4));
    key(&mut app, KeyCode::ArrowDown, false);
    assert_eq!(
        sel(&app, id),
        (10, 10),
        "down from the end: column 2 of line 3"
    );
}

/// A single-line `<input>`: ArrowUp to the start, ArrowDown to the end, Shift
/// extending — Chrome on Linux and Windows. Sampled mid-value.
#[test]
fn a_single_line_input_goes_to_its_start_and_end() {
    let (mut app, id) = mount("input", None, 200);
    type_str(&mut app, "hello");
    key(&mut app, KeyCode::ArrowLeft, false);
    key(&mut app, KeyCode::ArrowLeft, false);
    key(&mut app, KeyCode::ArrowUp, false);
    assert_eq!(sel(&app, id), (0, 0));
    key(&mut app, KeyCode::ArrowDown, false);
    assert_eq!(sel(&app, id), (5, 5));
    key(&mut app, KeyCode::ArrowLeft, false);
    key(&mut app, KeyCode::ArrowLeft, false);
    key(&mut app, KeyCode::ArrowUp, true);
    assert_eq!(sel(&app, id), (3, 0));
}

/// `type="number"` is not moved: in a browser its arrows step the value, which
/// rinch does not model — so the caret stays where it was rather than taking
/// the text-field meaning.
#[test]
fn a_number_input_is_left_alone() {
    let (mut app, id) = mount("input", Some("number"), 200);
    type_str(&mut app, "123");
    key(&mut app, KeyCode::ArrowLeft, false);
    key(&mut app, KeyCode::ArrowUp, false);
    assert_eq!(sel(&app, id), (2, 2));
    key(&mut app, KeyCode::ArrowDown, false);
    assert_eq!(sel(&app, id), (2, 2));
}

/// Up onto a soft-wrapped line whose end is left of the goal: the nearest
/// offset is that line's end, which is also the next line's start — and a
/// caret there paints on the *next* line. The move stops one character short,
/// so the caret is seen to go up. `iiii ` wraps before the wide `WWWWWWWW`.
#[test]
fn up_onto_a_shorter_soft_wrapped_line_stays_on_it() {
    let (mut app, id) = mount("textarea", None, 130);
    let text = "iiii WWWWWWWW";
    type_str(&mut app, text);
    let (_, end_y, _, line_h) = caret_rect(&mut app, id, text.len());
    let (_, first_y, _, _) = caret_rect(&mut app, id, 0);
    assert!(end_y > first_y, "the fixture wraps: {first_y} -> {end_y}");

    key(&mut app, KeyCode::ArrowUp, false);
    let up = caret(&app, id);
    assert_eq!(up, 4, "before the space the line broke at, not after it");
    let (_, up_y, _, _) = caret_rect(&mut app, id, up);
    assert!(
        (up_y - first_y).abs() < line_h * 0.5,
        "the caret paints on the first line: {up_y} vs {first_y}"
    );

    key(&mut app, KeyCode::ArrowDown, false);
    assert_eq!(caret(&app, id), text.len(), "and the goal brings it back");
}
