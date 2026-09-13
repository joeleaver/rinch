//! `radius` on the five components #474 wired, as a **computed** border-radius.
//!
//! The class-and-selector fixtures in `rinch-components`' `prop_wiring_474`
//! assert that the class reaches the element and that the stylesheet carries a
//! rule for it. Neither can see the **cascade**, and for two of the five that is
//! the whole question:
//!
//! - `.rinch-badge--radius-N` and `.rinch-badge` are both one class, so which
//!   wins is decided by nothing but **source order**. Move the radius block
//!   above the pill base and `Badge { radius: "xs" }` silently goes back to the
//!   32px pill — #474's own bug, restored, with every `css.contains(…)`
//!   assertion still green. That mutant survived the first round of review.
//! - `.rinch-tabs--radius-N .rinch-tabs__tab` ties `.rinch-tabs--pills
//!   .rinch-tabs__tab` on specificity, and is separated from it by source order
//!   too.
//!
//! So these read the number a real cascade produces. **Sampled at `xs`, never
//! `xl`:** `xl` is `2rem` — 32px, the same as the pill base a Badge starts
//! with — so an `xl` badge reads 32px whether the override applies or not. It
//! is a fixed point, and a fixture parked on it pins nothing.
//!
//! **The theme sheet has to be loaded as well as the component sheet.** Every
//! rule here resolves `var(--rinch-radius-*)`, which the theme declares; with
//! only `generate_component_css()` the whole property is invalid at
//! computed-value time and every reading is `0.0` — including the ones that
//! would otherwise fail. (`overlay_z_index_tests` needs no theme only because
//! its `var()`s carry literal fallbacks.)

use super::*;

use rinch_components::{
    Badge, NumberInput, PasswordInput, Tab, Tabs, TabsList, TextInput, class_utils::RADIUS_SCALE,
};
use rinch_core::Component;

const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// The default theme's radius scale, in px, as
/// `rinch_theme::radius::DEFAULT_RADIUS` declares it in rem.
const SCALE_PX: [(&str, f32); 5] = [
    ("xs", 2.0),
    ("sm", 4.0),
    ("md", 8.0),
    ("lg", 16.0),
    ("xl", 32.0),
];

fn px(step: &str) -> f32 {
    SCALE_PX
        .iter()
        .find(|(name, _)| *name == step)
        .map(|(_, v)| *v)
        .unwrap_or_else(|| panic!("{step} is not a scale step"))
}

/// Mount `build`'s tree under the real theme **and** component stylesheets.
fn mount(build: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> RinchApp {
    let mut app = RinchApp::new(build);
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

/// The four corners of the one node carrying `class` exactly, clockwise from
/// the top left.
fn corners(app: &RinchApp, class: &str) -> [f32; 4] {
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
    let style = &d
        .tree
        .get(matches[0])
        .expect("the node is in the tree")
        .computed_style;
    use rinch_dom::computed_style::LengthPercentageValue as L;
    let read = |v: &L| match v {
        L::Length(px) => *px,
        L::Zero => 0.0,
        other => panic!("`{class}` has a non-length radius: {other:?}"),
    };
    [
        read(&style.border_radius_top_left),
        read(&style.border_radius_top_right),
        read(&style.border_radius_bottom_right),
        read(&style.border_radius_bottom_left),
    ]
}

/// One tab inside a `Tabs` of `variant`, at `radius` (empty for unset).
fn tabs(variant: &'static str, radius: &'static str) -> RinchApp {
    mount(move |scope: &mut RenderScope| {
        let label = scope.create_text("One");
        let tab = Tab {
            value: "one".into(),
            ..Default::default()
        }
        .render(scope, &[label]);
        let list = TabsList {
            ..Default::default()
        }
        .render(scope, &[tab]);
        Tabs {
            variant: variant.into(),
            radius: radius.into(),
            ..Default::default()
        }
        .render(scope, &[list])
    })
}

// ------------------------------------------------------------------- Badge

/// The one that source order decides, at the one step that is not a fixed point.
#[test]
fn a_badge_radius_overrides_the_pill_base() {
    let pill = mount(|scope| Badge::default().render(scope, &[]));
    assert_eq!(
        corners(&pill, "rinch-badge"),
        [32.0; 4],
        "precondition: an unset badge is the 2rem pill it has always been"
    );

    for step in ["xs", "sm", "md", "lg"] {
        let app = mount(move |scope| {
            Badge {
                radius: step.into(),
                ..Default::default()
            }
            .render(scope, &[])
        });
        assert_eq!(
            corners(&app, "rinch-badge"),
            [px(step); 4],
            "radius {step:?} must beat the pill base — the two selectors carry \
             one class each, so only source order separates them, and losing \
             that race is #474's bug restored"
        );
    }
}

/// `xl` is the fixed point this file exists to avoid parking on: it agrees with
/// the pill base, so it is checked *as* a coincidence rather than as evidence.
#[test]
fn the_xl_badge_step_coincides_with_the_pill_base() {
    let app = mount(|scope| {
        Badge {
            radius: "xl".into(),
            ..Default::default()
        }
        .render(scope, &[])
    });
    assert_eq!(corners(&app, "rinch-badge"), [px("xl"); 4]);
    assert_eq!(
        px("xl"),
        32.0,
        "if the scale ever moves xl off 32px, `a_badge_radius_overrides_the_pill_base` \
         may sample it too"
    );
}

// -------------------------------------------------------------------- Tabs

#[test]
fn pills_tabs_round_by_the_radius_step() {
    let unset = tabs("pills", "");
    assert_eq!(
        corners(&unset, "rinch-tabs__tab"),
        [4.0; 4],
        "precondition: a pills tab is `var(--rinch-radius-sm)` by default"
    );

    let app = tabs("pills", "xl");
    assert_eq!(
        corners(&app, "rinch-tabs__tab"),
        [32.0; 4],
        "the radius rule and the pills base tie on specificity, so this is a \
         source-order question too"
    );
}

#[test]
fn outline_tabs_keep_square_bottom_corners() {
    let unset = tabs("outline", "");
    assert_eq!(
        corners(&unset, "rinch-tabs__tab"),
        [4.0, 4.0, 0.0, 0.0],
        "precondition: an outline tab is rounded on top only"
    );

    let app = tabs("outline", "xl");
    assert_eq!(
        corners(&app, "rinch-tabs__tab"),
        [32.0, 32.0, 0.0, 0.0],
        "the three-class outline rule must beat the two-class plain one, or the \
         tab detaches from the panel edge it sits on"
    );
}

// ------------------------------------------------- the three field components

/// `TextInput` and `PasswordInput` style a box *inside* the classed wrapper;
/// `NumberInput` rebinds the variable its five boxes share. All three are
/// separated from their base by specificity rather than order, so these are
/// coverage of "the class lands on the right box", not of the cascade race.
#[test]
fn a_field_radius_reaches_the_box_that_carries_the_border() {
    for step in RADIUS_SCALE {
        let text = mount(move |scope| {
            TextInput {
                radius: step.into(),
                ..Default::default()
            }
            .render(scope, &[])
        });
        assert_eq!(
            corners(&text, "rinch-text-input__input"),
            [px(step); 4],
            "TextInput {step:?}"
        );

        let password = mount(move |scope| {
            PasswordInput {
                radius: step.into(),
                ..Default::default()
            }
            .render(scope, &[])
        });
        assert_eq!(
            corners(&password, "rinch-password-input__wrapper"),
            [px(step); 4],
            "PasswordInput {step:?}"
        );
    }
}

/// The rebinding, read where it is least obvious: a stepper corner is
/// `calc(var(--rinch-radius-default) - 1px)`, so it must land one pixel inside
/// the step the container rebound.
#[test]
fn a_number_input_radius_moves_the_field_and_the_steppers_together() {
    let unset = mount(|scope| NumberInput::default().render(scope, &[]));
    assert_eq!(
        corners(&unset, "rinch-number-input__input"),
        [4.0; 4],
        "precondition: the field is `var(--rinch-radius-default)`, sm by default"
    );
    assert_eq!(
        corners(&unset, "rinch-number-input__control--up")[1],
        3.0,
        "precondition: the stepper's outer corner is one pixel inside it"
    );

    for step in ["xs", "md", "xl"] {
        let app = mount(move |scope| {
            NumberInput {
                radius: step.into(),
                ..Default::default()
            }
            .render(scope, &[])
        });
        assert_eq!(
            corners(&app, "rinch-number-input__input"),
            [px(step); 4],
            "the field must follow the rebound variable at {step:?}"
        );
        assert_eq!(
            corners(&app, "rinch-number-input__control--up")[1],
            px(step) - 1.0,
            "and so must the stepper, through its own `calc()`, at {step:?}"
        );
    }
}
