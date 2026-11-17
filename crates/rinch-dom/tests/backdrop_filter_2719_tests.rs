//! #2719: `backdrop-filter`, on the software painter against Chrome 157.
//!
//! The page is `tests/fixtures/backdrop_2719.html`, its Chrome screenshots
//! `tests/fixtures/backdrop_2719_chrome_{1,2}x.png`, made as
//! `filter_blur_2719_tests.rs`'s are. The cases and their tolerances, and why,
//! are in `support/backdrop_2719_chrome.rs`.
#![cfg(feature = "software-renderer")]

#[path = "support/backdrop_2719_chrome.rs"]
mod backdrop;
#[path = "support/filter_2719_chrome.rs"]
mod chrome;

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const PAGE: &str = backdrop::PAGE;
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
    let html = part(page, "<body>", "</body>");
    let html = &html[..html.find("<script>").unwrap_or(html.len())];
    doc.set_inner_html(body, html);
    doc.resolve_layout(W, H);
    if let Some(&s) = doc.query_selector_all("#scroller").first() {
        doc.set_scroll_top(s, 37.0);
        doc.resolve_layout(W, H);
    }
    doc
}

fn render(page: &str, scale: f64) -> (u32, u32, Vec<u8>) {
    let mut doc = document(page);
    let (pw, ph) = ((W as f64 * scale) as u32, (H as f64 * scale) as u32);
    let mut p = TinySkiaPainter::new(pw, ph);
    p.fill_white();
    let mut lcx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(&doc.tree, &mut p, scale, (W, H), &mut doc.font_cx, &mut lcx);
    (pw, ph, p.pixels().to_vec())
}

/// Chrome's RGB pixels at `scale`.
fn decode(scale: u32) -> Vec<[u8; 3]> {
    let img = image::load_from_memory(backdrop::chrome_png(scale))
        .unwrap()
        .to_rgb8();
    img.pixels().map(|p| p.0).collect()
}

#[test]
fn backdrops_match_chrome_at_1x() {
    let (w, _, px) = render(PAGE, 1.0);
    chrome::check_against_chrome(1, w, &px, &decode(1), backdrop::CASES);
}

#[test]
fn backdrops_match_chrome_at_2x() {
    let (w, _, px) = render(PAGE, 2.0);
    chrome::check_against_chrome(2, w, &px, &decode(2), backdrop::CASES);
}

/// Positive control: without `backdrop-filter` the page does not match.
#[test]
fn the_page_without_backdrops_does_not_match() {
    let page = PAGE.replace(
        "</style>",
        "* { backdrop-filter: none !important; }</style>",
    );
    let (_, _, plain) = render(&page, 1.0);
    let (_, _, with) = render(PAGE, 1.0);
    let differ = plain
        .chunks(4)
        .zip(with.chunks(4))
        .filter(|(a, b)| a != b)
        .count();
    assert!(differ > 10_000, "only {differ} pixels differ");
}

// ── Partial repaints ────────────────────────────────────────────────────────

const GLASS: &str = r#"<!doctype html><html><head><style>
html, body { margin: 0; background: #fff; }
body { width: 360px; height: 260px; position: relative; }
#bg { position: absolute; left: 0; top: 0; width: 360px; height: 260px;
      background: repeating-linear-gradient(90deg, #135 0 7px, #fd4 7px 13px); }
#track { position: absolute; left: 21px; top: 23px; width: 141px; height: 71px; }
#mover { position: absolute; left: 10px; top: 14px; width: 23px; height: 19px; background: #e11; }
#glass { position: absolute; left: 61px; top: 29px; width: 151px; height: 87px; border-radius: 11px;
         backdrop-filter: blur(6.5px) brightness(1.2); background: rgba(255, 255, 255, 0.2); }
#sc2 { position: absolute; left: 230px; top: 140px; width: 110px; height: 100px; overflow: hidden; }
#sc2 > .c { width: 110px; height: 400px;
                 background: repeating-linear-gradient(0deg, #06c 0 6px, #fff 6px 12px); }
#sc2 > .g { position: absolute; left: 11px; top: 30px; width: 80px; height: 40px;
                 backdrop-filter: blur(4px); }
</style></head><body><div id="bg"></div><div id="track"><div id="mover"></div></div><div id="glass"></div><div id="sc2"><div class="c"></div><div class="g"></div></div></body></html>"#;

fn by_id(doc: &RinchDocument, id: &str) -> rinch_core::dom::NodeId {
    doc.query_selector_all(&format!("#{id}"))[0]
}

/// Paint `page` in full, apply `change`, repaint only the damage the change
/// names through the damage clip, and compare with a full paint.
fn partial_matches_full(page: &str, scale: f64, change: impl Fn(&mut RinchDocument)) {
    use peniko::kurbo::Rect;
    let mut doc = document(page);
    let (pw, ph) = ((W as f64 * scale) as u32, (H as f64 * scale) as u32);
    let paint = |doc: &mut RinchDocument, p: &mut TinySkiaPainter| {
        let mut lcx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(&doc.tree, p, scale, (W, H), &mut doc.font_cx, &mut lcx);
    };
    let mut p = TinySkiaPainter::new(pw, ph);
    p.fill_white();
    paint(&mut doc, &mut p);
    let before = p.pixels().to_vec();
    doc.tree.consume_paint_dirty();
    change(&mut doc);
    doc.resolve_layout(W, H);
    let damage = rinch_dom::paint::compute_damage(&doc.tree, scale, pw as f64, ph as f64);
    assert!(!damage.is_full(), "the change damaged the whole frame");
    let rects: Vec<Rect> = damage.rects().to_vec();
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
    paint(&mut doc, &mut p);
    rinch_dom::paint::set_dirty_region(None);
    let partial = p.pixels().to_vec();
    // The full paint of the new state, on a fresh document: the same one
    // would draw from the rasters it kept (#2719), stale ones included.
    let mut fresh = document(page);
    change(&mut fresh);
    fresh.resolve_layout(W, H);
    let mut full = TinySkiaPainter::new(pw, ph);
    full.fill_white();
    paint(&mut fresh, &mut full);
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

/// Content moving behind a backdrop element: the element's blurred pixels
/// change as far as the blur spreads the move, past the mover's own rects,
/// and are repainted.
#[test]
fn content_moving_behind_a_backdrop_repaints_its_spread() {
    for scale in [1.0, 2.0] {
        // Into the glass, then within it.
        partial_matches_full(GLASS, scale, |doc| {
            let m = by_id(doc, "mover");
            doc.set_attribute(m, "style", "left: 76px; top: 38px");
        });
        partial_matches_full(GLASS, scale, |doc| {
            let m = by_id(doc, "mover");
            doc.set_attribute(m, "style", "left: 38px; top: 18px; background: #1e1");
        });
    }
}

/// The backdrop element's own filter changing repaints it.
#[test]
fn a_backdrop_filter_change_repaints_the_element() {
    partial_matches_full(GLASS, 1.0, |doc| {
        let g = by_id(doc, "glass");
        doc.set_attribute(g, "style", "backdrop-filter: blur(2px) grayscale(1)");
    });
}

/// A scroller scrolling under a backdrop element inside it: what is behind
/// the glass is the scrolled content.
#[test]
fn a_scroller_scrolling_behind_its_glass_repaints_it() {
    partial_matches_full(GLASS, 1.0, |doc| {
        let s = by_id(doc, "sc2");
        doc.set_scroll_top(s, 23.0);
    });
}

// ── Stacking and containing blocks ──────────────────────────────────────────

/// `backdrop-filter` is a stacking context and a containing block for an
/// absolute descendant (Filter Effects 2; Chrome 157: the `inset: 0` child
/// fills it).
#[test]
fn a_backdrop_filter_contains_absolute_descendants() {
    let page = r#"<!doctype html><html><head><style>
html, body { margin: 0; }
#f { margin: 37px 0 0 23px; width: 101px; height: 59px; backdrop-filter: blur(3px); }
#a { position: absolute; inset: 0; }
</style></head><body><div id="f"><div id="a"></div></div></body></html>"#;
    let doc = document(page);
    let a = doc.tree.get(by_id(&doc, "a").0).unwrap();
    assert_eq!((a.layout.width, a.layout.height), (101.0, 59.0));
    assert!(
        doc.tree
            .get(by_id(&doc, "f").0)
            .unwrap()
            .creates_stacking_context()
    );
}

/// A `backdrop-filter` that changes nothing — the component library's
/// `Modal` overlay declares `blur(var(--rinch-modal-overlay-blur, 0))` — is
/// still a stacking context but paints no backdrop: no second paint.
#[test]
fn an_identity_backdrop_filter_paints_no_backdrop() {
    let page = r#"<!doctype html><html><head><style>
html, body { margin: 0; background: #fff; }
#o { position: absolute; inset: 0; backdrop-filter: blur(0) brightness(1); background: rgba(0, 0, 0, 0.3); }
</style></head><body><div id="o"></div></body></html>"#;
    let mut doc = document(page);
    assert!(
        !doc.tree.backdrop_filters,
        "an identity list turned the backdrop machinery on"
    );
    assert!(
        doc.tree
            .get(by_id(&doc, "o").0)
            .unwrap()
            .creates_stacking_context()
    );
    doc.tree.perf.reset();
    let mut p = TinySkiaPainter::new(360, 260);
    let mut lcx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(&doc.tree, &mut p, 1.0, (W, H), &mut doc.font_cx, &mut lcx);
    let s = doc.tree.perf.end_frame();
    assert_eq!(s.get(rinch_dom::perf::Counter::BackdropRasters), 0);
}

// ── The kept backdrop ───────────────────────────────────────────────────────

/// A modal over a blurred overlay (#2719 review, M2): the panel is the
/// overlay's later, positioned sibling, so scrolling its body changes nothing
/// behind the overlay — the overlay is drawn from the backdrop kept from the
/// last paint, with no second paint of the page. A change behind it (the
/// page under the overlay) paints it again. Every frame is a full repaint,
/// as the embed overlay's `build_scene`, compared with a fresh document.
#[test]
fn a_modal_scrolling_over_its_blurred_overlay_keeps_the_backdrop() {
    use rinch_dom::perf::Counter::{BackdropCacheHits, BackdropRasters};
    let page = r#"<!doctype html><html><head><style>
html, body { margin: 0; background: #fff; }
body { width: 360px; height: 260px; position: relative; }
#page { position: absolute; left: 0; top: 0; width: 360px; height: 260px;
        background: repeating-linear-gradient(90deg, #135 0 7px, #fd4 7px 13px); }
#page > i { position: absolute; left: 20px; top: 20px; width: 30px; height: 20px; background: #e11; }
#root { position: absolute; inset: 0; }
#overlay { position: absolute; inset: 0; background: rgba(0, 0, 0, 0.3); backdrop-filter: blur(4px); }
#panel { position: absolute; left: 80px; top: 40px; width: 200px; height: 160px; background: #fff;
         z-index: 1; }
#body { position: absolute; left: 10px; top: 10px; width: 180px; height: 140px; overflow: hidden; }
#body > div { height: 600px; background: repeating-linear-gradient(0deg, #06c 0 6px, #fff 6px 12px); }
</style></head><body><div id="page"><i></i></div><div id="root"><div id="overlay"></div><div id="panel"><div id="body"><div></div></div></div></div></body></html>"#;
    // The shell's order: the change lists drained, then the paint.
    let frame = |doc: &mut RinchDocument| {
        doc.tree.consume_paint_dirty();
        doc.tree.perf.reset();
        let mut p = TinySkiaPainter::new(W as u32, H as u32);
        p.fill_white();
        let mut lcx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut p,
            1.0,
            (W, H),
            &mut doc.font_cx,
            &mut lcx,
        );
        let s = doc.tree.perf.end_frame();
        (
            p.pixels().to_vec(),
            s.get(BackdropRasters),
            s.get(BackdropCacheHits),
        )
    };
    let fresh = |scroll: f64, mover: Option<&str>| {
        let mut d = document(page);
        if let Some(style) = mover {
            let pg = by_id(&d, "page");
            let i = d.get_children(pg)[0];
            d.set_attribute(i, "style", style);
        }
        let b = by_id(&d, "body");
        d.set_scroll_top(b, scroll);
        d.resolve_layout(W, H);
        frame(&mut d).0
    };
    let mut doc = document(page);
    assert_eq!(frame(&mut doc).1, 1, "the first paint paints the backdrop");
    let body = by_id(&doc, "body");
    doc.set_scroll_top(body, 37.0);
    doc.resolve_layout(W, H);
    let (px, rasters, hits) = frame(&mut doc);
    assert_eq!(
        (rasters, hits),
        (0, 1),
        "scrolling the panel repainted the backdrop"
    );
    assert_eq!(px, fresh(37.0, None));
    // The page behind the overlay changes: the backdrop is painted again.
    let pg = by_id(&doc, "page");
    let i = doc.get_children(pg)[0];
    doc.set_attribute(i, "style", "left: 140px; background: #1e1");
    doc.resolve_layout(W, H);
    let (px, rasters, hits) = frame(&mut doc);
    assert_eq!(
        (rasters, hits),
        (1, 0),
        "a change behind kept a stale backdrop"
    );
    assert_eq!(px, fresh(37.0, Some("left: 140px; background: #1e1")));
}
