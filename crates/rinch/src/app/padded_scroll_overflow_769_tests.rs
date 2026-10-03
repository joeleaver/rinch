//! #769: every scroll finder (the wheel's routing), the body fallback and
//! layout's own clamp agree with the painted bar about a padded or bordered
//! container's overflow, on both axes. (rinch sizes every box border-box,
//! #1278, so `box-sizing: content-box` is not exercised here.)

use super::hit_testing::{
    find_horizontal_scroll_container, find_horizontal_scroll_container_at_point,
    find_scroll_container, find_scroll_container_at_point,
};
use super::*;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::scrollbar::scrollbars;

fn child_of(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let el = doc.create_element("div");
    doc.set_attribute(el, "style", style);
    doc.append_child(parent, el);
    el
}

/// A border alone (no padding) opens the same window: 10px borders on a
/// 100px border-box leave an 80px content box; a 90px child overflows it.
#[test]
fn a_bordered_containers_overflow_matches_its_painted_bar() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = child_of(
        &mut doc,
        body,
        "box-sizing: border-box; width: 200px; height: 100px; \
         border: 10px solid black; overflow: auto",
    );
    let child = child_of(&mut doc, c, "width: 50px; height: 90px");
    doc.resolve_layout(800.0, 600.0);
    assert!(scrollbars(&doc.tree, c.0, 1.0).vertical.is_some());
    assert_eq!(doc.client_height(c), 80.0);
    assert_eq!(doc.scroll_height(c), 90.0);
    assert_eq!(find_scroll_container(&doc.tree, child.0), Some(c.0));
    assert_eq!(
        find_scroll_container_at_point(&doc.tree, 30.0, 50.0),
        Some(c.0)
    );
}

/// Padding AND a border together (rinch sizes every box border-box, #1278, so
/// `box-sizing: content-box` cannot be exercised here): 200x100 less 20px
/// padding and 5px border is a 150x50 content box. An 80px child overflows it
/// and not the 100px border box.
#[test]
fn a_padded_and_bordered_containers_overflow_matches_its_painted_bar() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = child_of(
        &mut doc,
        body,
        "width: 200px; height: 100px; padding: 20px; border: 5px solid black; overflow: auto",
    );
    let child = child_of(&mut doc, c, "width: 50px; height: 80px");
    doc.resolve_layout(800.0, 600.0);
    assert!(scrollbars(&doc.tree, c.0, 1.0).vertical.is_some());
    assert_eq!(doc.client_height(c), 50.0);
    assert_eq!(find_scroll_container(&doc.tree, child.0), Some(c.0));
    assert_eq!(
        find_scroll_container_at_point(&doc.tree, 30.0, 50.0),
        Some(c.0)
    );
}

/// Horizontal twin: a 180px child overflows the 150px content box and not the
/// 200px border box.
#[test]
fn a_padded_and_bordered_containers_horizontal_overflow_matches_its_bar() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = child_of(
        &mut doc,
        body,
        "width: 200px; height: 100px; padding: 20px; border: 5px solid black; \
         overflow-x: auto; overflow-y: hidden",
    );
    let child = child_of(&mut doc, c, "flex-shrink: 0; width: 180px; height: 10px");
    doc.resolve_layout(800.0, 600.0);
    assert!(scrollbars(&doc.tree, c.0, 1.0).horizontal.is_some());
    assert_eq!(doc.client_width(c), 150.0);
    assert_eq!(doc.scroll_width(c), 180.0);
    assert_eq!(
        find_horizontal_scroll_container(&doc.tree, child.0),
        Some(c.0)
    );
    assert_eq!(
        find_horizontal_scroll_container_at_point(&doc.tree, 50.0, 40.0),
        Some(c.0)
    );
}

/// Horizontal fixed point: content exactly fills the content box (60 == 60)
/// while being smaller than the border box — no bar, no wheel.
#[test]
fn a_horizontal_exact_fill_neither_paints_nor_scrolls() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = child_of(
        &mut doc,
        body,
        "box-sizing: border-box; width: 100px; height: 200px; padding: 20px; overflow-x: auto",
    );
    let child = child_of(&mut doc, c, "width: 60px; height: 10px");
    doc.resolve_layout(800.0, 600.0);
    assert!(scrollbars(&doc.tree, c.0, 1.0).horizontal.is_none());
    assert_eq!(find_horizontal_scroll_container(&doc.tree, child.0), None);
}

/// `--rinch-scrollbar-width: none` removes the bar from paint and hit testing
/// but must NOT stop the wheel (a browser's `scrollbar-width: none` still
/// scrolls). `overflows` is therefore deliberately NOT "exactly scrollbars()".
#[test]
fn a_barless_padded_container_still_takes_the_wheel() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = child_of(
        &mut doc,
        body,
        "box-sizing: border-box; width: 200px; height: 100px; padding: 20px; \
         overflow: auto; --rinch-scrollbar-width: none",
    );
    let child = child_of(&mut doc, c, "width: 160px; height: 80px");
    doc.resolve_layout(800.0, 600.0);
    assert!(
        scrollbars(&doc.tree, c.0, 1.0).vertical.is_none(),
        "no bar painted"
    );
    assert_eq!(find_scroll_container(&doc.tree, child.0), Some(c.0));
}

/// The body fallback itself (a walk from a body child meets the body as an
/// ancestor and never reaches the fallback). Start the walk at the
/// `<html>` element so only the fallback can answer.
#[test]
fn the_body_fallback_uses_the_content_box() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "padding: 20px; overflow: auto");
    let _child = child_of(&mut doc, body, "width: 100%; height: 580px");
    doc.resolve_layout(800.0, 600.0);
    assert!(scrollbars(&doc.tree, body.0, 1.0).vertical.is_some());
    let html = doc.tree.html_id;
    assert_eq!(find_scroll_container(&doc.tree, html), Some(body.0));
}

fn padded_app(container_style: &'static str, child_style: &'static str) -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let c = scope.create_element("div");
        c.set_attribute("class", "scroller");
        c.set_attribute("style", container_style);
        let k = scope.create_element("div");
        k.set_attribute("style", child_style);
        c.append_child(&k);
        root.append_child(&c);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    app
}

fn scroller_offset(app: &RinchApp) -> (f64, f64) {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let id = d
        .tree
        .nodes
        .iter()
        .find(|(_, n)| n.attributes.get("class").is_some_and(|c| c == "scroller"))
        .map(|(id, _)| id)
        .unwrap();
    d.tree.nodes[id].scroll_offset
}

fn wheel(app: &mut RinchApp, x: f32, y: f32, dx: f64, dy: f64) {
    app.handle_event(
        PlatformEvent::MouseWheel {
            x,
            y,
            delta_x: dx,
            delta_y: dy,
        },
        (800, 600),
        1.0,
    );
}

/// End to end: a real wheel over the issue's repro scrolls the container by
/// exactly its painted range (content 80 - content box 60 = 20).
#[test]
fn a_real_wheel_scrolls_the_padded_container_by_its_painted_range() {
    let mut app = padded_app(
        "box-sizing: border-box; width: 200px; height: 100px; padding: 20px; overflow: auto",
        "width: 160px; height: 80px",
    );
    wheel(&mut app, 50.0, 50.0, 0.0, -1000.0);
    assert_eq!(scroller_offset(&app).1, 20.0);
}

/// End to end, horizontal and bordered.
#[test]
fn a_real_horizontal_wheel_scrolls_a_bordered_container_by_its_painted_range() {
    let mut app = padded_app(
        "box-sizing: border-box; width: 100px; height: 200px; border: 10px solid black; \
         overflow-x: auto; overflow-y: hidden",
        "width: 90px; height: 10px",
    );
    wheel(&mut app, 50.0, 50.0, -1000.0, 0.0);
    assert_eq!(scroller_offset(&app).0, 10.0);
}

/// Horizontal body fallback, reached from `<html>` as above: 780 > 760 (the
/// body's content box) and < 800 (its border box).
#[test]
fn the_horizontal_body_fallback_uses_the_content_box() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "padding: 20px; overflow: auto");
    let _child = child_of(&mut doc, body, "width: 780px; height: 10px");
    doc.resolve_layout(800.0, 600.0);
    assert!(scrollbars(&doc.tree, body.0, 1.0).horizontal.is_some());
    let html = doc.tree.html_id;
    assert_eq!(
        find_horizontal_scroll_container(&doc.tree, html),
        Some(body.0)
    );
}

/// Behaviour change: the body fallback used to skip the `scrolls_y()` test, so
/// an `overflow: hidden` body was still a wheel target when the walk started
/// outside it. Now it is not — the CSS answer (a hidden body's overflow
/// propagates to the viewport, which then takes no wheel).
#[test]
fn an_overflow_hidden_body_is_no_fallback_wheel_target() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "overflow: hidden");
    let _child = child_of(&mut doc, body, "width: 100px; height: 900px");
    doc.resolve_layout(800.0, 600.0);
    let html = doc.tree.html_id;
    assert_eq!(find_scroll_container(&doc.tree, html), None);
}

/// Layout's clamp (`clamp_scroll_offsets`) measures the range against the same
/// content box: 200x100 less 20px padding and 5px border is 50px tall, so when
/// the content shrinks to 80px the offset is clamped to 30 — not 20 (padding
/// alone), not 0 (the border box, or the border alone).
#[test]
fn layouts_clamp_uses_the_padded_and_bordered_content_box() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = child_of(
        &mut doc,
        body,
        "width: 200px; height: 100px; padding: 20px; border: 5px solid black; overflow: auto",
    );
    let child = child_of(&mut doc, c, "width: 50px; height: 300px");
    doc.resolve_layout(800.0, 600.0);
    doc.set_scroll_top(c, 250.0);
    assert_eq!(
        doc.scroll_top(c),
        250.0,
        "positive control: the full range is 300 - 50"
    );
    doc.set_attribute(child, "style", "width: 50px; height: 80px");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(doc.scroll_top(c), 30.0);
}
