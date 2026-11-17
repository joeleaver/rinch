//! A box with no room lays its text out at every break opportunity (#2382).
//!
//! `width: 0` is a width, and the narrowest there is: Chrome takes every
//! break opportunity the text has, the same lines as `width: 1px`. rinch
//! measured the box that way (its height was five lines) but broke the
//! paint layout with no limit, because the IFC root's content width was
//! read as "unconstrained" when it was not positive — so the glyphs were
//! one max-content line in a box laid out for five.
//!
//! Every expectation is Chrome 156's, on the same markup: the bundled Inter
//! registered as `ProbeFace`, `font: 16px/20px`, `* { margin: 0; padding: 0 }`,
//! lines read from `Range.getClientRects` per character, heights from
//! `getBoundingClientRect`.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// The issue's text: a space, a hyphen before a digit after a letter
/// (`a-5`, an opportunity since #2369) and one after a space (`-5`, none).
const TEXT: &str = "x -5 (-5) a-5";

/// Chrome's lines for [`TEXT`] in a box with no room, `white-space: normal`.
const EVERY_OPPORTUNITY: [&str; 5] = ["x ", "-5 ", "(-5) ", "a-", "5"];

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
    doc.load_css("* { margin: 0; padding: 0 } body { font: 16px/20px ProbeFace }");
    doc
}

/// `markup` in the body, laid out, and the node `#t`.
fn html(markup: &str) -> (RinchDocument, NodeId) {
    let mut d = doc();
    let body = d.body();
    d.set_inner_html(body, markup);
    d.resolve_layout(800.0, 600.0);
    let t = d.query_selector("#t").expect("#t");
    (d, t)
}

/// The text of each line of `node`'s inline layout.
fn lines_of(d: &RinchDocument, node: NodeId) -> Vec<String> {
    let n = d.tree.get(node.0).unwrap();
    let il = n.text_layout.as_ref().expect("an IFC root");
    il.layout
        .lines()
        .map(|l| il.text_content[l.text_range()].to_string())
        .collect()
}

/// Where each line starts, from the content box's left edge.
fn line_starts(d: &RinchDocument, node: NodeId) -> Vec<f32> {
    let n = d.tree.get(node.0).unwrap();
    let il = n.text_layout.as_ref().expect("an IFC root");
    il.layout
        .lines()
        .map(|l| {
            let m = l.metrics();
            m.inline_min_coord + m.offset
        })
        .collect()
}

fn height(d: &RinchDocument, node: NodeId) -> f32 {
    d.tree.get(node.0).unwrap().layout.height
}

/// Lines and border-box height of `#t` in `markup`, against Chrome's.
#[track_caller]
fn assert_chrome(markup: &str, lines: &[&str], chrome_height: f32) {
    let (d, t) = html(markup);
    assert_eq!(lines_of(&d, t), lines, "{markup}");
    assert_eq!(height(&d, t), chrome_height, "{markup}");
}

/// The issue's row. rinch: `x -5 (-5) a-5`, one line, in a box 100px tall.
#[test]
fn a_zero_width_box_breaks_at_every_opportunity() {
    let m = format!(r#"<div id=t style="width:0">{TEXT}</div>"#);
    assert_chrome(&m, &EVERY_OPPORTUNITY, 100.0);
    // The control: 1px was right before the fix, and 0 now agrees with it.
    let m = format!(r#"<div id=t style="width:1px">{TEXT}</div>"#);
    assert_chrome(&m, &EVERY_OPPORTUNITY, 100.0);
}

/// Every way a box gets no room, each one line in rinch before the fix.
#[test]
fn every_zero_room_box_breaks_at_every_opportunity() {
    for m in [
        format!(r#"<div id=t style="max-width:0">{TEXT}</div>"#),
        format!(r#"<div><div id=t style="float:left;width:0">{TEXT}</div></div>"#),
        format!(
            r#"<div style="display:flex;width:100px"><div id=t style="flex:0 1 0;min-width:0">{TEXT}</div><div style="flex:1 0 100px"></div></div>"#
        ),
        format!(r#"<div id=t style="width:0"><span>{TEXT}</span></div>"#),
        format!(
            r#"<div style="position:relative;height:0"><div id=t style="position:absolute;width:0">{TEXT}</div></div>"#
        ),
        format!(r#"<div><div id=t style="display:inline-block;width:0">{TEXT}</div></div>"#),
        format!(
            r#"<div style="display:grid;grid-template-columns:0px"><div id=t>{TEXT}</div></div>"#
        ),
        format!(r#"<div id=t style="width:0;white-space:pre-wrap">{TEXT}</div>"#),
    ] {
        assert_chrome(&m, &EVERY_OPPORTUNITY, 100.0);
    }
}

/// Padding takes all the room: a 4px `border-box` with 5px of padding each
/// side is 10px wide, its content box 0. Chrome 156: each line starts at x=5.
#[test]
fn padding_that_takes_all_the_room_leaves_a_zero_content_box() {
    let (d, t) = html(&format!(
        r#"<div id=t style="box-sizing:border-box;width:4px;padding:0 5px">{TEXT}</div>"#
    ));
    assert_eq!(lines_of(&d, t), EVERY_OPPORTUNITY);
    assert_eq!(height(&d, t), 100.0);
    assert_eq!(d.tree.get(t.0).unwrap().layout.width, 10.0);
}

/// A line wider than its box overflows at the start edge whatever the
/// alignment (CSS Text 3 §7.1); Chrome 156 starts all five at x=0.
#[test]
fn overflowing_lines_start_at_the_start_edge_when_centred_or_right_aligned() {
    for align in ["center", "right"] {
        let (d, t) = html(&format!(
            r#"<div id=t style="width:0;text-align:{align}">{TEXT}</div>"#
        ));
        assert_eq!(lines_of(&d, t), EVERY_OPPORTUNITY, "{align}");
        assert_eq!(height(&d, t), 100.0, "{align}");
        assert_eq!(line_starts(&d, t), [0.0; 5], "{align}");
    }
}

/// `overflow-wrap: anywhere` and `break-word` break inside a word that
/// still overflows, which at width 0 is every word: one grapheme a line,
/// the spaces hanging. rinch's height was already ten lines (the measure
/// was right) and its paint layout one line.
#[test]
fn overflow_wrap_breaks_every_grapheme_at_width_zero() {
    let ten = ["x ", "-", "5 ", "(", "-", "5", ") ", "a", "-", "5"];
    for wrap in ["anywhere", "break-word"] {
        let m = format!(r#"<div id=t style="width:0;overflow-wrap:{wrap}">{TEXT}</div>"#);
        assert_chrome(&m, &ten, 200.0);
    }
}

/// Unchanged by the fix, and Chrome's before it: boxes whose zero width is
/// floored by an automatic minimum (`min-width: auto` of a flex item, a
/// table cell's min-content width) and `width: min-content` itself are as
/// wide as the widest unbreakable run, `(-5) ` at 28.52px, where `a-5`
/// fits; `nowrap` keeps one line.
#[test]
fn boxes_floored_at_min_content_and_nowrap_are_unchanged() {
    let min_content = ["x ", "-5 ", "(-5) ", "a-5"];
    for m in [
        format!(r#"<div style="display:flex;width:0"><div id=t>{TEXT}</div></div>"#),
        format!(
            r#"<table style="border-collapse:collapse"><tr><td id=t style="width:0">{TEXT}</td></tr></table>"#
        ),
        format!(r#"<div id=t style="width:min-content">{TEXT}</div>"#),
    ] {
        assert_chrome(&m, &min_content, 80.0);
    }
    let m = format!(r#"<div id=t style="width:0;white-space:nowrap">{TEXT}</div>"#);
    assert_chrome(&m, &[TEXT], 20.0);
}

/// An auto-width box with no room gets no pixel of slack. Auto-width boxes
/// get 1px of slack because a shrink-to-fit width is rounded down; a box
/// with a content width of 0 is not one of those. With 2px glyphs (an `i`
/// is about 0.5px) the slack fitted two per line: 2 lines painted in a
/// 6px box laid out for 3. Chrome 156: 3 lines. The 16px fixtures above
/// cannot see this, because each of their unbreakable runs is wider than 1px.
#[test]
fn an_auto_width_box_with_no_room_gets_no_slack() {
    let (d, t) = html(
        r#"<div style="width:0"><p id=t style="font-size:2px;line-height:2px">i&#8203;i&#8203;i</p></div>"#,
    );
    assert_eq!(lines_of(&d, t), ["i\u{200b}", "i\u{200b}", "i"]);
    assert_eq!(height(&d, t), 6.0);
}
