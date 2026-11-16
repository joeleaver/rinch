//! #2719: `filter: blur()` and `drop-shadow()`, drawn through the offscreen
//! raster (`paint::filter`), on the software painter against Chrome 157.
//!
//! The page is `tests/fixtures/filter_2719.html`; Chrome's screenshots of it
//! are `tests/fixtures/filter_2719_chrome_{1,2}x.png` (the top-left 360x260
//! CSS px, RGB), made with, for `S` in 1 and 2,
//!
//! ```text
//! google-chrome --headless=new --no-sandbox --disable-gpu --hide-scrollbars \
//!     --force-device-scale-factor=S --window-size=360,400 \
//!     --user-data-dir=<a fresh dir> --screenshot=out.png file://<the page>
//! ```
//!
//! and cropping `out.png` to that rect. Software raster, so Chrome blurs with
//! Skia's three box blurs, which the raster reproduces: the blurred boxes and
//! the gradient are within 3 levels of Chrome everywhere. What is compared
//! more loosely, and why:
//!
//! - **Curved shapes with a crisp edge** (the disc and the ring with their
//!   shadows): the shape's own anti-aliased edge is rinch's, not Skia's, and
//!   differs by up to 66 levels on an edge pixel. Each pixel is compared with
//!   the range of Chrome's 3x3 neighbourhood (an edge a pixel off passes)
//!   and the mean difference is bounded.
//! - **Text** (bold text with a drop shadow, blurred text): the glyphs are
//!   rinch's, and the same text unfiltered already differs from Chrome by
//!   2.9–7.0 levels on average over these regions (hinting and line
//!   placement). Bounded by the mean.
#![cfg(feature = "software-renderer")]

use peniko::Brush;
use peniko::kurbo::Rect;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const PAGE: &str = chrome::PAGE;
const W: f32 = 360.0;
const H: f32 = 260.0;

fn part<'a>(page: &'a str, open: &str, close: &str) -> &'a str {
    let start = page.find(open).unwrap() + open.len();
    &page[start..start + page[start..].find(close).unwrap()]
}

fn document(page: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css(part(page, "<style>", "</style>"));
    let body = doc.body();
    doc.set_inner_html(body, part(page, "<body>", "</body>"));
    doc.resolve_layout(W, H);
    doc
}

fn paint(doc: &mut RinchDocument, p: &mut TinySkiaPainter, scale: f64) {
    let mut lcx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(&doc.tree, p, scale, (W, H), &mut doc.font_cx, &mut lcx);
}

fn render(page: &str, scale: f64) -> (u32, u32, Vec<u8>) {
    let mut doc = document(page);
    let (pw, ph) = ((W as f64 * scale) as u32, (H as f64 * scale) as u32);
    let mut p = TinySkiaPainter::new(pw, ph);
    p.fill_white();
    paint(&mut doc, &mut p, scale);
    (pw, ph, p.pixels().to_vec())
}

#[path = "support/filter_2719_chrome.rs"]
mod chrome;
use chrome::check_against_chrome;

/// Chrome's RGB pixels at `scale`.
fn decode(scale: u32) -> Vec<[u8; 3]> {
    let img = image::load_from_memory(chrome::chrome_png(scale))
        .unwrap()
        .to_rgb8();
    img.pixels().map(|p| p.0).collect()
}

#[test]
fn filters_match_chrome_at_1x() {
    let (w, _, px) = render(PAGE, 1.0);
    check_against_chrome(1, w, &px, &decode(1), chrome::CASES);
}

#[test]
fn filters_match_chrome_at_2x() {
    let (w, _, px) = render(PAGE, 2.0);
    check_against_chrome(2, w, &px, &decode(2), chrome::CASES);
}

/// The page with every filter taken out paints differently: the comparison
/// above is not passing on pages that would match without the feature.
#[test]
fn the_page_without_filters_does_not_match() {
    let page = PAGE.replace("</style>", "* { filter: none !important; }</style>");
    let (_, _, plain) = render(&page, 1.0);
    let (_, _, filtered) = render(PAGE, 1.0);
    let differ = plain
        .chunks(4)
        .zip(filtered.chunks(4))
        .filter(|(a, b)| a != b)
        .count();
    assert!(differ > 5000, "only {differ} pixels differ");
}

// ── Partial repaints ────────────────────────────────────────────────────────

const PANEL: &str = r#"<!doctype html><html><head><style>
html, body { margin: 0; background: #fff; }
body { width: 300px; height: 200px; position: relative; }
#panel { position: absolute; left: 61px; top: 47px; width: 113px; height: 71px;
         background: #fe8; filter: blur(6.5px); }
#panel > b { position: absolute; left: 13px; top: 17px; width: 21px; height: 19px; background: #a0c; }
#panel > i { position: absolute; left: 97px; top: 61px; width: 43px; height: 29px; background: #0a6; }
#ds { position: absolute; left: 197px; top: 103px; width: 37px; height: 31px; background: #48f;
      filter: drop-shadow(9px 6px 4.5px #400); }
#near { position: absolute; left: 7px; top: 157px; width: 17px; height: 13px; background: #999; }
</style></head><body><div id="panel"><b></b><i></i></div><div id="ds"></div><div id="near"></div></body></html>"#;

/// Paint `page` in full, apply `change`, then repaint only the damage the
/// change names (through the damage clip, as the desktop loop does) and
/// compare with a full paint of the new state.
fn partial_matches_full(page: &str, scale: f64, change: impl Fn(&mut RinchDocument)) {
    let mut doc = document(page);
    let (pw, ph) = ((W as f64 * scale) as u32, (H as f64 * scale) as u32);
    let mut p = TinySkiaPainter::new(pw, ph);
    p.fill_white();
    paint(&mut doc, &mut p, scale);
    let before = p.pixels().to_vec();
    doc.tree.consume_paint_dirty();
    change(&mut doc);
    doc.resolve_layout(W, H);
    let damage = rinch_dom::paint::compute_damage(&doc.tree, scale, pw as f64, ph as f64);
    assert!(!damage.is_full(), "the change damaged the whole frame");
    let rects: Vec<Rect> = damage.rects().to_vec();
    assert!(!rects.is_empty(), "the change named no damage");
    for r in &rects {
        let r = r
            .intersect(Rect::new(0.0, 0.0, pw as f64, ph as f64))
            .expand();
        p.clear_rect_white(
            r.x0 as u32,
            r.y0 as u32,
            r.width() as u32,
            r.height() as u32,
        );
    }
    p.push_damage_clip(&damage);
    rinch_dom::paint::set_dirty_rects(Some(&rects));
    paint(&mut doc, &mut p, scale);
    rinch_dom::paint::set_dirty_region(None);
    let partial = p.pixels().to_vec();
    // The full paint of the new state, on a fresh document: the same one
    // would draw from the rasters it kept, stale ones included.
    let mut fresh = document(page);
    change(&mut fresh);
    fresh.resolve_layout(W, H);
    let mut full = TinySkiaPainter::new(pw, ph);
    full.fill_white();
    paint(&mut fresh, &mut full, scale);
    assert_ne!(full.pixels(), &before[..], "the change changed nothing");
    let bad: Vec<(usize, usize)> = partial
        .chunks(4)
        .zip(full.pixels().chunks(4))
        .enumerate()
        .filter(|(_, (a, b))| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 1))
        .map(|(i, _)| (i % pw as usize, i / pw as usize))
        .collect();
    assert!(
        bad.is_empty(),
        "{} pixels of the partial repaint are not the new state, first {:?} (damage {rects:?})",
        bad.len(),
        &bad[..bad.len().min(5)]
    );
}

fn by_id(doc: &RinchDocument, id: &str) -> rinch_core::dom::NodeId {
    doc.query_selector_all(&format!("#{id}"))[0]
}

/// A child moving inside a blurred box: its blur, which spreads past the
/// child's own box by the filter's reach, is repainted where it was and where
/// it is.
#[test]
fn a_child_moving_in_a_blurred_box_repaints_its_spread() {
    for scale in [1.0, 2.0] {
        partial_matches_full(PANEL, scale, |doc| {
            let panel = by_id(doc, "panel");
            let b = doc.get_children(panel)[0];
            doc.set_attribute(b, "style", "left: 41px; top: 23px");
        });
    }
}

/// A child overflowing the blurred box, recoloured: its blurred ink past the
/// box is repainted.
#[test]
fn an_overflowing_child_recoloured_repaints_its_spread() {
    partial_matches_full(PANEL, 1.0, |doc| {
        let panel = by_id(doc, "panel");
        let i = doc.get_children(panel)[1];
        doc.set_attribute(i, "style", "background: #f40");
    });
}

/// A child removed from a blurred box clears its blurred pixels.
#[test]
fn a_child_removed_from_a_blurred_box_clears_its_spread() {
    partial_matches_full(PANEL, 1.0, |doc| {
        let panel = by_id(doc, "panel");
        let b = doc.get_children(panel)[0];
        doc.remove_child(panel, b);
    });
}

/// The filter itself changing: the old and the new reach are repainted.
#[test]
fn a_filter_change_repaints_both_reaches() {
    partial_matches_full(PANEL, 1.0, |doc| {
        let ds = by_id(doc, "ds");
        doc.set_attribute(ds, "style", "filter: drop-shadow(-7px -5px 2.5px #040)");
    });
    partial_matches_full(PANEL, 1.0, |doc| {
        let panel = by_id(doc, "panel");
        doc.set_attribute(panel, "style", "filter: blur(2.5px)");
    });
}

/// A box near a blurred one changing: only the damage it names is repainted,
/// and the blurred box's raster, drawn again for the part of it in that
/// damage, is the full paint's there.
#[test]
fn a_change_beside_a_blurred_box_repaints_it_exactly() {
    partial_matches_full(PANEL, 1.0, |doc| {
        let near = by_id(doc, "near");
        doc.set_attribute(near, "style", "left: 53px; top: 101px; background: #00f");
    });
}

// ── The cull ────────────────────────────────────────────────────────────────

/// A blurred box's child past the window's edge, and another's past a
/// scroller's clip, still blur into what shows: the cull inside the raster is
/// grown by the filter's reach. Each child is 70px out, past the 64px ink
/// margin every cull already allows, and blurred by a 40px deviation (113px
/// of reach).
#[test]
fn blur_reaches_in_from_past_the_window_and_a_clip() {
    let page = r#"<!doctype html><html><head><style>
html, body { margin: 0; background: #fff; }
body { width: 360px; height: 260px; position: relative; }
#win { position: absolute; left: 0; top: 0; width: 100px; height: 260px; filter: blur(40px); }
#win > i { position: absolute; left: -270px; top: -50px; width: 200px; height: 360px; background: #000; }
#scroller { position: absolute; left: 150px; top: 20px; width: 200px; height: 90px; overflow: hidden; }
#f { position: absolute; left: 0; top: 0; width: 200px; height: 90px; filter: blur(40px); }
#f > i { position: absolute; left: -50px; top: 160px; width: 300px; height: 300px; background: #000; }
</style></head><body><div id="win"><i></i></div>
<div id="scroller"><div id="f"><i></i></div></div></body></html>"#;
    let (w, _, px) = render(page, 1.0);
    let at = |x: usize, y: usize| px[(y * w as usize + x) * 4];
    // The window's left edge, 70px from the child's right edge.
    assert!(
        at(0, 130) < 252,
        "the blur from past the window is missing: {}",
        at(0, 130)
    );
    // The scroller's bottom row, 70px above the child's top.
    assert!(
        at(250, 109) < 252,
        "the blur from past the clip is missing: {}",
        at(250, 109)
    );
    assert_eq!(at(250, 112), 255, "the scroller's clip is not applied");
}

// ── Stacking and containing blocks ──────────────────────────────────────────

/// Any `filter` is a containing block for an absolute descendant (Filter
/// Effects 1 §5; Chrome 157: the `inset: 0` child fills the filtered box),
/// a `blur(0)` included.
#[test]
fn a_filter_contains_absolute_descendants() {
    for filter in ["blur(0)", "drop-shadow(0 0 0 #000)", "brightness(1)"] {
        let page = format!(
            r#"<!doctype html><html><head><style>
html, body {{ margin: 0; }}
#f {{ margin: 37px 0 0 23px; width: 101px; height: 59px; filter: {filter}; }}
#a {{ position: absolute; inset: 0; }}
</style></head><body><div id="f"><div id="a"></div></div></body></html>"#
        );
        let doc = document(&page);
        let a = by_id(&doc, "a");
        let f = by_id(&doc, "f");
        let n = doc.tree.get(a.0).unwrap();
        assert_eq!(
            (n.layout.width, n.layout.height),
            (101.0, 59.0),
            "{filter}: the absolute is not sized by the filtered box"
        );
        assert!(
            doc.tree.get(f.0).unwrap().creates_stacking_context(),
            "{filter}"
        );
    }
}

/// #2314's per-line text cull inside a blurred box: lines past the window
/// whose ink misses it but whose blur reaches it are drawn. The page paints
/// byte for byte as it does with the line cull off.
#[test]
fn the_text_line_cull_keeps_lines_blurred_into_view() {
    let mut lines = String::new();
    for i in 0..40 {
        lines.push_str(&format!("row {i} of blurred text<br>"));
    }
    let page = format!(
        r#"<!doctype html><html><head><style>
html, body {{ margin: 0; background: #fff; }}
body {{ width: 360px; height: 260px; position: relative; font-family: "DejaVu Sans"; }}
#p {{ position: absolute; left: 20px; top: -300px; width: 300px; font-size: 13px; line-height: 15px;
      filter: blur(30px); }}
#s {{ position: absolute; left: 20px; top: 330px; width: 300px; font: 13px/15px "DejaVu Sans";
      filter: drop-shadow(0 -40px 25px #000); }}
</style></head><body><div id="p">{lines}</div><div id="s">{lines}</div></body></html>"#
    );
    let culled = render(&page, 1.0).2;
    let was = rinch_dom::paint::set_text_line_cull(false);
    let all = render(&page, 1.0).2;
    rinch_dom::paint::set_text_line_cull(was);
    let differ = culled
        .chunks(4)
        .zip(all.chunks(4))
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(differ, 0, "{differ} pixels differ with the line cull on");
    // Positive control: the blur reaches into the window from both ends.
    let at = |x: usize, y: usize| culled[(y * 360 + x) * 4];
    assert!(at(150, 0) < 255, "nothing of the upper blur shows");
    assert!(at(150, 259) < 255, "nothing of the lower shadow shows");
}

// ── The kept raster ─────────────────────────────────────────────────────────

/// Full repaints (as the embed overlay's `build_scene` paints every frame)
/// draw an untouched filtered box from the raster kept from the last paint,
/// and rasterise again exactly the ones whose subtree changed or that moved.
/// Every frame is compared with a fresh document's paint of the same state.
#[test]
fn full_repaints_reuse_the_rasters_of_untouched_filters() {
    use rinch_dom::perf::Counter::{FilterCacheHits, FilterRasters};
    let mut doc = document(PANEL);
    // The shell's order: the change lists drained, then the paint.
    let frame = |doc: &mut RinchDocument| {
        doc.tree.consume_paint_dirty();
        doc.tree.perf.reset();
        let mut p = TinySkiaPainter::new(W as u32, H as u32);
        p.fill_white();
        paint(doc, &mut p, 1.0);
        let s = doc.tree.perf.end_frame();
        (
            p.pixels().to_vec(),
            s.get(FilterRasters),
            s.get(FilterCacheHits),
        )
    };
    let fresh = |styles: &[(&str, &str)]| {
        let mut d = document(PANEL);
        for (id, style) in styles {
            let n = by_id(&d, id);
            d.set_attribute(n, "style", style);
        }
        d.resolve_layout(W, H);
        let mut p = TinySkiaPainter::new(W as u32, H as u32);
        p.fill_white();
        paint(&mut d, &mut p, 1.0);
        p.pixels().to_vec()
    };
    let (_, rasters, hits) = frame(&mut doc);
    assert_eq!((rasters, hits), (2, 0), "the first paint rasterises both");
    let (px, rasters, hits) = frame(&mut doc);
    assert_eq!(
        (rasters, hits),
        (0, 2),
        "an unchanged frame rasterises nothing"
    );
    assert_eq!(px, fresh(&[]));
    // A box outside both filters changes: both are drawn from their rasters.
    let near = by_id(&doc, "near");
    doc.set_attribute(near, "style", "background: #0a0");
    doc.resolve_layout(W, H);
    let (px, rasters, hits) = frame(&mut doc);
    assert_eq!((rasters, hits), (0, 2));
    assert_eq!(px, fresh(&[("near", "background: #0a0")]));
    // A child inside the blurred panel changes: that one is rasterised again.
    let panel = by_id(&doc, "panel");
    let b = doc.get_children(panel)[0];
    doc.set_attribute(b, "style", "background: #0cf; left: 31px");
    doc.resolve_layout(W, H);
    let (px, rasters, hits) = frame(&mut doc);
    assert_eq!((rasters, hits), (1, 1));
    let want = {
        let mut d = document(PANEL);
        let n = by_id(&d, "near");
        d.set_attribute(n, "style", "background: #0a0");
        let p = by_id(&d, "panel");
        let b = d.get_children(p)[0];
        d.set_attribute(b, "style", "background: #0cf; left: 31px");
        d.resolve_layout(W, H);
        let mut pt = TinySkiaPainter::new(W as u32, H as u32);
        pt.fill_white();
        paint(&mut d, &mut pt, 1.0);
        pt.pixels().to_vec()
    };
    assert_eq!(px, want, "the changed panel is stale");
    // The drop-shadowed box moves: it is rasterised again where it is.
    let ds = by_id(&doc, "ds");
    doc.set_attribute(ds, "style", "left: 213px");
    doc.resolve_layout(W, H);
    let (_, rasters, hits) = frame(&mut doc);
    assert_eq!((rasters, hits), (1, 1));
}

/// A change inside a blurred box under a `scale(2.5)` spreads 2.5 times the
/// CSS reach on the device: the damage for it is grown by the reach through
/// the ancestor's transform (`filter::ancestor_reach`). Without the
/// transform's scale the ring between 1x and 2.5x the reach stays stale.
#[test]
fn a_change_inside_a_scaled_blur_repaints_its_scaled_spread() {
    let page = r#"<!doctype html><html><head><style>
html, body { margin: 0; background: #fff; }
body { width: 360px; height: 260px; position: relative; }
#zoom { position: absolute; left: 20px; top: 15px; width: 120px; height: 80px;
        transform: scale(2.5); transform-origin: 0 0; }
#panel { position: absolute; left: 13px; top: 11px; width: 91px; height: 57px; filter: blur(2.5px); }
#panel > b { position: absolute; left: 31px; top: 19px; width: 17px; height: 13px; background: #a0c; }
</style></head><body><div id="zoom"><div id="panel"><b></b></div></div></body></html>"#;
    partial_matches_full(page, 1.0, |doc| {
        let panel = by_id(doc, "panel");
        let b = doc.get_children(panel)[0];
        doc.set_attribute(b, "style", "background: #0a6");
    });
}
