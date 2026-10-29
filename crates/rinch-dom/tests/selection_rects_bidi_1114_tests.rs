//! #1114 — a selection's highlight is one rect per **visual** piece of the
//! selection on each line, not one rect from the logical start caret to the
//! logical end caret.
//!
//! On a right-to-left run the logical end stands left of the logical start, so
//! the old start-to-end rect had a negative width and was clamped to a 1px
//! sliver; on a mixed-direction line a selected logical range is several visual
//! pieces, which no single rect describes.
//!
//! The right-to-left text is the bundled Inter's Latin letters under an
//! explicit override (`U+202E RIGHT-TO-LEFT OVERRIDE` … `U+202C POP
//! DIRECTIONAL FORMATTING`), so every number below is a property of the bundled
//! face and not of the host's Hebrew fonts. Chrome 155 lays the same markup out
//! (`<p style="width: 400px">abc &#x202E;def ghi&#x202C; jkl</p>`, the same
//! Inter, `Range.getClientRects`) as `abc ihg fed jkl`: "abc " is `0..32.42`,
//! the reversed run `32.42..84.66`, " jkl" `84.66..105.69`. The literals are
//! Chrome's, to half a pixel.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const INTER: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
/// `abc ` 0..4, RLO 4..7, `def ghi` 7..14, PDF 14..17, ` jkl` 17..21 (bytes).
/// A right-to-left paragraph, wrapped: Chrome breaks it `alpha beta` /
/// `gamma delta` / `epsilon zeta eta`, each line reversed.
const WRAPPED: &str = "\u{202E}alpha beta gamma delta epsilon zeta eta\u{202C}";
const MIXED: &str = "abc \u{202E}def ghi\u{202C} jkl";

fn doc_with(text: &str, style: &str) -> (RinchDocument, u64) {
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
        &format!("margin: 0; font-family: ProbeFace; font-size: 16px; line-height: 40px; {style}"),
    );
    doc.append_child(body, p);
    let t = doc.create_text(text);
    doc.append_child(p, t);
    doc.resolve_layout(600.0, 600.0);
    (doc, p.0)
}

/// Byte offset of the `n`th char of `text`.
fn b(text: &str, n: usize) -> usize {
    text.char_indices()
        .nth(n)
        .map(|(i, _)| i)
        .unwrap_or(text.len())
}

#[track_caller]
fn assert_rects(got: &[(f32, f32, f32, f32)], want: &[(f32, f32)], what: &str) {
    let xs: Vec<(f32, f32)> = got.iter().map(|r| (r.0, r.0 + r.2)).collect();
    assert_eq!(
        xs.len(),
        want.len(),
        "{what}: one rect per visual piece: got {got:?}, want {want:?}"
    );
    for (g, w) in xs.iter().zip(want) {
        assert!(
            (g.0 - w.0).abs() <= 0.5 && (g.1 - w.1).abs() <= 0.5,
            "{what}: piece {g:?} is not Chrome's {w:?} (all: {got:?})"
        );
    }
    for r in got {
        assert_eq!((r.1, r.3), (0.0, 40.0), "{what}: on the line box: {got:?}");
    }
}

// ── The rects (`query_selection_rects`, which the editor's highlight and the
//    link bounds read through `selection_rects_for_layout`) ───────────────

/// From "b" (left-to-right) into "de" (right-to-left): two pieces, "bc " and
/// "de" — which the reversed run draws at its right end. The old rect ran
/// from "b" to the caret before "f", `8.98..65.52`, over "ihg f" and not over
/// "de".
#[test]
fn a_selection_across_a_direction_boundary_is_two_pieces() {
    let (doc, p) = doc_with(MIXED, "width: 400px");
    let got = doc.query_selection_rects(p, b(MIXED, 1), b(MIXED, 7));
    assert_rects(&got, &[(8.98, 32.42), (65.52, 84.66)], "b..e");
}

/// From inside the right-to-left run out into " jkl": "f ghi" at the run's
/// left end, then " j" after it. The old rect ran from the start caret
/// (74.85, before "e") to the end caret (93.02, before "k"): over "e" and
/// " j", and not over "f ghi".
#[test]
fn a_selection_out_of_a_right_to_left_run_is_two_pieces() {
    let (doc, p) = doc_with(MIXED, "width: 400px");
    let got = doc.query_selection_rects(p, b(MIXED, 6), b(MIXED, 15));
    assert_rects(&got, &[(32.42, 74.86), (84.66, 93.04)], "e..j");
}

/// A selection that starts and ends inside the right-to-left run: its logical
/// end stands LEFT of its start, which the old rect clamped to 1px.
#[test]
fn a_selection_inside_a_right_to_left_run_is_its_glyphs() {
    let (doc, p) = doc_with(MIXED, "width: 400px");
    let got = doc.query_selection_rects(p, b(MIXED, 6), b(MIXED, 10));
    assert_rects(&got, &[(45.75, 74.86)], "e..g");
}

/// The whole line is one contiguous stretch, so one rect: pieces that touch
/// are one piece. (Chrome's `getClientRects` reports one box per run,
/// `0..32.42`, `32.42..84.66`, `84.66..105.69`; the painted highlight is the
/// same pixels.)
#[test]
fn a_whole_mixed_line_is_one_rect() {
    let (doc, p) = doc_with(MIXED, "width: 400px");
    let got = doc.query_selection_rects(p, 0, MIXED.len());
    assert_rects(&got, &[(0.0, 105.69)], "all");
}

/// A wrapped right-to-left paragraph, all selected: every line's rect is its
/// text, where each used to be a 1px sliver. Chrome's widths are `78.8`,
/// `97.53` and `118.75`; the first two lines end in a space, which rinch
/// highlights with the line (as it does on a left-to-right line: the range
/// runs to the line's end) — Inter's space at 16px is 4.5px.
#[test]
fn every_line_of_a_right_to_left_paragraph_is_highlighted() {
    let (doc, p) = doc_with(WRAPPED, "width: 120px");
    let got = doc.query_selection_rects(p, 0, WRAPPED.len());
    let widths: Vec<f32> = got.iter().map(|r| r.2).collect();
    assert_eq!(widths.len(), 3, "three lines: {got:?}");
    for (w, chrome) in widths.iter().zip([78.8 + 4.5, 97.53 + 4.5, 118.75]) {
        assert!(
            (w - chrome).abs() <= 0.5,
            "line width {w} is not {chrome}: {got:?}"
        );
    }
    for (n, r) in got.iter().enumerate() {
        assert_eq!((r.1, r.3), (n as f32 * 40.0, 40.0), "line {n} box: {got:?}");
    }
}

/// A selection in the middle of a right-to-left line: "ha bet" is one piece,
/// as wide as Chrome's (`51.88`). Its x is Chrome's `14.06` plus the 4.5px
/// space rinch puts at a right-to-left line's left end where Chrome hangs it
/// out of the line (a known difference, not this issue's).
#[test]
fn a_selection_in_the_middle_of_a_right_to_left_line_is_its_width() {
    let (doc, p) = doc_with(WRAPPED, "width: 120px");
    let got = doc.query_selection_rects(p, b(WRAPPED, 3), b(WRAPPED, 9));
    assert_rects(&got, &[(14.06 + 4.5, 14.06 + 4.5 + 51.88)], "ha bet");
}

/// The issue's own measurement, on whatever Hebrew face the host has: every
/// line of a selected Hebrew paragraph is wider than 20px (they were 1px), and
/// a selection inside a line is wider than 5px. Structural only, so it holds
/// on any face that has the letters.
#[test]
fn a_hebrew_paragraph_is_not_slivers() {
    const HE: &str = "שלום עולם זה טקסט בעברית ארוך מאוד";
    let (doc, p) = doc_with(HE, "width: 120px");
    let all = doc.query_selection_rects(p, 0, HE.len());
    assert!(all.len() >= 2, "the paragraph wraps: {all:?}");
    for r in &all {
        assert!(r.2 > 20.0, "a line is a sliver: {all:?}");
    }
    let mid = doc.query_selection_rects(p, b(HE, 2), b(HE, 7));
    assert_eq!(mid.len(), 1, "one piece: {mid:?}");
    assert!(mid[0].2 > 5.0, "a mid-line selection is a sliver: {mid:?}");
}

/// A selected hard break on a line holding a right-to-left run is a newline's
/// width past the line's text, as on a left-to-right line (#1706's rule,
/// `selection_rects_for_layout`): the same width on both, measured from the
/// caret before the break. Chrome 155 lays `abc &#x202E;def&#x202C;` out `0..57`.
#[test]
fn a_hard_break_after_a_right_to_left_run_is_a_newline_wide() {
    const BIDI: &str = "abc \u{202E}def\u{202C}\nxyz";
    const LTR: &str = "abc def\nxyz";
    let style = "width: 400px; white-space: pre";
    let extension = |text: &str| {
        let (doc, p) = doc_with(text, style);
        let got = doc.query_selection_rects(p, 0, text.find('\n').unwrap() + 1);
        assert_eq!(got.len(), 1, "{text:?}: one piece: {got:?}");
        let (end_x, _) = doc
            .query_caret_position(p, text.find('\n').unwrap())
            .unwrap();
        (got[0].0 + got[0].2, end_x)
    };
    let (bidi_right, bidi_text) = extension(BIDI);
    assert!((bidi_text - 57.0).abs() <= 0.5, "Chrome's 57: {bidi_text}");
    let (ltr_right, ltr_text) = extension(LTR);
    let newline = ltr_right - ltr_text;
    assert!(
        newline > 2.0,
        "the left-to-right line's newline is wide: {newline}"
    );
    assert!(
        (bidi_right - (bidi_text + newline)).abs() <= 0.01,
        "the bidi line ends a newline ({newline}) past its text ({bidi_text}): {bidi_right}"
    );
}

// ── The pixels: the read-only highlight and the textarea's ──────────────────

#[cfg(feature = "software-renderer")]
mod pixels {
    use super::*;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    const VW: u32 = 240;
    const VH: u32 = 120;

    fn paint(doc: &mut RinchDocument) -> Vec<[u8; 4]> {
        let mut painter = TinySkiaPainter::new(VW, VH);
        let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut painter,
            1.0,
            (VW as f32, VH as f32),
            &mut doc.font_cx,
            &mut layout_cx,
        );
        painter.pixels().as_chunks::<4>().0.to_vec()
    }

    /// Whether the pixel at `(x, y)` is washed (not white). The text is
    /// `color: transparent` on white, so the wash is the only ink.
    fn washed(px: &[[u8; 4]], x: u32, y: u32) -> bool {
        let p = px[(y * VW + x) as usize];
        [p[0], p[1], p[2]] != [255, 255, 255]
    }

    /// The text box sits at `(20, 20)` and its first line's box starts there
    /// (a textarea's lines are its own, not 40px, so row 26 is the one both
    /// highlights cover): "bc " is washed at
    /// x 20 + 9..32, "de" at 20 + 66..84; "ihg f" between them, at
    /// 20 + 33..65, is not. The old single rect washed `20 + 9..65` and left
    /// "de" bare.
    #[track_caller]
    fn assert_two_pieces(px: &[[u8; 4]], what: &str) {
        let y = 26;
        for x in [20 + 12, 20 + 28, 20 + 70, 20 + 80] {
            assert!(
                washed(px, x, y),
                "{what}: column {x} is selected text, not washed"
            );
        }
        for x in [20 + 40, 20 + 55, 20 + 62, 20 + 90] {
            assert!(
                !washed(px, x, y),
                "{what}: column {x} is unselected text, washed"
            );
        }
    }

    #[test]
    fn the_read_only_highlight_is_two_pieces() {
        let (mut doc, _) = doc_with("", "display: none");
        let body = doc.body();
        doc.set_attribute(body, "style", "margin: 0; background: white");
        let d = doc.create_element("div");
        doc.set_attribute(
            d,
            "style",
            "position: absolute; left: 20px; top: 20px; width: 200px; padding: 0; \
             font-family: ProbeFace; font-size: 16px; line-height: 40px; color: transparent",
        );
        doc.append_child(body, d);
        let t = doc.create_text(MIXED);
        doc.append_child(d, t);
        doc.resolve_layout(VW as f32, VH as f32);
        for (k, v) in [
            ("data-text-sel", "true".to_string()),
            ("data-text-sel-start", b(MIXED, 1).to_string()),
            ("data-text-sel-end", b(MIXED, 7).to_string()),
        ] {
            doc.set_attribute(d, k, &v);
        }
        doc.resolve_layout(VW as f32 + 1.0, VH as f32);
        assert_two_pieces(&paint(&mut doc), "read-only");
    }

    #[test]
    fn the_textarea_highlight_is_two_pieces() {
        let (mut doc, _) = doc_with("", "display: none");
        let body = doc.body();
        doc.set_attribute(body, "style", "margin: 0; background: white");
        let ta = doc.create_element("textarea");
        doc.set_attribute(
            ta,
            "style",
            "position: absolute; left: 20px; top: 20px; width: 200px; height: 60px; \
             padding: 0; border: 0; background: white; font-family: ProbeFace; \
             font-size: 16px; line-height: 40px; color: transparent",
        );
        doc.set_attribute(ta, "value", MIXED);
        doc.append_child(body, ta);
        for (k, v) in [
            ("data-focused", "true".to_string()),
            ("data-cursor-visible", "false".to_string()),
            ("data-selection-start", b(MIXED, 1).to_string()),
            ("data-cursor-pos", b(MIXED, 7).to_string()),
        ] {
            doc.set_attribute(ta, k, &v);
        }
        doc.resolve_layout(VW as f32, VH as f32);
        assert_two_pieces(&paint(&mut doc), "textarea");
    }
}
