//! #2437: nested fractional insets round once, on the absolute position.
//!
//! Ten nested blocks, each `padding-left: 13.774px; padding-top: 7.3px`, in a
//! 400px column. Chrome 157 (headless, `getBoundingClientRect`) lays every
//! inset out in 1/64px units and snaps only at paint:
//!
//! | level | Chrome x   | Chrome y  | Chrome width |
//! |-------|------------|-----------|--------------|
//! | 1     | 13.765625  | 7.296875  | 386.234375   |
//! | 5     | 68.828125  | 36.484375 | 331.171875   |
//! | 9     | 123.890625 | 65.671875 | 276.109375   |
//! | inner | 137.65625  | 72.96875  | 262.34375    |
//!
//! and every right edge is the column's, 400. Rinch keeps whole-pixel
//! layout, so each box is the rounding of its exact position, at any depth:
//! the location rounds as the absolute edge `round(abs) - round(parent abs)`.
//! Rounding each parent-relative offset on its own (Taffy's `round_layout`
//! before the fix) put the innermost box at x 140, y 70, its right edge at
//! 402 — two pixels past its own containing block.
//!
//! No text: the geometry is the box model's alone, and does not pin fonts.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const CHROME_X: [f64; 10] = [
    13.765625, 27.53125, 41.296875, 55.0625, 68.828125, 82.59375, 96.359375, 110.125, 123.890625,
    137.65625,
];
const CHROME_Y: [f64; 10] = [
    7.296875, 14.59375, 21.890625, 29.1875, 36.484375, 43.78125, 51.078125, 58.375, 65.671875,
    72.96875,
];

fn page() -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css("html, body { margin: 0 } .p { padding-left: 13.774px; padding-top: 7.3px }");
    let body = doc.body();
    let open = r#"<div class=p>"#.repeat(10);
    let close = "</div>".repeat(10);
    doc.set_inner_html(
        body,
        &format!(
            r#"<div style="width:400px">{open}<div id=inner style="height:1px"></div>{close}</div>"#
        ),
    );
    doc.resolve_layout(800.0, 600.0);
    doc
}

/// `(x, y, right)` on screen of every `.p` below the first, then `#inner`:
/// the boxes Chrome's table lists at levels 1..=10.
fn boxes(doc: &RinchDocument) -> Vec<(f64, f64, f64)> {
    let mut ids = rinch_dom::query::query_all(&doc.tree, doc.tree.root_id, ".p");
    ids.remove(0);
    ids.extend(rinch_dom::query::query_all(
        &doc.tree,
        doc.tree.root_id,
        "#inner",
    ));
    ids.iter()
        .map(|&n| {
            let (x, y) = rinch_dom::paint::compute_absolute_position(&doc.tree, n, 1.0);
            (x, y, x + f64::from(doc.tree.nodes[n].layout.width))
        })
        .collect()
}

#[test]
fn every_level_rounds_its_exact_position_once() {
    let doc = page();
    let got = boxes(&doc);
    assert_eq!(got.len(), 10);
    // The exact sums, accumulated in `f32` as layout accumulates them.
    let (mut ex, mut ey) = (0.0_f32, 0.0_f32);
    for (i, &(x, y, _)) in got.iter().enumerate() {
        ex += 13.774;
        ey += 7.3;
        // `round` of the exact sum, once: within half a pixel of the exact
        // position, and of Chrome's but for Chrome's 1/64px snap of each
        // inset (level 5's y is exactly 36.5, rinch 37; Chrome's 36.484
        // paints at 36).
        assert_eq!(
            (x, y),
            (f64::from(ex.round()), f64::from(ey.round())),
            "level {}: rinch ({x}, {y}), Chrome ({}, {})",
            i + 1,
            CHROME_X[i],
            CHROME_Y[i]
        );
        assert!((x - CHROME_X[i]).abs() < 0.6 && (y - CHROME_Y[i]).abs() < 0.6);
    }
    // round(10 * 13.774) = 138; rounding each level on its own gave 140.
    assert_eq!((got[9].0, got[9].1), (138.0, 73.0));
}

#[test]
fn every_right_edge_stays_on_its_containing_block() {
    let doc = page();
    for (i, &(_, _, right)) in boxes(&doc).iter().enumerate() {
        assert_eq!(right, 400.0, "level {}: right edge", i + 1);
    }
}
