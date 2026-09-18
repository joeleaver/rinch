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
    let clip = cfg!(feature = "clipboard");
    assert_eq!(flags(&s), (clip, clip, clip, true));
}

#[test]
fn readonly_greys_cut_and_paste_and_keeps_copy() {
    let (mut app, ids, _log) = page("hello world", &[("readonly", "")]);
    focus_and_select(&mut app, ids.input);
    let x = x_for_offset(&mut app, ids.input, 4);
    let (_, by, _, bh) = abs_box(&app, ids.input);
    right_press(&mut app, x, by + bh / 2.0);
    let s = app.text_edit_state().unwrap();
    assert_eq!(flags(&s), (false, cfg!(feature = "clipboard"), false, true));
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

    // Arrow to the first enabled row so an "acting" mutant has something to
    // run: Cut with a clipboard, Select all without one.
    key(&mut app, KeyCode::ArrowDown);
    assert!(highlighted(&app).is_some());

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
    let clip = cfg!(feature = "clipboard");
    assert_eq!(flags(&s), (clip, clip, clip, true));
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
///
/// **One lock for the whole binary.** `editor_read_only_tests` is in this same
/// `--lib` target and touches the same in-memory clipboard, so a second `static
/// LOCK` of its own would serialize each file against itself and neither against
/// the other — two fixtures asserting on clipboard *content* would then race. Any
/// further module that reads what another wrote takes this one too.
#[cfg(feature = "clipboard")]
pub(super) fn clipboard_lock() -> std::sync::MutexGuard<'static, ()> {
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

/// Without a clipboard there is nothing for Cut or Copy to reach, and an
/// enabled Cut would delete text it never copied (the review's `p5`,
/// inverted). Runs under a bare `cargo test -p rinch`.
#[cfg(not(feature = "clipboard"))]
#[test]
fn without_the_clipboard_feature_a_selected_field_greys_cut_and_copy() {
    let (mut app, ids, _log) = page("hello world", &[]);
    focus_and_select(&mut app, ids.input);
    let x = x_for_offset(&mut app, ids.input, 4);
    let (_, by, _, bh) = abs_box(&app, ids.input);
    right_press(&mut app, x, by + bh / 2.0);
    assert_eq!(
        rows(&app),
        vec![
            (TextEditAction::Cut, false),
            (TextEditAction::Copy, false),
            (TextEditAction::Paste, false),
            (TextEditAction::SelectAll, true)
        ]
    );
    // And a press on the greyed Cut row deletes nothing.
    let (rx, ry) = row_center(&app, TextEditAction::Cut);
    left_press(&mut app, rx, ry);
    assert_eq!(
        field_state(&app, ids.input),
        ("hello world".into(), "2".into(), "5".into())
    );
}

/// The row under a window point, with its enabled state — `None` for the
/// panel's own padding and for anything outside the menu.
fn row_under(app: &RinchApp, x: f32, y: f32) -> Option<(TextEditAction, bool)> {
    let m = app.open_text_menu.as_ref()?;
    let d = app.doc.as_ref().unwrap().borrow();
    let mut cur = hit_test(&d.tree, x, y);
    while let Some(nid) = cur {
        let node = d.tree.get(nid)?;
        if let Some(i) = node
            .attributes
            .get("data-tcm-item")
            .and_then(|s| s.parse::<usize>().ok())
        {
            return m.items.get(i).map(|r| (r.action, r.enabled));
        }
        cur = node.parent;
    }
    None
}

/// The opening press's release lands on a row when both clamps moved the
/// panel up and left to fit the window (a bottom-right press): nothing may
/// run — items run on a press, and this release is the one that opened it.
/// (The review's `p1`.)
#[test]
fn the_opening_release_over_a_clamped_row_runs_nothing() {
    let (mut app, ids, _log) = page("hello world", &[]);
    focus_and_select(&mut app, ids.input);
    // The field sits at the top of the page; open at the bottom-right corner
    // through the public API (the gesture opens at the press point and the
    // clamp is the same code).
    let (x, y) = (780.0, 590.0);
    assert!(app.open_text_context_menu(x, y, 800.0, 600.0));
    let panel = app.open_text_menu.as_ref().unwrap().panel_id;
    let (px, py, pw, ph) = abs_box(&app, panel);
    assert!(
        px + pw <= 800.0 && py + ph <= 600.0 && px < x && py < y,
        "clamped up and left: ({px},{py}) {pw}x{ph}"
    );
    assert_eq!(
        row_under(&app, x, y),
        Some((TextEditAction::SelectAll, true)),
        "an enabled row sits under the pointer"
    );
    ev(&mut app, PlatformEvent::MouseMove { x, y });
    assert_eq!(highlighted(&app), Some(TextEditAction::SelectAll));
    let before = field_state(&app, ids.input);
    ev(
        &mut app,
        PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Right,
        },
    );
    assert!(
        app.is_text_context_menu_open(),
        "the release did not close it"
    );
    assert_eq!(
        field_state(&app, ids.input),
        before,
        "the release ran nothing"
    );
    ev(
        &mut app,
        PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Left,
        },
    );
    assert_eq!(field_state(&app, ids.input), before, "nor a left release");
    assert!(app.is_text_context_menu_open());
}

/// The same through the real gesture at scale factor 2, sliding onto an
/// enabled row — Select all, enabled with or without a clipboard — before
/// releasing. (The review's `p1b`, which used Cut.)
#[test]
fn the_gesture_at_scale_2_and_its_release_over_a_row() {
    let (mut app, ids, _log) = page("hello world", &[]);
    focus_and_select(&mut app, ids.input);
    let x = x_for_offset(&mut app, ids.input, 4);
    let (_, by, _, bh) = abs_box(&app, ids.input);
    let y = by + bh / 2.0;
    let hi = |app: &mut RinchApp, e: PlatformEvent| app.handle_event(e, (1600, 1200), 2.0);
    hi(
        &mut app,
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Right,
        },
    );
    assert!(app.is_text_context_menu_open());
    let panel = app.open_text_menu.as_ref().unwrap().panel_id;
    let (px, py, _, _) = abs_box(&app, panel);
    assert_eq!((px, py), (x, y), "the logical press point at scale 2");
    let before = field_state(&app, ids.input);
    assert_eq!((before.1.as_str(), before.2.as_str()), ("2", "5"));
    let (cx, cy) = row_center(&app, TextEditAction::SelectAll);
    hi(&mut app, PlatformEvent::MouseMove { x: cx, y: cy });
    assert_eq!(highlighted(&app), Some(TextEditAction::SelectAll));
    hi(
        &mut app,
        PlatformEvent::MouseUp {
            x: cx,
            y: cy,
            button: MouseButton::Right,
        },
    );
    assert!(app.is_text_context_menu_open());
    assert_eq!(
        field_state(&app, ids.input),
        before,
        "Select all did not run on the release"
    );
}

/// A press on the panel's own padding keeps it open. (The review's `p8`.)
#[test]
fn a_press_on_the_panels_padding_keeps_it_open() {
    let (mut app, ids, _log) = page("hello world", &[]);
    let (x, y) = abs_center(&app, ids.input);
    right_press(&mut app, x, y);
    let panel = app.open_text_menu.as_ref().unwrap().panel_id;
    let (px, py, _, _) = abs_box(&app, panel);
    assert_eq!(
        row_under(&app, px + 2.0, py + 2.0),
        None,
        "precondition: padding, not a row"
    );
    left_press(&mut app, px + 2.0, py + 2.0);
    assert!(
        app.is_text_context_menu_open(),
        "a padding press keeps the menu"
    );
}

/// A wheel closes it and scrolls nothing. (The review's `p9`.)
#[test]
fn a_wheel_closes_the_menu() {
    let (mut app, ids, _log) = page("hello world", &[]);
    let (x, y) = abs_center(&app, ids.input);
    right_press(&mut app, x, y);
    ev(
        &mut app,
        PlatformEvent::MouseWheel {
            x,
            y,
            delta_x: 0.0,
            delta_y: -20.0,
        },
    );
    assert!(!app.is_text_context_menu_open());
}

/// One panel per app: 200 open/close cycles add no slab entries (the review's
/// `p6`, where each open used to add 22 for the session), and the panel's
/// box is as tall as its rows (the bottom clamp used to leave 3px off).
#[test]
fn two_hundred_cycles_add_no_nodes_and_the_panel_is_as_tall_as_its_rows() {
    let (mut app, ids, _log) = page("hello world", &[]);
    let count = |app: &RinchApp| app.doc.as_ref().unwrap().borrow().tree.nodes.len();
    let (x, y) = abs_center(&app, ids.input);
    right_press(&mut app, x, y);
    let (_, py, _, ph) = abs_box(&app, app.open_text_menu.as_ref().unwrap().panel_id);
    let last = app
        .open_text_menu
        .as_ref()
        .unwrap()
        .items
        .last()
        .unwrap()
        .node_id;
    let (_, ry, _, rh) = abs_box(&app, last);
    // 4px padding + 1px border below the last row. The separator's vertical
    // margins used to be lost, leaving the panel 8px shorter than its rows.
    assert!(
        (py + ph - (ry + rh) - 5.0).abs() < 0.5,
        "panel bottom {} is the last row's bottom {} plus padding and border",
        py + ph,
        ry + rh
    );
    key(&mut app, KeyCode::Escape);
    let base = count(&app);
    for _ in 0..200 {
        right_press(&mut app, x, y);
        key(&mut app, KeyCode::Escape);
    }
    assert_eq!(
        count(&app),
        base,
        "no node was created after the first open"
    );
}

/// The panel's node ids and every node under it.
fn panel_subtree(app: &RinchApp) -> Vec<usize> {
    let Some(panel) = app.text_menu_panel.as_ref().map(|p| p.panel_id) else {
        return Vec::new();
    };
    let d = app.doc.as_ref().unwrap().borrow();
    let mut out = vec![panel];
    let mut i = 0;
    while i < out.len() {
        out.extend(d.tree.get(out[i]).unwrap().children.iter().copied());
        i += 1;
    }
    out
}

/// Hidden between opens, the panel has no box: nothing under the point where
/// its Copy row was is in the panel, and a press there reaches the page.
#[test]
fn a_closed_panel_is_out_of_hit_testing_and_layout() {
    let (mut app, ids, log) = page("hello world", &[]);
    let (x, y) = abs_center(&app, ids.input);
    right_press(&mut app, x, y);
    let panel = app.open_text_menu.as_ref().unwrap().panel_id;
    // A point that is both on the open panel's Copy row and on the plain
    // `data-rid` div beneath it.
    let (cx, cy) = row_center(&app, TextEditAction::Copy);
    let (dx, dy, dw, dh) = abs_box(&app, ids.plain);
    assert!(
        cx >= dx && cx <= dx + dw && cy >= dy && cy <= dy + dh,
        "precondition"
    );
    key(&mut app, KeyCode::Escape);
    assert_eq!(
        abs_box(&app, panel),
        (0.0, 0.0, 0.0, 0.0),
        "display: none — no box at all"
    );
    let hit = {
        let d = app.doc.as_ref().unwrap().borrow();
        let mut chain = Vec::new();
        let mut cur = hit_test(&d.tree, cx, cy);
        while let Some(id) = cur {
            chain.push(id);
            cur = d.tree.get(id).and_then(|n| n.parent);
        }
        chain
    };
    assert!(!hit.is_empty(), "something on the page is hit");
    let subtree = panel_subtree(&app);
    assert!(
        hit.iter().all(|id| !subtree.contains(id)),
        "no node of the hidden panel is hit: {hit:?}"
    );
    left_press(&mut app, cx, cy);
    assert!(
        log.borrow().iter().any(|e| e == "plain-click"),
        "the press reached the page under where the panel was"
    );
    // And the liveness check and dismiss entry still behave on the reused panel.
    let base = dismiss_handler_count();
    right_press(&mut app, x, y);
    assert_eq!(dismiss_handler_count(), base + 1);
    app.doc
        .as_ref()
        .unwrap()
        .borrow_mut()
        .remove_node(NodeId(ids.input));
    ev(&mut app, PlatformEvent::MouseMove { x: 1.0, y: 1.0 });
    assert!(!app.is_text_context_menu_open());
    assert_eq!(dismiss_handler_count(), base);
}

/// Tab after a close never lands in the hidden panel.
#[test]
fn tab_never_reaches_a_closed_panel() {
    let (mut app, ids, _log) = page("hello world", &[]);
    let (x, y) = abs_center(&app, ids.input);
    right_press(&mut app, x, y);
    key(&mut app, KeyCode::Escape);
    let subtree = panel_subtree(&app);
    let mut seen = Vec::new();
    for _ in 0..8 {
        key(&mut app, KeyCode::Tab);
        match app.focus_target {
            FocusTarget::Input(n) | FocusTarget::Node(n) => {
                assert!(!subtree.contains(&n), "Tab reached panel node {n}");
                seen.push(n);
            }
            other => panic!("Tab left the page's focusables: {other:?}"),
        }
    }
    assert!(
        seen.contains(&ids.other),
        "positive control: Tab walked the page: {seen:?}"
    );
}

/// The reused panel keeps its nodes, so an open must rewrite everything the
/// last one left on them. A highlight left by the previous open is gone.
/// (The review's `k3`.)
#[test]
fn reopening_clears_the_previous_opens_highlight() {
    let (mut app, ids, _log) = page("hello world", &[]);
    focus_and_select(&mut app, ids.input);
    assert!(app.open_text_context_menu(400.0, 300.0, 800.0, 600.0));
    let (cx, cy) = row_center(&app, TextEditAction::SelectAll);
    ev(&mut app, PlatformEvent::MouseMove { x: cx, y: cy });
    assert_eq!(highlighted(&app), Some(TextEditAction::SelectAll));
    key(&mut app, KeyCode::Escape);
    assert!(app.open_text_context_menu(400.0, 300.0, 800.0, 600.0));
    let lit: Vec<TextEditAction> = app
        .open_text_menu
        .as_ref()
        .unwrap()
        .items
        .iter()
        .filter(|r| attr(&app, r.node_id, "data-highlighted").is_some())
        .map(|r| r.action)
        .collect();
    assert_eq!(lit, vec![], "no row carries a stale highlight");
}

/// And a row greyed last time and enabled now loses its `data-disabled`:
/// Copy, greyed over a collapsed caret, then over a selection. (The review's
/// `k4`.) Without a clipboard Copy stays greyed and the attribute stays, which
/// is the same rule read the other way.
#[test]
fn reopening_follows_a_rows_new_enabled_state() {
    let (mut app, ids, _log) = page("hello world", &[]);
    let (bx, by, _, bh) = abs_box(&app, ids.input);
    left_press(&mut app, bx + 1.0, by + bh / 2.0);
    assert!(app.open_text_context_menu(400.0, 300.0, 800.0, 600.0));
    let copy = app
        .open_text_menu
        .as_ref()
        .unwrap()
        .items
        .iter()
        .find(|r| r.action == TextEditAction::Copy)
        .unwrap()
        .node_id;
    assert!(
        attr(&app, copy, "data-disabled").is_some(),
        "precondition: Copy greyed with no selection"
    );
    key(&mut app, KeyCode::Escape);
    select_2_to_5(&mut app);
    assert!(app.open_text_context_menu(400.0, 300.0, 800.0, 600.0));
    let enabled = rows(&app)
        .iter()
        .find(|r| r.0 == TextEditAction::Copy)
        .unwrap()
        .1;
    assert_eq!(enabled, cfg!(feature = "clipboard"));
    assert_eq!(
        attr(&app, copy, "data-disabled").is_some(),
        !enabled,
        "the attribute follows the state"
    );
}

// ── The anchor: the real selection rect ──────────────────────────────────────

mod anchor {
    use super::*;

    /// A field 40px from the left edge (its wrapper's padding), so an anchor
    /// at the field's origin and one at x 0 both read as wrong. Tall enough
    /// for a `<textarea>` to show two lines.
    const OFFSET_FIELD: &str = "width: 300px; height: 80px; padding: 0; margin: 0; \
         font-size: 16px; line-height: 20px; font-family: sans-serif";

    fn field_app(tag: &'static str, value: &'static str, style: &'static str) -> (RinchApp, usize) {
        let h = register_input_handler(InputCallback::new(|_| {}));
        let id: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
        let id_in = id.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_attribute("style", "padding-left: 40px");
            let f = scope.create_element(tag);
            f.set_attribute("style", style);
            f.set_attribute("value", value);
            f.set_attribute("data-oninput", &h.0.to_string());
            root.append_child(&f);
            id_in.set(Some(f.node_id().0));
            root
        });
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        (app, id.get().unwrap())
    }

    fn set_selection(app: &mut RinchApp, anchor: usize, head: usize) {
        app.focused_input_state.as_mut().unwrap().selection =
            rinch_editable::Selection::new(anchor, head);
        app.sync_input_cursor_to_dom();
    }

    /// The x at which the caret for `offset` is drawn, bracketed by the press
    /// map: pressing just left of it still lands on `offset`, pressing at the
    /// first x that maps to `offset + 1` does not.
    fn assert_caret_x(app: &mut RinchApp, input: usize, x: f32, offset: usize) {
        let lo = x_for_offset(app, input, offset);
        let hi = x_for_offset(app, input, offset + 1);
        assert!(
            lo < x && x < hi,
            "caret for offset {offset} at x {x} must lie between the first x mapping to it \
             ({lo}) and the first x mapping to the next offset ({hi})"
        );
    }

    #[test]
    fn a_collapsed_caret_is_a_one_pixel_rect_at_the_caret() {
        let (mut app, input) = field_app("input", "hello world", OFFSET_FIELD);
        let (bx, by, _, bh) = abs_box(&app, input);
        assert_eq!(bx, 40.0, "precondition: the field is off x 0");
        left_press(&mut app, bx + 1.0, by + bh / 2.0);
        set_selection(&mut app, 4, 4);
        let s = app.text_edit_state().unwrap();
        let (ax, ay, aw, ah) = s.anchor;
        assert_caret_x(&mut app, input, ax, 4);
        assert_eq!(aw, 1.0);
        assert!(
            ah > 0.0 && ah <= bh,
            "line height {ah} inside the field's {bh}"
        );
        assert!(
            ay >= by && ay + ah <= by + bh,
            "vertically inside the field"
        );
    }

    #[test]
    fn a_selection_on_one_line_spans_its_two_carets() {
        let (mut app, input) = field_app("input", "hello world", OFFSET_FIELD);
        let (bx, by, _, bh) = abs_box(&app, input);
        left_press(&mut app, bx + 1.0, by + bh / 2.0);
        set_selection(&mut app, 5, 2);
        let s = app.text_edit_state().unwrap();
        let (ax, ay, aw, ah) = s.anchor;
        assert_caret_x(&mut app, input, ax, 2);
        assert_caret_x(&mut app, input, ax + aw - 1.0, 5);
        assert!(aw > 10.0, "three glyphs wide: {aw}");
        assert!(ay >= by && ay + ah <= by + bh);
    }

    #[test]
    fn a_textarea_selection_across_two_lines_is_the_box_of_both_carets() {
        // 300px wide at 16px: the value wraps onto a second line.
        let value = "the quick brown fox jumps over the lazy dog and keeps running";
        let (mut app, input) = field_app("textarea", value, OFFSET_FIELD);
        let (bx, by, _, bh) = abs_box(&app, input);
        left_press(&mut app, bx + 1.0, by + 5.0);
        // A caret near the start and one near the end: different lines.
        set_selection(&mut app, 3, 3);
        let (_, y1, _, h1) = app.text_edit_state().unwrap().anchor;
        set_selection(&mut app, 55, 55);
        let (x2, y2, _, _) = app.text_edit_state().unwrap().anchor;
        assert!(
            y2 > y1 + h1 * 0.5,
            "the second caret is on a later line: {y1} vs {y2}"
        );
        set_selection(&mut app, 3, 55);
        let (ax, ay, aw, ah) = app.text_edit_state().unwrap().anchor;
        assert_eq!(
            ay,
            y1.max(by),
            "top of the first caret's line, inside the field"
        );
        assert!(
            ah > h1 * 1.5,
            "spans both lines: {ah} against one line's {h1}"
        );
        assert!(ax <= x2 && ax + aw >= x2, "the end caret is inside the box");
        assert!(ay >= by && ay + ah <= by + bh);
    }

    #[test]
    fn a_password_field_measures_through_its_bullets() {
        let (mut app, input) = field_app("input", "hello world", OFFSET_FIELD);
        app.doc
            .as_ref()
            .unwrap()
            .borrow_mut()
            .set_attribute(NodeId(input), "type", "password");
        app.resolve_and_repaint(800.0, 600.0);
        let (bx, by, _, bh) = abs_box(&app, input);
        left_press(&mut app, bx + 1.0, by + bh / 2.0);
        set_selection(&mut app, 2, 5);
        let (ax, _, aw, _) = app.text_edit_state().unwrap().anchor;
        assert_caret_x(&mut app, input, ax, 2);
        assert_caret_x(&mut app, input, ax + aw - 1.0, 5);
    }

    #[test]
    fn an_empty_field_falls_back_to_the_text_origin() {
        let (mut app, input) = field_app("input", "", OFFSET_FIELD);
        let (bx, by, _, bh) = abs_box(&app, input);
        left_press(&mut app, bx + 1.0, by + bh / 2.0);
        let s = app.text_edit_state().unwrap();
        assert_eq!(s.anchor, (bx, by, 1.0, bh));
    }
}

// ── A `value` the field has not adopted yet ──────────────────────────────────

/// An app write to the focused field from outside any keystroke — a timer, a
/// drained `Signal::send`, a `data-onmousedown` handler — lands in the DOM at
/// once and in the field's editable state only at the next adoption (issue
/// #238), which every frame makes. Until then the engine's offsets index the
/// old text and the DOM's `value` the new one. Both halves of the seam can be
/// reached in that window: a shell polls the state between frames, and a
/// right press arrives as an event, before any frame.
mod unadopted_write {
    use super::*;

    /// A focused field the user typed `typed` into, through the real key path.
    fn typed_field(kind: &'static str, typed: &str) -> (RinchApp, NodeHandle) {
        let node: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
        let h = register_input_handler(InputCallback::new(|_| {}));
        let node_in = node.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_attribute("style", "padding-left: 40px; padding-top: 25px");
            let f = scope.create_element("input");
            f.set_attribute("type", kind);
            f.set_attribute("style", FIELD_STYLE);
            f.set_attribute("value", "");
            f.set_attribute("data-oninput", &h.0.to_string());
            root.append_child(&f);
            *node_in.borrow_mut() = Some(f.clone());
            root
        });
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        let f = node.borrow_mut().take().expect("captured at mount");
        let id = f.node_id().0;
        let (bx, by, _, bh) = abs_box(&app, id);
        left_press(&mut app, bx + 2.0, by + bh / 2.0);
        assert_eq!(app.focus_target, FocusTarget::Input(id));
        for ch in typed.chars() {
            let s = ch.to_string();
            key_with(&mut app, KeyCode::KeyA, Some(&s), Modifiers::default());
        }
        app.resolve_and_repaint(800.0, 600.0);
        assert_eq!(attr(&app, id, "value").as_deref(), Some(typed));
        (app, f)
    }

    /// The review's `k7`: `hello` typed into a password field (caret at byte
    /// 5), then `€€` written from outside an event (6 bytes, byte 5 inside the
    /// second `€`), then the state polled before the next frame. It used to
    /// slice the new value at the engine's old offset and panic.
    #[test]
    fn polling_a_password_field_after_an_unadopted_multibyte_write_does_not_panic() {
        let (mut app, f) = typed_field("password", "hello");
        f.set_attribute("value", "\u{20ac}\u{20ac}");
        let s = app
            .text_edit_state()
            .expect("the field still holds the keyboard");
        assert!(s.can_select_all, "it answers for the new value");
        assert!(!s.can_cut && !s.can_copy, "a write leaves no selection");
    }

    /// The non-panicking half of the same window: a plain field reports the
    /// caret for the text it displays, so a poll before the frame answers what
    /// a poll after it does. `hello` → `hello, world` keeps the typed prefix,
    /// so the adopted caret stays at 5 while the engine's is also 5 — the two
    /// would agree by accident — hence `xyhello`, which moves it to 7.
    #[test]
    fn a_poll_before_the_frame_answers_what_a_poll_after_it_does() {
        let (mut app, f) = typed_field("text", "hello");
        f.set_attribute("value", "xyhello");
        let before_frame = app.text_edit_state().unwrap();
        app.resolve_and_repaint(800.0, 600.0);
        let after_frame = app.text_edit_state().unwrap();
        assert_eq!(before_frame, after_frame);
        assert_eq!(
            attr(&app, f.node_id().0, "data-cursor-pos").as_deref(),
            Some("7"),
            "precondition: the adopted caret moved off the typed one"
        );
    }

    /// A focused field inside a `data-rid` container whose click handler writes
    /// the field's `value` — during the press itself.
    fn rid_writing_page(new_value: &'static str) -> (RinchApp, usize, Rc<Cell<u32>>) {
        let node: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
        let fired = Rc::new(Cell::new(0u32));
        let h = register_input_handler(InputCallback::new(|_| {}));
        let (node_cb, fired_cb) = (node.clone(), fired.clone());
        let rid = rinch_core::register_handler(Rc::new(move || {
            fired_cb.set(fired_cb.get() + 1);
            if let Some(n) = node_cb.borrow().as_ref() {
                n.set_attribute("value", new_value);
            }
        }));
        let node_in = node.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            let wrap = scope.create_element("div");
            wrap.set_attribute("data-rid", &rid.0.to_string());
            wrap.set_attribute("style", "padding: 10px");
            let f = scope.create_element("input");
            f.set_attribute("style", FIELD_STYLE);
            f.set_attribute("value", "hello world");
            f.set_attribute("data-oninput", &h.0.to_string());
            wrap.append_child(&f);
            root.append_child(&wrap);
            *node_in.borrow_mut() = Some(f.clone());
            root
        });
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        let id = node.borrow().as_ref().unwrap().node_id().0;
        (app, id, fired)
    }

    /// A right press inside the selection keeps a write its `data-rid`
    /// ancestor made during the press, as a left press and a right press
    /// outside the selection do. The caret rule used to put the saved
    /// selection back and sync the state's old text over the write. (The
    /// review's round-3 fixture.)
    #[test]
    fn rid_write_survives_a_right_press_inside_the_selection() {
        for v in ["HELLO WORLD", "\u{20ac}\u{20ac}\u{20ac}\u{20ac}"] {
            let press = |button: MouseButton| {
                let (mut app, id, fired) = rid_writing_page(v);
                let (bx, by, _, bh) = abs_box(&app, id);
                left_press(&mut app, bx + 1.0, by + bh / 2.0);
                // Undo the focusing press's own write.
                app.doc.as_ref().unwrap().borrow_mut().set_attribute(
                    NodeId(id),
                    "value",
                    "hello world",
                );
                app.resolve_and_repaint(800.0, 601.0);
                select_2_to_5(&mut app);
                assert_eq!(
                    field_state(&app, id),
                    ("hello world".into(), "2".into(), "5".into())
                );
                let f0 = fired.get();
                let x = x_for_offset(&mut app, id, 3);
                press_with(&mut app, x, by + bh / 2.0, button);
                assert_eq!(fired.get(), f0 + 1, "positive control: the rid ran");
                let menu = app.is_text_context_menu_open();
                let _ = app.text_edit_state();
                app.resolve_and_repaint(800.0, 600.0);
                (field_state(&app, id), menu)
            };
            let (right, menu) = press(MouseButton::Right);
            assert!(menu, "the right press opened the menu");
            assert_eq!(right.0, v, "the app's write survives the gesture");
            // The write replaced the text the saved selection indexed, so the
            // caret is where the write left it — as after a left press — and
            // not the old 2..5, which on `€€€€` is inside a character.
            let (left, _) = press(MouseButton::Left);
            assert_eq!(right, left, "a right press ends as a left press does");
        }
    }

    /// The gesture's caret rule compares the selection it saved against the
    /// press offset. Saved from an unadopted state, the selection indexed the
    /// old text while the offset indexed the new one, and a press "inside" it
    /// restored `2..5` onto a value whose bytes 2 and 5 are inside a `€`.
    #[test]
    fn a_right_press_after_an_unadopted_write_does_not_restore_stale_offsets() {
        let (mut app, f) = typed_field("text", "hello world");
        let id = f.node_id().0;
        select_2_to_5(&mut app);
        assert_eq!(
            field_state(&app, id),
            ("hello world".into(), "2".into(), "5".into()),
            "precondition: bytes 2..5 selected"
        );
        f.set_attribute("value", "\u{20ac}\u{20ac}\u{20ac}\u{20ac}");
        // Byte 3 is inside the stale 2..5 — and a boundary of the new value.
        let x = x_for_offset(&mut app, id, 3);
        let (_, by, _, bh) = abs_box(&app, id);
        right_press(&mut app, x, by + bh / 2.0);
        assert!(app.is_text_context_menu_open());
        assert_eq!(
            field_state(&app, id),
            (
                "\u{20ac}\u{20ac}\u{20ac}\u{20ac}".into(),
                "3".into(),
                "3".into()
            ),
            "the write collapsed the selection, so the press moved the caret"
        );
    }
}

// ── The anchor against the pixels paint draws ────────────────────────────────

/// The `anchor` fixtures above bracket the anchor with the press map, on
/// fields with `padding: 0` and a line that fills the box — where an anchor
/// that forgot the padding, the single-line centring or the line height still
/// passes. These compare it with the caret **paint** draws: two full software
/// frames that differ only in `data-cursor-visible`, diffed. (The review's
/// `k1`/`k2`.)
///
/// `software_shell` only, like every `build_pixels` oracle: `--workspace`
/// unifies `rinch/gpu` on, so these run in CI's `-p rinch --features …` step.
#[cfg(software_shell)]
mod anchor_against_paint {
    use super::*;

    /// A full, non-incremental software frame at scale 1.
    pub(super) fn frame(app: &mut RinchApp) -> (Vec<u8>, u32, u32) {
        app.scene_dirty = true;
        app.has_previous_frame = false;
        let (p, w, h) = app.build_pixels(1.0, VP, false);
        (p.to_vec(), w, h)
    }

    /// The `(x0, y0, x1, y1)` box, in logical px at scale 1, of every pixel
    /// that differs between two frames.
    pub(super) fn diff_box(a: &[u8], b: &[u8], w: u32, h: u32) -> Option<(f32, f32, f32, f32)> {
        let mut bb: Option<(u32, u32, u32, u32)> = None;
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                if (0..4).any(|k| (a[i + k] as i32 - b[i + k] as i32).abs() > 8) {
                    bb = Some(match bb {
                        None => (x, y, x, y),
                        Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                    });
                }
            }
        }
        bb.map(|(x0, y0, x1, y1)| (x0 as f32, y0 as f32, (x1 + 1) as f32, (y1 + 1) as f32))
    }

    fn set_selection(app: &mut RinchApp, anchor: usize, head: usize) {
        app.focused_input_state.as_mut().unwrap().selection =
            rinch_editable::Selection::new(anchor, head);
        app.sync_input_cursor_to_dom();
    }

    pub(super) fn caret_visible(app: &RinchApp, id: usize, visible: bool) {
        app.doc
            .as_ref()
            .unwrap()
            .borrow_mut()
            .tree
            .get_mut(id)
            .unwrap()
            .attributes
            .insert("data-cursor-visible".into(), visible.to_string());
    }

    /// The box paint draws the collapsed caret at `offset` in.
    fn painted_caret(app: &mut RinchApp, id: usize, offset: usize) -> (f32, f32, f32, f32) {
        set_selection(app, offset, offset);
        caret_visible(app, id, true);
        let (on, w, h) = frame(app);
        caret_visible(app, id, false);
        let (off, _, _) = frame(app);
        diff_box(&on, &off, w, h).expect("paint drew a caret")
    }

    /// A focused field 40px from the left and 25px from the top.
    fn focused_field(
        tag: &'static str,
        value: &'static str,
        style: &'static str,
    ) -> (RinchApp, usize) {
        let h = register_input_handler(InputCallback::new(|_| {}));
        let id: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
        let id_in = id.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_attribute("style", "padding-left: 40px; padding-top: 25px");
            let f = scope.create_element(tag);
            f.set_attribute("style", style);
            f.set_attribute("value", value);
            f.set_attribute("data-oninput", &h.0.to_string());
            root.append_child(&f);
            id_in.set(Some(f.node_id().0));
            root
        });
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        let f = id.get().unwrap();
        let (bx, by, _, bh) = abs_box(&app, f);
        left_press(&mut app, bx + 2.0, by + bh.min(20.0) / 2.0);
        assert_eq!(app.focus_target, FocusTarget::Input(f));
        (app, f)
    }

    /// A tall, padded single-line input — the shape every styled `TextInput`
    /// has: the anchor sits where paint draws the caret on both axes, and is
    /// about as tall. Kills an anchor without `padding_left`, without the
    /// single-line centring, and a 1px-tall one.
    #[test]
    fn a_padded_tall_inputs_caret_anchor_is_where_paint_draws_the_caret() {
        let (mut app, f) = focused_field(
            "input",
            "hello  world  again",
            "width: 300px; height: 80px; padding: 5px 12px; margin: 0; \
             box-sizing: border-box; font-size: 16px; line-height: 20px; \
             font-family: sans-serif",
        );
        set_selection(&mut app, 9, 9);
        let (ax, ay, _, ah) = app.text_edit_state().unwrap().anchor;
        let (px, py, _, py1) = painted_caret(&mut app, f, 9);
        assert!((ax - px).abs() <= 1.5, "x: anchor {ax}, paint {px}");
        assert!((ay - py).abs() <= 2.0, "y: anchor {ay}, paint {py}");
        assert!(
            ah >= 0.8 * (py1 - py),
            "height: anchor {ah}, paint {}",
            py1 - py
        );
    }

    /// A `<textarea>` selection across two lines: the anchor box runs from
    /// the start caret paint draws to the end caret paint draws.
    #[test]
    fn a_two_line_textarea_selections_anchor_is_the_box_of_the_painted_carets() {
        let (mut app, f) = focused_field(
            "textarea",
            "the quick brown fox jumps over the lazy dog and keeps running far",
            "width: 300px; height: 80px; padding: 0; margin: 0; font-size: 16px; \
             line-height: 20px; font-family: sans-serif",
        );
        let c1 = painted_caret(&mut app, f, 7);
        let c2 = painted_caret(&mut app, f, 52);
        assert!(c2.1 > c1.1 + 10.0, "different lines: {c1:?} {c2:?}");
        set_selection(&mut app, 7, 52);
        let (ax, ay, aw, ah) = app.text_edit_state().unwrap().anchor;
        assert!(
            (ax - c1.0.min(c2.0)).abs() <= 1.5,
            "left: {ax} vs {c1:?} {c2:?}"
        );
        assert!(
            ((ax + aw - 1.0) - c1.0.max(c2.0)).abs() <= 1.5,
            "right: {} vs {c1:?} {c2:?}",
            ax + aw - 1.0
        );
        // Paint draws the first line's caret 1px above the field's box; the
        // anchor is clipped to the box, hence 2px.
        assert!((ay - c1.1).abs() <= 2.0, "top: {ay} vs {c1:?}");
        assert!(
            (ay + ah - c2.3).abs() <= 4.0,
            "bottom: {} vs {c2:?}",
            ay + ah
        );
    }
}

// ── The closed panel against the pixels paint draws ──────────────────────────

#[cfg(software_shell)]
mod closed_panel_against_paint {
    use super::anchor_against_paint::{caret_visible, diff_box, frame};
    use super::*;

    /// Hidden, the panel paints nothing: after a close the frame is the page's
    /// frame from before the first open, pixel for pixel — the incremental
    /// frame (the software painter's dirty-region path) and a full one alike,
    /// and again after reopening somewhere else. A panel hidden by
    /// `visibility: hidden` instead fails it: its rows' text is still drawn
    /// (#829).
    #[test]
    fn a_closed_panel_paints_nothing() {
        let (mut app, ids, _log) = page("hello world", &[]);
        focus_and_select(&mut app, ids.input);
        caret_visible(&app, ids.input, false);
        let (before, w, h) = frame(&mut app);
        for (x, y) in [(400.0, 300.0), (100.0, 400.0)] {
            assert!(app.open_text_context_menu(x, y, 800.0, 600.0));
            caret_visible(&app, ids.input, false);
            app.scene_dirty = true;
            assert!(app.has_previous_frame, "precondition: an incremental frame");
            let open = app.build_pixels(1.0, VP, false).0.to_vec();
            let panel = app.open_text_menu.as_ref().unwrap().panel_id;
            let (px, py, pw, ph) = abs_box(&app, panel);
            let shown = diff_box(&before, &open, w, h).expect("positive control: the panel paints");
            assert!(
                shown.0 >= px - 16.0 && shown.2 <= px + pw + 16.0 && shown.3 > py + ph / 2.0,
                "the change is the panel ({px}, {py}, {pw}, {ph}): {shown:?}"
            );
            key(&mut app, KeyCode::Escape);
            assert!(!app.is_text_context_menu_open());
            caret_visible(&app, ids.input, false);
            app.scene_dirty = true;
            let after = app.build_pixels(1.0, VP, false).0.to_vec();
            assert_eq!(
                diff_box(&before, &after, w, h),
                None,
                "the incremental frame after closing the menu opened at ({x}, {y})"
            );
        }
        let (full, _, _) = frame(&mut app);
        assert_eq!(
            diff_box(&before, &full, w, h),
            None,
            "a full frame after the close"
        );
    }
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
        let clip = cfg!(feature = "clipboard");
        assert_eq!(flags(&s), (clip, clip, clip, true));
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

    /// An empty editor is one empty paragraph: nothing to select, so Select
    /// all is greyed. (The review's `k5`; kills a `content_size() > 0` rule.)
    #[test]
    fn an_empty_editor_greys_select_all() {
        let mut p = editor_page("<p></p>");
        let (bx, by, _, _) = abs_box(&p.app, p.container);
        right_press(&mut p.app, bx + 20.0, by + 10.0);
        assert!(p.app.is_text_context_menu_open());
        assert!(!p.app.text_edit_state().unwrap().can_select_all);
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

// ── Cost of polling the state (a shell polls it while its toolbar is up) ─────

/// Not a pin — a measurement, `#[ignore]`d. Run with
/// `cargo test -p rinch --lib text_context_menu_tests::bench -- --ignored --nocapture`.
#[cfg(feature = "desktop")]
mod bench {
    use super::*;
    use rinch_editor_core::{Pos, Selection};

    #[test]
    #[ignore]
    fn text_edit_state_per_call_on_a_400_paragraph_document() {
        let html: &'static str = Box::leak(
            (0..400)
                .map(|i| {
                    format!("<p>paragraph {i} with some <strong>bold</strong> words in it</p>")
                })
                .collect::<String>()
                .into_boxed_str(),
        );
        let slot: Rc<RefCell<Option<(usize, crate::editor::EditorHandle)>>> =
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
            root.append_child(&container);
            *slot_in.borrow_mut() = Some((container.node_id().0, handle));
            root
        });
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        let (c, h) = slot.borrow_mut().take().unwrap();
        let (x, y, hh) = app.editor_caret_point(&h, Pos(3)).unwrap();
        left_press(&mut app, x + 1.0, y + hh / 2.0);
        assert_eq!(app.focus_target, FocusTarget::Editor(c));
        let per_call = |app: &mut RinchApp| {
            let t = Instant::now();
            for _ in 0..200 {
                assert!(app.text_edit_state().is_some());
            }
            t.elapsed().as_secs_f64() * 1000.0 / 200.0
        };
        let collapsed = per_call(&mut app);
        h.set_selection(Selection::text(Pos(3), Pos(12000)));
        let selected = per_call(&mut app);
        eprintln!(
            "BENCH text_edit_state() per call: collapsed {collapsed:.3}ms, \
             ~12k-char selection {selected:.3}ms (400-paragraph doc, debug build)"
        );
    }

    /// What keeping the panel costs a page that is not using it: ms per layout
    /// pass on a themed page before the menu is first opened, and after one
    /// open and close (#826 — text under `display: none` is re-laid-out on
    /// every pass). Run with `--release`.
    #[test]
    #[ignore]
    fn a_layout_pass_before_the_first_open_and_after_a_close() {
        let (mut app, ids, _log) = page("hello world", &[]);
        {
            let mut d = app.doc.as_ref().unwrap().borrow_mut();
            d.load_css(&rinch_theme::generate_theme_css(
                &rinch_theme::Theme::default(),
            ));
            d.load_css(&rinch_components::generate_component_css());
            d.recompute_all_styles_full();
        }
        app.resolve_and_repaint(800.0, 600.0);
        // A width flip on a plain div: a real layout pass each time, and
        // nothing about the menu.
        let per_pass = |app: &mut RinchApp| {
            let pass = |app: &mut RinchApp, i: u32| {
                let w = if i.is_multiple_of(2) {
                    "280px"
                } else {
                    "290px"
                };
                app.doc
                    .as_ref()
                    .unwrap()
                    .borrow_mut()
                    .set_style(NodeId(ids.plain), "width", w);
                app.resolve_and_repaint(800.0, 600.0);
            };
            for i in 0..50 {
                pass(app, i);
            }
            let computes = app.doc.as_ref().unwrap().borrow().tree.taffy_computes;
            let t = Instant::now();
            for i in 0..1000 {
                pass(app, i);
            }
            let ms = t.elapsed().as_secs_f64() * 1000.0 / 1000.0;
            let computed = app.doc.as_ref().unwrap().borrow().tree.taffy_computes - computes;
            assert!(
                computed >= 1000,
                "positive control: every pass laid out ({computed})"
            );
            ms
        };
        let before = per_pass(&mut app);
        let (x, y) = abs_center(&app, ids.input);
        right_press(&mut app, x, y);
        assert!(app.is_text_context_menu_open());
        key(&mut app, KeyCode::Escape);
        assert!(!app.is_text_context_menu_open());
        let after = per_pass(&mut app);
        eprintln!(
            "BENCH layout pass: before the first open {before:.4}ms, after a close {after:.4}ms"
        );
    }
}
