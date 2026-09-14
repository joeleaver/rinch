//! The two props #707 wired whose effect is a **cascade**, read as computed
//! style rather than as markup.
//!
//! `rinch-components`' `prop_wiring_707` says the attribute is published and
//! the sheet carries a rule for it. Neither can see what Stylo makes of the
//! two together, and for these two that is the whole question:
//!
//! - `Accordion::disable_chevron_rotation` has to *beat* the rotation, not
//!   merely coexist with it. Both declarations set `transform` on the same
//!   element, so which one applies is decided by specificity — and the rotation
//!   moved off an inline style precisely because an inline style would have won.
//! - `Textarea::max_rows` is a `calc()` over a custom property, a declared
//!   `line-height` in `em`, the theme's spacing scale and the border. A
//!   stylesheet `contains()` check cannot tell that from a `max-height` Stylo
//!   throws away as invalid at computed-value time, which is what an unresolved
//!   `var()` produces — silently, and with no `max-height` at all.
//!
//! The theme sheet is loaded beside the component sheet for that last reason:
//! `var(--rinch-spacing-sm)` is the theme's, and without it the whole `calc()`
//! is invalid and every reading here is `Auto` — including the ones that would
//! otherwise fail.

use super::*;

use rinch_components::accordion::{Accordion, AccordionControl, AccordionItem, AccordionPanel};
use rinch_components::textarea::Textarea;
use rinch_core::Component;
use rinch_core::events::{EventHandlerId, dispatch_event};
use rinch_dom::computed_style::DimensionValue;

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

// ================================================ Textarea::max_rows

/// A textarea capped at `rows`, and the `max-height` the cascade gives it.
fn max_height(rows: Option<u32>) -> (DimensionValue, f32) {
    let app = mount(move |scope: &mut RenderScope| {
        Textarea {
            max_rows: rows,
            ..Default::default()
        }
        .render(scope, &[])
    });
    let id = node_with_class(&app, "rinch-textarea__input");
    let doc = app.doc.as_ref().expect("a mounted document");
    let d = doc.borrow();
    let style = &d
        .tree
        .get(id)
        .expect("the node is in the tree")
        .computed_style;
    (style.max_height, style.font_size)
}

fn px(rows: u32) -> f32 {
    match max_height(Some(rows)) {
        (DimensionValue::Length(px), _) => px,
        other => panic!("a {rows}-row cap computed to {other:?}, not a length"),
    }
}

#[test]
fn an_uncapped_textarea_has_no_max_height() {
    let (value, _) = max_height(None);
    assert!(
        matches!(value, DimensionValue::Auto),
        "a textarea nobody capped must be free to grow — got {value:?}"
    );
}

#[test]
fn each_capped_row_is_one_declared_line_box() {
    let (_, font_size) = max_height(Some(6));
    let line = 1.5 * font_size;
    assert!(
        line > 1.0,
        "precondition: the input has a real font size to count lines in"
    );

    // Sampled at 2 and 6 rather than at 0 and 1: the difference between two
    // caps cancels the padding and border they share, so this reads the *slope*
    // — a cap that ignored the row count entirely, or counted in some other
    // unit, moves it. Neither sample is the count at which the formula would
    // agree with a constant.
    let slope = (px(6) - px(2)) / 4.0;
    assert!(
        (slope - line).abs() < 0.5,
        "four more rows must be four more line boxes: {} px per row against a \
         {line} px line",
        slope
    );
    assert!(
        px(2) > line * 2.0,
        "and the cap is the *border* box, so it holds the padding and border \
         as well as the rows"
    );
}
