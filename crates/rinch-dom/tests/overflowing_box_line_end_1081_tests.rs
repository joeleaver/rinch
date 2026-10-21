//! #1081 — a forced break or hangable white space right after an atomic
//! inline too wide for any line ends that box's line; it does not get a line
//! of its own.
//!
//! Parley 0.11.1 places a box too wide for any line through its *emergency*
//! branch: at the start of a line it consumes the box and commits the line on
//! the spot. Whatever follows is the next line's — so a `<br>` (pushed as
//! `"\n"`), a `pre-wrap` newline, or spaces after the box made a line of their
//! own, where CSS puts them on the box's line: a forced break ends the line it
//! is on, and white space at a soft wrap hangs (or, collapsible, is removed).
//! Spaces a forced break follows are the exception: Chrome puts them and the
//! break on a line of their own, as rinch always did.
//!
//! Chrome 155's heights (`font-size: 16px; line-height: 20px`, a 200px block,
//! a `300x80` `inline-block`; measured 2026-10-21, `getBoundingClientRect`):
//!
//! | shape                                | Chrome 155 |
//! |--------------------------------------|-----------:|
//! | `x <ib><br>`                         | 105 |
//! | `<ib><br>`                           |  85 |
//! | `x <ib><br><br>`                     | 125 |
//! | `pre-wrap`, `x <ib>\n`               | 105 |
//! | `pre-wrap`, `x <ib>\n\n`             | 125 |
//! | `x<br><ib><br>y`                     | 125 |
//! | `pre-wrap`, `x <ib> ` / `x <ib>   `  | 105 |
//! | `x <ib> y`, `x <ib><br>y`, `x <ib>y` | 125 (`y` at 106) |
//! | `pre-wrap`, `x <ib> y` / `x <ib>\ny` | 125 (`y` at 106) |
//! | `pre-wrap`, `x <ib>  \n`              | 125 (spaces and break: a line of their own) |
//! | `pre-wrap`, `x <ib>  \ny`             | 145 |
//!
//! rinch is 5px short of every one: a line holding only an atomic inline gets
//! no strut (#624), so the box's line is 80 and not 85. The fixtures pin the
//! part this issue is about — how many lines follow the box — as the box's
//! bottom edge plus 20px per line, with the strut's 5px of slack.
//!
//! Every fixture registers the bundled Inter and declares `line-height`.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const CONTAINER: &str = "width: 200px; font-family: ProbeFace; font-size: 16px; line-height: 20px";
const STRUT_SLACK: f32 = 5.0;

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

/// One piece of the paragraph after the chip.
enum Part<'a> {
    Text(&'a str),
    Br,
}

/// `"x "`, the text before the chip in most fixtures.
const X: &[Part] = &[Part::Text("x ")];

fn add_parts(d: &mut RinchDocument, c: NodeId, parts: &[Part]) {
    for p in parts {
        match p {
            Part::Text(s) => {
                let t = d.create_text(s);
                d.append_child(c, t);
            }
            Part::Br => {
                el(d, c, "br", "");
            }
        }
    }
}

/// `container { before <span ib WxH> after… }`.
fn build(
    ws: &str,
    before: &[Part],
    chip_w: f32,
    after: &[Part],
) -> (RinchDocument, NodeId, NodeId) {
    let mut d = doc();
    let body = d.body();
    let c = el(
        &mut d,
        body,
        "div",
        &format!("{CONTAINER}; white-space: {ws}"),
    );
    add_parts(&mut d, c, before);
    let s = el(
        &mut d,
        c,
        "span",
        &format!("display: inline-block; width: {chip_w}px; height: 80px"),
    );
    add_parts(&mut d, c, after);
    d.resolve_layout(400.0, 300.0);
    (d, c, s)
}

fn boxof(doc: &RinchDocument, id: NodeId) -> (f32, f32, f32, f32) {
    let n = doc.tree.get(id.0).unwrap();
    (n.layout.x, n.layout.y, n.layout.width, n.layout.height)
}

/// The paragraph ends `lines_after` 20px lines below the chip's bottom edge
/// (and at most the strut's 5px further).
fn assert_lines_after_chip(
    ws: &str,
    before: &[Part],
    after: &[Part],
    lines_after: f32,
    what: &str,
) {
    let (d, c, s) = build(ws, before, 300.0, after);
    let (_, sy, _, sh) = boxof(&d, s);
    let expected_top = if before.is_empty() { 0.0 } else { 20.0 };
    assert_eq!(sy, expected_top, "{what}: the chip's line");
    let h = boxof(&d, c).3;
    let bottom = sy + sh + 20.0 * lines_after;
    assert!(
        h >= bottom - 0.01 && h <= bottom + STRUT_SLACK + 0.01,
        "{what}: the paragraph ends {lines_after} line(s) below the chip ({bottom}, \
         Chrome {}) — got {h}",
        bottom + STRUT_SLACK,
    );
}

#[test]
fn a_br_after_an_overflowing_chip_ends_the_chips_line() {
    assert_lines_after_chip("normal", X, &[Part::Br], 0.0, "`x <ib><br>`");
}

#[test]
fn a_br_after_an_overflowing_chip_alone_ends_its_line() {
    assert_lines_after_chip("normal", &[], &[Part::Br], 0.0, "`<ib><br>`");
}

#[test]
fn a_second_br_after_an_overflowing_chip_is_one_more_line() {
    assert_lines_after_chip("normal", X, &[Part::Br, Part::Br], 1.0, "`x <ib><br><br>`");
}

#[test]
fn a_pre_wrap_newline_after_an_overflowing_chip_ends_its_line() {
    assert_lines_after_chip(
        "pre-wrap",
        X,
        &[Part::Text("\n")],
        0.0,
        "pre-wrap `x <ib>\\n`",
    );
}

#[test]
fn two_pre_wrap_newlines_after_an_overflowing_chip_are_one_more_line() {
    assert_lines_after_chip(
        "pre-wrap",
        X,
        &[Part::Text("\n\n")],
        1.0,
        "pre-wrap `x <ib>\\n\\n`",
    );
}

#[test]
fn a_pre_wrap_space_after_an_overflowing_chip_hangs() {
    assert_lines_after_chip("pre-wrap", X, &[Part::Text(" ")], 0.0, "pre-wrap `x <ib> `");
}

/// Off the one-space fixed point: every space of the run hangs.
#[test]
fn a_run_of_pre_wrap_spaces_after_an_overflowing_chip_hangs() {
    assert_lines_after_chip(
        "pre-wrap",
        X,
        &[Part::Text("   ")],
        0.0,
        "pre-wrap `x <ib>   `",
    );
}

/// Spaces or a tab that a newline follows are not folded: Chrome 155 puts
/// them, and the break, on a line of their own after the chip's (`x <ib>  \n`
/// is 124 there with this face — the chip's line ends at the chip). rinch
/// before #1081 already did; a fold of the spaces and the newline got 100.
#[test]
fn pre_wrap_spaces_then_a_newline_after_an_overflowing_chip_take_a_line_of_their_own() {
    for (before, after, lines, what) in [
        (X, &[Part::Text("  \n")][..], 1.0, "pre-wrap `x <ib>  \\n`"),
        (X, &[Part::Text("\t\n")][..], 1.0, "pre-wrap `x <ib>\\t\\n`"),
        (
            X,
            &[Part::Text("  \ny")][..],
            2.0,
            "pre-wrap `x <ib>  \\ny`",
        ),
        (
            X,
            &[Part::Text("  \n\ny")][..],
            3.0,
            "pre-wrap `x <ib>  \\n\\ny`",
        ),
        (
            &[][..],
            &[Part::Text("  \ny")][..],
            2.0,
            "pre-wrap `<ib>  \\ny`",
        ),
        // The editor's shape: a real `<br>` after the spaces.
        (
            &[Part::Text("ab ")][..],
            &[Part::Text("  "), Part::Br, Part::Text("cd")][..],
            2.0,
            "pre-wrap `ab <ib>  <br>cd`",
        ),
    ] {
        assert_lines_after_chip("pre-wrap", before, after, lines, what);
    }
}

/// A chip right after a forced break: the table's cursor then sits on the
/// previous line's newline, which the fold must not count as the chip line's
/// (counting it took one unit too many and swallowed `y` onto the chip's
/// line). Chrome 155: `x<br><ib><br>y` 124 with this face.
#[test]
fn a_chip_right_after_a_forced_break_folds_only_its_own_break() {
    for (ws, before, after, what) in [
        (
            "normal",
            &[Part::Text("x"), Part::Br][..],
            &[Part::Br, Part::Text("y")][..],
            "`x<br><ib><br>y`",
        ),
        (
            "pre-wrap",
            &[Part::Text("x\n")][..],
            &[Part::Text("\ny")][..],
            "pre-wrap `x\\n<ib>\\ny`",
        ),
        (
            "pre-wrap",
            &[Part::Text("x\n")][..],
            &[Part::Text(" y")][..],
            "pre-wrap `x\\n<ib> y`",
        ),
    ] {
        assert_lines_after_chip(ws, before, after, 1.0, what);
    }
}

/// Controls: text after the chip is a line of its own, whatever comes
/// between. Kills a fold that swallows the line after it whatever it holds.
#[test]
fn text_after_an_overflowing_chip_keeps_its_line() {
    for (ws, after, what) in [
        ("normal", &[Part::Text(" y")][..], "`x <ib> y`"),
        ("normal", &[Part::Br, Part::Text("y")][..], "`x <ib><br>y`"),
        ("normal", &[Part::Text("y")][..], "`x <ib>y`"),
        ("pre-wrap", &[Part::Text(" y")][..], "pre-wrap `x <ib> y`"),
        (
            "pre-wrap",
            &[Part::Text("\ny")][..],
            "pre-wrap `x <ib>\\ny`",
        ),
        (
            "pre-wrap",
            &[Part::Text("   y")][..],
            "pre-wrap `x <ib>   y`",
        ),
    ] {
        assert_lines_after_chip(ws, X, after, 1.0, what);
    }
}

/// `y` after a hung space starts its line at the line's start, not after the
/// space (Chrome 155: `y` at x 0 of line 2). With nothing before the chip:
/// before #1081 `y` was drawn after the space, at 4.5.
#[test]
fn text_after_a_hung_space_starts_at_the_line_start() {
    for ws in ["normal", "pre-wrap"] {
        let (d, c, _) = build(ws, &[], 300.0, &[Part::Text(" y")]);
        let layout = &d
            .tree
            .get(c.0)
            .unwrap()
            .text_layout
            .as_ref()
            .unwrap()
            .layout;
        let text_len = layout.lines().last().unwrap().text_range().end;
        let (x, y) = rinch_dom::text_query::caret_position_for_offset_layout(layout, text_len - 1);
        assert!(y >= 79.0, "{ws}: `y` is on line 2 — got y {y}");
        assert!(x.abs() < 0.01, "{ws}: `y` starts the line — got x {x}");
    }
}

/// Control: a chip that fits beside `x ` was never placed by the emergency
/// branch; the `<br>` ends its line (Chrome 85).
#[test]
fn a_fitting_chip_and_its_br_share_a_line() {
    let (d, c, s) = build("normal", X, 150.0, &[Part::Br]);
    // rinch puts a line's tallest box a couple of px above the line's top
    // (#663); on line 1 either way, not wrapped below `x `.
    assert!(
        boxof(&d, s).1 < 10.0,
        "the chip fits beside `x ` — top {}",
        boxof(&d, s).1
    );
    let h = boxof(&d, c).3;
    assert!((80.0..=85.01).contains(&h), "one line — got {h}");
}
