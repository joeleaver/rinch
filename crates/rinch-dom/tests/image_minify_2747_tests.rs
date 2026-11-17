//! #2747: an image bigger than one frame's minify budget
//! (`image_minify::MAX_MINIFY_WORK_PER_FRAME`, 16M source pixels) drawn at
//! half its size or less. Before the fix the budget was compared against the
//! whole image on every frame, so it was refused on every frame: the paint
//! asked for it again each time (`has_deferred_minify`), the app repainted
//! forever, and the picture was never filtered. Now its level is made a band
//! of rows at a time across frames, and once made it is drawn like any other.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::image_cache::DecodedImage;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VIEW: u32 = 120;
/// 17.64M pixels: over the 16M budget.
const SIDE: u32 = 4200;
/// More frames than this and the app is not going idle.
const MAX_FRAMES: usize = 4;

/// Thin diagonal stripes, which nearest sampling at 1/42 aliases into
/// something else, and a translucent band so the premultiplied levels are
/// made too.
fn picture(side: u32) -> DecodedImage {
    let s = side as usize;
    let mut px = vec![255u8; s * s * 4];
    for (y, row) in px.chunks_exact_mut(s * 4).enumerate() {
        for (x, p) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            if (x + 2 * y) % 7 < 2 {
                *p = [20, 40, 200, 255];
            }
            if y > s * 3 / 4 {
                p[3] = 96;
            }
        }
    }
    DecodedImage::new(px, side, side)
}

fn document(src: &str, image: DecodedImage) -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css(
        "html, body { margin: 0; background: #fff; }
         img { display: block; width: 100px; height: 100px; margin: 7px; }",
    );
    doc.tree.image_cache.insert_decoded(src.to_string(), image);
    let img = doc.create_element("img");
    doc.set_attribute(img, "src", src);
    let body = doc.body();
    doc.append_child(body, img);
    doc
}

fn paint(doc: &mut RinchDocument) -> Vec<u8> {
    doc.resolve_layout(VIEW as f32, VIEW as f32);
    let mut painter = TinySkiaPainter::new(VIEW, VIEW);
    painter.fill_white();
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (VIEW as f32, VIEW as f32),
        &mut doc.font_cx,
        &mut cx,
    );
    painter.pixels().to_vec()
}

#[test]
fn an_image_over_the_frame_budget_goes_idle_and_is_filtered() {
    let source = picture(SIDE);
    // What the filtered draw is: Chrome's level for a 100px draw of 4200px
    // (level 5, 132x132), drawn the rest of the way bilinearly, which is how
    // a 132px picture in the same box draws.
    let (lw, lh) = rinch_dom::image_minify::level_size(SIDE, SIDE, 5);
    assert_eq!((lw, lh), (132, 132));
    let level = rinch_dom::image_minify::mip_downscale(&source.data, SIDE, SIDE, lw, lh);
    let want = paint(&mut document("level.png", DecodedImage::new(level, lw, lh)));

    let mut doc = document("big.png", source);
    let made = rinch_dom::image_minify::made();
    let mut frames = 0;
    let mut got = paint(&mut doc);
    frames += 1;
    while doc.tree.has_deferred_minify() {
        assert!(
            frames < MAX_FRAMES,
            "still owed a frame after {frames} frames: the app never goes idle"
        );
        // The host's next frame: the deferred picture is paint-dirty again.
        doc.resolve_layout(VIEW as f32, VIEW as f32);
        got = paint(&mut doc);
        frames += 1;
    }
    assert!(frames > 1, "positive control: one frame cannot afford it");
    assert_eq!(
        rinch_dom::image_minify::made(),
        made + 1,
        "one copy, made once"
    );
    let bad = got
        .chunks(4)
        .zip(want.chunks(4))
        .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 1))
        .count();
    assert_eq!(bad, 0, "{bad} pixels differ from the filtered draw");

    // Idle: another frame makes nothing and owes nothing.
    doc.resolve_layout(VIEW as f32 + 1.0, VIEW as f32);
    let _ = paint(&mut doc);
    assert!(!doc.tree.has_deferred_minify());
    assert_eq!(rinch_dom::image_minify::made(), made + 1);
}
