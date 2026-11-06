//! A glyph placed on a half pixel lost its first row and its first column on
//! the software painter.
//!
//! `TinySkiaPainter::draw_glyphs` rasterises a glyph once, at the origin, and
//! places the coverage image by translating it with nearest-neighbour
//! sampling (see `CachedGlyph` in `paint/skia_painter.rs`). tiny-skia's
//! `draw_pixmap` fills the image's rectangle and samples the image at each
//! covered pixel's centre. At a translation of exactly `k + 0.5` the two
//! disagree: the rectangle covers pixels `k + 1 ..`, but the centre of pixel
//! `k + 1` lands on the boundary between the image's rows 0 and 1, and the
//! sample takes row 1. Row 0 is never drawn and the last row is drawn twice.
//! Horizontally the same happens to column 0. Measured on tiny-skia 0.12 with
//! a ten-row image: at `ty = 454.5` the rows drawn are 1..=9 then 9 again.
//!
//! On text that reads as flat-cut tops: the bar of a `T` gone ("To" reads
//! "Io"), lowercase letters whose x-height row is missing, and a sliver off
//! the left of a line's first glyph. Layout puts a box on a half pixel
//! wherever centring splits an odd amount of space, as a table cell's
//! `vertical-align: middle` does. parley 0.12 changed enough line heights in
//! real mail for that to start happening to whole footers.
//!
//! The oracle compares a glyph's drawn image with itself. The same text is
//! drawn at an integral position and at a half-pixel one. Its inked rows,
//! cropped to the ink, must be the same pixels shifted by whole pixels. That
//! is independent of the face, and it is what nearest-neighbour placement
//! promises. The face is the bundled Inter, registered under an override name
//! so the host's fonts cannot answer, and the line box is declared.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: u32 = 240;
const VH: u32 = 80;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// `T` has a flat bar on its first row; `o`'s round top sits on the x-height
/// row. Both lose that row to the bug.
const TEXT: &str = "To help";

const FONT: &str = "font-family: ProbeFace; font-size: 14px; line-height: 20px; \
                    color: rgb(0, 0, 0)";

fn paint(html: &str) -> Vec<u8> {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    let body = doc.body();
    doc.set_inner_html(body, html);
    doc.resolve_layout(VW as f32, VH as f32);
    let mut painter = TinySkiaPainter::new(VW, VH);
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (VW as f32, VH as f32),
        &mut doc.font_cx,
        &mut cx,
    );
    painter
        .pixels()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| p[3])
        .collect()
}

/// The alpha image cropped to its ink, and the crop's top-left corner.
fn ink(alpha: &[u8]) -> ((usize, usize), Vec<Vec<u8>>) {
    let (w, h) = (VW as usize, VH as usize);
    let at = |x: usize, y: usize| alpha[y * w + x];
    let rows: Vec<usize> = (0..h).filter(|&y| (0..w).any(|x| at(x, y) != 0)).collect();
    let cols: Vec<usize> = (0..w).filter(|&x| (0..h).any(|y| at(x, y) != 0)).collect();
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

/// A cell 20px tall holds the 20px line exactly; at 21px, `vertical-align:
/// middle` puts the line 0.5px down, and at 22px 1px down.
fn cell(height: u32) -> String {
    format!(
        r#"<table cellspacing="0" cellpadding="0" style="margin: 10px"><tr>
           <td style="height: {height}px; padding: 0; {FONT}">{TEXT}</td></tr></table>"#
    )
}

#[test]
fn a_line_centred_on_a_half_pixel_keeps_its_glyphs_top_row() {
    let (at_0, whole) = ink(&paint(&cell(20)));
    let (at_1, one_down) = ink(&paint(&cell(22)));
    let (at_half, half_down) = ink(&paint(&cell(21)));
    // Control: a whole-pixel move moves the ink and changes nothing else.
    assert_eq!(at_1.1, at_0.1 + 1, "22px cell: one row down");
    assert_eq!(one_down, whole, "22px cell: the same pixels");
    // The half pixel rounds down to the next whole row, as any other
    // fraction rounds to the nearest: the same pixels, one row down.
    assert_eq!(
        at_half.1,
        at_0.1 + 1,
        "21px cell: the half pixel rounds down to row +1"
    );
    assert_eq!(
        half_down.len(),
        whole.len(),
        "21px cell: as many inked rows as at a whole pixel"
    );
    assert_eq!(
        half_down, whole,
        "21px cell: the glyphs drawn at the half pixel are the same pixels \
         (the first row of each glyph was dropped and its last drawn twice)"
    );
}

/// An inline-block column 100px wide centred in a cell `width` wide: at
/// 121px it starts 10.5px in, and its first glyph with it. One glyph, `E`
/// (its stem is its first column): the glyphs after a line's first sit at
/// fractional offsets of their own, which a half-pixel shift rounds either
/// way, so only the first is the same pixels at every origin.
fn column(width: u32) -> String {
    format!(
        r#"<table cellspacing="0" cellpadding="0" style="margin: 10px"><tr>
           <td style="width: {width}px; padding: 0; text-align: center; {FONT}"><div
           style="display: inline-block; width: 100px; text-align: left">E</div></td></tr></table>"#
    )
}

#[test]
fn a_column_centred_on_a_half_pixel_keeps_its_first_glyphs_left_column() {
    let (at_0, whole) = ink(&paint(&column(120)));
    let (at_1, one_right) = ink(&paint(&column(122)));
    let (at_half, half_right) = ink(&paint(&column(121)));
    assert_eq!(at_1.0, at_0.0 + 1, "122px cell: one column right");
    assert_eq!(one_right, whole, "122px cell: the same pixels");
    assert_eq!(
        at_half.0,
        at_0.0 + 1,
        "121px cell: the half pixel rounds to column +1"
    );
    assert_eq!(
        half_right, whole,
        "121px cell: the glyph drawn at the half pixel is the same pixels \
         (its first column was dropped and its last drawn twice)"
    );
}
