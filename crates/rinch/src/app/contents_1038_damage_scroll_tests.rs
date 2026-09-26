//! A `display: contents` element in the damage walks and the scroll finders —
//! issue #1038, from the review of PR #1062.
//!
//! K1–K4 pin the damage clip chain and `layer_bounds` asking
//! `Node::box_position` rather than a contents wrapper's computed `position`:
//! reading the computed value only *widens* damage, which no pixel oracle sees —
//! but the `RepaintedPx` / `RepaintFull` counters do. K5 pins
//! `position_and_transform_in`'s transformed path.
//!
//! P1–P3 pin `Node::scrolls_y`/`scrolls_x`: `overflow` on a contents element
//! makes nothing scrollable (Chrome 153), so the wheel reaches the real
//! scroller above it and no scrollbar is drawn for it.

use super::*;
use rinch_dom::perf::{Counter, FrameStats};

const SIZE: (u32, u32) = (600, 400);

fn full_frame(app: &mut RinchApp) -> Vec<u8> {
    app.scene_dirty = true;
    app.has_previous_frame = false;
    let px = app.build_pixels(1.0, SIZE, false).0.to_vec();
    let _ = app.end_perf_frame();
    px
}

fn incremental_frame(app: &mut RinchApp) -> (Vec<u8>, FrameStats) {
    let px = app.build_pixels(1.0, SIZE, false).0.to_vec();
    let stats = app.end_perf_frame().expect("mounted");
    (px, stats)
}

fn diff(a: &[u8], b: &[u8]) -> usize {
    a.chunks(4)
        .zip(b.chunks(4))
        .filter(|(p, q)| (0..4).any(|k| (p[k] as i32 - q[k] as i32).abs() > 2))
        .count()
}

/// outer(600x400; `outer_style`) > clipper(20,20 200x100; `clipper_style`)
/// > wrapper(`wrapper_style`) > box(40x40 blue; `box_style`).
fn mount(
    outer_style: &'static str,
    clipper_style: &'static str,
    wrapper_style: &'static str,
    box_style: &'static str,
) -> (RinchApp, NodeHandle, NodeHandle) {
    type Slot = Option<(NodeHandle, NodeHandle)>;
    let slot: Rc<RefCell<Slot>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let outer = scope.create_element("div");
        outer.set_attribute(
            "style",
            &format!("width: 600px; height: 400px; {outer_style}"),
        );
        let clipper = scope.create_element("div");
        clipper.set_attribute(
            "style",
            &format!(
                "margin-left: 20px; margin-top: 20px; width: 200px; height: 100px; {clipper_style}"
            ),
        );
        outer.append_child(&clipper);
        let w = scope.create_element("div");
        w.set_attribute("style", wrapper_style);
        clipper.append_child(&w);
        let b = scope.create_element("div");
        b.set_attribute(
            "style",
            &format!("width: 40px; height: 40px; background: rgb(0, 0, 200); {box_style}"),
        );
        w.append_child(&b);
        *slot_in.borrow_mut() = Some((clipper, b));
        outer
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    let (c, b) = slot.borrow().clone().unwrap();
    (app, c, b)
}

fn step(app: &mut RinchApp) -> FrameStats {
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    let (inc, stats) = incremental_frame(app);
    let full = full_frame(app);
    assert_eq!(diff(&inc, &full), 0, "incremental != full");
    stats
}

/// K1: the box moves from on screen to below the clipper, inside a
/// `contents; position: fixed` wrapper. Its new rect is clipped away, so
/// only the old 40x40 is damage. Kills: `PaintedStyle::now` reading the
/// wrapper's computed `position` (the chain breaks at the wrapper and the
/// new rect is named unclipped).
#[test]
fn k1_new_rect_under_a_fixed_contents_wrapper_is_clipped() {
    let (mut app, _c, b) = mount(
        "",
        "overflow: hidden",
        "display: contents; position: fixed",
        "margin-top: 10px",
    );
    let _ = full_frame(&mut app);
    b.set_style("margin-top", "300px");
    let stats = step(&mut app);
    assert_eq!(stats.get(Counter::RepaintFull), 0, "{stats:?}");
    let px = stats.get(Counter::RepaintedPx);
    assert!(
        px <= 48 * 48,
        "repainted {px} px: damage reaches past the clip"
    );
}

/// K2: the reverse move. The *old* rect is the clipped-away one, asked
/// through the painted state. Kills: `PaintedState::of` recording the
/// wrapper's computed `position`.
#[test]
fn k2_old_rect_under_a_fixed_contents_wrapper_is_clipped() {
    let (mut app, _c, b) = mount(
        "",
        "overflow: hidden",
        "display: contents; position: fixed",
        "margin-top: 300px",
    );
    let _ = full_frame(&mut app);
    b.set_style("margin-top", "10px");
    let stats = step(&mut app);
    assert_eq!(stats.get(Counter::RepaintFull), 0, "{stats:?}");
    let px = stats.get(Counter::RepaintedPx);
    assert!(
        px <= 48 * 48,
        "repainted {px} px: damage reaches past the clip"
    );
}

/// K3/K4: a moved `position: relative` clipper carries its subtree reach
/// (`opacity_layer_bounds`). A contents wrapper that is `fixed`/`sticky` only
/// in its computed style must not make that walk answer "unbounded" (a full
/// repaint). Kills the `layer_bounds` `Fixed` / `Sticky` mutants.
#[test]
fn k3_k4_a_moved_box_over_a_positioned_contents_wrapper_repaints_partially() {
    for wrapper in [
        "display: contents; position: fixed",
        "display: contents; position: sticky",
    ] {
        let (mut app, clipper, _b) = mount("", "position: relative", wrapper, "");
        let _ = full_frame(&mut app);
        clipper.set_style("left", "30px");
        let stats = step(&mut app);
        assert_eq!(
            stats.get(Counter::RepaintFull),
            0,
            "`{wrapper}`: full repaint: {stats:?}"
        );
    }
    // Positive control: a real fixed descendant does answer unbounded.
    let (mut app, clipper, _b) = mount("", "position: relative", "position: fixed", "");
    let _ = full_frame(&mut app);
    clipper.set_style("left", "30px");
    let stats = step(&mut app);
    assert_eq!(stats.get(Counter::RepaintFull), 1, "control: {stats:?}");
}

/// P1: `overflow: auto` on a contents element makes nothing scrollable in
/// Chrome; the wheel over its child must reach the real scroller above.
#[test]
fn p1_wheel_through_an_overflow_auto_contents_wrapper() {
    let (app, clipper, b) = mount(
        "",
        "overflow: auto",
        "display: contents; overflow: auto",
        "height: 300px",
    );
    let doc = app.doc.as_ref().unwrap().borrow();
    let found = super::hit_testing::find_scroll_container(&doc.tree, b.node_id().0);
    assert_eq!(found, Some(clipper.node_id().0), "wheel target");
}

/// P2: no scrollbar on a contents element.
#[test]
fn p2_no_scrollbar_on_an_overflow_auto_contents_wrapper() {
    let (app, _c, b) = mount("", "", "display: contents; overflow: auto", "height: 300px");
    let doc = app.doc.as_ref().unwrap().borrow();
    let w = doc.tree.get(b.node_id().0).unwrap().parent.unwrap();
    let bars = rinch_dom::paint::scrollbar::scrollbars(&doc.tree, w, 1.0);
    assert!(
        bars.vertical.is_none() && bars.horizontal.is_none(),
        "a contents element paints a bar"
    );
}

/// P3: the whole wheel path. A real `overflow: auto` scroller holds a
/// `contents; overflow: auto` wrapper around a 300px child; a notch over the
/// child must scroll the real scroller (Chrome 153: `overflow` does not apply
/// to a contents element, so it is not a scroll container).
#[test]
fn p3_a_wheel_over_a_child_of_an_overflow_auto_contents_wrapper_scrolls_the_scroller() {
    let (mut app, clipper, b) = mount(
        "",
        "overflow: auto",
        "display: contents; overflow: auto",
        "height: 300px",
    );
    let hit = {
        let doc = app.doc.as_ref().unwrap().borrow();
        super::hit_testing::hit_test(&doc.tree, 60.0, 50.0)
    };
    eprintln!(
        "hit={hit:?} box={} clipper={}",
        b.node_id().0,
        clipper.node_id().0
    );
    app.handle_event(
        PlatformEvent::MouseWheel {
            x: 60.0,
            y: 50.0,
            delta_x: 0.0,
            delta_y: -100.0,
        },
        (SIZE.0, SIZE.1),
        1.0,
    );
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    let px = full_frame(&mut app);
    let blue = |x: usize, y: usize| {
        px[(y * SIZE.0 as usize + x) * 4 + 2] > 150 && px[(y * SIZE.0 as usize + x) * 4] < 50
    };
    let doc = app.doc.as_ref().unwrap().borrow();
    eprintln!(
        "after wheel: hit(60,50)={:?} painted blue at (30,30)={} abs={:?}",
        super::hit_testing::hit_test(&doc.tree, 60.0, 50.0),
        blue(30, 30),
        rinch_dom::paint::compute_absolute_position(&doc.tree, b.node_id().0, 1.0)
    );
    let w = doc.tree.get(b.node_id().0).unwrap().parent.unwrap();
    eprintln!(
        "scroller scroll_top={} wrapper scroll_offset={:?}",
        doc.scroll_top(rinch_core::dom::NodeId(clipper.node_id().0)),
        doc.tree.get(w).unwrap().scroll_offset
    );
    assert_eq!(
        doc.scroll_top(rinch_core::dom::NodeId(clipper.node_id().0)),
        100.0,
        "the real scroller scrolls"
    );
}

/// K5: a *transformed* ancestor above a `contents; position: fixed` wrapper.
/// The wrapper anchors nothing, so the child's painted position carries the
/// ancestor's translate. Kills `position_and_transform_in` reading the
/// computed `position` in its first (transform-detection) loop or in its
/// transformed-chain loop — the no-transform fixtures cannot see either.
#[test]
fn k5_a_transform_above_a_fixed_contents_wrapper_still_moves_its_child() {
    let (app, _c, b) = mount(
        "",
        "transform: translateX(50px)",
        "display: contents; position: fixed",
        "",
    );
    let doc = app.doc.as_ref().unwrap().borrow();
    let (x, y, t) =
        rinch_dom::paint::compute_absolute_position_and_transform(&doc.tree, b.node_id().0, 1.0);
    let p = t * peniko::kurbo::Point::new(x, y);
    assert_eq!((p.x, p.y), (70.0, 20.0), "painted origin of the child");
    // And the pixel agrees: blue at (75, 25), nothing at (25, 25).
    drop(doc);
}

/// P4: `scroll_into_view` of a box below the fold of a real scroller, reached
/// through a `contents; overflow: auto` wrapper. The nearest scroll container
/// is the real scroller (Chrome 153: `overflow` does not apply to a contents
/// element); the wrapper used to be picked, and a `scroll_offset` written
/// onto a box-less node moved nothing on screen.
#[test]
fn p4_scroll_into_view_through_an_overflow_auto_contents_wrapper_scrolls_the_scroller() {
    let (mut app, clipper, b) = mount(
        "",
        "overflow: auto",
        "display: contents; overflow: auto",
        "margin-top: 200px",
    );
    b.scroll_into_view();
    // A style write, so the frame runs a layout pass (the scroll is applied
    // right after it).
    b.set_style("outline", "1px solid red");
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    let doc = app.doc.as_ref().unwrap().borrow();
    let w = doc.tree.get(b.node_id().0).unwrap().parent.unwrap();
    assert_eq!(
        doc.tree.get(w).unwrap().scroll_offset,
        (0.0, 0.0),
        "nothing is written onto the box-less wrapper"
    );
    assert_eq!(
        doc.scroll_top(rinch_core::dom::NodeId(clipper.node_id().0)),
        140.0,
        "the real scroller brings the 40px box at y 200 to its 100px bottom"
    );
}
