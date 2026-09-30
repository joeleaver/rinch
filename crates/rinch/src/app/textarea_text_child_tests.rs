//! A `<textarea>` whose text arrived as a **child** is edited from that text
//! (#1159).
//!
//! The children are its default value — what a browser's `.value` holds until
//! something writes it — so the desktop field starts editing from them, and
//! the first keystroke writes `value` with them in it. Before, focus read only
//! the (absent) `value` attribute: the field started empty and the first key
//! replaced the text with itself, while the child went on painting underneath.

use super::*;
use rinch_core::events::{InputCallback, register_input_handler};
use std::cell::Cell;

fn mount(child: &'static str) -> (RinchApp, usize, Rc<RefCell<Vec<String>>>) {
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let input_id = register_input_handler(InputCallback::new({
        let log = log.clone();
        move |v: String| log.borrow_mut().push(v)
    }));
    let id: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let id_in = id.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let field = scope.create_element("textarea");
        field.set_attribute("style", "display: block; width: 200px; height: 60px");
        field.set_attribute("data-oninput", &input_id.0.to_string());
        let text = scope.create_text(child);
        field.append_child(&text);
        root.append_child(&field);
        id_in.set(Some(field.node_id().0));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    (app, id.get().expect("node id captured at mount"), log)
}

fn press(app: &mut RinchApp, key: KeyCode, text: Option<&str>) {
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: text.map(str::to_string),
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Unknown,
        },
        (800, 600),
        1.0,
    );
}

fn click_into(app: &mut RinchApp, id: usize) {
    let (x, y) = {
        let d = app.doc.as_ref().unwrap().borrow();
        let (ax, ay, w, h) = painted_element_box(&d.tree, id);
        (ax + w / 2.0, ay + h / 2.0)
    };
    for ev in [
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
        app.handle_event(ev, (800, 600), 1.0);
    }
}

fn value_attr(app: &RinchApp, id: usize) -> Option<String> {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree
        .get(id)
        .and_then(|n| n.attributes.get("value").cloned())
}

#[test]
fn typing_into_a_text_child_textarea_edits_its_text() {
    let (mut app, id, log) = mount("hello");
    click_into(&mut app, id);
    // A frame between the focus and the key: the per-frame adoption of a
    // programmatic `value` must read the children too, or it adopts "".
    app.resolve_and_repaint(800.0, 600.0);
    press(&mut app, KeyCode::End, None);
    press(&mut app, KeyCode::KeyX, Some("X"));
    assert_eq!(value_attr(&app, id).as_deref(), Some("helloX"));
    assert_eq!(log.borrow().last().map(String::as_str), Some("helloX"));
}

/// The first click into a text-child textarea places the caret from the child
/// text: a click right of "hello" lands at its end (review of #1179 — the
/// click→caret map reads `control_value` too).
#[test]
fn the_first_click_places_the_caret_in_the_child_text() {
    let (mut app, id, _log) = mount("hello");
    let (x, y) = {
        let d = app.doc.as_ref().unwrap().borrow();
        let (ax, ay, w, _h) = painted_element_box(&d.tree, id);
        (ax + w - 10.0, ay + 6.0)
    };
    for ev in [
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
        app.handle_event(ev, (800, 600), 1.0);
    }
    press(&mut app, KeyCode::KeyX, Some("X"));
    assert_eq!(value_attr(&app, id).as_deref(), Some("helloX"));
}

/// A `value` attribute removed while the field is focused leaves it holding
/// its text children, as removing the attribute leaves a browser's `.value`
/// alone — not "" (review of #1179: the per-frame adoption reads
/// `control_value`).
#[test]
fn a_value_removed_while_focused_falls_back_to_the_child_text() {
    let (mut app, id, _log) = mount("hello");
    click_into(&mut app, id);
    app.resolve_and_repaint(800.0, 600.0);
    app.doc
        .as_ref()
        .unwrap()
        .borrow_mut()
        .remove_attribute(rinch_core::dom::NodeId(id), "value");
    app.resolve_and_repaint(800.0, 600.0);
    press(&mut app, KeyCode::End, None);
    press(&mut app, KeyCode::KeyX, Some("X"));
    assert_eq!(value_attr(&app, id).as_deref(), Some("helloX"));
}
