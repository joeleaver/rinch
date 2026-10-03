//! Issue #1341 — found reviewing #1336: a report that a `Popover`'s dropdown
//! hit-tests to the backdrop over large stretches of its own interior, so
//! clicking inside an open popover could incorrectly dismiss it.
//!
//! **Investigated and found NOT to be a bug.** The report's reproduction
//! measured the dropdown's "own declared box" with
//! [`rinch_dom::paint::compute_absolute_position`], whose own doc comment
//! says plainly: *"It is not a screen rect... a CSS transform on the node...
//! moves it"* — it deliberately returns the **pre-transform** Taffy layout
//! position. `Popover`'s default (and every `top`/`bottom`, non-`-start`/
//! `-end`) position variant centers the dropdown under its target with
//! `transform: translateX(-50%) translateY(var(--rinch-popover-offset,
//! 8px))` (`styles/popover.rs`), so the dropdown's real, *painted* box sits
//! up to half the dropdown's own width to the left of the box
//! `compute_absolute_position` reports.
//!
//! Hit testing composes a node's CSS transform exactly as paint does
//! (`hit_testing::local_point`, #199) and tests the point against the
//! **painted** box — [`rinch_dom::paint::painted_border_box`], not
//! `compute_absolute_position`. A grid of points checked against the real
//! painted box below shows hit testing agrees with it at every single point,
//! and a same-markup/CSS probe in Chrome 153 (`elementFromPoint`) agrees with
//! the same transformed-box rule too (recorded in the PR body, not re-run
//! here since Chrome isn't available in this harness). So the dropdown DOES
//! correctly claim its own (painted) interior; the "backdrop" hits the
//! original report saw were points that were, once the transform is taken
//! into account, legitimately outside the dropdown.
//!
//! `DropdownMenu` and `Select`, which the issue named as likely sharing the
//! shape, do **not** use a centering `transform` on their panel (`left: 0` /
//! `right: 0` with a plain `margin-top`, `styles/dropdown_menu.rs`) — so
//! their `compute_absolute_position` already matches their painted box and
//! they are not subject to this measurement trap.
//!
//! This file exists to pin the correct (already-working) behaviour so a
//! future reviewer does not re-discover the same false positive, and to
//! document the mechanism for good.

use super::*;

use rinch_components::{Component, Popover, PopoverDropdown, PopoverTarget};
use rinch_core::Callback;
use std::cell::Cell;
use std::rc::Rc;

const VIEWPORT: (f32, f32) = (800.0, 600.0);

fn mount(opened: bool, onclose: Callback) -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let target_text = scope.create_text("Target");
        let target = PopoverTarget.render(scope, &[target_text]);
        let item = scope.create_text("content");
        let dropdown = PopoverDropdown.render(scope, &[item]);
        let popover = Popover {
            opened,
            with_arrow: false,
            position: "bottom".to_string(),
            close_on_click_outside: true,
            onclose: Some(onclose.clone()),
            ..Default::default()
        }
        .render(scope, &[target, dropdown]);
        // A plain positioning wrapper, well clear of every edge: the
        // dropdown centers under its target with a negative-offset
        // `transform` (see the module doc), and placing the target flush
        // against the viewport edge would push part of the dropdown's own
        // painted box off-screen, where "outside the box" and "off the
        // screen entirely" become the same untestable region.
        let wrapper = scope.create_element("div");
        wrapper.set_attribute("style", "position: absolute; left: 300px; top: 200px;");
        wrapper.append_child(&popover);
        wrapper
    });
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_theme::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    app
}

fn find_by_class(app: &RinchApp, class: &str) -> usize {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .nodes
        .iter()
        .find(|(_, n)| {
            n.attributes
                .get("class")
                .is_some_and(|c| c.split_whitespace().any(|one| one == class))
        })
        .map(|(id, _)| id)
        .expect(class)
}

fn tap(app: &mut RinchApp, x: f32, y: f32) {
    app.handle_event(
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
        (800, 600),
        1.0,
    );
    app.handle_event(
        PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Left,
        },
        (800, 600),
        1.0,
    );
}

/// Every point inside the dropdown's real, *painted* (post-transform) box
/// hits the dropdown subtree; every point outside it hits the backdrop.
/// `compute_absolute_position`'s pre-transform box is deliberately NOT used
/// here — that is the exact measurement the original report got wrong.
#[test]
fn hit_testing_follows_the_dropdowns_painted_box_not_its_pre_transform_layout_box() {
    let hits = Rc::new(Cell::new(0usize));
    let h = hits.clone();
    let onclose = Callback::new(move || h.set(h.get() + 1));
    let app = mount(true, onclose);

    let dropdown = find_by_class(&app, "rinch-popover__dropdown");
    let backdrop = find_by_class(&app, "rinch-popover__backdrop");

    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let painted = rinch_dom::paint::painted_border_box(&d.tree, dropdown, 1.0);

    // Sanity: the painted box must actually differ from the pre-transform
    // layout box, or this fixture would be sitting on a fixed point (the
    // exact trap `brief-common.md` #3 warns about) and could pass whether or
    // not hit testing honours the transform at all.
    let (pre_x, pre_y) = rinch_dom::paint::compute_absolute_position(&d.tree, dropdown, 1.0);
    assert!(
        (painted.x0 - pre_x).abs() > 10.0,
        "fixture assumption broken: painted box ({painted:?}) must differ \
         from the pre-transform layout position ({pre_x},{pre_y}) by more \
         than a few px, or this test can't distinguish correct from broken \
         hit testing"
    );

    // A grid spanning well outside the painted box on every side, through
    // its interior, on both sides of each edge. Clamped into the 800x600
    // viewport (the backdrop's own box) so an "outside the dropdown" probe
    // stays inside the backdrop rather than off the edge of the screen,
    // where nothing is hit at all and the assertion would be meaningless.
    let margin = 20.0_f64;
    let xs = [
        (painted.x0 - margin).clamp(1.0, 799.0),
        painted.x0 + 1.0,
        (painted.x0 + painted.x1) / 2.0,
        painted.x1 - 1.0,
        (painted.x1 + margin).clamp(1.0, 799.0),
    ];
    let ys = [
        (painted.y0 - margin).clamp(1.0, 599.0),
        painted.y0 + 1.0,
        (painted.y0 + painted.y1) / 2.0,
        painted.y1 - 1.0,
        (painted.y1 + margin).clamp(1.0, 599.0),
    ];

    let mut checked_inside = 0;
    let mut checked_outside = 0;
    for &y in &ys {
        for &x in &xs {
            let inside = x >= painted.x0 && x <= painted.x1 && y >= painted.y0 && y <= painted.y1;
            let hit = crate::app::hit_testing::hit_test(&d.tree, x as f32, y as f32);
            if inside {
                checked_inside += 1;
                assert_eq!(
                    hit,
                    Some(dropdown),
                    "point ({x},{y}) is inside the dropdown's painted box \
                     {painted:?} but hit {hit:?} instead of the dropdown \
                     ({dropdown})"
                );
            } else {
                checked_outside += 1;
                assert_eq!(
                    hit,
                    Some(backdrop),
                    "point ({x},{y}) is outside the dropdown's painted box \
                     {painted:?} but hit {hit:?} instead of the backdrop \
                     ({backdrop})"
                );
            }
        }
    }
    // Positive control: both buckets were actually exercised.
    assert!(checked_inside >= 4, "grid missed the dropdown interior");
    assert!(checked_outside >= 10, "grid missed the backdrop region");
}

/// The same geometry through the real click path: a tap anywhere inside the
/// dropdown's painted box must NOT fire `onclose` (the backdrop's handler),
/// and a tap outside it must.
#[test]
fn clicking_inside_the_painted_dropdown_does_not_dismiss_it() {
    let hits = Rc::new(Cell::new(0usize));
    let h = hits.clone();
    let onclose = Callback::new(move || h.set(h.get() + 1));
    let mut app = mount(true, onclose);

    let dropdown = find_by_class(&app, "rinch-popover__dropdown");
    let painted = {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        rinch_dom::paint::painted_border_box(&d.tree, dropdown, 1.0)
    };

    // Several points spread across the painted interior, not just its
    // center (a fixed point every reasonable implementation gets right) or
    // one axis — including near its edges, which is exactly where the
    // original report found the discrepancy.
    let probes = [
        (
            (painted.x0 + painted.x1) / 2.0,
            (painted.y0 + painted.y1) / 2.0,
        ),
        (painted.x0 + 2.0, painted.y0 + 2.0),
        (painted.x1 - 2.0, painted.y0 + 2.0),
        (painted.x0 + 2.0, painted.y1 - 2.0),
        (painted.x1 - 2.0, painted.y1 - 2.0),
    ];
    for (x, y) in probes {
        tap(&mut app, x as f32, y as f32);
        assert_eq!(
            hits.get(),
            0,
            "tap at ({x},{y}), inside the dropdown's painted box {painted:?}, \
             fired onclose — the backdrop stole a click meant for the dropdown"
        );
    }

    // And just outside the painted box, on each side, DOES dismiss — proving
    // the backdrop is still reachable and the test isn't vacuously passing
    // because nothing can ever reach onclose. Clamped into the 800x600
    // viewport (the backdrop's own box): the dropdown's painted box can sit
    // partly off-screen (it is centered under its target here, #204's
    // viewport-relative absolute), and a probe that steps off the edge of
    // the screen hits nothing at all rather than the backdrop.
    let outside_probes = [
        (
            (painted.x0 - 5.0).clamp(1.0, 799.0),
            (painted.y0 + painted.y1) / 2.0,
        ),
        (
            (painted.x1 + 5.0).clamp(1.0, 799.0),
            (painted.y0 + painted.y1) / 2.0,
        ),
        (
            (painted.x0 + painted.x1) / 2.0,
            (painted.y0 - 5.0).clamp(1.0, 599.0),
        ),
        (
            (painted.x0 + painted.x1) / 2.0,
            (painted.y1 + 5.0).clamp(1.0, 599.0),
        ),
    ];
    for (x, y) in outside_probes {
        tap(&mut app, x as f32, y as f32);
    }
    assert_eq!(
        hits.get(),
        outside_probes.len(),
        "a tap just outside the dropdown's painted box on every side should \
         have dismissed it every time"
    );
}
