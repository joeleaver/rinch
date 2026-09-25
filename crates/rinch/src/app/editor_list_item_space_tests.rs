//! Typing a space at the end of a bulleted list item leaves the bullet and the
//! caret on the item's line, driven through the real `PlatformEvent` path.
//!
//! The editor's `li` is a flex row (`align-items: baseline`) of the list marker
//! and the item's paragraph, and its text is `white-space: pre-wrap`. The
//! paragraph used to be measured without its trailing space and then laid out
//! at that width with the space in it, which parley wrapped onto a second,
//! empty line: the paragraph went two lines tall, the marker (baseline-aligned
//! by its bottom edge) dropped to the second line, and the caret after the
//! space sat at the start of that line. The next character put everything
//! back. The layout rules are pinned against Chrome in `rinch-dom`'s
//! `list_item_trailing_space_tests`; what is pinned here is the editor's own
//! markup and stylesheet, and a real key press. Host fonts, so every assertion
//! compares the item with itself before the space.

use super::*;
use rinch_editor_core::{Pos, Selection};

const VP: (u32, u32) = (800, 600);

/// "bullet text" is 3..14 (list 0, item 1, paragraph 2).
const END: Pos = Pos(14);

struct Page {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
}

/// One editor over a one-item bulleted list, focused by a real press in the
/// item, the caret at the end of its text.
fn page() -> Page {
    let slot: Rc<RefCell<Option<crate::editor::EditorHandle>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let (container, handle) = crate::editor::mount_editor(scope);
        handle.load_html("<ul><li><p>bullet text</p></li></ul>");
        container.set_attribute(
            "style",
            "width: 400px; height: 200px; font-size: 16px; font-family: sans-serif",
        );
        root.append_child(&container);
        *slot_in.borrow_mut() = Some(handle);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let handle = slot.borrow_mut().take().expect("captured at mount");
    let (x, y, h) = app
        .editor_caret_point(&handle, Pos(5))
        .expect("the position has a caret");
    let (x, y) = (x + 1.0, y + h / 2.0);
    let button = MouseButton::Left;
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
    assert!(matches!(app.focus_target, FocusTarget::Editor(_)));
    handle.set_selection(Selection::cursor(END));
    app.resolve_and_repaint(800.0, 600.0);
    Page { app, handle }
}

fn type_char(app: &mut RinchApp, key: KeyCode, text: &str) {
    let modifiers = Modifiers::default();
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: Some(text.to_string()),
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

fn item_text(handle: &crate::editor::EditorHandle) -> String {
    let doc = handle.doc();
    let para = doc.child(0).child(0).child(0);
    (0..para.child_count())
        .filter_map(|j| para.child(j).text().map(str::to_string))
        .collect()
}

/// `(marker, paragraph)` boxes of the list item, `(y, height)` each.
fn item_boxes(app: &RinchApp) -> ((f32, f32), (f32, f32)) {
    let doc = app.doc.as_ref().unwrap().borrow();
    let nodes = &doc.tree.nodes;
    let (_, li) = nodes
        .iter()
        .find(|(_, n)| n.tag() == Some("li"))
        .expect("the list item");
    let mut kids = li.children.iter().map(|&c| &nodes[c]);
    let marker = kids.next().expect("the marker");
    assert!(marker.is_pseudo_element, "the first child is the marker");
    let para = kids.find(|n| n.tag() == Some("p")).expect("the paragraph");
    (
        (marker.layout.y, marker.layout.height),
        (para.layout.y, para.layout.height),
    )
}

#[test]
fn a_space_at_the_end_of_a_list_item_keeps_the_bullet_on_its_line() {
    let Page { mut app, handle } = page();
    let before = item_boxes(&app);
    let caret_before = app.editor_caret_point(&handle, END).unwrap();

    type_char(&mut app, KeyCode::Space, " ");
    assert_eq!(item_text(&handle), "bullet text ");
    assert_eq!(handle.selection(), Selection::cursor(Pos(15)));
    assert_eq!(
        item_boxes(&app),
        before,
        "marker and paragraph (y, height) after the space"
    );
    let caret = app.editor_caret_point(&handle, Pos(15)).unwrap();
    assert_eq!(
        caret.1, caret_before.1,
        "the caret after the space stays on the line"
    );
    assert!(caret.0 > caret_before.0, "and moves right, past the space");

    // The control: the next character leaves the item as it was, too.
    type_char(&mut app, KeyCode::KeyX, "x");
    assert_eq!(item_text(&handle), "bullet text x");
    assert_eq!(item_boxes(&app), before, "after the next character");
}
