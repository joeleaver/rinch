//! Three float-layout faults the #2354 review found. Each number is Chrome
//! 156's for the same markup in a 560px column, with the bundled Inter at
//! 16px/20px where the content is text.
//!
//! - #2360: a line whose first word is wider than the room beside a float is
//!   moved down below the float. parley keeps such a word whole and ends the
//!   line `Regular` at the space after it, and a `Regular` end was taken to
//!   mean the line fit (`ifc::line_overflows_room`); the word overflowed
//!   beside the float.
//! - #2361: a shrink-to-fit box (a float, `width: fit-content`, a grid item
//!   aligned to its start) takes its horizontal margins out of its room once,
//!   not twice.
//! - #2362: a line starts after the rightmost left float it overlaps (and
//!   ends before the leftmost right one), not in a gap a shorter float left
//!   when it ended (CSS 2.2 §9.5).
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const CSS: &str = "body { margin: 0; font-family: InterT; font-size: 16px; line-height: 20px; }";
const TEXT: &str = "alpha beta gamma delta epsilon zeta eta theta iota kappa";

fn doc(html: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut d = RinchDocument::new();
    let registered = d.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("InterT"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    d.load_css(CSS);
    let body = d.body();
    d.set_inner_html(body, &format!("<div style='width:560px'>{html}</div>"));
    d.resolve_layout(800.0, 800.0);
    d
}

/// The border box of `#id`, in page coordinates: x, y, width, height.
fn abs(d: &RinchDocument, id: &str) -> (f32, f32, f32, f32) {
    let n = d.query_selector(&format!("#{id}")).expect(id);
    let l = d.tree.get(n.0).unwrap().layout;
    let (mut x, mut y) = (0.0, 0.0);
    let mut cur = Some(n.0);
    while let Some(i) = cur {
        let node = d.tree.get(i).unwrap();
        x += node.layout.x;
        y += node.layout.y;
        cur = node.parent;
    }
    (x, y, l.width, l.height)
}

/// A value [`near`] does not check.
const ANY: f32 = f32::NAN;

/// Within half a pixel on each of the given values but [`ANY`]: Chrome lays
/// out in fractions, rinch rounds.
#[track_caller]
fn near(got: (f32, f32, f32, f32), chrome: [f32; 4]) {
    let got_a = [got.0, got.1, got.2, got.3];
    assert!(
        got_a
            .iter()
            .zip(chrome)
            .all(|(g, c)| c.is_nan() || (g - c).abs() <= 0.5),
        "rinch (x, y, w, h) {got:?}, Chrome {chrome:?} (NaN: not checked)"
    );
}

/// Where the paint layout of the paragraph `#root` puts the line holding
/// `word`: the line's left edge and top, in page coordinates.
fn line_of(d: &RinchDocument, root: &str, word: &str) -> (f32, f32) {
    let (rx, ry, _, _) = abs(d, root);
    let n = d.query_selector(&format!("#{root}")).expect(root);
    let node = d.tree.get(n.0).unwrap();
    let il = node.text_layout.as_ref().expect("an IFC root");
    let line = il
        .layout
        .lines()
        .find(|l| il.text_content[l.text_range()].contains(word))
        .expect(word);
    let m = line.metrics();
    (rx + m.inline_min_coord + m.offset, ry + m.block_min_coord)
}

/// The line holding `word` starts at (`x`, `y`), within half a pixel.
#[track_caller]
fn line_at(d: &RinchDocument, root: &str, word: &str, x: f32, y: f32) {
    let got = line_of(d, root, word);
    assert!(
        (got.0 - x).abs() <= 0.5 && (got.1 - y).abs() <= 0.5,
        "the line of {word:?}: rinch (x, y) {got:?}, Chrome ({x}, {y})"
    );
}

fn float(side: &str, w: u32, h: u32) -> String {
    format!("<div style='float:{side};width:{w}px;height:{h}px'></div>")
}

/// An inline-block `w` wide on a 20px line.
fn ib(id: &str, w: u32) -> String {
    format!(
        "<span id={id} style='display:inline-block;vertical-align:top;width:{w}px;height:20px'></span>"
    )
}

// --- #2360: a first word too wide for the room beside a float ---------------

/// The issue's shape, off the line grid: the word (140px) does not fit the
/// 50px beside a 250x37 float. Chrome: the word at y 37, the div 57 tall
/// (rinch kept the word beside the float: 40 tall at the issue's 30).
#[test]
fn a_long_first_word_goes_below_a_left_float() {
    let d = doc(&format!(
        "<div id=a style='width:300px'>{}Supercalifragilistic word</div>",
        float("left", 250, 37)
    ));
    near(abs(&d, "a"), [0.0, 0.0, 300.0, 57.0]);
    line_at(&d, "a", "Supercal", 0.0, 37.0);
}

/// The same beside a right float: the word goes below it, at x 0.
#[test]
fn a_long_first_word_goes_below_a_right_float() {
    let d = doc(&format!(
        "<div id=a style='width:300px'>{}Supercalifragilistic word</div>",
        float("right", 250, 37)
    ));
    near(abs(&d, "a"), [0.0, 0.0, 300.0, 57.0]);
    line_at(&d, "a", "Supercal", 0.0, 37.0);
}

/// A paragraph after the float (the floats of its block context, not its
/// own), with more text after the word that wraps again: Chrome puts the
/// word at 37 and the paragraph is 77 tall.
#[test]
fn a_long_first_word_of_a_following_paragraph() {
    let d = doc(&format!(
        "<div style='width:300px'>{}<p id=p style='margin:0'>Supercalifragilistic \
         word and more words here to wrap again</p></div>",
        float("left", 250, 37)
    ));
    near(abs(&d, "p"), [0.0, 0.0, 300.0, 77.0]);
    line_at(&d, "p", "Supercal", 0.0, 37.0);
}

/// A word that fits the first line, then a long word on the second: the
/// second line, still beside the float, goes below it. Chrome: the long
/// word at y 37, the div 57 tall.
#[test]
fn a_long_word_starting_a_later_line_goes_below() {
    let d = doc(&format!(
        "<div id=a style='width:300px'>{}ab Supercalifragilistic word</div>",
        float("left", 250, 37)
    ));
    near(abs(&d, "a"), [0.0, 0.0, 300.0, 57.0]);
    line_at(&d, "a", "Supercal", 0.0, 37.0);
}

/// A long word (140px) beside two left floats, 150x23 and 60x47: it fits
/// neither the 90px beside both nor the 90px right of the taller one after
/// the shorter ends (the 150px gap left of the taller one is no room,
/// #2362). Chrome: the word at x 0, y 47; the div 67 tall.
#[test]
fn a_long_word_goes_below_two_left_floats() {
    let d = doc(&format!(
        "<div id=a style='width:300px'>{}{}Supercalifragilistic x</div>",
        float("left", 150, 23),
        float("left", 60, 47)
    ));
    near(abs(&d, "a"), [0.0, 0.0, 300.0, 67.0]);
    line_at(&d, "a", "Supercal", 0.0, 47.0);
}

/// A line that fits but for the space it hangs stays beside the float: a
/// 100px box in the 100px beside a 200x37 float, then a space. Chrome: the
/// box at x 200, y 0, and the next at x 200, y 20 (the div 40 tall).
#[test]
fn a_line_that_fits_but_for_its_hanging_space_stays_beside() {
    let d = doc(&format!(
        "<div id=a style='width:300px'>{}{} {}</div>",
        float("left", 200, 37),
        ib("b1", 100),
        ib("b2", 50)
    ));
    near(abs(&d, "b1"), [200.0, 0.0, ANY, ANY]);
    near(abs(&d, "b2"), [200.0, 20.0, ANY, ANY]);
    near(abs(&d, "a"), [ANY, ANY, ANY, 40.0]);
}

// --- #2361: shrink-to-fit room and horizontal margins ------------------------

/// The issue's float: `margin: 0 20px` in 300px. Chrome 260x40 (rinch
/// 220x60: the margins taken out twice).
#[test]
fn a_float_with_margins_takes_the_room_less_its_margins_once() {
    let d = doc(&format!(
        "<div style='width:300px'><div id=f style='float:left;margin:0 20px'>{TEXT}</div></div>"
    ));
    near(abs(&d, "f"), [20.0, 0.0, 260.0, 40.0]);
}

/// Unequal margins, 23 left and 17 right, on a left and a right float.
/// Chrome: each 260 wide at x 23.
#[test]
fn floats_with_unequal_margins() {
    for side in ["left", "right"] {
        let d = doc(&format!(
            "<div style='width:300px'><div id=f style='float:{side};margin:0 17px 0 23px'>{TEXT}</div></div>"
        ));
        near(abs(&d, "f"), [23.0, 0.0, 260.0, 40.0]);
    }
}

/// A percentage margin (5% of 300 is 15) beside a length one. Chrome: 262
/// wide at x 23.
#[test]
fn a_float_with_a_percentage_margin() {
    let d = doc(&format!(
        "<div style='width:300px'><div id=f style='float:left;margin:0 5% 0 23px'>{TEXT}</div></div>"
    ));
    near(abs(&d, "f"), [23.0, 0.0, 262.0, 40.0]);
}

/// Margins, padding and a border together: the margins come out of the room
/// once, the border box is what is left. Chrome: 260 wide at x 23, and 66
/// tall (the 240px content wraps to three lines).
#[test]
fn a_float_with_margins_padding_and_border() {
    let d = doc(&format!(
        "<div style='width:300px'><div id=f style='float:left;margin:0 17px 0 23px;\
         padding:0 7px;border:3px solid'>{TEXT}</div></div>"
    ));
    near(abs(&d, "f"), [23.0, 0.0, 260.0, 66.0]);
}

/// `width: fit-content` in flow. Chrome: 260x40 at x 23.
#[test]
fn a_fit_content_block_with_margins() {
    let d = doc(&format!(
        "<div style='width:300px'><div id=f style='width:fit-content;margin:0 17px 0 23px'>{TEXT}</div></div>"
    ));
    near(abs(&d, "f"), [23.0, 0.0, 260.0, 40.0]);
}

/// A grid item aligned to its start. Chrome: 260x40 at x 23.
#[test]
fn a_start_aligned_grid_item_with_margins() {
    let d = doc(&format!(
        "<div style='width:300px;display:grid;justify-items:start'>\
         <div id=f style='margin:0 17px 0 23px'>{TEXT}</div></div>"
    ));
    near(abs(&d, "f"), [23.0, 0.0, 260.0, 40.0]);
}

/// A negative margin gives the room back: `margin: 0 -17px 0 23px` leaves
/// 294. Chrome: 294 wide at x 23 (rinch 288, 6 taken out twice).
#[test]
fn a_fit_content_block_with_a_negative_margin() {
    let d = doc(&format!(
        "<div style='width:300px'><div id=f style='width:fit-content;margin:0 -17px 0 23px'>{TEXT}</div></div>"
    ));
    near(abs(&d, "f"), [23.0, 0.0, 294.0, 40.0]);
}

/// A column flex's items, aligned to the start: a box holding the text in a
/// block of its own was handed the whole width and was 300 wide; one of
/// `fit-content` width had its margins taken out twice (220). Chrome: each
/// 260 wide at x 23.
#[test]
fn column_flex_items_with_margins() {
    for item in [
        format!("<div id=f style='margin:0 17px 0 23px'><div>{TEXT}</div></div>"),
        format!("<div id=f style='margin:0 17px 0 23px;display:grid'><div>{TEXT}</div></div>"),
        format!("<div id=f style='margin:0 17px 0 23px;width:fit-content'>{TEXT}</div>"),
        format!("<div id=f style='margin:0 17px 0 23px'>{TEXT}</div>"),
    ] {
        let d = doc(&format!(
            "<div style='width:300px;display:flex;flex-direction:column;align-items:flex-start'>{item}</div>"
        ));
        near(abs(&d, "f"), [23.0, 0.0, 260.0, 40.0]);
    }
}

/// The two that were already right stay right: an inline-block and an
/// absolutely positioned box (whose margins were put back at the leaf
/// measure, `absolute_leaf_inputs`, until the leaf stopped taking them out).
/// Chrome: 260 wide at x 23. The inline-block's x is not checked: its line
/// ignores an atomic inline's margins (#1405).
#[test]
fn an_inline_block_and_an_abspos_box_with_margins() {
    let d = doc(&format!(
        "<div style='width:300px'><div id=f style='display:inline-block;margin:0 17px 0 23px'>{TEXT}</div></div>"
    ));
    near(abs(&d, "f"), [ANY, 0.0, 260.0, 40.0]);
    let d = doc(&format!(
        "<div style='width:300px;position:relative'>\
         <div id=f style='position:absolute;margin:0 17px 0 23px'>{TEXT}</div></div>"
    ));
    near(abs(&d, "f"), [23.0, 0.0, 260.0, 40.0]);
}

// --- #2362: no line in the gap left of a taller left float --------------------

/// Left floats 150x30 and 60x53 (the second at x 150..210), then three 60px
/// boxes. The third line, at 40, is past the first float but beside the
/// second: it starts at 210, not in the 150px gap at x 0.
#[test]
fn a_line_starts_after_the_taller_left_float() {
    let d = doc(&format!(
        "<div id=a style='width:300px'>{}{}{} {} {}</div>",
        float("left", 150, 30),
        float("left", 60, 53),
        ib("b1", 60),
        ib("b2", 60),
        ib("b3", 60)
    ));
    near(abs(&d, "b1"), [210.0, 0.0, ANY, ANY]);
    near(abs(&d, "b2"), [210.0, 20.0, ANY, ANY]);
    near(abs(&d, "b3"), [210.0, 40.0, ANY, ANY]);
    near(abs(&d, "a"), [ANY, ANY, ANY, 60.0]);
}

/// The issue's own shape: floats 100x30 and 100x50, boxes 60. The gap and
/// the room are both 100 wide. Chrome: the third box at x 200, y 40.
#[test]
fn the_issue_shape() {
    let d = doc(&format!(
        "<div style='width:300px'>{}{}{} {} {}</div>",
        float("left", 100, 30),
        float("left", 100, 50),
        ib("b1", 60),
        ib("b2", 60),
        ib("b3", 60)
    ));
    near(abs(&d, "b3"), [200.0, 40.0, ANY, ANY]);
}

/// A box too wide for the 90px room beside the taller float, but not for
/// the 150px gap: it goes below the floats, to x 0, y 53, and the div is 73
/// tall.
#[test]
fn a_box_too_wide_for_the_room_goes_below_not_into_the_gap() {
    let d = doc(&format!(
        "<div id=a style='width:300px'>{}{}{} {} {}</div>",
        float("left", 150, 30),
        float("left", 60, 53),
        ib("b1", 60),
        ib("b2", 60),
        ib("b3", 120)
    ));
    near(abs(&d, "b3"), [0.0, 53.0, ANY, ANY]);
    near(abs(&d, "a"), [ANY, ANY, ANY, 73.0]);
}

/// The mirror: right floats 150x30 and 60x53 (the second at x 90..150).
/// The third line ends before the taller float, not in the gap right of
/// it. Chrome: every box at x 0.
#[test]
fn a_line_ends_before_the_taller_right_float() {
    let d = doc(&format!(
        "<div style='width:300px'>{}{}{} {} {}</div>",
        float("right", 150, 30),
        float("right", 60, 53),
        ib("b1", 60),
        ib("b2", 60),
        ib("b3", 120)
    ));
    // 120 does not fit the 90px left of the taller float: below, at 53.
    near(abs(&d, "b3"), [0.0, 53.0, ANY, ANY]);
}

/// The floats in a block before the paragraph (its context's floats).
/// Chrome: the third box at x 210, y 40.
#[test]
fn a_following_paragraph_starts_after_the_taller_float() {
    let d = doc(&format!(
        "<div style='width:300px'>{}{}</div><p style='margin:0;width:300px'>{} {} {}</p>",
        float("left", 150, 30),
        float("left", 60, 53),
        ib("b1", 60),
        ib("b2", 60),
        ib("b3", 60)
    ));
    near(abs(&d, "b3"), [210.0, 40.0, ANY, ANY]);
}
