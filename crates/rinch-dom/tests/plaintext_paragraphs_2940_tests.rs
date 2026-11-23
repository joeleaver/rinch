//! #2940: a `unicode-bidi: plaintext` block gives each of its paragraphs
//! its own base direction, from that paragraph's first strong character
//! (UAX #9 P2/P3, `ltr` when it has none), where a forced break (a preserved
//! newline, a `<br>`) ends a paragraph. Before, the whole block took the
//! direction of the first strong character of all its text.
//!
//! Each case is measured in Chrome 158 (headless, `--dump-dom`) on this exact
//! markup: the bundled Inter at a declared `font: 16px/20px`, a `400px` host
//! at the origin, `body { margin: 0 }`, `pre { margin: 0; font: inherit }`
//! and `span.t { display: inline-block }`, so that a measured word has a box
//! of its own. A case's boxes are the host's `.t` elements in document
//! order, `[x, y, w, h]` relative to the host. No expected number depends on
//! a Hebrew glyph's width (Inter has none, so that would pin the host's
//! fallback font): a box in a right-to-left paragraph is its first item, at
//! the line's right.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::testing::{get_node_summary, query_selector};

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

type Case = (&'static str, String, &'static [[f32; 4]]);

fn cases() -> Vec<Case> {
    let a = "\u{5d0}";
    let t = r#"<span class=t>abc</span>"#;
    vec![
        // The issue's case: the second paragraph has only Latin, so it is
        // `ltr` and starts at the left.
        (
            "pre_dir_auto_hebrew_then_latin",
            format!("<pre dir=auto>{a}\n{t}</pre>"),
            &[[0.0, 20.0, 27.92, 20.0]],
        ),
        // The mirror: the first paragraph is `ltr`, the second `rtl`, its
        // box (a neutral, before the `א`) at the right.
        (
            "pre_dir_auto_latin_then_hebrew",
            format!("<pre dir=auto>{t}\n{t} {a}</pre>"),
            &[[0.0, 0.0, 27.92, 20.0], [372.08, 20.0, 27.92, 20.0]],
        ),
        (
            "pre_dir_auto_three_paragraphs",
            format!("<pre dir=auto>{a}\n{t}\n{t} {a}</pre>"),
            &[[0.0, 20.0, 27.92, 20.0], [372.08, 40.0, 27.92, 20.0]],
        ),
        // A paragraph with no strong character is `ltr` (P3), whatever the
        // block's `direction`.
        (
            "neutral_paragraph_after_hebrew",
            format!("<pre dir=auto>{a}\n{t}\n<span class=t>12</span></pre>"),
            &[[0.0, 20.0, 27.92, 20.0], [0.0, 40.0, 16.27, 20.0]],
        ),
        (
            "neutral_paragraph_in_an_rtl_block",
            r#"<pre dir=rtl style="unicode-bidi:plaintext"><span class=t>12</span>
abc</pre>"#
                .into(),
            &[[0.0, 0.0, 16.27, 20.0]],
        ),
        // A `<br>` ends a paragraph too.
        (
            "div_plaintext_br",
            format!(r#"<div style="unicode-bidi:plaintext">{a}<br>{t}</div>"#),
            &[[0.0, 20.0, 27.92, 20.0]],
        ),
        (
            "div_rtl_plaintext_br",
            format!(r#"<div dir=rtl style="unicode-bidi:plaintext">abc<br>{t} {a}<br>{t}</div>"#),
            &[[372.08, 20.0, 27.92, 20.0], [0.0, 40.0, 27.92, 20.0]],
        ),
        // `text-align: end` is the paragraph's end: the right of an `ltr` one.
        (
            "text_align_end",
            format!(
                r#"<pre dir=auto style="text-align:end">{a}
{t}</pre>"#
            ),
            &[[372.08, 20.0, 27.92, 20.0]],
        ),
        // Every line of a wrapped `ltr` paragraph after an `rtl` one starts at
        // the left.
        (
            "wrapped_paragraph",
            format!(
                r#"<div style="unicode-bidi:plaintext;width:100px">{a}<br>{t} {t} {t} {t}</div>"#
            ),
            &[
                [0.0, 20.0, 27.92, 20.0],
                [32.42, 20.0, 27.92, 20.0],
                [64.84, 20.0, 27.92, 20.0],
                [0.0, 40.0, 27.92, 20.0],
            ],
        ),
    ]
}

fn document(markup: &str) -> RinchDocument {
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
    doc.load_css(
        "body { margin: 0; font: 16px/20px ProbeFace; } span.t { display: inline-block } \
         pre { margin: 0; font: inherit } #host > div, #host > pre { width: 400px }",
    );
    let body = doc.body();
    let host = doc.create_element("div");
    doc.set_attribute(host, "id", "host");
    doc.set_attribute(host, "style", "width: 400px");
    doc.append_child(body, host);
    doc.set_inner_html(host, markup);
    doc.resolve_layout(800.0, 600.0);
    doc
}

fn boxes(doc: &RinchDocument) -> Vec<[f32; 4]> {
    query_selector(&doc.tree, ".t")
        .iter()
        .map(|&n| {
            let abs = &get_node_summary(&doc.tree, n).unwrap()["absolute"];
            let at = |k: &str| abs[k].as_f64().unwrap() as f32;
            [at("x"), at("y"), at("width"), at("height")]
        })
        .collect()
}

/// Every case against Chrome 158, each edge to within half a pixel.
#[test]
fn plaintext_paragraphs_take_their_own_direction_as_in_chrome_158() {
    let mut wrong = Vec::new();
    let all = cases();
    for (name, markup, chrome) in &all {
        let doc = document(markup);
        let got = boxes(&doc);
        let ok = got.len() == chrome.len()
            && got
                .iter()
                .zip(chrome.iter())
                .all(|(g, c)| (0..4).all(|i| (g[i] - c[i]).abs() <= 0.5));
        if !ok {
            wrong.push(format!("{name}: rinch {got:?}, Chrome {chrome:?}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {} cases differ from Chrome 158:\n{}",
        wrong.len(),
        all.len(),
        wrong.join("\n")
    );
}

/// The left of a line's leftmost glyph run, and whether the line runs right
/// to left.
fn line_start(layout: &parley::layout::Layout<peniko::Brush>, index: usize) -> (f32, bool) {
    let line = layout.get(index).expect("the line");
    let left = line
        .items()
        .filter_map(|item| match item {
            parley::layout::PositionedLayoutItem::GlyphRun(g) => Some(g.offset()),
            _ => None,
        })
        .fold(f32::INFINITY, f32::min);
    (left, line.is_rtl())
}

/// Only `plaintext` gives each paragraph its own direction: the text of a
/// flex container (a text leaf, `ifc::shape_text_leaf`) is laid out in its
/// `direction`, every line. Chrome 158: `abcdefghij⏎א x` in a 300px `ltr`
/// flex container with `white-space: pre` starts its second line at 0 (a
/// range over it starts there). parley's default, `Auto`, had laid it out
/// right to left (review of #2969).
#[test]
fn a_flex_containers_text_keeps_its_direction_on_every_line() {
    let doc = document(
        "<div style=\"display:flex;white-space:pre;width:300px\">abcdefghij\n\u{5d0} x</div>",
    );
    let flex = query_selector(&doc.tree, "#host > div")[0];
    let text = doc.tree.get(flex).unwrap().children[0];
    let layout = doc
        .tree
        .get(text)
        .unwrap()
        .cached_text_parley
        .as_ref()
        .expect("the leaf's layout");
    let (left, rtl) = line_start(layout, 1);
    assert!(!rtl && left.abs() < 0.5, "line 2 at {left}, rtl {rtl}");
}

/// An `ltr` `<textarea>` is `plaintext` only with `dir=auto`: its second
/// line, which starts with Hebrew, is left to right. Chrome 158 (a 300px
/// textarea, no padding or border, red text read off a screenshot): both
/// lines ink from column 1.
#[test]
fn an_ltr_textarea_keeps_its_direction_on_every_line() {
    let mut doc = document(
        "<textarea style=\"margin:0;padding:0;border:0;font:inherit;width:300px;height:60px\"></textarea>",
    );
    let ta = query_selector(&doc.tree, "textarea")[0];
    let mut layout_cx = parley::LayoutContext::new();
    let node = doc.tree.get(ta).unwrap().clone();
    let layout = rinch_dom::form_control::field_text_layout(
        &node,
        "abc\n\u{5e9}\u{5dc} 1!",
        1.0,
        None,
        &mut doc.font_cx,
        &mut layout_cx,
    );
    let (left, rtl) = line_start(&layout, 1);
    assert!(!rtl && left.abs() < 0.5, "line 2 at {left}, rtl {rtl}");
}

/// The "…" of a clamped line (`-webkit-line-clamp`) is at the end of its
/// own paragraph under `plaintext`, on a line that does not overflow (one
/// that does takes the side it overflows on). Chrome 158, a 100px `<pre
/// dir=auto>` clamped to 2 lines (ink read off a screenshot): `א⏎abc⏎more`
/// draws `abc…` from column 1 to 39, the "…" right of `abc`; `abc⏎אב⏎more`
/// draws the "…" left of the Hebrew, inking 73 to 98.
#[test]
fn the_ellipsis_of_a_clamped_plaintext_line_is_at_its_paragraphs_end() {
    let style = "display:-webkit-box;-webkit-box-orient:vertical;-webkit-line-clamp:2;overflow:hidden;width:100px";
    for (first, second, at_right) in [("\u{5d0}", "abc", true), ("abc", "\u{5d0}\u{5d1}", false)] {
        let doc = document(&format!(
            "<pre dir=auto style=\"{style}\">{first}\n{second}\nmore</pre>"
        ));
        let pre = query_selector(&doc.tree, "pre")[0];
        let il = doc.tree.get(pre).unwrap().text_layout.as_ref().unwrap();
        let cut = il.ellipsis.as_ref().expect("the second line is clamped");
        let mark = cut
            .marks
            .iter()
            .find(|m| m.line == 1)
            .expect("line 2's mark");
        let line = il.layout.get(1).unwrap();
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        for item in line.items() {
            if let parley::layout::PositionedLayoutItem::GlyphRun(g) = item {
                lo = lo.min(g.offset());
                hi = hi.max(g.offset() + g.advance());
            }
        }
        assert_eq!(
            mark.x >= (lo + hi) / 2.0,
            at_right,
            "{first}: the … at {} beside {lo}..{hi}",
            mark.x
        );
    }
}
