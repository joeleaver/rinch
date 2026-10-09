//! Review fixtures for PR #1494: an out-of-flow box placed from its static
//! position **after the lines are built** is repainted where it was and
//! where it is, and is hit where it is.
//!
//! Each case compares the incremental frame with a from-scratch frame of the
//! same state over the whole surface, asserts through the counters that the
//! frame was incremental, and has a positive control that the box moved.

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

fn differing(a: &[u8], b: &[u8]) -> usize {
    a.chunks(4)
        .zip(b.chunks(4))
        .filter(|(p, q)| (0..4).any(|k| (p[k] as i32 - q[k] as i32).abs() > 2))
        .count()
}

fn mount(
    cstyle: &'static str,
    tag: &'static str,
    position: &'static str,
    extra: &'static str,
) -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let outer = scope.create_element("div");
        outer.set_attribute("style", "width: 600px; height: 400px");
        outer.set_inner_html(&format!(
            r#"<div data-c style="{cstyle} width: 300px; margin: 40px 0 0 50px; font: 16px/20px sans-serif;"><span data-t>lead</span><{tag} data-b style="position: {position}; width: 40px; height: 30px; background: rgb(0, 0, 200); {extra}"></{tag}> tail text</div>"#
        ));
        outer
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    app
}

fn node(app: &RinchApp, selector: &str) -> usize {
    let doc = app.doc().unwrap().borrow();
    let found = rinch_dom::testing::query_selector(&doc.tree, selector);
    assert_eq!(found.len(), 1, "{selector}");
    found[0]
}

fn screen_box(app: &RinchApp, id: usize) -> (f32, f32, f32, f32) {
    let doc = app.doc().unwrap().borrow();
    let (x, y) = rinch_dom::paint::compute_absolute_position(&doc.tree, id, 1.0);
    let n = doc.tree.get(id).unwrap();
    (x as f32, y as f32, n.layout.width, n.layout.height)
}

fn hit(app: &RinchApp, x: f32, y: f32) -> Option<usize> {
    let doc = app.doc().unwrap().borrow();
    super::hit_testing::hit_test(&doc.tree, x, y)
}

fn check(
    cstyle: &'static str,
    tag: &'static str,
    position: &'static str,
    extra: &'static str,
    change: impl Fn(&mut RinchApp),
) {
    let what = format!("{position} <{tag} {extra}> in [{cstyle}]");
    let mut app = mount(cstyle, tag, position, extra);
    let b = node(&app, "[data-b]");
    let _ = full_frame(&mut app);
    let before = screen_box(&app, b);
    assert_eq!(
        hit(&app, before.0 + 20.0, before.1 + 15.0),
        Some(b),
        "{what}: hit before"
    );

    change(&mut app);
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    let after = screen_box(&app, b);
    assert_ne!(
        (before.0, before.1),
        (after.0, after.1),
        "{what}: positive control, the box moved"
    );
    let (inc, stats) = incremental_frame(&mut app);
    assert_eq!(stats.get(Counter::RepaintFull), 0, "{what}: {stats:?}");
    assert_eq!(stats.get(Counter::RepaintPartial), 1, "{what}: {stats:?}");
    let full = full_frame(&mut app);
    assert_eq!(
        differing(&inc, &full),
        0,
        "{what}: the incremental frame is the from-scratch one (moved {before:?} -> {after:?})"
    );
    assert_eq!(
        hit(&app, after.0 + 20.0, after.1 + 15.0),
        Some(b),
        "{what}: hit where it is now"
    );
    let old_centre = (before.0 + 20.0, before.1 + 15.0);
    let inside_new = old_centre.0 >= after.0
        && old_centre.0 < after.0 + 40.0
        && old_centre.1 >= after.1
        && old_centre.1 < after.1 + 30.0;
    if !inside_new {
        assert_ne!(
            hit(&app, old_centre.0, old_centre.1),
            Some(b),
            "{what}: not hit where it was"
        );
    }
}

fn longer_text(app: &mut RinchApp) {
    let t = node(app, "[data-t]");
    let doc = app.doc().unwrap().clone();
    let mut d = doc.borrow_mut();
    use rinch_core::dom::{DomDocument, NodeId};
    let text = d.get_children(NodeId(t))[0];
    d.set_text_content(
        text,
        "lead lead lead lead lead lead lead lead lead lead lead lead lead",
    );
}

fn align_right(app: &mut RinchApp) {
    let c = node(app, "[data-c]");
    let doc = app.doc().unwrap().clone();
    let mut d = doc.borrow_mut();
    use rinch_core::dom::{DomDocument, NodeId};
    d.set_style(NodeId(c), "text-align", "right");
}

#[test]
fn a_text_edit_before_the_box_repaints_and_hits_it_where_it_went() {
    for cstyle in ["position: relative;", ""] {
        for tag in ["span", "div"] {
            for position in ["absolute", "fixed"] {
                for extra in ["", "margin-top: 90px;"] {
                    check(cstyle, tag, position, extra, longer_text);
                }
            }
        }
    }
}

#[test]
fn a_text_align_change_repaints_and_hits_an_inline_level_box_where_it_went() {
    for cstyle in ["position: relative;", ""] {
        for position in ["absolute", "fixed"] {
            for extra in ["", "margin-top: 90px;"] {
                check(cstyle, "span", position, extra, align_right);
            }
        }
    }
}
