//! `Textarea::max_rows` really caps the laid-out height (issue #715).
//!
//! `rinch-components`' `textarea_max_rows_715.rs` pins that the prop reaches
//! a `max-height` style; this mounts the real component under the real theme
//! and component stylesheets (as `component_prop_707_tests.rs` does) to prove
//! the cap actually binds in layout — the thing #715 said could never happen
//! before the control's `rows` became a Taffy measure (#297/#1152) rather than
//! a `min-height`.

use super::*;

use rinch_components::textarea::Textarea;
use rinch_core::Component;

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

/// The laid-out height of the one `.rinch-textarea__input`.
fn input_height(app: &RinchApp) -> f32 {
    let doc = app.doc.as_ref().expect("a mounted document");
    let d = doc.borrow();
    let id = d
        .tree
        .nodes
        .iter()
        .find(|(_, n)| {
            n.attributes
                .get("class")
                .is_some_and(|c| c.split_whitespace().any(|w| w == "rinch-textarea__input"))
        })
        .map(|(id, _)| id)
        .expect("the control renders");
    d.tree.get(id).expect("laid out").layout.height
}

/// Issue #715's own measured shape: `min_rows: 20` lays out comfortably past
/// the stylesheet's size floor (which `an_author_min_height_is_a_floor...` and
/// `a_max_height_caps_the_control` in `rinch-dom`'s
/// `form_control_intrinsic_height_tests.rs` already establish at the raw-DOM
/// level); `max_rows: 10` must cut that back.
///
/// Before this issue's fix, `max_rows` was read into nothing — the control's
/// `min-height` beat any `max-height` the prop could have written, so the two
/// textareas below laid out identically (reproduced against the unwired
/// component: `capped_h == uncapped_h`, failing the assertion). A mutant that
/// wires the prop but discards the computed style (the classic #474 "read,
/// not used" shape) reproduces the same failure.
#[test]
fn max_rows_cuts_a_tall_min_rows_control_down() {
    let uncapped = mount(|scope| {
        Textarea {
            min_rows: Some(20),
            ..Default::default()
        }
        .render(scope, &[])
    });
    let capped = mount(|scope| {
        Textarea {
            min_rows: Some(20),
            max_rows: Some(10),
            ..Default::default()
        }
        .render(scope, &[])
    });

    let uncapped_h = input_height(&uncapped);
    let capped_h = input_height(&capped);

    assert!(
        uncapped_h > 300.0,
        "precondition: `min_rows: 20` with no cap really does grow well past \
         the stylesheet's size floor, so the next assertion is measuring a \
         real cut rather than two values that were never apart — got \
         {uncapped_h}"
    );
    assert!(
        capped_h < uncapped_h,
        "max_rows: 10 must cut the 20-row control down (got {capped_h} against \
         an uncapped {uncapped_h}) — if these are equal, max_rows is not \
         reaching the layout"
    );
    // Roughly half the rows, so roughly half the height (within the fixed
    // padding/border overhead, which does not double with the row count).
    assert!(
        capped_h < uncapped_h * 0.7,
        "a 10-row cap on a 20-row control should be close to half, not a \
         token reduction — got {capped_h} against {uncapped_h}"
    );
}

/// A `max_rows` whose cap sits *below* the stylesheet's own size floor is
/// unreachable — CSS resolves `min-height` over `max-height` whenever the two
/// conflict, matching a plain `<input>`/`<textarea>` (pinned at the raw-DOM
/// level by `an_author_min_height_is_a_floor_under_the_control_not_over_it`).
/// This is the one case issue #715 explicitly says is not a bug: the control
/// is at its usual, unwired-looking height not because `max_rows` was
/// ignored, but because CSS itself picked the floor.
#[test]
fn a_max_rows_cap_below_the_size_floor_loses_to_the_floor() {
    let floor_only = mount(|scope| Textarea::default().render(scope, &[]));
    let under_floor_cap = mount(|scope| {
        Textarea {
            max_rows: Some(1),
            ..Default::default()
        }
        .render(scope, &[])
    });
    assert_eq!(
        input_height(&floor_only),
        input_height(&under_floor_cap),
        "a 1-row cap is below the md size floor, so the floor still wins"
    );
}
