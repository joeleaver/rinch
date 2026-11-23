//! A text field with `dir=auto` takes its direction from its value as the
//! user types (#2272, review of PR #2942).
//!
//! The desktop shell writes a focused field's `value` itself, around
//! `set_attribute`, so the document's attribute-change path never saw the
//! edit and a `dir=auto` field kept the direction it was mounted with. Chrome
//! 158: `execCommand('insertText', 'א')` in an empty `<input dir=auto>` turns
//! it `rtl` at once, and deleting the letter turns it back.

use super::*;
use std::cell::Cell;

fn mount(tag: &'static str) -> (RinchApp, u64) {
    let id: Rc<Cell<u64>> = Rc::new(Cell::new(0));
    let id_in = id.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let field = scope.create_element(tag);
        field.set_attribute("style", "display: block; width: 200px; height: 30px");
        field.set_attribute("dir", "auto");
        root.append_child(&field);
        id_in.set(field.node_id().0);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let id = id.get();
    (app, id)
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

fn click_center(app: &mut RinchApp, id: u64) {
    let (x, y) = {
        let d = app.doc.as_ref().unwrap().borrow();
        let (ax, ay, w, h) = painted_element_box(&d.tree, id);
        (ax + w / 2.0, ay + h / 2.0)
    };
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
        app.handle_event(event, (800, 600), 1.0);
    }
}

fn rtl(app: &mut RinchApp, id: u64) -> bool {
    app.resolve_and_repaint(800.0, 600.0);
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree.get(id).unwrap().computed_style.direction_rtl
}

/// Typed keys and an IME commit, in an `<input>` and a `<textarea>`.
#[test]
fn typing_into_a_dir_auto_field_sets_its_direction() {
    for tag in ["input", "textarea"] {
        let (mut app, id) = mount(tag);
        assert!(!rtl(&mut app, id), "{tag}: an empty field is ltr");
        click_center(&mut app, id);

        key(&mut app, KeyCode::KeyA, Some("\u{5d0}"));
        assert!(rtl(&mut app, id), "{tag}: rtl once Hebrew is typed");

        key(&mut app, KeyCode::Backspace, None);
        assert!(!rtl(&mut app, id), "{tag}: ltr again once it is deleted");

        app.handle_event(
            PlatformEvent::Ime(ImeEvent::Commit("\u{5d1}".into())),
            (800, 600),
            1.0,
        );
        assert!(rtl(&mut app, id), "{tag}: rtl from an IME commit");
    }
}
