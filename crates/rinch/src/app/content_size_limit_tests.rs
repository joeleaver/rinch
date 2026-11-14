//! The box's limit across grants (#2552, R0): a dimension is free (the
//! largest size) until the compositor grants less than was asked, and from
//! then on its limit is the largest size the compositor has granted since.

use super::tests::{CHILD, MAX, grant, long_text, nothing_waiting, set_child_text, sized};
use super::*;
use crate::app::hit_testing::painted_element_box;

/// The component's node's laid-out width.
fn child_width(app: &RinchApp) -> f32 {
    let d = app.doc.as_ref().unwrap().borrow();
    let id = CHILD.with(|c| c.borrow().as_ref().unwrap().node_id().0);
    painted_element_box(&d.tree, id).2
}

fn set_child_width(px: u32) {
    CHILD.with(|c| {
        c.borrow()
            .as_ref()
            .unwrap()
            .set_style("width", &format!("{px}px"))
    });
}

/// Long text, granted 200 wide where it asked for more: limited at 200.
fn limited_at_200() -> RinchApp {
    let mut app = sized(long_text);
    let (w, h) = app.content_size().unwrap();
    assert!(w > 200 && h == 20, "one line, unconstrained: {w}x{h}");
    grant(&mut app, (200, 20));
    assert_eq!(app.content_size_changed(), None, "a grant asks nothing");
    assert_eq!(app.content_box_limit(), Some((200.0, MAX.1)));
    assert!(child_width(&app) <= 200.0, "wrapped in the grant");
    app
}

/// A grant of exactly the constrained size asked for is not a sign that more
/// would be given: the dimension stays limited.
#[test]
fn a_grant_of_the_constrained_size_asked_for_keeps_the_limit() {
    let mut app = limited_at_200();
    set_child_text("another line of text, long enough that it wraps at two hundred");
    app.resolve_and_repaint(200.0, 20.0);
    let asked = app.content_size_changed().expect("a change asks");
    assert!(
        asked.0 == 200 && asked.1 > 20,
        "the wrapped size: {asked:?}"
    );
    app.set_granted_size(asked.0 as f32, asked.1 as f32);
    assert!(nothing_waiting(&app), "the grant wrote nothing");
    assert_eq!(app.content_box_limit(), Some((200.0, MAX.1)));
    app.resize_layout(asked.0, asked.1);
    app.resolve_and_repaint(asked.0 as f32, asked.1 as f32);
    assert_eq!(app.content_size_changed(), None);
    assert_eq!(app.content_size(), Some(asked), "still the wrapped size");
    assert!(child_width(&app) <= 200.0, "still inside the grant");
}

/// A limited dimension's limit is the largest grant, not the latest: content
/// that shrank and was granted its smaller size can grow back up to it.
#[test]
fn a_smaller_grant_of_a_smaller_ask_does_not_lower_the_limit() {
    let mut app = limited_at_200();
    set_child_width(120);
    app.resolve_and_repaint(200.0, 20.0);
    let small = app.content_size_changed().expect("a change asks");
    assert_eq!(small.0, 120);
    grant(&mut app, small);
    assert_eq!(
        app.content_box_limit().unwrap().0,
        200.0,
        "not the 120 grant"
    );
    set_child_width(180);
    app.resolve_and_repaint(small.0 as f32, small.1 as f32);
    let grown = app.content_size_changed().expect("a change asks");
    assert_eq!(grown.0, 180, "180, not 120: {grown:?}");
    assert_eq!(child_width(&app), 180.0);
}

/// A larger grant raises a limited dimension's limit: laid out at once, not
/// counted, nothing asked.
#[test]
fn a_larger_grant_raises_the_limit() {
    let mut app = limited_at_200();
    app.set_granted_size(300.0, 20.0);
    assert!(nothing_waiting(&app), "laid out at once");
    assert_eq!(app.content_box_limit(), Some((300.0, MAX.1)));
    app.resize_layout(300, 20);
    app.resolve_and_repaint(300.0, 20.0);
    assert_eq!(app.content_size_changed(), None, "a grant asks nothing");
    let w = child_width(&app);
    assert!(w > 200.0 && w <= 300.0, "laid out in the new limit: {w}");
}

/// Before anything was asked, a grant is the limit, but never above the
/// largest size.
#[test]
fn a_grant_larger_than_the_largest_size_before_any_ask_is_clamped() {
    let mut app = RinchApp::new_content_sized(long_text, MAX, (false, false));
    app.mount_component(MAX.0, MAX.1);
    app.resolve_and_repaint(MAX.0, MAX.1);
    app.set_granted_size(1000.0, 900.0);
    assert_eq!(app.content_box_limit(), Some(MAX));
}

/// A new negotiation frees every dimension again, at the new largest size.
#[test]
fn a_restart_frees_a_limited_dimension() {
    let mut app = limited_at_200();
    app.restart_content_limit((700.0, 500.0));
    assert_eq!(app.content_box_limit(), Some((700.0, 500.0)));
    assert!(nothing_waiting(&app), "laid out at once");
    let (w, _) = app.content_size().unwrap();
    assert!(w > 200, "one line again: {w}");
}

/// On an axis the compositor assigns, the box fills the window and that
/// axis is measured as the window's, whatever the content: a bar whose
/// height is its content.
#[test]
fn a_filled_axis_is_the_windows_and_the_other_the_contents() {
    let mut app = RinchApp::new_content_sized(super::tests::fixed, MAX, (true, false));
    app.mount_component(MAX.0, MAX.1);
    app.resolve_and_repaint(MAX.0, MAX.1);
    assert_eq!(app.content_size(), Some((800, 40)), "the window's width");
    // The window is given 1000 wide: the box fills it, nothing is asked.
    grant(&mut app, (1000, 40));
    assert_eq!(app.content_size_changed(), None);
    let d = app.doc.as_ref().unwrap().borrow();
    let content_box = app.content_box().unwrap().0;
    assert_eq!(painted_element_box(&d.tree, content_box).2, 1000.0);
}
