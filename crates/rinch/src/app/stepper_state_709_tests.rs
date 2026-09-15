//! The `Stepper` state derivation, read through a real Stylo cascade (#709).
//!
//! `rinch-components`' `stepper_state_709` says which class each step ends up
//! carrying and which glyph sits in its icon box. Neither claim is what an app
//! sees: the classes only matter because the component stylesheet paints the
//! three states differently, and the *alternates* a step leaves for its parent
//! only stay invisible because two separate things hide them.
//!
//! Two questions, therefore, that no mock-DOM fixture can answer:
//!
//! - does a derived state class actually reach the cascade, or has the
//!   derivation merely written a class nothing selects on?
//! - does a promoted alternate come out visible? It was rendered inside a
//!   `display: none` wrapper, so promoting the *wrapper* instead of the glyph
//!   inside it would pass every structural assertion and paint nothing.
//!
//! Since #716 a stepper also **parks** a glyph a step's props supplied when the
//! step stops drawing it, against an insertion or a keyed reorder moving the
//! step back — so an alternate left on the tree is no longer proof that a
//! promotion went wrong. What still discriminates is whether a *visible* one is
//! left, which is what the promotion test below asks.

use super::*;

use rinch_components::stepper::{Stepper, StepperStep};
use rinch_core::Component;
use rinch_dom::computed_style::DisplayValue;
use rinch_tabler_icons::TablerIcon;

const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// Mount `build`'s tree under the real theme **and** component stylesheets.
fn mount(build: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> RinchApp {
    let mut app = RinchApp::new(build);
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    {
        let doc = app.doc.as_ref().expect("a mounted document");
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

/// Every node carrying `class`, in document order.
///
/// A **walk from the body**, not a scan of `tree.nodes`: removing a node
/// detaches it without freeing its slab entry, so a scan of the arena still
/// finds every alternate this component dropped and would report the promotion
/// as having happened when it had not.
fn nodes_with_class(app: &RinchApp, class: &str) -> Vec<usize> {
    fn walk(d: &RinchDocument, id: usize, class: &str, out: &mut Vec<usize>) {
        let Some(node) = d.tree.get(id) else { return };
        if node
            .attributes
            .get("class")
            .is_some_and(|c| c.split_whitespace().any(|one| one == class))
        {
            out.push(id);
        }
        for child in node.children.clone() {
            walk(d, child, class, out);
        }
    }
    let doc = app.doc.as_ref().expect("a mounted document");
    let d = doc.borrow();
    let mut out = Vec::new();
    walk(&d, d.tree.body_id, class, &mut out);
    out
}

fn background_of(app: &RinchApp, id: usize) -> Option<peniko::Color> {
    let doc = app.doc.as_ref().expect("a mounted document");
    let d = doc.borrow();
    d.tree
        .get(id)
        .expect("the node is in the tree")
        .computed_style
        .background_color()
}

fn display_of(app: &RinchApp, id: usize) -> DisplayValue {
    let doc = app.doc.as_ref().expect("a mounted document");
    let d = doc.borrow();
    d.tree
        .get(id)
        .expect("the node is in the tree")
        .computed_style
        .display
}

/// Three steps under a stepper on `active: 1`, none of them naming a state.
fn three_steps(step_completed_icon: Option<TablerIcon>) -> RinchApp {
    mount(move |scope: &mut RenderScope| {
        let steps: Vec<NodeHandle> = (0..3)
            .map(|_| {
                StepperStep {
                    completed_icon: step_completed_icon,
                    ..Default::default()
                }
                .render(scope, &[])
            })
            .collect();
        Stepper {
            active: 1,
            ..Default::default()
        }
        .render(scope, &steps)
    })
}

#[test]
fn a_derived_state_class_reaches_the_cascade() {
    let app = three_steps(None);
    let boxes = nodes_with_class(&app, "rinch-stepper__step-icon");
    assert_eq!(boxes.len(), 3, "one icon box per step");

    let colours: Vec<Option<peniko::Color>> =
        boxes.iter().map(|id| background_of(&app, *id)).collect();

    assert_eq!(
        colours[0], colours[1],
        "the completed step and the in-progress one share the stepper's colour \
         — the sheet gives both `--rinch-stepper-color`"
    );
    assert_ne!(
        colours[1], colours[2],
        "and the step after `active` does not. Comparing the three against each \
         other rather than against a literal is what keeps this a test of the \
         derivation instead of a test of the palette"
    );
    assert!(
        colours[0].is_some(),
        "a resolved colour, not a `var()` that fell through to nothing — an \
         unresolved custom property makes the whole declaration invalid at \
         computed-value time, silently, and both comparisons above would then \
         be comparing `None` with `None`"
    );
}

#[test]
fn a_promoted_alternate_is_not_promoted_inside_its_hiding_wrapper() {
    let app = three_steps(Some(TablerIcon::CircleCheck));
    let alts = nodes_with_class(&app, "rinch-stepper__step-icon-alt");
    assert_eq!(
        alts.len(),
        2,
        "the completed step's alternate was promoted out of its wrapper and the \
         wrapper went; the two steps that are *not* completed keep theirs, \
         parked against a later insertion or reorder moving them into completed \
         (issue #716). #709 dropped those two"
    );
    for alt in alts {
        assert_eq!(
            display_of(&app, alt),
            DisplayValue::None,
            "and a parked alternate must not paint — it is an offer to the \
             parent, not content"
        );
    }

    let boxes = nodes_with_class(&app, "rinch-stepper__step-icon");
    let icon = {
        let doc = app.doc.as_ref().expect("a mounted document");
        let d = doc.borrow();
        *d.tree
            .get(boxes[0])
            .expect("the node is in the tree")
            .children
            .first()
            .expect("the completed step's box holds the promoted icon")
    };
    assert_ne!(
        display_of(&app, icon),
        DisplayValue::None,
        "the promoted glyph paints. It was rendered inside a `display: none` \
         wrapper, so promoting that wrapper instead would leave the step \
         looking empty while every structural assertion still passed"
    );
}

#[test]
fn an_unparented_step_keeps_its_alternates_hidden() {
    let app = mount(|scope: &mut RenderScope| {
        StepperStep {
            completed_icon: Some(TablerIcon::CircleCheck),
            ..Default::default()
        }
        .render(scope, &[])
    });

    let alts = nodes_with_class(&app, "rinch-stepper__step-icon-alt");
    assert_eq!(
        alts.len(),
        1,
        "a step with no `Stepper` above it has nobody to take its alternate \
         away, so it keeps it"
    );
    assert_eq!(
        display_of(&app, alts[0]),
        DisplayValue::None,
        "and it must not draw it — an alternate is an offer to a parent, not \
         content"
    );
}
