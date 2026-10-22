//! #1156 / #1157 — the rasterised background-tile cache's eviction and
//! release.
//!
//! - #1156: the cache was cleared whole when an insert would pass its 32 MB
//!   budget, so a working set just over the budget re-rasterised **every**
//!   tile on every frame. It now evicts the least recently used entries, never
//!   one the frame being painted has used, and declines to keep a tile that
//!   would need one: a working set of three 11.6 MB tiles (34.7 MB) rasterises
//!   three tiles on its first frame and one on each later frame (the two that
//!   fit stay; the third is drawn uncached).
//! - #1157: the cache was a thread-local that lived as long as the thread. It
//!   now belongs to the document (dropped with it) and forgets a tile no frame
//!   has used for `TILE_CACHE_IDLE_FRAMES` painted frames.
#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use rinch_dom::perf::Counter;

const SCALE: f64 = 2.0;
const VW: f32 = 900.0;
const VH: f32 = 900.0;

fn paint(doc: &mut RinchDocument) {
    let mut painter =
        TinySkiaPainter::new((VW as f64 * SCALE) as u32, (VH as f64 * SCALE) as u32);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        SCALE,
        (VW, VH),
        &mut doc.font_cx,
        &mut layout_cx,
    );
}

/// Rasters and cache hits of one painted frame.
fn frame(doc: &mut RinchDocument) -> (u64, u64) {
    let r = doc.tree.perf.get(Counter::BackgroundTileRasters);
    let h = doc.tree.perf.get(Counter::BackgroundTileCacheHits);
    paint(doc);
    (
        doc.tree.perf.get(Counter::BackgroundTileRasters) - r,
        doc.tree.perf.get(Counter::BackgroundTileCacheHits) - h,
    )
}

/// A box whose repeating gradient tile is 850 CSS px = 1700 device px square:
/// 1700 * 1700 * 4 = 11.56 MB per raster. `colour` makes each tile distinct.
fn big_tile_box(doc: &mut RinchDocument, colour: &str) -> NodeId {
    let body = doc.body();
    let node = doc.create_element("div");
    doc.set_attribute(
        node,
        "style",
        &format!(
            "position: absolute; left: 0; top: 0; width: 900px; height: 900px; \
             background-image: linear-gradient(to right, {colour}, rgb(1, 2, 3)); \
             background-size: 850px 850px"
        ),
    );
    doc.append_child(body, node);
    node
}

fn document(colours: &[&str]) -> (RinchDocument, Vec<NodeId>) {
    let mut doc = RinchDocument::new();
    doc.load_css("html, body { margin: 0; }");
    let nodes = colours.iter().map(|c| big_tile_box(&mut doc, c)).collect();
    doc.resolve_layout(VW, VH);
    (doc, nodes)
}

/// #1156: three 11.56 MB tiles against a 32 MB budget. The clear-all cache
/// re-rasterised all three on every frame (3, 3, 3); the LRU keeps two.
#[test]
fn a_working_set_just_over_the_budget_rasterises_one_tile_a_frame() {
    let (mut doc, _) = document(&["rgb(250, 0, 0)", "rgb(0, 250, 0)", "rgb(0, 0, 250)"]);
    assert_eq!(frame(&mut doc), (3, 0), "first frame: every tile is new");
    for i in 1..4 {
        assert_eq!(
            frame(&mut doc),
            (1, 2),
            "frame {i}: two tiles fit the budget and stay cached; only the third is rasterised"
        );
    }
}

/// Two tiles that fit stay cached however often they are painted (control
/// for the test above: the cache does hit at this size).
#[test]
fn a_working_set_under_the_budget_rasterises_nothing_after_the_first_frame() {
    let (mut doc, _) = document(&["rgb(250, 0, 0)", "rgb(0, 250, 0)"]);
    assert_eq!(frame(&mut doc), (2, 0));
    for _ in 0..3 {
        assert_eq!(frame(&mut doc), (0, 2));
    }
}
