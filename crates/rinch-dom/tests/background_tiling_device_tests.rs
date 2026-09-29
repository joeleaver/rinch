//! #468, review round 2 of PR #1143 — tiling on the device grid.
//!
//! - `a_dot_grid_at_dpr_1_25_is_drawn` (N1): a 10px dot grid on 800x600 at
//!   DPR 1.25 is 12.5-device-pixel tiles, 4800 of them. Round 2 drew
//!   nothing (tile by tile past a 4096 cap); Chrome draws the grid. Now a
//!   pattern whose tile is rasterised at 13px and sampled down.
//! - `no_seam_under_a_fractional_translate` (N2): tile edges were snapped in
//!   shape space, so `translateX(10.5px)` put them back mid-pixel — the 75%
//!   seam. Chrome 153: pure red at every interior column.
//! - `vello_brush_matches_the_pattern_path`, `k6`, `k7` (N6): the Vello brush
//!   is one Repeat/nearest image draw of the device-size tile; a `scale(3)`
//!   box rasterises its tile at 30px; a 12.5px tile keeps its period
//!   (Chrome: tile 9 starts at device x = 100).
//! - `a_repaint_of_an_unchanged_layer_does_not_re_rasterise` (N4): counters.
//! - `probe_*`: print only (rows against Chrome's, quoted in the review).
#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const RB: &str =
    "background-image: linear-gradient(to right, rgb(255, 0, 0) 0 50%, rgb(0, 0, 255) 50% 100%)";

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
fn mount(style: &str, vw: f32, vh: f32) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css("html, body { margin: 0; background: rgb(255, 255, 255); }");
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
fn row(style: &str, scale: f64) -> String {
    let (mut doc, _) = mount(style, 200.0, 100.0);
    let p = paint_at(&mut doc, scale, 200.0, 100.0);
    let y = (10.0 * scale) as u32;
    (0..70)
        .map(|x| match px(&p, x, y) {
            [255, 0, 0, 255] => 'R',
            [0, 0, 255, 255] => 'B',
            [255, 255, 255, 255] => 'W',
            _ => '?',
        })
        .collect()
}

#[test]
fn probe_rows() {
    for (name, st, s) in [
        (
            "a_repx",
            "background-size:10px 20px;background-repeat:repeat-x",
            1.25,
        ),
        (
            "a_repx",
            "background-size:10px 20px;background-repeat:repeat-x",
            1.5,
        ),
        ("b_rep", "background-size:10px 20px", 1.25),
        ("b_rep", "background-size:10px 20px", 1.5),
        (
            "c_p03",
            "background-size:16px 20px;background-position:0.3px 0",
            1.0,
        ),
        (
            "d_p05",
            "background-size:16px 20px;background-position:0.5px 0",
            1.0,
        ),
        (
            "e_p07",
            "background-size:16px 20px;background-position:0.7px 0",
            1.0,
        ),
        (
            "c_p03",
            "background-size:16px 20px;background-position:0.3px 0",
            1.25,
        ),
        (
            "e_p07",
            "background-size:16px 20px;background-position:0.7px 0",
            1.5,
        ),
        (
            "f_p05rx",
            "background-size:16px 20px;background-position:0.5px 0;background-repeat:repeat-x",
            1.0,
        ),
    ] {
        eprintln!(
            "{name}_{s}: {}",
            row(&format!("width:100px;height:20px;{RB};{st}"), s)
        );
    }
}

/// The Vello path: the same document emits ONE image draw whose sampler is
/// Repeat/Low (nearest) on both axes, whose image is the tile at device size,
/// and whose brush transform places tile (0,0) where tiny-skia's pattern does.
#[test]
fn vello_brush_matches_the_pattern_path() {
    use rinch_dom::paint::vello_painter::VelloPainter;
    let scale = 1.5;
    let (mut doc, _) = mount(
        &format!(
            "width:600px;height:600px;{RB};background-size:8px 8px;background-position:3px 5px;transform:translate(10px, 20px)"
        ),
        700.0,
        700.0,
    );
    let mut vp = VelloPainter::new();
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut vp,
        scale,
        (700.0, 700.0),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    let enc = vp.scene().encoding();
    let n_img = enc.draw_tags.iter().filter(|t| t.0 == 0x28C).count();
    eprintln!("patches: {}", enc.resources.patches.len());
    eprintln!(
        "transforms: {:?}",
        enc.transforms
            .iter()
            .map(|t| (t.matrix, t.translation))
            .collect::<Vec<_>>()
    );
    let dd = &enc.draw_data;
    let wh = (12u32 << 16) | 12;
    let i = dd
        .iter()
        .position(|&v| v == wh)
        .expect("a 12x12 image draw");
    let sa = dd[i + 1];
    eprintln!(
        "sample_alpha = {sa:#b}: quality {}, x_extend {}, y_extend {}, alpha {}",
        (sa >> 12) & 3,
        (sa >> 10) & 3,
        (sa >> 8) & 3,
        sa & 0xff
    );
    assert_eq!((sa >> 10) & 3, 1, "x Repeat");
    assert_eq!((sa >> 8) & 3, 1, "y Repeat");
    assert_eq!(
        (sa >> 12) & 3,
        0,
        "quality Low (nearest), as tiny-skia's Nearest"
    );
    assert_eq!(n_img, 1, "one image draw for 5625 tiles");
}

#[test]
fn probe_fractional_translate() {
    for (st, s) in [
        ("background-size:16px 20px;transform:translateX(10px)", 1.25),
        (
            "background-size:16px 20px;transform:translateX(10.3px)",
            1.0,
        ),
        (
            "background-size:16px 20px;transform:translateX(10.3px);background-repeat:repeat-x",
            1.0,
        ),
        ("background-size:16px 20px;left:10.3px", 1.0),
    ] {
        eprintln!(
            "{st} @{s}: {}",
            row(&format!("width:100px;height:20px;{RB};{st}"), s)
        );
    }
}

#[test]
fn probe_seam_values() {
    let (mut doc, _) = mount(
        &format!(
            "width:100px;height:20px;{RB};background-size:16px 20px;transform:translateX(10.3px);background-repeat:repeat-x"
        ),
        200.0,
        100.0,
    );
    let p = paint_at(&mut doc, 1.0, 200.0, 100.0);
    eprintln!(
        "seam px: {:?}",
        (24..28).map(|x| px(&p, x, 10)).collect::<Vec<_>>()
    );
}

/// N2: solid red 10px tiles, `repeat-x`, under `translateX(10.5px)`.
/// Chrome 153: pure red at every interior column. Before the device-space
/// snap: `(255, 64, 64)` at x = 20, 30, …, 90.
#[test]
fn no_seam_under_a_fractional_translate() {
    let solid = "background-image: linear-gradient(rgb(255, 0, 0), rgb(255, 0, 0))";
    for repeat in ["repeat-x", "repeat"] {
        let (mut doc, _) = mount(
            &format!(
                "width:100px;height:20px;{solid};background-size:10px 20px;transform:translateX(10.5px);background-repeat:{repeat}"
            ),
            200.0,
            100.0,
        );
        let p = paint_at(&mut doc, 1.0, 200.0, 100.0);
        let bad: Vec<(u32, [u8; 4])> = (12..100)
            .map(|x| (x, px(&p, x, 10)))
            .filter(|(_, c)| *c != [255, 0, 0, 255])
            .collect();
        assert!(bad.is_empty(), "{repeat}: seams {bad:?}");
    }
}

/// N1: a 10px dot grid (a design-canvas background) on an 800x600 box. At DPR
/// 1.25 a tile is 12.5 device px and there are 4800 of them; round 2 drew
/// nothing there. Measured at DPR 1
/// and 2 as 19200 and 57600 dark px (0.04 of the box's device area), so DPR
/// 1.25 must land near 0.04 * 1000 * 750 = 30000.
#[test]
fn a_dot_grid_at_dpr_1_25_is_drawn() {
    for (s, lo, hi) in [
        (1.0, 19200, 19200),
        (1.25, 20000, 40000),
        (2.0, 57600, 57600),
    ] {
        let (mut doc, _) = mount(
            "width:800px;height:600px;background-color:rgb(250,250,250);background-image:radial-gradient(rgb(0,0,0) 20%, transparent 20%);background-size:10px 10px",
            900.0,
            700.0,
        );
        let p = paint_at(&mut doc, s, 900.0, 700.0);
        let dark = p.pixels().chunks(4).filter(|c| c[0] < 128).count();
        assert!(
            (lo..=hi).contains(&dark),
            "DPR {s}: {dark} dark px, want {lo}..={hi}"
        );
        // One pattern, not 4800 fills: the fractional tile took the pattern path.
        use rinch_dom::perf::Counter;
        let patterns = doc.tree.perf.get(Counter::BackgroundTileRasters)
            + doc.tree.perf.get(Counter::BackgroundTileCacheHits);
        assert_eq!(patterns, 1, "DPR {s}: the layer is one pattern");
    }
}

/// N4: a second paint of an unchanged tiled layer takes its tile from the
/// cache; a changed tile size rasterises again.
#[test]
fn a_repaint_of_an_unchanged_layer_does_not_re_rasterise() {
    use rinch_dom::perf::Counter;
    // Colours no other fixture uses, so the thread's cache starts cold for it.
    let grad = "background-image: linear-gradient(to right, rgb(7, 11, 13) 0 50%, rgb(17, 19, 23) 50% 100%)";
    let (mut doc, node) = mount(
        &format!("width:100px;height:20px;{grad};background-size:10px 20px"),
        200.0,
        100.0,
    );
    let _ = paint_at(&mut doc, 1.0, 200.0, 100.0);
    assert_eq!(doc.tree.perf.get(Counter::BackgroundTileRasters), 1);
    assert_eq!(doc.tree.perf.get(Counter::BackgroundTileCacheHits), 0);
    let _ = paint_at(&mut doc, 1.0, 200.0, 100.0);
    assert_eq!(
        doc.tree.perf.get(Counter::BackgroundTileRasters),
        1,
        "unchanged: no raster"
    );
    assert_eq!(doc.tree.perf.get(Counter::BackgroundTileCacheHits), 1);
    doc.set_style(node, "background-size", "12px 20px");
    doc.resolve_layout(201.0, 100.0);
    let _ = paint_at(&mut doc, 1.0, 200.0, 100.0);
    assert_eq!(
        doc.tree.perf.get(Counter::BackgroundTileRasters),
        2,
        "a new tile size rasterises"
    );
}

/// Kills "pattern path on a scaled box" (M6): a smooth 10px ramp under
/// scale(3) must be rasterised at 30 device px per period, not 10 upsampled.
#[test]
fn k6_scaled_box_tile_is_rasterised_at_scaled_resolution() {
    let (mut doc, _) = mount(
        "width:60px;height:20px;transform-origin:0 0;transform:scale(3);background-image:linear-gradient(to right, rgb(0,0,0), rgb(255,255,255));background-size:10px 10px",
        300.0,
        100.0,
    );
    let p = paint_at(&mut doc, 1.0, 300.0, 100.0);
    let mut v: Vec<u8> = (30..60).map(|x| px(&p, x, 30)[0]).collect();
    v.dedup();
    assert!(
        v.len() > 20,
        "a smooth 30px ramp, got {} distinct steps: {v:?}",
        v.len()
    );
}

/// Kills "pattern path for a fractional tile" (M7): 10px at DPR 1.25 is 12.5
/// device px, so the 9th tile starts at device x = 100, not 8*13 = 104.
#[test]
fn k7_fractional_tile_keeps_its_period() {
    let (mut doc, _) = mount(
        &format!("width:200px;height:20px;{RB};background-size:10px 10px"),
        300.0,
        100.0,
    );
    let p = paint_at(&mut doc, 1.25, 300.0, 100.0);
    assert_eq!(px(&p, 99, 5), [0, 0, 255, 255], "end of tile 8");
    assert_eq!(px(&p, 101, 5), [255, 0, 0, 255], "start of tile 9");
}

/// `repeat-x` / `repeat-y` on the pattern path: the pattern is clipped to the
/// one-tile strip on the axis that does not repeat. Chrome 153 (measured), a
/// 10x20 tile at `3px 7px` in a 100x40 box:
/// `repeat-x` column x=5: W 0-7, R 7-27, W 27-40; row y=15 repeats (B 0-3, R 3-8, …).
/// `repeat-y` row y=15: W 0-3, R 3-8, B 8-13, W 13-40; column x=5 all R.
#[test]
fn a_one_axis_repeat_is_clipped_to_its_strip() {
    let cls = |c: [u8; 4]| match c {
        [255, 0, 0, 255] => 'R',
        [0, 0, 255, 255] => 'B',
        [255, 255, 255, 255] => 'W',
        _ => '?',
    };
    let paint = |repeat: &str| {
        let (mut doc, _) = mount(
            &format!(
                "width:100px;height:40px;{RB};background-size:10px 20px;background-position:3px 7px;background-repeat:{repeat}"
            ),
            200.0,
            100.0,
        );
        paint_at(&mut doc, 1.0, 200.0, 100.0)
    };
    let p = paint("repeat-x");
    let col: String = [3, 10, 20, 30, 38]
        .iter()
        .map(|&y| cls(px(&p, 5, y)))
        .collect();
    assert_eq!(col, "WRRWW", "repeat-x column");
    let row: String = [1, 5, 10, 15, 35]
        .iter()
        .map(|&x| cls(px(&p, x, 15)))
        .collect();
    assert_eq!(row, "BRBRR", "repeat-x row");
    let p = paint("repeat-y");
    let row: String = [1, 5, 10, 20, 35]
        .iter()
        .map(|&x| cls(px(&p, x, 15)))
        .collect();
    assert_eq!(row, "WRBWW", "repeat-y row");
    let col: String = [1, 10, 25, 39].iter().map(|&y| cls(px(&p, 5, y))).collect();
    assert_eq!(col, "RRRR", "repeat-y column");
}
