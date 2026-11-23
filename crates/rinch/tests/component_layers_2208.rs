//! #2208: how the component sheet joins the cascade
//! (`ThemeProviderProps::component_styles`), and the contrast a component's
//! own `color` prop carries.
//!
//! `ComponentStyles::Layered` puts the theme sheet in `@layer rinch-theme` and
//! the component sheet in `@layer rinch-components`, so any unlayered app rule
//! outranks a component rule, the way author CSS outranks a browser's built-in
//! look of its controls; with no app CSS it paints what the unlayered sheets
//! paint. `ComponentStyles::Off` leaves the component sheet out and keeps the
//! theme. Reads what the real cascade paints.

#![cfg(feature = "windowing")]

#[path = "support/component_paint_2208.rs"]
mod support;

use rinch_components::color_utils::accent_vars;
use rinch_components::compose_theme_and_component_css;
use rinch_core::ComponentStyles;
use rinch_theme::{ColorName, Theme};
use support::{PRIMARY_FILLED, Probe, contrast, paint, probe, theme_css};

const WHITE: [u8; 3] = [255, 255, 255];

fn sheet(dark: bool, primary: ColorName, mode: ComponentStyles) -> String {
    let theme = Theme::builder()
        .dark_mode(dark)
        .primary_color(primary)
        .build();
    compose_theme_and_component_css(&theme_css(&theme), mode)
}

/// Without app CSS, the layered sheets paint what the unlayered ones paint:
/// fill and text of every filled component, both schemes, a dark and a light
/// primary. A real `<button>` is among them: the theme's base
/// `button { color: inherit }` must sit in a layer too, or it outranks every
/// layered component colour.
#[test]
fn layered_paints_like_unlayered() {
    let button = Probe {
        tag: "button",
        ..probe("<button> filled", "rinch-button rinch-button--filled")
    };
    let mut wrong = Vec::new();
    for dark in [false, true] {
        for primary in [ColorName::Blue, ColorName::Yellow] {
            let plain = sheet(dark, primary, ComponentStyles::Unlayered);
            let layered = sheet(dark, primary, ComponentStyles::Layered);
            assert!(layered.starts_with("@layer rinch-theme, rinch-components;"));
            for p in PRIMARY_FILLED.iter().chain([&button]) {
                let a = paint(&[&plain], p, None, None, "var(--rinch-primary-color)");
                let b = paint(&[&layered], p, None, None, "var(--rinch-primary-color)");
                if (a.text, a.fill, a.reference) != (b.text, b.fill, b.reference) {
                    wrong.push(format!(
                        "dark={dark} {primary:?} {}: {a:?} vs {b:?}",
                        p.name
                    ));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "layered differs:\n{}", wrong.join("\n"));
}

/// An app rule less specific than the component's (`button` against
/// `.rinch-button--filled`) loses to the unlayered sheet and wins over the
/// layered one, for the fill and for the text.
#[test]
fn any_app_rule_outranks_the_layered_sheet() {
    let p = Probe {
        tag: "button",
        ..probe("button", "rinch-button rinch-button--filled")
    };
    let app = "button { background-color: rgb(0, 128, 0); color: rgb(200, 0, 200); }";
    let plain = sheet(false, ColorName::Blue, ComponentStyles::Unlayered);
    let layered = sheet(false, ColorName::Blue, ComponentStyles::Layered);

    let before = paint(&[&plain, app], &p, None, None, "var(--rinch-primary-color)");
    assert_eq!(
        before.fill, before.reference,
        "unlayered: the component wins"
    );
    assert_eq!(before.text, WHITE);

    let after = paint(&[&layered, app], &p, None, None, "rgb(0, 128, 0)");
    assert_eq!(
        after.fill, after.reference,
        "layered: the app's `button` wins"
    );
    assert_eq!(after.text, [200, 0, 200]);

    // The app's `:root` override of a theme variable wins too.
    let vars = ":root { --rinch-primary-color: rgb(10, 20, 30); }";
    let p = probe("button", "rinch-button rinch-button--filled");
    let got = paint(&[&layered, vars], &p, None, None, "rgb(10, 20, 30)");
    assert_eq!(got.fill, got.reference);
}

/// `Off`: no component rules, the theme still there.
#[test]
fn off_leaves_the_components_unstyled() {
    let off = sheet(false, ColorName::Blue, ComponentStyles::Off);
    assert!(!off.contains(".rinch-button"));
    let p = probe("button", "rinch-button rinch-button--filled");
    let got = paint(&[&off], &p, None, None, "var(--rinch-primary-color)");
    assert_eq!(got.fill, got.page, "no fill");
    assert_ne!(got.reference, got.page, "the theme's variables are there");
}

/// The components whose `color` prop writes inline custom properties
/// (`accent_vars`): the fill and the text that reads on it.
const COLOURED: &[(&str, Probe)] = &[
    (
        "--rinch-action-icon",
        probe("action icon", "rinch-action-icon rinch-action-icon--filled"),
    ),
    (
        "--rinch-avatar",
        probe("avatar", "rinch-avatar rinch-avatar--filled"),
    ),
    (
        "--rinch-pagination",
        probe(
            "pagination",
            "rinch-pagination__item rinch-pagination__item--active",
        ),
    ),
    (
        "--rinch-navlink",
        probe(
            "navlink",
            "rinch-navlink rinch-navlink--default rinch-navlink--active",
        ),
    ),
    (
        "--rinch-tabs",
        support::wrapped(
            "tabs",
            &["rinch-tabs rinch-tabs--pills", "rinch-tabs__list"],
            "rinch-tabs__tab rinch-tabs__tab--active",
        ),
    ),
    (
        "--rinch-stepper",
        support::wrapped(
            "stepper",
            &["rinch-stepper__step rinch-stepper__step--completed"],
            "rinch-stepper__step-icon",
        ),
    ),
];

/// A light `color` (a palette name or a raw colour) gives text that reads; a
/// dark one keeps white.
#[test]
fn a_components_colour_carries_its_contrast() {
    let mut wrong = Vec::new();
    for dark in [false, true] {
        let css = sheet(dark, ColorName::Blue, ComponentStyles::Unlayered);
        for (prefix, p) in COLOURED {
            for (color, fill) in [
                ("yellow", "var(--rinch-color-yellow-6)"),
                ("lime", "var(--rinch-color-lime-6)"),
                ("#ffe066", "#ffe066"),
                ("red", "var(--rinch-color-red-6)"),
                ("#1864ab", "#1864ab"),
            ] {
                // The tabs' and the stepper's custom properties sit on the
                // component root; a descendant inherits them.
                let style = accent_vars(prefix, color);
                let got = paint(&[&css], p, Some(&style), None, fill);
                let light = matches!(color, "yellow" | "lime" | "#ffe066");
                let ok = got.fill == got.reference
                    && if light {
                        contrast(got.text, got.fill) >= 4.5
                    } else {
                        got.text == WHITE
                    };
                if !ok {
                    wrong.push(format!(
                        "dark={dark} {} {color}: text {:?} fill {:?} (want fill {:?})",
                        p.name, got.text, got.fill, got.reference
                    ));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// #2208 review: under `Layered` the theme's default focus ring (a bare
/// `:focus-visible` rule, now inside `@layer rinch-theme`) still paints when a
/// button takes keyboard focus.
#[test]
fn the_layered_theme_still_draws_the_focus_ring() {
    use rinch_core::dom::DomDocument;
    for mode in [ComponentStyles::Unlayered, ComponentStyles::Layered] {
        let mut doc = rinch_dom::RinchDocument::new();
        doc.load_css(&sheet(false, ColorName::Blue, mode));
        let body = doc.body();
        let button = doc.create_element("button");
        doc.set_attribute(button, "class", "rinch-button rinch-button--filled");
        doc.append_child(body, button);
        doc.resolve_layout(200.0, 80.0);
        let ring = |doc: &rinch_dom::RinchDocument| {
            doc.tree.get(button.0).unwrap().computed_style.outline_width
        };
        assert_eq!(ring(&doc), 0.0, "{mode:?}: no ring before focus");
        doc.update_focus(Some(button.0));
        doc.set_focus_visible(button.0, true);
        doc.resolve_layout(200.0, 80.0);
        assert_eq!(ring(&doc), 2.0, "{mode:?}: Tab draws the theme's 2px ring");
    }
}

/// `Off` keeps rinch's own window chrome: the menu bar's rules stay.
#[test]
fn off_keeps_the_window_chrome() {
    let off = sheet(false, ColorName::Blue, ComponentStyles::Off);
    assert!(off.contains(".rinch-app-menu-bar"));
    assert!(off.contains(".rinch-borderlesswindow"));
    assert!(!off.contains(".rinch-tabs"), "components are gone");
}
