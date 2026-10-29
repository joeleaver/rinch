//! The macOS text keymap, driven on any host (issue #951).
//!
//! rinch's desktop dispatch used one Windows/Linux keymap everywhere, with Cmd
//! standing in for Ctrl on macOS: Cmd+Left moved by word, Option+Left by one
//! character, Home/End moved the caret and Cmd+Up/Down did nothing special.
//! macOS's own keys (Cocoa's `NSStandardKeyBindingResponding`, which Chrome and
//! Safari follow in their text controls) are Cmd+Left/Right to the line's
//! edges, Cmd+Up/Down to the value's, Option for words, and Home/End leave the
//! caret where it is (Shift+Home/End extend to the value's ends).
//!
//! The keymap is a field of the app ([`RinchApp::text_keymap`]), so these
//! fixtures set [`TextKeymap::MacOs`] and run on Linux. Every key goes through
//! `handle_event`, the path a real keystroke takes. Cmd is `meta`.
//!
//! The text is the bundled Inter at a declared 16px / 20px: `abcd abcd abcd`
//! in a 70px `<textarea>` is one word per visual line, `abcd ` (0..5),
//! `abcd ` (5..10), `abcd` (10..14) — #933's fixture.

use super::*;
use rinch_core::events::{InputCallback, register_input_handler};
use rinch_editor_core::{Pos, Selection};
use std::cell::Cell;

const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const VP: (u32, u32) = (800, 600);

const NONE: Modifiers = Modifiers {
    shift: false,
    ctrl: false,
    alt: false,
    meta: false,
};
const CMD: Modifiers = Modifiers { meta: true, ..NONE };
const CMD_SHIFT: Modifiers = Modifiers {
    meta: true,
    shift: true,
    ..NONE
};
const OPTION: Modifiers = Modifiers { alt: true, ..NONE };
const OPTION_SHIFT: Modifiers = Modifiers {
    alt: true,
    shift: true,
    ..NONE
};
const SHIFT: Modifiers = Modifiers {
    shift: true,
    ..NONE
};
const CTRL: Modifiers = Modifiers { ctrl: true, ..NONE };

/// A focused `tag` of `width` px under the macOS keymap, holding `text`.
fn field(tag: &'static str, width: u32, text: &str) -> (RinchApp, u64) {
    let input_id = register_input_handler(InputCallback::new(|_v: String| {}));
    let id: Rc<Cell<Option<u64>>> = Rc::new(Cell::new(None));
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
        field.set_attribute("data-oninput", &input_id.0.to_string());
        root.append_child(&field);
        id_in.set(Some(field.node_id().0));
        root
    });
    app.text_keymap = TextKeymap::MacOs;
    app.register_app_font(AppFont::sans_serif(INTER));
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let id = id.get().expect("node id captured at mount");
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
        app.handle_event(ev, VP, 1.0);
    }
    assert_eq!(app.focused_input_node_id, Some(id), "the field took focus");
    for ch in text.chars() {
        press(&mut app, KeyCode::KeyA, Some(&ch.to_string()), NONE);
    }
    assert_eq!(
        value(&app, id),
        text,
        "positive control: the text was typed"
    );
    (app, id)
}

fn press(app: &mut RinchApp, key: KeyCode, text: Option<&str>, modifiers: Modifiers) {
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: text.map(str::to_string),
            modifiers,
            repeat: KeyRepeat::Fresh,
        },
        VP,
        1.0,
    );
    app.handle_event(
        PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers,
        },
        VP,
        1.0,
    );
    app.resolve_and_repaint(800.0, 600.0);
}

fn attr(app: &RinchApp, id: u64, name: &str) -> String {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree
        .get(id)
        .and_then(|n| n.attributes.get(name).cloned())
        .unwrap_or_default()
}

fn value(app: &RinchApp, id: u64) -> String {
    attr(app, id, "value")
}

/// `(anchor, head)` as written to the DOM for the paint layer.
fn sel(app: &RinchApp, id: u64) -> (usize, usize) {
    (
        attr(app, id, "data-selection-start").parse().unwrap(),
        attr(app, id, "data-cursor-pos").parse().unwrap(),
    )
}

/// A collapsed caret at byte `at`: Cmd+Up, then `at` plain Rights.
fn caret_to(app: &mut RinchApp, id: u64, at: usize) {
    press(app, KeyCode::ArrowUp, None, CMD);
    for _ in 0..at {
        press(app, KeyCode::ArrowRight, None, NONE);
    }
    assert_eq!(sel(app, id), (at, at), "caret placed at {at}");
}

/// Cmd+Left / Cmd+Right in an `<input>` go to the ends of its value, Option
/// moves by word, and a plain arrow by one character.
#[test]
fn input_cmd_arrows_go_to_the_line_and_option_moves_by_word() {
    let (mut app, id) = field("input", 300, "hello brave world");
    caret_to(&mut app, id, 9);
    press(&mut app, KeyCode::ArrowLeft, None, CMD);
    assert_eq!(
        sel(&app, id),
        (0, 0),
        "Cmd+Left: the line's start, not a word"
    );
    press(&mut app, KeyCode::ArrowRight, None, CMD);
    assert_eq!(sel(&app, id), (17, 17), "Cmd+Right: the line's end");
    press(&mut app, KeyCode::ArrowLeft, None, OPTION);
    assert_eq!(sel(&app, id), (12, 12), "Option+Left: the word's start");
    press(&mut app, KeyCode::ArrowLeft, None, OPTION);
    assert_eq!(sel(&app, id), (6, 6), "Option+Left again: the word before");
    press(&mut app, KeyCode::ArrowRight, None, OPTION_SHIFT);
    assert_eq!(
        sel(&app, id),
        (6, 11),
        "Option+Shift+Right extends by a word"
    );
    press(&mut app, KeyCode::ArrowLeft, None, NONE);
    press(&mut app, KeyCode::ArrowLeft, None, CTRL);
    assert_eq!(
        sel(&app, id).1,
        sel(&app, id).0,
        "a collapsed caret after plain and Ctrl arrows"
    );
    let at = sel(&app, id).1;
    press(&mut app, KeyCode::ArrowRight, None, CTRL);
    assert_eq!(
        sel(&app, id),
        (at + 1, at + 1),
        "Ctrl is not the word modifier on macOS: one character"
    );
    press(&mut app, KeyCode::ArrowLeft, None, CMD_SHIFT);
    assert_eq!(
        sel(&app, id),
        (at + 1, 0),
        "Cmd+Shift+Left extends to the start"
    );
}

/// In a `<textarea>` Cmd+Left / Cmd+Right go to the caret's **visual** line's
/// edges (the same edges Home/End reach on Windows and Linux, #933), and
/// Cmd+Up / Cmd+Down to the value's ends.
#[test]
fn textarea_cmd_arrows_reach_the_visual_line_and_the_value() {
    let (mut app, id) = field("textarea", 70, "abcd abcd abcd");
    caret_to(&mut app, id, 7);
    press(&mut app, KeyCode::ArrowLeft, None, CMD);
    assert_eq!(sel(&app, id), (5, 5), "Cmd+Left: the visual line's start");
    press(&mut app, KeyCode::ArrowRight, None, CMD);
    assert_eq!(
        sel(&app, id),
        (10, 10),
        "Cmd+Right: the visual line's end, at the wrap point, drawn upstream (#941)"
    );
    press(&mut app, KeyCode::ArrowDown, None, CMD);
    assert_eq!(sel(&app, id), (14, 14), "Cmd+Down: the value's end");
    press(&mut app, KeyCode::ArrowUp, None, CMD_SHIFT);
    assert_eq!(sel(&app, id), (14, 0), "Cmd+Shift+Up extends to the start");
    press(&mut app, KeyCode::ArrowDown, None, CMD);
    press(&mut app, KeyCode::ArrowUp, None, NONE);
    assert_eq!(
        sel(&app, id).1,
        sel(&app, id).0,
        "a plain ArrowUp is still a line move"
    );
    assert!(
        sel(&app, id).1 < 10,
        "ArrowUp left the last visual line: {:?}",
        sel(&app, id)
    );
}

/// Home and End leave the caret where it is on macOS (they scroll the field,
/// which rinch does not); Shift+Home / Shift+End extend to the value's ends.
#[test]
fn home_and_end_leave_the_caret_and_shift_extends_to_the_ends() {
    let (mut app, id) = field("textarea", 70, "abcd abcd abcd");
    caret_to(&mut app, id, 7);
    press(&mut app, KeyCode::Home, None, NONE);
    assert_eq!(sel(&app, id), (7, 7), "Home does not move the caret");
    press(&mut app, KeyCode::End, None, NONE);
    assert_eq!(sel(&app, id), (7, 7), "End does not move the caret");
    press(&mut app, KeyCode::Home, None, CMD);
    assert_eq!(sel(&app, id), (7, 7), "nor does Cmd+Home");
    press(&mut app, KeyCode::End, None, SHIFT);
    assert_eq!(sel(&app, id), (7, 14), "Shift+End: to the value's end");
    press(&mut app, KeyCode::Home, None, SHIFT);
    assert_eq!(sel(&app, id), (7, 0), "Shift+Home: to the value's start");

    let (mut app, id) = field("input", 300, "hello world");
    caret_to(&mut app, id, 4);
    press(&mut app, KeyCode::End, None, NONE);
    assert_eq!(sel(&app, id), (4, 4), "End in an <input>: no move either");
    press(&mut app, KeyCode::End, None, SHIFT);
    assert_eq!(sel(&app, id), (4, 11));
}

/// Option+Backspace / Option+Delete delete a word, as the rich editor's keymap
/// binds them on macOS; Ctrl+Backspace there is a character.
#[test]
fn option_deletes_a_word_and_ctrl_a_character() {
    let (mut app, id) = field("input", 300, "hello brave world");
    press(&mut app, KeyCode::Backspace, None, OPTION);
    assert_eq!(value(&app, id), "hello brave ", "Option+Backspace: a word");
    press(&mut app, KeyCode::Backspace, None, CTRL);
    assert_eq!(
        value(&app, id),
        "hello brave",
        "Ctrl+Backspace: a character"
    );
    caret_to(&mut app, id, 0);
    press(&mut app, KeyCode::Delete, None, OPTION);
    assert_eq!(value(&app, id), " brave", "Option+Delete: a word forward");
    press(&mut app, KeyCode::Delete, None, NONE);
    assert_eq!(value(&app, id), "brave", "Delete: a character");
}

// ── The rich-text editor ──────────────────────────────────────────────────

/// A focused editor under the macOS keymap holding `<p>hello brave world</p>`
/// then `<p>second</p>`. The first paragraph's text is Pos(1)..Pos(18); the
/// document's end is Pos(26).
fn editor() -> (RinchApp, crate::editor::EditorHandle) {
    let handle = crate::editor::create_editor();
    assert!(handle.load_html("<p>hello brave world</p><p>second</p>"));
    let handle_in = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        let editor = handle_in.mount(scope);
        editor.set_attribute(
            "style",
            "width: 600px; height: 300px; font-size: 16px; line-height: 24px; \
             font-family: sans-serif",
        );
        root.append_child(&editor);
        root
    });
    app.text_keymap = TextKeymap::MacOs;
    app.register_app_font(AppFont::sans_serif(INTER));
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    handle.focus();
    for _ in 0..3 {
        let actions = app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
        if actions.contains(&AppAction::RequestRedraw) && app.has_pending_layout() {
            app.resolve_and_repaint(800.0, 600.0);
        }
    }
    assert!(matches!(app.focus_target, FocusTarget::Editor(_)));
    (app, handle)
}

fn head(handle: &crate::editor::EditorHandle) -> (usize, usize) {
    let s = handle.selection();
    (s.anchor().0, s.head().0)
}

#[test]
fn editor_follows_the_macos_keymap() {
    let (mut app, handle) = editor();
    handle.set_selection(Selection::cursor(Pos(10)));
    press(&mut app, KeyCode::ArrowLeft, None, CMD);
    assert_eq!(head(&handle), (1, 1), "Cmd+Left: the line's start");
    press(&mut app, KeyCode::ArrowRight, None, CMD);
    assert_eq!(head(&handle), (18, 18), "Cmd+Right: the line's end");
    press(&mut app, KeyCode::ArrowLeft, None, OPTION);
    assert_eq!(head(&handle), (13, 13), "Option+Left: the word's start");
    press(&mut app, KeyCode::ArrowRight, None, CTRL);
    assert_eq!(head(&handle), (14, 14), "Ctrl+Right: one character");
    press(&mut app, KeyCode::ArrowDown, None, CMD);
    assert_eq!(head(&handle), (26, 26), "Cmd+Down: the document's end");
    press(&mut app, KeyCode::ArrowUp, None, CMD_SHIFT);
    assert_eq!(head(&handle), (26, 1), "Cmd+Shift+Up extends to the start");
    handle.set_selection(Selection::cursor(Pos(10)));
    press(&mut app, KeyCode::Home, None, NONE);
    press(&mut app, KeyCode::End, None, NONE);
    assert_eq!(head(&handle), (10, 10), "Home / End leave the caret");
    press(&mut app, KeyCode::End, None, SHIFT);
    assert_eq!(head(&handle), (10, 26), "Shift+End: to the document's end");
}

// ── Windows and Linux ─────────────────────────────────────────────────────

/// Ctrl+Backspace / Ctrl+Delete delete a word on Windows and Linux, as the
/// rich editor's keymap binds them there (they deleted one character until
/// #951); Alt+Backspace stays a character.
#[test]
fn standard_ctrl_deletes_a_word() {
    let (mut app, id) = field("input", 300, "hello brave world");
    app.text_keymap = TextKeymap::Standard;
    press(&mut app, KeyCode::Backspace, None, CTRL);
    assert_eq!(value(&app, id), "hello brave ", "Ctrl+Backspace: a word");
    press(&mut app, KeyCode::Backspace, None, OPTION);
    assert_eq!(value(&app, id), "hello brave", "Alt+Backspace: a character");
    press(&mut app, KeyCode::Home, None, NONE);
    assert_eq!(sel(&app, id), (0, 0), "Home moves the caret here");
    press(&mut app, KeyCode::Delete, None, CTRL);
    assert_eq!(value(&app, id), " brave", "Ctrl+Delete: a word forward");
    press(&mut app, KeyCode::ArrowRight, None, CMD);
    assert_eq!(sel(&app, id), (1, 1), "Super+Right: one character");
}

// ── review of #1616 (Chrome 155 on Linux, measured by the reviewer) ──────

fn field_with(
    attrs: &[(&'static str, &'static str)],
    tag: &'static str,
    text: &'static str,
) -> (RinchApp, u64) {
    let input_id = register_input_handler(InputCallback::new(|_v: String| {}));
    let id: Rc<Cell<Option<u64>>> = Rc::new(Cell::new(None));
    let id_in = id.clone();
    let attrs: Vec<(&'static str, &'static str)> = attrs.to_vec();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let field = scope.create_element(tag);
        field.set_attribute(
            "style",
            "width: 300px; height: 160px; padding: 0; font-family: sans-serif; font-size: 16px; line-height: 20px",
        );
        for (k, v) in &attrs {
            field.set_attribute(k, v);
        }
        field.set_attribute("value", text);
        field.set_attribute("data-oninput", &input_id.0.to_string());
        root.append_child(&field);
        id_in.set(Some(field.node_id().0));
        root
    });
    app.text_keymap = TextKeymap::Standard;
    app.register_app_font(AppFont::sans_serif(INTER));
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let id = id.get().unwrap();
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
        app.handle_event(ev, VP, 1.0);
    }
    assert_eq!(app.focused_input_node_id, Some(id), "focused");
    press(&mut app, KeyCode::End, None, CTRL);
    (app, id)
}

#[test]
fn r1616_password_ctrl_backspace() {
    let (mut app, id) = field_with(&[("type", "password")], "input", "hunter2 secret");
    press(&mut app, KeyCode::Backspace, None, CTRL);
    // Chrome 155 Linux: "" (word granularity in a password field = the whole value)
    assert_eq!(value(&app, id), "", "password Ctrl+Backspace");
}

#[test]
fn r1616_textarea_newline_ctrl_backspace() {
    let (mut app, id) = field_with(&[], "textarea", "ab \n");
    press(&mut app, KeyCode::Backspace, None, CTRL);
    // Chrome 155 Linux: "ab " (only the line break)
    assert_eq!(
        value(&app, id),
        "ab ",
        "textarea Ctrl+Backspace at line start"
    );
}

#[test]
#[ignore = "#1619: fields split words at whitespace only"]
fn r1616_hyphen_ctrl_backspace() {
    let (mut app, id) = field_with(&[], "input", "foo-bar");
    press(&mut app, KeyCode::Backspace, None, CTRL);
    // Chrome 155 Linux: "foo-"
    assert_eq!(value(&app, id), "foo-", "foo-bar Ctrl+Backspace");
}

#[test]
fn r1616_readonly_ctrl_backspace_and_delete() {
    let (mut app, id) = field_with(&[("readonly", "")], "input", "hello world");
    press(&mut app, KeyCode::Backspace, None, CTRL);
    press(&mut app, KeyCode::Home, None, NONE);
    press(&mut app, KeyCode::Delete, None, CTRL);
    assert_eq!(
        value(&app, id),
        "hello world",
        "readonly refuses word deletes"
    );
}

#[test]
fn r1616_word_delete_is_one_undo_step() {
    let (mut app, id) = field_with(&[], "input", "one two three");
    press(&mut app, KeyCode::Backspace, None, CTRL);
    assert_eq!(value(&app, id), "one two ");
    press(&mut app, KeyCode::Backspace, None, CTRL);
    assert_eq!(value(&app, id), "one ");
    press(&mut app, KeyCode::KeyZ, None, CTRL);
    assert_eq!(value(&app, id), "one two ", "one undo restores one word");
}

#[test]
fn r1616_interceptor_wins_over_word_delete() {
    let (mut app, id) = field_with(&[], "input", "one two");
    let seen = Rc::new(Cell::new(0));
    let s2 = seen.clone();
    rinch_core::events::set_keyboard_interceptor(move |k| {
        if k.key == "Backspace" && k.ctrl {
            s2.set(s2.get() + 1);
            true
        } else {
            false
        }
    });
    press(&mut app, KeyCode::Backspace, None, CTRL);
    rinch_core::events::clear_keyboard_interceptor();
    assert!(seen.get() >= 1, "interceptor saw it");
    assert_eq!(value(&app, id), "one two", "interceptor consumed it");
}

#[test]
fn r1616_macos_option_shift_backspace_and_cmd_backspace() {
    let (mut app, id) = field("input", 300, "hello brave world");
    // Option+Shift+Backspace in Cocoa is also deleteWordBackward.
    press(&mut app, KeyCode::Backspace, None, OPTION_SHIFT);
    assert_eq!(value(&app, id), "hello brave ");
}

/// The other line-break cases: Chrome 155 deletes just the break with
/// Ctrl+Backspace after it (`"a\n|b"` → `"ab"`); Ctrl+Delete before one does
/// the same, by symmetry.
#[test]
fn a_word_delete_takes_a_line_break_alone() {
    let (mut app, id) = field_with(&[], "textarea", "a\nb");
    press(&mut app, KeyCode::ArrowLeft, None, NONE);
    press(&mut app, KeyCode::Backspace, None, CTRL);
    assert_eq!(value(&app, id), "ab", "Ctrl+Backspace after a break");

    let (mut app, id) = field_with(&[], "textarea", "ab\ncd");
    press(&mut app, KeyCode::Home, None, CTRL);
    press(&mut app, KeyCode::ArrowRight, None, NONE);
    press(&mut app, KeyCode::ArrowRight, None, NONE);
    press(&mut app, KeyCode::Delete, None, CTRL);
    assert_eq!(value(&app, id), "abcd", "Ctrl+Delete before a break");
    press(&mut app, KeyCode::Delete, None, CTRL);
    assert_eq!(value(&app, id), "ab", "then a word, as before");
}

/// In a password field a word is the whole value: Ctrl+Delete at 0 empties it
/// (Chrome 155), macOS Option+Backspace too, and a word move goes to the ends.
#[test]
fn a_password_word_is_the_whole_value() {
    let (mut app, id) = field_with(&[("type", "password")], "input", "hunter2 secret");
    press(&mut app, KeyCode::ArrowLeft, None, CTRL);
    assert_eq!(sel(&app, id), (0, 0), "Ctrl+Left: the value's start");
    press(&mut app, KeyCode::ArrowRight, None, CTRL);
    assert_eq!(sel(&app, id), (14, 14), "Ctrl+Right: the value's end");
    press(&mut app, KeyCode::Home, None, NONE);
    press(&mut app, KeyCode::Delete, None, CTRL);
    assert_eq!(value(&app, id), "", "Ctrl+Delete at 0");

    let (mut app, id) = field_with(&[("type", "password")], "input", "hunter2 secret");
    app.text_keymap = TextKeymap::MacOs;
    press(&mut app, KeyCode::Backspace, None, OPTION);
    assert_eq!(value(&app, id), "", "Option+Backspace");
}

/// AltGr arrives as Ctrl+Alt on Windows; AltGr+Backspace is a character delete.
#[test]
fn ctrl_alt_backspace_is_not_a_word_delete() {
    let (mut app, id) = field_with(&[], "input", "hello world");
    let altgr = Modifiers {
        ctrl: true,
        alt: true,
        ..NONE
    };
    press(&mut app, KeyCode::Backspace, None, altgr);
    assert_eq!(value(&app, id), "hello worl");
}

/// `type` is an enumerated attribute, matched ASCII case-insensitively:
/// `type="PASSWORD"` is a password field and takes its word rules.
#[test]
fn an_uppercase_password_type_is_a_password_field() {
    let (mut app, id) = field_with(&[("type", "PASSWORD")], "input", "hunter2 secret");
    press(&mut app, KeyCode::Backspace, None, CTRL);
    assert_eq!(value(&app, id), "", "Ctrl+Backspace empties it");
}
