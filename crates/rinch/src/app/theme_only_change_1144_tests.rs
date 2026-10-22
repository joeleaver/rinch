//! #1144: a theme change that dirties no node is applied by every host.
//!
//! `rinch::update_theme` (any `set_current_theme_css`) and an embed
//! context's owned theme dirty no node: the change is found only by
//! `resolve_and_repaint` comparing `theme_key` with `last_theme_key`. The
//! Android loop resolves only when `has_pending_layout` says so, which did
//! not ask, so a dark-mode toggle there restyled nothing unless the same turn
//! dirtied a node. Both "is there anything to do?" predicates now ask.

use super::*;

const W: f32 = 800.0;
const H: f32 = 600.0;

const RED: &str = ":root { --probe-c: rgb(255, 0, 0); }";
const BLUE: &str = ":root { --probe-c: rgb(0, 0, 255); }";

fn color_of(app: &RinchApp) -> Option<(u8, u8, u8)> {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let (_, n) = d
        .tree
        .nodes
        .iter()
        .find(|(_, n)| n.attributes.get("class").is_some_and(|c| c == "theme-pin"))
        .unwrap();
    n.computed_style
        .color
        .map(|c| c.to_rgba8())
        .map(|c| (c.r, c.g, c.b))
}

/// Mounted under RED, laid out, and idle: nothing is owed.
fn mount_idle() -> RinchApp {
    rinch_core::set_current_theme_css(Some(RED.into()));
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
    assert_eq!(color_of(&app), Some((255, 0, 0)), "precondition: red");
    assert!(!app.has_pending_layout(), "precondition: idle");
    assert!(!app.has_dirty_nodes(), "precondition: idle");
    app
}

/// One Android loop iteration as `android_runtime` runs it: turn the clock,
/// then resolve only when the frame says layout is pending.
fn android_iteration(app: &mut RinchApp) {
    let frame = crate::shell::android_frame::pump_frame(app, (W as u32, H as u32), 1.0);
    if frame.pending_layout {
        app.resolve_and_repaint(W, H);
    }
}

#[test]
fn the_android_loop_applies_a_theme_change_that_dirties_no_node() {
    let mut app = mount_idle();
    // What `rinch::update_theme(&dark)` does from a menu item or a timer: the
    // slot moves, no node is touched.
    rinch_core::set_current_theme_css(Some(BLUE.into()));
    android_iteration(&mut app);
    let c = color_of(&app);
    rinch_core::set_current_theme_css(None);
    assert_eq!(c, Some((0, 0, 255)), "the loop restyled for the new theme");
}

#[test]
fn a_theme_change_is_owed_until_applied_and_then_nothing_is() {
    let mut app = mount_idle();
    rinch_core::set_current_theme_css(Some(BLUE.into()));
    let owed = (app.has_pending_layout(), app.has_dirty_nodes());
    app.resolve_and_repaint(W, H);
    let after = (app.has_pending_layout(), app.has_dirty_nodes());
    // The same theme set again moves the generation only if the CSS differs.
    rinch_core::set_current_theme_css(Some(BLUE.into()));
    let same_again = (app.has_pending_layout(), app.has_dirty_nodes());
    rinch_core::set_current_theme_css(None);
    assert_eq!(owed, (true, true), "both predicates see the change");
    assert_eq!(after, (false, false), "applied, the loop goes idle");
    assert_eq!(
        same_again,
        (false, false),
        "an identical theme owes nothing"
    );
}

/// An embed context's own theme (`RinchContext::set_theme`) is keyed by the
/// owned generation, not the thread slot.
#[test]
fn an_owned_theme_change_is_owed_too() {
    let mut app = mount_idle();
    app.set_owned_theme_css(Some(BLUE.into()));
    let owed = app.has_pending_layout();
    android_iteration(&mut app);
    let c = color_of(&app);
    let after = app.has_pending_layout();
    rinch_core::set_current_theme_css(None);
    assert!(owed, "an owned theme change is owed");
    assert_eq!(c, Some((0, 0, 255)));
    assert!(!after);
}
