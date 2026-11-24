//! #2208: the text on a filled component reads on its fill.
//!
//! The filled components (Button, ActionIcon, Badge, Avatar, Pagination's
//! active page, Tabs' active pill, Stepper's completed and current step,
//! NavLink's active link, Checkbox's check, Alert) painted `color: white`
//! whatever they were filled with, so a light accent (a yellow or lime
//! primary, a pale custom shade) gave unreadable text; Badge alone
//! special-cased yellow and lime. They now paint the theme's contrast tokens,
//! `--rinch-primary-color-contrast` and `--rinch-color-{name}-contrast`,
//! which the theme computes from the colour (white, or black on a light one)
//! and an app may set.
//!
//! Reads what the real cascade paints (the real theme and component sheets,
//! the software painter), in both schemes. The default theme paints exactly
//! what it painted before: white text everywhere, black on Badge's yellow and
//! lime, yellow-9 on Alert's yellow.

#![cfg(feature = "windowing")]

#[path = "support/component_paint_2208.rs"]
mod support;

use rinch_theme::{ColorName, Theme};
use support::{PRIMARY_FILLED, contrast, paint, probe, theme_css};

const WHITE: [u8; 3] = [255, 255, 255];
const BLACK: [u8; 3] = [0, 0, 0];

fn theme(dark: bool, primary: ColorName) -> Theme {
    Theme::builder()
        .dark_mode(dark)
        .primary_color(primary)
        .build()
}

fn sheets(theme: &Theme) -> [String; 2] {
    [theme_css(theme), rinch_components::generate_component_css()]
}

/// With a yellow primary, every filled component's text reads on the fill
/// (WCAG AA, 4.5:1), in both schemes. Before #2208 it was white at 1.8:1.
#[test]
fn text_reads_on_a_light_primary() {
    let mut wrong = Vec::new();
    for dark in [false, true] {
        let t = theme(dark, ColorName::Yellow);
        let [a, b] = sheets(&t);
        for p in PRIMARY_FILLED {
            let got = paint(&[&a, &b], p, None, None, "var(--rinch-primary-color)");
            let ratio = contrast(got.text, got.reference);
            if ratio < 4.5 {
                wrong.push(format!(
                    "dark={dark} {}: text {:?} on {:?} reads {ratio:.2}:1",
                    p.name, got.text, got.reference
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "unreadable:\n{}", wrong.join("\n"));
}

/// The default (blue) theme paints the white text it always painted, in both
/// schemes. A positive control for the sampler too: the fill is the primary.
#[test]
fn default_theme_text_is_still_white() {
    let mut wrong = Vec::new();
    for dark in [false, true] {
        let t = theme(dark, ColorName::Blue);
        let [a, b] = sheets(&t);
        for p in PRIMARY_FILLED {
            let got = paint(&[&a, &b], p, None, None, "var(--rinch-primary-color)");
            if got.text != WHITE {
                wrong.push(format!("dark={dark} {}: text {:?}", p.name, got.text));
            }
        }
        // The sampler sees the fill where there is one.
        let button = paint(
            &[&a, &b],
            &PRIMARY_FILLED[0],
            None,
            None,
            "var(--rinch-primary-color)",
        );
        assert_eq!(
            button.fill, button.reference,
            "the button is primary-filled"
        );
        assert_ne!(button.fill, button.page);
    }
    assert!(wrong.is_empty(), "changed:\n{}", wrong.join("\n"));
}

/// A palette colour picked per component reads too: a yellow Button
/// (`color: "yellow"`, its inline custom properties), while a red one keeps
/// white text.
#[test]
fn a_coloured_buttons_text_follows_its_colour() {
    for dark in [false, true] {
        let t = theme(dark, ColorName::Blue);
        let [a, b] = sheets(&t);
        let p = probe(
            "coloured button",
            "rinch-button rinch-button--filled rinch-button--colored",
        );
        for (color, want_white) in [("yellow", false), ("lime", false), ("red", true)] {
            let style = rinch_components::Button {
                color: color.to_string(),
                ..Default::default()
            }
            .style_string()
            .unwrap();
            let got = paint(
                &[&a, &b],
                &p,
                Some(&style),
                None,
                &format!("var(--rinch-color-{color}-6)"),
            );
            assert_eq!(got.fill, got.reference, "dark={dark} {color}: filled");
            if want_white {
                assert_eq!(got.text, WHITE, "dark={dark} {color}");
            } else {
                assert!(
                    contrast(got.text, got.fill) >= 4.5,
                    "dark={dark} {color}: text {:?} on {:?}",
                    got.text,
                    got.fill
                );
            }
        }
    }
}

/// Badge's and Alert's per-colour filled rules: what they painted before
/// (Badge black on yellow and lime, Alert yellow-9 on yellow, white
/// elsewhere).
#[test]
fn per_colour_badges_and_alerts_paint_as_before() {
    let t = theme(false, ColorName::Blue);
    let [a, b] = sheets(&t);
    let badge = probe("badge", "rinch-badge rinch-badge--filled");
    for name in ColorName::all() {
        let color = name.css_name();
        let got = paint(&[&a, &b], &badge, None, Some(color), "black");
        let want = match name {
            ColorName::Yellow | ColorName::Lime => BLACK,
            _ => WHITE,
        };
        assert_eq!(got.text, want, "badge {color}");
    }
    for color in ["blue", "green", "red", "gray", "yellow"] {
        let p = probe(
            "alert",
            if color == "yellow" {
                "rinch-alert rinch-alert--filled rinch-alert--yellow"
            } else {
                match color {
                    "blue" => "rinch-alert rinch-alert--filled rinch-alert--blue",
                    "green" => "rinch-alert rinch-alert--filled rinch-alert--green",
                    "red" => "rinch-alert rinch-alert--filled rinch-alert--red",
                    _ => "rinch-alert rinch-alert--filled rinch-alert--gray",
                }
            },
        );
        let got = paint(&[&a, &b], &p, None, None, "var(--rinch-color-yellow-9)");
        if color == "yellow" {
            assert_eq!(got.text, got.reference, "alert yellow keeps yellow-9");
        } else {
            assert_eq!(got.text, WHITE, "alert {color}");
        }
    }
}

/// An app sets the contrast colour in its own stylesheet, after the theme's.
#[test]
fn the_app_sets_the_contrast_token() {
    let t = theme(false, ColorName::Blue);
    let [a, b] = sheets(&t);
    let app = ":root { --rinch-primary-color-contrast: rgb(10, 20, 30); }";
    for p in PRIMARY_FILLED {
        let got = paint(&[&a, &b, app], p, None, None, "rgb(10, 20, 30)");
        assert_eq!(got.text, got.reference, "{}", p.name);
    }
}

/// #3020: a filled Alert with no colour class (hand-written markup; the
/// component always writes one) falls back to the primary fill and its
/// contrast token, its title and close button included, as a filled Badge
/// with no `data-color` does. It used to paint no fill and plain white text.
#[test]
fn a_colourless_filled_alert_falls_back_like_a_badge() {
    let badge = probe("badge", "rinch-badge rinch-badge--filled");
    let alert = probe("alert", "rinch-alert rinch-alert--filled");
    let title = support::Probe {
        swatch: "rinch-alert__title",
        ..alert
    };
    let close = support::Probe {
        swatch: "rinch-alert__close",
        ..alert
    };
    for dark in [false, true] {
        // A yellow primary's contrast token is black, so white is wrong.
        let t = theme(dark, ColorName::Yellow);
        let [a, b] = sheets(&t);
        let want = paint(&[&a, &b], &badge, None, None, "var(--rinch-primary-color)");
        assert_eq!(want.fill, want.reference, "dark={dark}: the badge control");
        assert_eq!(want.text, BLACK, "dark={dark}: the badge control");
        for p in [&alert, &title] {
            let got = paint(&[&a, &b], p, None, None, "var(--rinch-primary-color)");
            let swatch = p.swatch;
            assert_eq!(got.fill, want.fill, "dark={dark} {swatch}: fill");
            assert_eq!(got.text, want.text, "dark={dark} {swatch}: text");
        }
        // The close button is drawn at `opacity: 0.7`, so its text is the
        // badge's text blended 70% over the fill.
        let got = paint(&[&a, &b], &close, None, None, "var(--rinch-primary-color)");
        assert_eq!(got.fill, want.fill, "dark={dark} close: fill");
        for i in 0..3 {
            let blend = 0.7 * f32::from(want.text[i]) + 0.3 * f32::from(want.fill[i]);
            assert!(
                (f32::from(got.text[i]) - blend).abs() <= 2.0,
                "dark={dark} close: text {:?}, want {:?} at 70% over {:?}",
                got.text,
                want.text,
                want.fill
            );
        }
    }
}
