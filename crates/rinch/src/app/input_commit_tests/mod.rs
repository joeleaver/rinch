//! The `data-onchange` commit boundary for text inputs (issue #226).
//!
//! HTML `change` semantics: a typed gesture ends when focus leaves the input
//! (click elsewhere, Tab, a select/editor claim) or on an explicit Enter
//! commit — and the event fires only if the value actually changed since the
//! gesture began. `oninput` stays per-keystroke and is untouched.
//!
//! The tests are grouped by topic: the commit boundary itself
//! (`change_commit`), programmatic writes to the focused field
//! (`programmatic_writes`, issue #238) and undo/redo on it (`undo_redo`,
//! issue #288). The fixtures they share live here.

mod change_commit;
mod programmatic_writes;
mod undo_redo;

use super::*;
use rinch_core::events::{InputCallback, register_input_handler};
use std::cell::Cell;

/// Mount two `<input>`s and a `tabindex="0"` div. Input A records
/// `a-input:`/`a-change:`/`a-submit`, input B records `b-change:` (its
/// oninput is a no-op recorder-less handler). Returns the app, the three
/// node ids, and the shared event log.
#[allow(clippy::type_complexity)]
fn mount_fixture() -> (RinchApp, u64, u64, u64, Rc<RefCell<Vec<String>>>) {
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let record = |tag: &'static str| {
        let log = log.clone();
        register_input_handler(InputCallback::new(move |v: String| {
            log.borrow_mut().push(format!("{tag}:{v}"));
        }))
    };
    let a_input_id = record("a-input");
    let a_change_id = record("a-change");
    let b_input_id = register_input_handler(InputCallback::new(|_| {}));
    let b_change_id = record("b-change");
    let a_submit_id = rinch_core::register_handler(std::rc::Rc::new({
        let log = log.clone();
        move || log.borrow_mut().push("a-submit".to_string())
    }));

    let ids: Rc<Cell<Option<(u64, u64, u64)>>> = Rc::new(Cell::new(None));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let a = scope.create_element("input");
        a.set_attribute("style", "width: 200px; height: 30px");
        a.set_attribute("data-oninput", &a_input_id.0.to_string());
        a.set_attribute("data-onchange", &a_change_id.0.to_string());
        a.set_attribute("data-onsubmit", &a_submit_id.0.to_string());
        let b = scope.create_element("input");
        b.set_attribute("style", "width: 200px; height: 30px");
        b.set_attribute("data-oninput", &b_input_id.0.to_string());
        b.set_attribute("data-onchange", &b_change_id.0.to_string());
        let div = scope.create_element("div");
        div.set_attribute("style", "width: 200px; height: 40px");
        div.set_attribute("tabindex", "0");
        root.append_child(&a);
        root.append_child(&b);
        root.append_child(&div);
        ids_in.set(Some((a.node_id().0, b.node_id().0, div.node_id().0)));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (a_id, b_id, div_id) = ids.get().expect("node ids captured at mount");
    (app, a_id, b_id, div_id, log)
}

fn key(app: &mut RinchApp, key: KeyCode, text: Option<&str>) {
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

fn type_str(app: &mut RinchApp, text: &str) {
    for ch in text.chars() {
        // The key code is irrelevant for text insertion (the `_` arm inserts
        // the text payload); any non-special key works.
        key(app, KeyCode::KeyA, Some(&ch.to_string()));
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

fn abs_center(app: &RinchApp, id: u64) -> (f32, f32) {
    let d = app.doc.as_ref().unwrap().borrow();
    let (ax, ay, ax_w, ay_h) = painted_element_box(&d.tree, id);
    (ax + ax_w / 2.0, ay + ay_h / 2.0)
}

fn click_center(app: &mut RinchApp, id: u64) {
    let (cx, cy) = abs_center(app, id);
    click(app, cx, cy);
}

/// The focused input's engine text and `(anchor, head)` byte offsets.
fn engine(app: &RinchApp) -> (String, usize, usize) {
    let state = app
        .focused_input_state
        .as_ref()
        .expect("an input is focused");
    (
        state.document.to_text(),
        state.selection.anchor.0,
        state.selection.head.0,
    )
}

fn attr(app: &RinchApp, id: u64, name: &str) -> Option<String> {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree.get(id).and_then(|n| n.attributes.get(name).cloned())
}

/// A component/app write: the trait path, exactly what a `value_fn` effect does.
fn write_value(app: &RinchApp, id: u64, value: &str) {
    let doc = app.doc.clone().expect("doc");
    let mut d = doc.borrow_mut();
    d.set_attribute(rinch_core::dom::NodeId(id), "value", value);
}

fn changes(log: &Rc<RefCell<Vec<String>>>) -> Vec<String> {
    log.borrow()
        .iter()
        .filter(|e| e.starts_with("a-change"))
        .cloned()
        .collect()
}
