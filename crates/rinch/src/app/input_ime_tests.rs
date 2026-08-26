//! What an `<input>`'s `data-preedit` attribute holds, and what it does not.
//!
//! The composition is written to the attribute and **never** to the field's
//! `value`: the paint path splices it in at the caret for display only
//! (`paint_input_value`), so an abandoned composition leaves nothing behind.
//! The Android IME path writes to this attribute through
//! `shell::android_ime`, and these are the tests that say what it has to put
//! there — the convention is one convention, not one per backend.

use super::*;
use rinch_core::events::{InputCallback, register_input_handler};
use std::cell::Cell;

/// One `<input>` that records every `oninput` payload.
fn mount_fixture() -> (RinchApp, usize, Rc<RefCell<Vec<String>>>) {
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let input_id = {
        let log = log.clone();
        register_input_handler(InputCallback::new(move |v: String| {
            log.borrow_mut().push(v);
        }))
    };
    let id: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let id_in = id.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let a = scope.create_element("input");
        a.set_attribute("style", "width: 200px; height: 30px");
        a.set_attribute("data-oninput", &input_id.0.to_string());
        root.append_child(&a);
        id_in.set(Some(a.node_id().0));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let a_id = id.get().expect("node id captured at mount");
    (app, a_id, log)
}

fn focus(app: &mut RinchApp, id: usize) {
    let (cx, cy) = {
        let d = app.doc.as_ref().unwrap().borrow();
        let n = d.tree.get(id).unwrap();
        let (ax, ay) = RinchApp::compute_absolute_position(&d.tree, id);
        (ax + n.layout.width / 2.0, ay + n.layout.height / 2.0)
    };
    for button in [true, false] {
        let event = if button {
            PlatformEvent::MouseDown {
                x: cx,
                y: cy,
                button: MouseButton::Left,
            }
        } else {
            PlatformEvent::MouseUp {
                x: cx,
                y: cy,
                button: MouseButton::Left,
            }
        };
        app.handle_event(event, (800, 600), 1.0);
    }
}

fn type_str(app: &mut RinchApp, text: &str) {
    for ch in text.chars() {
        app.handle_event(
            PlatformEvent::KeyDown {
                key: KeyCode::KeyA,
                logical_key: None,
                text: Some(ch.to_string()),
                modifiers: Modifiers::default(),
            },
            (800, 600),
            1.0,
        );
    }
}

fn ime(app: &mut RinchApp, event: ImeEvent) {
    app.handle_event(PlatformEvent::Ime(event), (800, 600), 1.0);
}

fn preedit(app: &mut RinchApp, text: &str) {
    ime(
        app,
        ImeEvent::Preedit {
            text: text.to_string(),
            cursor: None,
        },
    );
}

fn attr(app: &RinchApp, id: usize, name: &str) -> Option<String> {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree.get(id).and_then(|n| n.attributes.get(name).cloned())
}

/// The composition goes to `data-preedit` and nowhere else: the value is
/// untouched and `oninput` does not fire, because nothing has been typed yet.
#[test]
fn a_composition_is_written_to_data_preedit_and_never_into_the_value() {
    let (mut app, id, log) = mount_fixture();
    focus(&mut app, id);
    type_str(&mut app, "ab");
    log.borrow_mut().clear();

    preedit(&mut app, "にほん");

    assert_eq!(attr(&app, id, "data-preedit").as_deref(), Some("にほん"));
    assert_eq!(
        attr(&app, id, "value").as_deref(),
        Some("ab"),
        "the composition is not in the value"
    );
    assert!(
        log.borrow().is_empty(),
        "composing is not editing — no oninput"
    );
}

/// Each `Preedit` carries the whole composition, so the attribute is replaced
/// rather than extended.
#[test]
fn a_second_composition_replaces_the_first() {
    let (mut app, id, _log) = mount_fixture();
    focus(&mut app, id);

    preedit(&mut app, "に");
    preedit(&mut app, "にほ");
    preedit(&mut app, "にほん");

    assert_eq!(attr(&app, id, "data-preedit").as_deref(), Some("にほん"));
}

/// An empty composition removes the attribute outright — paint keys off its
/// presence, so leaving an empty one behind would be a different state.
#[test]
fn an_empty_composition_removes_the_attribute() {
    let (mut app, id, _log) = mount_fixture();
    focus(&mut app, id);

    preedit(&mut app, "に");
    preedit(&mut app, "");

    assert_eq!(attr(&app, id, "data-preedit"), None);
}

/// A commit clears the composition and inserts its own text — which need not
/// be what was composed, and is not, for a CJK conversion or an autocorrect.
#[test]
fn a_commit_clears_the_composition_and_inserts_the_committed_text() {
    let (mut app, id, log) = mount_fixture();
    focus(&mut app, id);

    preedit(&mut app, "にほん");
    ime(&mut app, ImeEvent::Commit("日本".to_string()));

    assert_eq!(attr(&app, id, "data-preedit"), None);
    assert_eq!(attr(&app, id, "value").as_deref(), Some("日本"));
    assert_eq!(
        *log.borrow(),
        vec!["日本".to_string()],
        "one oninput, carrying the committed text"
    );
}

/// The caret in the *value* does not move while composing: the composition is
/// drawn at it, and `paint_input_value` splices it in there.
#[test]
fn composing_does_not_move_the_caret_in_the_value() {
    let (mut app, id, _log) = mount_fixture();
    focus(&mut app, id);
    type_str(&mut app, "ab");
    let before = attr(&app, id, "data-cursor-pos");

    preedit(&mut app, "にほん");

    assert_eq!(attr(&app, id, "data-cursor-pos"), before);
    assert_eq!(before.as_deref(), Some("2"));
}

/// Composition ending without a commit (`ImeEvent::Disabled` — the desktop
/// path for it) leaves the value alone. The Android path never sends this; it
/// commits instead, because Android's composing text is real text.
#[test]
fn a_disabled_composition_leaves_no_trace_in_the_value() {
    let (mut app, id, log) = mount_fixture();
    focus(&mut app, id);
    type_str(&mut app, "ab");
    log.borrow_mut().clear();

    preedit(&mut app, "にほん");
    ime(&mut app, ImeEvent::Disabled);

    assert_eq!(attr(&app, id, "data-preedit"), None);
    assert_eq!(attr(&app, id, "value").as_deref(), Some("ab"));
    assert!(log.borrow().is_empty());
}
