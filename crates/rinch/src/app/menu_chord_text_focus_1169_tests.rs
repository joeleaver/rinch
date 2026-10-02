//! A menu chord with no Ctrl/Cmd/Alt yields its key to a focused text target
//! (issue #1169).
//!
//! The desktop shell tries menu chords before it hands a key to the app, and
//! `return`s when one ran. A modifier-less chord (`"/"`, `"N"`, `"Delete"`) is
//! the same keystroke a text field types or edits with, so while it was live the
//! key never reached the field: typing `/` into an `<input>` inserted nothing.
//! A browser's page shortcut and a native macOS key equivalent both leave such a
//! key to the focused field; the chord answers only when focus is elsewhere.
//!
//! [`RinchApp::try_menu_shortcut`] is what the shell calls, so the fixtures call
//! it with the keyboard held by each kind of target, and then hand the same key
//! to the app the way the shell does when the chord did not run.
//!
//! Off the fixed points: the field already holds `ab` with the caret at its end,
//! so an inserted `/` is visible as `ab/`, and every case that must *not* yield
//! (Ctrl, Alt, F5, Escape, focus elsewhere) is sampled beside one that must.

use super::*;
use rinch_core::events::{InputCallback, register_input_handler};
use std::cell::Cell;
use winit::keyboard::KeyCode as W;

const VP: (u32, u32) = (800, 600);

const FIELD_STYLE: &str = "width: 300px; height: 30px; padding: 0; margin: 0; \
     font-size: 16px; line-height: 20px; font-family: sans-serif";

#[derive(Clone, Copy)]
struct Ids {
    input: usize,
    textarea: usize,
    checkbox: usize,
    other: usize,
    /// A `tabindex` div registered for composition: a custom text target.
    ime: usize,
}

fn page() -> (RinchApp, Ids) {
    let input_h = register_input_handler(InputCallback::new(|_v: String| {}));
    let textarea_h = register_input_handler(InputCallback::new(|_v: String| {}));
    let checkbox_h = register_input_handler(InputCallback::new(|_v: String| {}));
    let ids: Rc<Cell<Option<Ids>>> = Rc::new(Cell::new(None));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let input = scope.create_element("input");
        input.set_attribute("style", FIELD_STYLE);
        input.set_attribute("value", "ab");
        input.set_attribute("data-oninput", &input_h.0.to_string());
        let textarea = scope.create_element("textarea");
        textarea.set_attribute("style", FIELD_STYLE);
        textarea.set_attribute("value", "ab");
        textarea.set_attribute("data-oninput", &textarea_h.0.to_string());
        let checkbox = scope.create_element("input");
        checkbox.set_attribute("type", "checkbox");
        checkbox.set_attribute("style", "width: 20px; height: 20px");
        checkbox.set_attribute("data-oninput", &checkbox_h.0.to_string());
        let other = scope.create_element("div");
        other.set_attribute(
            "style",
            "position: absolute; left: 500px; top: 450px; width: 200px; height: 40px",
        );
        other.set_attribute("tabindex", "0");
        let ime = scope.create_element("div");
        ime.set_attribute("style", "width: 200px; height: 40px");
        ime.set_attribute("tabindex", "0");
        crate::focus_registry::register_focus_target(
            &ime,
            crate::focus_registry::FocusEntry::new().on_ime(|_| {}),
        );
        root.append_child(&input);
        root.append_child(&textarea);
        root.append_child(&checkbox);
        root.append_child(&other);
        root.append_child(&ime);
        ids_in.set(Some(Ids {
            input: input.node_id().0,
            textarea: textarea.node_id().0,
            checkbox: checkbox.node_id().0,
            other: other.node_id().0,
            ime: ime.node_id().0,
        }));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    (app, ids.get().expect("ids captured at mount"))
}

fn abs_box(app: &RinchApp, id: usize) -> (f32, f32, f32, f32) {
    let d = app.doc.as_ref().unwrap().borrow();
    painted_element_box(&d.tree, id)
}

/// A left press at the right edge of `id`: a field's caret lands at its end.
fn press(app: &mut RinchApp, id: usize) {
    let (x, y, w, h) = abs_box(app, id);
    let (x, y) = (x + w - 2.0, y + h / 2.0);
    for event in [
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
        PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Left,
        },
    ] {
        app.handle_event(event, VP, 1.0);
    }
}

fn value(app: &RinchApp, id: usize) -> String {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree
        .get(id)
        .and_then(|n| n.attributes.get("value").cloned())
        .unwrap_or_default()
}

fn bare() -> Modifiers {
    Modifiers::default()
}

/// The key the shell hands the app for winit's `key` once no chord ran
/// (`RinchRuntime::translate_key`): punctuation has no variant of its own.
fn platform_key(key: W) -> KeyCode {
    match key {
        W::Slash => KeyCode::Other,
        W::KeyN => KeyCode::KeyN,
        W::Backspace => KeyCode::Backspace,
        W::ArrowLeft => KeyCode::ArrowLeft,
        W::F5 => KeyCode::F5,
        W::Escape => KeyCode::Escape,
        other => panic!("no mapping in this fixture for {other:?}"),
    }
}

/// What the shell does with one press: the chord first, and the key to the app
/// only if no chord ran. Answers whether the chord ran.
fn shell_key(app: &mut RinchApp, mods: Modifiers, key: W, text: Option<&str>) -> bool {
    if app.try_menu_shortcut(mods, key) {
        return true;
    }
    let key = platform_key(key);
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: text.map(str::to_string),
            text: text.map(str::to_string),
            modifiers: mods,
            repeat: KeyRepeat::Fresh,
        },
        VP,
        1.0,
    );
    app.handle_event(
        PlatformEvent::KeyUp {
            key,
            logical_key: text.map(str::to_string),
            modifiers: mods,
        },
        VP,
        1.0,
    );
    false
}

/// Arm one menu holding `chords`, each counting into its own cell.
fn arm(chords: &[&'static str]) -> (crate::menu::MenuBarChords, Vec<Rc<Cell<u32>>>) {
    let mut menu = crate::menu::Menu::new();
    let mut counters = Vec::new();
    for chord in chords {
        let fired = Rc::new(Cell::new(0u32));
        let c = fired.clone();
        menu = menu.item(
            crate::menu::MenuItem::new(*chord)
                .shortcut(*chord)
                .on_click(move || c.set(c.get() + 1)),
        );
        counters.push(fired);
    }
    (
        crate::menu::register_menu_shortcuts(&[("File", &menu)]),
        counters,
    )
}

#[test]
fn a_bare_slash_types_into_a_focused_input_and_fires_once_focus_is_elsewhere() {
    let (mut app, ids) = page();
    let (_chords, fired) = arm(&["/"]);

    press(&mut app, ids.input);
    assert_eq!(app.focus_target, FocusTarget::Input(ids.input));
    assert!(!shell_key(&mut app, bare(), W::Slash, Some("/")));
    assert_eq!(value(&app, ids.input), "ab/", "the field took the `/`");
    assert_eq!(fired[0].get(), 0, "the item did not fire under the field");

    press(&mut app, ids.other);
    assert_eq!(app.focus_target, FocusTarget::Node(ids.other));
    assert!(
        shell_key(&mut app, bare(), W::Slash, Some("/")),
        "with focus off the field the chord runs and swallows the key"
    );
    assert_eq!(fired[0].get(), 1);
    assert_eq!(value(&app, ids.input), "ab/", "nothing reached the field");
}

#[test]
fn a_textarea_keeps_bare_and_shifted_keys_and_its_editing_keys() {
    let (mut app, ids) = page();
    let (_chords, fired) = arm(&["N", "Shift+/", "Backspace", "ArrowLeft"]);

    press(&mut app, ids.textarea);
    assert_eq!(app.focus_target, FocusTarget::Input(ids.textarea));
    assert!(!shell_key(&mut app, bare(), W::KeyN, Some("n")));
    let shift = Modifiers {
        shift: true,
        ..Default::default()
    };
    assert!(!shell_key(&mut app, shift, W::Slash, Some("?")));
    assert_eq!(value(&app, ids.textarea), "abn?");
    assert!(!shell_key(&mut app, bare(), W::Backspace, None));
    assert!(!shell_key(&mut app, bare(), W::ArrowLeft, None));
    assert_eq!(
        value(&app, ids.textarea),
        "abn",
        "Backspace edited the field"
    );
    assert_eq!(
        fired.iter().map(|c| c.get()).collect::<Vec<_>>(),
        vec![0, 0, 0, 0]
    );
}

#[test]
fn a_chord_with_ctrl_or_alt_or_a_non_text_key_still_fires_in_a_field() {
    let (mut app, ids) = page();
    let (_chords, fired) = arm(&["Ctrl+/", "Alt+/", "F5", "Escape"]);

    press(&mut app, ids.input);
    assert_eq!(app.focus_target, FocusTarget::Input(ids.input));
    let ctrl = Modifiers {
        ctrl: true,
        ..Default::default()
    };
    let alt = Modifiers {
        alt: true,
        ..Default::default()
    };
    assert!(shell_key(&mut app, ctrl, W::Slash, None));
    assert!(shell_key(&mut app, alt, W::Slash, None));
    assert!(shell_key(&mut app, bare(), W::F5, None));
    assert!(shell_key(&mut app, bare(), W::Escape, None));
    assert_eq!(
        fired.iter().map(|c| c.get()).collect::<Vec<_>>(),
        vec![1, 1, 1, 1]
    );
    assert_eq!(value(&app, ids.input), "ab");
}

#[test]
fn a_focused_checkbox_is_not_a_text_target() {
    let (mut app, ids) = page();
    let (_chords, fired) = arm(&["/"]);
    // A checkbox carrying `data-oninput` takes `FocusTarget::Input`, but it
    // has no text to type `/` into.
    app.set_focus_target(FocusTarget::Input(ids.checkbox));
    assert!(shell_key(&mut app, bare(), W::Slash, Some("/")));
    assert_eq!(fired[0].get(), 1);
}

#[test]
fn an_editor_and_a_registered_text_target_keep_a_bare_key() {
    let (mut app, ids) = page();
    let (_chords, fired) = arm(&["/"]);

    app.set_focus_target(FocusTarget::Editor(ids.other));
    assert!(!app.try_menu_shortcut(bare(), W::Slash));

    // A generic focusable node is not a text target…
    app.set_focus_target(FocusTarget::Node(ids.other));
    assert!(app.try_menu_shortcut(bare(), W::Slash));
    assert_eq!(fired[0].get(), 1);

    // …but one registered for composition is (#176).
    app.set_focus_target(FocusTarget::Node(ids.ime));
    assert!(!app.try_menu_shortcut(bare(), W::Slash));
    assert_eq!(fired[0].get(), 1);
}

/// A render surface and an open `<select>` popup are not text targets: a bare
/// chord still fires while either holds the keyboard. (Whether they should
/// keep keys of their own is a separate question from #1169's.)
#[test]
fn a_surface_or_an_open_select_does_not_take_a_bare_key() {
    let (mut app, ids) = page();
    let (_chords, fired) = arm(&["/"]);
    app.focus_target = FocusTarget::Surface(1);
    assert!(app.try_menu_shortcut(bare(), W::Slash));
    app.focus_target = FocusTarget::Select(ids.other);
    assert!(app.try_menu_shortcut(bare(), W::Slash));
    app.focus_target = FocusTarget::None;
    assert!(app.try_menu_shortcut(bare(), W::Slash));
    assert_eq!(fired[0].get(), 3);
}
