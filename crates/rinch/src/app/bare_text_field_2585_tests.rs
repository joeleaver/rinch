//! A text control is editable because of what it **is**, not because the app
//! listens to it (issue #2585).
//!
//! The desktop runtime used to recognise a text field only by `data-oninput`,
//! which `rsx!` writes for an `oninput` handler (or a no-op shim beside
//! `onchange` and friends). A bare `input { }` or `textarea { }` — an
//! uncontrolled field read later with `live_value()`, or one in HTML set with
//! `set_inner_html` — carried none, so a click did not focus it and typing
//! changed nothing, where Chrome and rinch-web give an ordinary editable field.
//! Every text-entry `<input>` type and every `<textarea>` now takes the text
//! engine by its element; the non-text types (checkbox, radio, the buttons,
//! range, color, file, image) still do not.

use super::*;
use rinch_core::dom::{DomDocument, NodeId};
use std::cell::Cell;

/// One field, with no handler of any kind, and a second bare `<input>` after
/// it for the Tab and programmatic paths. `attrs` are set on the first field.
fn mount(tag: &'static str, attrs: &[(&str, &str)]) -> (RinchApp, u64, u64) {
    let attrs: Vec<(String, String)> = attrs
        .iter()
        .map(|(n, v)| (n.to_string(), v.to_string()))
        .collect();
    let ids: Rc<Cell<(u64, u64)>> = Rc::new(Cell::new((0, 0)));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let field = scope.create_element(tag);
        field.set_attribute("style", "display: block; width: 200px; height: 30px");
        for (name, value) in &attrs {
            field.set_attribute(name, value);
        }
        let other = scope.create_element("input");
        other.set_attribute("style", "display: block; width: 200px; height: 30px");
        root.append_child(&field);
        root.append_child(&other);
        ids_in.set((field.node_id().0, other.node_id().0));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (a, b) = ids.get();
    (app, a, b)
}

fn press(app: &mut RinchApp, key: KeyCode, text: Option<&str>, modifiers: Modifiers) {
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: text.map(str::to_string),
            modifiers,
            repeat: KeyRepeat::Unknown,
        },
        (800, 600),
        1.0,
    );
}

fn type_str(app: &mut RinchApp, text: &str) {
    for ch in text.chars() {
        press(
            app,
            KeyCode::KeyA,
            Some(&ch.to_string()),
            Modifiers::default(),
        );
    }
}

fn ctrl(app: &mut RinchApp, key: KeyCode) {
    press(
        app,
        key,
        None,
        Modifiers {
            ctrl: true,
            ..Default::default()
        },
    );
}

fn click_center(app: &mut RinchApp, id: u64) {
    let (x, y) = {
        let d = app.doc.as_ref().unwrap().borrow();
        let (ax, ay, w, h) = painted_element_box(&d.tree, id);
        (ax + w / 2.0, ay + h / 2.0)
    };
    for down in [true, false] {
        let event = if down {
            PlatformEvent::MouseDown {
                x,
                y,
                button: MouseButton::Left,
            }
        } else {
            PlatformEvent::MouseUp {
                x,
                y,
                button: MouseButton::Left,
            }
        };
        app.handle_event(event, (800, 600), 1.0);
    }
}

/// What a script reads as the field's `.value` (`NodeHandle::live_value`).
fn live_value(app: &RinchApp, id: u64) -> Option<String> {
    app.doc.as_ref().unwrap().borrow().live_value(NodeId(id))
}

fn value_attr(app: &RinchApp, id: u64) -> Option<String> {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree
        .get(id)
        .and_then(|n| n.attributes.get("value").cloned())
}

/// Every text-entry `<input>` type — the default, an unknown one, any case —
/// and a `<textarea>`, with no handler at all: a click focuses it as a text
/// field, typing goes in, and the value is readable. Red before #2585: the
/// click took no `FocusTarget::Input` and the value stayed `None`.
#[test]
fn every_text_control_without_a_handler_takes_typed_text() {
    const CASES: &[(&str, &[(&str, &str)])] = &[
        ("input", &[]),
        ("input", &[("type", "text")]),
        ("input", &[("type", "TEXT")]),
        ("input", &[("type", "search")]),
        ("input", &[("type", "email")]),
        ("input", &[("type", "url")]),
        ("input", &[("type", "tel")]),
        ("input", &[("type", "password")]),
        ("input", &[("type", "number")]),
        ("input", &[("type", "bogus")]),
        ("textarea", &[]),
    ];
    for &(tag, attrs) in CASES {
        let (mut app, id, _) = mount(tag, attrs);
        click_center(&mut app, id);
        assert_eq!(
            app.focus_target,
            FocusTarget::Input(id),
            "{tag} {attrs:?}: a click focuses it as a text field"
        );
        type_str(&mut app, "12");
        assert_eq!(
            live_value(&app, id).as_deref(),
            Some("12"),
            "{tag} {attrs:?}: typed text is its value"
        );
        assert_eq!(
            value_attr(&app, id).as_deref(),
            Some("12"),
            "{tag} {attrs:?}"
        );
    }
}

/// The non-text `<input>` types stay what they were: a click gives none of
/// them the text engine, and typing writes no value into any of them.
#[test]
fn a_non_text_input_without_a_handler_takes_no_text() {
    for ty in [
        "checkbox", "radio", "button", "submit", "reset", "range", "color", "file", "image",
        "CHECKBOX",
    ] {
        let (mut app, id, _) = mount("input", &[("type", ty)]);
        click_center(&mut app, id);
        assert_ne!(app.focus_target, FocusTarget::Input(id), "{ty}");
        type_str(&mut app, "12");
        assert_eq!(value_attr(&app, id), None, "{ty}: typing wrote no value");
    }
}

/// Tab and programmatic focus reach a bare field as a click does.
#[test]
fn a_bare_field_takes_tab_and_programmatic_focus() {
    let (mut app, first, second) = mount("input", &[]);
    press(&mut app, KeyCode::Tab, None, Modifiers::default());
    assert_eq!(app.focus_target, FocusTarget::Input(first), "Tab");
    type_str(&mut app, "ab");
    assert_eq!(live_value(&app, first).as_deref(), Some("ab"));

    app.try_focus_input(second);
    assert_eq!(app.focus_target, FocusTarget::Input(second), "focus()");
    type_str(&mut app, "cd");
    assert_eq!(live_value(&app, second).as_deref(), Some("cd"));
    assert_eq!(
        live_value(&app, first).as_deref(),
        Some("ab"),
        "the first kept its text"
    );
}

/// `readonly` focuses and selects but refuses text; `disabled` takes no focus
/// at all — Chrome's rules, unchanged for a field with no handler.
#[test]
fn readonly_and_disabled_still_block_a_bare_field() {
    let (mut app, id, _) = mount("input", &[("readonly", ""), ("value", "keep")]);
    click_center(&mut app, id);
    assert_eq!(
        app.focus_target,
        FocusTarget::Input(id),
        "readonly still focuses"
    );
    type_str(&mut app, "x");
    press(&mut app, KeyCode::Backspace, None, Modifiers::default());
    assert_eq!(live_value(&app, id).as_deref(), Some("keep"), "readonly");

    let (mut app, id, _) = mount("textarea", &[("disabled", "")]);
    click_center(&mut app, id);
    assert_ne!(
        app.focus_target,
        FocusTarget::Input(id),
        "disabled takes no focus"
    );
    type_str(&mut app, "x");
    assert_eq!(value_attr(&app, id), None, "disabled");
}

/// The rest of the text engine — selection, undo/redo, the IME — works on a
/// bare field exactly as on one with a handler.
#[test]
fn selection_undo_and_ime_work_on_a_bare_field() {
    let (mut app, id, _) = mount("input", &[]);
    click_center(&mut app, id);
    type_str(&mut app, "abc");

    ctrl(&mut app, KeyCode::KeyA);
    type_str(&mut app, "x");
    assert_eq!(
        live_value(&app, id).as_deref(),
        Some("x"),
        "select all, then type"
    );

    ctrl(&mut app, KeyCode::KeyZ);
    assert_eq!(live_value(&app, id).as_deref(), Some("abc"), "undo");

    app.handle_event(
        PlatformEvent::Ime(ImeEvent::Preedit {
            text: "に".to_string(),
            cursor: None,
        }),
        (800, 600),
        1.0,
    );
    app.handle_event(
        PlatformEvent::Ime(ImeEvent::Commit("に".to_string())),
        (800, 600),
        1.0,
    );
    let v = live_value(&app, id).unwrap_or_default();
    assert!(
        v.contains('に') && v.len() > "abc".len(),
        "IME commit: {v:?}"
    );
}

/// A paste lands in a bare field too (the clipboard read's completion).
#[cfg(feature = "clipboard")]
#[test]
fn a_paste_lands_in_a_bare_field() {
    let (mut app, id, _) = mount("textarea", &[]);
    click_center(&mut app, id);
    let epoch = app.focus_epoch;
    app.complete_input_paste(id, epoch, "pasted".to_string());
    assert_eq!(live_value(&app, id).as_deref(), Some("pasted"));
}

/// A focused bare field whose node stops being a text field — the way a
/// recycled slot looks — gives the keyboard up at the next keystroke, as a
/// field whose handler was freed does, rather than writing a value into it.
#[test]
fn a_focused_bare_field_that_stops_being_one_releases_the_keyboard() {
    let (mut app, id, _) = mount("input", &[]);
    click_center(&mut app, id);
    assert_eq!(app.focus_target, FocusTarget::Input(id));
    {
        let mut d = app.doc.as_ref().unwrap().borrow_mut();
        let node = d.tree.get_mut(id).unwrap();
        // The slot now holds something that is no text field.
        node.attributes.insert("type".into(), "checkbox".into());
    }
    type_str(&mut app, "x");
    assert_eq!(app.focus_target, FocusTarget::None);
    assert_eq!(value_attr(&app, id), None);
}
