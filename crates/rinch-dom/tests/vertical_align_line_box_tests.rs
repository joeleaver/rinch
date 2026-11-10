//! A line grows to hold the inline boxes `vertical-align` moved (#1357).
//!
//! CSS 2.1 §10.8: a line box is as tall as the distance from the top of its
//! uppermost inline box to the bottom of its lowermost one, each box placed by
//! its `vertical-align`. Before this, a text span's `vertical-align` was a
//! glyph move rinch made after parley had sized the line from the unshifted
//! runs, so a raised `<sup>` painted into the line above and its line kept the
//! height of a line without it. Now the span's shift is a parley
//! `VerticalAlign`, which places the span box and sizes the line from it.
//!
//! Every number is Chrome 157's, measured from synthetic pages serving the
//! bundled Inter (`assets/fonts/Inter-Regular.ttf`) as `@font-face` under
//! `font: 16px/20px`, a 400px wide block, `getBoundingClientRect().height`.
//! rinch's lines are on the pixel grid (a quantized layout), so a fractional
//! Chrome height is compared within half a pixel.
//!
//! | block | Chrome 157 |
//! |---|---|
//! | `alpha beta gamma` | 20 |
//! | `alpha beta<sup>1</sup> gamma` | 25.33 |
//! | `…<span style="vertical-align: super">1</span>…` | 26.33 |
//! | `…<span style="vertical-align: sub">1</span>…` | 24.19 |
//! | `…<span style="vertical-align: 10px">1</span>…` | 30 |
//! | `…<span style="vertical-align: -7px">1</span>…` | 27 |
//! | `…<span style="vertical-align: 50%">1</span>…` | 30 |
//! | `<table><td><img width=300 height=150 align=center>` (cell) | 150 |
//! | the same `<img>` without `align` (cell) | 154 |

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const BLOCK: &str = "margin: 0; width: 400px; font-family: ProbeFace; font-size: 16px; \
                     line-height: 20px";

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

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    if !style.is_empty() {
        doc.set_attribute(e, "style", style);
    }
    doc.append_child(parent, e);
    e
}

fn text(doc: &mut RinchDocument, parent: NodeId, s: &str) {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
}

fn height(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().layout.height
}

/// `alpha beta<TAG style=STYLE>1</TAG> gamma` in a 16px/20px block; the
/// block's height.
fn line_with(tag: &str, style: &str) -> f32 {
    let mut d = doc();
    let body = d.body();
    let p = el(&mut d, body, "p", BLOCK);
    text(&mut d, p, "alpha beta");
    let s = el(&mut d, p, tag, style);
    text(&mut d, s, "1");
    text(&mut d, p, " gamma");
    d.resolve_layout(800.0, 600.0);
    height(&d, p)
}

#[track_caller]
fn assert_chrome(got: f32, chrome: f32, what: &str) {
    assert!(
        (got - chrome).abs() <= 0.5,
        "{what}: rinch {got}, Chrome 157 {chrome}"
    );
}

/// The control: a plain span leaves the line at its `line-height`.
#[test]
fn a_plain_span_leaves_the_line_at_its_line_height() {
    assert_chrome(line_with("span", ""), 20.0, "plain span");
}

#[test]
fn a_sup_grows_its_line() {
    assert_chrome(line_with("sup", ""), 25.328125, "<sup>");
}

#[test]
fn a_raised_span_grows_its_line_above() {
    assert_chrome(
        line_with("span", "vertical-align: super"),
        26.328125,
        "super",
    );
    assert_chrome(line_with("span", "vertical-align: 10px"), 30.0, "10px");
    assert_chrome(line_with("span", "vertical-align: 50%"), 30.0, "50%");
}

#[test]
fn a_lowered_span_grows_its_line_below() {
    assert_chrome(line_with("span", "vertical-align: sub"), 24.1875, "sub");
    assert_chrome(line_with("span", "vertical-align: -7px"), 27.0, "-7px");
}

/// The raised glyph is drawn inside its own line, not over the line above.
/// Alone in a block at the top of the page, a `1` raised by 10px sits in its
/// span's box, which the line now starts at: its first inked row is the row a
/// plain `1` alone in the same block inks first (Chrome lays both out the same
/// way, the glyph's box at the top of the line). Before, the raised `1` was
/// drawn 10px above its line's top, cut off by the page.
#[cfg(feature = "software-renderer")]
#[test]
fn a_raised_glyph_is_drawn_inside_its_line() {
    use rinch_dom::paint::skia_painter::TinySkiaPainter;
    const W: u32 = 200;
    const H: u32 = 60;
    let first_inked_row = |style: &str| {
        let mut d = doc();
        let body = d.body();
        d.set_attribute(body, "style", "margin: 0");
        let p = el(&mut d, body, "p", BLOCK);
        let s = el(&mut d, p, "span", style);
        text(&mut d, s, "1");
        d.resolve_layout(W as f32, H as f32);
        let mut painter = TinySkiaPainter::new(W, H);
        let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &d.tree,
            &mut painter,
            1.0,
            (W as f32, H as f32),
            &mut d.font_cx,
            &mut cx,
        );
        let px = painter.pixels();
        (0..H as usize)
            .position(|y| {
                (0..W as usize).any(|x| {
                    let i = (y * W as usize + x) * 4;
                    px[i + 3] > 0 && !(px[i] > 240 && px[i + 1] > 240 && px[i + 2] > 240)
                })
            })
            .expect("the glyph painted nothing")
    };
    let plain = first_inked_row("");
    let raised = first_inked_row("vertical-align: 10px");
    assert!(
        plain >= 2,
        "the control: a plain `1` inks below the block's top, at row {plain}"
    );
    assert!(
        raised.abs_diff(plain) <= 1,
        "a `1` raised by 10px inks first at row {raised}, a plain one at {plain}: \
         the line did not grow above it"
    );
}

/// Three lines at 120px, two of them holding a `<sup>`.
#[test]
fn only_the_lines_holding_a_sup_grow() {
    let mut d = doc();
    let body = d.body();
    let p = el(&mut d, body, "p", &format!("{BLOCK}; width: 120px"));
    text(&mut d, p, "alpha");
    let a = el(&mut d, p, "sup", "");
    text(&mut d, a, "1");
    text(&mut d, p, " beta gamma delta");
    let b = el(&mut d, p, "sup", "");
    text(&mut d, b, "2");
    text(&mut d, p, " eps zeta eta");
    d.resolve_layout(800.0, 600.0);
    // Chrome 157: 70.66 = 20 + 2 × 25.33; rinch's lines are whole pixels.
    let h = height(&d, p);
    assert!(
        (h - 70.65625).abs() <= 1.0,
        "three lines, two with a <sup>: rinch {h}, Chrome 157 70.66"
    );
}

/// `<img align=center>` is `-webkit-baseline-middle` in Chrome; alone in a
/// table cell it leaves no strut descent under the picture, as
/// `vertical-align: middle` does.
#[test]
fn an_img_aligned_center_leaves_no_strut_under_it_in_a_cell() {
    let cell_height = |align: Option<&str>| {
        let mut d = doc();
        let body = d.body();
        d.set_attribute(
            body,
            "style",
            "margin: 0; font-family: ProbeFace; font-size: 16px",
        );
        let table = el(&mut d, body, "table", "border-spacing: 0");
        let tr = el(&mut d, table, "tr", "");
        let td = el(&mut d, tr, "td", "padding: 0; line-height: 20px");
        let img = d.create_element("img");
        d.set_attribute(img, "width", "300");
        d.set_attribute(img, "height", "150");
        if let Some(a) = align {
            d.set_attribute(img, "align", a);
        }
        d.append_child(td, img);
        d.resolve_layout(800.0, 600.0);
        height(&d, td)
    };
    // The control: on the baseline, the strut's descent hangs under it.
    assert!(
        cell_height(None) > 150.5,
        "baseline img: cell {}",
        cell_height(None)
    );
    assert_chrome(cell_height(Some("center")), 150.0, "align=center");
    assert_chrome(cell_height(Some("middle")), 150.0, "align=middle");
}

/// A raised inline split by a block (#513) raises each of its fragments: the
/// lines before and after the block are both 30px (Chrome 157: 80 in all,
/// with the block's 20), whether the fragment's content is its own text or
/// an element of its own (`<b>`), which parley sees as no child of the span.
#[test]
fn a_split_raised_inline_grows_the_lines_on_both_sides_of_the_block() {
    for wrap_tail in [false, true] {
        let mut d = doc();
        let body = d.body();
        let p = el(&mut d, body, "div", BLOCK);
        let s = el(&mut d, p, "span", "vertical-align: 10px");
        text(&mut d, s, "a");
        let block = el(&mut d, s, "div", "");
        text(&mut d, block, "block");
        if wrap_tail {
            let b = el(&mut d, s, "b", "");
            text(&mut d, b, "b");
        } else {
            text(&mut d, s, "b");
        }
        d.resolve_layout(800.0, 600.0);
        assert_chrome(
            height(&d, p),
            80.0,
            if wrap_tail {
                "split, <b> tail"
            } else {
                "split, text tail"
            },
        );
    }
}
