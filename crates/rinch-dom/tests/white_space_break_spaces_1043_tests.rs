//! `white-space: break-spaces` (#1043): preserved spaces that **never hang**.
//!
//! CSS Text 3 §4.1.3 hangs a `pre-wrap` space at a soft wrap. `break-spaces`
//! is the value that does not: its spaces take room like any character, a
//! space that does not fit starts the next line, and there is a break
//! opportunity after every space, between two spaces too. There is none
//! between a word and the space after it, so a space that overflows right
//! after a word takes the word to the next line with it when there is an
//! opportunity before the word, and stays on the line, overflowing, when
//! there is none.
//!
//! rinch read `break-spaces` as `pre-wrap` until #1043: the spaces were kept
//! (#1192) but hung, so `bullet text ` in 75px was one line where Chrome
//! makes two.
//!
//! The issue's other half — `pre-line` keeping its spaces — was fixed by
//! #1192, which collapses each text node by its own element's `white-space`;
//! [`pre_line_collapses_the_spaces_at_a_lines_end`] pins Chrome's answer.
//!
//! Every expected number is Chrome 155's, measured with the same markup and
//! the bundled Inter (registered under an override name, so the host's fonts
//! cannot answer) at 16px with a 25px line box. Caret positions are read away
//! from a line's wrap point, where the two lines share one offset. Taffy
//! rounds box edges to whole pixels.

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

/// The issue's case. Chrome: two lines, `bullet ` / `text `: the final space
/// does not fit and does not hang, and there is no break between it and
/// `text`, so `text` goes to the next line with it. rinch (as `pre-wrap`):
/// one line, the space hanging.
#[test]
fn a_space_that_does_not_fit_takes_its_word_to_the_next_line() {
    let (d, b) = build(
        None,
        "width: 75px; white-space: break-spaces",
        "bullet text ",
    );
    assert_eq!(size(&d, b), (75.0, 2.0 * LINE));
    caret_at(&d, b, 11, (28.33, LINE));
    caret_at(&d, b, 12, (32.83, LINE));
}

/// Found while working on #1042 (the issue's comment): spaces that overflow
/// a right-aligned line go to the next one, as content. Chrome: `abc` and
/// two spaces on the first line, right-aligned (`abc` at 3.08), the other
/// eight spaces on the second (36 wide, from 4). rinch: one line, the spaces
/// hanging.
#[test]
fn spaces_that_do_not_fit_start_the_next_line_and_are_aligned_as_content() {
    let (d, b) = build(
        None,
        "width: 40px; white-space: break-spaces; text-align: right",
        "abc          ",
    );
    assert_eq!(size(&d, b), (40.0, 2.0 * LINE));
    caret_at(&d, b, 0, (3.08, 0.0));
    caret_at(&d, b, 2, (21.86, 0.0));
    caret_at(&d, b, 7, (13.0, LINE));
    caret_at(&d, b, 13, (40.0, LINE));
}

/// A run of spaces longer than a line is broken between spaces, on as many
/// lines as it takes. Chrome: `a` and six spaces, eight spaces, six spaces
/// and `b`.
#[test]
fn a_run_of_spaces_longer_than_a_line_is_broken_between_spaces() {
    let (d, b) = build(
        None,
        "width: 40px; white-space: break-spaces",
        &format!("a{}b", " ".repeat(20)),
    );
    assert_eq!(size(&d, b), (40.0, 3.0 * LINE));
    caret_at(&d, b, 1, (8.98, 0.0));
    caret_at(&d, b, 8, (4.5, LINE));
    caret_at(&d, b, 9, (9.0, LINE));
    caret_at(&d, b, 17, (9.0, 2.0 * LINE));
    caret_at(&d, b, 21, (27.0, 2.0 * LINE));
    caret_at(&d, b, 22, (36.8, 2.0 * LINE));
}

/// A word with no break opportunity before it keeps the space after it on its
/// own line, overflowing; the break is after that space. Chrome: `abcdef `
/// (57.47 wide in 20px) and ` x`. rinch: both spaces hanging on the first
/// line, `x` alone on the second.
#[test]
fn a_word_with_no_break_before_it_keeps_one_space_after_it() {
    let (d, b) = build(None, "width: 20px; white-space: break-spaces", "abcdef  x");
    assert_eq!(size(&d, b), (20.0, 2.0 * LINE));
    caret_at(&d, b, 6, (52.97, 0.0));
    caret_at(&d, b, 8, (4.5, LINE));
    caret_at(&d, b, 9, (13.23, LINE));
}

/// The same with nothing after the space: one line. Chrome: 25 tall at 19px
/// (`ab` is 18.78, its space overflows).
#[test]
fn a_lone_word_and_its_space_are_one_line() {
    let (d, b) = build(None, "width: 19px; white-space: break-spaces", "ab ");
    assert_eq!(size(&d, b), (19.0, LINE));
    caret_at(&d, b, 3, (23.28, 0.0));
}

/// The spaces count in the min-content width, and a word keeps the space
/// after it. Chrome: an `inline-block` in a 0-wide container, 32.42 wide
/// (`abc ` — no break between the word and its space) and three lines:
/// `abc `, the two other spaces, `def`. rinch: 28 x 50, `abc` and every
/// space on one line.
#[test]
fn the_min_content_width_counts_a_words_space() {
    let (d, b) = build(
        Some("width: 0"),
        "display: inline-block; white-space: break-spaces",
        "abc   def",
    );
    assert_eq!(size(&d, b), (32.0, 3.0 * LINE));
    caret_at(&d, b, 5, (4.5, LINE));
    caret_at(&d, b, 9, (25.05, 2.0 * LINE));
}

/// The max-content width counts every space. Chrome: 41.42. (rinch already
/// did: the guard that the fix does not drop them.)
#[test]
fn the_max_content_width_counts_the_spaces() {
    let (d, b) = build(
        None,
        "display: inline-block; white-space: break-spaces",
        "abc   ",
    );
    assert_eq!(size(&d, b), (41.0, LINE));
}

/// In a flex item, whose width is its max-content width clamped to the row.
/// Chrome: 75 x 50.
#[test]
fn a_flex_items_break_spaces_text_wraps_its_last_space() {
    let (d, b) = build(
        Some("display: flex; width: 75px"),
        "white-space: break-spaces",
        "bullet text ",
    );
    assert_eq!(size(&d, b), (75.0, 2.0 * LINE));
}

/// Read per element: `break-spaces` spaces in a `pre-wrap` block do not hang,
/// and `pre-wrap` spaces in a `break-spaces` block do. Chrome: 75 tall
/// (`abc` and two spaces, the other eight, `d`), and 50 (every space hanging
/// after `abc`, `d`).
#[test]
fn each_space_follows_its_own_elements_white_space() {
    for (root, span, height, carets) in [
        (
            "pre-wrap",
            "break-spaces",
            3.0 * LINE,
            vec![(1, (32.42, 0.0)), (4, (9.0, LINE)), (9, (31.5, LINE))],
        ),
        (
            "break-spaces",
            "pre-wrap",
            2.0 * LINE,
            vec![(1, (32.42, 0.0)), (4, (45.92, 0.0)), (9, (68.42, 0.0))],
        ),
    ] {
        let mut d = doc();
        let body = d.body();
        let b = el(
            &mut d,
            body,
            "div",
            &format!(
                "width: 40px; font-family: ProbeFace; font-size: 16px; line-height: 25px; \
                 white-space: {root}"
            ),
        );
        text(&mut d, b, "abc");
        let s = el(&mut d, b, "span", &format!("white-space: {span}"));
        text(&mut d, s, &" ".repeat(10));
        text(&mut d, b, "d");
        d.resolve_layout(400.0, 300.0);
        assert_eq!(size(&d, b).1, height, "{span} spaces in a {root} block");
        for (offset, at) in carets {
            caret_at(&d, b, 3 + offset, at);
        }
    }
}

/// The issue's first half, fixed by #1192: `pre-line` collapses its spaces
/// and removes the ones at a line's end. Chrome: a shrink-to-fit `abc   ` is
/// 27.92 wide, the caret after the spaces at 27.92.
#[test]
fn pre_line_collapses_the_spaces_at_a_lines_end() {
    let (d, b) = build(
        None,
        "display: inline-block; white-space: pre-line",
        "abc   ",
    );
    assert_eq!(size(&d, b), (28.0, LINE));
    caret_at(&d, b, 6, (27.92, 0.0));
}

/// The same with a word after the space: parley hangs the space and ends the
/// line there, a line no other test sends through the hang pass. Chrome:
/// `bullet ` / `text x`.
#[test]
fn a_space_that_does_not_fit_before_a_word_takes_its_word_along() {
    let (d, b) = build(
        None,
        "width: 75px; white-space: break-spaces",
        "bullet text x",
    );
    assert_eq!(size(&d, b), (75.0, 2.0 * LINE));
    caret_at(&d, b, 6, (41.56, 0.0));
    caret_at(&d, b, 11, (28.33, LINE));
    caret_at(&d, b, 13, (41.56, LINE));
}

/// Spaces that fit at a soft wrap are content too: they do not hang, so a
/// centred line is centred with them. Chrome: `ab` and six spaces, centred in
/// 60px (`ab` at 7.11), then `cd`. As `pre-wrap` they would hang and `ab`
/// would sit at 20.61.
#[test]
fn spaces_at_a_soft_wrap_are_aligned_as_content() {
    let (d, b) = build(
        None,
        "width: 60px; white-space: break-spaces; text-align: center",
        "ab      cd",
    );
    assert_eq!(size(&d, b), (60.0, 2.0 * LINE));
    caret_at(&d, b, 0, (7.11, 0.0));
    caret_at(&d, b, 4, (34.89, 0.0));
}
