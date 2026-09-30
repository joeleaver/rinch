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
            if let Some(el) = stale.item(i).and_then(|n| n.dyn_into::<web_sys::Element>().ok()) {
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
    handle.load_doc(table_doc(&handle, rows));
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
    ["grid-column-start", "grid-column-end", "grid-row-start", "grid-row-end"]
        .map(|p| cs.get_property_value(p).unwrap())
}

fn s4(a: &str, b: &str, c: &str, d: &str) -> [String; 4] {
    [a, b, c, d].map(str::to_string)
}

/// Control: an ordinary merged table is placed as its attributes say, a
/// `colspan = 2` cell over two cells side by side, and a `rowspan = 2` cell
/// beside them.
#[wasm_bindgen_test]
fn an_ordinary_merged_table_is_placed_by_its_attributes() {
    let m = mount(&[vec![(2, 1), (1, 2)], vec![(1, 1), (1, 1)]]);
    let c = cells(&m);
    assert_eq!(c.len(), 4, "control: four cells");
    let got: Vec<_> = c.iter().map(placement).collect();
    assert_eq!(
        got,
        vec![
            s4("span 2", "auto", "auto", "auto"),
            s4("auto", "auto", "span 2", "auto"),
            s4("auto", "auto", "auto", "auto"),
            s4("auto", "auto", "auto", "auto"),
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
            s4("span 2", "auto", "span 2", "auto"),
            s4("auto", "auto", "auto", "auto"),
            s4("auto", "auto", "auto", "auto"),
        ]
    );
}

/// Four rows of `colspan = 1_000_000, rowspan = i64::MAX`: the map gives the
/// first cell the grid's capped width and all four rows and finds the other
/// three rows full, so those cells are bands across the grid below it, and the
/// table is exactly as tall as the four.
#[wasm_bindgen_test]
fn a_table_of_unbounded_spans_is_placed_as_its_table_map() {
    let big = (1_000_000, i64::MAX);
    let m = mount(&[vec![big], vec![big], vec![big], vec![big]]);
    let c = cells(&m);
    assert_eq!(c.len(), 4, "control: four cells");
    assert_eq!(placement(&c[0])[2..], s4("", "", "span 4", "auto")[2..]);
    for band in &c[1..] {
        assert_eq!(placement(band), s4("1", "-1", "auto", "auto"));
    }
    let t = table(&m);
    let r: Vec<_> = c.iter().map(|e| rect(e, &t)).collect();
    let mut y = r[0].1 + r[0].3;
    for (i, b) in r.iter().enumerate().skip(1) {
        assert!((b.0 - r[0].0).abs() < 0.5 && (b.2 - r[0].2).abs() < 0.5, "{i}: {r:?}");
        assert!((b.1 - y).abs() < 0.5, "band {i} at {}, expected {y}: {r:?}", b.1);
        y = b.1 + b.3;
    }
    let th = t.get_bounding_client_rect().height();
    assert!((th - y).abs() <= 2.5, "table {th} tall, cells end at {y}: {r:?}");
}
