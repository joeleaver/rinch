//! Issue #234, item 2: an outermost batch that wrote nothing no longer calls
//! the signal-change callbacks. These pin what a handler that writes **no
//! signal** must still get on desktop: a theme change made through
//! `rinch::update_theme` (`rinch_core::set_current_theme_css`) — state outside
//! the signal graph that the shell only reads on a `ReRender` — and a direct
//! `NodeHandle` mutation, which the shell finds on its own dirty-node check.
//! Found by PR #1134's review (F1).

use super::*;
use std::cell::Cell;
use std::rc::Rc;

const W: f32 = 800.0;
const H: f32 = 600.0;

fn color_of(app: &RinchApp, class: &str) -> Option<(u8, u8, u8)> {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let (_, n) = d
        .tree
        .nodes
        .iter()
        .find(|(_, n)| n.attributes.get("class").is_some_and(|c| c == class))
        .unwrap();
    n.computed_style
        .color
        .map(|c| c.to_rgba8())
        .map(|c| (c.r, c.g, c.b))
}

/// One desktop turn after `handler` ran as a dispatched event: the shell
/// turns a signal-change notification into a `ReRender` (drained on the next
/// wake), and winit always follows with `AboutToWait`. Returns whether any
/// step asked for a redraw.
fn desktop_turn(app: &mut RinchApp, handler: impl Fn() + 'static) -> bool {
    let posted = Rc::new(Cell::new(0u32));
    let p = posted.clone();
    let _sub = rinch_core::subscribe_signal_change(move || p.set(p.get() + 1));
    let id = rinch_core::events::register_handler(Rc::new(handler));
    rinch_core::events::dispatch_event(id);
    drop(_sub);
    let mut redraw = false;
    if posted.get() > 0 {
        let a = app.handle_event(
            PlatformEvent::UserEvent(UserEvent::ReRender),
            (W as u32, H as u32),
            1.0,
        );
        redraw |= a.contains(&AppAction::RequestRedraw);
    }
    let a = app.handle_event(PlatformEvent::AboutToWait, (W as u32, H as u32), 1.0);
    redraw |= a.contains(&AppAction::RequestRedraw);
    redraw
}

#[cfg(feature = "theme")]
fn mount_probe() -> RinchApp {
    rinch_core::set_current_theme_css(Some(":root { --probe-c: rgb(255, 0, 0); }".into()));
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let probe = scope.create_element("div");
        probe.set_attribute("class", "theme-pin");
        probe.set_attribute("style", "color: var(--probe-c); width: 10px; height: 10px");
        root.append_child(&probe);
        root
    });
    app.mount_component(W, H);
    app.resolve_and_repaint(W, H);
    app.handle_event(PlatformEvent::AboutToWait, (W as u32, H as u32), 1.0);
    assert_eq!(
        color_of(&app, "theme-pin"),
        Some((255, 0, 0)),
        "precondition"
    );
    app
}

/// A handler (a "Dark mode" menu item, a button, a timer) that calls
/// `rinch::update_theme` directly — the desktop toggle `theming.md` documents —
/// and writes no signal.
#[cfg(feature = "theme")]
#[test]
fn a_theme_change_from_a_no_write_handler_applies_on_desktop() {
    let mut app = mount_probe();
    let redraw = desktop_turn(&mut app, || {
        rinch_core::set_current_theme_css(Some(":root { --probe-c: rgb(0, 0, 255); }".into()));
    });
    let c = color_of(&app, "theme-pin");
    rinch_core::set_current_theme_css(None);
    assert_eq!(
        c,
        Some((0, 0, 255)),
        "the theme change reached the document"
    );
    assert!(redraw, "and asked for a frame");
}

/// Positive control: the same handler plus a write to an unobserved signal.
#[cfg(feature = "theme")]
#[test]
fn control_a_theme_change_beside_a_signal_write_applies() {
    let mut app = mount_probe();
    let sig = rinch_core::Signal::new(0);
    desktop_turn(&mut app, move || {
        rinch_core::set_current_theme_css(Some(":root { --probe-c: rgb(0, 0, 255); }".into()));
        sig.set(1);
    });
    let c = color_of(&app, "theme-pin");
    rinch_core::set_current_theme_css(None);
    assert_eq!(c, Some((0, 0, 255)));
}

/// A NodeHandle mutation from a no-write handler still gets its frame.
#[test]
fn a_node_handle_style_from_a_no_write_handler_gets_a_frame() {
    let holder: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let h2 = holder.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let probe = scope.create_element("div");
        probe.set_attribute("class", "p");
        probe.set_attribute("style", "color: rgb(255,0,0); width: 10px; height: 10px");
        root.append_child(&probe);
        *h2.borrow_mut() = Some(probe);
        root
    });
    app.mount_component(W, H);
    app.resolve_and_repaint(W, H);
    app.handle_event(PlatformEvent::AboutToWait, (W as u32, H as u32), 1.0);
    let h = holder.borrow().clone().unwrap();
    let redraw = desktop_turn(&mut app, move || h.set_style("color", "rgb(0, 0, 255)"));
    assert_eq!(color_of(&app, "p"), Some((0, 0, 255)));
    assert!(redraw);
}

/// A theme change made **outside** any batch — `run_on_main_thread` called on
/// the main thread, a surface event, a selection callback — is still picked
/// up by the next handler, even one that writes nothing (PR #1134 round-2
/// review, N1). Before #234 every outermost batch notified, so the user's next
/// click delivered it; a gate that compared a counter only across the batch
/// itself left it waiting for a batch that wrote a signal.
#[cfg(feature = "theme")]
#[test]
fn a_theme_change_outside_a_batch_is_picked_up_by_the_next_no_write_handler() {
    let mut app = mount_probe();
    rinch_core::set_current_theme_css(Some(":root { --probe-c: rgb(0, 0, 255); }".into()));
    // The host's own wake afterwards: nothing dirty, nothing notified.
    app.handle_event(PlatformEvent::AboutToWait, (W as u32, H as u32), 1.0);
    let before_click = color_of(&app, "theme-pin");
    desktop_turn(&mut app, || {});
    let c = color_of(&app, "theme-pin");
    rinch_core::set_current_theme_css(None);
    assert_eq!(
        before_click,
        Some((255, 0, 0)),
        "precondition: nothing applied it without a notification"
    );
    assert_eq!(c, Some((0, 0, 255)), "the next no-write click applies it");
}
