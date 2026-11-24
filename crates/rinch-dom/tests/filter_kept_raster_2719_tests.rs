//! #2719 review: the kept filter rasters (`paint::raster_cache`) never show a
//! stale picture. Each probe paints twice (warm), applies a change, paints
//! again, and compares with a fresh document painted in the same state. Every
//! frame runs in the shell's order — the change lists drained, then the paint
//! (`build_scene_with`, the desktop `frame_damage`). Adopted from the
//! review's probes.
#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

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

fn by_id(doc: &RinchDocument, id: &str) -> rinch_core::dom::NodeId {
    doc.query_selector_all(&format!("#{id}"))[0]
}

/// One full frame in the shell's order: the change lists drained, then the
/// paint.
fn frame(doc: &mut RinchDocument, scale: f64) -> Vec<u8> {
    doc.tree.consume_paint_dirty();
    let (pw, ph) = ((W as f64 * scale) as u32, (H as f64 * scale) as u32);
    let mut p = TinySkiaPainter::new(pw, ph);
    p.fill_white();
    let mut lcx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(&doc.tree, &mut p, scale, (W, H), &mut doc.font_cx, &mut lcx);
    p.pixels().to_vec()
}

fn diff(a: &[u8], b: &[u8]) -> usize {
    a.chunks(4)
        .zip(b.chunks(4))
        .filter(|(x, y)| x.iter().zip(y.iter()).any(|(p, q)| p.abs_diff(*q) > 1))
        .count()
}

/// Paint twice (warm), apply `change`, paint, compare with a fresh document.
/// Returns the count of differing pixels, and asserts the change is visible.
fn probe(page: &str, scale: f64, change: impl Fn(&mut RinchDocument)) -> usize {
    let mut doc = document(page);
    let before = frame(&mut doc, scale);
    frame(&mut doc, scale);
    change(&mut doc);
    doc.resolve_layout(W, H);
    let after = frame(&mut doc, scale);
    let mut fresh = document(page);
    change(&mut fresh);
    fresh.resolve_layout(W, H);
    let want = frame(&mut fresh, scale);
    assert!(
        diff(&want, &before) > 0,
        "control: the change changed nothing"
    );
    diff(&after, &want)
}

const PANEL: &str = r#"<!doctype html><html><head><style>
html, body { margin: 0; background: #fff; }
body { width: 300px; height: 200px; position: relative; color: #000; }
#panel { position: absolute; left: 61px; top: 47px; width: 113px; height: 71px;
         background: #fe8; filter: blur(2.5px); font: 20px/24px sans-serif; }
#panel > b { position: absolute; left: 13px; top: 17px; width: 21px; height: 19px; background: #a0c; }
#panel > p { position: absolute; left: 10px; top: 40px; margin: 0; }
</style></head><body><div id="wrap"><div id="panel"><b></b><p id="t">Hello</p></div></div></body></html>"#;

fn recolour_child(doc: &mut RinchDocument) {
    let panel = by_id(doc, "panel");
    let b = doc.get_children(panel)[0];
    doc.set_attribute(b, "style", "background: #0cf; left: 31px");
}

#[test]
fn p1_a_child_recoloured_inside_a_filter() {
    let n = probe(PANEL, 1.0, recolour_child);
    assert_eq!(n, 0, "P1 stale");
}

#[test]
fn p3_inherited_colour_from_ancestor() {
    let n = probe(PANEL, 1.0, |doc| {
        let w = by_id(doc, "wrap");
        doc.set_attribute(w, "style", "color: #f00");
    });
    assert_eq!(n, 0);
}

#[test]
fn p3b_inherited_font_from_ancestor() {
    let page = PANEL.replace("font: 20px/24px sans-serif;", "line-height: 24px;");
    let n = probe(&page, 1.0, |doc| {
        let w = by_id(doc, "wrap");
        doc.set_attribute(w, "style", "font-size: 23px");
    });
    assert_eq!(n, 0);
}

#[test]
fn p4_text_change_inside() {
    let n = probe(PANEL, 1.0, |doc| {
        let t = by_id(doc, "t");
        doc.set_text_content(t, "World");
    });
    assert_eq!(n, 0);
}

#[test]
fn p5_scroll_inside_filter() {
    let page = r#"<!doctype html><html><head><style>
html, body { margin: 0; background: #fff; }
#panel { position: absolute; left: 61px; top: 47px; width: 113px; height: 71px; filter: blur(1.5px); }
#sc { width: 113px; height: 71px; overflow: hidden; }
#sc > div { height: 600px; background: repeating-linear-gradient(0deg, #06c 0 6px, #fff 6px 12px); }
</style></head><body><div id="panel"><div id="sc"><div></div></div></div></body></html>"#;
    let n = probe(page, 1.0, |doc| {
        let s = by_id(doc, "sc");
        doc.set_scroll_top(s, 3.0);
    });
    assert_eq!(n, 0);
}

#[test]
fn p6_nested_filter_change() {
    let page = r#"<!doctype html><html><head><style>
html, body { margin: 0; background: #fff; }
#outer { position: absolute; left: 61px; top: 47px; width: 113px; height: 71px; filter: drop-shadow(3px 4px 2px #000); }
#inner { position: absolute; left: 10px; top: 10px; width: 50px; height: 30px; background: #0a6; filter: blur(1px); }
</style></head><body><div id="outer"><div id="inner"></div></div></body></html>"#;
    let n = probe(page, 1.0, |doc| {
        let s = by_id(doc, "inner");
        doc.set_attribute(s, "style", "filter: blur(3px)");
    });
    assert_eq!(n, 0);
}

/// A move by part of a pixel. Box edges round to the nearest device pixel
/// (#2368, as Chrome's do), so 61.3px would paint where 61px does and change
/// nothing; 61.6px rounds to 62, a picture the kept raster must not hide.
#[test]
fn p7_subpixel_move() {
    let page = r#"<!doctype html><html><head><style>
html, body { margin: 0; background: #fff; }
#f { position: absolute; left: 61px; top: 47px; width: 51px; height: 31px; background: #a0c; filter: blur(1.2px); }
</style></head><body><div id="f"></div></body></html>"#;
    let n = probe(page, 1.0, |doc| {
        let s = by_id(doc, "f");
        doc.set_attribute(s, "style", "left: 61.6px");
    });
    assert_eq!(n, 0);
}

#[test]
fn p8_scale_change() {
    let mut doc = document(PANEL);
    frame(&mut doc, 1.0);
    frame(&mut doc, 1.0);
    let got = frame(&mut doc, 2.0);
    let mut fresh = document(PANEL);
    let want = frame(&mut fresh, 2.0);
    let n = diff(&got, &want);
    assert_eq!(n, 0);
}

#[test]
fn p9_theme_like_full_restyle() {
    let n = probe(PANEL, 1.0, |doc| {
        doc.load_css("#panel > b { background: #f60 !important; }");
    });
    assert_eq!(n, 0);
}

// ── Backdrops ───────────────────────────────────────────────────────────────

// ── Freed nodes, late loads, memory ────────────────────────────────────────

/// A filtered card discarded (as an `if` or a keyed `for` swapping cards
/// does), new nodes created in its slots, and painted: no stale-id panic
/// (#723), and the picture is the fresh document's.
#[test]
fn discarding_a_filtered_card_and_painting_again() {
    let page = r#"<!doctype html><html><head><style>
html, body { margin: 0; background: #fff; }
.card { position: absolute; top: 20px; width: 80px; height: 60px; background: #48f; filter: blur(3px); }
</style></head><body><div id="list"><div class="card" id="a" style="left: 20px"></div><div class="card" id="b" style="left: 140px"></div></div></body></html>"#;
    let swap = |doc: &mut RinchDocument| {
        let list = by_id(doc, "list");
        let a = by_id(doc, "a");
        doc.remove_child(list, a);
        doc.discard_node(a);
        for left in [20, 260] {
            let c = doc.create_element("div");
            doc.set_attribute(c, "class", "card");
            doc.set_attribute(c, "style", &format!("left: {left}px; background: #f80"));
            doc.append_child(list, c);
        }
    };
    let mut doc = document(page);
    frame(&mut doc, 1.0);
    frame(&mut doc, 1.0);
    swap(&mut doc);
    doc.resolve_layout(W, H);
    let got = frame(&mut doc, 1.0);
    let got2 = frame(&mut doc, 1.0);
    let mut fresh = document(page);
    swap(&mut fresh);
    fresh.resolve_layout(W, H);
    let want = frame(&mut fresh, 1.0);
    assert_eq!(diff(&got, &want), 0);
    assert_eq!(diff(&got2, &want), 0);
}

/// A font that arrives after the first paint, for text inside a blur: the
/// kept raster is drawn again with it.
#[test]
fn a_late_font_load_inside_a_filter_is_drawn() {
    const FACE: &[u8] = include_bytes!("../assets/fonts/SpaceGrotesk-VariableFont_wght.ttf");
    let page = r#"<!doctype html><html><head><style>
html, body { margin: 0; background: #fff; }
#p { position: absolute; left: 20px; top: 20px; width: 260px; filter: blur(1.5px);
     font: 40px/48px Probe2719Late, monospace; }
</style></head><body><div id="p">Lap 3 of 7</div></body></html>"#;
    let register = |doc: &mut RinchDocument| {
        let blob = peniko::Blob::new(std::sync::Arc::new(FACE));
        doc.font_cx.collection.register_fonts(
            blob,
            Some(parley::fontique::FontInfoOverride {
                family_name: Some("Probe2719Late"),
                ..Default::default()
            }),
        );
        doc.note_fonts_registered();
    };
    let mut doc = document(page);
    let before = frame(&mut doc, 1.0);
    frame(&mut doc, 1.0);
    register(&mut doc);
    doc.resolve_layout(W, H);
    let got = frame(&mut doc, 1.0);
    let mut fresh = RinchDocument::new();
    register(&mut fresh);
    let mut fresh = {
        fresh.load_css(part(page, "<style>", "</style>"));
        let body = fresh.body();
        fresh.set_inner_html(body, part(page, "<body>", "</body>"));
        fresh.resolve_layout(W, H);
        fresh
    };
    let want = frame(&mut fresh, 1.0);
    assert!(
        diff(&want, &before) > 0,
        "control: the font changed nothing"
    );
    assert_eq!(diff(&got, &want), 0, "the kept raster has the old font");
}

/// A picture that loads after the first paint, inside a drop shadow: drawn,
/// with its shadow, once it lands.
#[test]
fn a_late_image_load_inside_a_filter_is_drawn() {
    use base64::Engine;
    use rinch_core::image::{ImageLoadResult, ImageLoader};
    const RED: &str = "iVBORw0KGgoAAAANSUhEUgAAAAMAAAACCAYAAACddGYaAAAAEUlEQVR4nGP4z8DwH4YZkDkAm34L9XKwuTwAAAAASUVORK5CYII=";
    struct Later;
    impl ImageLoader for Later {
        fn load(&self, _: &str) -> ImageLoadResult {
            std::thread::sleep(std::time::Duration::from_millis(60));
            ImageLoadResult::Loaded(
                base64::engine::general_purpose::STANDARD
                    .decode(RED)
                    .unwrap(),
            )
        }
    }
    let page = r#"<!doctype html><html><head><style>
html, body { margin: 0; background: #fff; }
#f { position: absolute; left: 30px; top: 30px; width: 90px; height: 60px; filter: drop-shadow(6px 5px 3px #000); }
#f > img { width: 90px; height: 60px; display: block; }
</style></head><body><div id="f"><img src="later-2719.png"></div></body></html>"#;
    let load = |doc: &mut RinchDocument| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !doc.tree.image_cache.get("later-2719.png").is_some() {
            assert!(
                std::time::Instant::now() < deadline,
                "the picture never loaded"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
            doc.resolve_layout(W, H);
        }
        doc.resolve_layout(W, H);
    };
    let mut doc = RinchDocument::new();
    doc.tree.image_loader = Some(std::sync::Arc::new(Later));
    doc.load_css(part(page, "<style>", "</style>"));
    let body = doc.body();
    doc.set_inner_html(body, part(page, "<body>", "</body>"));
    doc.resolve_layout(W, H);
    let before = frame(&mut doc, 1.0);
    frame(&mut doc, 1.0);
    load(&mut doc);
    let got = frame(&mut doc, 1.0);
    let mut fresh = RinchDocument::new();
    fresh.tree.image_loader = Some(std::sync::Arc::new(Later));
    fresh.load_css(part(page, "<style>", "</style>"));
    let body = fresh.body();
    fresh.set_inner_html(body, part(page, "<body>", "</body>"));
    fresh.resolve_layout(W, H);
    load(&mut fresh);
    let want = frame(&mut fresh, 1.0);
    assert!(
        diff(&want, &before) > 0,
        "control: the picture changed nothing"
    );
    assert_eq!(diff(&got, &want), 0, "the kept raster lacks the picture");
}

/// Forty large blurred cards at 2x would hold about 100 MB of rasters: the
/// kept ones stay within the budget, least recently drawn dropped first.
#[test]
fn kept_rasters_stay_within_their_budget() {
    let mut html = String::new();
    for i in 0..40 {
        html.push_str(&format!(
            "<div class=\"c\" style=\"left: {}px; top: {}px\"></div>",
            (i % 5) * 420,
            (i / 5) * 320
        ));
    }
    let mut doc = RinchDocument::new();
    doc.load_css(
        "html, body { margin: 0; } .c { position: absolute; width: 400px; height: 300px; \
         background: #48f; filter: blur(10px); }",
    );
    let body = doc.body();
    doc.set_inner_html(body, &html);
    doc.resolve_layout(2100.0, 2600.0);
    doc.tree.consume_paint_dirty();
    let mut p = TinySkiaPainter::new(4200, 5200);
    let mut lcx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut p,
        2.0,
        (2100.0, 2600.0),
        &mut doc.font_cx,
        &mut lcx,
    );
    let held = doc.tree.raster_cache_bytes();
    assert!(
        held > 20 << 20,
        "positive control: rasters are kept ({held})"
    );
    assert!(
        held <= rinch_dom::paint::RASTER_CACHE_BUDGET_BYTES,
        "{held} bytes kept, past the budget"
    );
}

/// Blurred rows scrolled more than a window past the scroller are dropped
/// from the kept rasters.
#[test]
fn rasters_of_boxes_far_from_the_window_are_dropped() {
    let mut rows = String::new();
    for _ in 0..40 {
        rows.push_str("<div class=\"r\"></div>");
    }
    let page = format!(
        r#"<!doctype html><html><head><style>
html, body {{ margin: 0; background: #fff; }}
#s {{ position: absolute; left: 0; top: 0; width: 360px; height: 260px; overflow: hidden; }}
.r {{ height: 60px; margin-bottom: 10px; background: #48f; filter: blur(4px); }}
</style></head><body><div id="s">{rows}</div></body></html>"#
    );
    let mut doc = document(&page);
    frame(&mut doc, 1.0);
    let near = doc.tree.raster_cache_bytes();
    assert!(near > 0, "positive control: the rows on screen are kept");
    let s = by_id(&doc, "s");
    doc.set_scroll_top(s, 2000.0);
    doc.resolve_layout(W, H);
    frame(&mut doc, 1.0);
    frame(&mut doc, 1.0);
    // Only the rows near the window now: the first four are 1700px away.
    let rows_now = doc.tree.raster_cache_bytes();
    assert!(
        rows_now <= near * 3 / 2,
        "{rows_now} bytes kept after scrolling, {near} before"
    );
    let first = doc.get_children(s)[0];
    assert!(doc.tree.get(first.0).is_some());
}

// ── Freed node slots (the review's probes) ──────────────────────────────────

#[test]
fn f1_discarded_dirty_node_slot_reused() {
    let mut doc = document(PANEL);
    frame(&mut doc, 1.0);
    let panel = by_id(&doc, "panel");
    let b = doc.get_children(panel)[0];
    doc.discard_node(b);
    let body = doc.body();
    for _ in 0..12 {
        let n = doc.create_element("div");
        doc.append_child(body, n);
    }
    doc.resolve_layout(W, H);
    frame(&mut doc, 1.0);
}

/// The filtered node itself discarded and its slot reused: `raster_cached`
/// still lists the old id.
#[test]
fn f2_discarded_cached_node_slot_reused() {
    let mut doc = document(PANEL);
    frame(&mut doc, 1.0);
    doc.tree.consume_paint_dirty();
    let panel = by_id(&doc, "panel");
    doc.discard_node(panel);
    doc.tree.consume_paint_dirty();
    let body = doc.body();
    for _ in 0..12 {
        let n = doc.create_element("div");
        doc.append_child(body, n);
    }
    doc.resolve_layout(W, H);
    frame(&mut doc, 1.0);
}

/// A filtered box removed (kept, not freed), then inserted again after
/// another paint: its kept raster is no longer listed in `raster_cached`.
#[test]
fn f3_removed_then_reinserted_filtered_box() {
    let mut doc = document(PANEL);
    frame(&mut doc, 1.0);
    frame(&mut doc, 1.0);
    let panel = by_id(&doc, "panel");
    let wrap = by_id(&doc, "wrap");
    doc.remove_node(panel);
    doc.resolve_layout(W, H);
    frame(&mut doc, 1.0);
    // While detached, a child inside changes.
    let b = doc.get_children(panel)[0];
    doc.set_attribute(b, "style", "background: #0cf; left: 31px");
    doc.append_child(wrap, panel);
    doc.resolve_layout(W, H);
    let got = frame(&mut doc, 1.0);
    let mut fresh = document(PANEL);
    recolour_child(&mut fresh);
    fresh.resolve_layout(W, H);
    let want = frame(&mut fresh, 1.0);
    let n = diff(&got, &want);
    assert_eq!(n, 0);
}

/// Registering fonts drops every kept raster (`note_fonts_registered`, which
/// a system-font refresh calls too): text in it may now shape with another
/// face, and not every path pushes it paint-dirty.
#[test]
fn registering_fonts_drops_every_kept_raster() {
    let mut doc = document(PANEL);
    frame(&mut doc, 1.0);
    assert!(doc.tree.raster_cache_bytes() > 0, "positive control: kept");
    doc.note_fonts_registered();
    assert_eq!(doc.tree.raster_cache_bytes(), 0);
}
