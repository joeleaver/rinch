//! An atomic inline sits on its line by its **baseline** (CSS 2.1 §10.8.1,
//! #663): an `inline-block`'s is its last line box's, or its bottom margin
//! edge when it has none or its `overflow` is not `visible`; an `<input>`'s is
//! its text's. Since parley 0.12 every line box also holds the block's strut,
//! so a box lined up by its bottom edge grows the line by the strut's descent
//! — the baseline rinch hands parley (`ifc::atomic_inline_baseline`) is what
//! keeps each line Chrome's height.
//!
//! Every number is Chrome 156's (`--headless=new`, standards mode, the
//! bundled Inter registered as `ProbeFace` through `@font-face`, `16px/20px`,
//! a 400px block), `getBoundingClientRect` relative to the block.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn lay_out(inner: &str) -> RinchDocument {
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
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let c = doc.create_element("div");
    doc.set_attribute(c, "data-m", "c");
    doc.set_attribute(c, "style", "font: 16px/20px ProbeFace; width: 400px");
    doc.set_inner_html(c, inner);
    doc.append_child(body, c);
    doc.resolve_layout(800.0, 600.0);
    doc
}

/// `(y relative to the block, height)` of `[data-m=m]`.
fn at(doc: &RinchDocument, m: &str) -> (f32, f32) {
    let one = |m: &str| {
        let found = query_selector(&doc.tree, &format!("[data-m={m}]"));
        assert_eq!(found.len(), 1, "{m}");
        found[0]
    };
    let (id, c) = (one(m), one("c"));
    let y = |id| rinch_dom::paint::compute_absolute_position(&doc.tree, id, 1.0).1 as f32;
    (y(id) - y(c), doc.tree.get(id).unwrap().layout.height)
}

/// A box taller than the text beside it: its bottom on the baseline, the
/// line grown below it by the text's descent. Chrome: line 34, box at 0.
/// rinch put the box at 0 in a 30px line before parley 0.12 (and at y -3
/// before that, the issue's first row).
#[test]
fn a_box_taller_than_its_text_grows_the_line_below_it() {
    let doc = lay_out(
        r#"<div data-m="r">text <span data-m="b" style="display:inline-block;width:40px;height:30px"></span></div>"#,
    );
    assert_eq!(at(&doc, "r").1, 34.0, "Chrome: 34");
    assert_eq!(at(&doc, "b").0, 0.0, "Chrome: the box at 0");
}

/// An `inline-block` holding only block children has no line box: it is
/// lined up by its bottom edge, and the strut's descent hangs below it.
/// Chrome: 44 (the issue's 45 was the host font's).
#[test]
fn a_box_with_no_line_inside_is_lined_up_by_its_bottom() {
    let doc = lay_out(
        r#"<div data-m="r"><span style="display:inline-block"><div style="height:20px"></div><div style="height:20px"></div></span></div>"#,
    );
    assert_eq!(at(&doc, "r").1, 44.0, "Chrome: 44");
}

/// A box whose lines overflow its declared height: its baseline is its last
/// line's, below its bottom, so the line holds all three lines. Chrome: 60
/// (rinch 20 until this fix, the box's own height).
#[test]
fn a_box_whose_lines_overflow_it_is_lined_up_by_its_last_line() {
    let doc = lay_out(
        r#"<div data-m="r"><span data-m="b" style="display:inline-block;height:20px">x<br>y<br>z</span></div>"#,
    );
    assert_eq!(at(&doc, "r").1, 60.0, "Chrome: 60");
    assert_eq!(at(&doc, "b"), (0.0, 20.0), "Chrome: the box at 0, 20 tall");
}

/// `overflow: hidden` makes the bottom edge the baseline whatever is inside.
/// Chrome: the 10px box at 6 in a 20px line.
#[test]
fn a_clipping_box_is_lined_up_by_its_bottom() {
    let doc = lay_out(
        r#"<div data-m="r"><span data-m="b" style="display:inline-block;width:30px;height:10px;overflow:hidden">x</span> x</div>"#,
    );
    assert_eq!(at(&doc, "r").1, 20.0, "Chrome: 20");
    assert_eq!(at(&doc, "b").0, 6.0, "Chrome: at 6");
}

/// An `<input>` is lined up by its text, centred in it: Chrome's line is 22
/// tall, the input 21 at y 1 (rinch's input is 22 at 0.38, a separate
/// difference in its intrinsic height). Lined up by its bottom it made a
/// 25px line.
#[test]
fn an_input_is_lined_up_by_its_text() {
    let doc = lay_out(r#"<div data-m="r">x <input style="width:60px"></div>"#);
    assert_eq!(at(&doc, "r").1, 22.0, "Chrome: 22");
}

/// #577's `font-size` row: a 32px span beside 16px text raises the line it
/// shares (Chrome: 25 at `line-height: 20px`, the span's 20px box shifted to
/// the shared baseline).
#[test]
fn a_larger_span_raises_the_line_it_shares() {
    let doc = lay_out(r#"<div data-m="r">ab <span style="font-size:32px">CD</span> ef</div>"#);
    assert_eq!(at(&doc, "r").1, 25.0, "Chrome: 25");
}
