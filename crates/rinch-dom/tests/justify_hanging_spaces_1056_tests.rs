//! A justified line shares its free space among its justification
//! opportunities only (#1056): spaces that hang at a soft wrap take no part
//! (CSS Text 3 §4.1.3, §7.3), and the line ends flush with the box.
//!
//! parley 0.11.1 counts a line's opportunities as it breaks it
//! (`LineData::num_spaces`): every space or NBSP that fits, less one when the
//! line ends in one. So a run of two or more spaces that fit at a wrap took
//! shares of the free space, and a line parley ended by hanging the one space
//! that did not fit lost one (that space is not counted, and the "less one"
//! then removes a word gap). `ifc::justify_line` commits such a line again by
//! length, which makes parley's count exact, and widens its alignment width so
//! that the share each opportunity gets is the right one.
//!
//! Every expected number is Chrome 155's, measured with the same markup and
//! the same face (the bundled Inter, registered under an override name so the
//! host's fonts cannot answer instead) at 16px with a 25px line box. Chrome's
//! rule, as measured: the opportunities are the spaces and NBSPs of the line's
//! content (its hanging spaces excluded), less the last one when the content
//! ends in one (a `break-spaces` space or an NBSP, which do not hang).

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

/// A justified block `width` wide holding `parts`: plain text, or
/// `(style, text)` for a span.
fn block(width: &str, white_space: &str, parts: &[(&str, &str)]) -> (RinchDocument, NodeId) {
    let mut d = doc();
    let body = d.body();
    let b = d.create_element("div");
    d.set_attribute(
        b,
        "style",
        &format!(
            "width: {width}; white-space: {white_space}; text-align: justify; \
             font-family: ProbeFace; font-size: 16px; line-height: 25px"
        ),
    );
    d.append_child(body, b);
    for (style, s) in parts {
        let t = d.create_text(s);
        if style.is_empty() {
            d.append_child(b, t);
        } else {
            let span = d.create_element("span");
            d.set_attribute(span, "style", style);
            d.append_child(b, span);
            d.append_child(span, t);
        }
    }
    d.resolve_layout(400.0, 300.0);
    (d, b)
}

/// The caret at byte `offset` of the block's flat text: `(x, line)`.
fn at(d: &RinchDocument, b: NodeId, offset: usize) -> (f32, f32) {
    let (x, y) = caret_position_for_offset(d, b.0, offset).expect("a text layout");
    (x, y / LINE)
}

#[track_caller]
fn assert_at(d: &RinchDocument, b: NodeId, offset: usize, x: f32, line: f32, what: &str) {
    let (got_x, got_line) = at(d, b, offset);
    assert_eq!(got_line, line, "{what}: on line {got_line}, Chrome {line}");
    assert!((got_x - x).abs() < 0.05, "{what}: at {got_x}, Chrome {x}");
}

/// The issue's text. Chrome: `bb` and `dd` end flush at 50, and the one word
/// gap before each takes all the free space (`bb` starts at 30.41). On `main`
/// parley counted two of the three hanging spaces: `bb` ended at 46.03.
#[test]
fn three_hanging_spaces_take_no_share() {
    let (d, b) = block("50px", "pre-wrap", &[("", "aa bb   cc dd   ee ff gg")]);
    assert_at(&d, b, 3, 30.41, 0.0, "`bb` starts");
    assert_at(&d, b, 5, 50.0, 0.0, "`bb` ends");
    assert_at(&d, b, 11, 30.41, 1.0, "`dd` starts");
    assert_at(&d, b, 13, 50.0, 1.0, "`dd` ends");
    // A line that keeps one hanging space was right before: the control.
    assert_at(&d, b, 21, 50.0, 2.0, "`ff` ends");
    // Not Chrome's: the hanging spaces after the first take the shares parley
    // counts beyond CSS's (two here, of 7.94 each), so a caret between them
    // is that much right of Chrome's 54.5 and 59. They are past the box's
    // edge; nothing is drawn there.
    assert_at(
        &d,
        b,
        6,
        62.44,
        0.0,
        "the first hanging space ends (Chrome: 54.5)",
    );
}

/// Two hanging spaces, two word gaps. Chrome: each gap 16.05 wide, `c` ends at
/// 60. On `main` the first hanging space took a third of the free space and
/// `c` ended at 52.31.
#[test]
fn two_hanging_spaces_take_no_share() {
    let (d, b) = block("60px", "pre-wrap", &[("", "a b c  dd")]);
    assert_at(&d, b, 2, 25.02, 0.0, "`b` starts");
    assert_at(&d, b, 5, 60.0, 0.0, "`c` ends");
}

/// Two spaces between words both count, each taking a share. Chrome: 6.22
/// each, `bb` ends at 50. On `main`: 48.85.
#[test]
fn every_space_of_a_word_gap_is_an_opportunity() {
    let (d, b) = block("50px", "pre-wrap", &[("", "aa  bb   cc")]);
    assert_at(&d, b, 3, 24.19, 0.0, "after the gap's first space");
    assert_at(&d, b, 6, 50.0, 0.0, "`bb` ends");
}

/// A line parley ended by hanging the one space that did not fit: that space
/// is not counted, and its "less one" took the word gap away, so the line was
/// not justified at all. Chrome: `bb` ends flush at 44 in every white space
/// mode. On `main`: 42.27, start-aligned.
#[test]
fn a_line_ended_by_hanging_its_only_space_keeps_its_word_gap() {
    for ws in ["normal", "pre-wrap"] {
        let (d, b) = block("44px", ws, &[("", "aa bb cc dd")]);
        assert_at(&d, b, 3, 24.41, 0.0, &format!("{ws}: `bb` starts"));
        assert_at(&d, b, 5, 44.0, 0.0, &format!("{ws}: `bb` ends"));
    }
    // Two spaces, the second the one that did not fit.
    let (d, b) = block("44px", "pre-wrap", &[("", "aa bb  cc dd")]);
    assert_at(&d, b, 5, 44.0, 0.0, "two spaces: `bb` ends");
}

/// The same with more word gaps: one of three lost its share. Chrome: `bb`
/// starts at 22.69 and `cc` ends at 70. On `main` `bb` started at 22.80.
#[test]
fn a_hung_space_takes_no_word_gap_away() {
    let (d, b) = block("70px", "pre-wrap", &[("", "aa bb  cc dd  ee ff gg hh")]);
    assert_at(&d, b, 3, 22.69, 0.0, "`bb` starts");
    assert_at(&d, b, 5, 42.28, 0.0, "`bb` ends");
    assert_at(&d, b, 9, 70.0, 0.0, "`cc` ends");
    assert_at(&d, b, 14, 33.28, 1.0, "`ee` starts");
}

/// NBSPs do not hang: they are content, and the last one, ending the content,
/// is not an opportunity. Chrome: `c` ends at 46.31, the first NBSP is widened
/// to 9.2 and the second keeps its 4.5, ending at the box's edge. On `main`
/// both were widened, as content that hangs, and `c` ended at 48.46.
///
/// Not Chrome's: parley gives its shares to the first spaces of the line, as
/// many as it counts, and it counts one more than CSS here (the hanging space
/// after the NBSPs). The surplus share lands on the second NBSP, which ends at
/// 64.69 where Chrome's ends at 60. It is white space: only a caret after it
/// shows it.
#[test]
fn trailing_nbsps_are_content_and_the_last_takes_no_share() {
    let (d, b) = block("60px", "pre-wrap", &[("", "a b c\u{a0}\u{a0} dd")]);
    assert_at(&d, b, 2, 18.19, 0.0, "`b` starts");
    assert_at(&d, b, 5, 46.31, 0.0, "`c` ends");
    assert_at(&d, b, 7, 55.5, 0.0, "the first NBSP ends");
    assert_at(&d, b, 9, 64.69, 0.0, "the second NBSP ends (Chrome: 60)");
}

/// The hanging spaces are a run of their own (a larger font): they still take
/// no share. Chrome: `bb` ends at 50. On `main`: 46.03.
#[test]
fn hanging_spaces_in_a_run_of_their_own_take_no_share() {
    let (d, b) = block(
        "50px",
        "pre-wrap",
        &[("", "aa bb"), ("font-size: 20px", "   "), ("", "cc dd")],
    );
    assert_at(&d, b, 3, 30.41, 0.0, "`bb` starts");
    assert_at(&d, b, 5, 50.0, 0.0, "`bb` ends");
}

/// The issue's editor text: three spaces after each sentence. Chrome: each
/// line ends flush at 120. On `main` `three.` ended at 115.8.
#[test]
fn sentences_ending_in_three_spaces_justify_flush() {
    let s = "One two three.   Four five six.   Seven eight nine ten.";
    let (d, b) = block("120px", "pre-wrap", &[("", s)]);
    assert_at(&d, b, "One two".len(), 67.58, 0.0, "`two` ends");
    assert_at(&d, b, "One two three.".len(), 120.0, 0.0, "`three.` ends");
    assert_at(
        &d,
        b,
        "One two three.   Four".len(),
        33.89,
        1.0,
        "`Four` ends",
    );
    assert_at(
        &d,
        b,
        s.find("six.").unwrap() + 4,
        120.0,
        1.0,
        "`six.` ends",
    );
    assert_at(
        &d,
        b,
        s.find("eight").unwrap() + 5,
        120.0,
        2.0,
        "`eight` ends",
    );
}

/// `break-spaces` spaces never hang: every one is an opportunity but the last
/// on the line. Already Chrome's before the fix (they reach parley's count as
/// content); the control that the fix keeps them so. Chrome: the second line
/// starts with two spaces 13.61 wide, `cc` ends at 45.5 and its space at 50.
#[test]
fn break_spaces_count_all_but_the_last() {
    let (d, b) = block("50px", "break-spaces", &[("", "aa bb   cc dd   ee ff gg")]);
    assert_at(&d, b, 5, 45.5, 0.0, "`bb` ends");
    assert_at(&d, b, 7, 13.61, 1.0, "the first space of line two ends");
    assert_at(&d, b, 8, 27.22, 1.0, "the second space of line two ends");
    assert_at(&d, b, 10, 45.5, 1.0, "`cc` ends");
    let (d, b) = block("60px", "break-spaces", &[("", "a b c  dd")]);
    assert_at(&d, b, 5, 46.31, 0.0, "`c` ends");
    assert_at(&d, b, 6, 55.5, 0.0, "the first trailing space ends");
}

/// A line holding no opportunity but its hanging spaces is start-aligned, its
/// spaces at their own width. Chrome: the three spaces after `aa` end at 31.47.
#[test]
fn a_line_with_only_hanging_spaces_is_start_aligned() {
    let (d, b) = block("50px", "pre-wrap", &[("", "aa   bbbbbbbbbbb")]);
    assert_at(&d, b, 2, 17.97, 0.0, "`aa` ends");
    assert_at(&d, b, 3, 22.47, 0.0, "the first space ends");
}

/// The same line aligned right (#1042's narrowing, which reads parley's
/// trailing white space from the line's last shaped run). The hanging spaces
/// are one run, but the table the hang pass works from had them on two lines
/// — parley put the space that overflowed on its line and the rest on the
/// next — and a run's `Run::index` is its place on its line, so they looked
/// like two runs and the line was aligned 11.25px past the box's edge. Chrome:
/// `bb` ends at 50.
#[test]
fn right_aligned_hanging_spaces_in_a_run_of_their_own_hang() {
    let mut d = doc();
    let body = d.body();
    let b = d.create_element("div");
    d.set_attribute(
        b,
        "style",
        "width: 50px; white-space: pre-wrap; text-align: right; \
         font-family: ProbeFace; font-size: 16px; line-height: 25px",
    );
    d.append_child(body, b);
    let t = d.create_text("aa bb");
    d.append_child(b, t);
    let span = d.create_element("span");
    d.set_attribute(span, "style", "font-size: 20px");
    d.append_child(b, span);
    let t = d.create_text("   ");
    d.append_child(span, t);
    let t = d.create_text("cc dd");
    d.append_child(b, t);
    d.resolve_layout(400.0, 300.0);
    assert_at(&d, b, 0, 7.94, 0.0, "`aa` starts");
    assert_at(&d, b, 5, 50.0, 0.0, "`bb` ends");
}

/// A justified line after a forced break: the table's cursor starts on the
/// zero-width newline that ended the line before, which `justify_line` steps
/// over before it commits the line again by length (review of #1680). Chrome:
/// on `pre-line` line two, `six` ends at 57.39 and `eight` at 150; on
/// `pre-wrap` line two, `dd` starts at 40.41 and ends flush at 60.
#[test]
fn a_justified_line_after_a_forced_break() {
    let s = "One two three four.\nFive six seven eight nine ten eleven.";
    let (d, b) = block("150px", "pre-line", &[("", s)]);
    assert_at(&d, b, s.find("six").unwrap() + 3, 57.39, 1.0, "`six` ends");
    assert_at(
        &d,
        b,
        s.find("eight").unwrap() + 5,
        150.0,
        1.0,
        "`eight` ends",
    );
    let (d, b) = block("60px", "pre-wrap", &[("", "aa bb\ncc dd   ee ff gg")]);
    assert_at(&d, b, 9, 40.41, 1.0, "`dd` starts");
    assert_at(&d, b, 11, 60.0, 1.0, "`dd` ends");
}

/// A left-to-right block whose text starts with Hebrew: parley's base
/// direction is then right-to-left, and it offsets and justifies such a line
/// from its left end, so a widened alignment width would push it past the
/// box's right edge (113px in a 90px box, review of #1680). The fix leaves it
/// as it was. Asserted on the painted glyph runs, not on Chrome's numbers:
/// the Hebrew comes from the host's fallback face.
#[test]
fn a_right_to_left_paragraph_is_not_widened() {
    let (d, b) = block(
        "90px",
        "pre-wrap",
        &[("", "שלום עולם   אב גד   הו זח טי כל")],
    );
    let il = d.tree.nodes[b.0]
        .text_layout
        .as_ref()
        .expect("a text layout");
    let line = il.layout.lines().next().expect("a first line");
    let mut runs = 0;
    for item in line.items() {
        if let parley::layout::PositionedLayoutItem::GlyphRun(gr) = item {
            runs += 1;
            let end = gr.offset() + gr.advance();
            assert!(
                end <= 90.5,
                "a glyph run on line one ends at {end}, past the box's 90"
            );
        }
    }
    assert!(runs > 0, "line one has glyph runs");
}
