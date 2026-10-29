//! A `<textarea>` whose text is taller than its box, and the selection
//! highlight of a scrolled or multi-line text control (#827).
//!
//! Chrome 155: a textarea is a scroll container (`overflow: auto`). Its text
//! wraps at its content box, is clipped to its padding box, and scrolls
//! vertically — by the least that shows the caret when the caret moves, and
//! by the wheel when it overflows — and it keeps its `scrollTop` when it
//! loses focus. The desktop textarea painted every line from the top of its
//! box down, over whatever lay below it, and nothing scrolled it.
//!
//! Text is set in the bundled Inter (`sans-serif`). The field is `border: 3px`
//! + `padding: 5px 7px`, so its content box `x ∈ [10, 210), y ∈ [8, 62)` and
//! padding box `x ∈ [3, 217), y ∈ [3, 67)` sit off the border-box origin, and
//! the content box (54px) holds two and a fraction of a line — off every
//! whole-line fixed point.

use super::*;
use rinch_core::dom::SelectionDirection;
use rinch_core::events::{InputCallback, register_input_handler};
use std::cell::Cell;

const VP: (u32, u32) = (400, 300);
const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

const CONTENT_LEFT: f32 = 10.0;
const CONTENT_TOP: f32 = 8.0;
const CONTENT_BOTTOM: f32 = 62.0;
/// The padding box, which a scroll container clips its content to.
const PAD: (f32, f32, f32, f32) = (3.0, 3.0, 217.0, 67.0);

const AREA_STYLE: &str = "display: block; width: 220px; height: 70px; padding: 5px 7px; \
     margin: 0; border: 3px solid transparent; font-size: 16px; line-height: 20px; \
     font-family: sans-serif; color: rgb(255, 0, 0); background: transparent; resize: none";

/// Ten short lines: about 190px of text in a 54px content box.
const TEN: &str = "row zero\nrow one\nrow two\nrow three\nrow four\nrow five\nrow six\nrow seven\nrow eight\nrow nine";

fn ev(app: &mut RinchApp, event: PlatformEvent) -> Vec<AppAction> {
    app.handle_event(event, VP, 1.0)
}

fn key_mods(app: &mut RinchApp, key: KeyCode, modifiers: Modifiers) {
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

fn key(app: &mut RinchApp, key: KeyCode) {
    key_mods(app, key, Modifiers::default());
}

fn ctrl(app: &mut RinchApp, k: KeyCode) {
    key_mods(
        app,
        k,
        Modifiers {
            ctrl: true,
            ..Modifiers::default()
        },
    );
}

fn shift(app: &mut RinchApp, k: KeyCode) {
    key_mods(
        app,
        k,
        Modifiers {
            shift: true,
            ..Modifiers::default()
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

fn repaint(app: &mut RinchApp) {
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
}

fn caret_pos(app: &RinchApp, id: usize) -> usize {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree
        .get(id)
        .and_then(|n| n.attributes.get("data-cursor-pos"))
        .and_then(|s| s.parse().ok())
        .expect("a focused field carries its caret")
}

fn scroll_top(app: &RinchApp, id: usize) -> f64 {
    app.doc
        .as_ref()
        .unwrap()
        .borrow()
        .tree
        .get(id)
        .unwrap()
        .scroll_offset
        .1
}

/// One control (`tag`, `style`, `value`), with a button after it to take
/// focus away. Returns the app, the control's id and its handle.
fn page(
    tag: &'static str,
    style: &'static str,
    value: &'static str,
) -> (RinchApp, usize, NodeHandle) {
    let h = register_input_handler(InputCallback::new(|_| {}));
    let id = Rc::new(Cell::new(0usize));
    let handle: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let (id_in, handle_in) = (id.clone(), handle.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "padding: 0; margin: 0");
        let field = scope.create_element(tag);
        field.set_attribute("style", style);
        field.set_attribute("value", value);
        field.set_attribute("data-oninput", &h.0.to_string());
        root.append_child(&field);
        let button = scope.create_element("button");
        button.set_attribute(
            "style",
            "display: block; width: 40px; height: 20px; margin-top: 150px",
        );
        root.append_child(&button);
        id_in.set(field.node_id().0);
        *handle_in.borrow_mut() = Some(field);
        root
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.text_keymap = TextKeymap::Standard;
    app.mount_component(VP.0 as f32, VP.1 as f32);
    repaint(&mut app);
    let field = handle.borrow_mut().take().unwrap();
    (app, id.get(), field)
}

fn area(value: &'static str) -> (RinchApp, usize, NodeHandle) {
    page("textarea", AREA_STYLE, value)
}

fn focus(app: &mut RinchApp, field: &NodeHandle, id: usize) {
    field.focus();
    repaint(app);
    ev(app, PlatformEvent::AboutToWait);
    repaint(app);
    assert_eq!(app.focus_target, FocusTarget::Input(id));
}

fn pixels(app: &mut RinchApp) -> (Vec<u8>, usize) {
    repaint(app);
    app.scene_dirty = true;
    app.has_previous_frame = false;
    let (px, w, _) = app.build_pixels(1.0, VP, true);
    (px.to_vec(), w as usize)
}

/// The field's red ink: `(inside the padding box, outside it)`. The page is
/// transparent and so is the field's border, so any red pixel is the field's
/// text or caret.
fn ink(app: &mut RinchApp) -> (usize, usize) {
    let (px, w) = pixels(app);
    let (mut inside, mut outside) = (0, 0);
    for (i, p) in px.as_chunks::<4>().0.iter().enumerate() {
        if p[3] == 0 || p[0] < 120 || p[1] > 60 || p[2] > 60 {
            continue;
        }
        let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
        if x > PAD.0 && x < PAD.2 && y > PAD.1 && y < PAD.3 {
            inside += 1;
        } else {
            outside += 1;
        }
    }
    (inside, outside)
}

/// The selection wash's horizontal extent on pixel row `row`: the leftmost
/// and rightmost x whose pixel is the wash's blue (the text is pure red, so
/// a blue channel well above zero is the wash). Asked on a row above the
/// glyphs, where no red ink covers the wash's ends.
fn wash_span(px: &[u8], w: usize, row: usize) -> Option<(f32, f32)> {
    let line = &px[row * w * 4..(row + 1) * w * 4];
    let blue: Vec<usize> = line
        .as_chunks::<4>()
        .0
        .iter()
        .enumerate()
        .filter(|(_, p)| p[3] > 0 && p[2] > 40)
        .map(|(x, _)| x)
        .collect();
    Some((*blue.first()? as f32, *blue.last()? as f32 + 1.0))
}

fn caret_rect(field: &NodeHandle) -> (f32, f32, f32) {
    let r = field.caret_rect().expect("a laid-out field has a caret");
    (r.x, r.y, r.height)
}

/// The issue's symptom: the lines past the box were painted below it.
#[test]
fn an_overflowing_textarea_paints_inside_its_padding_box() {
    let (mut app, _id, _field) = area(TEN);
    let (inside, outside) = ink(&mut app);
    assert!(
        inside > 100,
        "positive control: the text is painted ({inside})"
    );
    assert_eq!(outside, 0, "no ink outside the padding box");
}

/// Ctrl+End scrolls the last line into view at the bottom of the content box;
/// Ctrl+Home scrolls back to the top; a blur keeps the scroll (a textarea is
/// a scroll container and keeps `scrollTop`, Chrome 155).
#[test]
fn the_caret_stays_in_view_vertically() {
    let (mut app, id, field) = area(TEN);
    focus(&mut app, &field, id);
    ctrl(&mut app, KeyCode::End);
    repaint(&mut app);
    assert_eq!(caret_pos(&app, id), TEN.len());
    let (_, y, h) = caret_rect(&field);
    assert!(
        y >= CONTENT_TOP && y + h <= CONTENT_BOTTOM + 1.0 && y + h > CONTENT_BOTTOM - 4.0,
        "Ctrl+End: the caret at the content box's bottom, got y {y}..{}",
        y + h
    );
    assert_eq!(
        ink(&mut app).1,
        0,
        "scrolled: nothing outside the padding box"
    );
    let scrolled = scroll_top(&app, id);
    assert!(
        scrolled > 100.0,
        "the textarea scrolled, scrollTop {scrolled}"
    );

    // ArrowUp inside the view does not scroll.
    key(&mut app, KeyCode::ArrowUp);
    repaint(&mut app);
    assert_eq!(scroll_top(&app, id), scrolled, "a move inside the view");

    // A blur keeps the scroll.
    key(&mut app, KeyCode::Tab);
    repaint(&mut app);
    assert_ne!(app.focus_target, FocusTarget::Input(id));
    assert_eq!(scroll_top(&app, id), scrolled, "a blur keeps scrollTop");

    focus(&mut app, &field, id);
    ctrl(&mut app, KeyCode::Home);
    repaint(&mut app);
    assert_eq!(caret_pos(&app, id), 0);
    let (_, y, _) = caret_rect(&field);
    assert!(
        (y - CONTENT_TOP).abs() < 0.5,
        "Ctrl+Home: the caret at the content box's top, got {y}"
    );
    assert_eq!(scroll_top(&app, id), 0.0);
}

/// Typing a new line at the end scrolls by the least: the new last line is
/// at the bottom, one line further than before.
#[test]
fn typing_past_the_bottom_scrolls_by_a_line() {
    let (mut app, id, field) = area(TEN);
    focus(&mut app, &field, id);
    ctrl(&mut app, KeyCode::End);
    repaint(&mut app);
    let before = scroll_top(&app, id);
    key(&mut app, KeyCode::Enter);
    typed(&mut app, "x");
    repaint(&mut app);
    let after = scroll_top(&app, id);
    assert!(
        (15.0..25.0).contains(&(after - before)),
        "one line further: {before} → {after}"
    );
    let (_, y, h) = caret_rect(&field);
    assert!(y + h <= CONTENT_BOTTOM + 1.0 && y + h > CONTENT_BOTTOM - 4.0);
    assert_eq!(ink(&mut app).1, 0);
}

/// A press maps through the scroll: after Ctrl+End, a press on the line
/// painted at the bottom of the box lands in "row nine", and the caret is
/// drawn under the press.
#[test]
fn a_click_after_scrolling_maps_to_the_line_under_it() {
    let (mut app, id, field) = area(TEN);
    focus(&mut app, &field, id);
    ctrl(&mut app, KeyCode::End);
    repaint(&mut app);
    let (_, y, h) = caret_rect(&field);
    let (px, py) = (CONTENT_LEFT + 20.0, y + h / 2.0);
    click(&mut app, px, py);
    repaint(&mut app);
    let at = caret_pos(&app, id);
    let row_nine = TEN.rfind('\n').unwrap() + 1;
    assert!(
        (row_nine..row_nine + 4).contains(&at),
        "the press lands in the last line's \"row\" ({row_nine}..), got {at}"
    );
    let (cx, cy, ch) = caret_rect(&field);
    assert!(
        (cx - px).abs() < 8.0,
        "caret x {cx} under the press at {px}"
    );
    assert!(
        cy <= py && py <= cy + ch,
        "caret y {cy}..{} holds {py}",
        cy + ch
    );
}

/// The wheel scrolls an overflowing textarea, focused or not, and stops at
/// the end of its text.
#[test]
fn the_wheel_scrolls_an_overflowing_textarea() {
    let (mut app, id, field) = area(TEN);
    // Over the field, towards the content's end (winit's sign: negative).
    ev(
        &mut app,
        PlatformEvent::MouseWheel {
            x: 100.0,
            y: 30.0,
            delta_x: 0.0,
            delta_y: -37.0,
        },
    );
    repaint(&mut app);
    assert_eq!(
        scroll_top(&app, id),
        37.0,
        "the wheel scrolled the textarea"
    );
    let y0 = field.caret_rect_at(0).unwrap().y;
    assert!(
        (y0 - (CONTENT_TOP - 37.0)).abs() < 0.5,
        "the text moved with it: offset 0 at {y0}"
    );
    assert_eq!(ink(&mut app).1, 0);
    for _ in 0..10 {
        ev(
            &mut app,
            PlatformEvent::MouseWheel {
                x: 100.0,
                y: 30.0,
                delta_x: 0.0,
                delta_y: -100.0,
            },
        );
    }
    repaint(&mut app);
    let end = field.caret_rect_at(TEN.len()).unwrap();
    assert!(
        end.y + end.height <= CONTENT_BOTTOM + 1.0 && end.y + end.height > CONTENT_BOTTOM - 4.0,
        "scrolled to the end, the last line at the bottom: {}..{}",
        end.y,
        end.y + end.height
    );
}

/// The OS candidate box sits at the scrolled caret, not at the field's top.
#[test]
fn the_ime_area_follows_the_scrolled_caret() {
    let (mut app, id, field) = area(TEN);
    focus(&mut app, &field, id);
    ctrl(&mut app, KeyCode::End);
    repaint(&mut app);
    let area = app.ime_state().cursor_tuple().expect("an IME area");
    let (x, y, h) = caret_rect(&field);
    assert!(
        (area.0 - x).abs() < 0.5 && (area.1 - y).abs() < 0.5 && (area.3 - h).abs() < 1.0,
        "IME area {area:?} vs caret ({x}, {y}, h {h})"
    );
}

/// The issue's point 3: a selection over two lines highlights the first line
/// from the selection's start and the last line up to its end — not the full
/// column on both.
#[test]
fn a_two_line_selection_is_partial_on_its_first_and_last_lines() {
    const TWO: &str = "alpha beta gamma\ndelta epsilon zeta";
    let (mut app, id, field) = area(TWO);
    focus(&mut app, &field, id);
    // "beta gamma\ndelta eps": 6..23.
    field.set_selection_range(6, 23, SelectionDirection::Forward);
    ev(&mut app, PlatformEvent::AboutToWait);
    repaint(&mut app);
    assert_eq!(
        field.selection_range(),
        Some((6, 23, SelectionDirection::Forward))
    );
    let start = field.caret_rect_at(6).unwrap();
    let end = field.caret_rect_at(23).unwrap();
    assert!(end.y > start.y + 10.0, "positive control: two lines");
    let (px, w) = pixels(&mut app);
    // The line box's top rows, above the glyphs' cap height, where the
    // wash is not under red ink.
    let row1 = (start.y + 1.5) as usize;
    let row2 = (end.y + 1.5) as usize;
    let (l1, _) = wash_span(&px, w, row1).expect("line 1 is highlighted");
    let (l2, r2) = wash_span(&px, w, row2).expect("line 2 is highlighted");
    assert!(
        (l1 - start.x).abs() <= 1.5,
        "line 1's wash starts at the selection's start {}, got {l1}",
        start.x
    );
    assert!(
        (r2 - end.x).abs() <= 1.5,
        "line 2's wash ends at the selection's end {}, got {r2}",
        end.x
    );
    assert!(
        (l2 - CONTENT_LEFT).abs() <= 1.5,
        "line 2's wash starts at the column, got {l2}"
    );
    let _ = id;
}

/// A selection in a scrolled single-line input is painted under the text it
/// selects, through the scroll, and clipped to the content box.
#[test]
fn a_scrolled_inputs_selection_is_under_its_text() {
    const LONG: &str = "The quick brown fox jumps over the lazy dog, again and again!";
    const STYLE: &str = "display: block; width: 120px; height: 30px; padding: 0 6px; margin: 0; \
         border: 3px solid transparent; font-size: 16px; line-height: 20px; \
         font-family: sans-serif; color: rgb(255, 0, 0); background: transparent";
    let (mut app, id, field) = page("input", STYLE, LONG);
    focus(&mut app, &field, id);
    key(&mut app, KeyCode::End);
    for _ in 0..5 {
        shift(&mut app, KeyCode::ArrowLeft);
    }
    repaint(&mut app);
    let n = LONG.len();
    let a = field.caret_rect_at(n - 5).unwrap();
    let b = field.caret_rect_at(n).unwrap();
    assert!(
        a.x > 40.0,
        "positive control: scrolled, the selection's start at {}",
        a.x
    );
    let (px, w) = pixels(&mut app);
    let (l, r) = wash_span(&px, w, (a.y + 1.5) as usize).expect("the selection is painted");
    assert!((l - a.x).abs() <= 1.5, "wash starts at {} , got {l}", a.x);
    assert!(
        (r - b.x).abs() <= 1.5 && r <= 111.0,
        "wash ends at {}, got {r}",
        b.x
    );
}

/// The cost: an unchanged textarea shapes nothing for its scroll. Ten pointer
/// moves, each followed by the IME poll and a paint, shape nothing; one typed
/// character shapes the field once.
#[test]
fn an_unchanged_textarea_shapes_nothing_for_its_scroll() {
    use rinch_dom::perf::Counter::ShapeInputScroll;
    let (mut app, id, field) = area(TEN);
    focus(&mut app, &field, id);
    ctrl(&mut app, KeyCode::End);
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
                x: 250.0 + i as f32,
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
    assert_eq!(shapes(&app) - before, 1, "a typed character shapes once");
    let _ = id;
}
