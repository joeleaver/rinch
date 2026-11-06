//! #2280 — a table with no width of its own, too wide for its room, shares
//! the room between its columns as Chrome does: each column is its
//! min-content width plus the same fraction of its max-content minus
//! min-content (CSS 2.1 §17.5.2.2; Blink's
//! `DistributeInlineSizeToComputedInlineSizeAuto`), not an equal share each.
//! A `colspan` cell's max-content width beyond its columns goes to them in
//! proportion to their own (Blink's `DistributeColspanCellToColumnsAuto`).
//! The table's Taffy grid sets `proportional_column_sizing` (the Taffy
//! fork's item 10 in `docs/internals/dependencies.md`).
//!
//! No text: each cell holds `n` 40px inline-blocks at `font-size: 0`, so its
//! min-content width is 40 and its max-content width `40 n`, on any host.
//! Every expected width was measured in Chrome 156 on the same markup.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const VP: (f32, f32) = (800.0, 600.0);
const CSS: &str = "body { margin: 0 } .room { width: 600px } \
    table { border-spacing: 0; font-size: 0; line-height: 0 } td { padding: 0 } \
    i { display: inline-block; width: 40px; height: 10px; vertical-align: top }";

/// A cell of `n` blocks, with `attrs` (an id, a colspan).
fn cell(attrs: &str, n: usize) -> String {
    format!("<td {attrs}>{}</td>", vec!["<i></i>"; n].join(" "))
}

fn layout(rows: &[String]) -> RinchDocument {
    layout_in(600, rows)
}

/// [`layout`] in a room `room` px wide.
fn layout_in(room: u32, rows: &[String]) -> RinchDocument {
    let mut d = RinchDocument::new();
    d.load_css(CSS);
    let body = d.body();
    let rows: String = rows.iter().map(|r| format!("<tr>{r}</tr>")).collect();
    d.set_inner_html(
        body,
        &format!("<div style=\"width:{room}px\"><table id=t>{rows}</table></div>"),
    );
    d.resolve_layout(VP.0, VP.1);
    d
}

/// The widths of the cells `ids`, and the table's height.
fn widths(d: &RinchDocument, ids: &[&str]) -> (Vec<f32>, f32) {
    let w = ids
        .iter()
        .map(|id| {
            let n = d.query_selector(&format!("#{id}")).unwrap();
            d.tree.nodes[n.0].layout.width
        })
        .collect();
    let t = d.query_selector("#t").unwrap();
    (w, d.tree.nodes[t.0].layout.height)
}

/// Each width within 1px of Chrome's (Taffy rounds each edge to the pixel).
fn assert_near(got: &[f32], chrome: &[f32]) {
    assert_eq!(got.len(), chrome.len());
    for (g, c) in got.iter().zip(chrome) {
        assert!((g - c).abs() <= 1.0, "rinch {got:?}, Chrome {chrome:?}");
    }
}

#[test]
fn an_auto_table_too_wide_for_its_room_shares_it_by_each_columns_room_to_grow() {
    // max 800 + 200 + 80 + 400 = 1480 into 600, min 40 each: every column is
    // 40 + (max - 40) * 440 / 1320. Equal shares gave 173, 174, 80, 173, so
    // the first column wrapped to five lines.
    let d = layout(&[[
        cell("id=a", 20),
        cell("id=b", 5),
        cell("id=c", 2),
        cell("id=d", 10),
    ]
    .concat()]);
    let (w, h) = widths(&d, &["a", "b", "c", "d"]);
    assert_near(&w, &[293.3, 93.3, 53.3, 160.0]);
    assert_eq!(h, 30.0, "three lines of the 20-block column, as in Chrome");
}

#[test]
fn a_colspan_cells_max_content_goes_to_its_columns_by_their_own() {
    // Columns of max 120 and 40 under a colspan of max 320, which fit: its 160
    // beyond them goes 3:1. Equal shares gave 200 and 120.
    let d = layout(&[
        [cell("id=a", 3), cell("id=b", 1)].concat(),
        cell("colspan=2", 8),
    ]);
    assert_near(&widths(&d, &["a", "b"]).0, &[240.0, 80.0]);
    // The same over the first two of three columns.
    let d = layout(&[
        [cell("id=a", 3), cell("id=b", 1), cell("id=c", 2)].concat(),
        cell("colspan=2", 8),
    ]);
    assert_near(&widths(&d, &["a", "b", "c"]).0, &[240.0, 80.0, 80.0]);
}

#[test]
fn a_colspan_and_a_table_too_wide_for_its_room() {
    // The four columns of the first test under a colspan=4 of 60 blocks:
    // the span raises their max-content widths in proportion, then the room
    // is shared by each one's room to grow.
    let d = layout(&[
        [
            cell("id=a", 20),
            cell("id=b", 5),
            cell("id=c", 2),
            cell("id=d", 10),
        ]
        .concat(),
        cell("colspan=4", 60),
    ]);
    let (w, h) = widths(&d, &["a", "b", "c", "d"]);
    assert_near(&w, &[287.0, 95.8, 57.6, 159.6]);
    assert_eq!(h, 80.0);
}

#[test]
fn an_auto_table_that_fits_is_at_its_max_content() {
    // The positive control of the shares: content that fits needs none.
    let d = layout(&[[
        cell("id=a", 3),
        cell("id=b", 2),
        cell("id=c", 1),
        cell("id=d", 2),
    ]
    .concat()]);
    assert_near(
        &widths(&d, &["a", "b", "c", "d"]).0,
        &[120.0, 80.0, 40.0, 80.0],
    );
}

// A column with a `width` reaches it before the `auto` columns grow (Blink's
// "specified guess"), as it did before #2280 in all but the last case: the
// first cut of the fix shared its room with theirs and squeezed it
// (20/560/20 came out 6/588/6). Mail's spacer `<td width=N>` columns.

#[test]
fn spacer_columns_keep_their_width() {
    let d = layout(&[[
        cell("id=a width=20", 0),
        cell("id=b", 50),
        cell("id=c width=20", 0),
    ]
    .concat()]);
    assert_near(&widths(&d, &["a", "b", "c"]).0, &[20.0, 560.0, 20.0]);
}

#[test]
fn a_width_column_beside_a_long_one_keeps_its_width() {
    let d = layout(&[[cell("id=a width=300", 3), cell("id=b", 30)].concat()]);
    assert_near(&widths(&d, &["a", "b"]).0, &[300.0, 300.0]);
    let d = layout_in(
        300,
        &[[cell("id=a style=\"width:150px\"", 2), cell("id=b", 10)].concat()],
    );
    assert_near(&widths(&d, &["a", "b"]).0, &[150.0, 150.0]);
}

#[test]
fn a_width_column_takes_its_width_before_the_auto_column_grows() {
    // min 40 + 40 into 300: the 200px column takes its 160 first, the auto
    // one the 60 left. Before #2280 equal shares gave 150 / 150.
    let d = layout_in(
        300,
        &[[cell("id=a width=200", 1), cell("id=b", 10)].concat()],
    );
    assert_near(&widths(&d, &["a", "b"]).0, &[200.0, 100.0]);
    // The 120 left after it goes to two auto columns by their room to grow.
    let d = layout_in(
        400,
        &[[cell("id=a width=200", 1), cell("id=b", 6), cell("id=c", 3)].concat()],
    );
    assert_near(&widths(&d, &["a", "b", "c"]).0, &[200.0, 125.7, 74.3]);
}

#[test]
fn a_colspan_over_a_width_column_keeps_its_width() {
    let d = layout_in(
        300,
        &[
            [cell("id=a width=100", 1), cell("id=b", 2)].concat(),
            cell("colspan=2", 20),
        ],
    );
    assert_near(&widths(&d, &["a", "b"]).0, &[100.0, 200.0]);
}
