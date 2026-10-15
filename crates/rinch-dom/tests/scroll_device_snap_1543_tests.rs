//! #1543: scrolled text moves rigidly, by whole device pixels.
//!
//! The software painter rasterises a glyph at the origin and places the image
//! by nearest-neighbour sampling, so each glyph's x and each line's baseline
//! round to a device pixel on their own. A scroll offset that puts the content
//! at a *fractional* device position therefore rounds every glyph differently:
//! at a fractional scale (Windows 125%, 150%) or after a fractional wheel delta
//! the spacing between glyphs, and between lines, changed from one scroll
//! position to the next — the text shimmered while scrolling.
//!
//! Chrome snaps the scrolled content to whole device pixels, so a scroll is a
//! rigid translation. The oracle here is exactly that: paint the same scroller
//! at two offsets a fraction of a device pixel apart, and require the second
//! image to be the first shifted by a whole number of device pixels along the
//! scroll axis (over the scroller's interior, away from its clipped edges).
//! Off the fixed point on purpose: two offsets a whole device pixel apart are a
//! rigid shift even with no snapping at all (`round(x + 1) == round(x) + 1`),
//! so the offsets differ by 0.4 / 0.6 device px.
//!
//! The face is the bundled Inter under an override name, and `font-size` /
//! `line-height` are declared, so nothing here measures the host's fonts —
//! and nothing asserts a glyph position, only a comparison between two
//! paints of the same text.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 300.0;
const VH: f32 = 120.0;
const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

const TEXT: &str = "Settings general appearance keyboard shortcuts privacy \
                    notifications accounts advanced about rinch software";

fn document(style: &str, text: &str, scale: f64) -> (RinchDocument, NodeId) {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1);
    // What the desktop shell does before it paints at `scale`.
    doc.set_device_pixel_ratio(scale as f32);
    let body = doc.body();
    let c = doc.create_element("div");
    doc.set_attribute(
        c,
        "style",
        &format!(
            "margin: 10px; font-family: ProbeFace; font-size: 16px; line-height: 20px; \
             color: rgb(0, 0, 0); background: rgb(255, 255, 255); {style}"
        ),
    );
    doc.append_child(body, c);
    let t = doc.create_text(text);
    doc.append_child(c, t);
    doc.resolve_layout(VW, VH);
    (doc, c)
}

/// Device pixels of the window painted at `scale` with the scroller at
/// `offset` (logical px) on `axis` (0 = x, 1 = y). Gray, one byte a pixel.
fn paint_at(style: &str, text: &str, scale: f64, axis: usize, offset: f64) -> (Vec<u8>, usize) {
    let (mut doc, c) = document(style, text, scale);
    if axis == 0 {
        doc.set_scroll_left(c, offset);
    } else {
        doc.set_scroll_top(c, offset);
    }
    doc.resolve_layout(VW, VH);
    let (w, h) = (
        (VW as f64 * scale).round() as u32,
        (VH as f64 * scale).round() as u32,
    );
    let mut painter = TinySkiaPainter::new(w, h);
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale,
        (VW, VH),
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

/// Whether `b` is `a` moved by a whole number of device pixels (−3..=3) along
/// `axis`, over the device rect `(x0, y0, x1, y1)`. Returns the shift.
fn rigid_shift(
    a: &[u8],
    b: &[u8],
    w: usize,
    axis: usize,
    (x0, y0, x1, y1): (usize, usize, usize, usize),
) -> Option<i32> {
    (-3i32..=3).find(|&k| {
        (y0..y1).all(|y| {
            (x0..x1).all(|x| {
                let (sx, sy) = if axis == 0 {
                    ((x as i32 + k) as usize, y)
                } else {
                    (x, (y as i32 + k) as usize)
                };
                b[y * w + x] == a[sy * w + sx]
            })
        })
    })
}

fn has_ink(a: &[u8], w: usize, (x0, y0, x1, y1): (usize, usize, usize, usize)) -> bool {
    (y0..y1).any(|y| (x0..x1).any(|x| a[y * w + x] < 128))
}

/// One horizontal line, scrolled sideways.
fn horizontal(scale: f64, from: f64, to: f64) {
    let style = "width: 200px; height: 20px; white-space: nowrap; overflow-x: auto; \
                 overflow-y: hidden; --rinch-scrollbar-width: none";
    let (a, w) = paint_at(style, TEXT, scale, 0, from);
    let (b, _) = paint_at(style, TEXT, scale, 0, to);
    // The scroller is 10..210 x 10..30 logical; keep 8px clear of its edges.
    let s = |v: f64| (v * scale).round() as usize;
    let rect = (s(18.0), s(10.0), s(202.0), s(30.0));
    assert!(
        has_ink(&a, w, rect),
        "nothing painted; the fixture measures nothing"
    );
    assert!(
        rigid_shift(&a, &b, w, 0, rect).is_some(),
        "at scale {scale}, scrolling from {from} to {to} logical px did not move the \
         line rigidly: glyph spacing changed (#1543)"
    );
}

/// Several lines, scrolled down.
fn vertical(scale: f64, from: f64, to: f64) {
    // A 17px line is 21.25 / 25.5 device px, so the lines' baselines sit at
    // different fractions of a device pixel; at 20px (25 / 30 device px) every
    // line shares one fraction and the stack is rigid with no snapping.
    let style = "width: 200px; height: 80px; overflow-y: auto; overflow-x: hidden; \
                 line-height: 17px; --rinch-scrollbar-width: none";
    let text = format!("{TEXT} {TEXT} {TEXT} {TEXT}");
    let (a, w) = paint_at(style, &text, scale, 1, from);
    let (b, _) = paint_at(style, &text, scale, 1, to);
    let s = |v: f64| (v * scale).round() as usize;
    let rect = (s(10.0), s(18.0), s(210.0), s(82.0));
    assert!(
        has_ink(&a, w, rect),
        "nothing painted; the fixture measures nothing"
    );
    assert!(
        rigid_shift(&a, &b, w, 1, rect).is_some(),
        "at scale {scale}, scrolling from {from} to {to} logical px did not move the \
         lines rigidly: line spacing changed (#1543)"
    );
}

#[test]
fn a_horizontal_scroll_at_125_percent_moves_the_line_rigidly() {
    // 12.5 and 12.9 device px.
    horizontal(1.25, 10.0, 10.32);
    // 12.5 and 13.4: on the grid, a whole device pixel apart.
    horizontal(1.25, 10.0, 10.72);
}

#[test]
fn a_horizontal_scroll_at_150_percent_moves_the_line_rigidly() {
    // 15.0 and 15.6 device px.
    horizontal(1.5, 10.0, 10.4);
}

#[test]
fn a_fractional_horizontal_scroll_at_100_percent_moves_the_line_rigidly() {
    // A fractional wheel delta (Wayland reports them) at scale 1.
    horizontal(1.0, 10.0, 10.4);
}

#[test]
fn a_vertical_scroll_at_125_percent_moves_the_lines_rigidly() {
    vertical(1.25, 10.0, 10.32);
}

#[test]
fn a_vertical_scroll_at_150_percent_moves_the_lines_rigidly() {
    vertical(1.5, 10.0, 10.4);
}

fn on_grid(v: f64, scale: f64) -> bool {
    let d = v * scale;
    (d - d.round()).abs() < 1e-6
}

/// A written offset lands on the grid at or below what was written, and is
/// what `scroll_top()` answers.
#[test]
fn a_written_offset_is_floored_to_the_device_grid() {
    let style = "width: 200px; height: 80px; overflow-y: auto; line-height: 17px";
    let text = format!("{TEXT} {TEXT} {TEXT} {TEXT}");
    let (mut doc, c) = document(style, &text, 1.25);
    doc.set_scroll_top(c, 10.33);
    // 12.9125 device px -> 12 -> 9.6.
    assert!(
        (doc.scroll_top(c) - 9.6).abs() < 1e-9,
        "got {}",
        doc.scroll_top(c)
    );
    // Idempotent: writing a value already on the grid keeps it.
    doc.set_scroll_top(c, 9.6);
    assert!(
        (doc.scroll_top(c) - 9.6).abs() < 1e-9,
        "got {}",
        doc.scroll_top(c)
    );
}

/// When the content shrinks under a scrolled region, layout clamps the offset
/// to the new range — on the grid and at or below the range, so the next pass
/// does not clamp it again.
#[test]
fn the_layout_clamp_lands_on_the_grid_and_stays() {
    let scale = 1.25;
    let (mut doc, c) = document("width: 200px; height: 80px; overflow-y: auto", "", scale);
    let inner = doc.create_element("div");
    doc.set_attribute(inner, "style", "height: 300px");
    doc.append_child(c, inner);
    doc.resolve_layout(VW, VH);
    doc.set_scroll_top(c, 200.0);
    assert_eq!(doc.scroll_top(c), 200.0);
    // Range 81 (layout rounds boxes to whole px): 101.25 device px, off
    // the grid.
    doc.set_attribute(inner, "style", "height: 161px");
    doc.resolve_layout(VW, VH);
    let top = doc.scroll_top(c);
    assert!(
        on_grid(top, scale) && (top - 80.8).abs() < 1e-9,
        "got {top}"
    );
    doc.tree.pending_scroll_clamps.clear();
    doc.resolve_layout(VW + 1.0, VH);
    assert!(
        doc.tree.pending_scroll_clamps.is_empty(),
        "a clamped offset was clamped again: {:?}",
        doc.tree.pending_scroll_clamps
    );
    assert_eq!(doc.scroll_top(c), top);
}

/// Writing back an offset read from the grid keeps it, even where the product
/// with the scale lands a hair below the whole pixel: at 175%, 61 device px is
/// `61 / 1.75` px, and `(61 / 1.75) * 1.75` is just under 61 in `f64`. A floor
/// with no slack would drop it to 60 on every write-back.
#[test]
fn rewriting_an_offset_on_the_grid_keeps_it() {
    let style = "width: 200px; height: 80px; overflow-y: auto";
    let (mut doc, c) = document(style, "", 1.75);
    let inner = doc.create_element("div");
    doc.set_attribute(inner, "style", "height: 400px");
    doc.append_child(c, inner);
    doc.resolve_layout(VW, VH);
    let v: f64 = 61.0 / 1.75;
    assert!(
        (v * 1.75).floor() < 61.0,
        "premise: the product is under 61"
    );
    doc.set_scroll_top(c, v);
    let first = doc.scroll_top(c);
    for _ in 0..3 {
        doc.set_scroll_top(c, doc.scroll_top(c));
    }
    assert_eq!(doc.scroll_top(c), first);
    assert!((first * 1.75 - 61.0).abs() < 1e-6, "got {first}");
}
