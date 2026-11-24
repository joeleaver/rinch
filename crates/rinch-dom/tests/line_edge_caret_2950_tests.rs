//! #2950: a caret at a line's logical start or end, beside text whose
//! direction is not the paragraph's, is drawn at the line's edge in the
//! paragraph's direction, and a press at that edge (or past it) puts the
//! caret there, as Chrome does. Before, both used the run's own edge: in an
//! rtl paragraph `abc`'s offset 0 was at the `a`'s left, inside the line.
//!
//! Chrome 158, headless, the bundled Inter at a declared `font: 16px/20px`,
//! a 400px `contenteditable` div at the origin. Carets are read off a
//! screenshot (a red `caret-color`; Chrome draws its 1px caret held inside
//! the box, so the right edge 400 shows as column 399); hits are
//! `caretRangeFromPoint` at `y = 10`. `abc` is 27.92px wide, so it inks
//! 372.08..400 right-aligned. No expected number depends on a Hebrew glyph's
//! width (Inter has none, so the host's fallback font draws them).
//!
//! | case | Chrome |
//! |---|---|
//! | `dir=rtl` `abc`: caret at 0 / 1 / 3 | 399 / 381 / 372 |
//! | `dir=rtl` `abc`: hit at x 10 / 375 / 399 | 3 / 3 / 0 |
//! | `dir=ltr` `אבג`: caret at 0 / end | 0 / the Hebrew's right edge |
//! | `dir=ltr` `אבג`: hit at x 1 / 300 | 0 / end |
//! | `dir=rtl` `abc אב`: caret at 0, hit at x 10 | 399 (the line's right), end |
//! | `dir=rtl` `אב abc`: caret at the end, hit at x 399 | the line's left, 0 |

use rinch_core::dom::{CaretAffinity, DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn doc(dir: &str, text: &str) -> (RinchDocument, NodeId) {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut d = RinchDocument::new();
    d.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    d.load_css("body { margin: 0 }");
    let body = d.body();
    let c = d.create_element("div");
    d.set_attribute(c, "dir", dir);
    d.set_attribute(c, "style", "font:16px/20px ProbeFace;width:400px");
    d.append_child(body, c);
    d.set_inner_html(c, text);
    d.resolve_layout(800.0, 600.0);
    (d, c)
}

fn caret_x(d: &RinchDocument, c: NodeId, byte: usize, a: CaretAffinity) -> f32 {
    d.query_caret_position_with_affinity(c.0, byte, a)
        .unwrap()
        .0
}

/// The byte a press at `x` on the first line puts the caret at, by both of
/// rinch's hit tests: the one by point (`grapheme_offset_from_position`,
/// through parley's `Cursor::from_point`) and the one by line
/// (`caret_on_line`, which a drag and a press inside a field use).
fn hit(d: &RinchDocument, c: NodeId, x: f32) -> usize {
    let il = d.tree.get(c.0).unwrap().text_layout.as_ref().unwrap();
    let text = il.text_content.as_str();
    let by_point = rinch_dom::text_query::grapheme_offset_from_position(&il.layout, text, x, 10.0);
    let (by_line, _) = rinch_dom::text_query::caret_on_line(&il.layout, text, 0, x).unwrap();
    assert_eq!(by_point, by_line, "x {x}: the two hit tests agree");
    by_point
}

/// The line's ink: the left of its first glyph run, the right of its last.
fn ink(d: &RinchDocument, c: NodeId) -> (f32, f32) {
    let il = d.tree.get(c.0).unwrap().text_layout.as_ref().unwrap();
    let line = il.layout.lines().next().unwrap();
    let (mut l, mut r) = (f32::MAX, f32::MIN);
    for item in line.items() {
        if let parley::layout::PositionedLayoutItem::GlyphRun(g) = item {
            l = l.min(g.offset());
            r = r.max(g.offset() + g.advance());
        }
    }
    (l, r)
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() <= 0.5
}

#[test]
fn latin_in_an_rtl_paragraph_has_its_first_and_last_carets_at_the_lines_edges() {
    let (d, c) = doc("rtl", "abc");
    let (x0, x1, x3) = (
        caret_x(&d, c, 0, CaretAffinity::Downstream),
        caret_x(&d, c, 1, CaretAffinity::Downstream),
        caret_x(&d, c, 3, CaretAffinity::Upstream),
    );
    // Chrome: 399 (the right edge, 400, its caret held inside), 381, 372.
    assert!(
        near(x0, 400.0) && near(x1, 381.4) && near(x3, 372.08),
        "carets {x0} {x1} {x3}"
    );
    assert_eq!(
        (hit(&d, c, 10.0), hit(&d, c, 375.0), hit(&d, c, 399.0)),
        (3, 3, 0)
    );
}

#[test]
fn hebrew_in_an_ltr_paragraph_has_its_first_and_last_carets_at_the_lines_edges() {
    let he = "\u{5d0}\u{5d1}\u{5d2}";
    let (d, c) = doc("ltr", he);
    let (left, right) = ink(&d, c);
    assert!(near(left, 0.0), "{left}");
    let (x0, x6) = (
        caret_x(&d, c, 0, CaretAffinity::Downstream),
        caret_x(&d, c, 6, CaretAffinity::Upstream),
    );
    assert!(
        near(x0, 0.0) && near(x6, right),
        "carets {x0} {x6}, ink to {right}"
    );
    assert_eq!((hit(&d, c, 1.0), hit(&d, c, 300.0)), (0, 6));
}

#[test]
fn a_mixed_line_takes_the_paragraphs_edges_too() {
    let (d, c) = doc("rtl", "abc \u{5d0}\u{5d1}");
    let x0 = caret_x(&d, c, 0, CaretAffinity::Downstream);
    assert!(near(x0, 400.0), "the line's right: {x0}");
    assert_eq!(hit(&d, c, 10.0), 8, "left of the line: its end");

    let (d, c) = doc("rtl", "\u{5d0}\u{5d1} abc");
    let (left, _) = ink(&d, c);
    let end = caret_x(&d, c, 8, CaretAffinity::Upstream);
    assert!(near(end, left), "the line's left: {end}, ink from {left}");
    assert_eq!(hit(&d, c, 399.0), 0);
}

/// Text in the paragraph's own direction is as it was: `abc` in a ltr
/// paragraph has offset 0 at the left and 3 at its right.
#[test]
fn text_in_the_paragraphs_direction_is_unchanged() {
    let (d, c) = doc("ltr", "abc");
    assert!(near(caret_x(&d, c, 0, CaretAffinity::Downstream), 0.0));
    assert!(near(caret_x(&d, c, 3, CaretAffinity::Upstream), 27.92));
    assert_eq!((hit(&d, c, 1.0), hit(&d, c, 300.0)), (0, 3));
}
