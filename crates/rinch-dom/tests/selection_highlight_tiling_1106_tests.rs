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

// ── Horizontal extents (review of #1706) ────────────────────────────────────
//
// Chrome 155 (bundled Inter through `@font-face`, `::selection` read back from
// a screenshot) ends every line's highlight at its text — a soft wrap's
// trailing space, or a newline's width before a hard break — whether or not
// the selection carries on to the next line. The two painters used to extend
// every line but the last to the content box's edge.

/// Paint a read-only `div` at (20, 20), 160px wide, holding `children` (`"<br>"`
/// is a `<br>`), with bytes `a..b` selected, at device scale `dpr`. Returns the
/// pixels and the surface width.
fn read_only(
    style: &str,
    children: &[&str],
    a: usize,
    b: usize,
    dpr: f64,
) -> (Vec<[u8; 4]>, usize) {
    let mut doc = doc();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0; background: white");
    let d = doc.create_element("div");
    doc.set_attribute(
        d,
        "style",
        &format!(
            "position: absolute; left: 20px; top: 20px; width: 160px; padding: 0; \
             font-family: ProbeFace; color: transparent; {style}"
        ),
    );
    doc.append_child(body, d);
    for c in children {
        let n = if *c == "<br>" {
            doc.create_element("br")
        } else {
            doc.create_text(c)
        };
        doc.append_child(d, n);
    }
    doc.resolve_layout(VW, VH);
    doc.set_attribute(d, "data-text-sel", "true");
    doc.set_attribute(d, "data-text-sel-start", &a.to_string());
    doc.set_attribute(d, "data-text-sel-end", &b.to_string());
    doc.resolve_layout(VW + 1.0, VH);
    paint_at(&mut doc, dpr)
}

fn paint_at(doc: &mut RinchDocument, dpr: f64) -> (Vec<[u8; 4]>, usize) {
    let (w, h) = ((VW as f64 * dpr) as u32, (VH as f64 * dpr) as u32);
    let mut painter = TinySkiaPainter::new(w, h);
    let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        dpr,
        (VW, VH),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    (painter.pixels().as_chunks::<4>().0.to_vec(), w as usize)
}

/// Every washed row's `(first, last + 1)` washed column.
fn row_spans(px: &[[u8; 4]], w: usize) -> Vec<(usize, usize, usize)> {
    px.chunks(w)
        .enumerate()
        .filter_map(|(y, row)| {
            let washed = |p: &[u8; 4]| p[..3] != [255, 255, 255];
            let first = row.iter().position(washed)?;
            let last = row.iter().rposition(washed)?;
            Some((y, first, last + 1))
        })
        .collect()
}

/// The right edge of each line's highlight, top to bottom: the rows' right
/// edges with runs of equal values folded.
fn right_edges(px: &[[u8; 4]], w: usize) -> Vec<usize> {
    let mut edges: Vec<usize> = Vec::new();
    for (_, _, right) in row_spans(px, w) {
        if edges.last() != Some(&right) {
            edges.push(right);
        }
    }
    edges
}

#[track_caller]
fn assert_edges(got: &[usize], chrome: &[f32], what: &str) {
    assert_eq!(
        got.len(),
        chrome.len(),
        "{what}: one edge per line: got {got:?}, Chrome {chrome:?}"
    );
    for (g, c) in got.iter().zip(chrome) {
        assert!(
            (*g as f32 - c).abs() <= 1.5,
            "{what}: right edges {got:?}, Chrome 155 {chrome:?}"
        );
    }
}

const LH: &str = "font-size: 16px; line-height: 1.65";

/// A selection across soft-wrapped lines: each ends at its trailing space.
/// Chrome 155: 164, 158, 174, 47 (the element's left edge at 20).
#[test]
fn a_read_only_line_the_selection_carries_past_ends_at_its_text() {
    let (px, w) = read_only(LH, &[TEXT], 6, 60, 1.0);
    assert_edges(
        &right_edges(&px, w),
        &[164.0, 158.0, 174.0, 47.0],
        "soft wraps",
    );
}

/// `abc<br><br>def`, all selected: the lines before each break carry a
/// newline's width, the empty one too. Chrome 155: 52, 25, 45.
#[test]
fn a_read_only_line_before_a_break_ends_at_a_newlines_width() {
    let (px, w) = read_only(LH, &["abc", "<br>", "<br>", "def"], 0, 8, 1.0);
    assert_edges(&right_edges(&px, w), &[52.0, 25.0, 45.0], "hard breaks");
}

/// A selection ending right after a `<br>` (`abc<br>|def`) highlights `abc`
/// and its newline. Chrome 155: 20..52. Before #1706 nothing was painted.
#[test]
fn a_read_only_selection_ending_after_a_break_highlights_the_line() {
    let (px, w) = read_only(LH, &["abc", "<br>", "def"], 0, 4, 1.0);
    let spans = row_spans(&px, w);
    assert!(spans.len() >= 20, "the line is washed: {spans:?}");
    assert!(
        spans.iter().all(|s| s.1 == 20),
        "from the line's start: {spans:?}"
    );
    assert_edges(&right_edges(&px, w), &[52.0], "after a break");
}

/// `alpha beta`, bytes 2..7, at a device scale of 1.5. Chrome 155 paints
/// x 32.67..75.33 and y 20..46.67 CSS px — 49..113 and 30..70 device px. The
/// read-only highlight added the CSS-px layout's rects to a device-px origin
/// unscaled, so it covered "pha" at 2/3 of the line's height.
#[test]
fn a_read_only_selection_is_scaled_on_a_hidpi_display() {
    let (px, w) = read_only(LH, &["alpha beta"], 2, 7, 1.5);
    let spans = row_spans(&px, w);
    let near = |g: usize, c: usize| (g as i64 - c as i64).abs() <= 2;
    let (top, bottom) = (spans[0].0, spans[spans.len() - 1].0 + 1);
    let left = spans.iter().map(|s| s.1).min().unwrap();
    let right = spans.iter().map(|s| s.2).max().unwrap();
    assert!(
        near(left, 49) && near(right, 113) && near(top, 30) && near(bottom, 70),
        "wash x {left}..{right}, y {top}..{bottom}; Chrome 155: x 49..113, y 30..70"
    );
}

fn textarea(style: &str, value: &str, a: usize, b: usize) -> (Vec<[u8; 4]>, usize) {
    let mut doc = doc();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0; background: white");
    let ta: NodeId = doc.create_element("textarea");
    doc.set_attribute(
        ta,
        "style",
        &format!(
            "position: absolute; left: 20px; top: 20px; width: 160px; height: 200px; \
             border: 0; background: white; font-family: ProbeFace; \
             font-size: 13.3333px; color: transparent; {style}"
        ),
    );
    doc.set_attribute(ta, "value", value);
    doc.append_child(body, ta);
    for (k, v) in [
        ("data-focused", "true".to_string()),
        ("data-cursor-visible", "false".to_string()),
        ("data-selection-start", a.to_string()),
        ("data-cursor-pos", b.to_string()),
    ] {
        doc.set_attribute(ta, k, &v);
    }
    doc.resolve_layout(VW, VH);
    paint_at(&mut doc, 1.0)
}

/// `abc\n\ndef\nghi`, bytes 0..9, in a textarea with 2px padding: each line
/// ends at a newline's width past its text, the selection's last as well as
/// the ones it carries past. Chrome 155 with a 1px border paints 50, 27, 48;
/// this textarea has none (rinch draws a control's text without its border,
/// a separate issue), so 49, 26, 47.
#[test]
fn a_textarea_line_before_a_newline_ends_at_a_newlines_width() {
    let (px, w) = textarea("padding: 2px", "abc\n\ndef\nghi", 0, 9);
    assert_edges(
        &right_edges(&px, w),
        &[49.0, 26.0, 47.0],
        "textarea newlines",
    );
}

/// The highlight starts at the textarea's padding edge: with `padding: 7px`
/// at x = 20, the wash's leftmost column is 27.
#[test]
fn a_textarea_selection_starts_inside_its_padding() {
    let (px, w) = textarea("padding: 7px", TEXT, 0, TEXT.len());
    let spans = row_spans(&px, w);
    assert!(spans.len() > 40, "several lines: {}", spans.len());
    let left = spans.iter().map(|s| s.1).min().unwrap();
    assert_eq!(left, 27, "the wash's leftmost column");
    assert_eq!(spans[0].0, 27, "and its top row");
}
