//! A single-line `<input>` whose value is wider than its content box (#1721).
//!
//! Chrome 155 lays the value out on one unwrapped line, clips it to the
//! content box and scrolls it horizontally to keep the caret in view: an
//! arrow, Home/End or typing past an edge scrolls by the least that shows the
//! caret, focusing a field whose caret is out of view centres it, and a blur
//! scrolls back to the start (measured: a 120px field of 62 monospace
//! characters scrolls 0 → 6 → 15 as ArrowRight crosses the right edge, End
//! reveals the end, blur reads `scrollLeft` 0). The desktop field wrapped the
//! value onto extra lines, centred them vertically and painted them over
//! whatever lay above and below it, unclipped.
//!
//! Text is set in the bundled Inter (`sans-serif`), the field is
//! `border: 3px` + `padding: 0 6px`, so its content box is `x ∈ [9, 111)`
//! inside a 120px border box — off the `left == padding` fixed point the old
//! text origin sat on.

use super::*;
use rinch_core::events::{InputCallback, register_input_handler};
use std::cell::Cell;

const VP: (u32, u32) = (400, 200);
const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

/// The field's content box, in window px.
const CONTENT_LEFT: f32 = 9.0;
const CONTENT_RIGHT: f32 = 111.0;
const FIELD_BOTTOM: usize = 30;

const FIELD_STYLE: &str = "display: block; width: 120px; height: 30px; padding: 0 6px; margin: 0; \
     border: 3px solid transparent; font-size: 16px; line-height: 20px; \
     font-family: sans-serif; color: rgb(255, 0, 0); background: transparent";

/// Sixty characters, about 470px of Inter at 16px: four times the column.
const LONG: &str = "The quick brown fox jumps over the lazy dog, again and again!";

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

fn typed(app: &mut RinchApp, t: &str) {
    ev(
        app,
        PlatformEvent::KeyDown {
            key: KeyCode::Other,
            logical_key: None,
            text: Some(t.into()),
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Fresh,
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

/// One `<input type=…>` holding `value`, with a button after it to take focus
/// away. Returns the app, the field and its handle.
fn page(ty: &'static str, value: &'static str) -> (RinchApp, u64, NodeHandle) {
    let h = register_input_handler(InputCallback::new(|_| {}));
    let id = Rc::new(Cell::new(0u64));
    let handle: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let (id_in, handle_in) = (id.clone(), handle.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "padding: 0; margin: 0");
        let field = scope.create_element("input");
        field.set_attribute("style", FIELD_STYLE);
        field.set_attribute("type", ty);
        field.set_attribute("value", value);
        field.set_attribute("data-oninput", &h.0.to_string());
        root.append_child(&field);
        let button = scope.create_element("button");
        button.set_attribute(
            "style",
            "display: block; width: 40px; height: 20px; margin-top: 40px",
        );
        root.append_child(&button);
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

/// Painted pixels of the field's red ink: `(inside the content box, outside
/// it)`, over the rows `0..FIELD_BOTTOM` and below them. The page is
/// transparent and the field's border too, so any opaque pixel is the field's
/// text, selection or caret.
fn ink(app: &mut RinchApp) -> (usize, usize) {
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    app.scene_dirty = true;
    app.has_previous_frame = false;
    let (px, w, _) = app.build_pixels(1.0, VP, true);
    let w = w as usize;
    let (mut inside, mut outside) = (0, 0);
    for (i, p) in px.as_chunks::<4>().0.iter().enumerate() {
        // Red ink only: the button below paints its own grey.
        if p[3] == 0 || p[1] > 40 || p[2] > 40 {
            continue;
        }
        let (x, y) = ((i % w) as f32 + 0.5, i / w);
        if y < FIELD_BOTTOM && x > CONTENT_LEFT && x < CONTENT_RIGHT {
            inside += 1;
        } else {
            outside += 1;
        }
    }
    (inside, outside)
}

fn focus_by_tab(app: &mut RinchApp, field: &NodeHandle, id: u64) {
    field.focus();
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    about_to_wait(app);
    assert_eq!(app.focus_target, FocusTarget::Input(id));
}

fn about_to_wait(app: &mut RinchApp) {
    ev(app, PlatformEvent::AboutToWait);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
}

fn caret_x(field: &NodeHandle) -> f32 {
    field.caret_rect().expect("a laid-out field has a caret").x
}

/// The issue's symptom: the wrapped lines were painted above and below the
/// box, over the content around it.
#[test]
fn a_long_value_paints_one_clipped_line() {
    for ty in ["text", "password"] {
        let (mut app, _id, _field) = page(ty, LONG);
        let (inside, outside) = ink(&mut app);
        assert!(inside > 50, "{ty}: positive control, the value is painted");
        assert_eq!(outside, 0, "{ty}: no ink outside the content box");
    }
}

/// End scrolls the end into view; the caret is inside the column and the
/// ink still is. Typing past the edge keeps it there; Home goes back.
#[test]
fn the_caret_stays_in_view() {
    let (mut app, id, field) = page("text", LONG);
    focus_by_tab(&mut app, &field, id);
    key(&mut app, KeyCode::End);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    assert_eq!(caret_pos(&app, id), LONG.len());
    let x = caret_x(&field);
    assert!(
        (CONTENT_RIGHT - 3.0..CONTENT_RIGHT).contains(&x),
        "End: the caret at the column's right edge, got x = {x}"
    );
    assert_eq!(ink(&mut app).1, 0, "End: nothing outside the content box");

    typed(&mut app, "W");
    typed(&mut app, "W");
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let x = caret_x(&field);
    assert!(
        (CONTENT_RIGHT - 3.0..CONTENT_RIGHT).contains(&x),
        "typing: the caret stays at the right edge, got x = {x}"
    );
    assert_eq!(ink(&mut app).1, 0);

    // ArrowLeft inside the view does not scroll: the caret walks left.
    key(&mut app, KeyCode::ArrowLeft);
    key(&mut app, KeyCode::ArrowLeft);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let walked = caret_x(&field);
    assert!(walked < x - 10.0, "the caret moved left, {x} → {walked}");

    key(&mut app, KeyCode::Home);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    assert_eq!(caret_pos(&app, id), 0);
    let x = caret_x(&field);
    assert!(
        (x - CONTENT_LEFT).abs() < 0.5,
        "Home: the caret at the column's left edge, got {x}"
    );
}

/// A click maps through the scroll: after End, a press near the column's
/// left edge lands on a character far into the value, and the caret is drawn
/// where it was pressed.
#[test]
fn a_click_maps_through_the_scroll() {
    let (mut app, id, field) = page("text", LONG);
    focus_by_tab(&mut app, &field, id);
    key(&mut app, KeyCode::End);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);

    let px = CONTENT_LEFT + 12.0;
    click(&mut app, px, 15.0);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let at = caret_pos(&app, id);
    assert!(
        at > 30,
        "a press at the scrolled column's left lands far in, got {at}"
    );
    let x = caret_x(&field);
    assert!(
        (x - px).abs() < 8.0,
        "the caret is drawn where it was pressed: {x} vs {px}"
    );

    let px = CONTENT_RIGHT - 2.0;
    click(&mut app, px, 15.0);
    let at = caret_pos(&app, id);
    assert!(
        at >= LONG.len() - 1,
        "a press at the right edge lands at the end, got {at}"
    );
}

/// The OS candidate box follows the caret through the scroll.
#[test]
fn the_ime_cursor_area_follows_the_scrolled_caret() {
    let (mut app, id, field) = page("text", LONG);
    focus_by_tab(&mut app, &field, id);
    key(&mut app, KeyCode::End);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let area = app
        .ime_state()
        .cursor_tuple()
        .expect("a focused field places its IME");
    let x = caret_x(&field);
    assert!(
        (area.0 - x).abs() < 0.5,
        "IME area x {} vs caret x {x}",
        area.0
    );
}

/// Focus leaving scrolls the value back to its start, as Chrome does; focus
/// coming back with the caret out of view centres it.
#[test]
fn a_blur_scrolls_back_to_the_start() {
    let (mut app, id, field) = page("text", LONG);
    focus_by_tab(&mut app, &field, id);
    key(&mut app, KeyCode::End);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    assert!(caret_x(&field) > CONTENT_RIGHT - 3.0);

    // Tab to the button.
    key(&mut app, KeyCode::Tab);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    assert_ne!(app.focus_target, FocusTarget::Input(id));
    // The start of the value is what is painted: the caret rect of offset 0
    // is at the column's left edge.
    let x0 = field.caret_rect_at(0).unwrap().x;
    assert!(
        (x0 - CONTENT_LEFT).abs() < 0.5,
        "unfocused: scrolled back, {x0}"
    );
    assert_eq!(ink(&mut app).1, 0);
}

/// An arrow that crosses an edge scrolls by the least that shows the caret
/// (Chrome 155: 0 → 6 → 15 by one advance a step), not to the middle; and a
/// focus that finds its caret out of view centres it.
#[test]
fn an_arrow_scrolls_by_the_least_and_a_focus_centres() {
    let (mut app, id, field) = page("text", LONG);
    focus_by_tab(&mut app, &field, id);
    key(&mut app, KeyCode::Home);
    // Every step once the caret has reached the right edge leaves it there:
    // the text moves under it by one advance.
    let mut at_edge = 0;
    for step in 1..=25 {
        key(&mut app, KeyCode::ArrowRight);
        let x = caret_x(&field);
        assert!(x < CONTENT_RIGHT, "step {step}: the caret in view, x = {x}");
        if at_edge > 0 {
            assert!(
                x >= CONTENT_RIGHT - 3.0,
                "step {step}: past the edge the caret stays at it, x = {x}"
            );
        }
        if x >= CONTENT_RIGHT - 3.0 {
            at_edge += 1;
        }
    }
    assert!(
        at_edge >= 5,
        "positive control: the caret reached the edge ({at_edge} steps)"
    );
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    assert_eq!(caret_pos(&app, id), 25);
    let x = caret_x(&field);
    // Back left inside the view: no scroll, the caret walks.
    for _ in 0..3 {
        key(&mut app, KeyCode::ArrowLeft);
    }
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    assert!(caret_x(&field) < x - 15.0);

    // Away and back: the caret (22) is out of view at the start, so the
    // focus centres it.
    key(&mut app, KeyCode::Tab);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    focus_by_tab(&mut app, &field, id);
    assert_eq!(caret_pos(&app, id), 22);
    let x = caret_x(&field);
    let middle = (CONTENT_LEFT + CONTENT_RIGHT) / 2.0;
    assert!(
        (x - middle).abs() < 2.0,
        "refocus centres the caret: {x} vs {middle}"
    );
}

/// Crossing the LEFT edge scrolls by the least too: once the caret reaches
/// it, every further ArrowLeft leaves it there (review of #1723, p13).
#[test]
fn crossing_the_left_edge_scrolls_by_the_least() {
    let (mut app, id, field) = page("text", LONG);
    focus_by_tab(&mut app, &field, id);
    key(&mut app, KeyCode::End);
    let mut at_edge = 0;
    for step in 0..30 {
        key(&mut app, KeyCode::ArrowLeft);
        let x = caret_x(&field);
        assert!(
            x >= CONTENT_LEFT - 0.5,
            "step {step}: the caret in view, x = {x}"
        );
        if at_edge > 0 {
            assert!(
                x <= CONTENT_LEFT + 0.5,
                "step {step}: past the left edge the caret stays at it, x = {x}"
            );
        }
        if x <= CONTENT_LEFT + 0.5 {
            at_edge += 1;
        }
    }
    assert!(
        at_edge >= 5,
        "positive control: the caret reached the edge ({at_edge})"
    );
}

/// A password field scrolls over its bullets: End puts the painted caret at
/// the right edge (review of #1723, p14).
#[test]
fn a_password_field_scrolls_over_its_bullets() {
    let (mut app, id, field) = page("password", LONG);
    focus_by_tab(&mut app, &field, id);
    key(&mut app, KeyCode::End);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let x = caret_x(&field);
    assert!(
        (CONTENT_RIGHT - 3.0..CONTENT_RIGHT).contains(&x),
        "password End: the caret at the right edge, got x = {x}"
    );
    assert_eq!(ink(&mut app).1, 0);
}

/// Keys with nothing reading the field's geometry between them scroll as
/// they would a frame apart: the reveal is made at each caret move, not at
/// the next read. From the end (scrolled to its maximum), Home then 25
/// ArrowRight with no query between leave the caret at the right edge — a
/// reveal made only at the next read would find it out to the left and put
/// it at the left edge.
#[test]
fn keys_between_reads_reveal_at_each_move() {
    let (mut app, id, field) = page("text", LONG);
    focus_by_tab(&mut app, &field, id);
    key(&mut app, KeyCode::End);
    assert!(caret_x(&field) > CONTENT_RIGHT - 3.0);
    key(&mut app, KeyCode::Home);
    for _ in 0..25 {
        key(&mut app, KeyCode::ArrowRight);
    }
    assert_eq!(caret_pos(&app, id), 25);
    let x = caret_x(&field);
    assert!(
        (CONTENT_RIGHT - 3.0..CONTENT_RIGHT).contains(&x),
        "the caret at the right edge, got x = {x}"
    );
}

/// During an IME composition the candidate box sits at the painted caret,
/// the composition's end, inside the field (review of #1723, p08: it was at
/// the composition's start under a scroll chosen for its end, x = −2.5).
#[test]
fn the_ime_area_follows_a_composition() {
    for preedit in ["にほ", "日本語にほんご"] {
        let (mut app, id, field) = page("text", LONG);
        focus_by_tab(&mut app, &field, id);
        key(&mut app, KeyCode::End);
        ev(
            &mut app,
            PlatformEvent::Ime(ImeEvent::Preedit {
                text: preedit.into(),
                cursor: None,
            }),
        );
        app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
        let area = app.ime_state().cursor_tuple().expect("an IME area");
        assert!(
            (CONTENT_RIGHT - 3.0..CONTENT_RIGHT).contains(&area.0),
            "{preedit}: the area at the composition's end, at the right edge, got {area:?}"
        );
        assert_eq!(
            ink(&mut app).1,
            0,
            "{preedit}: nothing outside the content box"
        );
        let _ = id;
    }
}

/// The cost: the field's line is shaped once per change of its text or
/// caret. Ten pointer moves, each followed by the IME poll and a paint, shape
/// nothing for the scroll; one typed character shapes it once (at the caret
/// sync), and the paint and poll after it reuse that.
#[test]
fn an_unchanged_field_shapes_nothing_for_its_scroll() {
    use rinch_dom::perf::Counter::ShapeInputScroll;
    let (mut app, id, field) = page("text", LONG);
    focus_by_tab(&mut app, &field, id);
    key(&mut app, KeyCode::End);
    let _ = ink(&mut app);
    let _ = app.ime_state();
    let shapes = |app: &RinchApp| {
        app.doc
            .as_ref()
            .unwrap()
            .borrow()
            .tree
            .perf
            .total()
            .get(ShapeInputScroll)
    };
    let before = shapes(&app);
    for i in 0..10 {
        ev(
            &mut app,
            PlatformEvent::MouseMove {
                x: 150.0 + i as f32,
                y: 100.0,
            },
        );
        let _ = app.ime_state();
        let _ = ink(&mut app);
    }
    assert_eq!(shapes(&app) - before, 0, "ten moves shape nothing");
    typed(&mut app, "Z");
    let _ = app.ime_state();
    let _ = ink(&mut app);
    assert_eq!(
        shapes(&app) - before,
        1,
        "a typed character shapes the line once"
    );
    let _ = id;
}
