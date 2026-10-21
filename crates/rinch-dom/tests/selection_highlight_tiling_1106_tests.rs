//! #1106 — the read-only (`user-select: text`) selection highlight and the
//! `<textarea>` selection highlight tile their lines: no gap and no overlap
//! between two lines' translucent rects (the sibling of the editor's #1024).
//!
//! Both painters used to derive their own line boxes. The read-only one put
//! each rect at `baseline - ascent - half_leading` with the line's fractional
//! `line_height`, while Parley rounds each line's position on its own, so at a
//! fractional line-height two neighbours overlapped by a pixel (a darker row
//! across the selection) or left one unpainted. The textarea's rects were
//! `1.2 × font-size` tall from each line's glyph top, whatever the line box
//! was. Both now go through `text_query::selection_rects_for_layout`, whose
//! rects end where the next line's box begins (Chrome's highlights are
//! contiguous: 16px × 1.65 paints `0-26, 26-53, 53-79, 79-106`).
//!
//! A local pixel oracle: the text is `color: transparent` on white, so in the
//! highlight's leftmost column the only ink is the wash, and every row of it
//! between its first and last must be the same colour. The face is the bundled
//! Inter under a private family name, and every size is declared, so which
//! rows collide is not a property of the host's fonts.
#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const VW: f32 = 400.0;
const VH: f32 = 400.0;
const TEXT: &str = "alpha beta gamma delta epsilon zeta eta theta iota kappa \
                    lambda mu nu xi omicron pi rho sigma tau upsilon";

fn doc() -> RinchDocument {
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
    doc
}

fn paint(doc: &mut RinchDocument) -> Vec<[u8; 4]> {
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter.pixels().as_chunks::<4>().0.to_vec()
}

/// Every non-white row of column `x`, top to bottom, as `(y, rgb)`.
fn column(px: &[[u8; 4]], x: usize) -> Vec<(usize, [u8; 3])> {
    (0..VH as usize)
        .map(|y| {
            let p = px[y * VW as usize + x];
            (y, [p[0], p[1], p[2]])
        })
        .filter(|(_, c)| *c != [255, 255, 255])
        .collect()
}

/// The wash in column `x` is one contiguous run, every row of it (but the two
/// edge rows, which may be anti-aliased) one colour, and at least `min_rows`
/// tall — several lines.
#[track_caller]
fn assert_even_wash(px: &[[u8; 4]], x: usize, min_rows: usize, what: &str) {
    let rows = column(px, x);
    assert!(
        rows.len() >= min_rows,
        "{what}: the wash is painted over several lines ({} rows): {rows:?}",
        rows.len()
    );
    let (top, bottom) = (rows[0].0, rows[rows.len() - 1].0);
    assert_eq!(
        bottom - top + 1,
        rows.len(),
        "{what}: a white row inside the wash — two lines' highlights leave a gap: {rows:?}"
    );
    let inner = &rows[1..rows.len() - 1];
    let first = inner[0].1;
    for (y, c) in inner {
        assert_eq!(
            *c, first,
            "{what}: row {y} is {c:?}, the rest {first:?} — two lines' highlights \
             overlap there (darker) or leave a gap (lighter): {rows:?}"
        );
    }
}

fn read_only_wash(font: &str) -> Vec<[u8; 4]> {
    let mut doc = doc();
    let body = doc.body();
    let d = doc.create_element("div");
    doc.set_attribute(
        d,
        "style",
        &format!(
            "position: absolute; left: 20px; top: 20px; width: 160px; padding: 0; \
             font-family: ProbeFace; {font}; color: transparent"
        ),
    );
    doc.append_child(body, d);
    let t = doc.create_text(TEXT);
    doc.append_child(d, t);
    let body_style = "margin: 0; background: white";
    doc.set_attribute(body, "style", body_style);
    doc.resolve_layout(VW, VH);
    for (k, v) in [
        ("data-text-sel", "true".to_string()),
        ("data-text-sel-start", "0".to_string()),
        ("data-text-sel-end", TEXT.len().to_string()),
    ] {
        doc.set_attribute(d, k, &v);
    }
    doc.resolve_layout(VW + 1.0, VH);
    paint(&mut doc)
}

#[test]
fn a_read_only_selection_has_no_seam_at_15px_x_1_65() {
    assert_even_wash(
        &read_only_wash("font-size: 15px; line-height: 1.65"),
        21,
        80,
        "15px x 1.65",
    );
}

#[test]
fn a_read_only_selection_has_no_seam_at_16px_x_1_65() {
    assert_even_wash(
        &read_only_wash("font-size: 16px; line-height: 1.65"),
        21,
        80,
        "16px x 1.65",
    );
}

#[test]
fn a_read_only_selection_has_no_seam_at_13px_x_1_37() {
    assert_even_wash(
        &read_only_wash("font-size: 13px; line-height: 1.37"),
        21,
        60,
        "13px x 1.37",
    );
}

fn textarea_wash(font_size: &str) -> Vec<[u8; 4]> {
    let mut doc = doc();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0; background: white");
    let ta: NodeId = doc.create_element("textarea");
    doc.set_attribute(
        ta,
        "style",
        &format!(
            "position: absolute; left: 20px; top: 20px; width: 160px; height: 300px; \
             padding: 0; border: 0; background: white; font-family: ProbeFace; \
             font-size: {font_size}; color: transparent"
        ),
    );
    doc.set_attribute(ta, "value", TEXT);
    doc.append_child(body, ta);
    for (k, v) in [
        ("data-focused", "true".to_string()),
        ("data-cursor-visible", "false".to_string()),
        ("data-selection-start", "0".to_string()),
        ("data-cursor-pos", TEXT.len().to_string()),
    ] {
        doc.set_attribute(ta, k, &v);
    }
    doc.resolve_layout(VW, VH);
    paint(&mut doc)
}

#[test]
fn a_textarea_selection_has_no_seam_at_13px() {
    assert_even_wash(&textarea_wash("13px"), 21, 60, "textarea 13px");
}

#[test]
fn a_textarea_selection_has_no_seam_at_15px() {
    assert_even_wash(&textarea_wash("15px"), 21, 60, "textarea 15px");
}

#[test]
fn a_textarea_selection_has_no_seam_at_17px() {
    assert_even_wash(&textarea_wash("17px"), 21, 60, "textarea 17px");
}
