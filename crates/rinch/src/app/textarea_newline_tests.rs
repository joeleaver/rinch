//! Enter in a `<textarea>` inserts a line break.
//!
//! The key path, headless: which of Enter's two meanings a focused control
//! takes, and what the insert does to the value and the caret. What the break
//! then *looks* like is the paint layer's (parley breaks on `\n`); what the
//! soft keyboard sends is Android's, and neither is reachable from here.

use super::*;
use rinch_core::events::{InputCallback, register_input_handler};
use std::cell::Cell;

/// A single focusable control of tag `tag`, with `data-oninput` recording
/// every value it is given and `data-onsubmit` present only if `with_submit`.
/// Returns the app, the control's node id, and the shared log.
fn mount(tag: &'static str, with_submit: bool) -> (RinchApp, usize, Rc<RefCell<Vec<String>>>) {
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let input_id = register_input_handler(InputCallback::new({
        let log = log.clone();
        move |v: String| log.borrow_mut().push(format!("input:{v}"))
    }));
    let submit_id = rinch_core::register_handler(std::rc::Rc::new({
        let log = log.clone();
        move || log.borrow_mut().push("submit".to_string())
    }));

    let id: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let id_in = id.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let field = scope.create_element(tag);
        field.set_attribute("style", "width: 200px; height: 60px");
        field.set_attribute("data-oninput", &input_id.0.to_string());
        if with_submit {
            field.set_attribute("data-onsubmit", &submit_id.0.to_string());
        }
        root.append_child(&field);
        id_in.set(Some(field.node_id().0));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let field_id = id.get().expect("node id captured at mount");
    (app, field_id, log)
}

fn key(app: &mut RinchApp, key: KeyCode, text: Option<&str>, shift: bool) {
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: text.map(str::to_string),
            modifiers: Modifiers {
                shift,
                ..Modifiers::default()
            },
        },
        (800, 600),
        1.0,
    );
}

fn type_str(app: &mut RinchApp, text: &str) {
    for ch in text.chars() {
        // The key code is irrelevant for text insertion (the `_` arm inserts
        // the text payload); any non-special key works.
        key(app, KeyCode::KeyA, Some(&ch.to_string()), false);
    }
}

fn click(app: &mut RinchApp, x: f32, y: f32) {
    app.handle_event(
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
        (800, 600),
        1.0,
    );
    app.handle_event(
        PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Left,
        },
        (800, 600),
        1.0,
    );
}

fn focus(app: &mut RinchApp, id: usize) {
    let (cx, cy) = {
        let d = app.doc.as_ref().unwrap().borrow();
        let n = d.tree.get(id).unwrap();
        let (ax, ay) = RinchApp::compute_absolute_position(&d.tree, id);
        (ax + n.layout.width / 2.0, ay + n.layout.height / 2.0)
    };
    click(app, cx, cy);
}

/// The control's live `value` attribute — what it displays, and what the paint
/// layer reads.
fn value(app: &RinchApp, id: usize) -> String {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree
        .get(id)
        .and_then(|n| n.attributes.get("value").cloned())
        .unwrap_or_default()
}

/// The caret's byte offset, as written to the DOM for the paint layer.
fn caret(app: &RinchApp, id: usize) -> usize {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree
        .get(id)
        .and_then(|n| n.attributes.get("data-cursor-pos"))
        .and_then(|s| s.parse().ok())
        .expect("a focused input carries data-cursor-pos")
}

/// The case the whole change exists for: a `<textarea>` with no submit handler
/// takes a line break from Enter. Before this, the key was swallowed whole and
/// a multi-line field could not be typed into on any platform.
#[test]
fn enter_in_a_plain_textarea_inserts_a_break() {
    let (mut app, id, log) = mount("textarea", false);

    focus(&mut app, id);
    type_str(&mut app, "a");
    key(&mut app, KeyCode::Enter, None, false);
    type_str(&mut app, "b");

    assert_eq!(value(&app, id), "a\nb", "the break is in the value");
    assert_eq!(
        *log.borrow(),
        vec![
            "input:a".to_string(),
            "input:a\n".to_string(),
            "input:a\nb".to_string()
        ],
        "the break fires oninput like any other edit — on the web a line break \
         is an input event (inputType: insertLineBreak), not a separate kind"
    );
}

/// The break goes in at the caret and takes the caret with it — it is an edit,
/// not an append. `\n` is one byte, so a two-character value carries a caret of
/// 2 after the break at offset 1.
#[test]
fn the_break_lands_at_the_caret_and_moves_it() {
    let (mut app, id, _log) = mount("textarea", false);

    focus(&mut app, id);
    type_str(&mut app, "ab");
    key(&mut app, KeyCode::ArrowLeft, None, false);
    key(&mut app, KeyCode::Enter, None, false);

    assert_eq!(value(&app, id), "a\nb", "split at the caret, not appended");
    assert_eq!(caret(&app, id), 2, "the caret is after the break it made");
}

/// The break is one character in the document, so one Backspace takes it back
/// out. Pins that `StringDocument` stores `\n` as text and nothing upstream
/// filters it (the premise the paint side relies on).
#[test]
fn a_backspace_takes_the_break_back_out() {
    let (mut app, id, _log) = mount("textarea", false);

    focus(&mut app, id);
    type_str(&mut app, "a");
    key(&mut app, KeyCode::Enter, None, false);
    key(&mut app, KeyCode::Backspace, None, false);

    assert_eq!(value(&app, id), "a");
    assert_eq!(caret(&app, id), 1);
}

/// Two in a row are two breaks — an empty line is typable.
#[test]
fn two_breaks_in_a_row_are_an_empty_line() {
    let (mut app, id, _log) = mount("textarea", false);

    focus(&mut app, id);
    type_str(&mut app, "a");
    key(&mut app, KeyCode::Enter, None, false);
    key(&mut app, KeyCode::Enter, None, false);
    type_str(&mut app, "b");

    assert_eq!(value(&app, id), "a\n\nb");
}

/// `data-onsubmit` on a `<textarea>` is the author declaring that Enter means
/// send — the web backend's keydown delegation does exactly this, ancestors
/// included, so the same tree behaves the same in a browser. Nothing is
/// inserted.
#[test]
fn a_textarea_with_a_submit_handler_submits_on_enter() {
    let (mut app, id, log) = mount("textarea", true);

    focus(&mut app, id);
    type_str(&mut app, "a");
    key(&mut app, KeyCode::Enter, None, false);

    assert_eq!(
        value(&app, id),
        "a",
        "a declared submit does not also insert"
    );
    assert_eq!(
        *log.borrow(),
        vec!["input:a".to_string(), "submit".to_string()]
    );
}

/// …and Shift+Enter is the way out of it. The escape hatch every chat composer
/// has taught, and what the web backend leaves to the browser by excluding
/// Shift from its submit path.
#[test]
fn shift_enter_inserts_even_where_enter_submits() {
    let (mut app, id, log) = mount("textarea", true);

    focus(&mut app, id);
    type_str(&mut app, "a");
    key(&mut app, KeyCode::Enter, None, true);

    assert_eq!(value(&app, id), "a\n", "Shift+Enter always inserts");
    assert!(
        !log.borrow().contains(&"submit".to_string()),
        "and never submits: {:?}",
        log.borrow()
    );
}

/// Shift or not, a `<textarea>` with no submit handler inserts. There is
/// nothing else for the key to do.
#[test]
fn shift_enter_in_a_plain_textarea_inserts_too() {
    let (mut app, id, _log) = mount("textarea", false);

    focus(&mut app, id);
    key(&mut app, KeyCode::Enter, None, true);

    assert_eq!(value(&app, id), "\n");
}

/// An `<input>` never inserts: a line break is not representable in a
/// single-line value. Enter stays a commit and only a commit.
#[test]
fn enter_in_an_input_submits_and_never_inserts() {
    let (mut app, id, log) = mount("input", true);

    focus(&mut app, id);
    type_str(&mut app, "a");
    key(&mut app, KeyCode::Enter, None, false);

    assert_eq!(value(&app, id), "a", "no break in a single-line value");
    assert_eq!(
        *log.borrow(),
        vec!["input:a".to_string(), "submit".to_string()]
    );
}

/// An `<input>` with nowhere to submit swallows Enter, exactly as before — the
/// half of the old behaviour that was right, and which the textarea branch
/// must not have taken with it.
#[test]
fn enter_in_a_plain_input_still_does_nothing() {
    let (mut app, id, log) = mount("input", false);

    focus(&mut app, id);
    type_str(&mut app, "a");
    key(&mut app, KeyCode::Enter, None, false);

    assert_eq!(value(&app, id), "a");
    assert_eq!(*log.borrow(), vec!["input:a".to_string()]);
}

/// Shift+Enter in an `<input>` still submits. Deliberate: with no line break
/// to fall back on, a modifier that turned the key into a no-op would be a
/// regression bought for nothing. (The web backend does gate its submit on
/// Shift; that difference is called out in the PR.)
#[test]
fn shift_enter_in_an_input_still_submits() {
    let (mut app, id, log) = mount("input", true);

    focus(&mut app, id);
    key(&mut app, KeyCode::Enter, None, true);

    assert_eq!(value(&app, id), "");
    assert_eq!(*log.borrow(), vec!["submit".to_string()]);
}

/// The flag Android is told per field. It is the focused control's tag, not a
/// property of the app or of the view that serves it.
#[test]
fn only_a_focused_textarea_is_multiline() {
    let (mut app, id, _log) = mount("textarea", false);
    assert!(
        !app.focused_input_is_multiline(),
        "nothing focused is not multiline"
    );

    focus(&mut app, id);
    assert!(app.focused_input_is_multiline());

    // Clicking away drops input focus, and with it the flag.
    click(&mut app, 700.0, 500.0);
    assert!(!app.focused_input_is_multiline());

    let (mut app, id, _log) = mount("input", false);
    focus(&mut app, id);
    assert!(
        !app.focused_input_is_multiline(),
        "a single-line <input> keeps the action key it always had"
    );
}
