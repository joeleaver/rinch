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
//! Text is set in the bundled Inter (`sans-serif`). The field has a 3px
//! border and 5px/7px padding, so its content box `x ∈ [10, 210), y ∈ [8, 62)`
//! and padding box `x ∈ [3, 217), y ∈ [3, 67)` sit off the border-box origin,
//! and the content box (54px) holds two and a fraction of a line — off every
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

fn caret_pos(app: &RinchApp, id: u64) -> usize {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree
        .get(id)
        .and_then(|n| n.attributes.get("data-cursor-pos"))
        .and_then(|s| s.parse().ok())
        .expect("a focused field carries its caret")
}

fn scroll_top(app: &RinchApp, id: u64) -> f64 {
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
) -> (RinchApp, u64, NodeHandle) {
    let h = register_input_handler(InputCallback::new(|_| {}));
    let id = Rc::new(Cell::new(0u64));
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

fn area(value: &'static str) -> (RinchApp, u64, NodeHandle) {
    page("textarea", AREA_STYLE, value)
}

fn focus(app: &mut RinchApp, field: &NodeHandle, id: u64) {
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

/// The caret is revealed to the field's **padding box**, its scrollport, as
/// Chrome does: Ctrl+End puts the last line at the padding box's bottom
/// (Chrome 156: 141 of a 146 range, the bottom padding scrolled out),
/// Ctrl+Home from below puts the first line at its top (Chrome: 5, the top
/// padding scrolled out), and three ArrowDowns from there scroll by the least
/// that shows the fourth line (Chrome: 21 with its 20px lines). A blur keeps
/// the scroll (a textarea is a scroll container and keeps `scrollTop`).
#[test]
fn the_caret_stays_in_view_vertically() {
    let (mut app, id, field) = area(TEN);
    // Where offset 0's caret is drawn unscrolled: the first line's top.
    let top0 = field.caret_rect_at(0).unwrap().y;
    assert!(
        (top0 - CONTENT_TOP).abs() < 1.5,
        "the text starts at the content box, {top0}"
    );
    focus(&mut app, &field, id);
    ctrl(&mut app, KeyCode::End);
    repaint(&mut app);
    assert_eq!(caret_pos(&app, id), TEN.len());
    let (_, y, h) = caret_rect(&field);
    assert!(
        (y + h - PAD.3).abs() <= 1.0,
        "Ctrl+End: the caret at the padding box's bottom {}, got y {y}..{}",
        PAD.3,
        y + h
    );
    let (height, client) = heights(&app, id);
    let short = height - client - scroll_top(&app, id);
    assert!(
        (4.0..=7.0).contains(&short),
        "Ctrl+End: the bottom padding (5px) short of the range's end, got {short}"
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
        (y - (top0 - 5.0)).abs() < 0.5,
        "Ctrl+Home: the caret at the padding box's top {}, got {y} (top0 {top0}, scroll {})",
        top0 - 5.0,
        scroll_top(&app, id)
    );
    assert_eq!(
        scroll_top(&app, id),
        5.0,
        "Chrome: 5, the top padding hidden"
    );

    // ArrowDown past the bottom, off the text's end: the least scroll puts
    // the caret's line at the padding box's bottom, not its top (the end of
    // the text would clamp either answer to the same place).
    for _ in 0..3 {
        key(&mut app, KeyCode::ArrowDown);
    }
    repaint(&mut app);
    let (_, y, h) = caret_rect(&field);
    assert!(
        (y + h - PAD.3).abs() <= 1.0,
        "ArrowDown: the caret's line at the padding box's bottom, got y {y}..{}",
        y + h
    );
    // Four lines down to the padding box's bottom: 5 + 3 × 19.36 + 19.2 − 64
    // with rinch's 19.36px lines (#320), on the grid — Chrome's 21 at 20px.
    let s = scroll_top(&app, id);
    assert!(
        (17.0..=19.0).contains(&s),
        "ArrowDown: scrollTop 18, got {s}"
    );
}

/// A scrolled textarea whose text gets shorter — here a smaller font — is
/// held inside its new range at the layout that shrank it, as any scroll
/// container is (`clamp_scroll_offsets`), not only drawn there.
#[test]
fn a_shrunk_textarea_clamps_its_scroll() {
    let (mut app, id, field) = area(TEN);
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
    let before = scroll_top(&app, id);
    assert!(
        before > 100.0,
        "positive control: scrolled to the end, {before}"
    );
    rinch_core::dom::DomDocument::set_attribute(
        &mut *app.doc.as_ref().unwrap().borrow_mut(),
        rinch_core::dom::NodeId(id),
        "style",
        &format!("{AREA_STYLE}; font-size: 8px"),
    );
    repaint(&mut app);
    let (height, client) = {
        let d = app.doc.as_ref().unwrap().borrow();
        let nid = rinch_core::dom::NodeId(id);
        (
            rinch_core::dom::DomDocument::scroll_height(&*d, nid),
            rinch_core::dom::DomDocument::client_height(&*d, nid),
        )
    };
    let max = height - client;
    assert!(
        max > 10.0 && max < before - 40.0,
        "the range shrank: {before} → {max}"
    );
    let after = scroll_top(&app, id);
    assert!(
        (after - max).abs() <= 1.0,
        "the scroll is clamped to the new range {max}, got {after}"
    );
    let _ = field;
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
    assert!(
        (y + h - PAD.3).abs() <= 1.0,
        "the new line at the padding box's bottom, got y {y}..{}",
        y + h
    );
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
    // Where offset 0's caret is drawn unscrolled: the first line's top.
    let top0 = field.caret_rect_at(0).unwrap().y;
    assert!(
        (top0 - CONTENT_TOP).abs() < 1.5,
        "the text starts at the content box, {top0}"
    );
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
        (y0 - (top0 - 37.0)).abs() < 0.5,
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

// ---- Review of #2023 ----

fn wheel_to_end(app: &mut RinchApp, x: f32) {
    for _ in 0..20 {
        ev(
            app,
            PlatformEvent::MouseWheel {
                x,
                y: 30.0,
                delta_x: 0.0,
                delta_y: -100.0,
            },
        );
    }
    repaint(app);
}

/// `(scroll_height, client_height)` as the DOM API answers them.
fn heights(app: &RinchApp, id: u64) -> (f64, f64) {
    let d = app.doc.as_ref().unwrap().borrow();
    let nid = rinch_core::dom::NodeId(id);
    (
        rinch_core::dom::DomDocument::scroll_height(&*d, nid),
        rinch_core::dom::DomDocument::client_height(&*d, nid),
    )
}

/// A value write runs no layout, and a textarea's scroll range is its text:
/// the resolve after the write holds the offset in the new value's range.
/// Chrome 156 (`scrollTop = 1000; value = 'a\nb'; value = TEN`): 146, 0, 0 —
/// a value that fits scrolls back to 0 and fires `onscroll`, and the long
/// value set again is drawn from the top. rinch kept 139 and drew the regrown
/// text scrolled.
#[test]
fn a_value_that_fits_resets_the_scroll() {
    use rinch_core::events::{ScrollCallback, ScrollEvent, register_scroll_handler};
    let fired = Rc::new(RefCell::new(Vec::new()));
    let fired_in = fired.clone();
    let handler = register_scroll_handler(ScrollCallback::from(move |e: ScrollEvent| {
        fired_in.borrow_mut().push(e.scroll_top);
    }));
    let (mut app, id, field) = area(TEN);
    field.set_attribute("data-onscroll", &handler.0.to_string());
    wheel_to_end(&mut app, 100.0);
    assert!(scroll_top(&app, id) > 100.0, "positive control: scrolled");
    fired.borrow_mut().clear();

    field.set_attribute("value", "a\nb");
    repaint(&mut app);
    assert_eq!(scroll_top(&app, id), 0.0, "a value that fits: scrollTop 0");
    assert_eq!(*fired.borrow(), vec![0.0], "the clamp fires onscroll once");
    let (inside, outside) = ink(&mut app);
    assert!(inside > 10, "the short value is drawn ({inside})");
    assert_eq!(outside, 0);

    field.set_attribute("value", TEN);
    repaint(&mut app);
    assert_eq!(scroll_top(&app, id), 0.0, "a value that grew keeps 0");
    let y0 = field.caret_rect_at(0).unwrap().y;
    assert!(
        (y0 - CONTENT_TOP).abs() < 1.5,
        "regrown: drawn from the top, first line at {y0}"
    );
    assert_eq!(fired.borrow().len(), 1, "growing scrolls nothing");
}

/// A value that shrinks but still overflows is clamped to its new end, not
/// to 0.
#[test]
fn a_shorter_value_clamps_to_its_own_end() {
    let (mut app, id, field) = area(TEN);
    wheel_to_end(&mut app, 100.0);
    let before = scroll_top(&app, id);
    field.set_attribute("value", "one\ntwo\nthree\nfour\nfive");
    repaint(&mut app);
    let (h, c) = heights(&app, id);
    let after = scroll_top(&app, id);
    assert!(
        h - c > 20.0 && h - c < before - 50.0,
        "positive control: still overflows, by less ({before} → {})",
        h - c
    );
    assert!(
        (after - (h - c)).abs() <= 1.0,
        "clamped to the new end {}, got {after}",
        h - c
    );
}

/// `scroll_height` right after a value write and a resolve is the new
/// value's, as a div's is after its content changes — "write, then scroll to
/// `scroll_height`" lands at the end. It answered the old one-line height
/// until the next paint.
#[test]
fn scroll_height_follows_a_value_write_at_the_resolve() {
    let (app, id, field) = area("a");
    let (h0, c) = heights(&app, id);
    assert!(h0 <= c, "positive control: one line fits ({h0} ≤ {c})");
    field.set_attribute("value", TEN);
    app.doc
        .as_ref()
        .unwrap()
        .borrow_mut()
        .resolve_layout(VP.0 as f32, VP.1 as f32);
    let (h, _) = heights(&app, id);
    assert!(
        h > 150.0,
        "scroll_height after the write and a resolve: {h}"
    );
}

/// A narrower textarea rewraps: its range grows with the extra lines and the
/// wheel reaches the new end (the metrics' key holds the wrap width).
#[test]
fn a_width_change_rewraps_the_scroll_range() {
    let (mut app, id, _field) = area(TEN);
    let (h0, _) = heights(&app, id);
    rinch_core::dom::DomDocument::set_attribute(
        &mut *app.doc.as_ref().unwrap().borrow_mut(),
        rinch_core::dom::NodeId(id),
        "style",
        &format!("{AREA_STYLE}; width: 60px"),
    );
    repaint(&mut app);
    let (h1, c) = heights(&app, id);
    assert!(h1 > h0 + 50.0, "rewrapped: {h0} → {h1}");
    wheel_to_end(&mut app, 30.0);
    assert!(
        (scroll_top(&app, id) - (h1 - c)).abs() <= 1.0,
        "the wheel reaches the new end {}, got {}",
        h1 - c,
        scroll_top(&app, id)
    );
}

/// An IME composition reveals itself across a wheel scroll: a preedit at
/// caret 0 with the field wheeled to the end scrolls back to the top, and a
/// long one at the end that wraps scrolls on by its new line, with the
/// candidate box inside the field.
#[test]
fn an_ime_preedit_reveals_across_a_wheel_scroll() {
    let (mut app, id, field) = area(TEN);
    focus(&mut app, &field, id);
    ctrl(&mut app, KeyCode::Home);
    repaint(&mut app);
    wheel_to_end(&mut app, 100.0);
    assert!(scroll_top(&app, id) > 100.0, "positive control: wheeled");
    assert_eq!(caret_pos(&app, id), 0);
    let ime = |app: &mut RinchApp, text: &str| {
        app.handle_event(
            PlatformEvent::Ime(ImeEvent::Preedit {
                text: text.into(),
                cursor: None,
            }),
            VP,
            1.0,
        );
        repaint(app);
    };
    app.handle_event(PlatformEvent::Ime(ImeEvent::Enabled), VP, 1.0);
    ime(&mut app, "ab");
    // The caret's line at the top of the padding box: its top padding
    // scrolled out, as Chrome reveals.
    let s1 = scroll_top(&app, id);
    assert!(s1 < 10.0, "the preedit at 0 is revealed: {s1}");

    ime(&mut app, "");
    ctrl(&mut app, KeyCode::End);
    repaint(&mut app);
    let s2 = scroll_top(&app, id);
    ime(&mut app, "日本語日本語日本語日本語日本語日本語日本語");
    let s3 = scroll_top(&app, id);
    assert!(
        s3 > s2 + 10.0,
        "the wrapped composition scrolled: {s2} → {s3}"
    );
    let area = app.ime_state().cursor_tuple().expect("an IME area");
    assert!(
        area.1 >= PAD.1 - 1.0 && area.1 + area.3 <= PAD.3 + 1.0,
        "the IME area in the padding box: {area:?}"
    );
}

/// The cost: a caret move reads the caret off the focused field's kept
/// layout. Ten ArrowDowns in a 15 KB textarea, each painted, shape its text
/// for the scroll zero times (they shaped it ten times: the metrics' key holds
/// the caret, and a miss shaped the whole text).
#[test]
fn caret_moves_in_a_big_textarea_shape_nothing_for_the_scroll() {
    use rinch_dom::perf::Counter::ShapeInputScroll;
    let big: &'static str = Box::leak(
        (0..400)
            .map(|i| format!("line number {i} with some words in it"))
            .collect::<Vec<_>>()
            .join("\n")
            .into_boxed_str(),
    );
    assert!(big.len() > 15_000);
    let (mut app, id, field) = area(big);
    focus(&mut app, &field, id);
    ctrl(&mut app, KeyCode::Home);
    let _ = ink(&mut app);
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
    for _ in 0..10 {
        key(&mut app, KeyCode::ArrowDown);
        let _ = ink(&mut app);
    }
    // Ten visual lines: each of these wraps once at 206px.
    assert_eq!(
        caret_pos(&app, id),
        big.match_indices('\n').nth(4).unwrap().0 + 1
    );
    assert!(
        scroll_top(&app, id) > 100.0,
        "positive control: it scrolled"
    );
    assert_eq!(shapes(&app) - before, 0, "ten arrows shape nothing");
    typed(&mut app, "Z");
    let _ = ink(&mut app);
    assert_eq!(shapes(&app) - before, 1, "an edit shapes once");
}
