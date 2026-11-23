//! What the real component stylesheet makes of #2195's `Stepper` content
//! region and #2196's `BorderlessWindow` titlebar height.
//!
//! The structural halves are in `rinch-components`' `stepper_content_2195` and
//! `borderless_window_2196` tests, on the mock document. These read computed
//! `display` and laid-out boxes, which is where the bugs showed: every step's
//! content was `display: none` under `.rinch-stepper__step-content`, a
//! `StepperCompleted` was laid out whatever `active` was, and the titlebar was
//! 36px in CSS whatever the Rust side thought.

use super::*;

use rinch_components::{BorderlessWindow, Stepper, StepperCompleted, StepperStep};
use rinch_core::Component;
use rinch_dom::computed_style::DisplayValue;

const VIEWPORT: (f32, f32) = (800.0, 600.0);

fn mount(build: impl Fn(&mut RenderScope) -> NodeHandle + 'static) -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let child = build(scope);
        root.append_child(&child);
        root
    });
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    app
}

fn nodes_with_class(app: &RinchApp, class: &str) -> Vec<u64> {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let mut found: Vec<u64> = d
        .tree
        .nodes
        .iter()
        .filter(|(_, n)| {
            n.attributes
                .get("class")
                .is_some_and(|c| c.split_whitespace().any(|one| one == class))
        })
        .map(|(id, _)| id)
        .collect();
    found.sort_unstable();
    found
}

fn one(app: &RinchApp, class: &str) -> u64 {
    let found = nodes_with_class(app, class);
    assert_eq!(found.len(), 1, "exactly one `{class}`");
    found[0]
}

fn display(app: &RinchApp, node: u64) -> DisplayValue {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree.get(node).unwrap().computed_style.display
}

/// `(x, y, width, height)` on screen.
fn abs_box(app: &RinchApp, node: u64) -> (f32, f32, f32, f32) {
    let d = app.doc.as_ref().unwrap().borrow();
    crate::app::hit_testing::painted_element_box(&d.tree, node)
}

/// A step whose content is one 40px-tall `.content-{name}`.
fn step(scope: &mut RenderScope, name: &str) -> NodeHandle {
    let body = scope.create_element("div");
    body.set_attribute("class", &format!("content-{name}"));
    body.set_attribute("style", "height: 40px");
    StepperStep {
        label: name.to_string(),
        ..Default::default()
    }
    .render(scope, &[body])
}

fn completed(scope: &mut RenderScope) -> NodeHandle {
    let body = scope.create_element("div");
    body.set_attribute("class", "content-done");
    body.set_attribute("style", "height: 40px");
    StepperCompleted.render(scope, &[body])
}

fn stepper_app(active: u32) -> RinchApp {
    mount(move |scope| {
        let kids = [
            step(scope, "one"),
            step(scope, "two"),
            step(scope, "three"),
            completed(scope),
        ];
        Stepper {
            active,
            ..Default::default()
        }
        .render(scope, &kids)
    })
}

#[test]
fn the_active_steps_content_is_laid_out_below_the_steps() {
    let app = stepper_app(1);

    let shown = one(&app, "content-two");
    let (_, y, w, h) = abs_box(&app, shown);
    assert_eq!(h, 40.0, "step two's content is laid out at its own height");
    assert!(w > 0.0, "and has a width");
    let (_, steps_y, _, steps_h) = abs_box(&app, one(&app, "rinch-stepper__steps"));
    assert!(
        y >= steps_y + steps_h,
        "below the row of steps: content at y {y}, steps end at {}",
        steps_y + steps_h
    );

    for hidden in ["content-one", "content-three", "content-done"] {
        let (.., h) = abs_box(&app, one(&app, hidden));
        assert_eq!(h, 0.0, "`{hidden}` is not shown while step two is active");
    }
    let completed = one(&app, "rinch-stepper__completed");
    assert_eq!(display(&app, completed), DisplayValue::None);
}

#[test]
fn the_completed_block_is_laid_out_once_active_is_past_the_last_step() {
    let app = stepper_app(3);
    let completed = one(&app, "rinch-stepper__completed");
    assert_ne!(display(&app, completed), DisplayValue::None);
    let (.., h) = abs_box(&app, one(&app, "content-done"));
    assert_eq!(h, 40.0, "the completed block's content is shown");
    for hidden in ["content-one", "content-two", "content-three"] {
        let (.., h) = abs_box(&app, one(&app, hidden));
        assert_eq!(h, 0.0, "`{hidden}` is not shown past the end");
    }
}

#[test]
fn the_titlebar_takes_its_height_from_the_prop() {
    for height in [36_u32, 52] {
        let app = mount(move |scope| {
            let body = scope.create_element("div");
            body.set_attribute("class", "page");
            body.set_attribute("style", "height: 10px");
            BorderlessWindow {
                titlebar_height: height,
                // Not the host desktop's layout: a GNOME host would draw no
                // minimize/maximize, and this reads the close button.
                controls_layout: Some(
                    rinch_components::borderless_window::WindowControlsLayout::windows(),
                ),
                ..Default::default()
            }
            .render(scope, &[body])
        });
        let h = height as f32;
        let (_, ty, _, th) = abs_box(&app, one(&app, "rinch-borderlesswindow__titlebar"));
        assert_eq!(th, h, "the titlebar is {height}px tall");
        let (_, cy, ..) = abs_box(&app, one(&app, "rinch-borderlesswindow__content"));
        assert_eq!(cy, ty + h, "the content starts under it");
        let (.., close_h) = abs_box(&app, one(&app, "rinch-borderlesswindow__control--close"));
        assert_eq!(close_h, h, "the control buttons fill it");
    }
}
