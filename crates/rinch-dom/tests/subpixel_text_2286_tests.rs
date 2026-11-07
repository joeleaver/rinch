//! Subpixel (LCD) text, #2286 stage 3, on the software painter, and the
//! decisions the Vello painter makes before the GPU sees anything.
//!
//! Black text on white makes the fringes exact: a pixel is `255 * (1 - c)`
//! in each channel, `c` that channel's coverage, so a grey glyph has equal
//! channels everywhere and a subpixel one does not, and the BGR render is the
//! RGB render with red and blue swapped, pixel for pixel.
//!
//! - over an opaque page, per channel; through a clip (rounded
//!   `overflow: hidden`), still per channel;
//! - inside an opacity group, a mask or a `filter`, grey; over a transparent surface (a transparent window) or
//!   a translucent background, grey per pixel;
//! - under a rotation, grey; under a uniform `scale()`, per channel;
//! - BGR is RGB with red and blue swapped;
//! - the Vello painter asks the atlas for LCD masks where tiny-skia does, and
//!   grey ones inside a layer.
//!
//! Inter, registered under its own name, at declared sizes and line heights.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::glyph_atlas::{GlyphAtlas, LcdMode, SharedAtlas};
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use rinch_dom::paint::text_rasterization::{
    LcdFilter, ResolvedTextRasterization, SubpixelOrder, SystemTextSettings, TextAntialias,
    TextRasterization, reset_thread_policy, set_thread_policy,
};
use rinch_dom::paint::vello_painter::VelloPainter;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const W: u32 = 320;
const H: u32 = 60;

fn policy(order: Option<SubpixelOrder>) -> ResolvedTextRasterization {
    let r = TextRasterization::default()
        .stem_darkening(rinch_dom::paint::text_rasterization::StemDarkening::Off);
    match order {
        Some(order) => r
            .antialias(TextAntialias::Subpixel)
            .subpixel_order(Some(order))
            .lcd_filter(Some(LcdFilter::Default)),
        None => r.antialias(TextAntialias::Grayscale),
    }
    .resolve(&SystemTextSettings::default())
}

/// `html` in the body, Inter 13px/18px, the body `body_css`.
fn document(body_css: &str, html: &str) -> RinchDocument {
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
    doc.load_css(&format!(
        "html {{ margin: 0; }} body {{ margin: 0; padding: 10px 4.3px; \
         font: 13px/18px ProbeFace; color: #000; {body_css} }}"
    ));
    let body = doc.body();
    doc.set_inner_html(body, html);
    doc.resolve_layout(W as f32, H as f32);
    doc
}

const TEXT: &str = "Sphinx of black quartz, judge my vow.";

/// RGBA (premultiplied) painted by tiny-skia under `policy`, on a surface
/// starting transparent.
fn software(doc: &mut RinchDocument, policy: ResolvedTextRasterization) -> Vec<[u8; 4]> {
    set_thread_policy(policy);
    let mut painter = TinySkiaPainter::new(W, H);
    painter.set_glyph_atlas(std::rc::Rc::new(std::cell::RefCell::new(
        GlyphAtlas::default(),
    )));
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (W as f32, H as f32),
        &mut doc.font_cx,
        &mut cx,
    );
    reset_thread_policy();
    painter.pixels().as_chunks::<4>().0.to_vec()
}

/// Pixels whose colour channels differ by at least 24: subpixel fringes.
fn fringed(px: &[[u8; 4]]) -> usize {
    px.iter()
        .filter(|p| {
            let (lo, hi) = (p[0].min(p[1]).min(p[2]), p[0].max(p[1]).max(p[2]));
            hi - lo >= 24
        })
        .count()
}

/// Pixels with any ink: not the page's white (or not transparent).
fn inked(px: &[[u8; 4]], page: [u8; 4]) -> usize {
    px.iter().filter(|&&p| p != page).count()
}

/// Over an opaque white page the text is per channel (red at the parent: the
/// policy was recorded and drawn grey before stage 3), and grey with the
/// grey policy; through a rounded `overflow: hidden` clip it is still per
/// channel.
#[test]
fn software_subpixel_text_over_an_opaque_page() {
    let mut d = document(
        "background: #fff;",
        &format!("<p style='margin:0'>{TEXT}</p>"),
    );
    let lcd = software(&mut d, policy(Some(SubpixelOrder::Rgb)));
    let grey = software(&mut d, policy(None));
    assert!(inked(&grey, [255; 4]) > 400, "the text is painted");
    assert_eq!(fringed(&grey), 0, "grey text has equal channels");
    let n = fringed(&lcd);
    assert!(n > 150, "per-channel pixels: {n}");
    // Every pixel stays opaque.
    assert!(lcd.iter().all(|p| p[3] == 255));

    let mut clipped = document(
        "background: #fff;",
        &format!(
            "<div style='overflow:hidden; border-radius: 6px; height: 30px'>\
             <p style='margin:0'>{TEXT}</p></div>"
        ),
    );
    let c = software(&mut clipped, policy(Some(SubpixelOrder::Rgb)));
    assert!(fringed(&c) > 150, "through a clip: {}", fringed(&c));
}

/// BGR is RGB with red and blue swapped, exactly, for black on white.
#[test]
fn software_bgr_swaps_red_and_blue() {
    let mut d = document(
        "background: #fff;",
        &format!("<p style='margin:0'>{TEXT}</p>"),
    );
    let rgb = software(&mut d, policy(Some(SubpixelOrder::Rgb)));
    let bgr = software(&mut d, policy(Some(SubpixelOrder::Bgr)));
    assert_ne!(rgb, bgr);
    for (i, (a, b)) in rgb.iter().zip(&bgr).enumerate() {
        assert_eq!([a[2], a[1], a[0], a[3]], *b, "pixel {i}");
    }
}

/// Inside a layer the text is grey: an opacity group (even 0.99), a mask
/// (a `destination-in` layer), a filter (rinch parses no `mix-blend-mode`;
/// blend modes reach only backgrounds). Beside the layer, in the same document, it is per channel.
#[test]
fn software_layers_draw_grey_text() {
    for wrapper in [
        "opacity: 0.99",
        // The one f32 below 1 within its epsilon: tiny-skia composites it as
        // no layer at all (#560's no-op branch), Vello as a layer; both grey.
        "opacity: 0.99999994",
        "mask-image: linear-gradient(#000, #000)",
        "filter: grayscale(1)",
    ] {
        let mut d = document(
            "background: #fff;",
            &format!("<p style='margin:0; {wrapper}'>{TEXT}</p><p style='margin:0'>{TEXT}</p>"),
        );
        let px = software(&mut d, policy(Some(SubpixelOrder::Rgb)));
        let rows = |y0: u32, y1: u32| -> Vec<[u8; 4]> {
            px[(y0 * W) as usize..(y1 * W) as usize].to_vec()
        };
        // The first paragraph's line box is rows 10..28, the second 28..46.
        let inside = rows(10, 28);
        let beside = rows(28, 46);
        assert!(inked(&inside, [255; 4]) > 400, "{wrapper}: painted");
        assert_eq!(fringed(&inside), 0, "{wrapper}: grey inside the layer");
        assert!(fringed(&beside) > 150, "{wrapper}: per channel beside it");
    }
}

/// Over a surface that is not opaque the text is grey, per pixel: a
/// transparent window (nothing under the text), and a translucent
/// background. Over an opaque box on the same transparent surface it is per
/// channel.
#[test]
fn software_text_over_transparency_is_grey() {
    let mut d = document(
        "",
        &format!(
            "<p style='margin:0'>{TEXT}</p>\
             <p style='margin:0; background: rgba(255,255,255,0.6)'>{TEXT}</p>\
             <p style='margin:0; background: #fff'>{TEXT}</p>"
        ),
    );
    // A light colour on transparency would be invisible in a check of
    // channels: black text on transparent is all zero colour, so look at
    // the pixels' channel spread after compositing over white.
    let px = software(&mut d, policy(Some(SubpixelOrder::Rgb)));
    let over_white: Vec<[u8; 4]> = px
        .iter()
        .map(|p| {
            let a = 255 - p[3] as u16;
            let c = |v: u8| (v as u16 + a).min(255) as u8;
            [c(p[0]), c(p[1]), c(p[2]), 255]
        })
        .collect();
    let rows = |y0: u32, y1: u32| over_white[(y0 * W) as usize..(y1 * W) as usize].to_vec();
    let (bare, translucent, opaque) = (rows(10, 28), rows(28, 46), rows(46, H));
    assert!(inked(&bare, [255; 4]) > 400);
    assert_eq!(fringed(&bare), 0, "over a transparent window: grey");
    assert_eq!(
        fringed(&translucent),
        0,
        "over a translucent background: grey"
    );
    assert!(
        fringed(&opaque) > 100,
        "over an opaque box: {}",
        fringed(&opaque)
    );
}

/// Off the device grid (a rotation) the text is grey; a uniform scale is
/// rasterised at the device size and stays per channel.
#[test]
fn software_transforms() {
    let mut rotated = document(
        "background: #fff;",
        &format!("<p style='margin:0; transform: rotate(4deg)'>{TEXT}</p>"),
    );
    let px = software(&mut rotated, policy(Some(SubpixelOrder::Rgb)));
    assert!(inked(&px, [255; 4]) > 400);
    assert_eq!(fringed(&px), 0, "rotated: grey");
    let mut scaled = document(
        "background: #fff;",
        &format!("<p style='margin:0; transform: scale(1.25); transform-origin: 0 0'>{TEXT}</p>"),
    );
    let px = software(&mut scaled, policy(Some(SubpixelOrder::Rgb)));
    assert!(fringed(&px) > 150, "scaled: {}", fringed(&px));
}

/// The Vello painter's choice, which needs no GPU: the runs it asks the atlas
/// for are LCD ones outside layers and grey ones inside, the same runs the
/// software painter asks for; and `set_subpixel_text(false)` asks for grey.
#[test]
fn vello_asks_for_lcd_masks_where_software_does() {
    let lcd = Some(LcdMode {
        order: SubpixelOrder::Rgb,
        filter: LcdFilter::Default,
    });
    let modes = |html: &str, subpixel: bool| -> Vec<Option<LcdMode>> {
        let mut d = document("background: #fff;", html);
        let atlas: SharedAtlas = std::rc::Rc::new(std::cell::RefCell::new(GlyphAtlas::default()));
        let mut p = VelloPainter::new();
        p.set_glyph_atlas(atlas.clone());
        p.set_subpixel_text(subpixel);
        set_thread_policy(policy(Some(SubpixelOrder::Rgb)));
        let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
        use rinch_dom::paint::painter::Painter;
        p.reset();
        rinch_dom::paint::paint_document(
            &d.tree,
            &mut p,
            1.0,
            (W as f32, H as f32),
            &mut d.font_cx,
            &mut cx,
        );
        reset_thread_policy();
        let modes = atlas.borrow().run_lcd_modes();
        modes
    };
    assert_eq!(modes(&format!("<p>{TEXT}</p>"), true), vec![lcd]);
    assert_eq!(
        modes(&format!("<p style='opacity: .5'>{TEXT}</p>"), true),
        vec![None]
    );
    assert_eq!(
        modes(
            &format!("<p style='opacity: .5'>{TEXT}</p><p>{TEXT}</p>"),
            true
        ),
        vec![lcd, None],
        "the opacity group paints last, as a stacking context"
    );
    assert_eq!(
        modes(
            &format!(
                "<p style='opacity: .5'>{TEXT}</p>\
                 <p style='position: relative; z-index: 1'>{TEXT}</p>"
            ),
            true
        ),
        vec![None, lcd],
        "the layer's pop gives LCD back to what paints after it"
    );
    assert_eq!(modes(&format!("<p>{TEXT}</p>"), false), vec![None]);
}
