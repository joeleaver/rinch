//! #468, review round 3 of PR #1143 — the rasterised-tile cache and the
//! pattern path.
//!
//! - The cache key: a theme toggle through a variable, a `.dark` class and
//!   `currentColor` stops all re-rasterise (stops are hashed as resolved
//!   colours); the gradient's **angle** is in it
//!   (`a_changed_angle_is_a_new_tile`); a reloaded image at the same url and
//!   size is a new tile (`a_reloaded_image_…`: the key was the data pointer,
//!   which the allocator reused — the old pixels came back).
//! - `angled_gradient_under_scale_x_keeps_its_css_slope`: the tile is laid out
//!   in CSS space and stretched to the raster, so `scaleX(2)` doubles the
//!   slope's run instead of keeping the device-space angle.
//! - `a_covering_image_is_drawn_not_rasterised`: `background-size: cover` is
//!   one clipped image draw, no raster (it was resampled into a device-size
//!   copy per cache miss, 172 ms/frame for four DPR 1.5 heroes).
//! - `a_fractional_tile_is_sampled_smoothly`: a 12.5-device-px tile blends at
//!   its edge (bilinear), and never with the page.
//! - `probe_*` / `timing_*`: prints, `#[ignore]`d.
#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use rinch_dom::perf::Counter;

fn paint_at(doc: &mut RinchDocument, scale: f64, vw: f32, vh: f32) -> TinySkiaPainter {
    let mut painter = TinySkiaPainter::new((vw as f64 * scale) as u32, (vh as f64 * scale) as u32);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale,
        (vw, vh),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter
}
fn px(p: &TinySkiaPainter, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * p.width() + x) * 4) as usize;
    let d = p.pixels();
    [d[i], d[i + 1], d[i + 2], d[i + 3]]
}
fn mount(css: &str, style: &str, vw: f32, vh: f32) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(&format!(
        "html, body {{ margin: 0; background: rgb(255, 255, 255); }} {css}"
    ));
    let body = doc.body();
    let node = doc.create_element("div");
    doc.set_attribute(
        node,
        "style",
        &format!("position: absolute; left: 0; top: 0; {style}"),
    );
    doc.append_child(body, node);
    doc.resolve_layout(vw, vh);
    (doc, node)
}

/// A theme toggle through a custom property: the tile must re-rasterise.
#[test]
fn a_theme_toggle_through_a_variable_repaints_the_tile() {
    let base = "width:100px;height:40px;background-image:linear-gradient(to right, var(--c) 0 50%, rgb(1, 2, 3) 50% 100%);background-size:10px 10px";
    let (mut doc, node) = mount("", &format!("--c: rgb(250, 0, 0); {base}"), 200.0, 100.0);
    let p = paint_at(&mut doc, 1.0, 200.0, 100.0);
    assert_eq!(px(&p, 12, 5), [250, 0, 0, 255]);
    doc.set_attribute(
        node,
        "style",
        &format!("position: absolute; left: 0; top: 0; --c: rgb(0, 0, 250); {base}"),
    );
    doc.resolve_layout(201.0, 100.0);
    let p = paint_at(&mut doc, 1.0, 201.0, 100.0);
    assert_eq!(
        px(&p, 12, 5),
        [0, 0, 250, 255],
        "stale tile after a theme toggle"
    );
    // and back: the old entry answers (cache hit), with the old pixels.
    doc.set_attribute(
        node,
        "style",
        &format!("position: absolute; left: 0; top: 0; --c: rgb(250, 0, 0); {base}"),
    );
    doc.resolve_layout(200.0, 100.0);
    let hits = doc.tree.perf.get(Counter::BackgroundTileCacheHits);
    let p = paint_at(&mut doc, 1.0, 200.0, 100.0);
    assert_eq!(px(&p, 12, 5), [250, 0, 0, 255]);
    assert!(doc.tree.perf.get(Counter::BackgroundTileCacheHits) > hits);
}

/// currentColor in a stop follows `color`.
#[test]
fn current_color_stops_follow_color() {
    let base = "width:100px;height:40px;background-image:linear-gradient(to right, currentColor 0 50%, rgb(1, 2, 3) 50% 100%);background-size:10px 10px";
    let (mut doc, node) = mount("", &format!("color: rgb(240, 0, 0); {base}"), 200.0, 100.0);
    let p = paint_at(&mut doc, 1.0, 200.0, 100.0);
    assert_eq!(px(&p, 12, 5), [240, 0, 0, 255]);
    doc.set_attribute(
        node,
        "style",
        &format!("position: absolute; left: 0; top: 0; color: rgb(0, 240, 0); {base}"),
    );
    doc.resolve_layout(201.0, 100.0);
    let p = paint_at(&mut doc, 1.0, 201.0, 100.0);
    assert_eq!(px(&p, 12, 5), [0, 240, 0, 255]);
}

/// A class toggle from a stylesheet (dark-mode shape).
#[test]
fn a_dark_class_toggle_repaints_the_tile() {
    let css = ":root { --c: rgb(230, 0, 0); } .dark { --c: rgb(0, 0, 230); } .t { width:100px;height:40px;background-image:linear-gradient(to right, var(--c) 0 50%, rgb(1, 2, 3) 50% 100%);background-size:10px 10px; }";
    let (mut doc, node) = mount(css, "", 200.0, 100.0);
    doc.set_attribute(node, "class", "t");
    doc.resolve_layout(200.0, 100.0);
    let p = paint_at(&mut doc, 1.0, 200.0, 100.0);
    assert_eq!(px(&p, 12, 5), [230, 0, 0, 255]);
    doc.set_attribute(doc.body(), "class", "dark");
    doc.resolve_layout(201.0, 100.0);
    let p = paint_at(&mut doc, 1.0, 201.0, 100.0);
    assert_eq!(px(&p, 12, 5), [0, 0, 230, 255]);
}

fn solid(w: u32, h: u32, c: [u8; 4]) -> rinch_dom::image_cache::DecodedImage {
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for _ in 0..w * h {
        data.extend_from_slice(&c);
    }
    rinch_dom::image_cache::DecodedImage::new(data, w, h)
}

/// The same url decoded again (a document on the same thread, or a reload)
/// with the same size: the key's data pointer can be reused by the allocator.
#[test]
fn a_reloaded_image_with_the_same_url_and_size_repaints() {
    let url = "test://r3-reload.png";
    let (mut doc, node) = mount(
        "",
        "width:100px;height:40px;background-size:10px 10px",
        200.0,
        100.0,
    );
    doc.tree
        .image_cache
        .insert_decoded(url.into(), solid(4, 4, [200, 0, 0, 255]));
    doc.set_attribute(node, "style", &format!("position: absolute; left: 0; top: 0; width:100px;height:40px;background-size:10px 10px;background-image:url({url})"));
    doc.resolve_layout(201.0, 100.0);
    let p = paint_at(&mut doc, 1.0, 201.0, 100.0);
    assert_eq!(px(&p, 12, 5), [200, 0, 0, 255]);
    // Free the old pixels before the new ones exist, as a drop + reload does.
    doc.tree.image_cache.mark_loading(url.into());
    doc.tree
        .image_cache
        .insert_decoded(url.into(), solid(4, 4, [0, 200, 0, 255]));
    doc.set_attribute(node, "data-x", "1");
    doc.resolve_layout(202.0, 100.0);
    let p = paint_at(&mut doc, 1.0, 202.0, 100.0);
    eprintln!("reload pixel {:?}", px(&p, 12, 5));
    assert_eq!(
        px(&p, 12, 5),
        [0, 200, 0, 255],
        "stale image tile (pointer ABA)"
    );
}

/// Bilinear sampling of a fractional tile wraps: the pixel after a tile's
/// last (blue) column blends into the next tile's first (red), never with
/// transparent (white page) — at DPR 1.25 and 1.5.
#[test]
#[ignore = "probe: prints the rows"]
fn probe_bilinear_wrap_at_fractional_dpr() {
    for scale in [1.25f64, 1.5] {
        let (mut doc, _) = mount(
            "",
            "width:100px;height:20px;background-image:linear-gradient(to right, rgb(255,0,0) 0 50%, rgb(0,0,255) 50% 100%);background-size:10px 20px",
            200.0,
            100.0,
        );
        let p = paint_at(&mut doc, scale, 200.0, 100.0);
        let y = (10.0 * scale) as u32;
        let row: Vec<String> = (0..40)
            .map(|x| {
                let c = px(&p, x, y);
                format!("{},{},{}", c[0], c[1], c[2])
            })
            .collect();
        eprintln!("dpr {scale}: {}", row.join(" | "));
        // no pixel may be light (a white/transparent bleed): green 0 everywhere
        for x in 0..(100.0 * scale) as u32 - 1 {
            let c = px(&p, x, y);
            assert!(c[1] < 40, "dpr {scale} x {x}: {c:?} bleeds the page");
        }
    }
}

/// Eviction: three 12 MB tiles alternating clear the whole cache every
/// time, so every paint re-rasterises every one of them.
#[test]
#[ignore = "probe: clear-all eviction thrash, a follow-up"]
fn probe_eviction_thrash() {
    // 1600x1600 device px tile at scale 2 => 800px css tile, 10.24 MB each
    let mut total_rasters = 0;
    for round in 0..3 {
        for k in 0..4 {
            let (mut doc, _) = mount(
                "",
                &format!(
                    "width:1700px;height:10px;background-image:linear-gradient(to right, rgb({k},9,9) 0 50%, rgb(0,0,{}) 50% 100%);background-size:800px 800px",
                    200 + round * 0
                ),
                1800.0,
                20.0,
            );
            let _ = paint_at(&mut doc, 2.0, 1800.0, 20.0);
            total_rasters += doc.tree.perf.get(Counter::BackgroundTileRasters);
        }
    }
    eprintln!("rasters over 12 paints of 4 distinct 10MB tiles: {total_rasters}");
}

/// Timing: a 3.9M-px tile rasterised per frame when the cache is thrashed;
/// and the cold raster itself.
#[test]
#[ignore]
fn timing_cold_raster_4m() {
    let t = std::time::Instant::now();
    let (mut doc, _) = mount(
        "",
        "width:1990px;height:1990px;background-image:radial-gradient(rgb(1,2,3), rgb(9,8,7));background-size:990px 990px;background-repeat:repeat",
        2000.0,
        2000.0,
    );
    let _ = paint_at(&mut doc, 2.0, 2000.0, 2000.0);
    eprintln!(
        "cold 4M tile paint: {:?} rasters={}",
        t.elapsed(),
        doc.tree.perf.get(Counter::BackgroundTileRasters)
    );
}

fn first_blue(p: &TinySkiaPainter, y: u32, from: u32) -> u32 {
    (from..p.width())
        .find(|&x| {
            let c = px(p, x, y);
            c[2] > 128 && c[0] < 128
        })
        .unwrap_or(9999)
}

/// Non-uniform scale: an angled gradient tile under `scaleX(2)`. CSS draws the
/// gradient in tile space and stretches it: the 45deg boundary shifts 2 device
/// px per device row. A raster drawn at device size with the angle applied in
/// device space shifts 1 per row.
#[test]
fn angled_gradient_under_scale_x_keeps_its_css_slope() {
    let (mut doc, _) = mount(
        "",
        "width:100px;height:100px;transform-origin:0 0;transform:scaleX(2);background-image:linear-gradient(45deg, rgb(255,0,0) 0 50%, rgb(0,0,255) 50% 100%);background-size:20px 20px",
        300.0,
        200.0,
    );
    let p = paint_at(&mut doc, 1.0, 300.0, 200.0);
    let a = first_blue(&p, 4, 0);
    let b = first_blue(&p, 8, 0);
    eprintln!("row 4 first blue {a}, row 8 {b}");
    // 45deg: value grows with x and falls with y; the red/blue boundary moves
    // right as y grows: by (8-4) tile px = 4 in tile space = 8 device px.
    assert_eq!(
        (b as i64 - a as i64).abs(),
        8,
        "slope distorted by the device-space raster"
    );
}

/// A scale()'d box inside a scale()'d ancestor: the period is 10*2*1.5 = 30.
#[test]
fn nested_scale_period() {
    let mut doc = RinchDocument::new();
    doc.load_css("html, body { margin: 0; background: rgb(255,255,255); }");
    let body = doc.body();
    let outer = doc.create_element("div");
    doc.set_attribute(outer, "style", "position:absolute;left:0;top:0;width:200px;height:50px;transform-origin:0 0;transform:scale(2)");
    let inner = doc.create_element("div");
    doc.set_attribute(inner, "style", "width:100px;height:20px;transform-origin:0 0;transform:scale(1.5);background-image:linear-gradient(to right, rgb(255,0,0) 0 50%, rgb(0,0,255) 50% 100%);background-size:10px 20px");
    doc.append_child(outer, inner);
    doc.append_child(body, outer);
    doc.resolve_layout(700.0, 200.0);
    let p = paint_at(&mut doc, 1.0, 700.0, 200.0);
    let row: String = (0..95)
        .map(|x| {
            let c = px(&p, x, 10);
            if c[0] > 200 && c[2] < 50 {
                'R'
            } else if c[2] > 200 && c[0] < 50 {
                'B'
            } else {
                '?'
            }
        })
        .collect();
    eprintln!("{row}");
    assert_eq!(
        &row[..90],
        &"R".repeat(15)
            .to_string()
            .chars()
            .chain("B".repeat(15).chars())
            .collect::<String>()
            .repeat(3)
    );
}

/// Three `background-size: cover` hero images at DPR 1.5, each ~11 MB of
/// raster: together over the 32 MB budget, so the clear-all eviction makes
/// every full repaint re-rasterise every one.
#[test]
#[ignore]
fn timing_cover_heroes_thrash() {
    let mut doc = RinchDocument::new();
    doc.load_css("html, body { margin: 0; }");
    let body = doc.body();
    for i in 0..4 {
        let url = format!("test://hero{i}.png");
        doc.tree
            .image_cache
            .insert_decoded(url.clone(), solid(1600, 1000, [10 * i as u8, 90, 200, 255]));
        let n = doc.create_element("div");
        doc.set_attribute(
            n,
            "style",
            &format!("width:1400px;height:780px;background-image:url({url});background-size:cover"),
        );
        doc.append_child(body, n);
    }
    doc.resolve_layout(1400.0, 3200.0);
    for f in 0..4 {
        let r0 = doc.tree.perf.get(Counter::BackgroundTileRasters);
        let t = std::time::Instant::now();
        let _ = paint_at(&mut doc, 1.5, 1400.0, 3200.0);
        eprintln!(
            "frame {f}: {:?}, rasters this frame {}",
            t.elapsed(),
            doc.tree.perf.get(Counter::BackgroundTileRasters) - r0
        );
    }
}

/// The gradient's angle is part of what decides the raster: `to right` then
/// `to left`, same stops and size, must not reuse the first tile.
#[test]
fn a_changed_angle_is_a_new_tile() {
    let grad = |dir: &str| {
        format!(
            "width:100px;height:20px;background-image:linear-gradient({dir}, rgb(201, 0, 0) 0 50%, rgb(0, 0, 201) 50% 100%);background-size:10px 20px"
        )
    };
    let (mut doc, node) = mount("", &grad("to right"), 200.0, 100.0);
    let p = paint_at(&mut doc, 1.0, 200.0, 100.0);
    assert_eq!(px(&p, 12, 10), [201, 0, 0, 255], "to right: red first");
    doc.set_attribute(
        node,
        "style",
        &format!("position: absolute; left: 0; top: 0; {}", grad("to left")),
    );
    doc.resolve_layout(201.0, 100.0);
    let p = paint_at(&mut doc, 1.0, 201.0, 100.0);
    assert_eq!(px(&p, 12, 10), [0, 0, 201, 255], "to left: blue first");
}

/// A 10px tile at DPR 1.25 is 12.5 device px, rasterised at 13 and sampled
/// back down bilinearly: the device pixel holding the tile boundary (x = 12,
/// covering 12.0..13.0 around the edge at 12.5) is a red/blue blend, as in
/// Chrome, and not a hard edge — and no pixel takes the page's white.
#[test]
fn a_fractional_tile_is_sampled_smoothly() {
    let (mut doc, _) = mount(
        "",
        "width:100px;height:20px;background-image:linear-gradient(to right, rgb(255,0,0) 0 50%, rgb(0,0,255) 50% 100%);background-size:10px 20px",
        200.0,
        100.0,
    );
    let p = paint_at(&mut doc, 1.25, 200.0, 100.0);
    let c = px(&p, 12, 12);
    assert!(
        (60..=200).contains(&c[0]) && (60..=200).contains(&c[2]) && c[1] < 40,
        "the tile boundary pixel is a red/blue blend: {c:?}"
    );
    for x in 0..124 {
        assert!(px(&p, x, 12)[1] < 40, "x {x}: the page bleeds in");
    }
}

/// `background-size: cover` on an image whose aspect differs from the box:
/// one tile covers the box without being the box. Drawn as one clipped image
/// draw, never rasterised into a pattern tile.
#[test]
fn a_covering_image_is_drawn_not_rasterised() {
    let mut doc = RinchDocument::new();
    doc.load_css("html, body { margin: 0; background: rgb(255, 255, 255); }");
    doc.tree
        .image_cache
        .insert_decoded("test://cover.png".into(), solid(300, 100, [0, 150, 0, 255]));
    let body = doc.body();
    let n = doc.create_element("div");
    doc.set_attribute(
        n,
        "style",
        "position:absolute;left:0;top:0;width:200px;height:100px;background-image:url(test://cover.png);background-size:cover",
    );
    doc.append_child(body, n);
    doc.resolve_layout(300.0, 200.0);
    let p = paint_at(&mut doc, 1.5, 300.0, 200.0);
    assert_eq!(doc.tree.perf.get(Counter::BackgroundTileRasters), 0);
    assert_eq!(doc.tree.perf.get(Counter::BackgroundTileCacheHits), 0);
    assert_eq!(px(&p, 150, 75), [0, 150, 0, 255], "the image is drawn");
    // The tile is 300x100 CSS px and the box 200 wide: device x 330 (CSS 220)
    // is on the tile and off the box.
    assert_eq!(
        px(&p, 330, 75),
        [255, 255, 255, 255],
        "and clipped to the box"
    );
}
