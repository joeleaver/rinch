//! #2368: at a fractional scale factor a box is drawn on whole device pixels.
//!
//! Chrome rounds every edge of a box's border box — and the clip rect derived
//! from it — to the nearest device pixel (`ToPixelSnappedRect`), so at 125% a
//! box at `left: 13px` (device x 16.25) has a hard edge at 16. rinch drew the
//! fractional edge, a one-pixel blend: 63 where Chrome has 0, and a clip's
//! blended edge multiplied by its child's (143 against 255, #2321).
//!
//! The oracle is Chrome 158 itself (`--headless=new --disable-gpu
//! --force-device-scale-factor=1.25` and `1.5`, this scene as HTML), read at
//! the same device pixels: a row of pixels across each box's left and right
//! edges and a column across its top and bottom, for twelve boxes — a
//! background; a 1px and a 3px border; an `overflow: hidden` clip, square,
//! rounded, inside a 3px border, and nested in another clip; a box inside a
//! padded box (snapped from its own accumulated position, 17.5 → 18, not from
//! its parent's snapped one, 16 + 1.25 → 17); `translateX(0.4px)` on the box
//! and on its parent (snapped first, then moved half a pixel, so the edge
//! blends as Chrome's does: a transform is not snapped); `scale(1.1)`; and an
//! `opacity: 0.5` box (whose group layer must hold the snapped box).
//!
//! Off the fixed point: every box is at 13px, which is 16.25 device px at
//! 1.25x and a tie (19.5) at 1.5x, and the parent and child cases put the two
//! on different fractions. At 1x every edge is already whole, so nothing here
//! would tell snapping from not snapping there. No text, so no fonts.

#![cfg(feature = "software-renderer")]

#[path = "support/box_snap_2368.rs"]
mod scene;

use rinch_dom::paint::skia_painter::TinySkiaPainter;

/// The scene painted by the software painter at `scale`: the green channel
/// (the scene is gray), one byte a device pixel, and the row width.
fn paint(scale: f64) -> (Vec<u8>, usize) {
    let mut doc = scene::document(scale);
    let (w, h) = scene::surface(scale);
    let mut painter = TinySkiaPainter::new(w, h);
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale,
        (scene::VW, scene::VH),
        &mut doc.font_cx,
        &mut cx,
    );
    let gray = painter
        .pixels()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| p[1])
        .collect();
    (gray, w as usize)
}

fn matches_chrome(scale: f64, chrome: &[scene::Probe]) {
    let (g, w) = paint(scale);
    let wrong = scene::mismatches(&g, w, scale, chrome);
    assert!(wrong.is_empty(), "at {scale}x:\n{}", wrong.join("\n"));
}

#[test]
fn box_edges_and_clips_are_on_device_pixels_at_125_percent() {
    matches_chrome(1.25, &scene::CHROME_125);
}

#[test]
fn box_edges_and_clips_are_on_device_pixels_at_150_percent() {
    matches_chrome(1.5, &scene::CHROME_15);
}

/// A hard `box-shadow` (`4px 4px 0`, whole device pixels at both scales) on
/// a box with no background, at (13, 9) and 40x30: the shadow is the snapped
/// border box moved by its offset, so its far edges are hard, and the part
/// under the box is cut out along the snapped box. Chrome 158 at 1.25x and
/// 1.5x, read across the shadow's right edge (a row at y 30) and its bottom
/// edge (a column at x 30): drawn from the unsnapped box, the shadow's far
/// edges blend (191 at 1.25x, 127 at 1.5x) a pixel past Chrome's.
#[test]
fn a_box_shadow_is_drawn_from_the_snapped_box() {
    use rinch_core::dom::DomDocument;
    const PAGE: &str = "<div style=\"position:absolute;left:13px;top:9px;width:40px;\
                        height:30px;box-shadow:4px 4px 0 #000\"></div>";
    // (scale, first pixel of the row and column, Chrome's pixels from it).
    let chrome: [(f64, usize, &[u8]); 2] = [
        (
            1.25,
            60,
            &[
                255, 255, 255, 255, 255, 255, 0, 0, 0, 0, 0, 255, 255, 255, 255,
            ],
        ),
        (
            1.5,
            73,
            &[
                255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 0, 0, 0, 255, 255, 255, 255,
            ],
        ),
    ];
    let column_from = [43, 52];
    for ((scale, row_from, want), col_from) in chrome.into_iter().zip(column_from) {
        let (vw, vh) = (100.0_f32, 60.0_f32);
        let mut doc = rinch_dom::RinchDocument::new();
        doc.set_device_pixel_ratio(scale as f32);
        let body = doc.body();
        doc.set_attribute(body, "style", "margin:0;background:#fff");
        doc.set_inner_html(body, PAGE);
        doc.resolve_layout(vw, vh);
        let (w, h) = (
            (vw as f64 * scale).round() as u32,
            (vh as f64 * scale).round() as u32,
        );
        let mut painter = TinySkiaPainter::new(w, h);
        painter.fill_white();
        let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut painter,
            scale,
            (vw, vh),
            &mut doc.font_cx,
            &mut cx,
        );
        let px = painter.pixels();
        let at = |x: usize, y: usize| px[(y * w as usize + x) * 4 + 1];
        let line = (30.0 * scale) as usize;
        let row: Vec<u8> = (row_from..row_from + want.len())
            .map(|x| at(x, line))
            .collect();
        let col: Vec<u8> = (col_from..col_from + want.len())
            .map(|y| at(line, y))
            .collect();
        let close = |got: &[u8]| got.iter().zip(want).all(|(&a, &b)| a.abs_diff(b) <= 3);
        assert!(
            close(&row),
            "at {scale}x, across the right edge: rinch {row:?}, Chrome {want:?}"
        );
        assert!(
            close(&col),
            "at {scale}x, across the bottom edge: rinch {col:?}, Chrome {want:?}"
        );
    }
}
