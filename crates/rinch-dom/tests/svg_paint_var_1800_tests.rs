//! A `var()` in an SVG `fill` or `stroke` (#1800).
//!
//! SVG's `fill` and `stroke` are CSS properties, and their presentation
//! attributes are parsed as CSS, so `stroke="var(--brand)"` and `style="stroke:
//! var(--brand)"` both paint in the variable's colour. rinch's Stylo build has
//! no `fill`/`stroke` property (they are Gecko-only there), so `paint/svg.rs`
//! reads both itself; before the fix it read only the attribute and parsed a
//! `var()` as a colour, which failed, so the shape painted nothing (or its
//! inherited paint).
//!
//! **The oracle is Chrome 155** (`getComputedStyle(path).fill`/`.stroke` for
//! the same markup): a missing variable with no fallback is invalid at
//! computed-value time, so the shape inherits the `<svg>`'s paint; a
//! `style` declaration beats the attribute; a variable holding `currentColor`
//! resolves against the shape's own `color`; a custom property the shape
//! itself declares wins over an inherited one.
//!
//! Every assertion reads a pixel of the software painter in the middle of a
//! 20x20 square the shape covers entirely, so the answer is one colour.

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const GREEN: [u8; 4] = [0, 128, 0, 255];
const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const PURPLE: [u8; 4] = [128, 0, 128, 255];
const ORANGE: [u8; 4] = [200, 100, 0, 255];

const ROOT_CSS: &str = ":root { --w: rgb(0, 128, 0); --n: none; --c: currentColor; \
     --nested: var(--w) }";

/// One 20x20 `<svg>` per case, stacked; each holds one shape covering it.
/// `svg_attrs` go on the `<svg>`, `shape` is `(tag, attrs)`.
struct Case {
    svg_attrs: &'static [(&'static str, &'static str)],
    shape: (&'static str, &'static [(&'static str, &'static str)]),
}

fn paint(cases: &[Case]) -> (TinySkiaPainter, Vec<NodeId>) {
    let mut doc = RinchDocument::new();
    doc.load_css(ROOT_CSS);
    let body = doc.body();
    let mut shapes = Vec::new();
    for case in cases {
        let svg = doc.create_element("svg");
        doc.set_attribute(svg, "viewBox", "0 0 20 20");
        doc.set_attribute(svg, "style", "display: block; width: 20px; height: 20px");
        for (k, v) in case.svg_attrs {
            doc.set_attribute(svg, k, v);
        }
        let (tag, attrs) = case.shape;
        let s = doc.create_element(tag);
        for (k, v) in attrs {
            doc.set_attribute(s, k, v);
        }
        doc.append_child(svg, s);
        doc.append_child(body, svg);
        shapes.push(s);
    }
    let h = 20.0 * cases.len() as f32;
    doc.resolve_layout(40.0, h);
    let mut painter = TinySkiaPainter::new(40, h as u32);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (40.0, h),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    (painter, shapes)
}

/// The pixel at the middle of case `i`'s square.
fn middle(p: &TinySkiaPainter, i: usize) -> [u8; 4] {
    let (x, y) = (10u32, i as u32 * 20 + 10);
    let idx = ((y * p.width() + x) * 4) as usize;
    let d = p.pixels();
    [d[idx], d[idx + 1], d[idx + 2], d[idx + 3]]
}

const RECT: &str = "rect";
const FULL: [(&str, &str); 4] = [("x", "0"), ("y", "0"), ("width", "20"), ("height", "20")];

macro_rules! rect {
    ($($k:expr => $v:expr),*) => {
        (RECT, &[FULL[0], FULL[1], FULL[2], FULL[3], $(($k, $v)),*])
    };
}

#[track_caller]
fn check(cases: &[Case], want: &[[u8; 4]]) {
    let (p, _) = paint(cases);
    for (i, w) in want.iter().enumerate() {
        assert_eq!(&middle(&p, i), w, "case {i} (Chrome 155)");
    }
}

#[test]
fn a_var_in_a_fill_attribute_paints_the_variables_colour() {
    check(
        &[
            Case {
                svg_attrs: &[],
                shape: rect!("fill" => "var(--w)"),
            },
            // Case-insensitive function name, as CSS.
            Case {
                svg_attrs: &[],
                shape: rect!("fill" => "VAR(--w)"),
            },
            // A variable that holds another.
            Case {
                svg_attrs: &[],
                shape: rect!("fill" => "var(--nested)"),
            },
        ],
        &[GREEN, GREEN, GREEN],
    );
}

#[test]
fn a_var_in_a_fill_property_paints_the_variables_colour() {
    check(
        &[
            Case {
                svg_attrs: &[],
                shape: rect!("style" => "fill: var(--w)"),
            },
            // The declaration beats the presentation attribute.
            Case {
                svg_attrs: &[],
                shape: rect!("fill" => "rgb(255, 0, 0)", "style" => "fill: var(--w)"),
            },
            Case {
                svg_attrs: &[],
                shape: rect!("style" => "fill: var(--w) !important"),
            },
            // A literal declaration beats the attribute too, var or not.
            Case {
                svg_attrs: &[],
                shape: rect!("fill" => "var(--w)", "style" => "fill: rgb(255, 0, 0)"),
            },
        ],
        &[GREEN, GREEN, GREEN, RED],
    );
}

#[test]
fn a_var_in_a_stroke_paints_the_variables_colour() {
    let line: &[(&str, &str)] = &[
        ("d", "M0 10H20"),
        ("stroke-width", "20"),
        ("fill", "none"),
        ("stroke", "var(--w)"),
    ];
    let styled: &[(&str, &str)] = &[
        ("d", "M0 10H20"),
        ("stroke-width", "20"),
        ("fill", "none"),
        ("style", "stroke: var(--w)"),
    ];
    check(
        &[
            Case {
                svg_attrs: &[],
                shape: ("path", line),
            },
            Case {
                svg_attrs: &[],
                shape: ("path", styled),
            },
        ],
        &[GREEN, GREEN],
    );
}

#[test]
fn a_missing_variable_takes_its_fallback_or_inherits() {
    check(
        &[
            Case {
                svg_attrs: &[],
                shape: rect!("fill" => "var(--missing, rgb(255, 0, 0))"),
            },
            // Invalid at computed-value time: the shape inherits the svg's.
            Case {
                svg_attrs: &[("fill", "rgb(0, 0, 255)")],
                shape: rect!("fill" => "var(--missing)"),
            },
            Case {
                svg_attrs: &[("fill", "rgb(0, 0, 255)")],
                shape: rect!("style" => "fill: var(--missing)"),
            },
            // A fallback holding a variable.
            Case {
                svg_attrs: &[],
                shape: rect!("fill" => "var(--missing, var(--w))"),
            },
        ],
        &[RED, BLUE, BLUE, GREEN],
    );
}

#[test]
fn a_var_on_the_svg_is_inherited_by_its_shapes() {
    check(
        &[Case {
            svg_attrs: &[("fill", "var(--w)")],
            shape: rect!(),
        }],
        &[GREEN],
    );
}

#[test]
fn a_var_holding_none_or_current_color() {
    let (p, _) = paint(&[
        Case {
            svg_attrs: &[],
            shape: rect!("fill" => "var(--n)"),
        },
        Case {
            svg_attrs: &[(
                "style",
                "display: block; width: 20px; height: 20px; color: rgb(128, 0, 128)",
            )],
            shape: rect!("fill" => "var(--c)"),
        },
        // currentColor through a variable on the svg is the shape's own.
        Case {
            svg_attrs: &[("fill", "var(--c)")],
            shape: rect!("style" => "color: rgb(200, 100, 0)"),
        },
    ]);
    assert_eq!(middle(&p, 0)[3], 0, "var(--n) is none");
    assert_eq!(middle(&p, 1), PURPLE);
    assert_eq!(middle(&p, 2), ORANGE);
}

#[test]
fn the_shapes_own_custom_property_wins() {
    check(
        &[Case {
            svg_attrs: &[],
            shape: rect!("style" => "--w: rgb(200, 100, 0)", "fill" => "var(--w)"),
        }],
        &[ORANGE],
    );
}
