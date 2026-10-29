//! Review fixtures for PR #1713 (#1114): two bidi-line pieces the PR misses.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const INTER: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn doc_parts(parts: &[Option<&str>]) -> (RinchDocument, u64) {
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
        "margin: 0; font-family: ProbeFace; font-size: 16px; line-height: 40px; width: 400px",
    );
    doc.append_child(body, p);
    for part in parts {
        match part {
            Some(t) => {
                let n = doc.create_text(t);
                doc.append_child(p, n);
            }
            None => {
                let s = doc.create_element("span");
                doc.set_attribute(
                    s,
                    "style",
                    "display: inline-block; width: 20px; height: 10px",
                );
                doc.append_child(p, s);
            }
        }
    }
    doc.resolve_layout(600.0, 600.0);
    (doc, p.0)
}

/// `x[box]abc RLO def PDF`, selected from the box (flat byte 1, the box's index) to
/// "c": on a left-to-right line, and in Chrome 155 (`Range.getClientRects`
/// reports the inline-block's box), the box at the selection's start is
/// covered: `8.73..47.52`. The line holds a right-to-left run, so the PR's
/// cluster walk skips the box and starts at "a" (28.73). Base: covered.
#[test]
fn an_inline_box_at_the_start_of_a_selection_on_a_bidi_line_is_covered() {
    let (doc, p) = doc_parts(&[Some("x"), None, Some("abc \u{202E}def\u{202C}")]);
    let got = doc.query_selection_rects(p, 1, 3);
    assert_eq!(got.len(), 1, "{got:?}");
    assert!((got[0].0 - 8.73).abs() < 0.5, "box not covered: {got:?}");
    assert!((got[0].0 + got[0].2 - 47.52).abs() < 0.5, "{got:?}");
}

/// `abc RLO def ghi PDF jkl`, RLO + "d" selected: Chrome 155 paints "d" only. The
/// zero-advance RLO cluster at 32.42 opened a piece of its own, a 1px sliver.
#[test]
fn a_zero_width_control_makes_no_sliver() {
    let text = "abc \u{202E}def ghi\u{202C} jkl";
    let (doc, p) = doc_parts(&[Some(text)]);
    let got = doc.query_selection_rects(p, 4, 8);
    assert_eq!(got.len(), 1, "a 1px sliver beside \"d\": {got:?}");
    assert!((got[0].0 - 74.85).abs() < 0.5, "{got:?}");
}
