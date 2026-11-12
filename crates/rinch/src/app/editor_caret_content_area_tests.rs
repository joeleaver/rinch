//! The rich-text editor's caret is drawn over its text's content area, as
//! Chrome draws a caret: its font's rounded ascent above the baseline to its
//! rounded descent below. At 16px Inter that is 20px; under `line-height:
//! 48px` Chrome's caret (a collapsed `Range`'s rect, the selection's too, in
//! a `contenteditable` paragraph; Chrome 157) starts 14px down the line. The
//! desktop overlay used to fill the line box, 48px from its top. Text in the
//! bundled Inter (`perf_expect::new_app`), the measured face.

use super::hit_testing::painted_element_box;
use super::*;
use rinch_editor_core::{Pos, Selection};
use std::cell::RefCell;
use std::rc::Rc;

const VP: (u32, u32) = (800, 400);

#[test]
fn the_editor_caret_is_drawn_over_the_glyphs_not_the_line_box() {
    let slot: Rc<RefCell<Option<crate::editor::EditorHandle>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = super::perf_expect::new_app(move |scope: &mut RenderScope| {
        let (container, handle) = crate::editor::mount_editor(scope);
        handle.load_html("<p>Hello world</p>");
        container.set_attribute(
            "style",
            "width: 600px; font-size: 16px; line-height: 48px; font-family: sans-serif",
        );
        *slot_in.borrow_mut() = Some(handle);
        container
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let handle = slot.borrow_mut().take().expect("captured at mount");
    let block = {
        let doc = app.doc.as_ref().unwrap().borrow();
        doc.query_selector_all("[data-pm-editor] p")[0].0
    };
    // Focus with a press inside the line, then put the caret after "Hel".
    let (bx, by, _, bh) = {
        let doc = app.doc.as_ref().unwrap().borrow();
        painted_element_box(&doc.tree, block)
    };
    assert_eq!(bh, 48.0, "one line, 48px");
    let (x, y, button) = (bx + 2.0, by + 24.0, MouseButton::Left);
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    assert!(matches!(app.focus_target, FocusTarget::Editor(_)));
    handle.set_selection(Selection::cursor(Pos(4)));
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);

    let painted = {
        let doc = app.doc.as_ref().unwrap().borrow();
        let id = doc.query_selector_all("[data-pm-caret]")[0];
        let (_, y, _, h) = painted_element_box(&doc.tree, id.0);
        (y - by, h)
    };
    // `EditorHandle::caret_rect` is the same box (a popup's anchor).
    let rect = handle.caret_rect(Pos(4)).expect("a caret rect");
    // Chrome: 14px down the line, 20px tall.
    for (what, (top, h)) in [
        ("painted", painted),
        ("caret_rect", (rect.y - by, rect.height)),
    ] {
        assert!(
            (top - 14.0).abs() < 0.1 && (h - 20.0).abs() < 0.1,
            "{what} caret: top {top} h {h}, Chrome 14 / 20"
        );
    }
}
