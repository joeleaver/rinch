//! A text control's `direction` and `text-align` (#2939).
//!
//! Chrome lays a field's value out in the field's own direction and aligns it
//! by its `text-align`, start resolved by that direction: `<input dir=rtl>`
//! right-aligns its value, draws an empty field's caret at the right edge,
//! and an overflowing value overflows to the left, its start (the right end)
//! in view; `text-align: right` / `center` move an LTR value the same way,
//! and `<textarea>` lines align as block text does. rinch shaped the value
//! with no base direction (parley's auto, so a Hebrew value flipped an LTR
//! field) and placed it at the column's left whatever the field said.
//!
//! Every number below is Chrome 158's, measured headless at scale 1 on the
//! same box: the bundled Inter at `16px/20px`, `box-sizing: border-box`,
//! 120px wide, `border: 3px` + `padding: 0 6px`, so the column (content box)
//! is `x ∈ [9, 111)`. A right-aligned value ends at the column's right edge
//! ("abc" is 27.92px in Chrome); a caret there is held inside it, Chrome's
//! 1px caret at 110 and rinch's 1.5px one (`CARET_WIDTH`) at 109.5. Ink is
//! the field's red glyphs, `[first, last + 1)` columns.
//!
//! The caret tests that walk an rtl value use Hebrew, so the value's own
//! direction is the field's: a Latin value in an rtl field puts its first
//! and last offsets on bidi boundaries, where Chrome draws the caret at the
//! paragraph's edge and parley at the run's (a separate matter, #2950). No
//! expected number depends on a Hebrew glyph's width (Inter has none, so the
//! host's fallback font draws them).
//!
//! | case | Chrome |
//! |---|---|
//! | `value=abc` (LTR, control) | ink 9..36 |
//! | `dir=rtl value=abc` | ink 83..110; caret at offset 0 at 110, at 3 at 83 |
//! | `style="direction: rtl" value=abc` | ink 83..110 |
//! | `text-align: right value=abc` | ink 83..110, caret at its end at 110 |
//! | `text-align: center value=abc` | ink 46..73 |
//! | `dir=rtl` empty | caret at 110 |
//! | `text-align: right` empty / `center` empty | caret at 110 / 60 |
//! | `dir=rtl` + a 470px value, unfocused | ink 11..110 (its start in view) |
//! | `<textarea dir=rtl>abc def` | ink 54..111 |
//! | `<textarea style="text-align:center">abc def` | ink 32..89 |
//! | `<textarea dir=rtl>` empty / centred empty | caret at 110 / 60 |

use super::*;
use rinch_core::events::{InputCallback, register_input_handler};
use std::cell::Cell;

const VP: (u32, u32) = (400, 200);
const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

const CONTENT_LEFT: f32 = 9.0;
const CONTENT_RIGHT: f32 = 111.0;

const BOX: &str = "display: block; box-sizing: border-box; width: 120px; padding: 0 6px; margin: 0; \
     border: 3px solid transparent; font-size: 16px; line-height: 20px; \
     font-family: sans-serif; color: rgb(255, 0, 0); background: transparent; ";

/// Sixty characters, about 470px of Inter at 16px.
const LONG: &str = "The quick brown fox jumps over the lazy dog, again and again!";

/// Three Hebrew letters: well inside the column in any font.
const HEBREW: &str = "אבג";

/// Hebrew far wider than the column in any font.
const LONG_HEBREW: &str = "שלום עולם זהו משפט ארוך מאוד בעברית שנועד לגלול את השדה הצידה";

/// A field's attributes.
type Attrs = &'static [(&'static str, &'static str)];

fn ev(app: &mut RinchApp, event: PlatformEvent) -> Vec<AppAction> {
    app.handle_event(event, VP, 1.0)
}

fn key(app: &mut RinchApp, key: KeyCode) {
    let modifiers = Modifiers::default();
    ev(
        app,
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: None,
            modifiers,
            repeat: KeyRepeat::Unknown,
        },
    );
    ev(
        app,
        PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers,
        },
    );
}

fn click(app: &mut RinchApp, x: f32, y: f32) {
    let button = MouseButton::Left;
    ev(app, PlatformEvent::MouseDown { x, y, button });
    ev(app, PlatformEvent::MouseUp { x, y, button });
}

fn caret_pos(app: &RinchApp, id: u64) -> usize {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree
        .get(id)
        .and_then(|n| n.attributes.get("data-cursor-pos"))
        .and_then(|s| s.parse().ok())
        .expect("a focused field carries its caret")
}

/// One `tag` (`input` or `textarea`) with `attrs` and `style` added to the
/// box, holding `value`. Returns the app, the field and its handle.
fn page(
    tag: &'static str,
    attrs: Attrs,
    style: &'static str,
    value: &'static str,
) -> (RinchApp, u64, NodeHandle) {
    let h = register_input_handler(InputCallback::new(|_| {}));
    let id = Rc::new(Cell::new(0u64));
    let handle: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let (id_in, handle_in) = (id.clone(), handle.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "padding: 0; margin: 0");
        let field = scope.create_element(tag);
        let height = if tag == "input" {
            "height: 30px; "
        } else {
            "height: 70px; resize: none; overflow: hidden; "
        };
        field.set_attribute("style", &format!("{BOX}{height}{style}"));
        for (k, v) in attrs {
            field.set_attribute(k, v);
        }
        if tag == "input" {
            field.set_attribute("value", value);
        } else if !value.is_empty() {
            field.append_child(&scope.create_text(value));
        }
        field.set_attribute("data-oninput", &h.0.to_string());
        root.append_child(&field);
        id_in.set(field.node_id().0);
        *handle_in.borrow_mut() = Some(field);
        root
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let field = handle.borrow_mut().take().unwrap();
    (app, id.get(), field)
}

/// The field's red ink, `(first column, last column + 1, ink outside the
/// column)`, over the field's rows; `None` for no ink.
fn ink(app: &mut RinchApp) -> Option<(usize, usize, usize)> {
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    app.scene_dirty = true;
    app.has_previous_frame = false;
    let (px, w, _) = app.build_pixels(1.0, VP, true);
    let w = w as usize;
    let (mut lo, mut hi, mut outside) = (usize::MAX, 0, 0);
    for (i, p) in px.as_chunks::<4>().0.iter().enumerate() {
        // Solid red only: anti-aliased fringes below half coverage are left
        // out, as the Chrome measurement leaves them.
        if p[3] < 128 || p[1] > 120 || p[2] > 120 {
            continue;
        }
        let (x, y) = (i % w, i / w);
        if y >= 70 {
            continue;
        }
        if (x as f32) < CONTENT_LEFT - 0.5 || (x as f32) >= CONTENT_RIGHT + 0.5 {
            outside += 1;
        }
        lo = lo.min(x);
        hi = hi.max(x + 1);
    }
    (hi > 0).then_some((lo, hi, outside))
}

fn focus(app: &mut RinchApp, field: &NodeHandle, id: u64) {
    field.focus();
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    ev(app, PlatformEvent::AboutToWait);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    assert_eq!(app.focus_target, FocusTarget::Input(id));
}

fn caret_x(field: &NodeHandle) -> f32 {
    field.caret_rect().expect("a laid-out field has a caret").x
}

fn near(got: f32, want: f32, tol: f32, what: &str) {
    assert!(
        (got - want).abs() <= tol,
        "{what}: got {got}, Chrome {want} (±{tol})"
    );
}

/// The ink of an unfocused field holding "abc", for each way a field is
/// made to align away from the left, against Chrome's.
#[test]
fn a_value_aligns_by_direction_and_text_align() {
    let cases: [(Attrs, &str, (usize, usize)); 5] = [
        (&[], "", (9, 36)),
        (&[("dir", "rtl")], "", (83, 110)),
        (&[], "direction: rtl", (83, 110)),
        (&[], "text-align: right", (83, 110)),
        (&[], "text-align: center", (46, 73)),
    ];
    for (attrs, style, (lo, hi)) in cases {
        let (mut app, _, _) = page("input", attrs, style, "abc");
        let (got_lo, got_hi, outside) = ink(&mut app).expect("the value is painted");
        assert!(
            got_lo.abs_diff(lo) <= 1 && got_hi.abs_diff(hi) <= 1,
            "{attrs:?} {style:?}: ink {got_lo}..{got_hi}, Chrome {lo}..{hi}"
        );
        assert_eq!(outside, 0);
    }
}

/// The caret of a focused field: an rtl field's logical start is its right
/// edge and its end the value's left — `dir=rtl`, and `dir=auto` with a
/// Hebrew value (rtl since #2272); a right-aligned LTR field's end is at the
/// right edge.
#[test]
fn the_caret_follows_the_aligned_value() {
    let dirs: [Attrs; 2] = [&[("dir", "rtl")], &[("dir", "auto")]];
    for attrs in dirs {
        let dir = attrs[0].1;
        let (mut app, id, field) = page("input", attrs, "", HEBREW);
        focus(&mut app, &field, id);
        key(&mut app, KeyCode::Home);
        assert_eq!(caret_pos(&app, id), 0);
        let home = caret_x(&field);
        near(
            home + rinch_dom::form_control::CARET_WIDTH,
            CONTENT_RIGHT,
            0.5,
            &format!("dir={dir} Home: the caret ends at the column's right"),
        );
        key(&mut app, KeyCode::End);
        assert_eq!(caret_pos(&app, id), HEBREW.len());
        let end = caret_x(&field);
        assert!(
            end > CONTENT_LEFT + 5.0 && end < home - 10.0,
            "dir={dir} End: at the value's left, inside the column: {end} (Home {home})"
        );
    }

    let (mut app, id, field) = page("input", &[], "text-align: right", "abc");
    focus(&mut app, &field, id);
    key(&mut app, KeyCode::End);
    near(
        caret_x(&field) + rinch_dom::form_control::CARET_WIDTH,
        111.0,
        0.75,
        "right-aligned End",
    );
}

/// An empty focused field draws its caret where its alignment puts an empty
/// line — painted there, and the caret rect says so too.
#[test]
fn an_empty_field_draws_its_caret_by_its_alignment() {
    let cases: [(&str, Attrs, &str, f32); 6] = [
        ("input", &[], "", 9.0),
        ("input", &[("dir", "rtl")], "", 110.0),
        ("input", &[], "text-align: right", 110.0),
        ("input", &[], "text-align: center", 60.0),
        ("textarea", &[("dir", "rtl")], "", 110.0),
        ("textarea", &[], "text-align: center", 60.0),
    ];
    for (tag, attrs, style, chrome) in cases {
        let what = format!("{tag} {attrs:?} {style:?}");
        let (mut app, id, field) = page(tag, attrs, style, "");
        focus(&mut app, &field, id);
        // Chrome's caret is 1px at `chrome`; rinch's 1.5px. A right-edge
        // caret ends at the column's right in both.
        let want = if chrome > 100.0 {
            CONTENT_RIGHT - rinch_dom::form_control::CARET_WIDTH
        } else {
            chrome
        };
        near(caret_x(&field), want, 1.0, &format!("{what}: caret rect"));
        let (lo, hi, outside) = ink(&mut app).expect("the caret is painted");
        assert!(
            hi - lo <= 3,
            "{what}: only the caret is painted, {lo}..{hi}"
        );
        near(lo as f32, chrome, 1.0, &format!("{what}: painted caret"));
        assert_eq!(outside, 0, "{what}");
    }
}

/// An rtl value wider than its field overflows to the left: unfocused, its
/// start (the right end) is what shows. Focused, End scrolls its logical
/// end — the left — into view at the column's left edge, Home back to the
/// right edge, and a click maps through that scroll.
#[test]
fn an_overflowing_rtl_value_scrolls_from_the_right() {
    let (mut app, _, _) = page("input", &[("dir", "rtl")], "", LONG);
    let (_, hi, outside) = ink(&mut app).expect("the value is painted");
    assert!(
        hi.abs_diff(110) <= 1,
        "unfocused: ink ends at {hi}, Chrome 110"
    );
    assert_eq!(outside, 0, "clipped to the column");

    let (mut app, id, field) = page("input", &[("dir", "rtl")], "", LONG_HEBREW);
    // Unfocused, the start is in view at the right edge (it stood some 300px
    // right of the field when the line started at the column's left).
    near(
        field.caret_rect_at(0).unwrap().x + rinch_dom::form_control::CARET_WIDTH,
        CONTENT_RIGHT,
        0.5,
        "unfocused: the start at the right edge",
    );
    focus(&mut app, &field, id);
    key(&mut app, KeyCode::Home);
    near(
        caret_x(&field) + rinch_dom::form_control::CARET_WIDTH,
        CONTENT_RIGHT,
        0.75,
        "Home: the caret at the right edge",
    );
    key(&mut app, KeyCode::End);
    assert_eq!(caret_pos(&app, id), LONG_HEBREW.len());
    near(caret_x(&field), CONTENT_LEFT, 0.75, "End: at the left edge");
    assert_eq!(ink(&mut app).unwrap().2, 0, "End: clipped to the column");

    // Scrolled to the end: a press near the left edge lands at the end, one
    // near the right edge far from the start.
    click(&mut app, CONTENT_LEFT + 1.0, 15.0);
    assert!(caret_pos(&app, id) >= LONG_HEBREW.len() - 2);
    click(&mut app, CONTENT_RIGHT - 2.0, 15.0);
    let at = caret_pos(&app, id);
    assert!(
        at > 30,
        "a press at the scrolled right edge lands far in: {at}"
    );

    // ArrowLeft in an rtl field moves visually left: towards the end. Back at
    // the start, walking right-to-left past the left edge keeps the caret in
    // view.
    key(&mut app, KeyCode::Home);
    for step in 0..30 {
        key(&mut app, KeyCode::ArrowLeft);
        let x = caret_x(&field);
        assert!(
            (CONTENT_LEFT - 0.5..CONTENT_RIGHT).contains(&x),
            "step {step}: the caret in view, x = {x}"
        );
    }
}

/// A press in a short rtl field: left of the value is its end, at the right
/// edge its start.
#[test]
fn a_press_maps_through_the_alignment() {
    let (mut app, id, field) = page("input", &[("dir", "rtl")], "", HEBREW);
    focus(&mut app, &field, id);
    click(&mut app, 30.0, 15.0);
    assert_eq!(
        caret_pos(&app, id),
        HEBREW.len(),
        "left of the value: its end"
    );
    click(&mut app, 109.0, 15.0);
    assert_eq!(caret_pos(&app, id), 0, "at the right edge: its start");
}

/// `<textarea>` lines align as block text does.
#[test]
fn textarea_lines_align() {
    let cases: [(Attrs, &str, (usize, usize)); 3] = [
        (&[], "", (9, 66)),
        (&[("dir", "rtl")], "", (54, 111)),
        (&[], "text-align: center", (32, 89)),
    ];
    for (attrs, style, (lo, hi)) in cases {
        let (mut app, _, _) = page("textarea", attrs, style, "abc def");
        let (got_lo, got_hi, _) = ink(&mut app).expect("the value is painted");
        assert!(
            got_lo.abs_diff(lo) <= 1 && got_hi.abs_diff(hi) <= 1,
            "{attrs:?} {style:?}: ink {got_lo}..{got_hi}, Chrome {lo}..{hi}"
        );
    }
}

/// A field's paragraph runs in the field's direction, not the value's: in
/// an LTR field whose value starts with Hebrew the Hebrew word stands left
/// of the Latin one, as in Chrome. parley's default base direction took it
/// from the text, which made the paragraph rtl and put the Latin word left.
#[test]
fn an_ltr_field_keeps_its_direction_whatever_its_value() {
    let (_app, _, field) = page("input", &[], "", "אב cd");
    // Inside each word, away from the bidi boundaries: between א and ב
    // (byte 2), between c and d (byte 6).
    let hebrew = field.caret_rect_at(2).unwrap().x;
    let latin = field.caret_rect_at(6).unwrap().x;
    assert!(
        hebrew < latin,
        "the Hebrew word left of the Latin: {hebrew} vs {latin}"
    );
}
