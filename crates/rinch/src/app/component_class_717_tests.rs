//! The rsx `class:` prop on a component, through the real macro (issue #717).
//!
//! `rinch-components`' `class_prop_717` drives the same components over a mock
//! document and seeds the caller's class by hand, because a component cannot be
//! *invoked* from `rsx!` outside a crate that depends on `rinch` — the macro
//! writes absolute `rinch::core::…` paths. These four fixtures close that gap:
//! they let `generate_class_code` put the class on, so nothing here assumes
//! where or when the macro applies it.
//!
//! Two claims, and they are different:
//!
//! 1. **A literal `class:` survives the component's own reactive effect.** That
//!    is #717: the effect rebuilt the whole attribute from a string captured
//!    during `render`, and `generate_class_code` runs *after* `render` returns,
//!    so the first toggle threw the caller's class away.
//! 2. **A reactive `class:` and the component's modifier do not fight.** The
//!    caller's effect takes its own previous classes off before laying the new
//!    ones on (#647, the `class:` half of #665's "author's own last value"
//!    rule), and it must take off only *its* own — never the modifier the
//!    component is holding, and never accumulate its old values.
//!
//! Both are asserted **twice**, once each way. The whole-string rewrite #717
//! killed is correct on the render pass and only destroys on the second write,
//! so a fixture that toggles once and stops would pass against it.

// `rsx!` writes absolute `rinch::` paths, and this *is* the rinch crate.
use super::*;
use crate as rinch;
use rinch_components::{Checkbox, Switch};
use rinch_core::Signal;
use rinch_macros::rsx;

const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// The `class` attribute of the one node carrying `marker`.
///
/// A `marker` that has gone missing entirely is the #717 failure itself, so the
/// message prints every class string in the tree rather than only a count.
fn class_of(app: &RinchApp, marker: &str) -> String {
    let doc = app.doc.as_ref().expect("a mounted document");
    let d = doc.borrow();
    let all: Vec<String> = d
        .tree
        .nodes
        .iter()
        .filter_map(|(_, n)| n.attributes.get("class").cloned())
        .collect();
    let mut found: Vec<String> = all
        .iter()
        .filter(|c| c.split_whitespace().any(|one| one == marker))
        .cloned()
        .collect();
    assert_eq!(
        found.len(),
        1,
        "expected exactly one node carrying `{marker}`, found {}. \
         The tree's class attributes are: {:?}",
        found.len(),
        all
    );
    found.pop().unwrap()
}

fn classes(app: &RinchApp, marker: &str) -> Vec<String> {
    class_of(app, marker)
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

fn has(app: &RinchApp, marker: &str, class: &str) -> bool {
    classes(app, marker).iter().any(|c| c == class)
}

fn mount(build: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> RinchApp {
    let mut app = RinchApp::new(build);
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    app
}

#[test]
fn a_literal_class_on_a_checkbox_survives_both_directions_of_a_toggle() {
    let checked = Signal::new(false);
    let app = mount(move |__scope: &mut RenderScope| {
        rsx! {
            div {
                Checkbox {
                    class: "mine",
                    checked_fn: move || checked.get(),
                }
            }
        }
    });

    assert!(has(&app, "mine", "rinch-checkbox"), "precondition");

    checked.set(true);
    assert!(
        has(&app, "mine", "rinch-checkbox--checked"),
        "positive control: the modifier goes on — got `{}`",
        class_of(&app, "mine")
    );
    assert!(
        has(&app, "mine", "mine"),
        "#717: the caller's class went with the rewrite"
    );

    checked.set(false);
    assert!(!has(&app, "mine", "rinch-checkbox--checked"));
    assert!(has(&app, "mine", "mine"), "#717, unchecking");
}

#[test]
fn a_literal_class_on_a_switch_survives_both_directions_of_a_toggle() {
    let checked = Signal::new(false);
    let app = mount(move |__scope: &mut RenderScope| {
        rsx! {
            div {
                Switch {
                    class: "mine",
                    checked_fn: move || checked.get(),
                }
            }
        }
    });

    assert!(has(&app, "mine", "rinch-switch"), "precondition");

    checked.set(true);
    assert!(
        has(&app, "mine", "rinch-switch--checked"),
        "positive control — got `{}`",
        class_of(&app, "mine")
    );
    assert!(has(&app, "mine", "mine"), "#717");

    checked.set(false);
    assert!(!has(&app, "mine", "rinch-switch--checked"));
    assert!(has(&app, "mine", "mine"), "#717, unchecking");
}

/// The mirror of #717: the **caller's** effect rewriting the class, while the
/// component's modifier is on.
///
/// `generate_class_code` remembers the value it last wrote and removes those
/// words before adding the new ones, so a reactive `class:` neither accumulates
/// its own old values nor touches anything it did not write. The component's
/// modifier is one of the things it did not write.
#[test]
fn a_reactive_caller_class_neither_accumulates_nor_eats_the_modifier() {
    let checked = Signal::new(true);
    let hot = Signal::new(false);
    let app = mount(move |__scope: &mut RenderScope| {
        rsx! {
            div {
                Checkbox {
                    class: {move || if hot.get() { "hot" } else { "cool" }},
                    checked_fn: move || checked.get(),
                }
            }
        }
    });

    assert!(has(&app, "cool", "rinch-checkbox--checked"), "precondition");

    hot.set(true);
    let after = classes(&app, "hot");
    assert!(
        !after.iter().any(|c| c == "cool"),
        "the caller's effect must take its own last value off — got `{}`",
        after.join(" ")
    );
    assert!(
        after.iter().any(|c| c == "rinch-checkbox--checked"),
        "and must leave the component's modifier alone — got `{}`",
        after.join(" ")
    );

    // Back again, and once more, so an accumulating writer shows up as a
    // repeat rather than needing an exact-string comparison.
    hot.set(false);
    hot.set(true);
    let again = classes(&app, "hot");
    assert_eq!(
        again.iter().filter(|c| *c == "hot").count(),
        1,
        "a class must not accumulate across re-runs — got `{}`",
        again.join(" ")
    );
    assert!(
        again.iter().any(|c| c == "rinch-checkbox--checked"),
        "and the modifier survives every re-run — got `{}`",
        again.join(" ")
    );
}

/// The two effects in one tree, driven independently.
///
/// The component's checked effect is registered during `render`; the caller's
/// class effect is registered after it, by `generate_class_code`. Registration
/// order is execution order (#154), so this is also the ordering fixture: a
/// modifier written by the earlier effect must survive the later one's run.
#[test]
fn the_modifier_and_the_callers_class_are_independent() {
    let checked = Signal::new(false);
    let hot = Signal::new(false);
    let app = mount(move |__scope: &mut RenderScope| {
        rsx! {
            div {
                Switch {
                    class: {move || if hot.get() { "hot" } else { "cool" }},
                    checked_fn: move || checked.get(),
                }
            }
        }
    });

    checked.set(true);
    assert!(has(&app, "cool", "rinch-switch--checked"));

    // The caller's effect runs; the modifier is not its business.
    hot.set(true);
    assert!(
        has(&app, "hot", "rinch-switch--checked"),
        "the caller's class effect ate the modifier — got `{}`",
        class_of(&app, "hot")
    );

    // The component's effect runs; the caller's class is not its business.
    checked.set(false);
    assert!(!has(&app, "hot", "rinch-switch--checked"));
    assert!(
        has(&app, "hot", "hot"),
        "#717: got `{}`",
        class_of(&app, "hot")
    );
}
