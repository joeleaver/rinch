//! The `Drawer`'s 300ms slide-in **does not run**, and this pins the deviation
//! (issue #703 changed it; issue #751 is where it gets restored).
//!
//! This file asserts something wrong on purpose. Read the next three paragraphs
//! before "fixing" it.
//!
//! # What happens
//!
//! `styles/drawer.rs` puts `display: none !important` on the drawer's **root**
//! and `transition: transform 300ms ease` on the **panel** underneath it, and
//! `drawer.rs`' one reactive effect removes the root's hidden class and adds the
//! panel's `--opened` class in a single batch. So the panel's ancestor stops
//! being `display: none` and the panel's own `transform` retargets in **one**
//! style pass.
//!
//! css-transitions-1 §3 starts no transition there: the before-change style of
//! an element that was not being rendered is its after-change style, so there is
//! nothing to animate from. rinch used to start one anyway; #703 stopped it, and
//! `display_none_transition_tests::showing_a_hidden_ancestor_whose_descendant_is_retargeted_does_not_transition`
//! is the rule this component meets.
//!
//! # Why the deviation is from the component, not from #703
//!
//! A browser does not animate this shape either — which is the whole reason
//! `@starting-style` and `transition-behavior: allow-discrete` exist — and
//! `rinch-components` ships **one** stylesheet to both backends, so the Drawer
//! has never animated on `rinch-web`. #703 moved desktop onto the web's
//! behaviour; it did not invent a new one. What it exposed is that the component
//! was relying on a rinch-only deviation for its open animation.
//!
//! The cure is the component's, and `Popover` already uses it
//! (`styles/popover.rs`): stay rendered and animate `opacity`/`visibility`
//! instead of toggling `display`. The other shape is two-phase — show first,
//! add the `--opened` class on the next frame — or `@keyframes`, which do run on
//! insertion. **Issue #751.** When that lands, this file is deleted and replaced
//! by a fixture asserting the animation *runs*, on both backends.
//!
//! # Why a fixture rather than a note
//!
//! Because `-p rinch-components` was green across the whole regression. A
//! shipped component lost its open animation and nothing in the suite noticed;
//! a paragraph in a PR body would have been the only record. This is the record
//! that fails when somebody changes it back.

use super::*;

use rinch_components::Drawer;
use rinch_core::{Component, Signal};

const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// The panel that carries `transition: transform 300ms ease`.
const PANEL: &str = "rinch-drawer";
/// The root that carries `display: none !important` while closed.
const ROOT: &str = "rinch-drawer__root";

/// The one node carrying `class` exactly, as a raw node id.
fn node_with_class(app: &RinchApp, class: &str) -> usize {
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
    matches[0]
}

fn running(app: &RinchApp, node: usize) -> usize {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .active_transitions
        .get(&node)
        .map(|m| m.len())
        .unwrap_or(0)
}

fn display_of(app: &RinchApp, node: usize) -> rinch_dom::computed_style::DisplayValue {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.get(node).unwrap().computed_style.display
}

/// The transitioned property, as the three arrays that carry it.
///
/// `TransformValue` has no `PartialEq`, and its `is_identity` flag describes the
/// **matrix** only — a `translateX(-100%)` leaves the matrix identity and lives
/// entirely in `pct_translate_w`, which is exactly the Drawer's closed state. So
/// the comparison has to be on the fields, and `pct_translate_w` is the one that
/// moves here.
fn transform_of(app: &RinchApp, node: usize) -> ([f64; 6], [f64; 2], [f64; 2]) {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let t = &d.tree.get(node).unwrap().computed_style.transform;
    (t.matrix, t.pct_translate_w, t.pct_translate_h)
}

/// How many `transition` declarations the cascade found on the node. Zero would
/// mean the fixture is measuring a component that declares no animation at all,
/// which is the fixed point every "0 transitions" assertion has to be kept off.
fn transition_specs(app: &RinchApp, node: usize) -> usize {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.get(node).unwrap().transition_specs.len()
}

/// A closed `Drawer` under the real component stylesheet, with the signal that
/// opens it.
fn mount_closed() -> (RinchApp, Signal<bool>) {
    let opened = Signal::new(false);
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let drawer = Drawer {
            opened_fn: Some(std::rc::Rc::new(move || opened.get())),
            position: "left".to_string(),
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
    (app, opened)
}

/// **A deviation, pinned deliberately. See the module doc and issue #751.**
///
/// Opening the drawer un-hides its root and retargets its panel's `transform`
/// in one pass, so §3 starts nothing and the panel arrives at its open position
/// instantly instead of sliding over 300ms.
#[test]
fn opening_the_drawer_does_not_run_its_300ms_slide() {
    let (mut app, opened) = mount_closed();

    let panel = node_with_class(&app, PANEL);
    let root = node_with_class(&app, ROOT);

    // Three preconditions, and each keeps this fixture off a fixed point where
    // "0 transitions" would be true for an uninteresting reason.
    assert_eq!(
        display_of(&app, root),
        rinch_dom::computed_style::DisplayValue::None,
        "precondition: the closed drawer's root is `display: none`, so the panel \
         under it is not being rendered"
    );
    assert!(
        transition_specs(&app, panel) > 0,
        "precondition: the panel really does declare a `transition` — without \
         this, zero running transitions would say nothing"
    );
    let closed_transform = transform_of(&app, panel);
    assert_eq!(
        closed_transform.1,
        [-1.0, 0.0],
        "precondition: closed, the panel is `translateX(-100%)` — off-screen to \
         the left — so opening it really is a change to the transitioned property"
    );

    opened.set(true);
    app.resolve_and_repaint(VIEWPORT.0 + 1.0, VIEWPORT.1);

    assert_eq!(
        display_of(&app, root),
        rinch_dom::computed_style::DisplayValue::Block,
        "the root is rendered now"
    );

    // These two assertions are one fact read two ways, and **both** of them
    // discriminate — which is worth saying, because the obvious third assertion
    // does not. "Did the panel's transform retarget in this pass?" cannot be
    // read off `computed_style` at all: while a transition runs, the cascade
    // writes the *interpolated* value back over the resolved one, so at t = 0 a
    // running transition and a refused one are told apart by `active_transitions`
    // and by where the value **landed**, never by comparing the value to the one
    // before the change. Measured against a build with #703's gate deleted: the
    // transform reads `[-1.0, 0.0]` there, identical to `closed_transform`,
    // because the slide has not advanced yet.
    assert_eq!(
        running(&app, panel),
        0,
        "DEVIATION (#751): the slide-in does not run. A browser does not animate \
         this shape either, and the Drawer has never animated on rinch-web; the \
         component has to stay rendered and animate opacity (as `Popover` does), \
         defer the `--opened` write by a frame, or use `@keyframes`"
    );
    assert_eq!(
        transform_of(&app, panel).1,
        [0.0, 0.0],
        "so the panel is at its open position immediately, rather than at the \
         closed {closed:?} a running transition would still be showing at t = 0",
        closed = closed_transform.1
    );
}
