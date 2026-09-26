//! #1050 — an atomic inline too wide for its line, wrapped onto a line of its
//! own, must not leave an extra line box behind it.
//!
//! Parley 0.11.1's line breaker places such a box through its *emergency*
//! branch: at the start of a line a box that can never fit is consumed and
//! the line committed at once. When that box is the paragraph's last item the
//! breaker is not marked done, so the next `break_next` commits one more,
//! empty line. That line carries an empty text-run item (when any text came
//! before the box), so the breaker's own "an empty last line adds no height"
//! rule does not apply, and its height — copied from the box's line — was
//! counted: a 300x80 box after `"x "` in a 200px container made a 180px
//! paragraph where Chrome makes 105. The same shape at parley's `main`
//! (checked 2026-09-26) still returns from that branch without setting `done`.
//!
//! Chrome 153's numbers, from the issue (`font-size: 16px; line-height: 20px`,
//! 200px wide, a `300x80` `inline-block` after `"x "`): the paragraph's
//! `scrollHeight` is **105**, and **155** after a 100x50 block (an anonymous
//! block box then holds the run). rinch's answer is 5px short of both: a line
//! holding only an atomic inline gets no strut, so the box's line is 80 and
//! not 85 — that is #624 / #663, not this issue. The fixtures therefore pin
//! the part #1050 is about, that nothing follows the box's line: the paragraph
//! ends between the box's bottom edge and Chrome's number.
//!
//! Every fixture registers the bundled Inter and declares `line-height`, so no
//! number here depends on the host's fonts.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const CONTAINER: &str = "width: 200px; font-family: ProbeFace; font-size: 16px; line-height: 20px";
/// Chrome 153 adds the strut's descent below a line holding only an atomic
/// inline; rinch does not (#624). The most a correct answer may exceed the
/// box's bottom edge by, here.
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

fn text(doc: &mut RinchDocument, parent: NodeId, s: &str) {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
}

fn boxof(doc: &RinchDocument, id: NodeId) -> (f32, f32, f32, f32) {
    let n = doc.tree.get(id.0).unwrap();
    (n.layout.x, n.layout.y, n.layout.width, n.layout.height)
}

fn line_count(doc: &RinchDocument, root: NodeId) -> usize {
    doc.tree
        .get(root.0)
        .unwrap()
        .text_layout
        .as_ref()
        .expect("an IFC root")
        .layout
        .len()
}

/// `container { [block 100x50] before <span ib WxH> after }`.
fn build(
    container_extra: &str,
    block_first: bool,
    before: &str,
    chip: (f32, f32),
    after: &str,
) -> (RinchDocument, NodeId, NodeId) {
    let mut d = doc();
    let body = d.body();
    let c = el(
        &mut d,
        body,
        "div",
        &format!("{CONTAINER}; {container_extra}"),
    );
    if block_first {
        el(&mut d, c, "div", "width: 100px; height: 50px");
    }
    if !before.is_empty() {
        text(&mut d, c, before);
    }
    let s = el(
        &mut d,
        c,
        "span",
        &format!(
            "display: inline-block; width: {}px; height: {}px",
            chip.0, chip.1
        ),
    );
    if !after.is_empty() {
        text(&mut d, c, after);
    }
    d.resolve_layout(400.0, 300.0);
    (d, c, s)
}

fn assert_ends_at_the_chip(d: &RinchDocument, c: NodeId, s: NodeId, offset: f32, what: &str) {
    let (_, _, _, h) = boxof(d, c);
    let (_, sy, _, sh) = boxof(d, s);
    let bottom = offset + sy + sh;
    assert!(
        h >= bottom - 0.01 && h <= bottom + STRUT_SLACK + 0.01,
        "{what}: the container ends between the chip's bottom edge ({bottom}) and \
         Chrome's {} — got {h}",
        bottom + STRUT_SLACK,
    );
}

/// The issue's first row: an IFC root holding `"x "` and a 300x80 chip that
/// wraps onto line 2 and overflows it. Chrome: 105. rinch was 180.
#[test]
fn an_overflowing_chip_wrapped_to_its_own_line_adds_no_line_after_it() {
    let (d, c, s) = build("", false, "x ", (300.0, 80.0), "");
    assert_eq!(
        boxof(&d, s).1,
        20.0,
        "the chip is on line 2, below one 20px line"
    );
    assert_ends_at_the_chip(&d, c, s, 0.0, "text then chip");
    assert_eq!(line_count(&d, c), 2, "a line for `x `, a line for the chip");
}

/// Off the trailing space #1052 reworked: no space before the box, the same
/// phantom line.
#[test]
fn the_phantom_line_does_not_need_a_space_before_the_chip() {
    let (d, c, s) = build("", false, "x", (300.0, 80.0), "");
    assert_ends_at_the_chip(&d, c, s, 0.0, "`x` then chip");
}

/// The issue's second row: after a block, the run is laid out by an anonymous
/// block box. Chrome: 155. rinch was 230 (an anonymous box 180 tall).
#[test]
fn an_anonymous_block_box_holding_the_chip_ends_at_the_chip() {
    let (d, c, s) = build("", true, "x ", (300.0, 80.0), "");
    // The chip's box is relative to the anonymous box, which starts under the
    // 50px block.
    assert_ends_at_the_chip(&d, c, s, 50.0, "block, then text and chip");
}

/// `white-space: pre-wrap` breaks through the hanging-space pass (#1018), a
/// second route to the same breaker.
#[test]
fn a_pre_wrap_paragraph_ends_at_the_chip_too() {
    let (d, c, s) = build("white-space: pre-wrap", false, "x  ", (300.0, 80.0), "");
    assert_ends_at_the_chip(&d, c, s, 0.0, "pre-wrap text then chip");
    assert_eq!(line_count(&d, c), 2);
}

/// Control: content after the chip is a real line and stays. Kills a repair
/// that drops the last line after an emergency break without asking whether
/// it holds anything.
#[test]
fn text_after_an_overflowing_chip_keeps_its_line() {
    let (d, c, s) = build("", false, "x ", (300.0, 80.0), " y");
    assert_eq!(line_count(&d, c), 3, "`x `, the chip, `y`");
    let (_, _, _, h) = boxof(&d, c);
    let (_, sy, _, sh) = boxof(&d, s);
    assert!(
        h >= sy + sh + 20.0 - 0.01,
        "a 20px line for `y` below the chip (chip bottom {}), got {h}",
        sy + sh
    );
}

/// Control: a chip that wraps but fits its line never took the emergency
/// branch, and was always right — so a fixture shaped like the others that
/// passes here is measuring the right thing.
#[test]
fn a_chip_that_fits_its_wrapped_line_was_always_right() {
    let (d, c, s) = build("", false, "xxxxxxxxxxxxxxxxxxx ", (150.0, 80.0), "");
    assert_eq!(boxof(&d, s).1, 20.0, "wrapped to line 2");
    assert_ends_at_the_chip(&d, c, s, 0.0, "fitting chip");
}

/// A box alone was already the right height (its trailing line held no item
/// at all, which the breaker does leave out of the height); now it is one line.
#[test]
fn an_overflowing_chip_alone_is_one_line() {
    let (d, c, s) = build("", false, "", (300.0, 80.0), "");
    assert_ends_at_the_chip(&d, c, s, 0.0, "chip alone");
    assert_eq!(line_count(&d, c), 1);
}
