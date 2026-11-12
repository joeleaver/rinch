//! A press on the scrollbar of a scroller whose content is draggable is the
//! scroller's, not the start of a drag.
//!
//! A horizontal board of draggable columns (a kanban: sortable lists whose
//! items are `draggable`) paints its overlay scrollbar over the columns. The
//! left press looked for a draggable under the pointer first, found a column
//! and armed a drag, so the thumb could not be grabbed and the track could
//! not be clicked (found in Pimble's kanban). Chrome gives a press on a
//! scrollbar to the scrollbar; the editor arm already did (#2549).
#![cfg(feature = "desktop")]
use super::*;
use rinch_dom::paint::scrollbar::{MARGIN, THICKNESS};
use std::cell::Cell;

const SIZE: (u32, u32) = (800, 600);
const HEIGHT: f32 = 200.0;

struct Rig {
    app: RinchApp,
    scroller: u64,
}

/// A 400x200 horizontal scroller at the origin holding ten draggable columns,
/// each 150 px wide and as tall as the scroller, scrolled part-way.
fn rig() -> Rig {
    let ids: Rc<Cell<Option<u64>>> = Rc::new(Cell::new(None));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "position: relative; width: 800px; height: 600px");
        let scroller = scope.create_element("div");
        scroller.set_attribute(
            "style",
            "position: absolute; left: 0px; top: 0px; width: 400px; height: 200px; overflow-x: auto; overflow-y: hidden; display: flex",
        );
        for _ in 0..10 {
            let column = scope.create_element("div");
            column.set_attribute("draggable", "true");
            column.set_attribute("style", "flex: 0 0 150px; height: 200px");
            scroller.append_child(&column);
        }
        root.append_child(&scroller);
        ids_in.set(Some(scroller.node_id().0));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let scroller = ids.get().unwrap();
    {
        let doc = app.doc.as_ref().unwrap();
        doc.borrow_mut().tree.nodes[scroller].scroll_offset.0 = 300.0;
    }
    app.resolve_and_repaint(800.0, 600.0);
    Rig { app, scroller }
}

fn scroll(app: &RinchApp, id: u64) -> f64 {
    app.doc.as_ref().unwrap().borrow().tree.nodes[id]
        .scroll_offset
        .0
}

fn track(app: &RinchApp, id: u64) -> rinch_dom::paint::scrollbar::ScrollbarTrack {
    let d = app.doc.as_ref().unwrap().borrow();
    rinch_dom::paint::scrollbar::scrollbars(&d.tree, id, 1.0)
        .horizontal
        .expect("a horizontal bar")
}

/// The window point `along` px across the bar, centred on its thickness.
fn on_bar(along: f64) -> (f32, f32) {
    (along as f32, HEIGHT - (MARGIN + THICKNESS / 2.0) as f32)
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
fn the_thumb_over_draggable_content_is_grabbed_and_dragged() {
    let mut r = rig();
    let start = scroll(&r.app, r.scroller);
    assert!(start > 0.0, "premise: the scroller is scrolled part-way");
    let t = track(&r.app, r.scroller);
    let grab = on_bar(t.thumb_start(start) + t.thumb_len / 2.0);

    press(&mut r.app, grab);
    assert!(
        r.app.scrollbar_drag.is_some(),
        "the press on the thumb grabs it"
    );
    assert!(
        r.app.pending_drag.is_none(),
        "the press armed no drag of a column"
    );

    move_to(&mut r.app, (grab.0 + 40.0, grab.1));
    r.app.resolve_and_repaint(800.0, 600.0);
    let moved = scroll(&r.app, r.scroller);
    assert!(moved > start, "the drag scrolled: {start} -> {moved}");
    release(&mut r.app, (grab.0 + 40.0, grab.1));
    assert!(r.app.scrollbar_drag.is_none(), "the release ends the drag");
}

#[test]
fn a_press_on_the_track_beside_draggable_content_jumps() {
    let mut r = rig();
    let start = scroll(&r.app, r.scroller);
    let t = track(&r.app, r.scroller);
    let beyond = t.thumb_start(start) + t.thumb_len + 20.0;
    assert!(
        beyond < t.track_start + t.track_len,
        "premise: there is track past the thumb"
    );

    press(&mut r.app, on_bar(beyond));
    r.app.resolve_and_repaint(800.0, 600.0);
    assert!(
        scroll(&r.app, r.scroller) > start,
        "the track press scrolled toward it"
    );
    assert!(
        r.app.pending_drag.is_none(),
        "the press armed no drag of a column"
    );
}

#[test]
fn a_press_on_a_column_still_arms_its_drag() {
    let mut r = rig();
    press(&mut r.app, (60.0, 80.0));
    assert!(
        r.app.scrollbar_drag.is_none(),
        "control: the column is not the scrollbar"
    );
    assert!(
        r.app.pending_drag.is_some(),
        "control: a press on a column arms its drag"
    );
}
