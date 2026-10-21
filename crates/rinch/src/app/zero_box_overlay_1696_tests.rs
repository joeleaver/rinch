//! The `Editor` overlay slot (#1677) paints its children (#1696).
//!
//! The slot is a `0x0` stacking context (`position: absolute; width: 0;
//! height: 0; z-index: 2`), and desktop paint skipped every descendant of a
//! box zero in both dimensions: a marker in it was laid out and hit-tested at
//! its box, and its pixels were the page's. These drive the real `Editor`
//! component through the frames the shell runs, read the software painter's
//! pixels at the marker's painted box, and move the marker to check the
//! incremental frame repaints both where it went and where it was.

use super::hit_testing::{hit_test, painted_element_box};
use super::*;
use rinch_core::Component;

const VP: (u32, u32) = (600, 400);

/// An editor with one paragraph and a 16x12 green marker in its overlay slot
/// at `left: 200px; top: 20px` (of the slot), on a white page.
fn page() -> (RinchApp, Rc<RefCell<Option<NodeHandle>>>) {
    let handle = crate::editor::create_editor();
    assert!(handle.load_html("<p>text</p>"));
    let marker_out: Rc<RefCell<Option<NodeHandle>>> = Rc::default();
    let out = marker_out.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "padding: 30px 0 0 40px; background-color: rgb(255, 255, 255); height: 300px",
        );
        let marker = scope.create_element("div");
        marker.set_attribute("data-marker", "");
        marker.set_attribute(
            "style",
            "position: absolute; left: 200px; top: 20px; width: 16px; height: 12px; \
             background-color: rgb(0, 128, 0)",
        );
        *out.borrow_mut() = Some(marker.clone());
        let editor = crate::editor::Editor {
            editor: Some(handle.clone()),
            ..Default::default()
        }
        .render(scope, &[marker]);
        editor.set_attribute("style", "width: 300px; line-height: 24px");
        root.append_child(&editor);
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    for _ in 0..3 {
        frame(&mut app);
    }
    (app, marker_out)
}

/// One redraw, as the shell performs it: an `AboutToWait` turn, the layout,
/// the software pixels.
fn frame(app: &mut RinchApp) -> Vec<u8> {
    frame_with_stats(app).0
}

fn frame_with_stats(app: &mut RinchApp) -> (Vec<u8>, rinch_dom::perf::FrameStats) {
    let _ = app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let px = app.build_pixels(1.0, VP, false).0.to_vec();
    (px, app.end_perf_frame().expect("mounted"))
}

fn marker_box(app: &RinchApp) -> (f32, f32, f32, f32) {
    let doc = app.doc.as_ref().unwrap().borrow();
    let id = doc.query_selector_all("[data-marker]")[0];
    painted_element_box(&doc.tree, id.0)
}

/// Pixels in `(x, y, w, h)` that are exactly opaque `rgb(0, 128, 0)`.
fn green_in(px: &[u8], (x, y, w, h): (f32, f32, f32, f32)) -> usize {
    let mut n = 0;
    for yy in y as u32..(y + h) as u32 {
        for xx in x as u32..(x + w) as u32 {
            let i = ((yy * VP.0 + xx) * 4) as usize;
            if px[i..i + 4] == [0, 128, 0, 255] {
                n += 1;
            }
        }
    }
    n
}

/// The slot's child is drawn at its box, whole, and is what the pointer hits
/// there.
#[test]
fn a_child_of_the_overlay_slot_paints_where_it_is_laid_out() {
    let (mut app, _) = page();
    {
        let doc = app.doc.as_ref().unwrap().borrow();
        let slot = doc.query_selector_all("[data-pm-overlay]")[0];
        let l = doc.tree.get(slot.0).unwrap().layout;
        assert_eq!(
            (l.width, l.height),
            (0.0, 0.0),
            "precondition: the slot is 0x0"
        );
    }
    let b = marker_box(&app);
    assert_eq!((b.2, b.3), (16.0, 12.0), "precondition: {b:?}");
    let px = frame(&mut app);
    assert_eq!(green_in(&px, b), 16 * 12, "the marker is drawn at {b:?}");
    let doc = app.doc.as_ref().unwrap().borrow();
    let marker = doc.query_selector_all("[data-marker]")[0];
    assert_eq!(
        hit_test(&doc.tree, b.0 + 8.0, b.1 + 6.0),
        Some(marker.0),
        "and hit there"
    );
}

/// Moving the child repaints it where it went and clears where it was, on
/// the incremental frame (not a full repaint).
#[test]
fn moving_a_child_of_the_overlay_slot_repaints_both_places() {
    let (mut app, marker) = page();
    let before = marker_box(&app);
    let px = frame(&mut app);
    assert_eq!(green_in(&px, before), 16 * 12, "drawn before the move");
    marker.borrow().as_ref().unwrap().set_style("top", "90px");
    let (px, stats) = frame_with_stats(&mut app);
    let after = marker_box(&app);
    assert_eq!(after.1, before.1 + 70.0, "precondition: it moved");
    assert_eq!(
        (
            stats.get(rinch_dom::perf::Counter::RepaintPartial),
            stats.get(rinch_dom::perf::Counter::RepaintFull)
        ),
        (1, 0),
        "precondition: a partial repaint, so the damage is what is tested"
    );
    assert_eq!(green_in(&px, after), 16 * 12, "drawn where it went");
    assert_eq!(green_in(&px, before), 0, "cleared where it was");
}
