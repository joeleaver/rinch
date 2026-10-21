//! #804: a colour emoji face whose glyphs are bitmap strikes (CBDT/CBLC —
//! Noto Color Emoji, Android's emoji; sbix — Apple Color Emoji) painted
//! NOTHING on the software backend: `TinySkiaPainter::draw_glyphs` asked swash
//! for a colour outline (COLR) or a plain outline only, and such a glyph has
//! neither.
//!
//! The face is `assets/fonts/NotoColorEmoji-U1F600.ttf`, a one-glyph subset of
//! Noto Color Emoji (OFL, `NotoColorEmoji-OFL.txt`): one strike at 109 ppem,
//! registered under an override family name so no host font answers instead.
//!
//! The oracle is local: the only ink in the page is the emoji's, so the
//! coloured pixels and their bounding box are its. The box pins the scaling
//! (the strike is 109 ppem, the text 32px: unscaled it would be ~128px tall
//! and run off the 100px page).

#![cfg(feature = "software-renderer")]

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 200.0;
const VH: f32 = 100.0;
const FACE: &[u8] = include_bytes!("../assets/fonts/NotoColorEmoji-U1F600.ttf");

fn document(size_px: u32) -> RinchDocument {
    styled(&format!(
        "font-size: {size_px}px; line-height: 40px; color: rgb(0, 0, 0)"
    ))
}

/// The emoji in a `div` at (10, 10) styled `style` (font size, line height and
/// colour), the face named `ProbeEmoji`.
fn styled(style: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeEmoji"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    let body = doc.body();
    let c = doc.create_element("div");
    doc.set_attribute(
        c,
        "style",
        &format!("margin: 10px 0 0 10px; {style}; font-family: ProbeEmoji"),
    );
    doc.append_child(body, c);
    let t = doc.create_text("\u{1F600}");
    doc.append_child(c, t);
    doc.resolve_layout(VW, VH);
    doc
}

fn paint(doc: &mut RinchDocument, reference: bool) -> Vec<[u8; 4]> {
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    painter.set_reference_mode(reference);
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut cx,
    );
    painter.pixels().as_chunks::<4>().0.to_vec()
}

/// Coloured pixels (a channel spread over 40: the page is white and the text
/// colour black, so nothing else is coloured) and their bounding box.
fn coloured(px: &[[u8; 4]]) -> (usize, Option<(usize, usize, usize, usize)>) {
    let mut n = 0;
    let mut bb: Option<(usize, usize, usize, usize)> = None;
    for (i, p) in px.iter().enumerate() {
        let (mx, mn) = (p[0].max(p[1]).max(p[2]), p[0].min(p[1]).min(p[2]));
        if mx - mn > 40 {
            n += 1;
            let (x, y) = (i % VW as usize, i / VW as usize);
            bb = Some(match bb {
                None => (x, y, x, y),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
            });
        }
    }
    (n, bb)
}

#[test]
fn a_cbdt_emoji_paints_in_colour_at_the_font_size_804() {
    let mut doc = document(32);
    let px = paint(&mut doc, false);
    // The page is transparent, so every pixel is the glyph's own,
    // premultiplied: no channel above its alpha. A strike drawn straight
    // (unpremultiplied) breaks this at every antialiased edge.
    let bad = px
        .iter()
        .filter(|p| p[0] > p[3] || p[1] > p[3] || p[2] > p[3])
        .count();
    assert_eq!(bad, 0, "{bad} pixels are not valid premultiplied colour");
    let (n, bb) = coloured(&px);
    let (x0, y0, x1, y1) = bb.expect("the CBDT emoji painted no coloured pixel at all (#804)");
    let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
    // The smiley fills most of a 32px em: scaled from the 109 ppem strike it
    // is about 30px across. Unscaled it would be about 4x that.
    assert!(
        (24..=40).contains(&w) && (24..=40).contains(&h),
        "ink box {w}x{h} at ({x0},{y0})"
    );
    assert!(n > 300, "only {n} coloured pixels");
    // It sits on its line, inside the 40px line box starting at y = 10.
    assert!(y0 >= 10 && y1 < 52, "ink rows {y0}..={y1}");
}

/// A larger size scales up from the same strike: the box grows with it.
#[test]
fn the_strike_scales_with_the_font_size_804() {
    let (_, small) = coloured(&paint(&mut document(20), false));
    let (_, large) = coloured(&paint(&mut document(40), false));
    let (s, l) = (small.unwrap(), large.unwrap());
    let (sw, lw) = (s.2 - s.0 + 1, l.2 - l.0 + 1);
    // 40 / 20 = 2, within a pixel or two of rounding at each edge.
    assert!((2 * sw).abs_diff(lw) <= 3, "20px is {sw} wide, 40px {lw}");
}

/// The glyph cache draws the same pixels as the uncached reference path,
/// cold and warm.
#[test]
fn the_cached_bitmap_glyph_matches_the_reference_804() {
    let mut doc = document(32);
    let reference = paint(&mut doc, true);
    assert!(
        coloured(&reference).0 > 0,
        "the reference painted the emoji"
    );
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    for pass in 0..2 {
        painter.fill_transparent();
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut painter,
            1.0,
            (VW, VH),
            &mut doc.font_cx,
            &mut cx,
        );
        let px: Vec<[u8; 4]> = painter.pixels().as_chunks::<4>().0.to_vec();
        assert!(px == reference, "pass {pass} differs from the reference");
    }
}

/// The text colour's alpha reaches a colour glyph, as in Chrome 155 and
/// Vello: `rgba(0, 0, 0, .5)` draws the emoji at half alpha (Chrome: max
/// alpha 128) and `color: transparent` draws nothing (Chrome: 0 px). Only the
/// colour's channels are ignored.
#[test]
fn the_text_colours_alpha_reaches_the_emoji_804() {
    let max_alpha = |style: &str| {
        let mut doc = styled(&format!("font-size: 32px; line-height: 40px; {style}"));
        paint(&mut doc, false)
            .iter()
            .map(|p| p[3])
            .max()
            .unwrap_or(0)
    };
    assert_eq!(max_alpha("color: rgb(0, 0, 0)"), 255, "positive control");
    let half = max_alpha("color: rgba(0, 0, 0, 0.5)");
    assert!(
        (127..=129).contains(&half),
        "rgba(.5) gave max alpha {half}"
    );
    assert_eq!(
        max_alpha("color: transparent"),
        0,
        "a transparent colour hid nothing"
    );
}

/// At the strike's own ppem (109px, so no resampling) every drawn pixel is
/// the strike premultiplied exactly ONCE. A second premultiply (the glyph
/// sent through the straight-alpha blitter) still gives valid premultiplied
/// colour, so only this sees it. (From the review of #1689.)
#[test]
fn at_strike_ppem_pixels_are_the_strike_premultiplied_once_804() {
    let sf = swash::FontRef::from_index(FACE, 0).unwrap();
    let gid = sf.charmap().map('\u{1F600}');
    let mut cx = swash::scale::ScaleContext::new();
    let mut s = cx.builder(sf).size(109.0).build();
    let img = s
        .scale_color_bitmap(gid, swash::scale::StrikeWith::ExactSize)
        .unwrap();
    let (sw, sh) = (img.placement.width as usize, img.placement.height as usize);
    let pm = |c: u8, a: u8| ((c as u32 * a as u32 + 127) / 255) as u8;
    let strike: Vec<[u8; 4]> = img.data.as_chunks::<4>().0[..sw * sh]
        .iter()
        .map(|p| [pm(p[0], p[3]), pm(p[1], p[3]), pm(p[2], p[3]), p[3]])
        .collect();
    let alpha_box = |px: &[[u8; 4]], w: usize| {
        let mut b = (usize::MAX, usize::MAX, 0, 0);
        for (i, p) in px.iter().enumerate() {
            if p[3] > 0 {
                let (x, y) = (i % w, i / w);
                b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
            }
        }
        b
    };
    let sb = alpha_box(&strike, sw);
    // Laid out at 300x300 (the shared builder's 200x100 page would cut it).
    let (w, h) = (300usize, 300usize);
    let mut doc = RinchDocument::new();
    {
        use parley::fontique::{Blob, FontInfoOverride};
        doc.font_cx.collection.register_fonts(
            Blob::new(std::sync::Arc::new(FACE)),
            Some(FontInfoOverride {
                family_name: Some("ProbeEmoji"),
                ..Default::default()
            }),
        );
    }
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let c = doc.create_element("div");
    doc.set_attribute(
        c,
        "style",
        "position: absolute; left: 10px; top: 10px; font: 109px/218px ProbeEmoji; color: black",
    );
    doc.append_child(body, c);
    let t = doc.create_text("\u{1F600}");
    doc.append_child(c, t);
    doc.resolve_layout(w as f32, h as f32);
    let mut painter = TinySkiaPainter::new(w as u32, h as u32);
    let mut lcx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (w as f32, h as f32),
        &mut doc.font_cx,
        &mut lcx,
    );
    let px: Vec<[u8; 4]> = painter.pixels().as_chunks::<4>().0.to_vec();
    let ab = alpha_box(&px, w);
    assert_eq!(
        (ab.2 - ab.0, ab.3 - ab.1),
        (sb.2 - sb.0, sb.3 - sb.1),
        "same ink box"
    );
    let (dx, dy) = (ab.0 as isize - sb.0 as isize, ab.1 as isize - sb.1 as isize);
    let mut differing = 0;
    for y in 0..sh {
        for x in 0..sw {
            let (px_, py_) = (x as isize + dx, y as isize + dy);
            if px_ < 0 || py_ < 0 || px_ as usize >= w || py_ as usize >= h {
                continue;
            }
            let (a, b) = (strike[y * sw + x], px[py_ as usize * w + px_ as usize]);
            if (0..4).any(|c| (a[c] as i32 - b[c] as i32).abs() > 1) {
                differing += 1;
            }
        }
    }
    assert_eq!(
        differing, 0,
        "{differing} pixels are not the strike premultiplied once"
    );
}
