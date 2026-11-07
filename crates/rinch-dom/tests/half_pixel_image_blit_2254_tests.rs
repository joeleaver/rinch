//! #2254: tiny-skia's half-pixel blit dropped the first row and column of
//! images and shadow masks, as #2251 fixed for glyphs.
//!
//! `TinySkiaPainter` draws an image (and a blurred text shadow's mask drawn
//! through a transform, and a nearest-sampled repeating tile) with
//! `draw_pixmap` and nearest sampling. That fills the image's device rect with
//! its edges rounded half up and samples the texel under each pixel's centre,
//! so at an edge of exactly `k + 0.5` the first pixel's centre is the boundary
//! between texels 0 and 1 and takes texel 1: texel row (or column) 0 is never
//! drawn and the last is drawn twice. Under `scale(2)` the first texel is drawn
//! once and the last three times. The painter now rounds the rect's edges to
//! whole pixels itself and stretches the image to them
//! (`skia_painter::snap_image_placement`), which is also where Chrome puts a
//! replaced element's picture.
//!
//! Three kinds of oracle:
//! - the painter driven directly, where each texel's device pixels are known
//!   exactly: every texel of a 1-pixel-per-texel image must be drawn once,
//!   and under `scale(2)` twice, wherever the image lands;
//! - Chrome 156's pixels for `<img>`s at half and other fractional positions;
//! - the issue's scenario, a logo centred by `vertical-align: middle` in a
//!   cell one pixel taller than an odd fit, against the same logo one whole
//!   pixel up and down.
//!
//! And #2251's review gaps for glyphs: rotated and unevenly scaled text keep
//! their transform on the software painter (the old `if true` mutant of the
//! translation-only guard passed the whole suite).

#![cfg(feature = "software-renderer")]

use peniko::kurbo::Affine;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::image_cache::DecodedImage;
use rinch_dom::paint::painter::{ImageSampling, PaintImage, Painter};
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const YELLOW: [u8; 4] = [255, 255, 0, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];

/// A `w` x `h` image whose texel `(x, y)` is `(10 * x + 5, 10 * y + 5, 77)`,
/// opaque or with alpha 200: every texel is told apart by its colour.
fn ramp(w: u32, h: u32, opaque: bool) -> Vec<u8> {
    let a = if opaque { 255 } else { 200 };
    (0..h)
        .flat_map(|y| (0..w).map(move |x| [(10 * x + 5) as u8, (10 * y + 5) as u8, 77, a]))
        .flatten()
        .collect()
}

/// The texel a drawn pixel came from, read back from its colour (on a
/// transparent surface, so a translucent texel is its premultiplied self).
fn texel(p: [u8; 4], opaque: bool) -> Option<(u32, u32)> {
    if p[3] == 0 {
        return None;
    }
    let (r, g) = if opaque {
        (p[0] as u32, p[1] as u32)
    } else {
        // Un-premultiply at alpha 200.
        (
            (p[0] as u32 * 255 + 100) / 200,
            (p[1] as u32 * 255 + 100) / 200,
        )
    };
    Some(((r + 2) / 10, (g + 2) / 10))
}

/// Draw a `w` x `h` ramp through `transform` onto a 64x64 transparent
/// surface, and count how many device pixels each texel landed on.
fn texel_counts(w: u32, h: u32, opaque: bool, transform: Affine) -> Vec<Vec<u32>> {
    let data = ramp(w, h, opaque);
    let mut painter = TinySkiaPainter::new(64, 64);
    painter.fill_transparent();
    painter.draw_image(
        &PaintImage {
            data: &data,
            width: w,
            height: h,
            decoded: None,
            frame: None,
            opaque,
            sampling: ImageSampling::Nearest,
        },
        transform,
    );
    let mut counts = vec![vec![0_u32; w as usize]; h as usize];
    for p in painter.pixels().as_chunks::<4>().0 {
        if let Some((x, y)) = texel(*p, opaque) {
            counts[y as usize][x as usize] += 1;
        }
    }
    counts
}

#[test]
fn an_image_on_a_half_pixel_draws_every_texel_once() {
    // At whole pixels, then at the half pixel on each axis and both, then at
    // the fractions either side of it; opaque (the row copy, once the edges
    // are whole) and translucent (`draw_pixmap`).
    for opaque in [true, false] {
        for (tx, ty) in [
            (10.0, 10.0),
            (10.5, 10.0),
            (10.0, 10.5),
            (10.5, 10.5),
            (10.499, 10.501),
            (10.3, 10.7),
            (33.5, 7.5),
        ] {
            let counts = texel_counts(5, 4, opaque, Affine::translate((tx, ty)));
            assert!(
                counts.iter().flatten().all(|&n| n == 1),
                "opaque {opaque}, at ({tx}, {ty}): pixels per texel {counts:?}"
            );
        }
    }
}

#[test]
fn an_image_under_scale_2_on_a_half_pixel_draws_every_texel_twice_each_way() {
    for opaque in [true, false] {
        for (tx, ty) in [(10.0, 10.0), (10.5, 10.5), (10.5, 11.0), (9.25, 9.75)] {
            let counts = texel_counts(
                5,
                4,
                opaque,
                Affine::translate((tx, ty)) * Affine::scale(2.0),
            );
            assert!(
                counts.iter().flatten().all(|&n| n == 4),
                "opaque {opaque}, scale(2) at ({tx}, {ty}): pixels per texel {counts:?}"
            );
        }
    }
}

/// Where the image lands: rounded half up on each edge, as Chrome rounds a
/// replaced element's picture rect.
#[test]
fn an_image_on_a_half_pixel_rounds_its_edges_half_up() {
    let data = ramp(3, 2, true);
    let mut painter = TinySkiaPainter::new(16, 16);
    painter.fill_transparent();
    painter.draw_image(
        &PaintImage {
            data: &data,
            width: 3,
            height: 2,
            decoded: None,
            frame: None,
            opaque: true,
            sampling: ImageSampling::Nearest,
        },
        Affine::translate((4.5, 6.5)),
    );
    let px = painter.pixels().as_chunks::<4>().0;
    let inked: Vec<(u32, u32)> = (0..16 * 16)
        .filter(|&i| px[i as usize][3] != 0)
        .map(|i| (i % 16, i / 16))
        .collect();
    let want: Vec<(u32, u32)> = (7..9).flat_map(|y| (5..8).map(move |x| (x, y))).collect();
    assert_eq!(inked, want, "a 3x2 image at (4.5, 6.5) covers (5..8, 7..9)");
    assert_eq!(
        texel(px[7 * 16 + 5], true),
        Some((0, 0)),
        "texel (0, 0) first"
    );
}

/// The inked columns and rows of a `w` x `h` ramp drawn through `ts` on a
/// 32x32 transparent surface, and the texel at the inked top-left pixel.
fn landing(w: u32, h: u32, ts: Affine) -> (Vec<u32>, Vec<u32>, Option<(u32, u32)>) {
    let data = ramp(w, h, true);
    let mut painter = TinySkiaPainter::new(32, 32);
    painter.fill_transparent();
    painter.draw_image(
        &PaintImage {
            data: &data,
            width: w,
            height: h,
            decoded: None,
            frame: None,
            opaque: true,
            sampling: ImageSampling::Nearest,
        },
        ts,
    );
    let px = painter.pixels().as_chunks::<4>().0;
    let cols: Vec<u32> = (0..32)
        .filter(|&x| (0..32).any(|y| px[y * 32 + x as usize][3] != 0))
        .collect();
    let rows: Vec<u32> = (0..32_usize)
        .filter(|&y| (0..32).any(|x| px[y * 32 + x][3] != 0))
        .map(|y| y as u32)
        .collect();
    let first = match (cols.first(), rows.first()) {
        (Some(&x), Some(&y)) => texel(px[(y * 32 + x) as usize], true),
        _ => None,
    };
    (cols, rows, first)
}

/// Half up below zero too, as Chrome rounds `left: -0.5px` to 0 (a
/// round-half-away-from-zero `-0.5 -> -1` would put texel 0 off the surface).
#[test]
fn an_image_at_a_negative_half_pixel_rounds_up_to_the_pixel_above() {
    let (cols, rows, first) = landing(3, 2, Affine::translate((-0.5, -0.5)));
    assert_eq!(cols, vec![0, 1, 2], "x -0.5 covers columns 0..3");
    assert_eq!(rows, vec![0, 1], "y -0.5 covers rows 0..2");
    assert_eq!(first, Some((0, 0)), "texel (0, 0) at the surface's corner");
}

/// Each edge is rounded on its own: the far edge is `round(x0 + width)`, not
/// `round(x0) + round(width)`. Three texels at scale 0.8 from 10.3 span
/// 10.3..12.7, so columns 10..13; rounding the width alone gives 10..12.
#[test]
fn an_image_s_far_edge_is_rounded_where_it_falls() {
    let (cols, rows, _) = landing(3, 3, Affine::translate((10.3, 4.3)) * Affine::scale(0.8));
    assert_eq!(cols, vec![10, 11, 12], "x 10.3..12.7");
    assert_eq!(rows, vec![4, 5, 6], "y 4.3..6.7");
}

/// A blurred shadow's coverage mask is drawn through the text's transform
/// when that is not a translation (`text_shadow::draw_masked`): the same
/// half-pixel loss, through `draw_alpha_mask`.
#[test]
fn an_alpha_mask_on_a_half_pixel_keeps_its_first_row_and_column() {
    // A 4x3 mask whose rows are 64, 128, 255 and whose first column is 0:
    // each row's and column's coverage can be read back.
    let mask: Vec<u8> = (0..3)
        .flat_map(|y| (0..4).map(move |x| if x == 0 { 40 } else { [64, 128, 255][y] }))
        .collect();
    let draw = |transform: Affine| {
        let mut painter = TinySkiaPainter::new(32, 32);
        painter.fill_transparent();
        painter.draw_alpha_mask(&mask, 4, 3, peniko::Color::BLACK, transform);
        let px = painter.pixels().as_chunks::<4>().0.to_vec();
        // The inked rows' alphas, top to bottom, each row left to right.
        let rows: Vec<Vec<u8>> = (0..32)
            .map(|y| {
                (0..32)
                    .map(|x| px[y * 32 + x][3])
                    .filter(|&a| a != 0)
                    .collect::<Vec<u8>>()
            })
            .filter(|r| !r.is_empty())
            .collect();
        rows
    };
    let whole = draw(Affine::translate((5.0, 5.0)));
    assert_eq!(
        whole,
        vec![
            vec![40, 64, 64, 64],
            vec![40, 128, 128, 128],
            vec![40, 255, 255, 255]
        ],
        "control: the mask at a whole pixel"
    );
    for (tx, ty) in [(5.5, 5.0), (5.0, 5.5), (5.5, 5.5)] {
        assert_eq!(
            draw(Affine::translate((tx, ty))),
            whole,
            "at ({tx}, {ty}): the first row and column kept, the last not doubled"
        );
    }
    let twice = |rows: &[Vec<u8>]| -> Vec<Vec<u8>> {
        rows.iter()
            .flat_map(|r| {
                let wide: Vec<u8> = r.iter().flat_map(|&a| [a, a]).collect();
                [wide.clone(), wide]
            })
            .collect()
    };
    for (tx, ty) in [(5.0, 5.0), (5.5, 5.5), (4.25, 5.5)] {
        assert_eq!(
            draw(Affine::translate((tx, ty)) * Affine::scale(2.0)),
            twice(&whole),
            "scale(2) at ({tx}, {ty}): every mask texel 2x2"
        );
    }
}

// ── Through layout: Chrome's pixels and the issue's scenario ──────────────

/// `t.png`: 4x4, its top row red, its bottom row blue, the rest of its left
/// column green and of its right column yellow, black inside. Every edge
/// row and column is told apart.
fn edges() -> DecodedImage {
    let px: Vec<[u8; 4]> = (0..16)
        .map(|i| match (i % 4, i / 4) {
            (_, 0) => RED,
            (_, 3) => BLUE,
            (0, _) => GREEN,
            (3, _) => YELLOW,
            _ => BLACK,
        })
        .collect();
    DecodedImage::new(px.concat(), 4, 4)
}

fn paint(css: &str, html: &str, w: u32, h: u32) -> Vec<[u8; 4]> {
    let mut doc = RinchDocument::new();
    doc.load_css(css);
    doc.tree
        .image_cache
        .insert_decoded("t.png".to_string(), edges());
    let body = doc.body();
    doc.set_inner_html(body, html);
    doc.resolve_layout(w as f32, h as f32);
    let mut painter = TinySkiaPainter::new(w, h);
    painter.fill_white();
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (w as f32, h as f32),
        &mut doc.font_cx,
        &mut cx,
    );
    painter.pixels().as_chunks::<4>().0.to_vec()
}

const R: [u8; 3] = [255, 0, 0];
const G: [u8; 3] = [0, 255, 0];
const B: [u8; 3] = [0, 0, 255];
const Y: [u8; 3] = [255, 255, 0];
const K: [u8; 3] = [0, 0, 0];
const WH: [u8; 3] = [255, 255, 255];

/// Chrome 156's oracle page for these, at `--force-device-scale-factor=1`.
const CHROME_CSS: &str = "html, body { margin: 0; background: #fff; }
    .a { position: absolute; display: block; image-rendering: pixelated; }";
const CHROME_HTML: &str = r#"
<img class="a" src="t.png" style="left: 100.5px; top: 70.5px">
<img class="a" src="t.png" style="left: 120.5px; top: 70.5px; width: 8px; height: 8px">
<img class="a" src="t.png" style="left: 140.5px; top: 70.5px; width: 8px; height: 8px; padding: 3px; border: 1px solid rgb(128, 128, 128); background: rgb(255, 0, 255)">
<img class="a" src="t.png" style="left: 100.3px; top: 100.7px">
<img class="a" src="t.png" style="left: 120.7px; top: 100.3px; width: 8px; height: 8px">"#;

/// `(x0, y0, rows)`: Chrome's pixels from `(x0, y0)`, row by row.
type Region = (u32, u32, &'static [&'static [[u8; 3]]]);

/// Chrome 156's pixels for [`CHROME_HTML`].
#[rustfmt::skip]
const CHROME: &[Region] = &[
    (99, 69, &[
        &[WH, WH, WH, WH, WH, WH, WH],
        &[WH, WH, WH, WH, WH, WH, WH],
        &[WH, WH, R, R, R, R, WH],
        &[WH, WH, G, K, K, Y, WH],
        &[WH, WH, G, K, K, Y, WH],
        &[WH, WH, B, B, B, B, WH],
        &[WH, WH, WH, WH, WH, WH, WH],
    ]),
    (119, 69, &[
        &[WH, WH, WH, WH, WH, WH, WH, WH, WH, WH, WH, WH],
        &[WH, WH, WH, WH, WH, WH, WH, WH, WH, WH, WH, WH],
        &[WH, WH, R, R, R, R, R, R, R, R, WH, WH],
        &[WH, WH, R, R, R, R, R, R, R, R, WH, WH],
        &[WH, WH, G, G, K, K, K, K, Y, Y, WH, WH],
        &[WH, WH, G, G, K, K, K, K, Y, Y, WH, WH],
        &[WH, WH, G, G, K, K, K, K, Y, Y, WH, WH],
        &[WH, WH, G, G, K, K, K, K, Y, Y, WH, WH],
        &[WH, WH, B, B, B, B, B, B, B, B, WH, WH],
        &[WH, WH, B, B, B, B, B, B, B, B, WH, WH],
        &[WH, WH, WH, WH, WH, WH, WH, WH, WH, WH, WH, WH],
        &[WH, WH, WH, WH, WH, WH, WH, WH, WH, WH, WH, WH],
    ]),
    // Padded and bordered at (140.5, 70.5): the picture only. Chrome snaps
    // the box's background and border to whole pixels too; rinch paints
    // them at their fractional edges, anti-aliased, on both painters.
    (145, 75, &[
        &[R, R, R, R, R, R, R, R],
        &[R, R, R, R, R, R, R, R],
        &[G, G, K, K, K, K, Y, Y],
        &[G, G, K, K, K, K, Y, Y],
        &[G, G, K, K, K, K, Y, Y],
        &[G, G, K, K, K, K, Y, Y],
        &[B, B, B, B, B, B, B, B],
        &[B, B, B, B, B, B, B, B],
    ]),
    (99, 99, &[
        &[WH, WH, WH, WH, WH, WH, WH],
        &[WH, WH, WH, WH, WH, WH, WH],
        &[WH, R, R, R, R, WH, WH],
        &[WH, G, K, K, Y, WH, WH],
        &[WH, G, K, K, Y, WH, WH],
        &[WH, B, B, B, B, WH, WH],
        &[WH, WH, WH, WH, WH, WH, WH],
    ]),
    (119, 99, &[
        &[WH, WH, WH, WH, WH, WH, WH, WH, WH, WH, WH, WH],
        &[WH, WH, R, R, R, R, R, R, R, R, WH, WH],
        &[WH, WH, R, R, R, R, R, R, R, R, WH, WH],
        &[WH, WH, G, G, K, K, K, K, Y, Y, WH, WH],
        &[WH, WH, G, G, K, K, K, K, Y, Y, WH, WH],
        &[WH, WH, G, G, K, K, K, K, Y, Y, WH, WH],
        &[WH, WH, G, G, K, K, K, K, Y, Y, WH, WH],
        &[WH, WH, B, B, B, B, B, B, B, B, WH, WH],
        &[WH, WH, B, B, B, B, B, B, B, B, WH, WH],
        &[WH, WH, WH, WH, WH, WH, WH, WH, WH, WH, WH, WH],
        &[WH, WH, WH, WH, WH, WH, WH, WH, WH, WH, WH, WH],
        &[WH, WH, WH, WH, WH, WH, WH, WH, WH, WH, WH, WH],
    ]),
];

#[test]
fn an_img_at_a_fractional_position_draws_as_chrome_draws_it() {
    let px = paint(CHROME_CSS, CHROME_HTML, 400, 160);
    let bad = compare(&px, 400, CHROME);
    assert!(bad.is_empty(), "(point, tiny-skia, chrome): {bad:?}");
}

/// A repeating `image-rendering: pixelated` background at a half-pixel
/// phase: Chrome 156 rounds the tile grid's origin half up, so `0.5px` draws
/// as `1px` does, at natural size and at `background-size: 8px 8px`; and a
/// box at (70.5, 10.5) starts its tiles at (71, 11).
const TILE_HTML: &str = r#"
<div class="t" style="left: 10px; top: 10px; background-position: 0.5px 0.5px"></div>
<div class="t" style="left: 30px; top: 10px; background-position: 0.5px 0.5px; background-size: 8px 8px"></div>
<div class="t" style="left: 50px; top: 10px; background-position: 1px 1px"></div>
<div class="t" style="left: 70.5px; top: 10.5px"></div>"#;
const TILE_CSS: &str = "html, body { margin: 0; background: #fff; }
    .t { position: absolute; display: block; image-rendering: pixelated; width: 12px;
         height: 12px; background-image: url(t.png); }";

/// Chrome 156's pixels for [`TILE_HTML`].
#[rustfmt::skip]
const CHROME_TILES: &[Region] = &[
    (10, 10, &[
        &[B, B, B, B, B, B, B, B, B, B, B, B, WH],
        &[R, R, R, R, R, R, R, R, R, R, R, R, WH],
        &[Y, G, K, K, Y, G, K, K, Y, G, K, K, WH],
        &[Y, G, K, K, Y, G, K, K, Y, G, K, K, WH],
        &[B, B, B, B, B, B, B, B, B, B, B, B, WH],
        &[R, R, R, R, R, R, R, R, R, R, R, R, WH],
        &[Y, G, K, K, Y, G, K, K, Y, G, K, K, WH],
        &[Y, G, K, K, Y, G, K, K, Y, G, K, K, WH],
        &[B, B, B, B, B, B, B, B, B, B, B, B, WH],
        &[R, R, R, R, R, R, R, R, R, R, R, R, WH],
        &[Y, G, K, K, Y, G, K, K, Y, G, K, K, WH],
        &[Y, G, K, K, Y, G, K, K, Y, G, K, K, WH],
        &[WH, WH, WH, WH, WH, WH, WH, WH, WH, WH, WH, WH, WH],
    ]),
    (30, 10, &[
        &[B, B, B, B, B, B, B, B, B, B, B, B, WH],
        &[R, R, R, R, R, R, R, R, R, R, R, R, WH],
        &[R, R, R, R, R, R, R, R, R, R, R, R, WH],
        &[Y, G, G, K, K, K, K, Y, Y, G, G, K, WH],
        &[Y, G, G, K, K, K, K, Y, Y, G, G, K, WH],
        &[Y, G, G, K, K, K, K, Y, Y, G, G, K, WH],
        &[Y, G, G, K, K, K, K, Y, Y, G, G, K, WH],
        &[B, B, B, B, B, B, B, B, B, B, B, B, WH],
        &[B, B, B, B, B, B, B, B, B, B, B, B, WH],
        &[R, R, R, R, R, R, R, R, R, R, R, R, WH],
        &[R, R, R, R, R, R, R, R, R, R, R, R, WH],
        &[Y, G, G, K, K, K, K, Y, Y, G, G, K, WH],
        &[WH, WH, WH, WH, WH, WH, WH, WH, WH, WH, WH, WH, WH],
    ]),
    (50, 10, &[
        &[B, B, B, B, B, B, B, B, B, B, B, B, WH],
        &[R, R, R, R, R, R, R, R, R, R, R, R, WH],
        &[Y, G, K, K, Y, G, K, K, Y, G, K, K, WH],
        &[Y, G, K, K, Y, G, K, K, Y, G, K, K, WH],
        &[B, B, B, B, B, B, B, B, B, B, B, B, WH],
        &[R, R, R, R, R, R, R, R, R, R, R, R, WH],
        &[Y, G, K, K, Y, G, K, K, Y, G, K, K, WH],
        &[Y, G, K, K, Y, G, K, K, Y, G, K, K, WH],
        &[B, B, B, B, B, B, B, B, B, B, B, B, WH],
        &[R, R, R, R, R, R, R, R, R, R, R, R, WH],
        &[Y, G, K, K, Y, G, K, K, Y, G, K, K, WH],
        &[Y, G, K, K, Y, G, K, K, Y, G, K, K, WH],
        &[WH, WH, WH, WH, WH, WH, WH, WH, WH, WH, WH, WH, WH],
    ]),
    // At (70.5, 10.5): inside the box only; its fractional edges are
    // anti-aliased in rinch, snapped in Chrome.
    (71, 11, &[
        &[R, R, R, R, R, R, R, R, R, R, R],
        &[G, K, K, Y, G, K, K, Y, G, K, K],
        &[G, K, K, Y, G, K, K, Y, G, K, K],
        &[B, B, B, B, B, B, B, B, B, B, B],
        &[R, R, R, R, R, R, R, R, R, R, R],
        &[G, K, K, Y, G, K, K, Y, G, K, K],
        &[G, K, K, Y, G, K, K, Y, G, K, K],
        &[B, B, B, B, B, B, B, B, B, B, B],
        &[R, R, R, R, R, R, R, R, R, R, R],
        &[G, K, K, Y, G, K, K, Y, G, K, K],
        &[G, K, K, Y, G, K, K, Y, G, K, K],
    ]),
];

/// A pixel that differs: where, what tiny-skia drew, what Chrome drew.
type Miss = ((u32, u32), [u8; 3], [u8; 3]);

fn compare(px: &[[u8; 4]], w: u32, regions: &[Region]) -> Vec<Miss> {
    let mut bad = Vec::new();
    for &(x0, y0, rows) in regions {
        for (dy, row) in rows.iter().enumerate() {
            for (dx, &want) in row.iter().enumerate() {
                let (x, y) = (x0 + dx as u32, y0 + dy as u32);
                let p = px[(y * w + x) as usize];
                if p[..3].iter().zip(want).any(|(a, b)| a.abs_diff(b) > 2) {
                    bad.push(((x, y), [p[0], p[1], p[2]], want));
                }
            }
        }
    }
    bad
}

#[test]
fn a_pixelated_tile_at_a_half_pixel_phase_draws_as_chrome_draws_it() {
    let px = paint(TILE_CSS, TILE_HTML, 200, 60);
    let bad = compare(&px, 200, CHROME_TILES);
    assert!(bad.is_empty(), "(point, tiny-skia, chrome): {bad:?}");
}

/// The pixels painted over white, cropped to them, and the crop's corner.
fn ink(px: &[[u8; 4]], w: usize, h: usize) -> ((usize, usize), Vec<Vec<[u8; 4]>>) {
    let at = |x: usize, y: usize| px[y * w + x];
    let inked = |p: [u8; 4]| p != [255, 255, 255, 255];
    let rows: Vec<usize> = (0..h)
        .filter(|&y| (0..w).any(|x| inked(at(x, y))))
        .collect();
    let cols: Vec<usize> = (0..w)
        .filter(|&x| (0..h).any(|y| inked(at(x, y))))
        .collect();
    assert!(
        !rows.is_empty(),
        "nothing was painted; the fixture measures nothing"
    );
    let (y0, y1) = (rows[0], *rows.last().unwrap());
    let (x0, x1) = (cols[0], *cols.last().unwrap());
    let image = (y0..=y1)
        .map(|y| (x0..=x1).map(|x| at(x, y)).collect())
        .collect();
    ((x0, y0), image)
}

/// The issue's scenario: a logo `vertical-align: middle` in a cell. At a
/// cell height of 20px a 7px-tall logo (the 4x4 image at 7x7) sits on a half
/// pixel; at 19px and 21px on whole pixels either side. The logo is centred
/// horizontally in a cell 21px wide too.
fn logo_cell(height: u32, width: u32) -> String {
    format!(
        r#"<table cellspacing="0" cellpadding="0" style="margin: 10px"><tr>
           <td style="height: {height}px; width: {width}px; padding: 0; text-align: center; line-height: 0; font-size: 0"><img
           src="t.png" style="width: 7px; height: 7px; vertical-align: middle; image-rendering: pixelated"></td></tr></table>"#
    )
}

#[test]
fn a_logo_centred_on_a_half_pixel_keeps_its_top_row_and_left_column() {
    let css = "html, body { margin: 0; background: #fff; }";
    let shot = |h: u32, w: u32| {
        let px = paint(css, &logo_cell(h, w), 80, 60);
        ink(&px, 80, 60)
    };
    let (at_whole, whole) = shot(19, 20);
    let (at_half, half) = shot(20, 21);
    assert_eq!(whole.len(), 7, "control: the logo is 7 rows tall");
    assert_eq!(
        [whole[0][0][0], whole[0][0][1], whole[0][0][2]],
        [255, 0, 0],
        "control: its top-left pixel is the red top row"
    );
    assert_eq!(
        half, whole,
        "at the half pixel: the same pixels (the top row and left column were \
         dropped and the bottom row and right column drawn twice); placed at {at_half:?}, \
         the whole-pixel logo at {at_whole:?}"
    );
}

// ── #2251's review gaps: text under a transform on the software painter ──

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn paint_text(style: &str) -> ((usize, usize), Vec<Vec<[u8; 4]>>) {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    doc.load_css("html, body { margin: 0; background: #fff; }");
    let body = doc.body();
    doc.set_inner_html(
        body,
        &format!(
            r#"<div style="position: absolute; left: 60px; top: 60px; font-family: ProbeFace;
               font-size: 14px; line-height: 20px; color: #000; white-space: nowrap;
               transform-origin: 0 0; {style}">To help</div>"#
        ),
    );
    doc.resolve_layout(240.0, 240.0);
    let mut painter = TinySkiaPainter::new(240, 240);
    painter.fill_white();
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (240.0, 240.0),
        &mut doc.font_cx,
        &mut cx,
    );
    let px = painter.pixels().as_chunks::<4>().0.to_vec();
    ink(&px, 240, 240)
}

#[test]
fn rotated_skewed_and_unevenly_scaled_text_keeps_its_transform() {
    let (_, plain) = paint_text("");
    let (pw, ph) = (plain[0].len(), plain.len());
    assert!(pw > 2 * ph, "control: the line is wide ({pw}x{ph})");

    // Off the glyph atlas's grid: drawn through the transform.
    let (_, turned) = paint_text("transform: rotate(90deg)");
    let (tw, th) = (turned[0].len(), turned.len());
    assert!(
        th.abs_diff(pw) <= 2 && tw.abs_diff(ph) <= 2,
        "rotate(90deg): {tw}x{th}, the line's {pw}x{ph} turned"
    );
    // A transform with a skew term: each glyph is drawn through it, slanted.
    // The `T` leads the line: its bar's first inked column is left of its
    // stem's foot upright, and right of it under `skewX(-40deg)` (the bar is
    // 10 rows above the foot, 8px further right). A placement that dropped
    // the skew and kept the origins would draw upright glyphs along the same
    // baseline. (At 90deg the axes' scales are zero, so a rotation alone does
    // not tell: such a placement rounds to nothing and keeps the transform.)
    let lead =
        |img: &[Vec<[u8; 4]>], y: usize| img[y].iter().position(|p| p[0] < 128).unwrap() as i64;
    let upright = lead(&plain, 0) - lead(&plain, 10);
    assert!(
        upright < 0,
        "control: an upright T's bar starts left of its stem ({upright})"
    );
    let (_, slanted) = paint_text("transform: skewX(-40deg)");
    let lean = lead(&slanted, 0) - lead(&slanted, 10);
    assert!(
        lean >= 3,
        "skewX(-40deg): the T's bar starts right of its stem's foot ({lean})"
    );
    let (_, wide) = paint_text("transform: scale(2, 1)");
    let (sw, sh) = (wide[0].len(), wide.len());
    assert!(
        sw.abs_diff(2 * pw) <= 3 && sh.abs_diff(ph) <= 1,
        "scale(2, 1): {sw}x{sh}, the line's {pw}x{ph} twice as wide"
    );
}
