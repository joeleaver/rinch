//! Paints a component probe under the real theme and component sheets and
//! samples its text colour, for the #2208 fixtures
//! (`filled_contrast_2208.rs`, `component_layers_2208.rs`).
//!
//! A probe is a box with the component's classes (inside ancestors with the
//! classes its selector needs), at a fixed place with no text; inside it a
//! swatch whose background is `currentColor` shows the text colour the
//! cascade gave it. A reference swatch paints a colour expression of the
//! test's choosing at another fixed place, so a test can compare the text with
//! the fill it sits on.

#![allow(dead_code)]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VIEWPORT: (f32, f32) = (200.0, 80.0);

/// One component's filled state.
#[derive(Clone, Copy, Debug)]
pub struct Probe {
    pub name: &'static str,
    /// Ancestors, outermost first, each a `div` with these classes.
    pub wrap: &'static [&'static str],
    /// The probe's own classes.
    pub class: &'static str,
    /// The probe's tag.
    pub tag: &'static str,
    /// The swatch's extra class (`rinch-alert__title` for an Alert's title).
    pub swatch: &'static str,
}

pub const fn probe(name: &'static str, class: &'static str) -> Probe {
    Probe {
        name,
        wrap: &[],
        class,
        tag: "div",
        swatch: "",
    }
}

pub const fn wrapped(
    name: &'static str,
    wrap: &'static [&'static str],
    class: &'static str,
) -> Probe {
    Probe {
        name,
        wrap,
        class,
        tag: "div",
        swatch: "",
    }
}

/// Every component that paints contrast text on a filled primary colour.
pub const PRIMARY_FILLED: &[Probe] = &[
    probe("button filled", "rinch-button rinch-button--filled"),
    probe(
        "action icon filled",
        "rinch-action-icon rinch-action-icon--filled",
    ),
    probe("badge filled", "rinch-badge rinch-badge--filled"),
    probe("avatar filled", "rinch-avatar rinch-avatar--filled"),
    probe(
        "pagination active",
        "rinch-pagination__item rinch-pagination__item--active",
    ),
    wrapped(
        "tabs pills active",
        &["rinch-tabs rinch-tabs--pills", "rinch-tabs__list"],
        "rinch-tabs__tab rinch-tabs__tab--active",
    ),
    wrapped(
        "stepper completed",
        &["rinch-stepper__step rinch-stepper__step--completed"],
        "rinch-stepper__step-icon",
    ),
    wrapped(
        "stepper progress",
        &["rinch-stepper__step rinch-stepper__step--progress"],
        "rinch-stepper__step-icon",
    ),
    probe(
        "navlink default active",
        "rinch-navlink rinch-navlink--default rinch-navlink--active",
    ),
    probe(
        "navlink filled active",
        "rinch-navlink rinch-navlink--filled rinch-navlink--active",
    ),
    wrapped(
        "checkbox check",
        &["rinch-checkbox rinch-checkbox--checked"],
        "rinch-checkbox__icon",
    ),
    probe(
        "alert filled",
        "rinch-alert rinch-alert--filled rinch-alert--primary",
    ),
    Probe {
        name: "alert filled title",
        wrap: &[],
        class: "rinch-alert rinch-alert--filled rinch-alert--primary",
        tag: "div",
        swatch: "rinch-alert__title",
    },
];

/// What one paint gave.
#[derive(Clone, Copy, Debug)]
pub struct Painted {
    /// The probe's text colour.
    pub text: [u8; 3],
    /// The probe's fill, right of the swatch (a round probe's corner is not fill).
    pub fill: [u8; 3],
    /// The reference swatch.
    pub reference: [u8; 3],
    /// The page.
    pub page: [u8; 3],
}

/// Paints `p` under `sheets` (loaded in order, as the shell orders the theme
/// sheet before the app's), with `inline` as the probe's `style` and
/// `data_color` as its `data-color`, and the reference swatch painting
/// `reference`.
pub fn paint(
    sheets: &[&str],
    p: &Probe,
    inline: Option<&str>,
    data_color: Option<&str>,
    reference: &str,
) -> Painted {
    let mut doc = RinchDocument::new();
    for sheet in sheets {
        doc.load_css(sheet);
    }
    doc.load_css(&format!(
        "html, body {{ margin: 0; background-color: var(--rinch-color-body, rgb(250, 250, 250)); }}
         .probe.probe {{ position: absolute; left: 10px; top: 10px; width: 60px;
                        height: 30px; padding: 0; margin: 0; display: block;
                        box-sizing: border-box; transform: none; min-height: 0;
                        min-width: 0; font-size: 0; opacity: 1; transition: none; }}
         .sw.sw {{ position: absolute; left: 4px; top: 4px; width: 10px; height: 10px;
                  padding: 0; margin: 0; display: block; font-size: 0;
                  background-color: currentColor; }}
         .wrap.wrap {{ display: block; position: static; }}
         .ref {{ position: absolute; left: 100px; top: 10px; width: 20px;
                 height: 20px; background-color: {reference}; }}"
    ));
    let mut parent = doc.body();
    for class in p.wrap {
        let div = doc.create_element("div");
        // Plain blocks, so the probe is placed against the page (an empty
        // inline-flex wrapper would sit on the line's baseline).
        doc.set_attribute(div, "class", &format!("{class} wrap"));
        doc.append_child(parent, div);
        parent = div;
    }
    let el = doc.create_element(p.tag);
    doc.set_attribute(el, "class", &format!("{} probe", p.class));
    if let Some(style) = inline {
        doc.set_attribute(el, "style", style);
    }
    if let Some(color) = data_color {
        doc.set_attribute(el, "data-color", color);
    }
    doc.append_child(parent, el);
    let sw = doc.create_element("div");
    doc.set_attribute(sw, "class", &format!("{} sw", p.swatch));
    doc.append_child(el, sw);
    let body = doc.body();
    let r = doc.create_element("div");
    doc.set_attribute(r, "class", "ref");
    doc.append_child(body, r);

    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    let mut painter = TinySkiaPainter::new(VIEWPORT.0 as u32, VIEWPORT.1 as u32);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        VIEWPORT,
        &mut doc.font_cx,
        &mut layout_cx,
    );
    let pixel = |x: u32, y: u32| {
        let i = ((y * painter.width() + x) * 4) as usize;
        let d = painter.pixels();
        [d[i], d[i + 1], d[i + 2]]
    };
    Painted {
        text: pixel(19, 19),
        fill: pixel(45, 30),
        reference: pixel(110, 20),
        page: pixel(150, 60),
    }
}

/// The theme sheet for `theme`.
pub fn theme_css(theme: &rinch_theme::Theme) -> String {
    rinch_theme::generate_theme_css(theme)
}

/// WCAG contrast ratio of two sRGB colours.
pub fn contrast(a: [u8; 3], b: [u8; 3]) -> f32 {
    let lum = |c: [u8; 3]| {
        let ch = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(c[0]) + 0.7152 * ch(c[1]) + 0.0722 * ch(c[2])
    };
    let (la, lb) = (lum(a), lum(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}
