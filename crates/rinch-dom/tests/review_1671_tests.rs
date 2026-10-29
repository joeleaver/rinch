//! Review fixtures for PR #1671 (#1043).

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::text_query::caret_position_for_offset;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const LINE: f32 = 25.0;

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
    doc.set_attribute(e, "style", style);
    doc.append_child(parent, e);
    e
}

fn text(doc: &mut RinchDocument, parent: NodeId, s: &str) -> NodeId {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
    t
}

/// A `style`d `div` holding `s`, inside a wrapper styled `outer` when given.
fn build(outer: Option<&str>, style: &str, s: &str) -> (RinchDocument, NodeId) {
    let mut d = doc();
    let body = d.body();
    let root = el(
        &mut d,
        body,
        "div",
        "margin: 0; font-family: ProbeFace; font-size: 16px; line-height: 25px",
    );
    let parent = match outer {
        Some(o) => el(&mut d, root, "div", o),
        None => root,
    };
    let b = el(&mut d, parent, "div", style);
    text(&mut d, b, s);
    d.resolve_layout(400.0, 300.0);
    (d, b)
}

fn size(d: &RinchDocument, id: NodeId) -> (f32, f32) {
    let n = d.tree.get(id.0).unwrap();
    (n.layout.width, n.layout.height)
}

/// Asserts the caret before byte `offset` of `root`'s text is at `(x, y)`.
fn caret_at(d: &RinchDocument, root: NodeId, offset: usize, (x, y): (f32, f32)) {
    let (cx, cy) = caret_position_for_offset(d, root.0, offset).expect("an IFC root has a layout");
    assert!(
        (cx - x).abs() < 0.1 && cy == y,
        "caret at {offset}: ({cx}, {cy}), Chrome ({x}, {y})"
    );
}

// Review of PR #1671. Every number is Chrome 155's, measured with the same
// markup and the bundled Inter at 16px / 25px.

/// With `overflow-wrap: break-word` (ProseMirror's own pairing with
/// `break-spaces`) a word that fits keeps its letters: Chrome breaks between
/// `bullet` and its space. The PR re-broke the line with room for less than
/// the word, so the breaker split the word: `bulle` / `t text `.
#[test]
fn rv_overflow_wrap_does_not_split_a_word_that_fits() {
    for ow in ["break-word", "anywhere"] {
        let (d, b) = build(
            None,
            &format!("width: 45px; white-space: break-spaces; overflow-wrap: {ow}"),
            "bullet text ",
        );
        assert_eq!(size(&d, b), (45.0, 2.0 * LINE), "{ow}");
        caret_at(&d, b, 5, (36.33, 0.0));
        caret_at(&d, b, 7, (4.5, LINE));
    }
}

/// The same when the word has already been wrapped into: `ghij` keeps its
/// `j` and the three spaces start the next line.
#[test]
fn rv_overflow_wrap_keeps_the_last_letter_on_its_line() {
    let (d, b) = build(
        None,
        "width: 30px; white-space: break-spaces; overflow-wrap: anywhere",
        "abcdefghij   k",
    );
    assert_eq!(size(&d, b), (30.0, 4.0 * LINE));
    caret_at(&d, b, 9, (23.14, 2.0 * LINE));
    caret_at(&d, b, 11, (4.5, 3.0 * LINE));
}

/// After a forced break the line holds the spaces that fit, not one more:
/// the length the pass committed counted the newline unit the cursor sat on.
#[test]
fn rv_spaces_after_a_newline_fill_the_line_exactly() {
    let (d, b) = build(
        None,
        "width: 10px; white-space: break-spaces",
        "abc\n      d",
    );
    assert_eq!(size(&d, b), (10.0, 5.0 * LINE));
    caret_at(&d, b, 5, (4.5, LINE));
    caret_at(&d, b, 7, (4.5, 2.0 * LINE));
    caret_at(&d, b, 9, (4.5, 3.0 * LINE));
}

/// A line that fits within half a pixel is kept (the fit tolerance).
#[test]
fn rv_a_line_that_just_fits_is_kept() {
    let (d, b) = build(
        None,
        "width: 39px; white-space: break-spaces",
        "abc    def  gh   ",
    );
    assert_eq!(size(&d, b), (39.0, 3.0 * LINE));
    caret_at(&d, b, 8, (18.8, LINE));
    caret_at(&d, b, 13, (14.31, 2.0 * LINE));
}

/// Only `break-spaces` spaces are kept in the min-content width: the
/// `pre-wrap` spaces of the same paragraph still hang. Chrome: 52.97
/// (`abcdef`), where counting every space of a paragraph that holds any
/// `break-spaces` text gives 66.
#[test]
fn rv_min_content_keeps_only_break_spaces_spaces() {
    let mut d = doc();
    let body = d.body();
    let root = el(
        &mut d,
        body,
        "div",
        "margin: 0; font-family: ProbeFace; font-size: 16px; line-height: 25px; width: 0",
    );
    let b = el(
        &mut d,
        root,
        "div",
        "display: inline-block; white-space: pre-wrap",
    );
    text(&mut d, b, "abcdef   gh   ");
    let s = el(&mut d, b, "span", "white-space: break-spaces");
    text(&mut d, s, "x");
    d.resolve_layout(400.0, 300.0);
    assert_eq!(size(&d, b), (53.0, 2.0 * LINE));
}
