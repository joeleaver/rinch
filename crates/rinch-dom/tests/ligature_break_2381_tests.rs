//! A line breaks inside a ligature where Chrome does, and both sides are
//! shaped again on their own (#2381).
//!
//! The bundled Inter shapes `->` as one arrow glyph (its `calt` feature), one
//! cluster of two characters, and Blink's ASCII pair table has an opportunity
//! between `-` and `>` (rinch hands parley that table since #2369). parley
//! broke lines only between clusters, so the arrow was never broken. Chrome
//! breaks there and reshapes each side: `a->b` in a narrow box is `a-` /
//! `>b`, drawn with a hyphen and a greater-than sign. The parley fork splits
//! such a cluster at the opportunity into pieces (`ShapedText::ligature_splits`):
//! joined they draw the ligature, and a line that ends between them draws
//! each side shaped on its own.
//!
//! Every number is Chrome 157's, on the same markup: the bundled Inter
//! registered as `ProbeFace`, `font: 16px/20px`, `* { margin: 0 }`, lines read
//! from `Range.getClientRects` per character and their widths from the
//! element's own rects, min-content widths from an `inline-block` with
//! `width: min-content`.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

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
    doc.load_css("* { margin: 0; padding: 0 } body { font: 16px/20px ProbeFace }");
    doc
}

/// A `<div>` with `style` holding the one text `text`, laid out.
fn lay_out(style: &str, text: &str) -> (RinchDocument, NodeId) {
    let mut d = doc();
    let body = d.body();
    let c = d.create_element("div");
    d.set_attribute(c, "style", &format!("font: 16px/20px ProbeFace; {style}"));
    d.append_child(body, c);
    let t = d.create_text(text);
    d.append_child(c, t);
    d.resolve_layout(800.0, 600.0);
    (d, c)
}

/// Each line's text, its advance and the number of glyphs it draws.
fn lines(width: f32, text: &str) -> Vec<(String, f32, usize)> {
    let (d, c) = lay_out(&format!("width: {width}px"), text);
    let n = d.tree.get(c.0).unwrap();
    let il = n.text_layout.as_ref().expect("an IFC root");
    il.layout
        .lines()
        .map(|l| {
            let glyphs = l
                .items()
                .map(|item| match item {
                    parley::layout::PositionedLayoutItem::GlyphRun(g) => g.glyphs().count(),
                    parley::layout::PositionedLayoutItem::InlineBox(_) => 0,
                })
                .sum();
            (
                il.text_content[l.text_range()].to_string(),
                l.metrics().advance,
                glyphs,
            )
        })
        .collect()
}

/// The texts of `lines`.
fn texts(lines: &[(String, f32, usize)]) -> Vec<&str> {
    lines.iter().map(|(t, _, _)| t.as_str()).collect()
}

#[track_caller]
fn close(got: f32, chrome: f32, what: &str) {
    assert!(
        (got - chrome).abs() < 0.05,
        "{what}: rinch {got}, Chrome 157 {chrome}"
    );
}

/// The issue's rows, at every width Chrome breaks them: one line where the
/// whole fits, else broken after the hyphen, each side as wide as it is on
/// its own (the arrow is 15.27px, `-` 7.36 and `>` 10.59).
#[test]
fn an_arrow_breaks_after_its_hyphen_as_in_chrome() {
    // (text, widths that break it, its two lines' widths, its one-line width)
    let rows: [(&str, &[f32], [f32; 2], f32); 2] = [
        (
            "a->b",
            &[1.0, 16.0, 20.0, 25.0, 30.0],
            [16.3438, 20.3906],
            34.0469,
        ),
        ("1->1", &[1.0, 20.0, 25.0], [13.875, 17.0938], 28.2813),
    ];
    for (text, widths, [first, second], whole) in rows {
        let (head, tail) = text.split_at(2);
        for &w in widths {
            let got = lines(w, text);
            assert_eq!(texts(&got), [head, tail], "{text:?} in {w}px");
            close(got[0].1, first, &format!("{text:?} in {w}px, line 1"));
            close(got[1].1, second, &format!("{text:?} in {w}px, line 2"));
            // `-` and `>` each draw a glyph of their own.
            assert_eq!(got[0].2, 2, "{text:?} in {w}px draws {head:?}");
            assert_eq!(got[1].2, 2, "{text:?} in {w}px draws {tail:?}");
        }
        // Where it fits, the arrow is one glyph again.
        let got = lines(40.0, text);
        assert_eq!(texts(&got), [text]);
        close(got[0].1, whole, &format!("{text:?} on one line"));
        assert_eq!(got[0].2, 3, "{text:?} draws the arrow");
    }
}

/// With an earlier opportunity, a line still ends inside the arrow when it
/// fits there with the hyphen alone: `xx a-` is 38.31 in 40px, where the
/// joined `xx a→` would be 41.7.
#[test]
fn a_line_ends_inside_the_arrow_where_it_fits_reshaped() {
    let got = lines(40.0, "xx a->b");
    assert_eq!(texts(&got), ["xx a-", ">b"]);
    close(got[0].1, 38.3125, "line 1");
    close(got[1].1, 20.3906, "line 2");
    assert_eq!(texts(&lines(1.0, "xx a->b")), ["xx ", "a-", ">b"]);
    assert_eq!(texts(&lines(30.0, "xx a->b")), ["xx ", "a-", ">b"]);
    assert_eq!(texts(&lines(60.0, "xx a->b")), ["xx a->b"]);
}

/// Chrome does not reshape at min-content: the arrow counts whole with the
/// text after the break (`a->b` is the arrow and `b`, 25.06, not `>b`'s
/// 20.39).
#[test]
fn min_content_counts_the_arrow_with_the_text_after_it() {
    for (text, chrome) in [("a->b", 25.0625), ("1->1", 21.7656), ("xx a->b", 25.0625)] {
        let mut d = doc();
        let body = d.body();
        let ib = d.create_element("div");
        d.set_attribute(
            ib,
            "style",
            "display: inline-block; width: min-content; font: 16px/20px ProbeFace",
        );
        d.append_child(body, ib);
        let t = d.create_text(text);
        d.append_child(ib, t);
        d.resolve_layout(800.0, 600.0);
        // A box's width is on the pixel grid (Taffy rounds layouts); the
        // reshaped lines' widest, `>b`, is 20.39.
        let got = d.tree.get(ib.0).unwrap().layout.width;
        assert!(
            (got - chrome).abs() <= 0.5,
            "{text:?} at min-content: rinch {got}, Chrome 157 {chrome}"
        );
    }
}

/// A ligature that does not cross an opportunity is untouched: Inter's `<-`
/// is followed by one (`a<-` / `b`) and `=>` has none.
#[test]
fn ligatures_without_an_opportunity_inside_are_unchanged() {
    let got = lines(1.0, "a<-b");
    assert_eq!(texts(&got), ["a<-", "b"]);
    close(got[0].1, 24.25, "a<-");
    let got = lines(1.0, "a=>b");
    assert_eq!(texts(&got), ["a=>b"]);
    close(got[0].1, 34.0469, "a=>b");
}

/// The lines of a 1px-, 20px-, …, 70px-wide `<div>` holding `markup`.
fn markup_lines(width: f32, markup: &str, style: &str) -> Vec<String> {
    let mut d = doc();
    let body = d.body();
    let c = d.create_element("div");
    d.set_attribute(c, "style", &format!("width: {width}px; {style}"));
    d.append_child(body, c);
    d.set_inner_html(c, markup);
    d.resolve_layout(800.0, 600.0);
    let n = d.tree.get(c.0).unwrap();
    let il = n.text_layout.as_ref().expect("an IFC root");
    il.layout
        .lines()
        .map(|l| il.text_content[l.text_range()].trim_end().to_string())
        .collect()
}

/// The arrow beside markup, as Chrome 157 breaks it at each width: an arrow
/// across a colour change (one shaping item), two arrows, a right-to-left
/// paragraph, and the paragraphs rinch breaks with its own lines (a `nowrap`
/// element, and `pre` text right before wrapping text, #2390).
#[test]
fn the_arrow_beside_markup_breaks_as_in_chrome() {
    let widths = [1.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0];
    let rows: [(&str, &str, [&[&str]; 7]); 5] = [
        (
            r#"a-<span style="color: red">&gt;</span>b"#,
            "",
            [
                &["a-", ">b"],
                &["a-", ">b"],
                &["a-", ">b"],
                &["a->b"],
                &["a->b"],
                &["a->b"],
                &["a->b"],
            ],
        ),
        (
            "a-&gt;b-&gt;c",
            "",
            [
                &["a-", ">b-", ">c"],
                &["a-", ">b-", ">c"],
                &["a-", ">b-", ">c"],
                &["a-", ">b-", ">c"],
                &["a->b-", ">c"],
                &["a->b->c"],
                &["a->b->c"],
            ],
        ),
        (
            "a-&gt;b",
            "direction: rtl",
            [
                &["a-", ">b"],
                &["a-", ">b"],
                &["a-", ">b"],
                &["a->b"],
                &["a->b"],
                &["a->b"],
                &["a->b"],
            ],
        ),
        (
            r#"xx <span style="white-space: nowrap">yy</span> a-&gt;b"#,
            "",
            [
                &["xx", "yy", "a-", ">b"],
                &["xx", "yy", "a-", ">b"],
                &["xx", "yy", "a-", ">b"],
                &["xx yy", "a->b"],
                &["xx yy", "a->b"],
                &["xx yy", "a->b"],
                &["xx yy a-", ">b"],
            ],
        ),
        (
            r#"Say <span style="white-space: pre">abc-</span>1-&gt;1"#,
            "",
            [
                &["Say", "abc-", "1-", ">1"],
                &["Say", "abc-", "1-", ">1"],
                &["Say", "abc-", "1->1"],
                &["Say", "abc-", "1->1"],
                &["Say", "abc-1-", ">1"],
                &["Say", "abc-1-", ">1"],
                &["Say abc-", "1->1"],
            ],
        ),
    ];
    for (markup, style, want) in rows {
        for (w, want) in widths.iter().zip(want) {
            assert_eq!(
                markup_lines(*w, markup, style),
                want,
                "{markup:?} ({style}) in {w}px"
            );
        }
    }
}

/// On one line the arrow is shared over its two characters, so a caret
/// between them stands inside it, where Chrome 157 puts it (collapsed
/// `Range` rects at offsets 1, 2 and 3).
#[test]
fn a_caret_inside_the_joined_arrow_is_at_its_middle() {
    let (d, c) = lay_out("width: 400px", "a->b");
    let n = d.tree.get(c.0).unwrap();
    let il = n.text_layout.as_ref().expect("an IFC root");
    for (byte, chrome) in [(1, 8.9844), (2, 16.6094), (3, 24.25)] {
        let cursor = parley::editing::Cursor::from_byte_index(
            &il.layout,
            byte,
            parley::layout::Affinity::Downstream,
        );
        close(
            cursor.geometry(&il.layout, 1.0).x0 as f32,
            chrome,
            &format!("caret at {byte}"),
        );
    }
}

/// A subset of Noto Sans Arabic (lam, alef, kaf, meem, space), which ligates
/// lam-alef (`ligature_break_2381/`, OFL).
const ARABIC: &[u8] = include_bytes!("ligature_break_2381/NotoSansArabic-LamAlef-subset.ttf");

/// Broken inside lam-alef under `word-break: break-all`, lam and alef keep
/// their joining forms, as Chrome 157 draws them (the subset served as
/// `@font-face`, 16px, a 1px box): `لا` is lam 4.17 / alef 4.66 and `كلام`
/// kaf 6.59 / lam 4.81 / alef 4.66 / meem 7.75. Each piece was shaped with no
/// context across the break and took its isolated form (lam 11.12).
/// Chrome's widths are in 1/64 px.
#[test]
fn arabic_letters_broken_inside_lam_alef_keep_their_joining_forms() {
    use parley::fontique::{Blob, FontInfoOverride};
    for (text, chrome) in [
        ("\u{644}\u{627}", &[4.1719, 4.6563][..]),
        (
            "\u{643}\u{644}\u{627}\u{645}",
            &[6.5938, 4.8125, 4.6563, 7.75][..],
        ),
    ] {
        let mut d = doc();
        d.font_cx.collection.register_fonts(
            Blob::new(std::sync::Arc::new(ARABIC)),
            Some(FontInfoOverride {
                family_name: Some("ArabicProbe"),
                ..Default::default()
            }),
        );
        let body = d.body();
        let c = d.create_element("div");
        d.set_attribute(
            c,
            "style",
            "width: 1px; word-break: break-all; font: 16px/20px ArabicProbe",
        );
        d.append_child(body, c);
        let t = d.create_text(text);
        d.append_child(c, t);
        d.resolve_layout(800.0, 600.0);
        let il = d.tree.get(c.0).unwrap().text_layout.as_ref().unwrap();
        let got: Vec<f32> = il.layout.lines().map(|l| l.metrics().advance).collect();
        assert_eq!(got.len(), chrome.len(), "{text:?}: {got:?}");
        for (g, c) in got.iter().zip(chrome) {
            assert!(
                (g - c).abs() < 0.02,
                "{text:?}: rinch {got:?}, Chrome 157 {chrome:?}"
            );
        }
    }
}
