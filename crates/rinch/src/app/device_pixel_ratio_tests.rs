//! The shell half of issue #211: the display scale factor reaches Stylo's
//! `device_pixel_ratio` at mount and on a runtime `ScaleFactorChanged`.
//!
//! The rinch-dom half (storage, Device rebuild survival) is pinned by
//! `crates/rinch-dom/tests/rem_tests.rs`; these tests pin the plumbing above
//! it — `RinchApp::set_device_pixel_ratio` before `mount_component`, and the
//! `PlatformEvent::ScaleFactorChanged` arm of `handle_event`.
//!
//! The fixture trap here is scale 1.0: at dpr 1 a working push and the
//! hard-coded default it replaced are indistinguishable, so every
//! discriminating assertion runs at a scale != 1, and the fractional tests
//! run at 1.25 with a query that matches (1.2dppx) and one that does not
//! (2dppx), so a value rounded to either integer fails.

use super::*;
use rinch_core::dom::NodeId;
use std::cell::Cell;

/// `.x` is 100x10; at >= 2dppx the width becomes 200, at >= 1.2dppx the
/// height becomes 20. Both gated properties are layout-visible, so the
/// assertions read straight from the layout tree.
const MEDIA_CSS: &str = "\
    .x { width: 100px; height: 10px } \
    @media (min-resolution: 2dppx) { .x { width: 200px } } \
    @media (min-resolution: 1.2dppx) { .x { height: 20px } }";

/// An app whose component carries the stylesheet as a `<style>` element (the
/// same pipeline a real app's CSS takes) and one `.x` div, whose NodeId is
/// reported through `target`.
fn app_with_gated_target(target: Rc<Cell<Option<NodeId>>>) -> RinchApp {
    RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let style = scope.create_element("style");
        let css = scope.create_text(MEDIA_CSS);
        style.append_child(&css);
        root.append_child(&style);
        let div = scope.create_element("div");
        div.set_attribute("class", "x");
        target.set(Some(div.node_id()));
        root.append_child(&div);
        root
    })
}

fn target_size(app: &RinchApp, target: &Rc<Cell<Option<NodeId>>>) -> (f32, f32) {
    let id = target.get().expect("component ran");
    let doc = app.doc.as_ref().expect("mounted").borrow();
    let node = doc.tree.get(id.0).expect("target node exists");
    (node.layout.width, node.layout.height)
}

/// A document mounted on a 2x display resolves `resolution` media queries
/// immediately — not after the first resize or event.
///
/// Kills: dropping the `set_device_pixel_ratio` push in `mount_component`
/// (or the pre-mount field store in `RinchApp::set_device_pixel_ratio`).
#[test]
fn mount_at_2x_resolves_resolution_media_queries_immediately() {
    let target = Rc::new(Cell::new(None));
    let mut app = app_with_gated_target(target.clone());
    app.set_device_pixel_ratio(2.0);
    app.mount_component(800.0, 600.0);

    assert_eq!(
        target_size(&app, &target),
        (200.0, 20.0),
        "both resolution-gated rules must apply at mount, before any event"
    );
}

/// The control for the test above: with no push the queries must NOT match.
/// Without this the 2x test could pass vacuously (a media query that always
/// matches would satisfy it with the push deleted).
#[test]
fn mount_at_default_scale_leaves_resolution_queries_unmatched() {
    let target = Rc::new(Cell::new(None));
    let mut app = app_with_gated_target(target.clone());
    app.mount_component(800.0, 600.0);

    assert_eq!(
        target_size(&app, &target),
        (100.0, 10.0),
        "at the 1.0 default neither gated rule applies"
    );
}

/// A runtime `ScaleFactorChanged` (the window dragged to a display with a
/// different scale) restyles the live document: the event marks it dirty and
/// the next resolve — the desktop paint preamble, embed's post-event check —
/// applies the newly-matching rules.
///
/// Kills: the arm ignoring the event's payload (the pre-#211 code), and a
/// `RinchApp::set_device_pixel_ratio` that stores the field without
/// forwarding to the mounted document.
#[test]
fn scale_factor_changed_event_restyles_the_live_document() {
    let target = Rc::new(Cell::new(None));
    let mut app = app_with_gated_target(target.clone());
    app.mount_component(800.0, 600.0);
    assert_eq!(target_size(&app, &target), (100.0, 10.0));

    let actions = app.handle_event(PlatformEvent::ScaleFactorChanged(2.0), (1600, 1200), 2.0);
    assert!(
        actions.contains(&AppAction::RequestRedraw),
        "the shell is asked to repaint"
    );
    assert!(
        app.has_pending_layout(),
        "the event must leave the document dirty — this flag is what makes \
         the paint preamble re-resolve before the next frame"
    );

    // The resolve every production path performs before painting.
    app.resolve_and_repaint(800.0, 600.0);
    assert_eq!(
        target_size(&app, &target),
        (200.0, 20.0),
        "at 2x both gated rules now apply"
    );
}

/// Fractional scale, both directions. 1.25 matches 1.2dppx but not 2dppx —
/// a dpr rounded down to 1 misses the height rule, one rounded up to 2 hits
/// the width rule. Scaling back down to 1.0 must un-apply the rule (kills a
/// latch that only ever raises the value).
#[test]
fn fractional_scale_matches_fractionally_and_unwinds() {
    let target = Rc::new(Cell::new(None));
    let mut app = app_with_gated_target(target.clone());
    app.set_device_pixel_ratio(1.25);
    app.mount_component(1000.0, 750.0);

    assert_eq!(
        target_size(&app, &target),
        (100.0, 20.0),
        "1.25 matches the 1.2dppx rule and not the 2dppx one"
    );

    app.handle_event(PlatformEvent::ScaleFactorChanged(1.0), (1000, 750), 1.0);
    app.resolve_and_repaint(1000.0, 750.0);
    assert_eq!(
        target_size(&app, &target),
        (100.0, 10.0),
        "back at 1.0 the fractional rule un-applies"
    );
}
