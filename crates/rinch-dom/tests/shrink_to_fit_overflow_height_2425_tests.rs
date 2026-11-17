//! A shrink-to-fit box with less room than its min-content width is laid out
//! at that width, and its height is that layout's (#2425).
//!
//! A float, an absolutely positioned box or a flex item with `width: auto`
//! in a containing block narrower than its min-content width takes its
//! min-content width (CSS 2.2 §10.3.5 / §10.3.7). rinch took that width but
//! the height of the text broken at the room it was offered: the leaf's
//! measure broke the lines at 0, reported the widest line (the min-content
//! width) with those lines' height, and Taffy then laid the box out at both.
//! So its four lines were painted in a box five lines tall.
//!
//! Every expectation is Chrome 156's, on the same markup: the bundled Inter
//! registered as `ProbeFace`, `font: 16px/20px`, `* { margin: 0; padding: 0 }`,
//! sizes from `getBoundingClientRect`. Widths are compared to the nearest
//! pixel: Chrome's is 28.515625 and rinch rounds the layout to whole pixels.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// #2382's text: at no room it breaks at every opportunity, five lines
/// (`x ` / `-5 ` / `(-5) ` / `a-` / `5`); at its min-content width, 28.52,
/// `a-5` fits on one, four lines.
const TEXT: &str = "x -5 (-5) a-5";

fn doc() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    doc.load_css("* { margin: 0; padding: 0 } body { font: 16px/20px ProbeFace }");
    doc
}

/// `#t`'s `(width, height, lines)`, `markup` laid out in the body.
fn measure(markup: &str) -> (f32, f32, usize) {
    let mut d = doc();
    let body = d.body();
    d.set_inner_html(body, markup);
    d.resolve_layout(800.0, 600.0);
    let t: NodeId = d.query_selector("#t").expect("#t");
    let n = d.tree.get(t.0).unwrap();
    let lines = n.text_layout.as_ref().expect("an IFC root").layout.len();
    (n.layout.width, n.layout.height, lines)
}

fn check(cases: &[(&str, String, (f32, f32, usize))]) {
    let mut wrong = Vec::new();
    for (name, markup, chrome) in cases {
        let got = measure(markup);
        if got != *chrome {
            wrong.push(format!("{name}: rinch {got:?}, Chrome {chrome:?}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Chrome: 28.52 wide, 80 tall, four lines, for each of them.
#[test]
fn a_shrink_to_fit_box_with_no_room_is_as_tall_as_its_min_content_lines() {
    check(&[
        (
            "float in a 0-wide block",
            format!(r#"<div style="width:0"><div id="t" style="float:left">{TEXT}</div></div>"#),
            (29.0, 80.0, 4),
        ),
        (
            "float in a 15px block",
            format!(r#"<div style="width:15px"><div id="t" style="float:left">{TEXT}</div></div>"#),
            (29.0, 80.0, 4),
        ),
        (
            "absolute box in a 0-wide containing block",
            format!(
                r#"<div style="width:0;position:relative"><div id="t" style="position:absolute">{TEXT}</div></div>"#
            ),
            (29.0, 80.0, 4),
        ),
        (
            "flex item in a 0-wide flex container",
            format!(r#"<div style="width:0;display:flex"><div id="t">{TEXT}</div></div>"#),
            (29.0, 80.0, 4),
        ),
        (
            "inline-block in a 0-wide block",
            format!(
                r#"<div style="width:0"><div id="t" style="display:inline-block">{TEXT}</div></div>"#
            ),
            (29.0, 80.0, 4),
        ),
    ]);
}

/// What must not change: a box given its width is broken at that width,
/// however narrow. A block in a 0-wide block is 0 wide and breaks at every
/// opportunity (#2382); a float or an absolute box held to its own width,
/// `max-width` or `min-width` is broken at it. Chrome's sizes, each.
#[test]
fn a_box_given_its_width_is_broken_at_it() {
    let zero = |style: &str| {
        format!(
            r#"<div style="width:0;position:relative"><div id="t" style="{style}">{TEXT}</div></div>"#
        )
    };
    check(&[
        ("block in a 0-wide block", zero(""), (0.0, 100.0, 5)),
        ("float width 0", zero("float:left;width:0"), (0.0, 100.0, 5)),
        (
            "float max-width 10px",
            zero("float:left;max-width:10px"),
            (10.0, 100.0, 5),
        ),
        (
            "absolute box max-width 10px",
            zero("position:absolute;max-width:10px"),
            (10.0, 100.0, 5),
        ),
        (
            "float min-width 40px",
            zero("float:left;min-width:40px"),
            (40.0, 60.0, 3),
        ),
    ]);
}

/// The measure cache keeps an IFC root's sizes by the room they were broken
/// at, across style changes that leave its text alone. A box sized by its
/// content and one given its width, at the same room, have different lines:
/// neither may be answered with the other's. The float toggles between
/// `width: 0` and `width: auto` in a 0-wide block, re-laid out each time (at
/// another viewport, so the layout runs again).
#[test]
fn a_room_and_a_width_at_the_same_room_do_not_share_a_measure() {
    let mut d = doc();
    let body = d.body();
    d.set_inner_html(
        body,
        &format!(
            r#"<div style="width:0"><div id="t" style="float:left;width:0">{TEXT}</div></div>"#
        ),
    );
    let t: NodeId = d.query_selector("#t").expect("#t");
    let size = |d: &RinchDocument| {
        let n = d.tree.get(t.0).unwrap();
        (n.layout.width, n.layout.height)
    };
    let mut got = Vec::new();
    for (i, style) in [
        "float:left;width:0",
        "float:left",
        "float:left;width:0",
        "float:left",
    ]
    .iter()
    .enumerate()
    {
        d.set_attribute(t, "style", style);
        d.resolve_layout(800.0 + i as f32, 600.0);
        got.push(size(&d));
    }
    // Chrome: 0 x 100 given its width, 28.52 x 80 sized by its content.
    assert_eq!(
        got,
        [(0.0, 100.0), (29.0, 80.0), (0.0, 100.0), (29.0, 80.0)],
        "Chrome 0x100, 28.52x80, 0x100, 28.52x80"
    );
}
