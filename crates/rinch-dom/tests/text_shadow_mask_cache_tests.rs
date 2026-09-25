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

/// A scroll by fractions of a physical pixel keeps the cache wherever it
/// moves no glyph to another pixel: the key is the pixel each glyph rounds
/// to, as the software painter places it, not its exact position. Over a
/// sweep of nine tenth-pixel steps a line's baseline crosses a half pixel at
/// most once, so the sweep rasterises at most one new mask per shadow; a
/// second sweep rasterises none; and every warm paint equals a cold one.
///
/// Kills: the key hashing exact positions (every step rasterises every
/// shadow: nine times the bound); the key rounding differently from the
/// painter (a warm paint differs from the cold one).
#[test]
fn a_sub_pixel_scroll_rasterises_only_where_a_glyph_moves_pixel() {
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
        let at = |j: usize| (1.0 + j as f64 / 10.0) / scale;
        rinch_dom::paint::clear_text_shadow_cache();
        d.set_scroll_top(s, at(0));
        d.tree.perf.reset();
        full(&mut d, &mut p, 1200.0, 800.0, scale);
        let visible = d.tree.perf.get(Counter::TextShadowMasksRasterised);
        assert!(
            visible > 10,
            "positive control: {visible} shadows on screen"
        );
        d.tree.perf.reset();
        for j in 1..10 {
            d.set_scroll_top(s, at(j));
            full(&mut d, &mut p, 1200.0, 800.0, scale);
        }
        let swept = d.tree.perf.get(Counter::TextShadowMasksRasterised);
        assert!(
            swept <= visible,
            "scale {scale}: a sub-pixel sweep rasterised {swept} masks for {visible} shadows"
        );
        let mut warm = vec![];
        for j in 0..10 {
            d.set_scroll_top(s, at(j));
            d.tree.perf.reset();
            full(&mut d, &mut p, 1200.0, 800.0, scale);
            let n = d.tree.perf.get(Counter::TextShadowMasksRasterised);
            assert_eq!(
                n, 0,
                "scale {scale}: step {j} of a second sweep rasterised {n}"
            );
            warm.push(p.pixels().to_vec());
        }
        for (j, warm) in warm.iter().enumerate() {
            d.set_scroll_top(s, at(j));
            rinch_dom::paint::clear_text_shadow_cache();
            full(&mut d, &mut p, 1200.0, 800.0, scale);
            assert_eq!(
                diff(warm, p.pixels()).0,
                0,
                "scale {scale}, step {j}: a served mask is not the cold paint's"
            );
        }
    }
}

/// Weighted centroid of the pixels leaning to one channel: red (`r - b`)
/// or blue (`b - r`).
fn centroid(px: &[u8], w: u32, blue: bool) -> (f64, f64) {
    let (mut sx, mut sy, mut sw) = (0.0, 0.0, 0.0);
    for (i, c) in px.as_chunks::<4>().0.iter().enumerate() {
        let lean = if blue {
            c[2] as f64 - c[0] as f64
        } else {
            c[0] as f64 - c[2] as f64
        };
        if lean > 0.0 {
            let (x, y) = ((i as u32 % w) as f64, (i as u32 / w) as f64);
            sx += x * lean;
            sy += y * lean;
            sw += lean;
        }
    }
    assert!(
        sw > 0.0,
        "positive control: some {} ink",
        if blue { "blue" } else { "red" }
    );
    (sx / sw, sy / sw)
}

/// A blurred shadow sits exactly where its text is, plus its offset, at any
/// sub-pixel position: the mask is rasterised at the text's true offset, so
/// each glyph lands on the pixel the text's own glyph does. And a mask served
/// from the cache — rasterised at another position — is the one a cold
/// paint rasterises here.
///
/// Kills: the phase snapped before rasterising (review of #1020, round 3, F2:
/// a line's shadow moved 1.04 px against its text at 1.1x); a key that
/// shares a mask between positions that round differently.
#[test]
fn a_shadow_is_drawn_where_its_text_is_at_any_sub_pixel_position() {
    for scale in [1.0, 1.1, 2.0] {
        for blur in [0.5, 3.0] {
            let (vw, vh) = (220.0f32, 90.0f32);
            let (w, h) = ((vw as f64 * scale) as u32, (vh as f64 * scale) as u32);
            let base = format!(
                "margin: 10px 0 0 10px; font-size: 20px; line-height: 24px; \
                 color: rgb(255,0,0); text-shadow: 0 30px {blur}px rgb(0,0,255)"
            );
            let mut d = doc(vw, vh, "", &format!("<p style=\"{base}\">Hxgl fiv</p>"));
            let body = d.body();
            let el = NodeId(d.tree.get(body.0).unwrap().children[0]);
            let mut p = TinySkiaPainter::new(w, h);
            rinch_dom::paint::clear_text_shadow_cache();
            for f in [0.0, 0.1, 0.3, 0.45, 0.5, 0.55, 0.7, 0.9, 0.3, 0.0] {
                d.set_attribute(
                    el,
                    "style",
                    &format!("{base}; transform: translate({f}px, {f}px)"),
                );
                d.resolve_layout(vw, vh + 1.0);
                d.resolve_layout(vw, vh);
                full(&mut d, &mut p, vw, vh, scale);
                let warm = p.pixels().to_vec();
                rinch_dom::paint::clear_text_shadow_cache();
                full(&mut d, &mut p, vw, vh, scale);
                let cold = p.pixels().to_vec();
                assert_eq!(
                    diff(&warm, &cold).0,
                    0,
                    "scale {scale} blur {blur} at {f}: a served mask is not the cold paint's"
                );
                let text = centroid(&cold, w, false);
                let shadow = centroid(&cold, w, true);
                let (dx, dy) = (
                    shadow.0 - text.0,
                    shadow.1 - text.1 - (30.0 * scale).round(),
                );
                assert!(
                    dx.abs() <= 0.15 && dy.abs() <= 0.15,
                    "scale {scale} blur {blur} at {f}: the shadow is ({dx:.2}, {dy:.2}) px off its text"
                );
            }
        }
    }
}

/// Damage over a box's own ink but not its box, partial against full: the
/// worst count of pixels differing by more than a step, with a positive
/// control that the strip holds ink.
fn bleed_partial_vs_full(css: &str, html: &str, strip: Rect, scale: f64) -> usize {
    let (vw, vh) = (400.0f32, 300.0f32);
    let mut d = doc(vw, vh, css, html);
    let (w, h) = ((vw as f64 * scale) as u32, (vh as f64 * scale) as u32);
    let mut p = TinySkiaPainter::new(w, h);
    full(&mut d, &mut p, vw, vh, scale);
    let a = p.pixels().to_vec();
    let r = Rect::new(
        (strip.x0 * scale).floor(),
        (strip.y0 * scale).floor(),
        (strip.x1 * scale).ceil(),
        (strip.y1 * scale).ceil(),
    );
    let mut inked = 0;
    for y in r.y0 as u32..r.y1 as u32 {
        for x in r.x0 as u32..r.x1 as u32 {
            let i = ((y * w + x) * 4) as usize;
            if a[i..i + 3] != [255, 255, 255] {
                inked += 1;
            }
        }
    }
    assert!(
        inked > 0,
        "positive control: the strip holds ink ({inked} px)"
    );
    partial(&mut d, &mut p, vw, vh, scale, r);
    diff(&a, p.pixels()).2
}

/// The prune's ink check covers `box-shadow` and `outline`, not only
/// `text-shadow`, in every placement (the review's fixtures, round 3).
///
/// Kills: `box-shadow` or `outline` left out of the ink check (1,152 to
/// 3,720 px wrong per case).
#[test]
fn damage_over_a_box_shadows_or_outlines_bleed_repaints_it() {
    let b = "position: absolute; left: 100px; top: 60px; width: 120px; height: 50px; \
             background: #eee;";
    let cases: &[(&str, String, &str, Rect)] = &[
        (
            "box-shadow blur+spread",
            format!("#b {{ {b} box-shadow: 0 0 12px 8px rgb(0,0,200); }}"),
            "<div id=b></div>",
            Rect::new(90.0, 112.0, 240.0, 120.0),
        ),
        (
            "box-shadow negative spread, offset",
            format!("#b {{ {b} box-shadow: 0 20px 12px -4px rgb(0,0,200); }}"),
            "<div id=b></div>",
            Rect::new(90.0, 112.0, 240.0, 125.0),
        ),
        (
            "outline + offset",
            format!("#b {{ {b} outline: 4px solid rgb(0,150,0); outline-offset: 6px; }}"),
            "<div id=b></div>",
            Rect::new(90.0, 113.0, 240.0, 119.0),
        ),
        (
            "static box",
            "#b { margin: 60px 0 0 100px; width: 120px; height: 50px; background: #eee; \
             box-shadow: 0 0 12px 8px rgb(0,0,200); }"
                .into(),
            "<div id=b></div><div style=\"height:20px\"></div>",
            Rect::new(90.0, 112.0, 240.0, 120.0),
        ),
        (
            "rotated 30deg",
            "#b { position: absolute; left: 140px; top: 100px; width: 120px; height: 50px; \
             background: #eee; transform: rotate(30deg); box-shadow: 0 0 14px 6px rgb(0,0,200); }"
                .into(),
            "<div id=b></div>",
            Rect::new(100.0, 190.0, 320.0, 200.0),
        ),
        (
            "hoisted z-index",
            "#w { width: 300px; height: 200px; } #b { position: relative; z-index: 3; \
             left: 30px; top: 30px; width: 120px; height: 50px; background: #eee; \
             box-shadow: 0 0 12px 8px rgb(0,0,200); }"
                .into(),
            "<div id=w><div id=b></div></div>",
            Rect::new(20.0, 82.0, 170.0, 90.0),
        ),
        (
            "fixed",
            format!("#b {{ {b} position: fixed; box-shadow: 0 0 12px 8px rgb(0,0,200); }}"),
            "<div id=b></div>",
            Rect::new(90.0, 112.0, 240.0, 120.0),
        ),
        (
            "child of a clipping parent",
            "#p { position: absolute; left: 50px; top: 30px; width: 300px; height: 200px; \
             overflow: hidden; padding: 30px; } #b { width: 120px; height: 50px; \
             background: #eee; box-shadow: 0 0 12px 8px rgb(0,0,200); }"
                .into(),
            "<div id=p><div id=b></div></div>",
            Rect::new(70.0, 112.0, 220.0, 120.0),
        ),
    ];
    let mut bad = vec![];
    for (name, css, html, strip) in cases {
        for scale in [1.0, 1.5] {
            let n = bleed_partial_vs_full(css, html, *strip, scale);
            if n > 0 {
                bad.push(format!("{name} @{scale}: {n}"));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "the strip lost the ink painted in it: {bad:?}"
    );
}

/// The ink reach is scaled to device pixels: at 2x a strip 25 to 35 CSS px
/// below a box whose hard shadow spreads 40px is inside the reach, and would
/// be outside it (20 CSS px) if the outsets were taken as device pixels.
///
/// Kills: the outsets not scaled.
#[test]
fn the_ink_reach_is_scaled_to_the_device() {
    let css = "#b { position: absolute; left: 100px; top: 40px; width: 120px; height: 50px; \
               background: #eee; box-shadow: 0 0 0 40px rgb(0,0,200); }";
    for scale in [1.0, 2.0] {
        let n = bleed_partial_vs_full(
            css,
            "<div id=b></div>",
            Rect::new(80.0, 115.0, 240.0, 125.0),
            scale,
        );
        assert_eq!(
            n, 0,
            "scale {scale}: the strip lost the shadow painted in it"
        );
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
