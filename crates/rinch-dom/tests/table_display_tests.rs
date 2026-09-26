//! The table-internal displays that hold flow content — `table-cell` and
//! `table-caption` — are block containers, not flex containers (#1072).
//!
//! rinch has no table formatting context. `display_from_stylo` used to send
//! every (outside, inside) pair it did not name to `DisplayValue::Flex`, so a
//! `table-cell` was a flex *row*: two block children sat side by side, and
//! its own text was an anonymous flex item, which draws no `text-overflow`
//! "…". Chrome 153 stacks a cell's block children and draws the "…"
//! (measured; the numbers are quoted at each fixture). The cell is now a
//! block container; `display: table` / `table-row` keep the flex-row
//! approximation, which is what puts cells side by side.
//!
//! Only font-independent facts are asserted: the ellipsis string is far wider
//! than 90px in any face, and every line box is declared.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const CSS: &str = "
    body { margin: 0; font-family: sans-serif; font-size: 16px; line-height: 20px; }
    .clip { width: 90px; max-width: 90px; overflow: hidden; white-space: nowrap;
            text-overflow: ellipsis; }
";

fn el(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let x = doc.create_element("div");
    if !style.is_empty() {
        doc.set_attribute(x, "style", style);
    }
    doc.append_child(parent, x);
    x
}

fn text(doc: &mut RinchDocument, parent: NodeId, s: &str) {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
}

/// Whether the box's own inline layout ends in "…" (`None`: not an IFC root).
fn ellipsis_for(display: &str) -> Option<bool> {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let wrap = el(&mut doc, body, "");
    let x = el(&mut doc, wrap, &format!("display: {display}"));
    doc.set_attribute(x, "class", "clip");
    text(&mut doc, x, "Hello wonderful world of text, far too long for ninety px");
    doc.resolve_layout(400.0, 300.0);
    doc.tree
        .get(x.0)
        .unwrap()
        .text_layout
        .as_ref()
        .map(|l| l.text_content.ends_with('\u{2026}'))
}

/// Chrome 153: `lone` (a `table-cell`, 90px, nowrap, ellipsis) and `cap` (a
/// `table-caption`) both draw the "…"; a flex container's text does not.
#[test]
fn a_table_cell_and_a_table_caption_draw_their_ellipsis() {
    let cases: &[(&str, bool)] = &[
        ("table-cell", true),
        ("table-caption", true),
        // Controls: agree with Chrome before and after #1072.
        ("block", true),
        ("flow-root", true),
        ("list-item", true),
        ("flex", false),
    ];
    let mut failures = Vec::new();
    for &(display, want) in cases {
        let got = ellipsis_for(display).unwrap_or(false);
        if got != want {
            failures.push(format!("display: {display}: ellipsis {got}, want {want}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Chrome 153, `display: table; width: 400px` holding two cells, the first
/// holding two block children `A1` / `B1` at `line-height: 20px`:
/// `c1` 0,0 220x40 · `a1` 0,0 · `b1` 0,20 · `c2` 220,0. The cells sit side by
/// side (kept: the table is still a flex row) and a cell's blocks stack
/// (new: the cell was a flex row and put `b1` beside `a1`).
#[test]
fn a_cells_blocks_stack_while_the_cells_stay_side_by_side() {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let t = el(&mut doc, body, "display: table; width: 400px");
    let c1 = el(&mut doc, t, "display: table-cell");
    let a1 = el(&mut doc, c1, "");
    text(&mut doc, a1, "A1");
    let b1 = el(&mut doc, c1, "");
    text(&mut doc, b1, "B1");
    let c2 = el(&mut doc, t, "display: table-cell");
    text(&mut doc, c2, "C2");
    doc.resolve_layout(800.0, 600.0);

    let at = |n: NodeId| {
        let l = &doc.tree.get(n.0).unwrap().layout;
        (l.x, l.y, l.width, l.height)
    };
    let (c1x, c1y, c1w, c1h) = at(c1);
    let (c2x, c2y, ..) = at(c2);
    let (a1x, a1y, ..) = at(a1);
    let (b1x, b1y, ..) = at(b1);

    assert_eq!((c1x, c1y), (0.0, 0.0), "c1");
    assert_eq!(c2y, 0.0, "c2 shares c1's row");
    assert!(c2x >= c1w && c2x > 0.0, "c2 sits right of c1: c2.x {c2x}, c1.w {c1w}");
    assert_eq!((a1x, a1y), (0.0, 0.0), "a1");
    assert_eq!((b1x, b1y), (0.0, 20.0), "b1 stacks under a1");
    assert_eq!(c1h, 40.0, "c1 holds two 20px lines");
}
