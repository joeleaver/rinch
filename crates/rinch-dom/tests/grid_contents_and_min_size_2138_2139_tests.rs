//! #2138 and #2139 — two grid layout bugs found together while windowing an
//! asset grid.
//!
//! - #2138: the children of a `display: contents` wrapper inside a grid are
//!   grid items of that grid (css-display-3 §2.5), so `repeat(auto-fill, …)`
//!   places one per cell.
//! - #2139: a grid that is a flex item of a column flex container with a
//!   definite height keeps its content height (its automatic minimum size,
//!   css-flexbox-1 §4.5), and its `auto` rows are sized to their items.
//!
//! Every size is declared and no item holds text, so nothing here pins the
//! local font set. Chrome 156 is the oracle for each number.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

fn el(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let e = doc.create_element("div");
    doc.set_attribute(e, "style", style);
    doc.append_child(parent, e);
    e
}

fn rect(doc: &RinchDocument, id: NodeId) -> (f32, f32, f32, f32) {
    let l = &doc.tree.get(id.0).unwrap().layout;
    (l.x, l.y, l.width, l.height)
}

const AUTO_FILL: &str =
    "display: grid; grid-template-columns: repeat(auto-fill, minmax(108px, 1fr)); gap: 8px;";

// ── #2138 ───────────────────────────────────────────────────────────────────

/// One `display: contents` wrapper per card (a keyed `for`): 400px wide, gap
/// 8, so three 128px columns. Chrome 156: cards at x = 0, 136, 272, then a new
/// row at y = 58.
#[test]
fn contents_wrapper_per_item_auto_fill_places_one_card_per_cell_2138() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let g = el(&mut doc, body, &format!("{AUTO_FILL} width: 400px;"));
    let cards: Vec<NodeId> = (0..5)
        .map(|_| {
            let w = el(&mut doc, g, "display: contents;");
            el(&mut doc, w, "height: 50px;")
        })
        .collect();
    doc.resolve_layout(VW, VH);

    let got: Vec<_> = cards.iter().map(|&c| rect(&doc, c)).collect();
    assert_eq!(
        got,
        vec![
            (0.0, 0.0, 128.0, 50.0),
            (136.0, 0.0, 128.0, 50.0),
            (272.0, 0.0, 128.0, 50.0),
            (0.0, 58.0, 128.0, 50.0),
            (136.0, 58.0, 128.0, 50.0),
        ]
    );
    assert_eq!(rect(&doc, g).3, 108.0, "two rows of 50 and a gap of 8");
}

/// One wrapper holding several cards (a `Vec<NodeHandle>` embedded as
/// `{nodes}`): each card is a grid item, not one stacked cell.
#[test]
fn one_contents_wrapper_holding_all_cards_places_one_card_per_cell_2138() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let g = el(&mut doc, body, &format!("{AUTO_FILL} width: 400px;"));
    let w = el(&mut doc, g, "display: contents;");
    let cards: Vec<NodeId> = (0..4).map(|_| el(&mut doc, w, "height: 30px;")).collect();
    doc.resolve_layout(VW, VH);

    let got: Vec<_> = cards.iter().map(|&c| rect(&doc, c)).collect();
    assert_eq!(
        got,
        vec![
            (0.0, 0.0, 128.0, 30.0),
            (136.0, 0.0, 128.0, 30.0),
            (272.0, 0.0, 128.0, 30.0),
            (0.0, 38.0, 128.0, 30.0),
        ]
    );
}

// ── #2139 ───────────────────────────────────────────────────────────────────

/// The issue's case: a 300px column flex container (264px wide, scrolling)
/// holds the grid; eight `overflow: hidden` cards each hold a 100px box. Two
/// 128px columns, four 100px rows, three 8px gaps: Chrome 156 lays the grid
/// out 424 tall and each card 100.
#[test]
fn grid_flex_item_in_fixed_height_column_keeps_content_height_2139() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let col = el(
        &mut doc,
        body,
        "display: flex; flex-direction: column; height: 300px; overflow: auto; width: 264px;",
    );
    let g = el(&mut doc, col, AUTO_FILL);
    let cards: Vec<NodeId> = (0..8)
        .map(|_| {
            let c = el(&mut doc, g, "overflow: hidden;");
            el(&mut doc, c, "height: 100px;");
            c
        })
        .collect();
    doc.resolve_layout(VW, VH);

    assert_eq!(rect(&doc, g), (0.0, 0.0, 264.0, 424.0));
    let got: Vec<_> = cards.iter().map(|&c| rect(&doc, c)).collect();
    for (i, r) in got.iter().enumerate() {
        let (row, colm) = ((i / 2) as f32, (i % 2) as f32);
        assert_eq!(*r, (colm * 136.0, row * 108.0, 128.0, 100.0), "card {i}");
    }
}

/// A grid styled `grid` in a parent styled `outer`, holding `n` cards styled
/// `card`, each holding a 100px box. Returns the grid's height and the first
/// card's.
fn grid_in(outer: &str, grid: &str, card: &str, n: usize) -> (f32, f32) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let o = el(&mut doc, body, outer);
    let g = el(&mut doc, o, grid);
    let cards: Vec<NodeId> = (0..n)
        .map(|_| {
            let c = el(&mut doc, g, card);
            el(&mut doc, c, "height: 100px;");
            c
        })
        .collect();
    doc.resolve_layout(VW, VH);
    (rect(&doc, g).3, rect(&doc, cards[0]).3)
}

/// The same rule wherever the auto-height grid sits: its rows are sized to
/// their content, not to the space its parent offers (Chrome 156 for each).
/// The grid is two 128px columns of `overflow: hidden` cards holding 100px.
#[test]
fn auto_height_grid_rows_ignore_the_parents_height_2139() {
    let hidden = "overflow: hidden;";
    let cases = [
        // (name, outer, extra grid style, Chrome 156 (grid, card))
        (
            "column flex, no scroller",
            "display: flex; flex-direction: column; height: 300px; width: 264px;",
            "",
            (424.0, 100.0),
        ),
        (
            "column flex, min-content rows",
            "display: flex; flex-direction: column; height: 300px; width: 264px;",
            "grid-template-rows: min-content;",
            (424.0, 100.0),
        ),
        (
            "block parent with a height",
            "height: 300px; width: 264px;",
            "",
            (424.0, 100.0),
        ),
        (
            "grid parent with a height",
            "display: grid; height: 300px; width: 264px;",
            "",
            (424.0, 100.0),
        ),
        (
            "grid parent, min-content row",
            "display: grid; grid-template-rows: min-content; height: 300px; width: 264px;",
            "",
            (424.0, 100.0),
        ),
        (
            "row flex, align-items: start",
            "display: flex; height: 300px; width: 264px; align-items: start;",
            "width: 264px;",
            (424.0, 100.0),
        ),
        (
            "max-height clamps the grid, not its rows",
            "height: 300px; width: 264px;",
            "max-height: 200px;",
            (200.0, 100.0),
        ),
    ];
    let mut wrong = Vec::new();
    for (name, outer, extra, want) in cases {
        let got = grid_in(outer, &format!("{AUTO_FILL} {extra}"), hidden, 8);
        if got != want {
            wrong.push(format!("{name}: got {got:?}, Chrome 156 {want:?}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// Where the grid's height *is* definite — stretched by a row flex container,
/// flexed to 300 with `min-height: 0`, styled, or raised by `min-height` — the free
/// space is shared among the auto rows (Chrome 156). These held before #2139's
/// fix and must hold after it.
#[test]
fn definite_height_grid_still_shares_free_space_among_auto_rows_2139() {
    let hidden = "overflow: hidden;";
    let cases = [
        (
            "row flex, stretched",
            "display: flex; height: 300px; width: 264px;",
            "width: 264px;",
            (300.0, 69.0),
        ),
        (
            "column flex, min-height: 0",
            "display: flex; flex-direction: column; height: 300px; width: 264px;",
            "min-height: 0;",
            (300.0, 69.0),
        ),
        (
            "a styled height",
            "width: 264px;",
            "height: 300px;",
            (300.0, 69.0),
        ),
        (
            "min-height above the content",
            "width: 264px;",
            "min-height: 600px;",
            (600.0, 144.0),
        ),
    ];
    let mut wrong = Vec::new();
    for (name, outer, extra, want) in cases {
        let got = grid_in(outer, &format!("{AUTO_FILL} {extra}"), hidden, 8);
        if got != want {
            wrong.push(format!("{name}: got {got:?}, Chrome 156 {want:?}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// `fr` rows in an auto-height grid are sized from their content (each card
/// 100, the grid 200), not by dividing the parent's 300. Chrome 156.
#[test]
fn fr_rows_of_an_auto_height_grid_ignore_the_parents_height_2139() {
    let got = grid_in(
        "height: 300px; width: 264px;",
        "display: grid; grid-template-rows: 1fr 1fr;",
        "overflow: hidden;",
        2,
    );
    assert_eq!(got, (200.0, 100.0));
}

/// `repeat(auto-fill, 50px)` rows repeat once in an auto-height grid
/// whatever its parent's height, and fill a `max-height` (four rows in 220).
/// Chrome 156.
#[test]
fn auto_fill_rows_count_from_the_grids_own_height_2139() {
    let g = "display: grid; grid-template-rows: repeat(auto-fill, 50px); \
             grid-auto-flow: column; grid-auto-columns: 40px;";
    let got = grid_in("height: 300px; width: 264px;", g, "height: 10px;", 8);
    assert_eq!(got, (50.0, 10.0), "one repetition");
    let got = grid_in(
        "width: 264px;",
        &format!("{g} max-height: 220px;"),
        "height: 10px;",
        8,
    );
    assert_eq!(got, (200.0, 10.0), "four repetitions in a 220px max-height");
}
