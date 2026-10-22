//! A press on the thumb drawn over a scroller's `position: relative` row grabs
//! the thumb, not the row (#1717).
//!
//! Paint now draws a scroller's bars after its hoisted positioned rows; the
//! press must agree with what is drawn on top. `find_scrollbar_hit` walks the
//! tree for a bar before any click is resolved, so the row under the thumb gets
//! no click; this pins that against the shape paint changed for, scrolled off
//! 0 and pressed off the thumb's centre (`scrollbar_thumb_press_443_tests`).

use super::*;
use rinch_dom::paint::scrollbar::{MARGIN, THICKNESS};
use std::cell::Cell;
use std::rc::Rc;

const SIZE: (u32, u32) = (800, 600);

#[test]
fn a_press_on_the_thumb_over_a_relative_row_grabs_the_thumb() {
    let clicks = Rc::new(Cell::new(0u32));
    let counter = clicks.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "position: relative; width: 800px; height: 600px");
        let scroller = scope.create_element("div");
        scroller.set_attribute(
            "style",
            "position: absolute; left: 0px; top: 0px; width: 200px; height: 100px; \
             overflow-y: auto",
        );
        scroller.set_attribute("id", "scroller");
        for _ in 0..10 {
            let row = scope.create_element("div");
            row.set_attribute(
                "style",
                "position: relative; height: 40px; background-color: rgb(255, 0, 0)",
            );
            let c = counter.clone();
            let h = scope.register_handler(move || c.set(c.get() + 1));
            row.set_attribute("data-rid", &h.0.to_string());
            scroller.append_child(&row);
        }
        root.append_child(&scroller);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let id = {
        let d = app.doc.as_ref().unwrap().borrow();
        d.tree
            .nodes
            .iter()
            .find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some("scroller"))
            .map(|(id, _)| id)
            .unwrap()
    };
    app.handle_event(
        PlatformEvent::MouseWheel {
            x: 100.0,
            y: 50.0,
            delta_x: 0.0,
            delta_y: -90.0,
        },
        SIZE,
        1.0,
    );
    app.resolve_and_repaint(800.0, 600.0);
    let scroll = |app: &RinchApp| {
        app.doc.as_ref().unwrap().borrow().tree.nodes[id]
            .scroll_offset
            .1
    };
    let s = scroll(&app);
    let t = {
        let d = app.doc.as_ref().unwrap().borrow();
        rinch_dom::paint::scrollbar::scrollbars(&d.tree, id, 1.0)
            .vertical
            .expect("a vertical bar")
    };
    assert!(s > 0.0 && s < t.max_scroll, "scrolled part-way: {s}");
    let along = t.thumb_start(s) + t.thumb_len * 0.7;
    let x = (200.0 - MARGIN - THICKNESS / 2.0) as f32;

    // The premise: the point is over a relative row, which a plain hit test
    // answers.
    {
        let d = app.doc.as_ref().unwrap().borrow();
        let hit = app.shared_hit(&d, x, along as f32).expect("a hit");
        assert_ne!(hit, id, "the point is over a row, not the bare scroller");
    }

    app.handle_event(
        PlatformEvent::MouseDown {
            x,
            y: along as f32,
            button: rinch_platform::MouseButton::Left,
        },
        SIZE,
        1.0,
    );
    assert!(app.scrollbar_drag.is_some(), "the press grabs the thumb");
    assert_eq!(scroll(&app), s, "a thumb press does not jump");
    app.handle_event(
        PlatformEvent::MouseMove {
            x,
            y: along as f32 + 10.0,
        },
        SIZE,
        1.0,
    );
    app.handle_event(
        PlatformEvent::MouseUp {
            x,
            y: along as f32 + 10.0,
            button: rinch_platform::MouseButton::Left,
        },
        SIZE,
        1.0,
    );
    assert!(scroll(&app) > s, "the drag scrolled");
    assert_eq!(clicks.get(), 0, "the row under the thumb got no click");
}
