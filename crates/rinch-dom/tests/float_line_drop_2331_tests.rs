//! A line that does not fit beside a float is moved down below it, and its
//! block counts the move in its height (#2331). Each number is Chrome 156's
//! for the same markup in a 560px column.
//!
//! - A float the paragraph places at the start of a line (it stands first in
//!   the line) can leave the line too little room for its first piece of
//!   content: the line goes below the float, and the block is as tall as the
//!   float's height and the line together. The line was moved in the paint
//!   layout but not in the measure that sizes the block, which was as tall
//!   as the line alone (a cell 38 tall where Chrome gives 62).
//! - A `nowrap` run is one piece. parley breaks it after every box, and the
//!   line breaker's trial break did too, so a run too wide for the room
//!   beside a float stayed beside it, overflowing; and the out-of-flow box of
//!   a float standing before the run let the CSS line end right after it,
//!   which made an empty line and put the run on a line of its own below.
//!
//! The content is sized `vertical-align: top` inline-blocks in 20px lines,
//! so no number here depends on the host's fonts.
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const CSS: &str =
    "body { margin: 0; font-family: sans-serif; font-size: 16px; line-height: 20px; }";

fn doc(html: &str) -> RinchDocument {
    let mut d = RinchDocument::new();
    d.load_css(CSS);
    let body = d.body();
    d.set_inner_html(body, &format!("<div style='width:560px'>{html}</div>"));
    d.resolve_layout(800.0, 800.0);
    d
}

/// The border box of `#id`, in page coordinates.
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

/// Within half a pixel: Chrome lays out in fractions, rinch rounds.
#[track_caller]
fn near(got: (f32, f32, f32, f32), chrome: (f32, f32, f32, f32)) {
    let off = [
        got.0 - chrome.0,
        got.1 - chrome.1,
        got.2 - chrome.2,
        got.3 - chrome.3,
    ];
    assert!(
        off.iter().all(|d| d.abs() <= 0.5),
        "rinch {got:?}, Chrome {chrome:?}"
    );
}

/// An inline-block `w` wide on a 20px line, with an id when given one.
fn ib(w: u32, id: &str) -> String {
    let id = if id.is_empty() {
        String::new()
    } else {
        format!("id={id} ")
    };
    format!(
        "<span {id}style='display:inline-block;vertical-align:top;width:{w}px;height:20px'></span>"
    )
}

fn float(side: &str, w: u32, h: u32) -> String {
    format!("<div style='float:{side};width:{w}px;height:{h}px'></div>")
}

// --- A float placed at the start of a line -----------------------------

/// The issue's first shape: 311px of content in the 249px beside a 307x36
/// float. Chrome: the line at 36, the div 56 tall (it was 20).
#[test]
fn a_line_too_wide_beside_its_own_float_goes_below_it_and_counts() {
    let d = doc(&format!(
        "<div id=a style='width:556px'>{}{}</div>",
        float("left", 307, 36),
        ib(311, "ib")
    ));
    near(abs(&d, "a"), (0.0, 0.0, 556.0, 56.0));
    near(abs(&d, "ib"), (0.0, 36.0, 311.0, 20.0));
}

/// The same in an auto table cell (1px padding): Chrome's cell is 556x58.
#[test]
fn in_an_auto_cell() {
    let d = doc(&format!(
        "<table><tr><td id=c>{}{}</td></tr></table>",
        float("left", 307, 36),
        ib(311, "ib")
    ));
    near(abs(&d, "c"), (2.0, 2.0, 556.0, 58.0));
}

/// A 300px cell holding a 290x117 float and 70px of content: Chrome 302x139,
/// the line at 117.
#[test]
fn in_a_fixed_width_cell() {
    let d = doc(&format!(
        "<table><tr><td id=c style='width:300px'>{}{}</td></tr></table>",
        float("left", 290, 117),
        ib(70, "ib")
    ));
    near(abs(&d, "c"), (2.0, 2.0, 302.0, 139.0));
}

/// Content before the float keeps the line where it is: the float goes to
/// the line's start beside it (it fits), and what no longer fits wraps to
/// the next line, below the float. Chrome: `ib0` at x 300, the rest at 47.
#[test]
fn content_before_the_float_stays_and_the_rest_wraps_below() {
    let d = doc(&format!(
        "<div id=b style='width:400px'>{}{}{}</div>",
        ib(50, "ib0"),
        float("left", 300, 47),
        ib(150, "ib")
    ));
    near(abs(&d, "b"), (0.0, 0.0, 400.0, 67.0));
    near(abs(&d, "ib0"), (300.0, 0.0, 50.0, 20.0));
    near(abs(&d, "ib"), (0.0, 47.0, 150.0, 20.0));
}

/// Two floats leave 10px, then 100px from 23 down: 150px fits only below
/// both. Chrome: the line at 47.
#[test]
fn moved_past_every_band_it_does_not_fit_beside() {
    let d = doc(&format!(
        "<div id=b style='width:400px'>{}{}{}</div>",
        float("left", 300, 47),
        float("right", 90, 23),
        ib(150, "ib")
    ));
    near(abs(&d, "b"), (0.0, 0.0, 400.0, 67.0));
    near(abs(&d, "ib"), (0.0, 47.0, 150.0, 20.0));
}

/// A float that clears the one before it leaves 150px from 17 to 36, still
/// too little for 170px of content. Chrome: the line at 36, the div 56 tall.
#[test]
fn below_a_cleared_float() {
    let d = doc(&format!(
        "<div id=b style='width:400px'>{}\
         <div style='float:left;clear:left;width:250px;height:19px'></div>{}</div>",
        float("left", 300, 17),
        ib(170, "ib")
    ));
    near(abs(&d, "b"), (0.0, 0.0, 400.0, 56.0));
    near(abs(&d, "ib"), (0.0, 36.0, 170.0, 20.0));
}

/// **Control**: content that fits beside the float stays beside it, and the
/// div is as tall as its line (the float overflows it). Chrome: x 107, 20.
#[test]
fn content_that_fits_beside_the_float_stays_beside_it() {
    let d = doc(&format!(
        "<div id=b style='width:556px'>{}{}</div>",
        float("left", 107, 36),
        ib(211, "ib")
    ));
    near(abs(&d, "b"), (0.0, 0.0, 556.0, 20.0));
    near(abs(&d, "ib"), (107.0, 0.0, 211.0, 20.0));
}

// --- A `nowrap` run beside a float --------------------------------------

/// The issue's second shape: a float, then 603px of `nowrap` content. Chrome
/// puts the run below the float, on one line (31), and the auto cell is as
/// wide as the run.
#[test]
fn a_nowrap_run_after_a_float_in_an_auto_cell() {
    let d = doc(&format!(
        "<table><tr><td id=c>{}<span style='white-space:nowrap'>{}{}</span></td></tr></table>",
        float("left", 100, 31),
        ib(300, "ib"),
        ib(303, "")
    ));
    near(abs(&d, "c"), (2.0, 2.0, 605.0, 53.0));
}

/// The same in a block: one line of the run, below the float. It was beside
/// the float, overflowing (parley's trial break split the run), or, before
/// that, on a line of its own under an empty line.
#[test]
fn a_nowrap_run_after_a_float_in_a_block() {
    let d = doc(&format!(
        "<div id=b>{}<span style='white-space:nowrap'>{}{}</span></div>",
        float("left", 100, 31),
        ib(300, "ib"),
        ib(303, "")
    ));
    near(abs(&d, "b"), (0.0, 0.0, 560.0, 51.0));
    near(abs(&d, "ib"), (0.0, 31.0, 300.0, 20.0));
}

/// A float that is a sibling of the paragraph (placed by the block, not by
/// the paragraph's line breaker): the 333px run does not fit the 300px
/// beside it. Chrome: the line at 31.
#[test]
fn a_nowrap_run_beside_a_sibling_float() {
    let d = doc(&format!(
        "<div id=b style='width:400px'>{}<p id=p style='margin:0'>\
         <span style='white-space:nowrap'>{}{}</span></p></div>",
        float("left", 100, 31),
        ib(300, "ib"),
        ib(33, "")
    ));
    near(abs(&d, "p"), (0.0, 0.0, 400.0, 51.0));
    near(abs(&d, "ib"), (0.0, 31.0, 300.0, 20.0));
}

/// The second line of a paragraph beside a sibling float: the first fits,
/// the second (a 313px run in 300px) goes below the float. Chrome: at 53.
#[test]
fn a_later_nowrap_line_beside_a_sibling_float() {
    let d = doc(&format!(
        "<div id=b style='width:400px'>{}<p id=p style='margin:0'>{}<br>\
         <span style='white-space:nowrap'>{}{}</span></p></div>",
        float("left", 100, 53),
        ib(50, ""),
        ib(250, "ib"),
        ib(63, "")
    ));
    near(abs(&d, "p"), (0.0, 0.0, 400.0, 73.0));
    near(abs(&d, "ib"), (0.0, 53.0, 250.0, 20.0));
}

/// A word that the float stands in the middle of: `abc` is content of the
/// line's own before the float, so the line stays at the top, the word
/// overflowing the 100px beside the float, and the div is one line tall.
/// Chrome: 20 (`abc` at x 300). Moving it below the float would make 67.
#[test]
fn a_line_with_content_before_its_float_is_not_moved() {
    let d = doc(&format!(
        "<div id=b style='width:400px'>abc{}defghijklmnopqrstuvwxyz</div>",
        float("left", 300, 47)
    ));
    near(abs(&d, "b"), (0.0, 0.0, 400.0, 20.0));
}

/// A `nowrap` run that more content follows: the CSS line ends after the run
/// at a soft wrap, and the run is wider than the room beside the float, so
/// the line still goes below it. Chrome: the run at 31, the next box on the
/// line after it, the div 71 tall.
#[test]
fn a_nowrap_run_with_more_after_it_still_goes_below() {
    let d = doc(&format!(
        "<div id=b>{}<span style='white-space:nowrap'>{}{}</span> {}</div>",
        float("left", 100, 31),
        ib(300, "ib"),
        ib(303, ""),
        ib(50, "")
    ));
    near(abs(&d, "b"), (0.0, 0.0, 560.0, 71.0));
    near(abs(&d, "ib"), (0.0, 31.0, 300.0, 20.0));
}

/// The same with a run that fits the whole width but not the room beside
/// the float (320 in 300): below the float, the next box beside the run.
/// Chrome: at 31, 51 tall.
#[test]
fn a_nowrap_run_too_wide_only_for_the_room_beside_the_float() {
    let d = doc(&format!(
        "<div id=b style='width:400px'>{}<span style='white-space:nowrap'>{}{}</span> {}</div>",
        float("left", 100, 31),
        ib(200, "ib"),
        ib(120, ""),
        ib(50, "")
    ));
    near(abs(&d, "b"), (0.0, 0.0, 400.0, 51.0));
    near(abs(&d, "ib"), (0.0, 31.0, 200.0, 20.0));
}
