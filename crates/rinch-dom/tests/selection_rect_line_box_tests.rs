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
//   block coords are whole pixels, 25 tall; the highlight tiles them (#1024),
//   so one rect of the four is 24.

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
    // Whole pixels, and tiled (#1024): each rect ends where the next line's
    // box begins, so a 24.75px line box is 25 rows or 24 — line 2's box is
    // `50..75` and line 3's `74..99`, so line 2 is highlighted `50..74`. The
    // last line keeps its own 25.
    let spans: Vec<(f32, f32)> = rects.iter().map(|r| (r.1, r.3)).collect();
    assert_eq!(
        spans,
        [(0.0, 25.0), (25.0, 25.0), (50.0, 24.0), (74.0, 25.0)],
        "{rects:?}"
    );
    // The caret's box is Parley's own, untouched: 25 tall.
    assert_eq!(doc.query_glyph_bounds(p, 0).unwrap().height, 25.0);
}

// ── Adjacent lines tile (#1024) ────────────────────────────────────────────
//
// At a fractional line-height Parley quantizes each line's block coords on
// its own: the top is the line's rounded `y`, the bottom the rounded ascent,
// descent and leading added to it. Two neighbours can then overlap by a pixel
// (or leave one): at 15px x 1.65 on Inter line 3 is `50..75` and line 4
// `74..99`, and a translucent highlight painted over both draws a darker seam
// along row 74. Chrome 153 snaps a 16px x 1.65 paragraph's highlights to
// `0-26, 26-53, 53-79, 79-106` — contiguous. Each line's rect ends where the
// next line's begins; the last keeps its own bottom. Several line-heights,
// because which rows collide depends on how the fractions accumulate.

fn assert_tiles(style: &str) {
    let (doc, p) = inter_paragraph(style);
    let rects = doc.query_selection_rects(p, 0, TEXT.len());
    assert!(
        rects.len() >= 4,
        "{style}: the paragraph must wrap: {rects:?}"
    );
    assert_eq!(
        rects[0].1, 0.0,
        "{style}: the first line starts at 0: {rects:?}"
    );
    for n in 0..rects.len() - 1 {
        let (_, y, _, h) = rects[n];
        assert_eq!(
            y + h,
            rects[n + 1].1,
            "{style}: line {n}'s highlight must end where line {}'s begins \
             (all rects: {rects:?})",
            n + 1
        );
        assert!(h > 0.0, "{style}: line {n} is not empty: {rects:?}");
    }
}

#[test]
fn fractional_line_height_highlights_tile_at_15px_x_1_65() {
    assert_tiles("font-size: 15px; line-height: 1.65");
}

#[test]
fn fractional_line_height_highlights_tile_at_16px_x_1_65() {
    assert_tiles("font-size: 16px; line-height: 1.65");
}

#[test]
fn fractional_line_height_highlights_tile_at_13px_x_1_37() {
    assert_tiles("font-size: 13px; line-height: 1.37");
}

#[test]
fn fractional_line_height_highlights_tile_at_a_declared_23_4px() {
    assert_tiles("font-size: 16px; line-height: 23.4px");
}

// ── A soft-wrapped line is highlighted to its end (#1010) ──────────────────
//
// A selection that runs past a soft-wrapped line's end used to take that
// line's right edge from a *downstream* caret at the line's end — which
// Parley places at the start of the NEXT line (x = 0), so the rect was
// floored to a 1px sliver at the line's left. The line is highlighted to its
// trailing edge, as a browser does.

#[test]
fn a_selection_past_a_soft_wrap_highlights_the_whole_line() {
    let (doc, p) = inter_paragraph("font-size: 16px; line-height: 40px");
    let rects = doc.query_selection_rects(p, 0, TEXT.len());
    assert!(rects.len() >= 3, "{rects:?}");
    for (n, &(x, _, w, _)) in rects.iter().enumerate() {
        assert_eq!(x, 0.0, "line {n} starts at the left edge: {rects:?}");
        // Every line of this paragraph holds at least one whole word, and no
        // word here is narrower than 20px at 16px.
        assert!(w > 20.0, "line {n} is highlighted, not a sliver: {rects:?}");
    }
}

/// Off the fixed point of a selection that starts at 0: from the middle of the
/// first line to the middle of the third. The middle line is covered from its
/// start to its trailing edge, the first from the anchor to its trailing edge.
#[test]
fn a_selection_from_mid_line_highlights_to_each_wrapped_lines_end() {
    let (doc, p) = inter_paragraph("font-size: 16px; line-height: 40px");
    let all = doc.query_selection_rects(p, 0, TEXT.len());
    // "beta" starts on the first line (after "alpha ").
    let a = TEXT.find("beta").unwrap();
    let b = TEXT.find("kappa").unwrap();
    let rects = doc.query_selection_rects(p, a, b);
    assert!(rects.len() >= 3, "{rects:?}");
    let (x0, _, w0, _) = rects[0];
    assert!(x0 > 20.0, "the first rect starts at 'beta': {rects:?}");
    // Its right edge is the first line's own right edge, the same one the
    // whole-paragraph selection reaches.
    assert_eq!(x0 + w0, all[0].0 + all[0].2, "{rects:?} vs {all:?}");
    assert_eq!(rects[1], all[1], "a middle line is covered whole");
}
