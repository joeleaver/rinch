//! The blurred `text-shadow` mask against everything that crops it, keys it
//! and keeps it (#980; review of #1020, round 2 — most fixtures here are the
//! reviewer's, adopted).
//!
//! A blurred shadow is rasterised into a coverage mask cropped to what can be
//! seen, blurred, and kept between paints under a key hashed from what was
//! rasterised. Each fixture compares a paint that may be served from the
//! cache, or cropped, with one that cannot be: the whole question is whether
//! a shortcut ever changes a pixel.

#![cfg(feature = "software-renderer")]

use peniko::kurbo::{Affine, Rect};
use peniko::{Brush, Fill};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::{PaintShape, Painter};
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use rinch_dom::perf::Counter;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const LOREM: &str = "The quick brown fox jumps over the lazy dog while a sphinx of black \
    quartz judges my vow; pack my box with five dozen liquor jugs, then 0123456789.";

fn doc(w: f32, h: f32, css: &str, html: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    doc.load_css("html, body { margin: 0; font-family: ProbeFace; color: rgb(0,0,0); }");
    doc.load_css(css);
    let body = doc.body();
    doc.set_inner_html(body, html);
    doc.resolve_layout(w, h);
    doc.resolve_layout(w, h);
    doc
}

fn full(d: &mut RinchDocument, p: &mut TinySkiaPainter, vw: f32, vh: f32, scale: f64) {
    p.fill_white();
    let mut cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(&d.tree, p, scale, (vw, vh), &mut d.font_cx, &mut cx);
}

fn partial(d: &mut RinchDocument, p: &mut TinySkiaPainter, vw: f32, vh: f32, scale: f64, r: Rect) {
    p.clear_rect_white(
        r.x0 as u32,
        r.y0 as u32,
        r.width() as u32,
        r.height() as u32,
    );
    rinch_dom::paint::set_dirty_rects(Some(&[r]));
    p.push_clip(Fill::NonZero, Affine::IDENTITY, &PaintShape::Rect(r));
    let mut cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(&d.tree, p, scale, (vw, vh), &mut d.font_cx, &mut cx);
    p.pop_layer();
    rinch_dom::paint::set_dirty_region(None);
}

/// Pixels that differ at all, the largest channel difference, and pixels
/// that differ by more than one step.
fn diff(a: &[u8], b: &[u8]) -> (usize, i32, usize) {
    let (mut n, mut m, mut big) = (0, 0, 0);
    for (x, y) in a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0) {
        let d = (0..4)
            .map(|c| (x[c] as i32 - y[c] as i32).abs())
            .max()
            .unwrap();
        if d > 0 {
            n += 1;
        }
        if d > 1 {
            big += 1;
        }
        m = m.max(d);
    }
    (n, m, big)
}

/// A partial repaint against a full paint of the same state, for every rect,
/// blur and scale, with the cache warm and cold: the worst pixel count that
/// differs by more than one step (the box blur's float drift at a crop edge
/// is one step; see the review).
fn partial_vs_full(blurs: &[f64], rects: &[Rect]) -> usize {
    let mut worst = 0;
    for &blur in blurs {
        for scale in [1.0, 1.5] {
            let (vw, vh) = (400.0f32, 300.0f32);
            let css = format!(
                "div {{ font-size: 28px; line-height: 34px; margin: 20px; \
                 text-shadow: 7px 13px {blur}px rgba(200,0,0,0.8); }}"
            );
            let mut d = doc(
                vw,
                vh,
                &css,
                "<div>Hamburgefontsiv quick</div><div>Second line WAVY</div>",
            );
            let (w, h) = ((vw as f64 * scale) as u32, (vh as f64 * scale) as u32);
            let mut p = TinySkiaPainter::new(w, h);
            for warm in [false, true] {
                for r in rects {
                    let r = Rect::new(r.x0 * scale, r.y0 * scale, r.x1 * scale, r.y1 * scale);
                    let r = Rect::new(r.x0.floor(), r.y0.floor(), r.x1.ceil(), r.y1.ceil());
                    rinch_dom::paint::clear_text_shadow_cache();
                    full(&mut d, &mut p, vw, vh, scale);
                    let a = p.pixels().to_vec();
                    if !warm {
                        rinch_dom::paint::clear_text_shadow_cache();
                    }
                    partial(&mut d, &mut p, vw, vh, scale, r);
                    let b = p.pixels().to_vec();
                    let (_, _, big) = diff(&a, &b);
                    if big > 0 {
                        eprintln!("blur {blur} scale {scale} warm {warm} rect {r:?}: {big} px");
                    }
                    worst = worst.max(big);
                }
            }
        }
    }
    worst
}

/// A partial repaint of a blurred shadow matches the full paint wherever the
/// damage touches the shadowed text's box — including a rect that crops only
/// the mask's right or bottom edge, which keeps its origin and changes only
/// its size.
///
/// Kills: the mask's width and height left out of the cache key (the full
/// mask is served for the smaller crop).
#[test]
fn a_partial_repaint_matches_the_full_paint() {
    let worst = partial_vs_full(
        &[0.0, 2.0, 6.0, 16.0, 30.0],
        &[
            Rect::new(50.0, 40.0, 130.0, 90.0),
            Rect::new(211.0, 0.0, 219.0, 300.0),
            Rect::new(90.0, 100.0, 260.0, 160.0),
            Rect::new(0.0, 0.0, 400.0, 300.0),
            Rect::new(0.0, 0.0, 150.0, 60.0),
            Rect::new(0.0, 0.0, 400.0, 45.0),
        ],
    );
    assert_eq!(worst, 0, "a partial repaint differs from the full paint");
}

/// Damage over a shadow's bleed but not its box — a strip in the gap below a
/// shadowed line — repaints the shadow there (#889 item 1): the paint prune
/// tests the node's box grown by its own ink, `text-shadow` included.
///
/// Kills: the prune testing the bare border box (the strip is cleared and
/// nothing repaints the shadow in it: 1,975 px wrong at a 12px blur).
#[test]
fn damage_over_a_shadows_bleed_repaints_it() {
    for blur in [0.0, 12.0] {
        let worst = partial_vs_full(&[blur], &[Rect::new(3.0, 61.0, 397.0, 67.0)]);
        assert_eq!(
            worst, 0,
            "blur {blur}: the strip lost the shadow painted in it"
        );
    }
}

/// A shadow partly off the top of the window: the crop to what can be seen
/// changes nothing inside the window (the reference paints with no window,
/// so nothing is cropped).
///
/// Kills: the crop not grown by the blur's reach (pixels just off screen
/// bleed nothing back in).
#[test]
fn the_viewport_crop_is_exact() {
    for top in [-10.0, -23.5, -40.0, -61.0] {
        for blur in [3.0, 12.0] {
            let (vw, vh) = (300.0f32, 120.0f32);
            let css = format!(
                "div {{ font-size: 32px; line-height: 40px; margin: {top}px 0 0 10px; \
                 text-shadow: 4px 30px {blur}px rgb(0,0,200); }}"
            );
            let mut d = doc(vw, vh, &css, "<div>HxHg jump</div>");
            let mut p = TinySkiaPainter::new(300, 120);
            rinch_dom::paint::clear_text_shadow_cache();
            full(&mut d, &mut p, vw, vh, 1.0);
            let a = p.pixels().to_vec();
            rinch_dom::paint::clear_text_shadow_cache();
            full(&mut d, &mut p, 0.0, 0.0, 1.0);
            let b = p.pixels().to_vec();
            assert_eq!(
                diff(&a, &b).0,
                0,
                "top {top} blur {blur}: the crop changed pixels"
            );
        }
    }
}

/// One input changed in place, in **one** document (each document registers
/// its own copy of the face, and its blob id is in the key, so two documents
/// never share a mask), painted from the cache and then from nothing: the two
/// must agree, and each must differ from before the change.
///
/// Kills: the blur left out of the key (2px to 1.8px keeps the kernel's
/// reach and the mask's size, so only sigma tells them apart); any other
/// input of the rasterised copy left out of it.
#[test]
fn a_cached_mask_is_never_stale() {
    let base = "font-size: 30px; line-height: 38px; margin: 20px; \
                text-shadow: 5px 25px 8px rgb(200,0,0)";
    let cases: &[(&str, &str, &str)] = &[
        ("underline", "", "text-decoration: underline"),
        ("line-through", "", "text-decoration: line-through"),
        ("wavy", "", "text-decoration: underline wavy"),
        (
            "fractional offset",
            "",
            "text-shadow: 5.5px 25.25px 8px rgb(200,0,0)",
        ),
        ("blur", "", "text-shadow: 5px 25px 8.2px rgb(200,0,0)"),
        (
            "small blur",
            "text-shadow: 5px 25px 2px rgb(200,0,0)",
            "text-shadow: 5px 25px 1.8px rgb(200,0,0)",
        ),
        ("letter-spacing", "", "letter-spacing: 0.3px"),
        ("colour", "", "text-shadow: 5px 25px 8px rgb(0,0,200)"),
        (
            "colour alpha",
            "",
            "text-shadow: 5px 25px 8px rgba(200,0,0,0.3)",
        ),
        ("margin 0.5", "", "margin-left: 20.5px"),
    ];
    let mut stale = vec![];
    for scale in [1.0, 1.5] {
        for (name, pre, extra) in cases {
            let (w, h) = ((300.0 * scale) as u32, (150.0 * scale) as u32);
            let mut p = TinySkiaPainter::new(w, h);
            let mut d = doc(
                300.0,
                150.0,
                "",
                &format!("<div style=\"{base}; {pre}\">Hx<span>Hq</span> go</div>"),
            );
            let body = d.body();
            let div = NodeId(d.tree.get(body.0).unwrap().children[0]);
            rinch_dom::paint::clear_text_shadow_cache();
            full(&mut d, &mut p, 300.0, 150.0, scale);
            let before = p.pixels().to_vec();
            d.set_attribute(div, "style", &format!("{base}; {pre}; {extra}"));
            d.resolve_layout(300.0, 151.0);
            d.resolve_layout(300.0, 150.0);
            full(&mut d, &mut p, 300.0, 150.0, scale);
            let warm = p.pixels().to_vec();
            rinch_dom::paint::clear_text_shadow_cache();
            full(&mut d, &mut p, 300.0, 150.0, scale);
            let cold = p.pixels().to_vec();
            // Positive control. At 1.5x the small blur's sigma, 1.5 and 1.35,
            // is past the sampled kernel, and three box blurs of whole-pixel
            // widths do not tell the two apart: the page is the same.
            if scale == 1.0 {
                assert!(
                    diff(&before, &cold).0 > 0,
                    "positive control: {name} changes the page"
                );
            }
            if diff(&warm, &cold).0 > 0 {
                stale.push((scale, *name));
            }
        }
    }
    assert!(stale.is_empty(), "stale masks served: {stale:?}");
}

/// A scroll by a fraction of a physical pixel keeps the cache: the text's
/// sub-pixel phase is snapped to a quarter pixel, so a scroll to `k + 0.1`
/// after one to `k` rasterises no mask for a shadow wholly inside the
/// scroller (review of #1020, round 2, F2: every frame of a fling used to
/// re-rasterise every shadow on screen) — and draws within a step of what a
/// cold paint draws there.
///
/// Kills: the phase not snapped.
#[test]
fn a_fractional_scroll_rasterises_no_mask() {
    let mut html = String::new();
    for i in 0..30 {
        html.push_str(&format!("<p>{i}: {LOREM}</p>"));
    }
    // The paragraphs are one line each and the scroller is taller than the
    // content can move in this test, so no shadow crosses its clip edge.
    let css = "body { font-size: 14px; line-height: 18px; } \
               #s { height: 700px; overflow-y: auto; margin-top: 40px; } \
               p { margin: 0 0 2px 0; white-space: nowrap; \
               text-shadow: 0 1px 4px rgba(0,0,0,.4); }";
    let mut d = doc(
        1200.0,
        800.0,
        css,
        &format!("<div id=s>{html}</div><div style=\"height: 2000px\"></div>"),
    );
    let body = d.body();
    let s = NodeId(d.tree.get(body.0).unwrap().children[0]);
    for scale in [1.0, 1.25] {
        let (w, h) = ((1200.0 * scale) as u32, (800.0 * scale) as u32);
        let mut p = TinySkiaPainter::new(w, h);
        for k in 1..=4 {
            d.set_scroll_top(s, k as f64 / scale);
            full(&mut d, &mut p, 1200.0, 800.0, scale);
            d.tree.perf.reset();
            d.set_scroll_top(s, (k as f64 + 0.1) / scale);
            full(&mut d, &mut p, 1200.0, 800.0, scale);
            let warm = p.pixels().to_vec();
            let n = d.tree.perf.get(Counter::TextShadowMasksRasterised);
            assert_eq!(
                n, 0,
                "scale {scale}: a scroll of 0.1 physical px from {k} rasterised {n} masks"
            );
            rinch_dom::paint::clear_text_shadow_cache();
            d.tree.perf.reset();
            full(&mut d, &mut p, 1200.0, 800.0, scale);
            assert!(
                d.tree.perf.get(Counter::TextShadowMasksRasterised) > 10,
                "positive control: a cold paint rasterises every visible shadow"
            );
            let cold = p.pixels().to_vec();
            assert_eq!(
                diff(&warm, &cold).2,
                0,
                "the served masks are the cold paint's"
            );
        }
    }
}

/// The scratch pixmap shadows are rasterised into is trimmed: after a paint
/// that needed a large mask, eight paints that need none give it back.
///
/// Kills: the scratch only ever growing (64 MB a thread at the side cap).
#[test]
fn the_scratch_pixmap_is_given_back() {
    let css = "div { font-size: 90px; line-height: 100px; margin: 20px; \
               text-shadow: 0 10px 20px rgb(200,0,0); }";
    let mut big = doc(1000.0, 400.0, css, "<div>A large shadowed line</div>");
    let mut none = doc(1000.0, 400.0, "", "<div>no shadow</div>");
    let mut p = TinySkiaPainter::new(1000, 400);
    rinch_dom::paint::clear_text_shadow_cache();
    full(&mut big, &mut p, 1000.0, 400.0, 1.0);
    let (w, h) = rinch_dom::paint::text_shadow_scratch_size().expect("a mask was rasterised");
    assert!(
        w > 500 && h > 100,
        "positive control: the scratch is large ({w}x{h})"
    );
    for _ in 0..8 {
        full(&mut none, &mut p, 1000.0, 400.0, 1.0);
    }
    assert_eq!(
        rinch_dom::paint::text_shadow_scratch_size(),
        None,
        "eight paints with no blurred shadow keep the scratch pixmap"
    );
}
