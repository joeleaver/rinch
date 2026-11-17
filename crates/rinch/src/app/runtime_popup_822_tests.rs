//! What an open **runtime popup** — the built-in text context menu (#813) or
//! the native `<select>` popup (#671) — does with the input it does not take
//! through its pointer and key arms (issue #822). The rule, decided once: a
//! popup owns the input while it is open, as a native popup menu does, and
//! closes when the window changes under it.
//!
//! Each fixture has a positive control — the same input with the popup closed
//! does what it always did — so a run where the input never arrived at all
//! cannot pass.

use super::*;
use rinch_core::events::{InputCallback, register_input_handler};
use std::cell::Cell;
use winit::keyboard::KeyCode as W;

const VP: (u32, u32) = (800, 600);

const FIELD_STYLE: &str = "width: 300px; height: 30px; padding: 0; margin: 0; \
     font-size: 16px; line-height: 20px; font-family: sans-serif";

#[derive(Clone, Copy)]
struct Ids {
    input: u64,
    select: u64,
}

fn page() -> (RinchApp, Ids) {
    let input_h = register_input_handler(InputCallback::new(|_v: String| {}));
    let ids: Rc<Cell<Option<Ids>>> = Rc::new(Cell::new(None));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let input = scope.create_element("input");
        input.set_attribute("style", FIELD_STYLE);
        input.set_attribute("value", "abcdefghij");
        input.set_attribute("data-oninput", &input_h.0.to_string());
        let select = scope.create_element("select");
        select.set_attribute("style", "width: 200px; height: 30px");
        for label in ["one", "two", "three"] {
            let opt = scope.create_element("option");
            opt.set_attribute("value", label);
            let t = scope.create_text(label);
            opt.append_child(&t);
            select.append_child(&opt);
        }
        root.append_child(&input);
        root.append_child(&select);
        ids_in.set(Some(Ids {
            input: input.node_id().0,
            select: select.node_id().0,
        }));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    (app, ids.get().expect("ids captured at mount"))
}

fn abs_box(app: &RinchApp, id: u64) -> (f32, f32, f32, f32) {
    let d = app.doc.as_ref().unwrap().borrow();
    painted_element_box(&d.tree, id)
}

fn value(app: &RinchApp, id: u64) -> String {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree
        .get(id)
        .and_then(|n| n.attributes.get("value").cloned())
        .unwrap_or_default()
}

fn press(app: &mut RinchApp, x: f32, y: f32, button: MouseButton) {
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
}

/// Focus the field with a left press at its right end (caret at the end),
/// then open the text menu with a right press `dx` px into it.
fn open_menu_at(app: &mut RinchApp, ids: Ids, dx: f32) -> (f32, f32) {
    let (x, y, w, h) = abs_box(app, ids.input);
    press(app, x + w - 2.0, y + h / 2.0, MouseButton::Left);
    assert_eq!(app.focus_target, FocusTarget::Input(ids.input));
    let at = (x + dx, y + h / 2.0);
    press(app, at.0, at.1, MouseButton::Right);
    assert!(app.is_text_context_menu_open(), "precondition: menu open");
    at
}

fn escape(app: &mut RinchApp) {
    app.handle_event(
        PlatformEvent::KeyDown {
            key: KeyCode::Escape,
            logical_key: None,
            text: None,
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Fresh,
        },
        VP,
        1.0,
    );
    app.handle_event(
        PlatformEvent::KeyUp {
            key: KeyCode::Escape,
            logical_key: None,
            modifiers: Modifiers::default(),
        },
        VP,
        1.0,
    );
}

fn ctrl() -> Modifiers {
    Modifiers {
        ctrl: true,
        ..Modifiers::default()
    }
}

/// Arm one menu holding `Ctrl+N`, counting into the returned cell.
fn arm_ctrl_n() -> (crate::menu::MenuBarChords, Rc<Cell<u32>>) {
    let fired = Rc::new(Cell::new(0u32));
    let c = fired.clone();
    let menu = crate::menu::Menu::new().item(
        crate::menu::MenuItem::new("New")
            .shortcut("Ctrl+N")
            .on_click(move || c.set(c.get() + 1)),
    );
    (
        crate::menu::register_menu_shortcuts(&[("File", &menu)]),
        fired,
    )
}

fn try_ctrl_n(app: &RinchApp) -> bool {
    app.try_menu_shortcut(
        ctrl(),
        W::KeyN,
        &winit::keyboard::Key::Character("n".into()),
        None,
    )
}

/// #822 item 1: an `App::menu` chord does not run under an open runtime
/// popup — a native popup menu holds the keyboard — and runs again once the
/// popup closed.
#[test]
fn a_menu_chord_does_not_run_under_an_open_text_menu_or_select_popup() {
    let (_chords, fired) = arm_ctrl_n();
    let (mut app, ids) = page();

    open_menu_at(&mut app, ids, 40.0);
    assert!(!try_ctrl_n(&app), "refused under the text menu");
    assert_eq!(fired.get(), 0);
    escape(&mut app);
    assert!(!app.is_text_context_menu_open());
    assert!(try_ctrl_n(&app), "positive control: the chord runs");
    assert_eq!(fired.get(), 1);

    app.open_select_popup(ids.select, 800.0, 600.0);
    assert!(app.is_select_open(), "precondition: the popup is open");
    assert!(!try_ctrl_n(&app), "refused under the <select> popup");
    assert_eq!(fired.get(), 1);
}

/// #822 item 2: the input method is off while the text menu is open, and a
/// commit that arrives anyway is not typed into the field under it. Closed,
/// the same commit is typed (the positive control).
#[test]
fn composition_does_not_reach_the_field_under_the_open_text_menu() {
    let (mut app, ids) = page();
    open_menu_at(&mut app, ids, 40.0);
    assert!(!app.ime_state().enabled, "the IME is off under the menu");

    app.handle_event(PlatformEvent::Ime(ImeEvent::Commit("XY".into())), VP, 1.0);
    assert_eq!(value(&app, ids.input), "abcdefghij", "nothing typed");
    assert!(app.is_text_context_menu_open(), "and the menu stays");

    escape(&mut app);
    assert!(app.ime_state().enabled, "back on once it closed");
    app.handle_event(PlatformEvent::Ime(ImeEvent::Commit("XY".into())), VP, 1.0);
    assert_ne!(
        value(&app, ids.input),
        "abcdefghij",
        "positive control: the commit is typed with the menu closed"
    );
}

/// #822 item 5: a resize closes both popups (Chrome closes its menus and its
/// `<select>` popup on a bounds change); the `<select>` keeps the keyboard.
#[test]
fn a_resize_closes_the_text_menu_and_the_select_popup() {
    let (mut app, ids) = page();
    open_menu_at(&mut app, ids, 40.0);
    app.handle_event(
        PlatformEvent::Resized {
            width: 640,
            height: 480,
        },
        (640, 480),
        1.0,
    );
    assert!(!app.is_text_context_menu_open(), "the text menu closed");
    assert_eq!(
        app.focus_target,
        FocusTarget::Input(ids.input),
        "the field keeps the keyboard"
    );

    app.open_select_popup(ids.select, 640.0, 480.0);
    assert!(app.is_select_open(), "precondition: the popup is open");
    app.handle_event(
        PlatformEvent::Resized {
            width: 700,
            height: 500,
        },
        (700, 500),
        1.0,
    );
    assert!(!app.is_select_open(), "the <select> popup closed");
    assert_eq!(
        app.focus_target,
        FocusTarget::Node(ids.select),
        "and handed the keyboard back to its control"
    );
}

/// #822 item 3: an OS file drag arriving over the window closes the menu.
#[test]
fn a_file_drag_closes_the_text_menu() {
    let (mut app, ids) = page();
    open_menu_at(&mut app, ids, 40.0);
    app.handle_event(
        PlatformEvent::FileHoverEnter {
            path: std::path::PathBuf::from("/tmp/a.txt"),
            position: (600.0, 400.0),
        },
        VP,
        1.0,
    );
    assert!(!app.is_text_context_menu_open());
}

/// #822 item 6: a second right press on the field, outside the open menu,
/// opens it again **at the new point** (GTK and Chrome), rather than only
/// closing it. A left press outside still only closes it.
#[test]
fn a_second_right_press_on_the_field_reopens_the_menu_where_it_landed() {
    let (mut app, ids) = page();
    let first = open_menu_at(&mut app, ids, 40.0);
    // Left of the first point: outside the panel, which opened there and
    // extends right and down.
    let (x, _, _, _) = abs_box(&app, ids.input);
    let second = (x + 8.0, first.1);
    assert!(second.0 < first.0, "precondition: outside the panel");
    press(&mut app, second.0, second.1, MouseButton::Right);

    assert!(app.is_text_context_menu_open(), "open again");
    let panel = app.open_text_menu.as_ref().unwrap().panel_id;
    let (px, py, _, _) = abs_box(&app, panel);
    assert_eq!((px, py), second, "at the second press");

    press(&mut app, x + 4.0, first.1, MouseButton::Left);
    assert!(!app.is_text_context_menu_open(), "a left press only closes");
}

/// #822 item 7: a field disabled under its open menu closes the menu.
#[test]
fn disabling_the_field_closes_its_text_menu() {
    let (mut app, ids) = page();
    open_menu_at(&mut app, ids, 40.0);
    app.doc.as_ref().unwrap().borrow_mut().set_attribute(
        rinch_core::dom::NodeId(ids.input),
        "disabled",
        "",
    );
    app.handle_event(PlatformEvent::MouseMove { x: 1.0, y: 1.0 }, VP, 1.0);
    assert!(!app.is_text_context_menu_open());
}
