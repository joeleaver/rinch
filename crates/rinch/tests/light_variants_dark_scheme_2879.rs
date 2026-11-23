//! #2879: the `light` variants (Button, ActionIcon, Badge, Alert, Code,
//! Avatar), the light-scheme hover/active fills of `outline`/`subtle`, the
//! checkbox/radio/switch focus rings and the Mark/Highlight colour paint
//! colours that belong to their scheme.
//!
//! They used the palette's light shades (`-0`, `-1`, `-2`, `yellow-2`)
//! directly, so in the dark scheme a light Button was a near-white slab and an
//! Alert's `-9` text sat unreadable on the dark page. They now paint the
//! theme's scheme-aware variant tokens (`--rinch-<color>-light`,
//! `-light-hover`, `-light-active`, `-light-color`, `-light-color-strong`,
//! `-light-text`, `--rinch-color-focus-ring`, `--rinch-color-mark`).
//!
//! Like the #2871 fixture this reads what the real cascade **paints**: the
//! real theme and component sheets, real `:hover` / `:active` / `:focus`
//! state, the software painter. Each probe is sampled at its fill, and at
//! swatches whose background is `currentColor` for its text (and an Alert's
//! title). In the light scheme every sample is compared with a swatch painted
//! with the shade the rule used before, so that scheme is pixel-identical.

// The software painter, parley and peniko come with `windowing` (the theme and
// component crates are dev-dependencies).
#![cfg(feature = "windowing")]

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VIEWPORT: (f32, f32) = (200.0, 80.0);

#[derive(Clone, Copy, PartialEq, Debug)]
enum State {
    Rest,
    Hover,
    Active,
    Focus,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Part {
    /// The probe's background, at (60, 35).
    Fill,
    /// A `currentColor` swatch inside the probe: its text colour.
    Text,
    /// A `.rinch-alert__title` swatch inside the probe.
    Title,
    /// The focus ring, 1px outside the probe's left edge.
    Ring,
}

struct Case {
    name: &'static str,
    class: &'static str,
    /// The probe's `data-color`, or for a coloured Button its `color` prop.
    color: Option<&'static str>,
    state: State,
    part: Part,
    /// What the rule painted before #2879, in either scheme.
    old: &'static str,
}

const fn case(
    name: &'static str,
    class: &'static str,
    state: State,
    part: Part,
    old: &'static str,
) -> Case {
    Case {
        name,
        class,
        color: None,
        state,
        part,
        old,
    }
}

const fn badge(name: &'static str, color: &'static str, part: Part, old: &'static str) -> Case {
    Case {
        name,
        class: "rinch-badge rinch-badge--light",
        color: Some(color),
        state: State::Rest,
        part,
        old,
    }
}

/// A `Button { color, variant: light }`: its classes and the inline custom
/// properties `Button::style_string` writes.
const fn colored(name: &'static str, state: State, part: Part, old: &'static str) -> Case {
    Case {
        name,
        class: "rinch-button rinch-button--light rinch-button--colored",
        color: Some("red"),
        state,
        part,
        old,
    }
}

use Part::*;
use State::*;

const BTN_LIGHT: &str = "rinch-button rinch-button--light";
const BTN_OUTLINE: &str = "rinch-button rinch-button--outline";
const BTN_SUBTLE: &str = "rinch-button rinch-button--subtle";
const AI_LIGHT: &str = "rinch-action-icon rinch-action-icon--light";
const AI_OUTLINE: &str = "rinch-action-icon rinch-action-icon--outline";
const AI_SUBTLE: &str = "rinch-action-icon rinch-action-icon--subtle";

const CASES: &[Case] = &[
    case(
        "button light",
        BTN_LIGHT,
        Rest,
        Fill,
        "var(--rinch-primary-color-0)",
    ),
    case(
        "button light hover",
        BTN_LIGHT,
        Hover,
        Fill,
        "var(--rinch-primary-color-1)",
    ),
    case(
        "button light active",
        BTN_LIGHT,
        Active,
        Fill,
        "var(--rinch-primary-color-2)",
    ),
    case(
        "button light text",
        BTN_LIGHT,
        Rest,
        Text,
        "var(--rinch-primary-color-6)",
    ),
    colored("red button light", Rest, Fill, "var(--rinch-color-red-0)"),
    colored(
        "red button light hover",
        Hover,
        Fill,
        "var(--rinch-color-red-1)",
    ),
    colored(
        "red button light text",
        Rest,
        Text,
        "var(--rinch-color-red-6)",
    ),
    case(
        "button outline hover",
        BTN_OUTLINE,
        Hover,
        Fill,
        "var(--rinch-primary-color-0)",
    ),
    case(
        "button outline active",
        BTN_OUTLINE,
        Active,
        Fill,
        "var(--rinch-primary-color-1)",
    ),
    case(
        "button subtle hover",
        BTN_SUBTLE,
        Hover,
        Fill,
        "var(--rinch-primary-color-0)",
    ),
    case(
        "button subtle active",
        BTN_SUBTLE,
        Active,
        Fill,
        "var(--rinch-primary-color-1)",
    ),
    case(
        "action icon light",
        AI_LIGHT,
        Rest,
        Fill,
        "var(--rinch-primary-color-0)",
    ),
    case(
        "action icon light hover",
        AI_LIGHT,
        Hover,
        Fill,
        "var(--rinch-primary-color-1)",
    ),
    case(
        "action icon light active",
        AI_LIGHT,
        Active,
        Fill,
        "var(--rinch-primary-color-2)",
    ),
    case(
        "action icon light text",
        AI_LIGHT,
        Rest,
        Text,
        "var(--rinch-primary-color-6)",
    ),
    case(
        "action icon outline hover",
        AI_OUTLINE,
        Hover,
        Fill,
        "var(--rinch-primary-color-0)",
    ),
    case(
        "action icon outline active",
        AI_OUTLINE,
        Active,
        Fill,
        "var(--rinch-primary-color-1)",
    ),
    case(
        "action icon subtle hover",
        AI_SUBTLE,
        Hover,
        Fill,
        "var(--rinch-primary-color-0)",
    ),
    case(
        "action icon subtle active",
        AI_SUBTLE,
        Active,
        Fill,
        "var(--rinch-primary-color-1)",
    ),
    case(
        "badge light",
        "rinch-badge rinch-badge--light",
        Rest,
        Fill,
        "var(--rinch-primary-color-0)",
    ),
    case(
        "badge light text",
        "rinch-badge rinch-badge--light",
        Rest,
        Text,
        "var(--rinch-primary-color-6)",
    ),
    badge("badge red", "red", Fill, "var(--rinch-color-red-0)"),
    badge("badge red text", "red", Text, "var(--rinch-color-red-6)"),
    badge("badge gray", "gray", Fill, "var(--rinch-color-gray-0)"),
    badge("badge gray text", "gray", Text, "var(--rinch-color-gray-6)"),
    badge("badge teal", "teal", Fill, "var(--rinch-color-teal-0)"),
    case(
        "alert blue",
        "rinch-alert rinch-alert--light rinch-alert--blue",
        Rest,
        Fill,
        "var(--rinch-color-blue-0)",
    ),
    case(
        "alert blue text",
        "rinch-alert rinch-alert--light rinch-alert--blue",
        Rest,
        Text,
        "var(--rinch-color-blue-9)",
    ),
    case(
        "alert blue title",
        "rinch-alert rinch-alert--light rinch-alert--blue",
        Rest,
        Title,
        "var(--rinch-color-blue-7)",
    ),
    case(
        "alert red",
        "rinch-alert rinch-alert--light rinch-alert--red",
        Rest,
        Fill,
        "var(--rinch-color-red-0)",
    ),
    case(
        "alert red text",
        "rinch-alert rinch-alert--light rinch-alert--red",
        Rest,
        Text,
        "var(--rinch-color-red-9)",
    ),
    case(
        "alert yellow",
        "rinch-alert rinch-alert--light rinch-alert--yellow",
        Rest,
        Fill,
        "var(--rinch-color-yellow-0)",
    ),
    case(
        "alert yellow title",
        "rinch-alert rinch-alert--light rinch-alert--yellow",
        Rest,
        Title,
        "var(--rinch-color-yellow-7)",
    ),
    case(
        "alert green",
        "rinch-alert rinch-alert--light rinch-alert--green",
        Rest,
        Fill,
        "var(--rinch-color-green-0)",
    ),
    case(
        "alert gray title",
        "rinch-alert rinch-alert--light rinch-alert--gray",
        Rest,
        Title,
        "var(--rinch-color-gray-7)",
    ),
    case(
        "alert primary",
        "rinch-alert rinch-alert--light rinch-alert--primary",
        Rest,
        Fill,
        "var(--rinch-primary-color-0)",
    ),
    case(
        "alert primary text",
        "rinch-alert rinch-alert--light rinch-alert--primary",
        Rest,
        Text,
        "var(--rinch-primary-color-9)",
    ),
    case(
        "alert primary title",
        "rinch-alert rinch-alert--light rinch-alert--primary",
        Rest,
        Title,
        "var(--rinch-primary-color-7)",
    ),
    case(
        "code",
        "rinch-code rinch-code--primary",
        Rest,
        Fill,
        "var(--rinch-primary-color-0)",
    ),
    case(
        "code text",
        "rinch-code rinch-code--primary",
        Rest,
        Text,
        "var(--rinch-primary-color-7)",
    ),
    case(
        "avatar light",
        "rinch-avatar rinch-avatar--light",
        Rest,
        Fill,
        "var(--rinch-primary-color-1)",
    ),
    case(
        "mark",
        "rinch-mark",
        Rest,
        Fill,
        "var(--rinch-color-yellow-2)",
    ),
    case(
        "highlight",
        "rinch-highlight__match",
        Rest,
        Fill,
        "var(--rinch-color-yellow-2)",
    ),
    case(
        "checkbox focus ring",
        "rinch-checkbox__box",
        Focus,
        Ring,
        "var(--rinch-primary-color-2)",
    ),
    case(
        "radio focus ring",
        "rinch-radio__indicator",
        Focus,
        Ring,
        "var(--rinch-primary-color-2)",
    ),
    case(
        "switch focus ring",
        "rinch-switch__track",
        Focus,
        Ring,
        "var(--rinch-primary-color-2)",
    ),
];

/// The `<input>` whose `:focus` rings each focus-ring probe (its next sibling).
fn ring_input(class: &str) -> &'static str {
    match class {
        "rinch-checkbox__box" => "rinch-checkbox__input",
        "rinch-radio__indicator" => "rinch-radio__input",
        "rinch-switch__track" => "rinch-switch__input",
        other => panic!("no input for {other}"),
    }
}

/// Paints one case and returns (the sampled colour, the old shade's swatch,
/// the page background).
fn paint(dark: bool, c: &Case) -> ([u8; 3], [u8; 3], [u8; 3]) {
    let theme = rinch_theme::Theme {
        dark_mode: dark,
        ..Default::default()
    };
    let mut doc = RinchDocument::new();
    doc.load_css(&rinch_theme::generate_theme_css(&theme));
    doc.load_css(&rinch_components::generate_component_css());
    // Boxes without text at fixed places, so each sample is one colour.
    doc.load_css(&format!(
        "html, body {{ margin: 0; background-color: var(--rinch-color-body); }}
         .probe.probe {{ position: absolute; left: 10px; top: 10px; width: 60px;
                        height: 30px; padding: 0; margin: 0; display: block;
                        box-sizing: border-box; transform: none; min-height: 0;
                        min-width: 0; font-size: 0; }}
         .sw.sw {{ position: absolute; top: 4px; width: 10px; height: 10px;
                  padding: 0; margin: 0; display: block; font-size: 0;
                  background-color: currentColor; }}
         .ref {{ position: absolute; left: 100px; top: 10px; width: 20px;
                 height: 20px; background-color: {}; }}
         input {{ display: none; }}",
        c.old
    ));
    let body = doc.body();
    let mut focus_target: Option<NodeId> = None;
    if c.part == Ring {
        let input = doc.create_element("input");
        doc.set_attribute(input, "class", ring_input(c.class));
        doc.append_child(body, input);
        focus_target = Some(input);
    }
    let probe = doc.create_element("div");
    doc.set_attribute(probe, "class", &format!("{} probe", c.class));
    if let Some(color) = c.color {
        if c.class.contains("rinch-button--colored") {
            let button = rinch_components::Button {
                color: color.to_string(),
                ..Default::default()
            };
            doc.set_attribute(probe, "style", &button.style_string().unwrap());
        } else {
            doc.set_attribute(probe, "data-color", color);
        }
    }
    doc.append_child(body, probe);
    let text = doc.create_element("div");
    doc.set_attribute(text, "class", "sw");
    doc.set_attribute(text, "style", "left: 4px;");
    doc.append_child(probe, text);
    let title = doc.create_element("div");
    doc.set_attribute(title, "class", "rinch-alert__title sw");
    doc.set_attribute(title, "style", "left: 30px;");
    doc.append_child(probe, title);
    let reference = doc.create_element("div");
    doc.set_attribute(reference, "class", "ref");
    doc.append_child(body, reference);

    // Set the state before the first style pass, so nothing depends on
    // invalidation: every element is styled fresh, in that state.
    match c.state {
        Rest => {}
        Hover => {
            let mut changed = false;
            doc.update_hover(Some(probe.0), &mut changed);
            assert!(changed, "the hover reached the probe");
        }
        Active => {
            doc.update_active(Some(probe.0));
        }
        Focus => {
            doc.update_focus(Some(focus_target.unwrap().0));
        }
    }
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
    let sample = match c.part {
        Fill => pixel(60, 35),
        Text => pixel(19, 19),
        Title => pixel(45, 19),
        Ring => pixel(9, 25),
    };
    (sample, pixel(110, 20), pixel(150, 60))
}

fn sample(dark: bool, name: &str) -> [u8; 3] {
    let c = CASES.iter().find(|c| c.name == name).unwrap();
    paint(dark, c).0
}

/// The light scheme paints exactly what it painted before #2879.
#[test]
fn light_scheme_is_unchanged() {
    let mut wrong = Vec::new();
    for c in CASES {
        let (got, old, _) = paint(false, c);
        if got != old {
            wrong.push(format!("{}: painted {got:?}, was {old:?}", c.name));
        }
    }
    assert!(
        wrong.is_empty(),
        "light scheme changed:\n{}",
        wrong.join("\n")
    );
}

/// WCAG contrast ratio of two sRGB colours.
fn contrast(a: [u8; 3], b: [u8; 3]) -> f32 {
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

/// In the dark scheme a fill is a dark tint (not the light shade), text on it
/// reads against the page (WCAG AA, 4.5:1), and a focus ring or mark shows against the page without being a
/// light-scheme shade.
#[test]
fn dark_scheme_paints_dark_variants() {
    let mut wrong = Vec::new();
    for c in CASES {
        let (got, old, page) = paint(true, c);
        let max = *got.iter().max().unwrap();
        let ok = match c.part {
            Fill => max < 130 && got != old && got != page,
            Text | Title => contrast(got, page) >= 4.5,
            Ring => max < 200 && got != old && got != page,
        };
        if !ok {
            wrong.push(format!(
                "{}: painted {got:?} (old shade {old:?}, page {page:?}, contrast {:.2})",
                c.name,
                contrast(got, page)
            ));
        }
    }
    assert!(wrong.is_empty(), "dark scheme:\n{}", wrong.join("\n"));
}

/// Rest, hover and active are three colours in both schemes.
#[test]
fn states_stay_distinct() {
    for dark in [false, true] {
        for (a, b) in [
            ("button light", "button light hover"),
            ("red button light", "red button light hover"),
            ("button light hover", "button light active"),
            ("button outline hover", "button outline active"),
            ("button subtle hover", "button subtle active"),
            ("action icon light", "action icon light hover"),
            ("action icon light hover", "action icon light active"),
            ("action icon outline hover", "action icon outline active"),
            ("action icon subtle hover", "action icon subtle active"),
        ] {
            let (ca, cb) = (sample(dark, a), sample(dark, b));
            assert_ne!(ca, cb, "dark={dark}: {a} and {b} paint one colour");
        }
    }
}
