//! A press on the scrollbar of a scroller that holds an editor is the
//! scroller's, not a click in the text.
//!
//! An app puts its editor in an `overflow-y: auto` box as wide as the editor,
//! so the overlay scrollbar is painted over the editor's right edge. The left
//! press went to the editor first (`try_new_editor_click`), which placed the
//! caret and returned, so the thumb could not be grabbed and the track could
//! not be clicked (found in Pimble's note pane). Chrome gives a press on a
//! scrollbar to the scrollbar.
#![cfg(feature = "desktop")]
use super::*;
use rinch_dom::paint::scrollbar::{MARGIN, THICKNESS};
use rinch_editor_core::{Pos, Selection};
use std::cell::Cell;

const SIZE: (u32, u32) = (800, 600);
const WIDTH: f32 = 400.0;

struct Rig {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
    scroller: u64,
}

/// A 400x200 scroller at the origin holding an editor as wide as it, with
/// enough paragraphs to overflow, scrolled part-way by the wheel.
fn rig() -> Rig {
    let ids: Rc<Cell<Option<u64>>> = Rc::new(Cell::new(None));
    let ids_in = ids.clone();
    let handle = crate::editor::create_editor();
    let html: String = (0..40)
        .map(|i| format!("<p>paragraph {i} of the note</p>"))
        .collect();
    assert!(handle.load_html(&html));
    let handle_in = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "position: relative; width: 800px; height: 600px");
        let scroller = scope.create_element("div");
        scroller.set_attribute(
            "style",
            "position: absolute; left: 0px; top: 0px; width: 400px; height: 200px; overflow-y: auto",
        );
        let editor = handle_in.mount(scope);
        editor.set_attribute("style", "width: 100%");
        scroller.append_child(&editor);
        root.append_child(&scroller);
        ids_in.set(Some(scroller.node_id().0));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let scroller = ids.get().unwrap();
    app.handle_event(
        PlatformEvent::MouseWheel {
            x: 100.0,
            y: 100.0,
            delta_x: 0.0,
            delta_y: -120.0,
        },
        SIZE,
        1.0,
    );
    app.resolve_and_repaint(800.0, 600.0);
    handle.set_selection(Selection::cursor(Pos(3)));
    Rig {
        app,
        handle,
        scroller,
    }
}

fn scroll(app: &RinchApp, id: u64) -> f64 {
    app.doc.as_ref().unwrap().borrow().tree.nodes[id]
        .scroll_offset
        .1
}

fn track(app: &RinchApp, id: u64) -> rinch_dom::paint::scrollbar::ScrollbarTrack {
    let d = app.doc.as_ref().unwrap().borrow();
    rinch_dom::paint::scrollbar::scrollbars(&d.tree, id, 1.0)
        .vertical
        .expect("a vertical bar")
}

/// The window point `along` px down the bar, centred on its thickness.
fn on_bar(along: f64) -> (f32, f32) {
    (WIDTH - (MARGIN + THICKNESS / 2.0) as f32, along as f32)
}

fn press(app: &mut RinchApp, (x, y): (f32, f32)) {
    app.handle_event(
        PlatformEvent::MouseDown {
            x,
            y,
            button: rinch_platform::MouseButton::Left,
        },
        SIZE,
        1.0,
    );
}

fn move_to(app: &mut RinchApp, (x, y): (f32, f32)) {
    app.handle_event(PlatformEvent::MouseMove { x, y }, SIZE, 1.0);
}

fn release(app: &mut RinchApp, (x, y): (f32, f32)) {
    app.handle_event(
        PlatformEvent::MouseUp {
            x,
            y,
            button: rinch_platform::MouseButton::Left,
        },
        SIZE,
        1.0,
    );
}

#[test]
fn the_thumb_over_an_editor_is_grabbed_and_dragged() {
    let mut r = rig();
    let start = scroll(&r.app, r.scroller);
    assert!(
        start > 0.0,
        "premise: the wheel scrolled the scroller part-way"
    );
    let t = track(&r.app, r.scroller);
    let grab = on_bar(t.thumb_start(start) + t.thumb_len / 2.0);
    let caret = r.handle.selection();

    press(&mut r.app, grab);
    assert!(
        r.app.scrollbar_drag.is_some(),
        "the press on the thumb grabs it"
    );
    assert_eq!(
        r.handle.selection(),
        caret,
        "the press placed no caret in the text"
    );

    move_to(&mut r.app, (grab.0, grab.1 + 40.0));
    r.app.resolve_and_repaint(800.0, 600.0);
    let moved = scroll(&r.app, r.scroller);
    assert!(moved > start, "the drag scrolled: {start} -> {moved}");
    release(&mut r.app, (grab.0, grab.1 + 40.0));
    assert!(r.app.scrollbar_drag.is_none(), "the release ends the drag");
}

#[test]
fn a_press_on_the_track_beside_an_editor_jumps() {
    let mut r = rig();
    let start = scroll(&r.app, r.scroller);
    let t = track(&r.app, r.scroller);
    let below = t.thumb_start(start) + t.thumb_len + 30.0;
    assert!(
        below < t.track_start + t.track_len,
        "premise: there is track below the thumb"
    );
    let caret = r.handle.selection();

    press(&mut r.app, on_bar(below));
    r.app.resolve_and_repaint(800.0, 600.0);
    assert!(
        scroll(&r.app, r.scroller) > start,
        "the track press scrolled toward it"
    );
    assert_eq!(
        r.handle.selection(),
        caret,
        "the press placed no caret in the text"
    );
}

#[test]
fn a_press_in_the_text_still_places_the_caret() {
    let mut r = rig();
    let caret = r.handle.selection();
    press(&mut r.app, (60.0, 100.0));
    assert!(
        r.app.scrollbar_drag.is_none(),
        "control: the text is not the scrollbar"
    );
    assert_ne!(
        r.handle.selection(),
        caret,
        "control: a press in the text moves the caret"
    );
}
