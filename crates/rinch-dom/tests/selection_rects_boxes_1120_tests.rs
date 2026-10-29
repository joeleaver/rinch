//! #1120 — `query_selection_rects_with_boxes`: a selection that names the
//! inline boxes (images) it holds, since a box has no bytes and the byte
//! range cannot say which side of one an endpoint is on.
//!
//! Inter under an explicit `U+202E RIGHT-TO-LEFT OVERRIDE` makes a line hold a
//! right-to-left run (the walk `bidi_line_pieces` takes) with the bundled face
//! only, as in `selection_rects_bidi_1114_tests.rs`.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const INTER: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// `<p>{before}<img 40x16>{after}</p>`: the document, the paragraph, the
/// image, and the image's byte (the length of `before`).
fn doc_with(before: &str, after: &str) -> (RinchDocument, u64, u64, usize) {
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
        "margin: 0; width: 400px; font-family: ProbeFace; font-size: 16px; line-height: 40px",
    );
    doc.append_child(body, p);
    let t = doc.create_text(before);
    doc.append_child(p, t);
    let img = doc.create_element("img");
    doc.set_attribute(img, "style", "width: 40px; height: 16px");
    doc.append_child(p, img);
    let t = doc.create_text(after);
    doc.append_child(p, t);
    doc.resolve_layout(600.0, 600.0);
    (doc, p.0, img.0, before.len())
}

/// The image's box `(left, right)`, paragraph-local: the caret on each side.
fn image_edges(doc: &RinchDocument, p: u64, img: u64) -> (f32, f32) {
    let l = doc
        .query_inline_box_caret(p, img, false)
        .expect("laid out")
        .0;
    let r = doc
        .query_inline_box_caret(p, img, true)
        .expect("laid out")
        .0;
    (l, r)
}

#[track_caller]
fn assert_one(got: &[(f32, f32, f32, f32)], left: f32, right: f32, what: &str) {
    assert_eq!(got.len(), 1, "{what}: {got:?}");
    let (l, r) = (got[0].0, got[0].0 + got[0].2);
    assert!(
        (l - left).abs() < 0.5 && (r - right).abs() < 0.5,
        "{what}: {l}..{r}, want {left}..{right}"
    );
}

/// Left to right. Just the image (`[i, i)` with the image named), from after
/// it (`[i, i + 1)` with nothing named) and up to after it.
#[test]
fn a_left_to_right_line_takes_the_named_image_and_no_other() {
    let (doc, p, img, i) = doc_with("ab", "cd");
    let (l, r) = image_edges(&doc, p, img);
    assert!(r >= l + 39.0, "positive control: {l}..{r}");
    let c_end = doc.query_caret_position(p, i + 1).unwrap().0;
    assert_one(
        &doc.query_selection_rects_with_boxes(p, i, i, &[img]),
        l,
        r,
        "the image",
    );
    assert_one(
        &doc.query_selection_rects_with_boxes(p, i, i + 1, &[]),
        r,
        c_end,
        "after the image over `c`",
    );
    let a = doc.query_caret_position(p, 1).unwrap().0;
    assert_one(
        &doc.query_selection_rects_with_boxes(p, 1, i, &[img]),
        a,
        r,
        "`b` and the image",
    );
    // The byte-only query is unchanged: `[i, i + 1)` starts before the box.
    assert_one(
        &doc.query_selection_rects(p, i, i + 1),
        l,
        c_end,
        "bytes only",
    );
}

/// A line holding a right-to-left run is walked in visual order: the named
/// image is selected and an unnamed one at the range's start byte is not.
#[test]
fn a_line_with_a_right_to_left_run_takes_the_named_image_and_no_other() {
    let before = "\u{202E}def\u{202C} ab";
    let (doc, p, img, i) = doc_with(before, "cd");
    let (l, r) = image_edges(&doc, p, img);
    assert!(r >= l + 39.0, "positive control: {l}..{r}");
    let rtl = doc.query_selection_rects(p, 0, 6);
    assert!(
        !rtl.is_empty(),
        "positive control: the reversed run has geometry"
    );
    assert_one(
        &doc.query_selection_rects_with_boxes(p, i, i, &[img]),
        l,
        r,
        "the image",
    );
    let got = doc.query_selection_rects_with_boxes(p, i, i + 1, &[]);
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(
        (got[0].0 - r).abs() < 0.5,
        "after the image over `c`: starts at its right edge {r}: {got:?}"
    );
}
