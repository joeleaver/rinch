//! Line breaks inserted into a single-line field (#824) by the paths that
//! need no clipboard: an IME commit, a key's text (Android's soft keyboard
//! commits one character per key), and the fold Android applies to a whole
//! committed text first. The paste half is in `input_paste_async_tests`.
//!
//! Chrome 155 folds text inserted into an `<input>`: an inner `\r\n`, `\r`
//! or `\n` is one space, trailing ones are dropped
//! (`rinch_editable::single_line_insert`). A `<textarea>` keeps them.

use super::*;
use rinch_core::events::{InputCallback, register_input_handler};
use std::cell::Cell;

const VP: (u32, u32) = (800, 600);

const FIELD_STYLE: &str = "width: 300px; height: 30px; padding: 0; margin: 0; \
     font-size: 16px; line-height: 20px; font-family: sans-serif";

type Log = Rc<RefCell<Vec<String>>>;

#[derive(Clone, Copy)]
struct Ids {
    input: u64,
    textarea: u64,
}

fn ev(app: &mut RinchApp, event: PlatformEvent) -> Vec<AppAction> {
    app.handle_event(event, VP, 1.0)
}

fn key_with(app: &mut RinchApp, key: KeyCode, modifiers: Modifiers) {
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
    key_with(app, key, Modifiers::default());
}

fn primary() -> Modifiers {
    Modifiers {
        ctrl: !cfg!(target_os = "macos"),
        meta: cfg!(target_os = "macos"),
        ..Default::default()
    }
}

fn attr(app: &RinchApp, id: u64, name: &str) -> Option<String> {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree.get(id).and_then(|n| n.attributes.get(name).cloned())
}

/// `(value, selection start, cursor)` as the DOM carries them.
fn field(app: &RinchApp, id: u64) -> (String, String, String) {
    (
        attr(app, id, "value").unwrap_or_default(),
        attr(app, id, "data-selection-start").unwrap_or_default(),
        attr(app, id, "data-cursor-pos").unwrap_or_default(),
    )
}

fn state(value: &str, anchor: usize, head: usize) -> (String, String, String) {
    (value.into(), anchor.to_string(), head.to_string())
}

/// Focus `id` with a press at its left edge.
fn focus(app: &mut RinchApp, id: u64) {
    let (bx, by, _, bh) = {
        let d = app.doc.as_ref().unwrap().borrow();
        painted_element_box(&d.tree, id)
    };
    let (x, y, button) = (bx + 1.0, by + bh / 2.0, MouseButton::Left);
    ev(app, PlatformEvent::MouseDown { x, y, button });
    ev(app, PlatformEvent::MouseUp { x, y, button });
    assert_eq!(app.focus_target, FocusTarget::Input(id));
}

/// An `<input>` and a `<textarea>`, both holding `hello world`, each logging
/// its `oninput`.
fn page() -> (RinchApp, Ids, Log) {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let record = |tag: &'static str| {
        let log = log.clone();
        register_input_handler(InputCallback::new(move |v: String| {
            log.borrow_mut().push(format!("{tag}:{v}"));
        }))
    };
    let input_h = record("input");
    let textarea_h = record("textarea");
    let ids: Rc<Cell<Option<Ids>>> = Rc::new(Cell::new(None));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let input = scope.create_element("input");
        input.set_attribute("style", FIELD_STYLE);
        input.set_attribute("value", "hello world");
        input.set_attribute("data-oninput", &input_h.0.to_string());
        let textarea = scope.create_element("textarea");
        textarea.set_attribute("style", FIELD_STYLE);
        textarea.set_attribute("value", "hello world");
        textarea.set_attribute("data-oninput", &textarea_h.0.to_string());
        root.append_child(&input);
        root.append_child(&textarea);
        ids_in.set(Some(Ids {
            input: input.node_id().0,
            textarea: textarea.node_id().0,
        }));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    (app, ids.get().expect("ids captured at mount"), log)
}

/// The IME commit path reaches the same insertion: a committed `\n` in an
/// `<input>` is a space, in a `<textarea>` a line break.
#[test]
fn an_ime_commit_with_a_line_break_is_one_line_in_an_input() {
    let (mut app, ids, _log) = page();
    focus(&mut app, ids.input);
    key(&mut app, KeyCode::End);
    ev(
        &mut app,
        PlatformEvent::Ime(ImeEvent::Commit("x\ny".into())),
    );
    assert_eq!(field(&app, ids.input).0, "hello worldx y");

    focus(&mut app, ids.textarea);
    key(&mut app, KeyCode::End);
    ev(
        &mut app,
        PlatformEvent::Ime(ImeEvent::Commit("x\ny".into())),
    );
    assert_eq!(field(&app, ids.textarea).0, "hello worldx\ny");
}

/// A lone `\n` as a key's text — what Android's soft keyboard commits one
/// character at a time (`apply_ime_action`) — inserts nothing into an
/// `<input>` and leaves no undo step behind: Ctrl+Z after it undoes the
/// keystroke before it.
#[test]
fn a_lone_newline_typed_into_an_input_inserts_nothing() {
    let (mut app, ids, log) = page();
    focus(&mut app, ids.input);
    key(&mut app, KeyCode::End);
    let typed = |app: &mut RinchApp, t: &str| {
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
    };
    typed(&mut app, "!");
    log.borrow_mut().clear();
    typed(&mut app, "\n");
    assert_eq!(field(&app, ids.input), state("hello world!", 12, 12));
    assert!(log.borrow().is_empty(), "no oninput: nothing changed");
    key_with(&mut app, KeyCode::KeyZ, primary());
    assert_eq!(field(&app, ids.input).0, "hello world");
}

/// What Android's soft keyboard path folds a whole committed text with,
/// before it splits it into one key per character (`apply_ime_action`, which
/// no host test compiles): line breaks in an `<input>`, untouched in a
/// `<textarea>`.
#[test]
fn a_committed_text_is_folded_for_the_focused_field() {
    let (mut app, ids, _log) = page();
    focus(&mut app, ids.input);
    assert_eq!(app.insertable_text("a\r\nb\n".into()), "a b");
    focus(&mut app, ids.textarea);
    assert_eq!(app.insertable_text("a\r\nb\n".into()), "a\r\nb\n");
}
