//! The content-sized mode (#2552, R0).

use super::*;
use rinch_core::dom::{NodeHandle, RenderScope};
use std::cell::RefCell;

thread_local! {
    static CHILD: RefCell<Option<NodeHandle>> = const { RefCell::new(None) };
}

const MAX: (f32, f32) = (800.0, 600.0);

/// A content-sized app, mounted and laid out at `MAX`, as before its window.
fn sized(build: fn(&mut RenderScope) -> NodeHandle) -> RinchApp {
    let mut app = RinchApp::new(build);
    app.set_content_sized(MAX);
    app.mount_component(MAX.0, MAX.1);
    app.resolve_and_repaint(MAX.0, MAX.1);
    app
}

/// A configure: the window given `(w, h)`, laid out there.
fn grant(app: &mut RinchApp, (w, h): (u32, u32)) {
    app.set_granted_size(w as f32, h as f32);
    app.resolve_and_repaint(w as f32, h as f32);
}

fn styled(scope: &mut RenderScope, style: &str, text: Option<&str>) -> NodeHandle {
    let node = scope.create_element("div");
    node.set_attribute("style", style);
    if let Some(text) = text {
        node.append_child(&scope.create_text(text));
    }
    CHILD.with(|c| *c.borrow_mut() = Some(node.clone()));
    node
}

fn fixed(scope: &mut RenderScope) -> NodeHandle {
    styled(scope, "width: 300px; height: 40px", None)
}

fn short_text(scope: &mut RenderScope) -> NodeHandle {
    styled(scope, "font: 16px/20px sans-serif", Some("hello"))
}

fn long_text(scope: &mut RenderScope) -> NodeHandle {
    styled(
        scope,
        "font: 16px/20px sans-serif",
        Some("a long line of text that wraps when it is narrower than its max-content width"),
    )
}

fn half_the_viewport(scope: &mut RenderScope) -> NodeHandle {
    styled(scope, "width: 50vw; height: 30px", None)
}

#[test]
fn a_node_of_its_own_width_gets_exactly_that_and_one_without_shrinks() {
    let mut app = sized(fixed);
    assert_eq!(
        app.content_size(),
        Some((300, 40)),
        "the component's width, untouched"
    );
    let mut app = sized(short_text);
    let (w, h) = app.content_size().unwrap();
    assert!(w > 0 && w < 100, "shrunk to its text: {w}");
    assert_eq!(h, 20);
}

#[test]
fn text_wrapping_at_a_narrower_grant_asks_for_nothing_more() {
    let mut app = sized(long_text);
    let (w, h) = app.content_size().unwrap();
    assert!(w > 200 && h == 20, "one line, unconstrained: {w}x{h}");
    // Granted narrower than asked: it wraps, taller, and scrolls; no request.
    grant(&mut app, (200, 20));
    assert_eq!(app.content_size_changed(), None);
    for _ in 0..10 {
        grant(&mut app, (200, 20));
        assert_eq!(app.content_size_changed(), None);
    }
}

#[test]
fn viewport_relative_content_settles_after_one_request_per_change() {
    let mut app = sized(half_the_viewport);
    let first = app.content_size().unwrap();
    assert_eq!(first, (400, 30), "half the viewport it was laid out in");
    // The window takes it; its 50vw is now half of that, but a granted size
    // asks for nothing.
    for _ in 0..10 {
        grant(&mut app, first);
        assert_eq!(
            app.content_size_changed(),
            None,
            "no shrinking step by step"
        );
    }
    // A real change of the content asks once.
    CHILD.with(|c| c.borrow().as_ref().unwrap().set_style("height", "50px"));
    app.resolve_and_repaint(first.0 as f32, first.1 as f32);
    let asked = app.content_size_changed();
    assert_eq!(
        asked,
        Some((200, 50)),
        "50vw of the window it is in, and the new height"
    );
    grant(&mut app, asked.unwrap());
    assert_eq!(app.content_size_changed(), None, "and settles there");
}

#[test]
fn an_unchanged_size_after_a_change_asks_nothing() {
    let mut app = sized(fixed);
    app.content_size();
    CHILD.with(|c| c.borrow().as_ref().unwrap().set_style("background", "red"));
    app.resolve_and_repaint(300.0, 40.0);
    assert_eq!(app.content_size_changed(), None, "a change, the same size");
}

#[test]
fn an_ordinary_window_is_not_content_sized() {
    let mut app = RinchApp::new(fixed);
    app.mount_component(MAX.0, MAX.1);
    app.resolve_and_repaint(MAX.0, MAX.1);
    assert_eq!(app.content_size(), None);
    assert_eq!(app.content_size_changed(), None);
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let body = d.tree.body_id;
    assert!(
        d.tree
            .get(body)
            .and_then(|n| n.attributes.get("style"))
            .is_none(),
        "rinch writes nothing on an ordinary window's body"
    );
}
