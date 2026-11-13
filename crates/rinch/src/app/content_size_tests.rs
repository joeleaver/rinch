//! The content-sized mode (#2552, R0).

use super::*;
use rinch_core::dom::{NodeHandle, RenderScope};
use std::cell::RefCell;

thread_local! {
    pub(super) static CHILD: RefCell<Option<NodeHandle>> = const { RefCell::new(None) };
}

pub(super) const MAX: (f32, f32) = (800.0, 600.0);

/// A content-sized app, mounted and laid out at `MAX`, as before its window.
pub(super) fn sized(build: fn(&mut RenderScope) -> NodeHandle) -> RinchApp {
    let mut app = RinchApp::new_content_sized(build, MAX);
    app.mount_component(MAX.0, MAX.1);
    app.resolve_and_repaint(MAX.0, MAX.1);
    app
}

/// A configure: the window given `(w, h)` (R1's call, then the `Resized`
/// event's layout), then the frame.
pub(super) fn grant(app: &mut RinchApp, (w, h): (u32, u32)) {
    app.set_granted_size(w as f32, h as f32);
    app.resize_layout(w, h);
    app.resolve_and_repaint(w as f32, h as f32);
}

pub(super) fn styled(scope: &mut RenderScope, style: &str, text: Option<&str>) -> NodeHandle {
    let node = scope.create_element("div");
    node.set_attribute("style", style);
    if let Some(text) = text {
        node.append_child(&scope.create_text(text));
    }
    CHILD.with(|c| *c.borrow_mut() = Some(node.clone()));
    node
}

pub(super) fn fixed(scope: &mut RenderScope) -> NodeHandle {
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

fn set_child_text(text: &str) {
    CHILD.with(|c| c.borrow().as_ref().unwrap().set_text(text));
}

fn nothing_waiting(app: &RinchApp) -> bool {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree.dirty_nodes.is_empty() && d.tree.style_dirty_nodes.is_empty()
}

/// Text that grows after a grant of what was asked asks for more, up to the
/// largest size, instead of wrapping at the old width: the grant left the
/// box's limit at the largest size, and wrote nothing.
#[test]
fn content_that_grows_after_a_grant_of_what_was_asked_asks_for_more() {
    let mut app = sized(short_text);
    let short = app.content_size().unwrap();
    assert!(short.0 < 100, "short: {short:?}");
    app.set_granted_size(short.0 as f32, short.1 as f32);
    assert!(
        nothing_waiting(&app),
        "the ordinary configure dirties nothing"
    );
    app.resize_layout(short.0, short.1);
    app.resolve_and_repaint(short.0 as f32, short.1 as f32);

    set_child_text("a longer line of text than the one the window was made for");
    app.resolve_and_repaint(short.0 as f32, short.1 as f32);
    let longer = app.content_size_changed().expect("asks again");
    assert!(
        longer.0 > 200 && longer.1 == 20,
        "wider, one line: {longer:?}"
    );
    grant(&mut app, longer);

    set_child_text(&"a line far longer than the largest size allows; ".repeat(4));
    app.resolve_and_repaint(longer.0 as f32, longer.1 as f32);
    let most = app.content_size_changed().expect("asks again");
    assert_eq!(most.0, MAX.0 as u32, "up to the largest width: {most:?}");
    assert!(most.1 > 20, "and wraps there: {most:?}");
}

/// A change of the document after a grant smaller than asked, on a node the
/// grant's own write dirtied, before the frame (an effect run between the
/// configure and the frame), is a change of the document: it asks.
#[test]
fn a_change_after_a_constraining_grant_on_a_node_it_dirtied_asks() {
    let mut app = sized(long_text);
    let (w, _) = app.content_size().unwrap();
    assert!(w > 200);
    // Narrower than asked: the box's limit is written, and laid out at once.
    app.set_granted_size(200.0, 20.0);
    assert!(
        nothing_waiting(&app),
        "the grant's own layout consumed its dirt"
    );
    app.resize_layout(200, 20);
    // A style write on the existing child: no new node, only one the grant's
    // write had dirtied too.
    CHILD.with(|c| c.borrow().as_ref().unwrap().set_style("width", "120px"));
    app.resolve_and_repaint(200.0, 20.0);
    let asked = app.content_size_changed().expect("the change asks");
    assert_eq!(asked.0, 120, "the new width: {asked:?}");
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
    assert_eq!(app.content_box(), None, "no content box");
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let body = d.tree.body_id;
    let content = CHILD.with(|c| c.borrow().as_ref().unwrap().node_id().0);
    assert_eq!(
        d.tree.get(content).and_then(|n| n.parent),
        Some(body),
        "an ordinary window's content is in <body>, unwrapped"
    );
    assert!(
        d.tree
            .get(body)
            .and_then(|n| n.attributes.get("style"))
            .is_none(),
        "rinch writes nothing on an ordinary window's body"
    );
}
