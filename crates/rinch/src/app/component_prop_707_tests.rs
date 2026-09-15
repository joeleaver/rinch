//! `Accordion::disable_chevron_rotation` — the one prop #707 wired whose effect
//! is a **cascade** — read as computed style rather than as markup.
//!
//! `rinch-components`' `prop_wiring_707` says the attribute is published and the
//! sheet carries a rule for it. Neither can see what Stylo makes of the two
//! together, and that is the whole question here: the prop has to *beat* the
//! rotation, not merely coexist with it. Both declarations set `transform` on
//! the same element, so which one applies is decided by specificity — and the
//! rotation moved off an inline style precisely because an inline style would
//! have won whatever the sheet said.
//!
//! The theme sheet is loaded beside the component sheet because these rules
//! resolve theme `var()`s, and an unresolved one makes a whole declaration
//! invalid at computed-value time — silently, and in a direction that makes a
//! failing assertion pass.

use super::*;

use rinch_components::accordion::{Accordion, AccordionControl, AccordionItem, AccordionPanel};
use rinch_core::Component;
use rinch_core::events::{EventHandlerId, dispatch_event};

const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// Mount `build`'s tree under the real theme **and** component stylesheets.
fn mount(build: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> RinchApp {
    let mut app = RinchApp::new(build);
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    restyle(&app);
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    app
}

/// Re-run the whole cascade, for a tree whose classes changed after mount.
fn restyle(app: &RinchApp) {
    let doc = app.doc.as_ref().expect("a mounted document");
    let mut d = doc.borrow_mut();
    d.load_css(&rinch_theme::generate_theme_css(
        &rinch_theme::Theme::default(),
    ));
    d.load_css(&rinch_components::generate_component_css());
    d.recompute_all_styles_full();
}

/// The one node carrying `class` exactly.
fn node_with_class(app: &RinchApp, class: &str) -> usize {
    let doc = app.doc.as_ref().expect("a mounted document");
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

/// The `data-rid` of the one node carrying `class`.
fn handler_on(app: &RinchApp, class: &str) -> EventHandlerId {
    let id = node_with_class(app, class);
    let doc = app.doc.as_ref().expect("a mounted document");
    let d = doc.borrow();
    let rid = d
        .tree
        .get(id)
        .expect("the node is in the tree")
        .attributes
        .get("data-rid")
        .expect("the node carries a handler")
        .parse()
        .expect("a handler id");
    EventHandlerId(rid)
}

// ================================== Accordion::disable_chevron_rotation

/// An accordion of one item, opened, so its chevron carries the rotation class.
fn open_accordion(disable_chevron_rotation: bool) -> RinchApp {
    let app = mount(move |scope: &mut RenderScope| {
        let label = scope.create_text("Section");
        let control = AccordionControl::default().render(scope, &[label]);
        let panel = AccordionPanel.render(scope, &[]);
        let item = AccordionItem {
            value: "one".into(),
        }
        .render(scope, &[control, panel]);
        Accordion {
            disable_chevron_rotation,
            ..Default::default()
        }
        .render(scope, &[item])
    });

    let toggle = handler_on(&app, "rinch-accordion__control");
    assert!(dispatch_event(toggle), "the item's toggle ran");
    restyle(&app);
    app
}

/// The chevron's computed transform matrix.
fn chevron_matrix(app: &RinchApp) -> [f64; 6] {
    let id = node_with_class(app, "rinch-accordion__chevron--rotated");
    let doc = app.doc.as_ref().expect("a mounted document");
    let d = doc.borrow();
    d.tree
        .get(id)
        .expect("the node is in the tree")
        .computed_style
        .transform
        .matrix
}

#[test]
fn an_open_chevron_is_rotated_by_default() {
    let m = chevron_matrix(&open_accordion(false));
    // rotate(180deg) is [-1, 0, 0, -1, 0, 0], up to the cosine of π.
    assert!(
        (m[0] + 1.0).abs() < 1e-6 && (m[3] + 1.0).abs() < 1e-6,
        "precondition: an open chevron really is turned over, so the next test \
         is measuring a countermand and not an absence — got {m:?}"
    );
}

#[test]
fn disable_chevron_rotation_beats_the_rotation() {
    let m = chevron_matrix(&open_accordion(true));
    assert_eq!(
        m,
        [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        "`transform: none` on two classes and an attribute has to beat \
         `rotate(180deg)` on one class — and it only can because the rotation \
         is a class rather than the inline style it used to be"
    );
}
