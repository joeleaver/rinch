//! Which keys a menu chord may take from the focused control (issues #934,
//! #1291, #1290), driven the way the desktop shell drives a key press: the
//! chord first (`RinchApp::try_menu_shortcut`), then — when no chord ran — a
//! `PlatformEvent::KeyDown` whose text the shell filtered with
//! `rinch_platform::alt_graph_filtered_text`.
//!
//! - **AltGr** (#934, #1291): Windows reports AltGr as Ctrl+Alt. Polish
//!   AltGr+C types `ć`, German AltGr+Q `@`. Such a press is typing: a
//!   `Ctrl+Alt+Q` item leaves it to a focused field, and the field inserts it
//!   rather than reading Ctrl+C / Ctrl+A. A real Ctrl+Alt chord (winit's
//!   `text` is then the bare key's own character) is still a chord.
//! - **Controls** (#1290): a focused closed `<select>` keeps its arrows,
//!   letters, Space and Enter; a button or `tabindex` node keeps Space and
//!   Enter; a `RenderSurface` or an open select popup keeps every bare key.
//!   `ChordYield::TextOnly` is the escape hatch back to "text fields only".
//!
//! Off the fixed points: the field holds `ab` with the caret at its end, so an
//! insertion shows as `ab?`, and every case that yields is sampled beside one
//! that fires.

use super::*;
use crate::menu::{ChordYield, set_chord_yield};
use rinch_core::events::{InputCallback, register_input_handler};
use std::cell::Cell;
use winit::keyboard::KeyCode as W;

const VP: (u32, u32) = (800, 600);

#[derive(Clone, Copy)]
struct Ids {
    input: u64,
    select: u64,
    button: u64,
}

fn page() -> (RinchApp, Ids) {
    let input_h = register_input_handler(InputCallback::new(|_v: String| {}));
    let ids: Rc<Cell<Option<Ids>>> = Rc::new(Cell::new(None));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let input = scope.create_element("input");
        input.set_attribute(
            "style",
            "width: 300px; height: 30px; padding: 0; margin: 0; font-size: 16px; \
             line-height: 20px; font-family: sans-serif",
        );
        input.set_attribute("value", "ab");
        input.set_attribute("data-oninput", &input_h.0.to_string());
        let select = scope.create_element("select");
        for label in ["one", "two"] {
            let option = scope.create_element("option");
            option.append_child(&scope.create_text(label));
            select.append_child(&option);
        }
        let button = scope.create_element("button");
        button.append_child(&scope.create_text("Go"));
        root.append_child(&input);
        root.append_child(&select);
        root.append_child(&button);
        ids_in.set(Some(Ids {
            input: input.node_id().0,
            select: select.node_id().0,
            button: button.node_id().0,
        }));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    (app, ids.get().expect("ids captured at mount"))
}

/// A left press at the right edge of the input: its caret lands at the end.
fn focus_input(app: &mut RinchApp, id: u64) {
    let (x, y, w, h) = {
        let d = app.doc.as_ref().unwrap().borrow();
        painted_element_box(&d.tree, id)
    };
    let (x, y) = (x + w - 2.0, y + h / 2.0);
    let button = MouseButton::Left;
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
    assert_eq!(app.focus_target, FocusTarget::Input(id));
}

fn value(app: &RinchApp, id: u64) -> String {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree
        .get(id)
        .and_then(|n| n.attributes.get("value").cloned())
        .unwrap_or_default()
}

fn bare() -> Modifiers {
    Modifiers::default()
}

fn ctrl_alt() -> Modifiers {
    Modifiers {
        ctrl: true,
        alt: true,
        ..Modifiers::default()
    }
}

fn ctrl() -> Modifiers {
    Modifiers {
        ctrl: true,
        ..Modifiers::default()
    }
}

fn platform_key(key: W) -> KeyCode {
    match key {
        W::KeyA => KeyCode::KeyA,
        W::KeyC => KeyCode::KeyC,
        W::KeyN => KeyCode::KeyN,
        W::KeyQ => KeyCode::KeyQ,
        W::KeyW => KeyCode::KeyW,
        W::Space => KeyCode::Space,
        W::Enter => KeyCode::Enter,
        W::ArrowDown => KeyCode::ArrowDown,
        W::Escape => KeyCode::Escape,
        W::F5 => KeyCode::F5,
        other => panic!("no mapping in this fixture for {other:?}"),
    }
}

/// What the shell does with one press of `key`, whose layout types `base`
/// unmodified and `text` with these modifiers: the chord first, and the key to
/// the app — with the shell's AltGr filtering — only if no chord ran.
/// Answers whether a chord ran.
fn shell_key(
    app: &mut RinchApp,
    mods: Modifiers,
    key: W,
    base: Option<&str>,
    text: Option<&str>,
) -> bool {
    let kwm = match base {
        Some(b) => winit::keyboard::Key::Character(b.into()),
        None => winit::keyboard::Key::Unidentified(winit::keyboard::NativeKey::Unidentified),
    };
    if app.try_menu_shortcut(mods, key, &kwm, text) {
        return true;
    }
    let text = rinch_platform::alt_graph_filtered_text(mods, text, base);
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

fn counts(fired: &[Rc<Cell<u32>>]) -> Vec<u32> {
    fired.iter().map(|c| c.get()).collect()
}

/// #934: Polish AltGr+C on Windows — Ctrl+Alt+C carrying `ć` — types `ć` into
/// a focused field; it is neither Ctrl+C nor dropped. AltGr+A `ą` likewise is
/// not select-all: the second insertion lands after the first.
#[test]
fn altgr_letters_type_into_a_focused_input() {
    let (mut app, ids) = page();
    focus_input(&mut app, ids.input);
    assert!(!shell_key(
        &mut app,
        ctrl_alt(),
        W::KeyC,
        Some("c"),
        Some("ć")
    ));
    assert_eq!(value(&app, ids.input), "abć");
    assert!(!shell_key(
        &mut app,
        ctrl_alt(),
        W::KeyA,
        Some("a"),
        Some("ą")
    ));
    assert_eq!(value(&app, ids.input), "abćą", "AltGr+A did not select all");
}

/// A real Ctrl+Alt+C (Linux: winit's `text` is the bare `c`) types nothing,
/// and plain Ctrl+A still selects all — the next character replaces it.
#[test]
fn a_real_ctrl_alt_chord_types_nothing_and_ctrl_a_still_selects_all() {
    let (mut app, ids) = page();
    focus_input(&mut app, ids.input);
    assert!(!shell_key(
        &mut app,
        ctrl_alt(),
        W::KeyC,
        Some("c"),
        Some("c")
    ));
    assert_eq!(value(&app, ids.input), "ab");
    assert!(!shell_key(&mut app, ctrl(), W::KeyA, Some("a"), Some("a")));
    assert!(!shell_key(&mut app, bare(), W::KeyN, Some("n"), Some("n")));
    assert_eq!(value(&app, ids.input), "n", "Ctrl+A selected all");
}

/// #1291: a `Ctrl+Alt+Q` item leaves the `@` German AltGr+Q types to a
/// focused field, fires on a real Ctrl+Alt+Q there, and fires on AltGr+Q from
/// a control that types nothing.
#[test]
fn a_ctrl_alt_item_leaves_altgr_text_to_a_field() {
    let (mut app, ids) = page();
    let (_chords, fired) = arm(&["Ctrl+Alt+Q"]);
    focus_input(&mut app, ids.input);
    assert!(!shell_key(
        &mut app,
        ctrl_alt(),
        W::KeyQ,
        Some("q"),
        Some("@")
    ));
    assert_eq!(value(&app, ids.input), "ab@");
    assert_eq!(counts(&fired), vec![0]);

    assert!(shell_key(
        &mut app,
        ctrl_alt(),
        W::KeyQ,
        Some("q"),
        Some("q")
    ));
    assert_eq!(counts(&fired), vec![1]);

    app.set_focus_target(FocusTarget::Node(ids.button));
    assert!(shell_key(
        &mut app,
        ctrl_alt(),
        W::KeyQ,
        Some("q"),
        Some("@")
    ));
    assert_eq!(counts(&fired), vec![2]);
}

/// #1290: a focused closed `<select>` keeps the keys it moves and opens by;
/// a bare F5 still fires there, and so does any chord with Ctrl.
#[test]
fn a_focused_select_keeps_its_arrows_letters_space_and_enter() {
    let (mut app, ids) = page();
    let (_chords, fired) = arm(&["ArrowDown", "N", "Space", "Enter", "F5", "Ctrl+N"]);
    app.set_focus_target(FocusTarget::Node(ids.select));
    for key in [W::ArrowDown, W::KeyN, W::Space, W::Enter] {
        assert!(
            !app.try_menu_shortcut(
                bare(),
                key,
                &winit::keyboard::Key::Character("n".into()),
                None
            ),
            "{key:?} is the select's"
        );
    }
    assert!(app.try_menu_shortcut(bare(), W::F5, &no_char(), None));
    assert!(app.try_menu_shortcut(
        ctrl(),
        W::KeyN,
        &winit::keyboard::Key::Character("n".into()),
        None
    ));
    assert_eq!(counts(&fired), vec![0, 0, 0, 0, 1, 1]);
}

fn no_char() -> winit::keyboard::Key {
    winit::keyboard::Key::Unidentified(winit::keyboard::NativeKey::Unidentified)
}

/// #1290: a focused button keeps Space and Enter — they activate it — and
/// leaves a bare letter or arrow to the menu.
#[test]
fn a_focused_button_keeps_space_and_enter() {
    let (mut app, ids) = page();
    let (_chords, fired) = arm(&["Space", "Enter", "N", "ArrowDown"]);
    app.set_focus_target(FocusTarget::Node(ids.button));
    assert!(!app.try_menu_shortcut(bare(), W::Space, &no_char(), Some(" ")));
    assert!(!app.try_menu_shortcut(bare(), W::Enter, &no_char(), None));
    let n = winit::keyboard::Key::Character("n".into());
    assert!(app.try_menu_shortcut(bare(), W::KeyN, &n, Some("n")));
    assert!(app.try_menu_shortcut(bare(), W::ArrowDown, &no_char(), None));
    assert_eq!(counts(&fired), vec![0, 0, 1, 1]);
}

/// #1290: a focused surface keeps every bare key — a game's W and Escape —
/// and a chord with Ctrl still fires from it. An open select popup likewise.
#[test]
fn a_focused_surface_or_open_popup_keeps_every_bare_key() {
    let (mut app, ids) = page();
    let (_chords, fired) = arm(&["W", "Escape", "F5", "Ctrl+W"]);
    let w = winit::keyboard::Key::Character("w".into());
    for target in [FocusTarget::Surface(1), FocusTarget::Select(ids.select)] {
        app.focus_target = target;
        assert!(!app.try_menu_shortcut(bare(), W::KeyW, &w, Some("w")));
        assert!(!app.try_menu_shortcut(bare(), W::Escape, &no_char(), None));
        assert!(!app.try_menu_shortcut(bare(), W::F5, &no_char(), None));
        assert!(app.try_menu_shortcut(ctrl(), W::KeyW, &w, Some("w")));
    }
    assert_eq!(counts(&fired), vec![0, 0, 0, 2]);
}

/// `ChordYield::TextOnly`, the escape hatch: a bare chord fires from a
/// select, a button and a surface again — and a text field still keeps it.
#[test]
fn text_only_mode_lets_a_bare_chord_fire_from_controls() {
    let (mut app, ids) = page();
    let (_chords, fired) = arm(&["N", "Space"]);
    set_chord_yield(ChordYield::TextOnly);
    let n = winit::keyboard::Key::Character("n".into());
    app.set_focus_target(FocusTarget::Node(ids.select));
    assert!(app.try_menu_shortcut(bare(), W::KeyN, &n, Some("n")));
    app.set_focus_target(FocusTarget::Node(ids.button));
    assert!(app.try_menu_shortcut(bare(), W::Space, &no_char(), Some(" ")));
    app.focus_target = FocusTarget::Surface(1);
    assert!(app.try_menu_shortcut(bare(), W::KeyN, &n, Some("n")));
    focus_input(&mut app, ids.input);
    assert!(!shell_key(&mut app, bare(), W::KeyN, Some("n"), Some("n")));
    set_chord_yield(ChordYield::FocusedControls);
    assert_eq!(counts(&fired), vec![2, 1]);
    assert_eq!(value(&app, ids.input), "abn");
}

/// The editor inserts AltGr text too (#934): Ctrl+Alt+C carrying `ć` is
/// typing there, ahead of the clipboard chord and the keymap.
#[test]
fn altgr_letters_type_into_a_focused_editor() {
    let slot: Rc<RefCell<Option<crate::editor::EditorHandle>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let (container, handle) = crate::editor::mount_editor(scope);
        handle.load_html("<p>ab</p>");
        container.set_attribute(
            "style",
            "width: 400px; font-size: 16px; line-height: 24px; font-family: sans-serif",
        );
        root.append_child(&container);
        *slot_in.borrow_mut() = Some(handle);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let handle = slot.borrow_mut().take().expect("captured at mount");
    let (x, y, h) = app
        .editor_caret_point(&handle, rinch_editor_core::Pos(3))
        .expect("a caret");
    let button = MouseButton::Left;
    let (x, y) = (x + 1.0, y + h / 2.0);
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
    assert!(matches!(app.focus_target, FocusTarget::Editor(_)));

    assert!(!shell_key(
        &mut app,
        ctrl_alt(),
        W::KeyC,
        Some("c"),
        Some("ć")
    ));
    let doc = handle.doc();
    let para = doc.child(0);
    let text: String = (0..para.child_count())
        .filter_map(|i| para.child(i).text().map(str::to_string))
        .collect();
    assert_eq!(text, "abć", "AltGr+C typed into the editor");
}
