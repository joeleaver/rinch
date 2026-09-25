//! A text selection's highlight covers the **line box**, the same box the caret
//! stands in (#1008).
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
