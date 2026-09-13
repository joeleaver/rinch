//! `Modal::overlay_opacity` and `Drawer::overlay_opacity`, end to end (#474).
//!
//! Both components already *published* their opacity as a custom property on
//! the overlay element. Neither stylesheet spent it: both hard-coded
//! `background-color: rgba(0, 0, 0, 0.75)`, so the prop was declared,
//! documented and inert — the same shape as the five `z_index` props, one layer
//! further in. Found by the review of #646, which also showed why the scan in
//! `no_dead_props` could not see it: a prop read into a variable no rule
//! consumes *is* read, as far as source text goes.
//!
//! The level rides the **alpha channel** rather than an `opacity` declaration.
//! `LoadingOverlay` uses `opacity`, and can: its backdrop is an opaque
//! `--rinch-color-body`. These two are already 75% transparent, so an `opacity`
//! would compose with that alpha (0.5 asked → 0.375 painted) and would make the
//! overlay a stacking context besides. The fallback in each rule is the number
//! that was hard-coded there, so an unset prop paints exactly as before —
//! asserted below rather than assumed.

use super::*;

use rinch_components::{Drawer, Modal};
use rinch_core::Component;

const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// Mount one overlay of each kind at `overlay_opacity`, under the real sheet.
fn mount(overlay_opacity: Option<f32>) -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let modal = Modal {
            opened: true,
            overlay_opacity,
            ..Default::default()
        }
        .render(scope, &[]);
        root.append_child(&modal);
        let drawer = Drawer {
            opened: true,
            overlay_opacity,
            ..Default::default()
        }
        .render(scope, &[]);
        root.append_child(&drawer);
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

/// The alpha of the background of the one node carrying `class` exactly.
fn backdrop_alpha(app: &RinchApp, class: &str) -> f32 {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let matches: Vec<usize> = d
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
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one node carrying `{class}`, found {}",
        matches.len()
    );
    match &d
        .tree
        .get(matches[0])
        .expect("the node is in the tree")
        .computed_style
        .background
    {
        rinch_dom::computed_style::BackgroundValue::Color(c) => c.components[3],
        other => panic!("`{class}` has no background colour: {other:?}"),
    }
}

/// 8-bit colour, so 0.75 arrives as 191/255. Compare at that resolution.
fn assert_alpha(got: f32, want: f32, what: &str) {
    assert!(
        (got - want).abs() < 0.005,
        "{what}: alpha {got} is not {want}"
    );
}

const OVERLAYS: [&str; 2] = ["rinch-modal__overlay", "rinch-drawer__overlay"];

#[test]
fn an_unset_overlay_opacity_paints_the_level_that_was_hard_coded() {
    let app = mount(None);
    for class in OVERLAYS {
        assert_alpha(backdrop_alpha(&app, class), 0.75, class);
    }
}

#[test]
fn overlay_opacity_moves_the_backdrop_alpha() {
    // 0.25 rather than 0.5: it is not the midpoint of anything, and it differs
    // from the 0.75 default in both directions of rounding.
    let app = mount(Some(0.25));
    for class in OVERLAYS {
        assert_alpha(backdrop_alpha(&app, class), 0.25, class);
    }

    // Fully opaque and fully clear both survive the substitution — the two ends
    // a caller reaches for ("solid scrim", "no dimming").
    let opaque = mount(Some(1.0));
    let clear = mount(Some(0.0));
    for class in OVERLAYS {
        assert_alpha(backdrop_alpha(&opaque, class), 1.0, class);
        assert_alpha(backdrop_alpha(&clear, class), 0.0, class);
    }
}
