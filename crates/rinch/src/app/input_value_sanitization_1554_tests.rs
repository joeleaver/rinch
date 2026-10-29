//! A programmatic `value` holding line breaks, in a desktop `<input>`
//! (#1554). HTML's value sanitization for the text states strips them
//! (Chrome 155: `setAttribute("value", "s\nt")` leaves the attribute as
//! written and `.value` is `"st"`), so the field paints, focuses, edits and
//! adopts the stripped text and keeps the attribute as it was written until
//! the user edits. A `<textarea>` keeps its line breaks.
//!
//! Not the rule for text the **user** inserts (#824), which folds an inner
//! break into a space.

use super::*;
use rinch_core::events::{InputCallback, register_input_handler};
use std::cell::Cell;

const VP: (u32, u32) = (400, 200);

/// 30px tall, no border or padding, red text on a transparent page, so any
/// painted pixel below `y = 30` is the field's text outside its box.
const FIELD_STYLE: &str = "display: block; width: 300px; height: 30px; padding: 0; margin: 0; \
     border: 0; font-size: 16px; line-height: 20px; font-family: sans-serif; \
     color: rgb(255, 0, 0); background: transparent";

type Log = Rc<RefCell<Vec<String>>>;

fn ev(app: &mut RinchApp, event: PlatformEvent) -> Vec<AppAction> {
    app.handle_event(event, VP, 1.0)
}

fn key(app: &mut RinchApp, key: KeyCode) {
    let modifiers = Modifiers::default();
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

fn typed(app: &mut RinchApp, t: &str) {
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
}

fn attr(app: &RinchApp, id: u64, name: &str) -> Option<String> {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree.get(id).and_then(|n| n.attributes.get(name).cloned())
}

fn set_value(app: &RinchApp, id: u64, v: &str) {
    let doc = app.doc.as_ref().unwrap().clone();
    doc.borrow_mut()
        .set_attribute(rinch_core::dom::NodeId(id), "value", v);
}

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

/// One `<input>` (or `tag`) holding `value`, logging its `oninput`.
fn page(tag: &'static str, value: &'static str) -> (RinchApp, u64, Log) {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let h = register_input_handler(InputCallback::new(move |v: String| {
        l.borrow_mut().push(v);
    }));
    let id = Rc::new(Cell::new(0u64));
    let id_in = id.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "padding: 0; margin: 0");
        let field = scope.create_element(tag);
        field.set_attribute("style", FIELD_STYLE);
        field.set_attribute("value", value);
        field.set_attribute("data-oninput", &h.0.to_string());
        root.append_child(&field);
        id_in.set(field.node_id().0);
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    (app, id.get(), log)
}

/// `(inside, below)`: the painted pixels inside the field's 30px box, and
/// those below it.
fn painted(app: &mut RinchApp) -> (usize, usize) {
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    app.scene_dirty = true;
    app.has_previous_frame = false;
    let (px, w, _) = app.build_pixels(1.0, VP, true);
    let w = w as usize;
    let (mut inside, mut below) = (0, 0);
    for (i, p) in px.as_chunks::<4>().0.iter().enumerate() {
        if p[3] != 0 {
            if i / w < 30 {
                inside += 1;
            } else {
                below += 1;
            }
        }
    }
    (inside, below)
}

/// The issue's symptom: the extra lines were painted below the box.
#[test]
fn an_inputs_line_breaks_are_not_painted_below_its_box() {
    let (mut app, _id, _log) = page("input", "ab\ncd\nef");
    let (inside, below) = painted(&mut app);
    assert!(inside > 0, "positive control: the value is painted");
    assert_eq!(below, 0, "nothing below the field's box");

    // Positive control for the oracle: a textarea keeps its line breaks, and
    // given room below 30px its second and third lines are painted there. (In
    // its 30px box they are clipped to it and scrolled to, #827.)
    let (mut app, id, _log) = page("textarea", "ab\ncd\nef");
    rinch_core::dom::DomDocument::set_attribute(
        &mut *app.doc.as_ref().unwrap().borrow_mut(),
        rinch_core::dom::NodeId(id),
        "style",
        &format!("{FIELD_STYLE}; height: 80px"),
    );
    assert!(painted(&mut app).1 > 0, "a textarea's lines do reach below");
}

/// Focus edits the stripped text and keeps the attribute as written; the
/// first edit writes the field's text over it.
#[test]
fn focus_edits_the_stripped_value() {
    let (mut app, id, log) = page("input", "ab\r\ncd");
    focus(&mut app, id);
    assert_eq!(attr(&app, id, "value").as_deref(), Some("ab\r\ncd"));
    key(&mut app, KeyCode::End);
    assert_eq!(attr(&app, id, "data-cursor-pos").as_deref(), Some("4"));
    typed(&mut app, "X");
    assert_eq!(attr(&app, id, "value").as_deref(), Some("abcdX"));
    assert_eq!(*log.borrow(), vec!["abcdX".to_string()]);
}

/// A write to the focused field is adopted stripped, the caret lands at the
/// end of the stripped text, and the attribute stays as written over later
/// frames (no write-back, nothing adopted again).
#[test]
fn a_write_to_the_focused_input_is_adopted_stripped() {
    let (mut app, id, log) = page("input", "hello");
    focus(&mut app, id);
    key(&mut app, KeyCode::End);
    set_value(&app, id, "x\ny\r\n");
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    assert_eq!(app.focused_input_value, "xy");
    assert_eq!(attr(&app, id, "data-cursor-pos").as_deref(), Some("2"));
    for _ in 0..3 {
        app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    }
    assert_eq!(attr(&app, id, "value").as_deref(), Some("x\ny\r\n"));
    assert!(log.borrow().is_empty(), "a programmatic write is no input");
    typed(&mut app, "Z");
    assert_eq!(attr(&app, id, "value").as_deref(), Some("xyZ"));
}
