//! A text selection's highlight covers the **line box**, the same box the caret
//! stands in (#1008).
//! (Under a line-height smaller than the text's content area that box is the
//! content area, taller than the line box — see the negative-leading fixture
//! at the end.)
//!
//! `selection_rects_for_layout` used to put each rect's top at
//! `baseline - ascent` with the height of the whole line box. That top is the
//! line box's top **plus the half-leading** — `(line-height - (ascent +
//! descent)) / 2` — so the highlight hung that far below the line and poked the
//! same amount into the next one, while the caret (Parley's
//! `Cursor::geometry`, `block_min_coord..block_max_coord`) sat on the line box.
//! A browser paints the selection over the line box.
//!
//! # Why these numbers
//!
//! The line box is **declared**: `line-height: 40px` at `font-size: 16px`, so
//! line *n*'s box is `40n..40(n + 1)` whatever the face, and every literal
//! below is a statement, not a font measurement. The leading is deliberately
//! large — a line-height near the face's own `ascent + descent` would put the
//! half-leading near zero, where the broken and fixed tops agree (the fixed
//! point). And the fixture runs on **two** bundled faces with different
//! vertical metrics (Inter, Space Grotesk), each registered under an override
//! family name so the host's fonts cannot answer instead: the half-leading is
//! per face, the line box is not.
//!
//! The second line is asserted as well as the first, so a fix that zeroes the
//! top (`0.0`) instead of reading the line's own top is caught.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const INTER: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const GROTESK: &[u8] = include_bytes!("../assets/fonts/SpaceGrotesk-VariableFont_wght.ttf");

const LINE: f32 = 40.0;
const TEXT: &str = "alpha beta gamma delta epsilon zeta eta theta iota kappa";

/// A `width: 120px` paragraph of `TEXT` set in `face`, laid out: returns the
/// document and the paragraph (the IFC root the rects are asked of).
fn paragraph(face: &'static [u8]) -> (RinchDocument, u64) {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(face)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    let body = doc.body();
    let p = doc.create_element("p");
    doc.set_attribute(
        p,
        "style",
        "margin: 0; width: 120px; font-family: ProbeFace; font-size: 16px; \
         line-height: 40px",
    );
    doc.append_child(body, p);
    let t = doc.create_text(TEXT);
    doc.append_child(p, t);
    doc.resolve_layout(400.0, 600.0);
    (doc, p.0 as u64)
}

fn assert_on_line_boxes(face: &'static [u8], name: &str) {
    let (doc, p) = paragraph(face);
    let rects = doc.query_selection_rects(p, 0, TEXT.len());
    assert!(
        rects.len() >= 2,
        "{name}: the paragraph must wrap, or the second-line check tests nothing: {rects:?}"
    );
    for (n, &(_, y, _, h)) in rects.iter().enumerate() {
        assert_eq!(
            (y, h),
            (n as f32 * LINE, LINE),
            "{name}: line {n}'s highlight must cover its line box {}..{} \
             (all rects: {rects:?})",
            n as f32 * LINE,
            (n + 1) as f32 * LINE,
        );
    }
    // The caret at the start of the second line stands in the same box.
    let second = rects[1];
    let start_of_second = TEXT
        .char_indices()
        .map(|(i, _)| i)
        .find(|&i| {
            doc.query_caret_position(p, i)
                .is_some_and(|(_, cy)| cy >= LINE)
        })
        .expect("a caret on the second line");
    let (_, caret_y) = doc.query_caret_position(p, start_of_second).unwrap();
    assert_eq!(caret_y, second.1, "{name}: highlight and caret share a top");
    // And the glyph bounds the caret takes its height from describe that box.
    let g = doc.query_glyph_bounds(p, start_of_second).unwrap();
    assert_eq!(
        (g.y, g.height),
        (LINE, LINE),
        "{name}: glyph bounds on the line box"
    );
}

#[test]
fn the_highlight_covers_the_line_box_on_inter() {
    assert_on_line_boxes(INTER, "Inter");
}

#[test]
fn the_highlight_covers_the_line_box_on_space_grotesk() {
    assert_on_line_boxes(GROTESK, "Space Grotesk");
}

// ── The height, off the fixed point (review of #1016) ─────────────────────
//
// At `line-height: 40px` the line box's height (`metrics.line_height`) and
// Parley's `block_max_coord - block_min_coord` agree, so the fixtures above
// cannot tell which one a rect's height is taken from. They differ in two
// cases, and these pin both on the bundled Inter:
//
// - **negative leading** — a line-height smaller than the face's content area.
//   Parley clamps the leading at 0 for the block coords, so each rect is the
//   content area (rounded ascent + descent), centred on its line box and
//   overlapping the neighbouring lines. Chrome 153 paints exactly that for this
//   paragraph at `font-size: 16px; line-height: 12px`: line n's highlight is
//   `12n - 4 .. 12n + 16`, measured by the review from a screenshot with a
//   translucent `::selection`.
// - **a fractional line-height** — 15px at 1.65 is a 24.75px line box, and the
//   block coords are whole pixels, 25 tall.

/// `TEXT` in a `width: 120px` paragraph of the bundled Inter at `style`.
fn inter_paragraph(style: &str) -> (RinchDocument, u64) {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(INTER)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    let body = doc.body();
    let p = doc.create_element("p");
    doc.set_attribute(
        p,
        "style",
        &format!("margin: 0; width: 120px; font-family: ProbeFace; {style}"),
    );
    doc.append_child(body, p);
    let t = doc.create_text(TEXT);
    doc.append_child(p, t);
    doc.resolve_layout(400.0, 600.0);
    (doc, p.0 as u64)
}

#[test]
fn a_negative_leading_highlight_is_the_content_area_as_in_chrome() {
    let (doc, p) = inter_paragraph("font-size: 16px; line-height: 12px");
    let rects = doc.query_selection_rects(p, 0, TEXT.len());
    assert!(rects.len() >= 3, "{rects:?}");
    for (n, &(_, y, _, h)) in rects.iter().enumerate() {
        assert_eq!((y, h), (12.0 * n as f32 - 4.0, 20.0), "line {n}: {rects:?}");
    }
    // The caret takes its height from the glyph bounds, and its top from
    // Parley's caret box, which is this same content area.
    let g = doc.query_glyph_bounds(p, 0).unwrap();
    assert_eq!((g.y, g.height), (-4.0, 20.0), "glyph bounds on line 0");
    assert_eq!(doc.query_caret_position(p, 0).map(|c| c.1), Some(-4.0));
}

#[test]
fn a_fractional_line_height_highlight_is_whole_pixels_tall() {
    let (doc, p) = inter_paragraph("font-size: 15px; line-height: 1.65");
    let rects = doc.query_selection_rects(p, 0, TEXT.len());
    assert!(rects.len() >= 3, "{rects:?}");
    for (n, &(_, _, _, h)) in rects.iter().enumerate() {
        assert_eq!(h, 25.0, "line {n}: {rects:?}");
    }
    assert_eq!(doc.query_glyph_bounds(p, 0).unwrap().height, 25.0);
}
