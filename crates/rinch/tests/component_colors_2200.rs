//! #2200 (Alert) and #2201 (Badge): a colour that is not one of the built-in
//! names paints, in every variant, through the components' colour variables;
//! and the built-in names paint what they painted before the stylesheet moved
//! onto those variables.
//!
//! Each case paints a probe carrying the component's own class string and
//! inline style, and compares a sample with a swatch painted from the value
//! the colour should resolve to, in both schemes.

// The software painter, parley and peniko come with `windowing` (the theme and
// component crates are dev-dependencies).
#![cfg(feature = "windowing")]

use peniko::Brush;
use rinch_components::alert::Alert;
use rinch_components::badge::Badge;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VIEWPORT: (f32, f32) = (200.0, 80.0);

#[derive(Clone, Copy, PartialEq, Debug)]
enum Part {
    /// The probe's background.
    Fill,
    /// A `currentColor` swatch inside the probe: its text colour.
    Text,
    /// A `currentColor` swatch with `.rinch-alert__title`: the title colour.
    Title,
}
use Part::*;

/// What a probe is: its class, inline style and `data-color`.
struct Probe {
    class: String,
    style: Option<String>,
    data_color: Option<String>,
}

fn alert(color: &str, variant: &str) -> Probe {
    let a = Alert {
        color: color.into(),
        variant: variant.into(),
        ..Default::default()
    };
    Probe {
        class: a.class_string(),
        style: a.style_string(),
        data_color: None,
    }
}

fn badge(color: &str, variant: &str) -> Probe {
    let b = Badge {
        color: color.into(),
        variant: variant.into(),
        ..Default::default()
    };
    Probe {
        class: b.class_string(),
        style: b.style_string(),
        data_color: (!color.is_empty()).then(|| color.to_string()),
    }
}

/// Paints `probe` and returns (the sample of `part`, the swatch of `want`,
/// the page background).
fn paint(dark: bool, probe: &Probe, part: Part, want: &str) -> ([u8; 3], [u8; 3], [u8; 3]) {
    paint_in(dark, None, probe, part, want)
}

/// [`paint`], with `probe` nested inside an `outer` component box.
fn paint_in(
    dark: bool,
    outer: Option<&Probe>,
    probe: &Probe,
    part: Part,
    want: &str,
) -> ([u8; 3], [u8; 3], [u8; 3]) {
    let theme = rinch_theme::Theme {
        dark_mode: dark,
        ..Default::default()
    };
    let mut doc = RinchDocument::new();
    doc.load_css(&rinch_theme::generate_theme_css(&theme));
    doc.load_css(&rinch_components::generate_component_css());
    doc.load_css(&format!(
        "html, body {{ margin: 0; background-color: var(--rinch-color-body); }}
         .probe.probe {{ position: absolute; left: 10px; top: 10px; width: 60px;
                        height: 30px; padding: 0; margin: 0; display: block;
                        box-sizing: border-box; min-height: 0; min-width: 0;
                        font-size: 0; border-width: 0; }}
         .sw.sw {{ position: absolute; top: 4px; width: 10px; height: 10px;
                  padding: 0; margin: 0; display: block; font-size: 0;
                  background-color: currentColor; }}
         .outer.outer {{ position: absolute; left: 0; top: 0; width: 90px;
                        height: 60px; padding: 0; margin: 0; display: block;
                        box-sizing: border-box; font-size: 0; border-width: 0; }}
         .ref {{ position: absolute; left: 100px; top: 10px; width: 20px;
                 height: 20px; background-color: {want}; }}"
    ));
    let body = doc.body();
    let p = doc.create_element("div");
    doc.set_attribute(p, "class", &format!("{} probe", probe.class));
    if let Some(style) = &probe.style {
        doc.set_attribute(p, "style", style);
    }
    if let Some(color) = &probe.data_color {
        doc.set_attribute(p, "data-color", color);
    }
    match outer {
        Some(o) => {
            let wrap = doc.create_element("div");
            doc.set_attribute(wrap, "class", &format!("{} outer", o.class));
            if let Some(style) = &o.style {
                doc.set_attribute(wrap, "style", style);
            }
            if let Some(color) = &o.data_color {
                doc.set_attribute(wrap, "data-color", color);
            }
            doc.append_child(body, wrap);
            doc.append_child(wrap, p);
        }
        None => doc.append_child(body, p),
    }
    let text = doc.create_element("div");
    doc.set_attribute(text, "class", "sw");
    doc.set_attribute(text, "style", "left: 4px;");
    doc.append_child(p, text);
    let title = doc.create_element("div");
    doc.set_attribute(title, "class", "rinch-alert__title sw");
    doc.set_attribute(title, "style", "left: 30px;");
    doc.append_child(p, title);
    let reference = doc.create_element("div");
    doc.set_attribute(reference, "class", "ref");
    doc.append_child(body, reference);
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
    let sample = match part {
        Fill => pixel(60, 35),
        Text => pixel(19, 19),
        Title => pixel(45, 19),
    };
    (sample, pixel(110, 20), pixel(150, 60))
}

/// Checks every case in both schemes and reports all the mismatches at once.
fn check(cases: &[(&str, Probe, Part, &str)]) {
    let mut wrong = Vec::new();
    for dark in [false, true] {
        for (name, probe, part, want) in cases {
            let (got, swatch, page) = paint(dark, probe, *part, want);
            if *part == Fill && got == page {
                wrong.push(format!("dark={dark} {name}: no fill (page {page:?})"));
            } else if got != swatch {
                wrong.push(format!(
                    "dark={dark} {name}: painted {got:?}, {want} is {swatch:?}"
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn an_alert_in_any_palette_colour_or_raw_colour_paints_it() {
    check(&[
        (
            "violet light",
            alert("violet", "light"),
            Fill,
            "var(--rinch-color-violet-light)",
        ),
        (
            "violet light text",
            alert("violet", "light"),
            Text,
            "var(--rinch-color-violet-light-text)",
        ),
        (
            "violet light title",
            alert("violet", "light"),
            Title,
            "var(--rinch-color-violet-light-color-strong)",
        ),
        (
            "violet filled",
            alert("violet", "filled"),
            Fill,
            "var(--rinch-color-violet-6)",
        ),
        (
            "violet filled text",
            alert("violet", "filled"),
            Text,
            "white",
        ),
        (
            "violet outline text",
            alert("violet", "outline"),
            Text,
            "var(--rinch-color-violet-6)",
        ),
        ("raw filled", alert("#e8590c", "filled"), Fill, "#e8590c"),
        (
            "raw light",
            alert("#e8590c", "light"),
            Fill,
            "color-mix(in srgb, #e8590c 10%, transparent)",
        ),
        (
            "raw light title",
            alert("#e8590c", "light"),
            Title,
            "#e8590c",
        ),
    ]);
}

/// The built-in names, now set through the variables, paint the shades their
/// old per-colour rules named (the light variant is covered by #2879's test).
#[test]
fn an_alerts_built_in_colours_paint_as_before() {
    check(&[
        (
            "blue filled",
            alert("blue", "filled"),
            Fill,
            "var(--rinch-color-blue-6)",
        ),
        ("blue filled text", alert("blue", "filled"), Text, "white"),
        (
            "yellow filled text",
            alert("yellow", "filled"),
            Text,
            "var(--rinch-color-yellow-9)",
        ),
        (
            "yellow filled title",
            alert("yellow", "filled"),
            Title,
            "var(--rinch-color-yellow-9)",
        ),
        (
            "yellow outline text",
            alert("yellow", "outline"),
            Text,
            "var(--rinch-color-yellow-7)",
        ),
        (
            "red outline text",
            alert("red", "outline"),
            Text,
            "var(--rinch-color-red-6)",
        ),
        (
            "gray light",
            alert("gray", "light"),
            Fill,
            "var(--rinch-color-filled)",
        ),
        (
            "primary filled",
            alert("primary", "filled"),
            Fill,
            "var(--rinch-primary-color)",
        ),
    ]);
}

#[test]
fn a_badge_in_a_raw_colour_or_primary_paints_it() {
    check(&[
        ("raw filled", badge("#123456", "filled"), Fill, "#123456"),
        ("raw filled text", badge("#123456", "filled"), Text, "white"),
        ("raw light text", badge("#123456", "light"), Text, "#123456"),
        (
            "raw light",
            badge("#123456", "light"),
            Fill,
            "color-mix(in srgb, #123456 10%, transparent)",
        ),
        (
            "raw outline text",
            badge("#123456", "outline"),
            Text,
            "#123456",
        ),
        (
            "primary light",
            badge("primary", "light"),
            Fill,
            "var(--rinch-primary-color-light)",
        ),
    ]);
}

#[test]
fn a_badges_built_in_colours_paint_as_before() {
    check(&[
        (
            "red filled",
            badge("red", "filled"),
            Fill,
            "var(--rinch-color-red-6)",
        ),
        (
            "yellow filled text",
            badge("yellow", "filled"),
            Text,
            "#000",
        ),
        ("lime filled text", badge("lime", "filled"), Text, "#000"),
        ("teal filled text", badge("teal", "filled"), Text, "white"),
        (
            "teal light text",
            badge("teal", "light"),
            Text,
            "var(--rinch-color-teal-light-color)",
        ),
        (
            "grape outline text",
            badge("grape", "outline"),
            Text,
            "var(--rinch-color-grape-6)",
        ),
        (
            "default filled",
            badge("", "filled"),
            Fill,
            "var(--rinch-primary-color)",
        ),
        (
            "default light text",
            badge("", "light"),
            Text,
            "var(--rinch-primary-color-light-color)",
        ),
    ]);
}

/// A component nested in another sets every colour variable itself, so none
/// of the outer one's leaks in (review of #2960): a yellow Alert's text
/// colours, a raw-coloured Alert's, a coloured Badge's.
#[test]
fn a_nested_component_does_not_inherit_the_outer_ones_colours() {
    let mut wrong = Vec::new();
    let cases: [(&str, Probe, Probe, Part, &str); 5] = [
        (
            "blue filled in yellow filled",
            alert("yellow", "filled"),
            alert("blue", "filled"),
            Text,
            "white",
        ),
        (
            "blue outline in yellow outline",
            alert("yellow", "outline"),
            alert("blue", "outline"),
            Text,
            "var(--rinch-color-blue-6)",
        ),
        (
            "violet filled in yellow filled",
            alert("yellow", "filled"),
            alert("violet", "filled"),
            Text,
            "white",
        ),
        (
            "default badge in a raw badge",
            badge("#123456", "filled"),
            badge("", "filled"),
            Fill,
            "var(--rinch-primary-color)",
        ),
        (
            "teal badge in a yellow badge",
            badge("yellow", "filled"),
            badge("teal", "filled"),
            Text,
            "white",
        ),
    ];
    for dark in [false, true] {
        for (name, outer, inner, part, want) in &cases {
            let (got, swatch, _) = paint_in(dark, Some(outer), inner, *part, want);
            if got != swatch {
                wrong.push(format!(
                    "dark={dark} {name}: painted {got:?}, {want} is {swatch:?}"
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
