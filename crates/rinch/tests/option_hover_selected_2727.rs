//! #2727: in an option list, a hovered option, the selected option and a plain
//! option are three different colours, in the light scheme and in the dark one.
//!
//! The theme used to give `--rinch-color-option-hover` and
//! `--rinch-color-option-selected` the same value in the light scheme (both
//! `primary-color-0`), so the option under the pointer looked selected; and in
//! the dark scheme `option-selected` was `default-hover` (both `dark-4`), so a
//! hovered default control looked like a selected option.
//!
//! This reads what a real cascade **paints**: the real theme sheet and the real
//! component sheet, a `Select`'s option classes, a real `:hover` (through
//! `update_hover`), the software painter. A token-string comparison would miss
//! a component rule that stops using the tokens, or two different `var()`s that
//! resolve to one colour.
//!
//! Set `RINCH_2727_PPM=<dir>` to also write each scheme's render as a PPM.

// The software painter, parley and peniko come with `windowing` (the theme and
// component crates are dev-dependencies).
#![cfg(feature = "windowing")]

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const ROW: f32 = 30.0;
const WIDTH: f32 = 200.0;
const VIEWPORT: (f32, f32) = (240.0, 200.0);

/// The rows, top to bottom. Each is one option of a Select's list (inside its
/// panel background), except the last, a box painted `default-hover`.
const ROWS: [(&str, &str); 5] = [
    ("plain", "rinch-select__option"),
    ("hovered", "rinch-select__option"),
    (
        "highlighted",
        "rinch-select__option rinch-select__option--highlighted",
    ),
    (
        "selected",
        "rinch-select__option rinch-select__option--selected",
    ),
    ("default-hover", "default-hover"),
];

fn render(dark: bool) -> (TinySkiaPainter, (f32, f32)) {
    let theme = rinch_theme::Theme {
        dark_mode: dark,
        ..Default::default()
    };
    let mut doc = RinchDocument::new();
    doc.load_css(&rinch_theme::generate_theme_css(&theme));
    doc.load_css(&rinch_components::generate_component_css());
    // Options without text, one fixed-height row each, so the sample point is
    // the option's own background and nothing else.
    doc.load_css(&format!(
        "html, body {{ margin: 0; }}
         .panel {{ position: absolute; left: 0; top: 0; width: {WIDTH}px;
                   background-color: var(--rinch-color-body); }}
         .panel > div {{ height: {ROW}px; padding: 0; margin: 0; }}
         .default-hover {{ background-color: var(--rinch-color-default-hover); }}"
    ));
    let body = doc.body();
    let panel = doc.create_element("div");
    doc.set_attribute(panel, "class", "panel");
    doc.append_child(body, panel);
    let mut hovered: Option<NodeId> = None;
    for (name, class) in ROWS {
        let row = doc.create_element("div");
        doc.set_attribute(row, "class", class);
        doc.append_child(panel, row);
        if name == "hovered" {
            hovered = Some(row);
        }
    }
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    let mut changed = false;
    doc.update_hover(hovered.map(|n| n.0), &mut changed);
    assert!(changed, "the hover reached the option");
    // A different viewport, so the resolve cannot early-return.
    let vp = (VIEWPORT.0 + 1.0, VIEWPORT.1);
    doc.resolve_layout(vp.0, vp.1);

    let mut painter = TinySkiaPainter::new(vp.0 as u32, vp.1 as u32);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        vp,
        &mut doc.font_cx,
        &mut layout_cx,
    );
    (painter, vp)
}

fn pixel(p: &TinySkiaPainter, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * p.width() + x) * 4) as usize;
    let d = p.pixels();
    [d[i], d[i + 1], d[i + 2]]
}

fn dump(p: &TinySkiaPainter, vp: (f32, f32), scheme: &str) {
    let Ok(dir) = std::env::var("RINCH_2727_PPM") else {
        return;
    };
    let (w, h) = (vp.0 as u32, (ROW * ROWS.len() as f32) as u32);
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    for y in 0..h {
        for x in 0..w {
            out.extend_from_slice(&pixel(p, x, y));
        }
    }
    std::fs::write(format!("{dir}/option-states-{scheme}.ppm"), out).unwrap();
}

fn check(dark: bool) {
    let scheme = if dark { "dark" } else { "light" };
    let (p, vp) = render(dark);
    dump(&p, vp, scheme);
    let colours: Vec<(&str, [u8; 3])> = ROWS
        .iter()
        .enumerate()
        .map(|(i, (name, _))| {
            let y = (ROW * i as f32 + ROW / 2.0) as u32;
            (*name, pixel(&p, (WIDTH / 2.0) as u32, y))
        })
        .collect();
    let of = |name: &str| colours.iter().find(|(n, _)| *n == name).unwrap().1;
    let report = format!("{scheme} scheme, painted: {colours:?}");

    // The hover and the keyboard highlight are one kind of "here" (#434).
    assert_eq!(of("hovered"), of("highlighted"), "{report}");
    assert_ne!(of("hovered"), of("plain"), "hover shows nothing; {report}");
    assert_ne!(
        of("selected"),
        of("plain"),
        "selection shows nothing; {report}"
    );
    assert_ne!(
        of("hovered"),
        of("selected"),
        "a hovered option looks selected; {report}"
    );
    assert_ne!(
        of("default-hover"),
        of("selected"),
        "a hovered default control looks like a selected option; {report}"
    );
}

#[test]
fn light_scheme_option_states_are_distinct() {
    check(false);
}

#[test]
fn dark_scheme_option_states_are_distinct() {
    check(true);
}
