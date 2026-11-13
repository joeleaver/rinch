//! #2546: a `direction: rtl` scroll container scrolls from its right edge, as
//! Chrome does — `scrollLeft` is 0 at the start and runs negative toward the
//! left — through every desktop writer of the offset: the wheel, the
//! scrollbar (thumb, drag, track press), `scroll_into_view`, and the
//! `onscroll` payload. rinch-web reads the browser's own `scrollLeft`, which
//! runs the same way, so the values agree across backends.
//!
//! The scroller is 200x100 at the document origin, `overflow-x: auto;
//! direction: rtl`, holding an 800px row: its range is 600, so its offsets
//! run `-600..=0`. Every fixture is scrolled off 0 or checks both ends, so a
//! writer still clamping to `0..=600` cannot pass by sitting on 0.

use super::*;
use rinch_core::events::{ScrollCallback, ScrollEvent, register_scroll_handler};
use rinch_dom::paint::scrollbar::{MARGIN, THICKNESS};
use std::cell::{Cell, RefCell};

const SIZE: (u32, u32) = (800, 600);
const RANGE: f64 = 600.0;

/// The scroller (its content `inner` html), and the `ScrollEvent`s its
/// `onscroll` heard.
fn mount(
    inner: &'static str,
    extra: &'static str,
) -> (RinchApp, u64, Rc<RefCell<Vec<ScrollEvent>>>) {
    mount_at(inner, extra, 1.0)
}

/// [`mount`] at a device pixel ratio of `scale`.
fn mount_at(
    inner: &'static str,
    extra: &'static str,
    scale: f64,
) -> (RinchApp, u64, Rc<RefCell<Vec<ScrollEvent>>>) {
    let heard: Rc<RefCell<Vec<ScrollEvent>>> = Rc::new(RefCell::new(Vec::new()));
    let heard_in = heard.clone();
    let handler = register_scroll_handler(ScrollCallback::from(move |e: ScrollEvent| {
        heard_in.borrow_mut().push(e);
    }));
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "position: relative; width: 800px; height: 600px");
        let scroller = scope.create_element("div");
        scroller.set_attribute(
            "style",
            &format!(
                "position: absolute; left: 0px; top: 0px; width: 200px; height: 100px; \
                 overflow-x: auto; overflow-y: hidden; direction: rtl; {extra}"
            ),
        );
        scroller.set_attribute("id", "scroller");
        scroller.set_attribute("data-onscroll", &handler.0.to_string());
        root.append_child(&scroller);
        root
    });
    app.set_device_pixel_ratio(scale);
    app.mount_component(800.0, 600.0);
    let id = {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        let id = d
            .tree
            .nodes
            .iter()
            .find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some("scroller"))
            .map(|(id, _)| id)
            .unwrap();
        d.set_inner_html(rinch_core::dom::NodeId(id), inner);
        id
    };
    app.resolve_and_repaint(800.0, 600.0);
    (app, id, heard)
}

fn mount_row() -> (RinchApp, u64, Rc<RefCell<Vec<ScrollEvent>>>) {
    mount(r#"<div style="width: 800px; height: 40px"></div>"#, "")
}

fn left(app: &RinchApp, id: u64) -> f64 {
    app.doc.as_ref().unwrap().borrow().tree.nodes[id]
        .scroll_offset
        .0
}

fn set_left(app: &RinchApp, id: u64, x: f64) {
    let doc = app.doc.as_ref().unwrap();
    let mut d = doc.borrow_mut();
    d.tree.nodes[id].scroll_offset.0 = x;
    d.tree.mark_scrolled(id);
}

/// A pixel wheel over the scroller, with winit's sign: a positive `delta_x`
/// scrolls the content toward the left (DOM `-delta_x`).
fn wheel(app: &mut RinchApp, delta_x: f64) {
    app.handle_event(
        PlatformEvent::MouseWheel {
            x: 100.0,
            y: 50.0,
            delta_x,
            delta_y: 0.0,
        },
        SIZE,
        1.0,
    );
    app.resolve_and_repaint(800.0, 600.0);
}

fn track(app: &RinchApp, id: u64) -> rinch_dom::paint::scrollbar::ScrollbarTrack {
    let d = app.doc.as_ref().unwrap().borrow();
    rinch_dom::paint::scrollbar::scrollbars(&d.tree, id, 1.0)
        .horizontal
        .expect("a horizontal bar")
}

/// The window point `along` px across the horizontal bar.
fn on_bar(along: f64) -> (f32, f32) {
    (along as f32, (100.0 - MARGIN - THICKNESS / 2.0) as f32)
}

fn button(app: &mut RinchApp, (x, y): (f32, f32), down: bool) {
    let button = rinch_platform::MouseButton::Left;
    let event = if down {
        PlatformEvent::MouseDown { x, y, button }
    } else {
        PlatformEvent::MouseUp { x, y, button }
    };
    app.handle_event(event, SIZE, 1.0);
}

/// The wheel moves an rtl scroller into negative offsets and clamps at both
/// ends, `-600` and `0`; `onscroll` reports Chrome's negative `scrollLeft`.
#[test]
fn the_wheel_scrolls_an_rtl_scroller_into_negative_offsets() {
    let (mut app, id, heard) = mount_row();
    assert_eq!(left(&app, id), 0.0, "it starts at 0, its right end showing");
    {
        let d = app.doc.as_ref().unwrap().borrow();
        assert!(!rinch_dom::paint::scrollbar::can_scroll_toward(
            &d.tree, id, 1.0, 0.0
        ));
        assert!(rinch_dom::paint::scrollbar::can_scroll_toward(
            &d.tree, id, -1.0, 0.0
        ));
    }
    wheel(&mut app, -50.0);
    assert_eq!(left(&app, id), 0.0, "nothing to the right of the start");
    wheel(&mut app, 70.0);
    assert_eq!(left(&app, id), -70.0);
    assert_eq!(
        heard.borrow().last().map(|e| e.scroll_left),
        Some(-70.0),
        "onscroll hears Chrome's scrollLeft"
    );
    wheel(&mut app, 5000.0);
    assert_eq!(left(&app, id), -RANGE, "clamped at the far (left) end");
    wheel(&mut app, -30.0);
    assert_eq!(left(&app, id), -RANGE + 30.0);
    wheel(&mut app, -5000.0);
    assert_eq!(left(&app, id), 0.0, "clamped back at the start");
}

/// The thumb sits at the right at the start and at the left at the far end;
/// a drag and a track press write negative offsets.
#[test]
fn the_rtl_thumb_starts_at_the_right_and_drags_into_negative_offsets() {
    let (mut app, id, _) = mount_row();
    let t = track(&app, id);
    assert_eq!(t.min_scroll, -RANGE);
    assert_eq!(
        t.thumb_start(0.0),
        t.track_start + t.thumb_travel,
        "right end"
    );
    assert_eq!(t.thumb_start(-RANGE), t.track_start, "left end");

    // Grab the thumb mid-way along it and drag it 30px left.
    let grab = t.thumb_start(0.0) + t.thumb_len / 2.0;
    button(&mut app, on_bar(grab), true);
    assert_eq!(left(&app, id), 0.0, "a grab moves nothing");
    app.handle_event(
        PlatformEvent::MouseMove {
            x: on_bar(grab - 30.0).0,
            y: on_bar(grab).1,
        },
        SIZE,
        1.0,
    );
    let want = -30.0 * RANGE / t.thumb_travel;
    assert!(
        (left(&app, id) - want).abs() < 1.0,
        "dragged to {} (want {want})",
        left(&app, id)
    );
    button(&mut app, on_bar(grab - 30.0), false);

    // A press on the empty track at the left end jumps there.
    button(&mut app, on_bar(t.track_start + 1.0), true);
    button(&mut app, on_bar(t.track_start + 1.0), false);
    assert!(
        left(&app, id) < -RANGE + 10.0,
        "jumped to {}",
        left(&app, id)
    );
}

/// `scroll_into_view`'s `nearest` in an rtl flex row of 120, 120, 120 and
/// 300px items, from Chrome 157 (`scrollIntoView({inline: 'nearest'})`):
/// from 0, `t1` → -40, `t2` → -160, `t3` (wider than the scrollport,
/// aligned to its start, the right) → -360; from -500, `t1` → -120.
#[test]
fn scroll_into_view_reveals_in_an_rtl_scroller_as_chrome() {
    let (mut app, id, _) = mount(
        r#"<div style="width:120px;height:20px;flex-shrink:0"></div><div id="t1" style="width:120px;height:20px;flex-shrink:0"></div><div id="t2" style="width:120px;height:20px;flex-shrink:0"></div><div id="t3" style="width:300px;height:20px;flex-shrink:0"></div>"#,
        "display: flex",
    );
    let target = |app: &RinchApp, t: &str| {
        let d = app.doc.as_ref().unwrap().borrow();
        d.tree
            .nodes
            .iter()
            .find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some(t))
            .map(|(id, _)| id)
            .unwrap()
    };
    let mut reveal = |from: f64, t: &str| {
        set_left(&app, id, from);
        let t = target(&app, t);
        {
            let doc = app.doc.as_ref().unwrap();
            let mut d = doc.borrow_mut();
            d.request_scroll_into_view(rinch_core::dom::NodeId(t));
            d.tree.dirty_nodes.insert(t);
        }
        app.resolve_and_repaint(800.0, 600.0);
        left(&app, id)
    };
    let got = [
        reveal(0.0, "t1"),
        reveal(0.0, "t2"),
        reveal(0.0, "t3"),
        reveal(-500.0, "t1"),
    ];
    assert_eq!(got, [-40.0, -160.0, -360.0, -120.0]);
}

/// The onscroll count moves with each wheel write, so the payload pinned
/// above is the write's own.
#[test]
fn each_rtl_wheel_write_fires_onscroll_once() {
    let (mut app, _, heard) = mount_row();
    let fired = Cell::new(heard.borrow().len());
    for dx in [40.0, 40.0] {
        wheel(&mut app, dx);
        assert_eq!(heard.borrow().len(), fired.get() + 1);
        fired.set(heard.borrow().len());
    }
    assert_eq!(heard.borrow().last().unwrap().scroll_left, -80.0);
}

/// At a fractional scale the offset is on the device grid, rounded toward 0
/// (#1543): revealing a target to the left rounds the other way, so the
/// target is wholly in view rather than up to a device pixel short of it.
/// `t1` needs -41 here (its right neighbour is 121px), which is off the 1.5
/// grid: the grid point past it is -41.333, and rounding toward 0 stops at
/// -40.667, a third of a pixel short.
#[test]
fn an_rtl_reveal_at_a_fractional_scale_reaches_the_whole_target() {
    let (mut app, id, _) = mount_at(
        r#"<div style="width:121px;height:20px;flex-shrink:0"></div><div id="t1" style="width:120px;height:20px;flex-shrink:0"></div><div style="width:300px;height:20px;flex-shrink:0"></div>"#,
        "display: flex",
        1.5,
    );
    let t = {
        let d = app.doc.as_ref().unwrap().borrow();
        d.tree
            .nodes
            .iter()
            .find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some("t1"))
            .map(|(id, _)| id)
            .unwrap()
    };
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.request_scroll_into_view(rinch_core::dom::NodeId(t));
        d.tree.dirty_nodes.insert(t);
    }
    app.resolve_and_repaint(800.0, 600.0);
    let x = left(&app, id);
    assert!(x <= -41.0, "{x}: the target's left edge is still hidden");
    assert!(x > -41.5, "{x}: no further than the grid point past it");
    assert!(
        ((x * 1.5).round() - x * 1.5).abs() < 1e-6,
        "{x} is on the grid"
    );
}
