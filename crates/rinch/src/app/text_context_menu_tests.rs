//! The built-in text context menu (issue #813), driven through the real
//! `PlatformEvent` path: a right press on a text target opens Cut / Copy /
//! Paste / Select all, each item runs exactly what its chord runs, and the
//! menu lives no longer than its target's claim.
//!
//! Every fixture samples **off the fixed points**: the selection is `2..5`
//! rather than `0..n`, the inside/outside presses land at offsets 4 and 8, the
//! editor selection spans two blocks. Text geometry pins the local font set,
//! so every field declares `font-size` and `line-height`.
//!
//! The clipboard fixtures run under the `clipboard` feature (CI's
//! `--workspace` run unifies it on) against `rinch_clipboard`'s in-memory
//! backend, and hold one process-wide lock each, because the clipboard is one
//! per process and the test binary is multi-threaded.

use super::*;
use rinch_core::dom::NodeId;
use rinch_core::events::{InputCallback, dismiss_handler_count, register_input_handler};
use std::cell::Cell;

const VP: (u32, u32) = (800, 600);

fn ev(app: &mut RinchApp, event: PlatformEvent) -> Vec<AppAction> {
    app.handle_event(event, VP, 1.0)
}

fn press_with(app: &mut RinchApp, x: f32, y: f32, button: MouseButton) -> Vec<AppAction> {
    let mut actions = ev(app, PlatformEvent::MouseDown { x, y, button });
    actions.extend(ev(app, PlatformEvent::MouseUp { x, y, button }));
    actions
}

fn right_press(app: &mut RinchApp, x: f32, y: f32) -> Vec<AppAction> {
    press_with(app, x, y, MouseButton::Right)
}

fn left_press(app: &mut RinchApp, x: f32, y: f32) -> Vec<AppAction> {
    press_with(app, x, y, MouseButton::Left)
}

fn key_with(app: &mut RinchApp, key: KeyCode, text: Option<&str>, modifiers: Modifiers) {
    ev(
        app,
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: text.map(str::to_string),
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
    key_with(app, key, None, Modifiers::default());
}

/// The primary chord modifier on the host the test runs on.
fn primary() -> Modifiers {
    Modifiers {
        ctrl: !cfg!(target_os = "macos"),
        meta: cfg!(target_os = "macos"),
        ..Default::default()
    }
}

fn shift() -> Modifiers {
    Modifiers {
        shift: true,
        ..Default::default()
    }
}

fn abs_box(app: &RinchApp, id: usize) -> (f32, f32, f32, f32) {
    let d = app.doc.as_ref().unwrap().borrow();
    painted_element_box(&d.tree, id)
}

fn abs_center(app: &RinchApp, id: usize) -> (f32, f32) {
    let (x, y, w, h) = abs_box(app, id);
    (x + w / 2.0, y + h / 2.0)
}

fn attr(app: &RinchApp, id: usize, name: &str) -> Option<String> {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree.get(id).and_then(|n| n.attributes.get(name).cloned())
}

/// The field's `(value, selection start, cursor)` as the DOM carries them —
/// what paint reads, so what the user sees.
fn field_state(app: &RinchApp, id: usize) -> (String, String, String) {
    (
        attr(app, id, "value").unwrap_or_default(),
        attr(app, id, "data-selection-start").unwrap_or_default(),
        attr(app, id, "data-cursor-pos").unwrap_or_default(),
    )
}

/// The menu's rows as `(action, enabled)`, in menu order.
fn rows(app: &RinchApp) -> Vec<(TextEditAction, bool)> {
    app.open_text_menu
        .as_ref()
        .map(|m| m.items.iter().map(|r| (r.action, r.enabled)).collect())
        .unwrap_or_default()
}

fn row_center(app: &RinchApp, action: TextEditAction) -> (f32, f32) {
    let id = app
        .open_text_menu
        .as_ref()
        .and_then(|m| m.items.iter().find(|r| r.action == action))
        .map(|r| r.node_id)
        .expect("the menu is open and has the row");
    abs_center(app, id)
}

fn highlighted(app: &RinchApp) -> Option<TextEditAction> {
    let m = app.open_text_menu.as_ref()?;
    m.highlighted.map(|i| m.items[i].action)
}

/// The smallest window x at which a press in `input` places the caret at
/// byte `offset` — found with the runtime's own click-to-offset map, so the
/// fixture presses exactly where the user would have to.
fn x_for_offset(app: &mut RinchApp, input: usize, offset: usize) -> f32 {
    let (bx, by, bw, bh) = abs_box(app, input);
    let y = by + bh / 2.0;
    let doc = app.doc.clone().unwrap();
    let d = doc.borrow();
    let mut x = bx;
    while x < bx + bw {
        let got = RinchApp::compute_input_cursor_from_click(
            &d.tree,
            &mut app.hit_test_font_cx,
            &mut app.hit_test_layout_cx,
            input,
            x,
            y,
        );
        if got == offset {
            return x;
        }
        x += 1.0;
    }
    panic!("no x in the field maps to offset {offset}");
}

/// Select bytes `2..5` of the focused field through the keyboard: Home, two
/// steps right, three shifted steps. Off the `0..n` fixed point on purpose.
fn select_2_to_5(app: &mut RinchApp) {
    key(app, KeyCode::Home);
    key(app, KeyCode::ArrowRight);
    key(app, KeyCode::ArrowRight);
    for _ in 0..3 {
        key_with(app, KeyCode::ArrowRight, None, shift());
    }
}

const FIELD_STYLE: &str = "width: 300px; height: 30px; padding: 0; margin: 0; \
     font-size: 16px; line-height: 20px; font-family: sans-serif";

#[derive(Clone, Copy)]
struct Ids {
    input: usize,
    textarea: usize,
    checkbox: usize,
    plain: usize,
    /// An input under a `div` carrying a live `data-oncontextmenu`.
    guarded_input: usize,
    /// A `<div tabindex>` beside the fields, for a press that lands elsewhere.
    other: usize,
}

type Log = Rc<RefCell<Vec<String>>>;

/// One page holding every target kind the menu has to distinguish. `input`
/// carries `value`; `extra` lands on it as attributes (`readonly`,
/// `type=password`, `disabled`).
fn page(
    value: &'static str,
    extra: &'static [(&'static str, &'static str)],
) -> (RinchApp, Ids, Log) {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let record = |tag: &'static str| {
        let log = log.clone();
        register_input_handler(InputCallback::new(move |v: String| {
            log.borrow_mut().push(format!("{tag}:{v}"));
        }))
    };
    let input_h = record("input");
    let textarea_h = record("textarea");
    let checkbox_h = record("checkbox");
    let guarded_h = record("guarded");
    let ctx_rid = rinch_core::register_handler(Rc::new({
        let log = log.clone();
        move || log.borrow_mut().push("contextmenu".to_string())
    }));
    let plain_rid = rinch_core::register_handler(Rc::new({
        let log = log.clone();
        move || log.borrow_mut().push("plain-click".to_string())
    }));

    let ids: Rc<Cell<Option<Ids>>> = Rc::new(Cell::new(None));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let input = scope.create_element("input");
        input.set_attribute("style", FIELD_STYLE);
        input.set_attribute("value", value);
        input.set_attribute("data-oninput", &input_h.0.to_string());
        for (k, v) in extra {
            input.set_attribute(k, v);
        }
        let textarea = scope.create_element("textarea");
        textarea.set_attribute("style", FIELD_STYLE);
        textarea.set_attribute("value", value);
        textarea.set_attribute("data-oninput", &textarea_h.0.to_string());
        let checkbox = scope.create_element("input");
        checkbox.set_attribute("type", "checkbox");
        checkbox.set_attribute("style", "width: 20px; height: 20px");
        checkbox.set_attribute("data-oninput", &checkbox_h.0.to_string());
        let plain = scope.create_element("div");
        plain.set_attribute("style", "width: 300px; height: 40px");
        plain.set_attribute("data-rid", &plain_rid.0.to_string());
        let guard = scope.create_element("div");
        guard.set_attribute("style", "width: 300px; height: 40px");
        guard.set_attribute("data-oncontextmenu", &ctx_rid.0.to_string());
        let guarded_input = scope.create_element("input");
        guarded_input.set_attribute("style", FIELD_STYLE);
        guarded_input.set_attribute("value", value);
        guarded_input.set_attribute("data-oninput", &guarded_h.0.to_string());
        guard.append_child(&guarded_input);
        // Far from every field, so a press on it is outside any menu that
        // opened at a field.
        let other = scope.create_element("div");
        other.set_attribute(
            "style",
            "position: absolute; left: 500px; top: 450px; width: 200px; height: 40px",
        );
        other.set_attribute("tabindex", "0");
        root.append_child(&input);
        root.append_child(&textarea);
        root.append_child(&checkbox);
        root.append_child(&plain);
        root.append_child(&guard);
        root.append_child(&other);
        ids_in.set(Some(Ids {
            input: input.node_id().0,
            textarea: textarea.node_id().0,
            checkbox: checkbox.node_id().0,
            plain: plain.node_id().0,
            guarded_input: guarded_input.node_id().0,
            other: other.node_id().0,
        }));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let ids = ids.get().expect("node ids captured at mount");
    (app, ids, log)
}

/// Focus `input` with a left press at its left edge and select `2..5`.
fn focus_and_select(app: &mut RinchApp, input: usize) {
    let (bx, by, _, bh) = abs_box(app, input);
    left_press(app, bx + 1.0, by + bh / 2.0);
    assert_eq!(app.focus_target, FocusTarget::Input(input));
    select_2_to_5(app);
    assert_eq!(
        (
            attr(app, input, "data-selection-start"),
            attr(app, input, "data-cursor-pos")
        ),
        (Some("2".into()), Some("5".into())),
        "precondition: bytes 2..5 selected"
    );
}

// ── Opening: which targets, and which not ────────────────────────────────────

#[test]
fn a_right_press_on_a_text_input_opens_the_menu_and_keeps_the_claim() {
    let (mut app, ids, _log) = page("hello world", &[]);
    assert!(!app.is_text_context_menu_open());
    let before = dismiss_handler_count();

    let (x, y) = abs_center(&app, ids.input);
    right_press(&mut app, x, y);

    assert!(app.is_text_context_menu_open(), "the menu opened");
    assert_eq!(
        app.focus_target,
        FocusTarget::Input(ids.input),
        "the field holds the keyboard under the menu"
    );
    assert_eq!(
        rows(&app).iter().map(|r| r.0).collect::<Vec<_>>(),
        vec![
            TextEditAction::Cut,
            TextEditAction::Copy,
            TextEditAction::Paste,
            TextEditAction::SelectAll
        ],
        "four rows in menu order"
    );
    assert_eq!(
        dismiss_handler_count(),
        before + 1,
        "the menu joined the dismiss stack at open"
    );
    // The panel is a real box in the document, at the press point.
    let panel = app.open_text_menu.as_ref().unwrap().panel_id;
    let (px, py, pw, ph) = abs_box(&app, panel);
    assert_eq!((px, py), (x, y), "anchored at the press");
    assert!(pw >= 180.0 && ph > 60.0, "laid out: {pw}x{ph}");
}

#[test]
fn a_textarea_opens_it_too() {
    let (mut app, ids, _log) = page("hello world", &[]);
    let (x, y) = abs_center(&app, ids.textarea);
    right_press(&mut app, x, y);
    assert!(app.is_text_context_menu_open());
    assert_eq!(app.focus_target, FocusTarget::Input(ids.textarea));
}

#[test]
fn no_menu_on_a_checkbox_a_plain_div_or_under_a_live_oncontextmenu() {
    let (mut app, ids, log) = page("hello world", &[]);

    let (x, y) = abs_center(&app, ids.checkbox);
    right_press(&mut app, x, y);
    assert!(
        !app.is_text_context_menu_open(),
        "a checkbox has no text to cut"
    );

    let (x, y) = abs_center(&app, ids.plain);
    right_press(&mut app, x, y);
    assert!(
        !app.is_text_context_menu_open(),
        "a plain div is not a text target"
    );

    // Precedence: the handler above the field wins, and the menu stays shut.
    let (x, y) = abs_center(&app, ids.guarded_input);
    right_press(&mut app, x, y);
    assert_eq!(
        log.borrow().iter().filter(|e| *e == "contextmenu").count(),
        1,
        "the ancestor's data-oncontextmenu ran"
    );
    assert!(
        !app.is_text_context_menu_open(),
        "a live data-oncontextmenu suppresses the built-in menu"
    );
}

#[test]
fn a_disabled_field_takes_no_focus_and_opens_nothing() {
    let (mut app, ids, _log) = page("hello world", &[("disabled", "")]);
    let (x, y) = abs_center(&app, ids.input);
    right_press(&mut app, x, y);
    assert_eq!(
        app.focus_target,
        FocusTarget::None,
        "disabled takes no claim (#315)"
    );
    assert!(!app.is_text_context_menu_open());
    assert_eq!(app.text_edit_state(), None);
}

// ── Enabled states ───────────────────────────────────────────────────────────

fn state_after_right_press(app: &mut RinchApp, input: usize) -> TextEditState {
    let (x, y) = abs_center(app, input);
    right_press(app, x, y);
    app.text_edit_state()
        .expect("a text target holds the keyboard")
}

fn flags(s: &TextEditState) -> (bool, bool, bool, bool) {
    (s.can_cut, s.can_copy, s.can_paste, s.can_select_all)
}

#[test]
fn an_empty_field_enables_only_paste() {
    let (mut app, ids, _log) = page("", &[]);
    let s = state_after_right_press(&mut app, ids.input);
    assert_eq!(
        flags(&s),
        (false, false, cfg!(feature = "clipboard"), false)
    );
    assert_eq!(
        rows(&app).iter().map(|r| r.1).collect::<Vec<_>>(),
        vec![false, false, cfg!(feature = "clipboard"), false],
        "the rows carry the same states"
    );
}

#[test]
fn a_field_with_content_but_no_selection_enables_paste_and_select_all() {
    let (mut app, ids, _log) = page("hello world", &[]);
    let s = state_after_right_press(&mut app, ids.input);
    assert_eq!(flags(&s), (false, false, cfg!(feature = "clipboard"), true));
}

#[test]
fn a_selection_enables_cut_and_copy() {
    let (mut app, ids, _log) = page("hello world", &[]);
    focus_and_select(&mut app, ids.input);
    // Press inside the selection so it survives (the caret rule).
    let x = x_for_offset(&mut app, ids.input, 4);
    let (_, by, _, bh) = abs_box(&app, ids.input);
    right_press(&mut app, x, by + bh / 2.0);
    let s = app.text_edit_state().unwrap();
    assert_eq!(flags(&s), (true, true, cfg!(feature = "clipboard"), true));
}

#[test]
fn readonly_greys_cut_and_paste_and_keeps_copy() {
    let (mut app, ids, _log) = page("hello world", &[("readonly", "")]);
    focus_and_select(&mut app, ids.input);
    let x = x_for_offset(&mut app, ids.input, 4);
    let (_, by, _, bh) = abs_box(&app, ids.input);
    right_press(&mut app, x, by + bh / 2.0);
    let s = app.text_edit_state().unwrap();
    assert_eq!(flags(&s), (false, true, false, true));
}

#[test]
fn password_greys_cut_and_copy_and_keeps_paste() {
    let (mut app, ids, _log) = page("hello world", &[("type", "password")]);
    focus_and_select(&mut app, ids.input);
    let x = x_for_offset(&mut app, ids.input, 4);
    let (_, by, _, bh) = abs_box(&app, ids.input);
    right_press(&mut app, x, by + bh / 2.0);
    let s = app.text_edit_state().unwrap();
    assert_eq!(flags(&s), (false, false, cfg!(feature = "clipboard"), true));
}

// ── The caret rule ───────────────────────────────────────────────────────────

#[test]
fn a_right_press_inside_the_selection_keeps_it_and_outside_moves_the_caret() {
    let (mut app, ids, _log) = page("hello world", &[]);
    focus_and_select(&mut app, ids.input);
    let (_, by, _, bh) = abs_box(&app, ids.input);
    let y = by + bh / 2.0;

    let inside = x_for_offset(&mut app, ids.input, 4);
    right_press(&mut app, inside, y);
    assert!(app.is_text_context_menu_open());
    assert_eq!(
        (
            attr(&app, ids.input, "data-selection-start"),
            attr(&app, ids.input, "data-cursor-pos")
        ),
        (Some("2".into()), Some("5".into())),
        "a press inside the selection keeps it"
    );

    // Close the menu (outside press), then press outside the selection.
    let (ox, oy) = abs_center(&app, ids.other);
    left_press(&mut app, ox, oy);
    assert!(!app.is_text_context_menu_open());
    let outside = x_for_offset(&mut app, ids.input, 8);
    right_press(&mut app, outside, y);
    assert!(app.is_text_context_menu_open());
    assert_eq!(
        (
            attr(&app, ids.input, "data-selection-start"),
            attr(&app, ids.input, "data-cursor-pos")
        ),
        (Some("8".into()), Some("8".into())),
        "a press outside the selection moves the caret to the press"
    );
}

// ── Pointer and keyboard on the open menu ────────────────────────────────────

#[test]
fn an_outside_press_closes_the_menu_without_acting_and_is_swallowed() {
    let (mut app, ids, log) = page("hello world", &[]);
    focus_and_select(&mut app, ids.input);
    let x = x_for_offset(&mut app, ids.input, 4);
    let (_, by, _, bh) = abs_box(&app, ids.input);
    right_press(&mut app, x, by + bh / 2.0);
    let before_open = dismiss_handler_count() - 1;
    let state_before = field_state(&app, ids.input);

    // Arrow to Cut so an "acting" mutant has something to run.
    key(&mut app, KeyCode::ArrowDown);
    assert_eq!(highlighted(&app), Some(TextEditAction::Cut));

    // The plain div's left end: on the div, and outside the panel, which
    // opened at the field's centre and extends right and down from there.
    let (px, py, _, ph) = abs_box(&app, ids.plain);
    let (px, py) = (px + 10.0, py + ph / 2.0);
    let panel = app.open_text_menu.as_ref().unwrap().panel_id;
    let (mx, my, mw, mh) = abs_box(&app, panel);
    assert!(
        px < mx || px > mx + mw || py < my || py > my + mh,
        "precondition: the press point is outside the panel"
    );
    left_press(&mut app, px, py);

    assert!(!app.is_text_context_menu_open(), "closed");
    assert_eq!(
        dismiss_handler_count(),
        before_open,
        "its dismiss entry left with it"
    );
    assert_eq!(field_state(&app, ids.input), state_before, "nothing acted");
    assert_eq!(
        app.focus_target,
        FocusTarget::Input(ids.input),
        "the press was swallowed: the field keeps the keyboard"
    );
    assert!(
        !log.borrow().iter().any(|e| e == "plain-click"),
        "the data-rid under the press did not fire"
    );
}

#[test]
fn keys_navigate_the_enabled_rows_and_enter_runs_the_highlighted_one() {
    let (mut app, ids, _log) = page("hello world", &[]);
    // No selection: Cut and Copy are disabled and must be skipped.
    let (x, y) = abs_center(&app, ids.input);
    right_press(&mut app, x, y);
    assert_eq!(highlighted(&app), None, "opens with nothing highlighted");

    key(&mut app, KeyCode::ArrowDown);
    let first_enabled = if cfg!(feature = "clipboard") {
        TextEditAction::Paste
    } else {
        TextEditAction::SelectAll
    };
    assert_eq!(
        highlighted(&app),
        Some(first_enabled),
        "Down skips disabled rows"
    );
    key(&mut app, KeyCode::End);
    assert_eq!(highlighted(&app), Some(TextEditAction::SelectAll));
    key(&mut app, KeyCode::ArrowDown);
    assert_eq!(highlighted(&app), Some(first_enabled), "Down wraps");
    key(&mut app, KeyCode::ArrowUp);
    assert_eq!(
        highlighted(&app),
        Some(TextEditAction::SelectAll),
        "Up wraps"
    );
    key(&mut app, KeyCode::Home);
    assert_eq!(highlighted(&app), Some(first_enabled));
    key(&mut app, KeyCode::End);

    key(&mut app, KeyCode::Enter);
    assert!(
        !app.is_text_context_menu_open(),
        "Enter ran the row and closed"
    );
    assert_eq!(
        (
            attr(&app, ids.input, "data-selection-start"),
            attr(&app, ids.input, "data-cursor-pos")
        ),
        (Some("0".into()), Some("11".into())),
        "Select all selected the whole value"
    );
    assert_eq!(app.focus_target, FocusTarget::Input(ids.input));
}

#[test]
fn escape_closes_through_the_dismiss_stack_and_releases_the_entry() {
    let (mut app, ids, _log) = page("hello world", &[]);
    // An app-level handler beneath the menu: it must not be reached while
    // the menu is open, and must be reached once the menu has gone — the
    // positive control that the entry was really released.
    let hits = Rc::new(Cell::new(0u32));
    let _app_handler = {
        let hits = hits.clone();
        rinch_core::push_dismiss_handler(app.doc_key(), move || {
            hits.set(hits.get() + 1);
            true
        })
    };
    let base = dismiss_handler_count();

    let (x, y) = abs_center(&app, ids.input);
    right_press(&mut app, x, y);
    assert_eq!(dismiss_handler_count(), base + 1);

    key(&mut app, KeyCode::Escape);
    assert!(!app.is_text_context_menu_open(), "Escape closed the menu");
    assert_eq!(hits.get(), 0, "the menu, on top, consumed it");
    assert_eq!(dismiss_handler_count(), base, "the entry was released");
    assert_eq!(
        app.focus_target,
        FocusTarget::Input(ids.input),
        "the field keeps the keyboard"
    );

    key(&mut app, KeyCode::Escape);
    assert_eq!(
        hits.get(),
        1,
        "with the menu gone, Escape reaches the app's handler"
    );
}

#[test]
fn every_other_key_is_consumed_while_the_menu_is_open() {
    let (mut app, ids, log) = page("hello world", &[]);
    let (x, y) = abs_center(&app, ids.input);
    right_press(&mut app, x, y);
    let before = field_state(&app, ids.input);
    key_with(&mut app, KeyCode::KeyZ, Some("z"), Modifiers::default());
    assert!(
        app.is_text_context_menu_open(),
        "typing does not close a native menu"
    );
    assert_eq!(
        field_state(&app, ids.input),
        before,
        "and does not reach the field"
    );
    assert!(!log.borrow().iter().any(|e| e.starts_with("input:")));
}

#[test]
fn hovering_a_row_highlights_it_and_the_pointer_is_the_menus() {
    let (mut app, ids, _log) = page("hello world", &[]);
    let (x, y) = abs_center(&app, ids.input);
    right_press(&mut app, x, y);
    let (rx, ry) = row_center(&app, TextEditAction::SelectAll);
    let actions = ev(&mut app, PlatformEvent::MouseMove { x: rx, y: ry });
    assert_eq!(highlighted(&app), Some(TextEditAction::SelectAll));
    assert!(
        actions.contains(&AppAction::SetCursor(rinch_platform::CursorStyle::Default)),
        "an arrow over the menu, not the field's I-beam: {actions:?}"
    );
    // A disabled row is not highlighted by hover either.
    let (cx, cy) = row_center(&app, TextEditAction::Cut);
    ev(&mut app, PlatformEvent::MouseMove { x: cx, y: cy });
    assert_eq!(highlighted(&app), Some(TextEditAction::SelectAll));
}

// ── Lifetime: the menu goes with its target ──────────────────────────────────

#[test]
fn removing_the_field_closes_the_menu_and_releases_its_entry() {
    let (mut app, ids, _log) = page("hello world", &[]);
    let base = dismiss_handler_count();
    let (x, y) = abs_center(&app, ids.input);
    right_press(&mut app, x, y);
    assert_eq!(dismiss_handler_count(), base + 1);

    app.doc
        .as_ref()
        .unwrap()
        .borrow_mut()
        .remove_node(NodeId(ids.input));
    // Nothing told the app yet — the next event of any kind finds out.
    ev(&mut app, PlatformEvent::MouseMove { x: 1.0, y: 1.0 });

    assert!(
        !app.is_text_context_menu_open(),
        "the field left the document"
    );
    assert_eq!(
        dismiss_handler_count(),
        base,
        "and its dismiss entry left with it"
    );
}

#[test]
fn a_window_blur_closes_the_menu_and_keeps_the_claim() {
    let (mut app, ids, _log) = page("hello world", &[]);
    let base = dismiss_handler_count();
    let (x, y) = abs_center(&app, ids.input);
    right_press(&mut app, x, y);

    ev(&mut app, PlatformEvent::WindowFocus(false));

    assert!(!app.is_text_context_menu_open());
    assert_eq!(dismiss_handler_count(), base);
    assert_eq!(
        app.focus_target,
        FocusTarget::Input(ids.input),
        "notify-and-retain (#147): the blur keeps the in-document claim"
    );
}

#[test]
fn a_focus_move_closes_the_menu() {
    let (mut app, ids, _log) = page("hello world", &[]);
    let (x, y) = abs_center(&app, ids.input);
    right_press(&mut app, x, y);
    // Programmatic: anything that moves the claim, not only a press.
    app.set_focus_target(FocusTarget::Node(ids.other));
    assert!(!app.is_text_context_menu_open());
}

// ── Presentation: the shell's half ───────────────────────────────────────────

#[test]
fn under_shell_presentation_the_gesture_prepares_and_emits_instead_of_opening() {
    let (mut app, ids, _log) = page("hello world", &[]);
    app.set_text_context_menu_presentation(TextContextMenuPresentation::Shell);
    focus_and_select(&mut app, ids.input);
    let x = x_for_offset(&mut app, ids.input, 4);
    let (_, by, _, bh) = abs_box(&app, ids.input);

    let actions = ev(
        &mut app,
        PlatformEvent::MouseDown {
            x,
            y: by + bh / 2.0,
            button: MouseButton::Right,
        },
    );

    assert!(
        actions.contains(&AppAction::ShowTextContextMenu),
        "{actions:?}"
    );
    assert!(
        !app.is_text_context_menu_open(),
        "no DOM menu under Shell presentation"
    );
    let s = app.text_edit_state().expect("the state the shell reads");
    assert_eq!(flags(&s), (true, true, cfg!(feature = "clipboard"), true));
    assert_eq!(
        (
            attr(&app, ids.input, "data-selection-start"),
            attr(&app, ids.input, "data-cursor-pos")
        ),
        (Some("2".into()), Some("5".into())),
        "the caret rule ran before the shell was told"
    );
    // The shell drives the item through the seam.
    app.perform_text_edit(TextEditAction::SelectAll);
    assert_eq!(
        (
            attr(&app, ids.input, "data-selection-start"),
            attr(&app, ids.input, "data-cursor-pos")
        ),
        (Some("0".into()), Some("11".into()))
    );
    // A press on nothing textual emits nothing.
    let (px, py) = abs_center(&app, ids.plain);
    let actions = right_press(&mut app, px, py);
    assert!(!actions.contains(&AppAction::ShowTextContextMenu));
}

// ── Item versus chord: one code path ─────────────────────────────────────────

/// The clipboard is one per process and the test binary is many threads:
/// each clipboard fixture holds this for its whole body.
#[cfg(feature = "clipboard")]
fn clipboard_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    rinch_clipboard::use_in_memory_clipboard();
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// A field's `(value, selection start, cursor)` plus the clipboard text after it.
#[cfg(feature = "clipboard")]
type FieldAndClip = ((String, String, String), String);

/// Run `action` on a fresh page with `2..5` selected, by the chord and by the
/// menu item, and answer both `(field state, clipboard text)`.
#[cfg(feature = "clipboard")]
fn by_chord_and_by_item(
    action: TextEditAction,
    chord: KeyCode,
    preload: Option<&str>,
) -> (FieldAndClip, FieldAndClip) {
    let run = |via_menu: bool| {
        let (mut app, ids, _log) = page("hello world", &[]);
        focus_and_select(&mut app, ids.input);
        rinch_clipboard::clear().unwrap();
        if let Some(text) = preload {
            rinch_clipboard::copy_text(text).unwrap();
        }
        if via_menu {
            let x = x_for_offset(&mut app, ids.input, 4);
            let (_, by, _, bh) = abs_box(&app, ids.input);
            right_press(&mut app, x, by + bh / 2.0);
            assert!(app.is_text_context_menu_open());
            let (rx, ry) = row_center(&app, action);
            left_press(&mut app, rx, ry);
            assert!(!app.is_text_context_menu_open(), "the item closed the menu");
        } else {
            key_with(&mut app, chord, None, primary());
        }
        assert_eq!(app.focus_target, FocusTarget::Input(ids.input));
        let clip = rinch_clipboard::paste_text().unwrap_or_default();
        (field_state(&app, ids.input), clip)
    };
    (run(false), run(true))
}

#[cfg(feature = "clipboard")]
#[test]
fn cut_by_item_equals_cut_by_chord() {
    let _lock = clipboard_lock();
    let ((cs, cc), (is, ic)) = by_chord_and_by_item(TextEditAction::Cut, KeyCode::KeyX, None);
    assert_eq!(
        cs,
        ("he world".into(), "2".into(), "2".into()),
        "the chord cut 2..5"
    );
    assert_eq!(cc, "llo", "and put it on the clipboard");
    assert_eq!((is, ic), (cs, cc), "the item did exactly the same");
}

#[cfg(feature = "clipboard")]
#[test]
fn copy_by_item_equals_copy_by_chord() {
    let _lock = clipboard_lock();
    let ((cs, cc), (is, ic)) = by_chord_and_by_item(TextEditAction::Copy, KeyCode::KeyC, None);
    assert_eq!(
        cs,
        ("hello world".into(), "2".into(), "5".into()),
        "copy leaves the field alone"
    );
    assert_eq!(cc, "llo");
    assert_eq!((is, ic), (cs, cc));
}

#[cfg(feature = "clipboard")]
#[test]
fn paste_by_item_equals_paste_by_chord() {
    let _lock = clipboard_lock();
    let ((cs, cc), (is, ic)) =
        by_chord_and_by_item(TextEditAction::Paste, KeyCode::KeyV, Some("XY"));
    assert_eq!(
        cs,
        ("heXY world".into(), "4".into(), "4".into()),
        "pasted over 2..5"
    );
    assert_eq!(cc, "XY", "paste leaves the clipboard alone");
    assert_eq!((is, ic), (cs, cc));
}

#[cfg(feature = "clipboard")]
#[test]
fn select_all_by_item_equals_select_all_by_chord() {
    let _lock = clipboard_lock();
    let ((cs, cc), (is, ic)) = by_chord_and_by_item(TextEditAction::SelectAll, KeyCode::KeyA, None);
    assert_eq!(cs, ("hello world".into(), "0".into(), "11".into()));
    assert_eq!(cc, "", "select all touches no clipboard");
    assert_eq!((is, ic), (cs, cc));
}

// ── The rich-text editor ─────────────────────────────────────────────────────

#[cfg(feature = "desktop")]
mod editor {
    use super::*;
    use rinch_editor_core::{Pos, Selection};

    struct EditorPage {
        app: RinchApp,
        container: usize,
        handle: crate::editor::EditorHandle,
        other: usize,
    }

    fn editor_page(html: &'static str) -> EditorPage {
        let slot: Rc<RefCell<Option<(usize, crate::editor::EditorHandle, usize)>>> =
            Rc::new(RefCell::new(None));
        let slot_in = slot.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            let (container, handle) = crate::editor::mount_editor(scope);
            handle.load_html(html);
            container.set_attribute(
                "style",
                "width: 400px; height: 200px; font-size: 16px; line-height: 24px; \
                 font-family: sans-serif",
            );
            let other = scope.create_element("div");
            other.set_attribute("style", "width: 300px; height: 40px");
            other.set_attribute("tabindex", "0");
            root.append_child(&container);
            root.append_child(&other);
            *slot_in.borrow_mut() = Some((container.node_id().0, handle, other.node_id().0));
            root
        });
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        let (container, handle, other) = slot.borrow_mut().take().expect("captured at mount");
        EditorPage {
            app,
            container,
            handle,
            other,
        }
    }

    /// A window point just after the caret for `pos`.
    fn point_at(page: &EditorPage, pos: usize) -> (f32, f32) {
        let (x, y, h) = page
            .app
            .editor_caret_point(&page.handle, Pos(pos))
            .expect("the position has a caret");
        (x + 1.0, y + h / 2.0)
    }

    #[test]
    fn a_right_press_in_the_editor_opens_the_menu_and_the_editor_keeps_the_claim() {
        let mut p = editor_page("<p>hello world</p>");
        let (x, y) = point_at(&p, 4);
        right_press(&mut p.app, x, y);
        assert!(p.app.is_text_context_menu_open());
        assert_eq!(p.app.focus_target, FocusTarget::Editor(p.container));
        let s = p.app.text_edit_state().unwrap();
        assert_eq!(flags(&s), (false, false, cfg!(feature = "clipboard"), true));
    }

    #[test]
    fn a_multi_block_selection_enables_cut_and_copy_and_a_press_inside_keeps_it() {
        let mut p = editor_page("<p>one two</p><p>three four</p>");
        // Focus the editor with a left press, then select from inside the
        // first block to inside the second: `<p>` opens at 0, "one two" is
        // 1..8, `</p><p>` puts "three four" at 10..20.
        let (x, y) = point_at(&p, 2);
        left_press(&mut p.app, x, y);
        assert_eq!(p.app.focus_target, FocusTarget::Editor(p.container));
        let selection = Selection::text(Pos(3), Pos(14));
        p.handle.set_selection(selection.clone());

        let (x, y) = point_at(&p, 12);
        right_press(&mut p.app, x, y);
        assert!(p.app.is_text_context_menu_open());
        assert_eq!(
            p.handle.selection(),
            selection,
            "a press inside the selection keeps it"
        );
        let s = p.app.text_edit_state().unwrap();
        assert_eq!(flags(&s), (true, true, cfg!(feature = "clipboard"), true));
        // The anchor spans both carets.
        let (ax, ay, aw, ah) = s.anchor;
        assert!(
            aw > 1.0 && ah > 24.0,
            "two lines of selection: {ax},{ay} {aw}x{ah}"
        );

        // Close, then press outside the selection: the caret moves there.
        let (ox, oy) = abs_center(&p.app, p.other);
        left_press(&mut p.app, ox, oy);
        assert!(!p.app.is_text_context_menu_open());
        assert_eq!(
            p.app.focus_target,
            FocusTarget::Editor(p.container),
            "the outside press was swallowed"
        );
        let (x, y) = point_at(&p, 18);
        right_press(&mut p.app, x, y);
        assert!(p.app.is_text_context_menu_open());
        let after = p.handle.selection();
        assert!(after.is_empty(), "collapsed: {after:?}");
        assert_eq!(after.head(), Pos(18));
    }

    #[test]
    fn select_all_by_item_equals_the_chord() {
        let mut a = editor_page("<p>one two</p><p>three four</p>");
        let mut b = editor_page("<p>one two</p><p>three four</p>");
        for p in [&mut a, &mut b] {
            let (x, y) = point_at(p, 2);
            left_press(&mut p.app, x, y);
        }
        key_with(&mut a.app, KeyCode::KeyA, None, primary());

        let (x, y) = point_at(&b, 2);
        right_press(&mut b.app, x, y);
        let (rx, ry) = row_center(&b.app, TextEditAction::SelectAll);
        left_press(&mut b.app, rx, ry);
        assert!(!b.app.is_text_context_menu_open());

        assert_eq!(a.handle.selection(), b.handle.selection());
        assert!(
            !a.handle.selection().is_empty(),
            "the chord selected something"
        );
        assert_eq!(b.app.focus_target, FocusTarget::Editor(b.container));
    }

    #[cfg(feature = "clipboard")]
    #[test]
    fn cut_and_paste_by_item_equal_the_chords() {
        let _lock = clipboard_lock();
        let run = |via_menu: bool| {
            let mut p = editor_page("<p>one two</p><p>three four</p>");
            let (x, y) = point_at(&p, 2);
            left_press(&mut p.app, x, y);
            p.handle.set_selection(Selection::text(Pos(3), Pos(14)));
            rinch_clipboard::clear().unwrap();
            if via_menu {
                let (x, y) = point_at(&p, 12);
                right_press(&mut p.app, x, y);
                let (rx, ry) = row_center(&p.app, TextEditAction::Cut);
                left_press(&mut p.app, rx, ry);
            } else {
                key_with(&mut p.app, KeyCode::KeyX, None, primary());
            }
            // The editor's copy is queued on the clipboard worker; a blocking
            // read behind it in the queue sees its result.
            let clip = rinch_clipboard::paste_text().unwrap_or_default();
            (format!("{:?}", p.handle.doc()), p.handle.selection(), clip)
        };
        let chord = run(false);
        let item = run(true);
        assert_eq!(item, chord);
        assert_eq!(
            chord.2, "e two\nthre",
            "the cut text reached the clipboard: {:?}",
            chord.2
        );
    }
}

// ── A modal that locks the page ──────────────────────────────────────────────

mod in_a_modal {
    use super::*;
    use rinch_components::Modal;
    use rinch_core::{Component, Signal};

    /// A `Modal` with `lock_scroll` (its default) holding one text field, under
    /// the real theme and component sheets, opened from the start.
    fn modal_page() -> (RinchApp, usize) {
        let opened = Signal::new(true);
        let input_h = register_input_handler(InputCallback::new(|_| {}));
        let ids: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
        let ids_in = ids.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let input = scope.create_element("input");
            input.set_attribute("style", FIELD_STYLE);
            input.set_attribute("value", "hello world");
            input.set_attribute("data-oninput", &input_h.0.to_string());
            ids_in.set(Some(input.node_id().0));
            Modal {
                opened_fn: Some(Rc::new(move || opened.get())),
                ..Default::default()
            }
            .render(scope, &[input])
        });
        app.mount_component(800.0, 600.0);
        {
            let doc = app.doc.as_ref().unwrap();
            let mut d = doc.borrow_mut();
            d.load_css(&rinch_theme::generate_theme_css(
                &rinch_theme::Theme::default(),
            ));
            d.load_css(&rinch_components::generate_component_css());
            d.recompute_all_styles_full();
        }
        app.resolve_and_repaint(800.0, 600.0);
        (app, ids.get().expect("captured at mount"))
    }

    #[test]
    fn the_menu_opens_and_acts_inside_a_scroll_locked_modal() {
        let (mut app, input) = modal_page();
        assert!(
            !app.doc
                .as_ref()
                .unwrap()
                .borrow()
                .tree
                .scroll_lock_roots
                .is_empty(),
            "precondition: the modal locked the page"
        );
        let (x, y) = abs_center(&app, input);
        right_press(&mut app, x, y);
        assert!(
            app.is_text_context_menu_open(),
            "the menu opens over the modal"
        );
        assert_eq!(app.focus_target, FocusTarget::Input(input));

        let (rx, ry) = row_center(&app, TextEditAction::SelectAll);
        left_press(&mut app, rx, ry);
        assert!(!app.is_text_context_menu_open());
        assert_eq!(
            (
                attr(&app, input, "data-selection-start"),
                attr(&app, input, "data-cursor-pos")
            ),
            (Some("0".into()), Some("11".into())),
            "the item acted on the field inside the modal"
        );
        // And the modal's own Escape is still beneath the menu's, not displaced.
        let (x, y) = abs_center(&app, input);
        right_press(&mut app, x, y);
        key(&mut app, KeyCode::Escape);
        assert!(
            !app.is_text_context_menu_open(),
            "Escape closed the menu first"
        );
        assert_eq!(
            app.focus_target,
            FocusTarget::Input(input),
            "and the modal stayed"
        );
    }
}
