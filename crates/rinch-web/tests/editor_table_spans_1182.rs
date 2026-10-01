//! #1182 in a browser: an editor table's cells are placed by their
//! `TableMap` rectangles, not by their raw `colspan` / `rowspan`.
//!
//! ```text
//! CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_table_spans_1182
//! ```
//!
//! The native twin is `rinch-editor-view`'s `view::table_span_tests` (the
//! written placement) and `rinch`'s `editor_table_span_1182_tests` (the
//! desktop layout). Here the browser is the authority on what the written
//! placement lays out to.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::AttrValue;
use rinch_editor_core::model::{Attrs, Fragment, Node};
use rinch_web::{EditorHandle, RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-test-host-table-spans";

fn table_doc(handle: &EditorHandle, rows: &[Vec<(i64, i64)>]) -> Node {
    let state = handle.state();
    let s = state.schema();
    let rows: Vec<Node> = rows
        .iter()
        .enumerate()
        .map(|(ri, r)| {
            let cells: Vec<Node> = r
                .iter()
                .enumerate()
                .map(|(ci, &(c, h))| {
                    let p = s
                        .create_node(
                            "paragraph",
                            Attrs::new(),
                            Fragment::from_node(s.text(&format!("r{ri}c{ci}")).unwrap()),
                        )
                        .unwrap();
                    s.create_node(
                        "table_cell",
                        Attrs::from_iter([
                            ("colspan", AttrValue::Int(c)),
                            ("rowspan", AttrValue::Int(h)),
                        ]),
                        Fragment::from_node(p),
                    )
                    .unwrap()
                })
                .collect();
            s.create_node("table_row", Attrs::new(), Fragment::from_children(cells))
                .unwrap()
        })
        .collect();
    let t = s
        .create_node("table", Attrs::new(), Fragment::from_children(rows))
        .unwrap();
    s.branch("doc", Fragment::from_node(t)).unwrap()
}

struct Mounted {
    root: RootHandle,
    host: web_sys::Element,
}

impl Drop for Mounted {
    fn drop(&mut self) {
        self.root.unmount();
        self.host.remove();
    }
}

fn mount(rows: &[Vec<(i64, i64)>]) -> Mounted {
    if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
        for i in 0..stale.length() {
            if let Some(el) = stale
                .item(i)
                .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
            {
                el.remove();
            }
        }
    }
    let host = document().create_element("div").unwrap();
    host.set_attribute(HOST_MARKER, "").unwrap();
    host.set_attribute(
        "style",
        "width: 400px; font-family: monospace; font-size: 16px; line-height: 20px;",
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let handle = create_editor();
    // As an app's own transaction: a load would cap the 1,000,000 colspans
    // at 1000 (#1214), and this shape is about spans that reach the model
    // unbounded.
    let table = table_doc(&handle, rows);
    assert!(handle.update(|st| {
        let mut tr = st.tr();
        tr.replace_with(0, st.doc.content_size(), table.content().clone())
            .ok()?;
        Some(tr)
    }));
    let mounted = handle.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| mounted.mount(scope),
    );
    Mounted { root, host }
}

fn cells(m: &Mounted) -> Vec<web_sys::Element> {
    let list = m
        .host
        .query_selector_all("[data-pm-type='table_cell']")
        .unwrap();
    (0..list.length())
        .map(|i| list.item(i).unwrap().dyn_into().unwrap())
        .collect()
}

fn table(m: &Mounted) -> web_sys::Element {
    m.host
        .query_selector("[data-pm-type='table']")
        .unwrap()
        .expect("the table")
}

/// `(x, y, width, height)` relative to the table.
fn rect(el: &web_sys::Element, t: &web_sys::Element) -> (f64, f64, f64, f64) {
    let r = el.get_bounding_client_rect();
    let o = t.get_bounding_client_rect();
    (r.x() - o.x(), r.y() - o.y(), r.width(), r.height())
}

/// Computed `grid-{column,row}-{start,end}`: the browser's reading of what the
/// view wrote.
fn placement(el: &web_sys::Element) -> [String; 4] {
    let cs = web_sys::window()
        .unwrap()
        .get_computed_style(el)
        .unwrap()
        .unwrap();
    [
        "grid-column-start",
        "grid-column-end",
        "grid-row-start",
        "grid-row-end",
    ]
    .map(|p| cs.get_property_value(p).unwrap())
}

fn s4(a: &str, b: &str, c: &str, d: &str) -> [String; 4] {
    [a, b, c, d].map(str::to_string)
}

/// Control: an ordinary merged table is placed as its attributes say, a
/// `colspan = 2` cell over two cells side by side, and a `rowspan = 2` cell
/// beside them — on the grid lines of its map rectangles (#1209).
#[wasm_bindgen_test]
fn an_ordinary_merged_table_is_placed_by_its_attributes() {
    let m = mount(&[vec![(2, 1), (1, 2)], vec![(1, 1), (1, 1)]]);
    let c = cells(&m);
    assert_eq!(c.len(), 4, "control: four cells");
    let got: Vec<_> = c.iter().map(placement).collect();
    assert_eq!(
        got,
        vec![
            s4("1", "3", "1", "2"),
            s4("3", "4", "1", "3"),
            s4("1", "2", "2", "3"),
            s4("2", "3", "2", "3"),
        ]
    );
    let t = table(&m);
    let r: Vec<_> = c.iter().map(|e| rect(e, &t)).collect();
    assert!((r[0].2 - (r[2].2 + r[3].2)).abs() < 1.0, "{r:?}");
    assert!((r[1].3 - (r[0].3 + r[2].3)).abs() < 1.0, "{r:?}");
}

/// A rowspan past the last row is placed cut at it, as the map cuts it.
#[wasm_bindgen_test]
fn an_overlong_rowspan_is_placed_cut_at_the_last_row() {
    let m = mount(&[vec![(2, i64::MAX), (1, 1)], vec![(1, 1)]]);
    let got: Vec<_> = cells(&m).iter().map(placement).collect();
    assert_eq!(
        got,
        vec![
            s4("1", "3", "1", "3"),
            s4("3", "4", "1", "2"),
            s4("3", "4", "2", "3"),
        ]
    );
}

/// #1209: `[[A rowspan=2, X], [B, C]]` is `A X . / A B C` in the map, and B
/// and C are drawn in row 1 under X and the slot beside it. Auto-placement put
/// B in that empty slot of row 0, beside X.
#[wasm_bindgen_test]
fn a_cell_in_a_row_a_rowspan_leaves_short_stays_in_its_row() {
    let m = mount(&[vec![(1, 2), (1, 1)], vec![(1, 1), (1, 1)]]);
    let c = cells(&m);
    assert_eq!(c.len(), 4, "control: four cells");
    let t = table(&m);
    let r: Vec<_> = c.iter().map(|e| rect(e, &t)).collect();
    let [a, x, b, cc] = [r[0], r[1], r[2], r[3]];
    assert!(b.1 > x.1 + 1.0, "B is in row 1, below X: {r:?}");
    assert!((b.1 - cc.1).abs() < 0.5, "B and C share row 1: {r:?}");
    assert!((b.0 - x.0).abs() < 0.5 && cc.0 > b.0, "B under X: {r:?}");
    assert!(
        (cc.1 + cc.3 - (a.1 + a.3)).abs() <= 1.0,
        "A spans both: {r:?}"
    );
}

/// Four rows of `colspan = 1_000_000, rowspan = i64::MAX`. The map's grid is
/// `2^22 / 4 = 1_048_576` columns wide: the first cell spans 1_000_000 of them
/// and all four rows, the second the 48_576 left beside it and the three rows
/// below its own, and the last two find their rows full — no slot, so they are
/// bands across the grid below. Written raw, every cell asked for 1_000_000
/// columns and Chrome's clamped `span 1e+07` rows.
#[wasm_bindgen_test]
fn a_table_of_unbounded_spans_is_placed_as_its_table_map() {
    let big = (1_000_000, i64::MAX);
    let m = mount(&[vec![big], vec![big], vec![big], vec![big]]);
    let c = cells(&m);
    assert_eq!(c.len(), 4, "control: four cells");
    let got: Vec<_> = c.iter().map(placement).collect();
    // Chrome serialises `span 1000000` as `span 1e+06`; the other three are exact.
    assert_eq!(got[0][2..], s4("", "", "span 4", "auto")[2..], "{got:?}");
    assert_eq!(got[1], s4("span 48576", "auto", "span 3", "auto"));
    assert_eq!(got[2], s4("1", "-1", "auto", "auto"));
    assert_eq!(got[3], s4("1", "-1", "auto", "auto"));
    let t = table(&m);
    let r: Vec<_> = c.iter().map(|e| rect(e, &t)).collect();
    // The first cell spans the map's 1_000_000 of 1_048_576 columns — of the
    // bands' width, which is the grid's — and the second starts where it ends.
    // (Auto-placement puts the second at the first cell's top, where the map
    // has it one row down: #1209, which places a table wider than 9999
    // columns — Stylo's line cap — by spans in both axes. Its own padding makes it wider than the
    // 48_576 columns it spans, so it is not asserted to end at the grid's edge.)
    let grid_w = r[2].2;
    assert!(
        (r[0].2 / grid_w - 1_000_000.0 / 1_048_576.0).abs() < 0.005,
        "{r:?}"
    );
    assert!((r[1].0 - (r[0].0 + r[0].2)).abs() < 1.0, "{r:?}");
    assert!((r[1].1 - r[0].1).abs() < 0.5, "#1209 moved: {r:?}");
    // The bands span the grid and stack below both.
    let mut y = (r[0].1 + r[0].3).max(r[1].1 + r[1].3);
    for (i, b) in r.iter().enumerate().skip(2) {
        assert!(
            (b.0 - r[0].0).abs() < 0.5 && (b.2 - grid_w).abs() < 1.0,
            "{i}: {r:?}"
        );
        assert!(
            (b.1 - y).abs() < 1.0,
            "band {i} at {}, expected {y}: {r:?}",
            b.1
        );
        y = b.1 + b.3;
    }
    let th = t.get_bounding_client_rect().height();
    assert!(
        (th - y).abs() <= 2.5,
        "table {th} tall, cells end at {y}: {r:?}"
    );
}
