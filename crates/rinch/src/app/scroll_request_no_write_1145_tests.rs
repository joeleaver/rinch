//! #1145: a `NodeHandle::scroll_into_view` / `scroll_to_fraction` asked for by
//! code that changes nothing else — a "jump to" button, a find-next — is
//! applied on the next turn of the loop.
//!
//! The requests only queue on the tree (`scroll_into_view_requests`,
//! `scroll_to_fraction_requests`), and `apply_scroll_into_view` drains them
//! inside `resolve_and_repaint`. Neither "is there anything to do?" predicate
//! counted the queues and the resolve's short-circuit did not either, so the
//! request waited for an unrelated change to lay the document out. A request
//! made beside a DOM write was always applied (the control below).
//!
//! The scroller is 100px over 1000px of spacer, so the target's offset is far
//! from 0 (the fixed point a broken drain also reads).

use super::*;
use rinch_platform::PlatformEvent;
use std::cell::RefCell;
use std::rc::Rc;

const W: f32 = 800.0;
const H: f32 = 600.0;

/// Mount the scroller; answer the app, the scroller's id and the target's handle.
fn mount() -> (RinchApp, u64, NodeHandle) {
    let held: Rc<RefCell<Option<(NodeHandle, NodeHandle)>>> = Rc::new(RefCell::new(None));
    let h = held.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "height: 100px; overflow-y: auto; width: 200px");
        let tall = scope.create_element("div");
        tall.set_attribute("style", "height: 1000px");
        let target = scope.create_element("div");
        target.set_attribute("style", "height: 20px");
        let after = scope.create_element("div");
        after.set_attribute("style", "height: 400px");
        root.append_child(&tall);
        root.append_child(&target);
        root.append_child(&after);
        *h.borrow_mut() = Some((root.clone(), target));
        root
    });
    app.mount_component(W, H);
    app.resolve_and_repaint(W, H);
    app.handle_event(PlatformEvent::AboutToWait, (W as u32, H as u32), 1.0);
    let (root, target) = held.borrow().clone().expect("rendered");
    (app, root.node_id().0, target)
}

fn scroll_top(app: &RinchApp, id: u64) -> f64 {
    let doc = app.doc.as_ref().expect("mounted");
    doc.borrow().scroll_top(rinch_core::dom::NodeId(id))
}

/// One turn of the desktop loop for a click whose handler is `handler`: the
/// handler runs as a batch, a `ReRender` follows only if it notified, and
/// `AboutToWait` closes the turn. Answers whether either asked for a redraw.
fn click_turn(app: &mut RinchApp, handler: impl Fn() + 'static) -> bool {
    let posted = Rc::new(std::cell::Cell::new(0u32));
    let p = posted.clone();
    let sub = rinch_core::subscribe_signal_change(move || p.set(p.get() + 1));
    let id = rinch_core::events::register_handler(Rc::new(handler));
    rinch_core::events::dispatch_event(id);
    drop(sub);
    let mut redraw = false;
    if posted.get() > 0 {
        let a = app.handle_event(
            PlatformEvent::UserEvent(rinch_platform::UserEvent::ReRender),
            (W as u32, H as u32),
            1.0,
        );
        redraw |= a.contains(&AppAction::RequestRedraw);
    }
    let a = app.handle_event(PlatformEvent::AboutToWait, (W as u32, H as u32), 1.0);
    redraw | a.contains(&AppAction::RequestRedraw)
}

#[test]
fn scroll_into_view_from_a_handler_that_writes_nothing_scrolls() {
    let (mut app, scroller, target) = mount();
    assert_eq!(scroll_top(&app, scroller), 0.0);
    let redraw = click_turn(&mut app, move || target.scroll_into_view());
    // The target spans 1000..1020 in a 100px view: nearest puts its bottom
    // at the view's bottom.
    assert_eq!(scroll_top(&app, scroller), 920.0, "the request was applied");
    assert!(redraw, "and the scroll asked for a frame");
}

#[test]
fn scroll_to_fraction_from_a_handler_that_writes_nothing_scrolls() {
    let (mut app, scroller, target) = mount();
    click_turn(&mut app, move || target.scroll_to_fraction(0.25, 0.0));
    // The target's top at a quarter of the 100px view: 1000 - 25.
    assert_eq!(
        scroll_top(&app, scroller),
        975.0,
        "the placement was applied"
    );
}

/// The control the issue was found with: the same request beside a DOM write
/// was applied before the fix too.
#[test]
fn scroll_into_view_beside_a_dom_write_scrolls() {
    let (mut app, scroller, target) = mount();
    click_turn(&mut app, move || {
        target.set_style("color", "red");
        target.scroll_into_view();
    });
    assert_eq!(scroll_top(&app, scroller), 920.0);
}

/// Both predicates every host asks — the `AboutToWait` frame clock, the
/// desktop wake and paint preamble, Android's loop, embed's `update` and
/// `needs_update` — see a queued request, and stop seeing it once it is
/// applied (so it does not keep a loop spinning).
#[test]
fn a_queued_request_is_pending_work_until_it_is_applied() {
    let (mut app, scroller, target) = mount();
    assert!(!app.has_dirty_nodes() && !app.has_pending_layout(), "idle");
    target.scroll_into_view();
    assert!(app.has_dirty_nodes(), "has_dirty_nodes counts the queue");
    assert!(
        app.has_pending_layout(),
        "has_pending_layout counts the queue"
    );
    assert!(
        app.resolve_and_repaint(W, H),
        "the resolve does not short-circuit"
    );
    assert_eq!(scroll_top(&app, scroller), 920.0);
    // The applied scroll dirties paint, which the next frame consumes; the
    // queue itself is empty now.
    let doc = app.doc.as_ref().unwrap().clone();
    let d = doc.borrow();
    assert!(d.tree.scroll_into_view_requests.is_empty());
    assert!(d.tree.scroll_to_fraction_requests.is_empty());
}

/// The fraction queue on its own is pending work too.
#[test]
fn a_queued_placement_is_pending_work() {
    let (app, _, target) = mount();
    target.scroll_to_fraction(0.5, 0.0);
    assert!(app.has_dirty_nodes() && app.has_pending_layout());
}

/// A request with no scroll container to move is drained and dropped, not
/// left pending: the loop goes idle.
#[test]
fn a_request_with_no_scroller_does_not_stay_pending() {
    let held: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let h = held.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "height: 20px");
        *h.borrow_mut() = Some(root.clone());
        root
    });
    app.mount_component(W, H);
    app.resolve_and_repaint(W, H);
    app.handle_event(PlatformEvent::AboutToWait, (W as u32, H as u32), 1.0);
    let n = held.borrow().clone().unwrap();
    n.scroll_into_view();
    assert!(app.has_dirty_nodes());
    app.handle_event(PlatformEvent::AboutToWait, (W as u32, H as u32), 1.0);
    assert!(
        !app.has_dirty_nodes() && !app.has_pending_layout(),
        "idle again"
    );
}
