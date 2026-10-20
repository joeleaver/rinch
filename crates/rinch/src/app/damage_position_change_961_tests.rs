//! A box whose `position` or containing block changed this frame clears the
//! pixels it was painted at (#961).
//!
//! The damage for a node's old pixels is placed by `Frame::Painted`
//! (`paint::previous_painted_rect`, `paint::clip_chain_bounds`). It used to
//! decide "this box is a hoisted `position: fixed` box" from the node's
//! **current** `position`, so a box that went static → fixed had its old
//! in-flow rect computed as a viewport rect, and one that went fixed → static
//! had its old fixed rect computed as an in-flow one; both left the old pixels
//! on screen. Likewise a box that became a containing block: its absolute
//! descendants' old pixels were placed and clipped as if it had always been
//! one.
//!
//! Local pixel oracles in `damage_clip_chain_tests`' idiom: the incremental
//! frame must equal a from-scratch frame of the same state, a positive control
//! says the old pixels were really there, and a counter check says the frame
//! really was incremental (a full repaint passes every pixel assertion
//! vacuously). Each fixture is off the fixed point where old and new places
//! coincide: the box moves by hundreds of pixels.

use super::*;
use rinch_dom::perf::{Counter, FrameStats};

const SIZE: (u32, u32) = (600, 400);
const WHOLE: (i32, i32, i32, i32) = (0, 0, 600, 400);
const BLUE: &str = "width: 40px; height: 40px; background: rgb(0, 0, 200); flex-shrink: 0;";

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

fn diff_in(a: &[u8], b: &[u8], rect: (i32, i32, i32, i32)) -> usize {
    let (w, h) = (SIZE.0 as i32, SIZE.1 as i32);
    let mut n = 0;
    for y in rect.1.max(0)..rect.3.min(h) {
        for x in rect.0.max(0)..rect.2.min(w) {
            let i = ((y * w + x) * 4) as usize;
            if (0..4).any(|k| (a[i + k] as i32 - b[i + k] as i32).abs() > 2) {
                n += 1;
            }
        }
    }
    n
}

fn ink_in(px: &[u8], rect: (i32, i32, i32, i32)) -> usize {
    let (w, h) = (SIZE.0 as i32, SIZE.1 as i32);
    let mut n = 0;
    for y in rect.1.max(0)..rect.3.min(h) {
        for x in rect.0.max(0)..rect.2.min(w) {
            let i = ((y * w + x) * 4) as usize;
            if px[i] < 250 || px[i + 1] < 250 || px[i + 2] < 250 {
                n += 1;
            }
        }
    }
    n
}

fn resolve(app: &mut RinchApp) {
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
}

fn el(scope: &mut RenderScope, parent: &NodeHandle, style: &str) -> NodeHandle {
    let e = scope.create_element("div");
    e.set_attribute("style", style);
    parent.append_child(&e);
    e
}

type Build = Box<dyn Fn(&mut RenderScope, &NodeHandle) -> Vec<NodeHandle>>;

fn mount(build: Build) -> (RinchApp, Vec<NodeHandle>) {
    let slot: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(vec![]));
    let s2 = slot.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let outer = scope.create_element("div");
        outer.set_attribute("style", "width: 600px; height: 400px;");
        *s2.borrow_mut() = build(scope, &outer);
        outer
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    resolve(&mut app);
    let v = slot.borrow().clone();
    (app, v)
}

/// The next frame equals a from-scratch one and left nothing at `old`; with
/// `incremental`, it was a partial repaint.
fn check(app: &mut RinchApp, label: &str, old: (i32, i32, i32, i32), incremental: bool) {
    let (inc, stats) = incremental_frame(app);
    if incremental {
        assert_eq!(stats.get(Counter::RepaintFull), 0, "{label}: {stats:?}");
        assert_eq!(stats.get(Counter::RepaintPartial), 1, "{label}: {stats:?}");
    }
    let full = full_frame(app);
    assert_eq!(ink_in(&full, old), 0, "{label}: the old place is empty now");
    assert_eq!(ink_in(&inc, old), 0, "{label}: ghost at the old place");
    assert_eq!(diff_in(&inc, &full, WHOLE), 0, "{label}: incremental != full");
}

/// static → fixed: the old in-flow rect inside a clipper is cleared.
#[test]
fn a_box_that_becomes_fixed_clears_its_in_flow_pixels() {
    let (mut app, h) = mount(Box::new(|s, o| {
        let c = el(
            s,
            o,
            "width: 200px; height: 100px; overflow: hidden; margin-left: 50px;",
        );
        let r = el(s, &c, BLUE);
        vec![c, r]
    }));
    let old = (50, 0, 90, 40);
    let before = full_frame(&mut app);
    assert!(ink_in(&before, old) > 1000, "positive control");
    h[1].set_style("position", "fixed");
    h[1].set_style("left", "400px");
    h[1].set_style("top", "300px");
    resolve(&mut app);
    check(&mut app, "static -> fixed", old, true);
}

/// fixed → static: the old viewport rect is cleared.
#[test]
fn a_box_that_stops_being_fixed_clears_its_viewport_pixels() {
    let (mut app, h) = mount(Box::new(|s, o| {
        let c = el(
            s,
            o,
            "width: 200px; height: 100px; overflow: hidden; margin-left: 50px;",
        );
        let r = el(
            s,
            &c,
            &format!("{BLUE} position: fixed; left: 400px; top: 300px;"),
        );
        vec![c, r]
    }));
    let old = (400, 300, 440, 340);
    let before = full_frame(&mut app);
    assert!(ink_in(&before, old) > 1000, "positive control");
    h[1].set_style("position", "static");
    resolve(&mut app);
    check(&mut app, "fixed -> static", old, true);
}

/// A clipper becomes the containing block of an absolute child that was drawn
/// outside it: the child moves inside and is clipped; where it was is cleared.
/// A clipper that starts clipping its absolute descendants in repaints in full
/// (#1408), so this one pins the pixels, not the damage.
#[test]
fn a_clipper_that_becomes_a_containing_block_clears_its_escaped_child() {
    let (mut app, h) = mount(Box::new(|s, o| {
        let c = el(
            s,
            o,
            "width: 200px; height: 100px; overflow: hidden; margin-left: 20px; margin-top: 20px;",
        );
        let r = el(
            s,
            &c,
            &format!("{BLUE} position: absolute; left: 300px; top: 200px;"),
        );
        vec![c, r]
    }));
    let old = (300, 200, 340, 240);
    let before = full_frame(&mut app);
    assert!(ink_in(&before, old) > 1000, "positive control");
    h[0].set_style("position", "relative");
    resolve(&mut app);
    check(&mut app, "clipper becomes containing block", old, false);
}

/// The reverse: a containing-block clipper stops being one, and its absolute
/// child, clipped away inside it, is drawn at the viewport position. In full,
/// as above (#1408).
#[test]
fn a_clipper_that_stops_being_a_containing_block_lets_its_child_out() {
    let (mut app, h) = mount(Box::new(|s, o| {
        let c = el(
            s,
            o,
            "width: 200px; height: 100px; overflow: hidden; margin-left: 300px; margin-top: 250px; position: relative;",
        );
        let r = el(
            s,
            &c,
            &format!("{BLUE} position: absolute; left: 10px; top: 10px;"),
        );
        vec![c, r]
    }));
    let old = (310, 260, 350, 300);
    let before = full_frame(&mut app);
    assert!(ink_in(&before, old) > 1000, "positive control");
    h[0].set_style("position", "static");
    resolve(&mut app);
    check(&mut app, "clipper stops being containing block", old, false);
}

/// A box **inside** a clipper becomes the containing block of an absolute
/// child that escaped the clipper: the child moves under the clipper and is
/// clipped away. The flipping box does not clip, and the clipper is above it,
/// so this frame is partial — the child's old rect is placed and clipped by
/// the chain as it was painted (escaping the clipper), not as it is now.
#[test]
fn a_box_inside_a_clipper_that_becomes_a_containing_block_clears_the_escaped_child() {
    let (mut app, h) = mount(Box::new(|s, o| {
        let c = el(
            s,
            o,
            "width: 200px; height: 100px; overflow: hidden; margin-left: 20px; margin-top: 20px;",
        );
        let w = el(s, &c, "width: 100px; height: 50px;");
        let r = el(
            s,
            &w,
            &format!("{BLUE} position: absolute; left: 300px; top: 200px;"),
        );
        vec![c, w, r]
    }));
    let old = (300, 200, 340, 240);
    let before = full_frame(&mut app);
    assert!(ink_in(&before, old) > 1000, "positive control");
    h[1].set_style("position", "relative");
    resolve(&mut app);
    check(&mut app, "wrapper becomes containing block", old, true);
}
