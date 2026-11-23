//! #2871: a `Tree` row's plain, hovered, selected and selected-and-hovered
//! states are four different colours, and each belongs to its scheme: light
//! shades in the light scheme, dark ones in the dark scheme.
//!
//! The rows used `gray-1`, `primary-color-1` and `primary-color-2` directly,
//! the light scheme's shades, so in the dark scheme a hovered row was
//! near-white under light text. They now paint the theme's scheme-aware
//! option tokens, as `Select` does.
//!
//! Like the #2727 fixture this reads what the real cascade **paints**: the
//! real theme and component sheets, the Tree's row classes, a real `:hover`
//! (through `update_hover`), the software painter.

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

const PLAIN: &str = "rinch-tree__node-content";
const SELECTED: &str = "rinch-tree__node-content rinch-tree__node-content--selected";

/// Paints a plain row and a selected row inside a tree on the body
/// background, with the pointer over row `hover` (0 = plain, 1 = selected),
/// and returns the two rows' colours.
fn paint(dark: bool, hover: usize) -> [[u8; 3]; 2] {
    let theme = rinch_theme::Theme {
        dark_mode: dark,
        ..Default::default()
    };
    let mut doc = RinchDocument::new();
    doc.load_css(&rinch_theme::generate_theme_css(&theme));
    doc.load_css(&rinch_components::generate_component_css());
    // Rows without text, one fixed height each, so the sample point is the
    // row's own background and nothing else.
    doc.load_css(&format!(
        "html, body {{ margin: 0; }}
         .rinch-tree {{ position: absolute; left: 0; top: 0; width: {WIDTH}px;
                        background-color: var(--rinch-color-body); }}
         .rinch-tree__node-content {{ height: {ROW}px; padding: 0; margin: 0; }}"
    ));
    let body = doc.body();
    let tree = doc.create_element("ul");
    doc.set_attribute(tree, "class", "rinch-tree");
    doc.append_child(body, tree);
    let mut rows: Vec<NodeId> = Vec::new();
    for class in [PLAIN, SELECTED] {
        let li = doc.create_element("li");
        doc.set_attribute(li, "class", "rinch-tree__node");
        doc.append_child(tree, li);
        let row = doc.create_element("div");
        doc.set_attribute(row, "class", class);
        doc.append_child(li, row);
        rows.push(row);
    }
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    let mut changed = false;
    doc.update_hover(Some(rows[hover].0), &mut changed);
    assert!(changed, "the hover reached the row");
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
    let pixel = |y: u32| {
        let i = ((y * painter.width() + (WIDTH / 2.0) as u32) * 4) as usize;
        let d = painter.pixels();
        [d[i], d[i + 1], d[i + 2]]
    };
    [pixel((ROW / 2.0) as u32), pixel((ROW * 1.5) as u32)]
}

fn check(dark: bool) {
    let scheme = if dark { "dark" } else { "light" };
    let [hovered, selected] = paint(dark, 0);
    let [plain, selected_hovered] = paint(dark, 1);
    let states = [
        ("plain", plain),
        ("hovered", hovered),
        ("selected", selected),
        ("selected+hovered", selected_hovered),
    ];
    let report = format!("{scheme} scheme, painted: {states:?}");

    for (i, (a, ca)) in states.iter().enumerate() {
        for (b, cb) in &states[i + 1..] {
            assert_ne!(ca, cb, "{a} and {b} paint one colour; {report}");
        }
    }
    // Each state belongs to its scheme: under light text (dark scheme) a row
    // is dark, under dark text (light scheme) a row is light.
    for (name, c) in states {
        let max = *c.iter().max().unwrap();
        let min = *c.iter().min().unwrap();
        if dark {
            assert!(max < 110, "{name} is a light-scheme shade; {report}");
        } else {
            assert!(min > 150, "{name} is a dark-scheme shade; {report}");
        }
    }
}

#[test]
fn light_scheme_tree_row_states() {
    check(false);
}

#[test]
fn dark_scheme_tree_row_states() {
    check(true);
}
