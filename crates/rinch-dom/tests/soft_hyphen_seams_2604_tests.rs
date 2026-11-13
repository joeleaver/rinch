//! A soft hyphen (or another zero-width character with no break before it)
//! at the end of a line the seams pass re-breaks (#2604). parley ends a line
//! broken at a soft hyphen after it, at the advance of the text before it,
//! and the hang pass placed the line's end by that advance, before the
//! hyphen. The next line it broke again was then counted from the hyphen,
//! one character more than the breaker had left: it ended a character early
//! and split the word after the hyphen (`Extra` / `o` / `rdinary`). The end is
//! now read from the line's text (`BreakLines::prior_line_text_range`).
//!
//! The seams pass runs in a paragraph with text that does not wrap right
//! before wrapping text (`<span nowrap>abc-</span>123`, #2390) or with an
//! atomic inline and an element that does not wrap (#1238).
//!
//! Every expectation is Chrome 157's line breaks for the same markup. A line
//! is written as rinch's text of it, a soft hyphen as `~` (the space a line
//! ends with is on it). Text is the bundled Inter registered as `ProbeFace`,
//! `font: 16px/20px`.
//!
//! Chrome draws a hyphen at a soft hyphen it breaks at and fits the line with
//! it; rinch draws none (#2614). The widths here are ones where the hyphen's
//! 5.7px does not move a break.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// Text that does not wrap right before wrapping text: a text seam (#2390).
const TEXT_SEAM: &str = "<span style='white-space:nowrap'>abc-</span>123 ";

/// An atomic inline and an element that does not wrap: box seams (#1238).
const BOX_SEAM: &str = "<span style='display:inline-block;vertical-align:top;width:10px;height:20px'></span> <span style='white-space:nowrap'>abc</span> ";

fn html(markup: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut d = RinchDocument::new();
    let registered = d.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    d.load_css("* { margin: 0; padding: 0 } body { font: 16px/20px ProbeFace }");
    let body = d.body();
    d.set_inner_html(body, markup);
    d.resolve_layout(800.0, 800.0);
    d
}

fn by_id(d: &RinchDocument, id: &str) -> NodeId {
    d.query_selector(&format!("#{id}")).expect(id)
}

/// `markup` in a `width`px div: the text of each of its lines, a soft
/// hyphen as `~` and a zero-width space as `^`, and the div's height.
fn layout(markup: &str, width: u32) -> (Vec<String>, f32) {
    let d = html(&format!("<div id=a style='width:{width}px'>{markup}</div>"));
    let n = d.tree.get(by_id(&d, "a").0).unwrap();
    let il = n.text_layout.as_ref().expect("an IFC root");
    let lines = il
        .layout
        .lines()
        .map(|l| {
            il.text_content
                .get(l.text_range())
                .unwrap_or("")
                .replace('\u{ad}', "~")
                .replace('\u{200b}', "^")
        })
        .collect();
    (lines, n.layout.height)
}

#[track_caller]
fn breaks(markup: &str, width: u32, chrome: &[&str]) {
    let (got, _) = layout(markup, width);
    assert_eq!(got, chrome, "{width}px: {markup}");
}

/// [`breaks`], and the div is `height` tall in Chrome.
#[track_caller]
fn breaks_tall(markup: &str, width: u32, chrome: &[&str], height: f32) {
    let (got, h) = layout(markup, width);
    assert_eq!(got, chrome, "{width}px: {markup}");
    assert!(
        (h - height).abs() < 0.5,
        "{width}px: {markup}: rinch {h}, Chrome {height}"
    );
}

/// A soft hyphen at the start, the middle and near the end of the word, at
/// a width where nothing fits (the hyphenated piece overflows too) and one
/// where the seam's line fits. rinch split the word one character after the
/// hyphen at each (`Ex~` / `t` / `raordinary`).
#[test]
fn a_soft_hyphen_anywhere_in_a_word_breaks_there() {
    let row = |word: &str| format!("{TEXT_SEAM}{word}");
    breaks(
        &row("Extra&shy;ordinary"),
        1,
        &["abc-", "123 ", "Extra~", "ordinary"],
    );
    breaks(
        &row("Extra&shy;ordinary"),
        80,
        &["abc-123 ", "Extra~", "ordinary"],
    );
    breaks(
        &row("Ex&shy;traordinary"),
        1,
        &["abc-", "123 ", "Ex~", "traordinary"],
    );
    breaks(
        &row("Ex&shy;traordinary"),
        80,
        &["abc-123 ", "Ex~", "traordinary"],
    );
    breaks(
        &row("Extraordi&shy;nary"),
        1,
        &["abc-", "123 ", "Extraordi~", "nary"],
    );
    breaks(
        &row("Extraordi&shy;nary"),
        130,
        &["abc-123 ", "Extraordi~nary"],
    );
}

/// Several soft hyphens in one word, with a text seam and with box seams:
/// one piece per line, and several on a line. rinch split the word after
/// the first hyphen (`Ex~` / `t` / `ra~`).
#[test]
fn several_soft_hyphens_in_one_word() {
    let word = "Ex&shy;tra&shy;or&shy;di&shy;nary";
    let text = format!("{TEXT_SEAM}{word}");
    breaks(
        &text,
        1,
        &["abc-", "123 ", "Ex~", "tra~", "or~", "di~", "nary"],
    );
    breaks(&text, 80, &["abc-123 ", "Ex~tra~or~di~", "nary"]);
    breaks(&text, 130, &["abc-123 Ex~tra~or~", "di~nary"]);
    // The box's line holds the space after it.
    let boxed = format!("{BOX_SEAM}{word}");
    breaks_tall(
        &boxed,
        1,
        &[" ", "abc ", "Ex~", "tra~", "or~", "di~", "nary"],
        140.0,
    );
    breaks_tall(&boxed, 100, &[" abc Ex~tra~", "or~di~nary"], 40.0);
    breaks_tall(&boxed, 130, &[" abc Ex~tra~or~di~", "nary"], 40.0);
}

/// A soft hyphen right before and right inside a `nowrap` element: the
/// opportunity after it is the wrapping parent's (Chrome breaks after
/// `Extra&shy;` at the end of a `nowrap` span, and draws no hyphen there).
#[test]
fn a_soft_hyphen_next_to_a_nowrap_seam() {
    let before = "<span style='white-space:nowrap'>abc-</span>Extra&shy;<span style='white-space:nowrap'>ordinary</span>";
    breaks(before, 1, &["abc-", "Extra~", "ordinary"]);
    breaks(before, 70, &["abc-", "Extra~", "ordinary"]);
    breaks(before, 80, &["abc-Extra~", "ordinary"]);
    let inside = "<span style='white-space:nowrap'>Extra&shy;</span>ordinary";
    breaks(inside, 1, &["Extra~", "ordinary"]);
    breaks(inside, 130, &["Extra~ordinary"]);
}

/// No room even for the piece before the hyphen: each piece overflows on a
/// line of its own, with box seams as with a text seam.
#[test]
fn no_room_for_the_hyphenated_piece() {
    let boxed = format!("{BOX_SEAM}Extra&shy;ordinary");
    breaks_tall(&boxed, 1, &[" ", "abc ", "Extra~", "ordinary"], 80.0);
    breaks_tall(&boxed, 30, &[" ", "abc ", "Extra~", "ordinary"], 80.0);
    breaks_tall(&boxed, 80, &[" abc ", "Extra~", "ordinary"], 60.0);
    breaks(
        &format!("{TEXT_SEAM}Extra&shy;ordinary"),
        30,
        &["abc-", "123 ", "Extra~", "ordinary"],
    );
}

/// A soft hyphen right after a space has an opportunity on each side (UAX
/// #14 LB18 before it, LB21 after it), both at the line's width: the line
/// ends after it when it fits, and at the space when it overflows, where
/// Chrome puts the hyphen on a line of its own. rinch split `Extra` after
/// `E` at both widths.
#[test]
fn a_soft_hyphen_after_a_space() {
    let row = format!("{TEXT_SEAM}&shy;Extraordinary");
    breaks_tall(&row, 1, &["abc-", "123 ", "~", "Extraordinary"], 80.0);
    breaks_tall(&row, 80, &["abc-123 ~", "Extraordinary"], 40.0);
    let two = format!("{TEXT_SEAM}&shy;Extra&shy;ordinary");
    breaks_tall(
        &two,
        30,
        &["abc-", "123 ", "~", "Extra~", "ordinary"],
        100.0,
    );
    breaks_tall(&two, 100, &["abc-123 ~", "Extra~", "ordinary"], 60.0);
}

/// A zero-width space is the same: no opportunity before it, one after.
#[test]
fn a_zero_width_space_ends_the_line_it_follows() {
    let row = format!("{TEXT_SEAM}Extra&#x200b;ordinary");
    breaks(&row, 1, &["abc-", "123 ", "Extra^", "ordinary"]);
    breaks(&row, 130, &["abc-123 Extra^", "ordinary"]);
}
