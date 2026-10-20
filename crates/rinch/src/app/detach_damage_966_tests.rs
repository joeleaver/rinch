//! A subtree taken out of the document by `replace_node`, `set_text_content`
//! or `set_inner_html` clears the pixels it was painted at (#966).
//!
//! `remove_node` and the move verbs record a leaving subtree's painted rects
//! and forget its painted state (`mark_subtree_paint_dirty`,
//! `record_pixels_left_by_move`). These three routes did neither: only the
//! parent's box was repainted, so ink of the subtree outside that box — an
//! overflowing child, a shadow — stayed on screen; and a displaced or
//! orphaned node kept its painted state, so appended elsewhere its old rect
//! was placed along its **new** parents and clipped by them.
//!
//! Local pixel oracles in `damage_clip_chain_tests`' idiom: the incremental
//! frame must equal a from-scratch frame of the same state, a positive control
//! says the old pixels were really there, and a counter check says the frame
//! really was incremental (a full repaint passes every pixel assertion
//! vacuously). Every ghost region lies outside the box of the node whose
//! child list changed, which is the only rect these routes damaged before.

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
/// The next frame equals a from-scratch one and left nothing at `old`, and
/// was a partial repaint.
fn check(app: &mut RinchApp, label: &str, old: (i32, i32, i32, i32)) {
    let (inc, stats) = incremental_frame(app);
    assert_eq!(stats.get(Counter::RepaintFull), 0, "{label}: {stats:?}");
    assert_eq!(stats.get(Counter::RepaintPartial), 1, "{label}: {stats:?}");
    let full = full_frame(app);
    assert_eq!(ink_in(&full, old), 0, "{label}: the old place is empty now");
    assert_eq!(ink_in(&inc, old), 0, "{label}: ghost at the old place");
    assert_eq!(
        diff_in(&inc, &full, WHOLE),
        0,
        "{label}: incremental != full"
    );
}

fn small(scope: &mut RenderScope) -> NodeHandle {
    let n = scope.create_element("div");
    n.set_attribute(
        "style",
        "width: 10px; height: 10px; background: rgb(0, 200, 0);",
    );
    n
}

/// `replace_with`: the displaced node overflows a parent that does not clip.
#[test]
fn a_displaced_node_clears_its_overflow() {
    let (mut app, h) = mount(Box::new(|s, o| {
        let p = el(s, o, "width: 200px; height: 50px;");
        let old = el(
            s,
            &p,
            "width: 40px; height: 120px; background: rgb(200, 0, 0);",
        );
        let n = small(s);
        vec![p, old, n]
    }));
    let before = full_frame(&mut app);
    assert!(ink_in(&before, (0, 60, 40, 120)) > 2000, "positive control");
    h[1].replace_with(&h[2]);
    resolve(&mut app);
    check(&mut app, "displaced", (0, 60, 40, 120));
}

/// `replace_with` inside a clipper: the displaced node's overflow is inside
/// the clipper, so it was on screen and has to go.
#[test]
fn a_displaced_node_clears_its_overflow_inside_a_clipper() {
    let (mut app, h) = mount(Box::new(|s, o| {
        let c = el(s, o, "width: 300px; height: 200px; overflow: hidden;");
        let p = el(s, &c, "width: 200px; height: 50px;");
        let old = el(
            s,
            &p,
            "width: 40px; height: 120px; background: rgb(200, 0, 0);",
        );
        let n = small(s);
        vec![p, old, n]
    }));
    let before = full_frame(&mut app);
    assert!(ink_in(&before, (0, 60, 40, 120)) > 2000, "positive control");
    h[1].replace_with(&h[2]);
    resolve(&mut app);
    check(&mut app, "displaced in clipper", (0, 60, 40, 120));
}

/// `set_inner_html("")`: the freed child overflows its parent.
#[test]
fn freed_children_clear_their_overflow() {
    let (mut app, h) = mount(Box::new(|s, o| {
        let c = el(s, o, "width: 300px; height: 200px; overflow: hidden;");
        let p = el(s, &c, "width: 200px; height: 50px;");
        el(
            s,
            &p,
            "width: 40px; height: 120px; background: rgb(200, 0, 0);",
        );
        vec![p]
    }));
    let before = full_frame(&mut app);
    assert!(ink_in(&before, (0, 60, 40, 120)) > 2000, "positive control");
    h[0].set_inner_html("");
    resolve(&mut app);
    check(&mut app, "inner html", (0, 60, 40, 120));
}

/// `set_inner_html` with new content: a freed grandchild's shadow outside
/// every box it had.
#[test]
fn freed_children_clear_a_shadow_below_them() {
    let (mut app, h) = mount(Box::new(|s, o| {
        let p = el(s, o, "width: 200px; height: 50px;");
        let w = el(s, &p, "width: 100px; height: 40px;");
        el(
            s,
            &w,
            "width: 40px; height: 20px; background: rgb(200, 0, 0); \
             box-shadow: 0px 80px 0px 0px rgb(0, 0, 200);",
        );
        vec![p]
    }));
    let before = full_frame(&mut app);
    assert!(ink_in(&before, (0, 80, 40, 100)) > 700, "positive control");
    h[0].set_inner_html("<div style=\"width: 10px; height: 10px\"></div>");
    resolve(&mut app);
    check(&mut app, "inner html shadow", (0, 80, 40, 100));
}

/// `set_text`: an orphaned child overflows its old parent and is not put back.
#[test]
fn an_orphan_clears_its_overflow() {
    let (mut app, h) = mount(Box::new(|s, o| {
        let p = el(s, o, "width: 200px; height: 40px;");
        el(
            s,
            &p,
            "width: 40px; height: 80px; margin-left: 300px; background: rgb(0, 0, 200);",
        );
        vec![p]
    }));
    let before = full_frame(&mut app);
    assert!(ink_in(&before, (300, 45, 340, 80)) > 1000, "positive control");
    h[0].set_text("");
    resolve(&mut app);
    check(&mut app, "orphan", (300, 45, 340, 80));
}

/// `set_text`: the orphan is appended into a clipper in the same frame. Its
/// old rect must be placed under the parent it was painted under, not cut
/// away by the clipper it went into.
#[test]
fn an_orphan_moved_into_a_clipper_clears_where_it_was() {
    let (mut app, h) = mount(Box::new(|s, o| {
        let p = el(s, o, "width: 200px; height: 40px;");
        let r = el(
            s,
            &p,
            "width: 40px; height: 80px; margin-left: 300px; background: rgb(0, 0, 200);",
        );
        let c = el(
            s,
            o,
            "width: 200px; height: 100px; overflow: hidden; margin-top: 160px;",
        );
        vec![p, r, c]
    }));
    let before = full_frame(&mut app);
    assert!(ink_in(&before, (300, 45, 340, 80)) > 1000, "positive control");
    h[0].set_text("");
    h[2].append_child(&h[1]);
    resolve(&mut app);
    check(&mut app, "orphan into clipper", (300, 45, 340, 80));
}

/// `set_text`: the orphan overflows its old parent and is appended into an
/// absolute clipper sitting exactly where that parent is, so the new chain
/// clips its old rect to the parent's box.
#[test]
fn an_orphan_moved_into_a_coincident_clipper_clears_its_overflow() {
    let (mut app, h) = mount(Box::new(|s, o| {
        o.set_attribute(
            "style",
            "width: 600px; height: 400px; position: relative;",
        );
        let w = el(s, o, "width: 200px; height: 10px; margin-top: 100px;");
        let r = el(s, &w, BLUE);
        let c = el(
            s,
            o,
            "position: absolute; left: 0; top: 100px; width: 200px; height: 10px; overflow: hidden;",
        );
        vec![w, r, c]
    }));
    let before = full_frame(&mut app);
    assert!(ink_in(&before, (0, 115, 40, 140)) > 900, "positive control");
    h[0].set_text("");
    h[2].append_child(&h[1]);
    resolve(&mut app);
    check(&mut app, "orphan into coincident clipper", (0, 115, 40, 140));
}

/// The same shape through `replace_with`'s displaced node.
#[test]
fn a_displaced_node_moved_into_a_coincident_clipper_clears_its_overflow() {
    let (mut app, h) = mount(Box::new(|s, o| {
        o.set_attribute(
            "style",
            "width: 600px; height: 400px; position: relative;",
        );
        let w = el(s, o, "width: 200px; height: 10px; margin-top: 100px;");
        let r = el(s, &w, BLUE);
        let c = el(
            s,
            o,
            "position: absolute; left: 0; top: 100px; width: 200px; height: 10px; overflow: hidden;",
        );
        let n = small(s);
        vec![w, r, c, n]
    }));
    let before = full_frame(&mut app);
    assert!(ink_in(&before, (0, 115, 40, 140)) > 900, "positive control");
    h[1].replace_with(&h[3]);
    h[2].append_child(&h[1]);
    resolve(&mut app);
    check(&mut app, "displaced into coincident clipper", (0, 115, 40, 140));
}

/// `remove_child`, the fourth route the issue did not name: the removed
/// child overflows its parent.
#[test]
fn a_removed_child_clears_its_overflow() {
    let (mut app, h) = mount(Box::new(|s, o| {
        let p = el(s, o, "width: 200px; height: 50px;");
        let r = el(
            s,
            &p,
            "width: 40px; height: 120px; background: rgb(200, 0, 0);",
        );
        vec![p, r]
    }));
    let before = full_frame(&mut app);
    assert!(ink_in(&before, (0, 60, 40, 120)) > 2000, "positive control");
    h[0].remove_child(&h[1]);
    resolve(&mut app);
    check(&mut app, "remove_child", (0, 60, 40, 120));
}

/// `remove_child`, then the child appended into a coincident clipper.
#[test]
fn a_removed_child_moved_into_a_coincident_clipper_clears_its_overflow() {
    let (mut app, h) = mount(Box::new(|s, o| {
        o.set_attribute(
            "style",
            "width: 600px; height: 400px; position: relative;",
        );
        let w = el(s, o, "width: 200px; height: 10px; margin-top: 100px;");
        let r = el(s, &w, BLUE);
        let c = el(
            s,
            o,
            "position: absolute; left: 0; top: 100px; width: 200px; height: 10px; overflow: hidden;",
        );
        vec![w, r, c]
    }));
    let before = full_frame(&mut app);
    assert!(ink_in(&before, (0, 115, 40, 140)) > 900, "positive control");
    h[0].remove_child(&h[1]);
    h[2].append_child(&h[1]);
    resolve(&mut app);
    check(&mut app, "remove_child into coincident clipper", (0, 115, 40, 140));
}

/// A generated box freed when its rule stops matching (the pseudo-element
/// pruning in `resolve.rs`, a fifth route): it overflows its originator.
#[test]
fn a_generated_box_that_stops_matching_clears_its_overflow() {
    let (mut app, h) = mount(Box::new(|s, o| {
        let st = s.create_element("style");
        let t = s.create_text(".x::before { content: ''; display: block; width: 40px; height: 120px; background: rgb(200,0,0); }");
        st.append_child(&t);
        o.append_child(&st);
        let p = el(s, o, "width: 200px; height: 50px;");
        p.set_attribute("class", "x");
        vec![p]
    }));
    let before = full_frame(&mut app);
    assert!(ink_in(&before, (0, 60, 40, 120)) > 2000, "positive control");
    h[0].set_attribute("class", "y");
    resolve(&mut app);
    check(&mut app, "generated box", (0, 60, 40, 120));
}
