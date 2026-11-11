//! A grid or flex container's own text (#2443, #2411).
//!
//! CSS Grid 1 §6.1 and CSS Flexbox 1 §4: each run of a container's child text
//! is wrapped in an anonymous item, and a run of nothing but white space is
//! not rendered, "just as if its text nodes were display:none". rinch made a
//! grid holding only text an IFC root of its own, broken at the container's
//! width rather than its track's (#2411), and gave a white-space run between
//! tags a grid cell, a flex `gap` and a share of `justify-content` (#2443).
//!
//! Every expectation is Chrome 157's, on the same markup: the bundled Inter
//! registered as `ProbeFace`, `font: 16px/20px`, `* { margin: 0; padding: 0 }`,
//! boxes from `getBoundingClientRect`, lines from `Range.getClientRects` per
//! character. Chrome's window was 765px wide, so a container's `auto` width
//! is 800 here where it was 765 there; every other number is the same.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

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

fn html(markup: &str) -> RinchDocument {
    let mut d = doc();
    let body = d.body();
    d.set_inner_html(body, markup);
    d.resolve_layout(800.0, 600.0);
    d
}

/// `(x, y, width, height)` of `sel`'s border box on the page.
fn rect(d: &mut RinchDocument, sel: &str) -> (f32, f32, f32, f32) {
    let id: NodeId = d.query_selector(sel).expect(sel);
    let r = d.bounding_rect(id).expect("laid out");
    (r.x, r.y, r.width, r.height)
}

/// The lines of the one anonymous item holding `sel`'s text.
fn item_lines(d: &RinchDocument, sel: &str) -> Vec<String> {
    let id = d.query_selector(sel).expect(sel);
    let n = d.tree.get(id.0).unwrap();
    assert!(n.text_layout.is_none(), "the container is not an IFC root");
    assert_eq!(n.run_boxes.len(), 1, "one anonymous item");
    let item = d.tree.get(n.run_boxes[0]).unwrap();
    let il = item.text_layout.as_ref().expect("the item is an IFC root");
    il.layout
        .lines()
        .map(|l| il.text_content[l.text_range()].to_string())
        .collect()
}

// ---- #2443: a white-space run takes no grid cell -------------------------

const GRID: &str = r#"<div id=g style="display:grid; grid-template-columns:100px 1fr; width:300px"> <span id=s>a</span><span id=x>x</span></div>"#;

/// The issue's markup. Chrome 157: `#g` 300 x 20, `#s` in the first column,
/// `#x` in the second. rinch gave the leading space the first cell, so every
/// item moved one cell along and `#x` wrapped to a second row.
#[test]
fn a_white_space_run_takes_no_grid_cell() {
    let mut d = html(GRID);
    assert_eq!(rect(&mut d, "#g"), (0.0, 0.0, 300.0, 20.0));
    assert_eq!(rect(&mut d, "#s"), (0.0, 0.0, 100.0, 20.0));
    assert_eq!(rect(&mut d, "#x"), (100.0, 0.0, 200.0, 20.0));
}

/// Indented markup, a run of spaces and newlines before, between and after
/// the items. Chrome: one row, `#x6` in the second column.
#[test]
fn indentation_between_grid_items_takes_no_cell() {
    let mut d = html(
        "<div id=g6 style=\"display:grid; grid-template-columns:100px 100px; width:300px\">\n  <span id=s6>a</span>\n  <span id=x6>x</span>\n</div>",
    );
    assert_eq!(rect(&mut d, "#g6"), (0.0, 0.0, 300.0, 20.0));
    assert_eq!(rect(&mut d, "#s6"), (0.0, 0.0, 100.0, 20.0));
    assert_eq!(rect(&mut d, "#x6"), (100.0, 0.0, 100.0, 20.0));
}

/// The rule holds under `white-space: pre` too ("characters that can be
/// affected by the white-space property", not collapsible ones): Chrome puts
/// `#s9` in the first, 0px, column, and the grid is one line tall.
#[test]
fn a_preserved_white_space_run_takes_no_grid_cell_either() {
    let mut d = html(
        r#"<div id=g9 style="display:grid; grid-template-columns:0px; white-space:pre"> <span id=s9>a</span></div>"#,
    );
    assert_eq!(rect(&mut d, "#g9"), (0.0, 0.0, 800.0, 20.0));
    assert_eq!(rect(&mut d, "#s9"), (0.0, 0.0, 0.0, 20.0));
}

/// An inline grid's text is a Taffy text leaf rather than an anonymous box,
/// and is hidden the same way. Chrome: `#x15` beside `#s15`, one row.
#[test]
fn a_white_space_run_takes_no_inline_grid_cell() {
    let mut d = html(
        r#"<div id=g15 style="display:inline-grid; grid-template-columns:100px 100px"> <span id=s15>a</span><span id=x15>x</span></div>"#,
    );
    assert_eq!(rect(&mut d, "#g15"), (0.0, 0.0, 200.0, 20.0));
    assert_eq!(rect(&mut d, "#x15"), (100.0, 0.0, 100.0, 20.0));
}

/// Text that becomes white space leaves its cell, and text that stops being
/// white space takes one: the structural pass that hides the run is re-run
/// for an edit of the text alone.
#[test]
fn editing_the_text_in_or_out_of_white_space_moves_the_items() {
    let mut d = html(GRID);
    let g = d.query_selector("#g").unwrap();
    let text = NodeId(d.tree.get(g.0).unwrap().children[0]);

    d.set_text_content(text, "q");
    d.resolve_layout(800.0, 600.0);
    // Chrome: the text is the first item, so `#s` takes the second column
    // and `#x` a second row.
    assert_eq!(rect(&mut d, "#s"), (100.0, 0.0, 200.0, 20.0));
    assert_eq!(rect(&mut d, "#x"), (0.0, 20.0, 100.0, 20.0));

    d.set_text_content(text, "  ");
    d.resolve_layout(800.0, 600.0);
    assert_eq!(rect(&mut d, "#s"), (0.0, 0.0, 100.0, 20.0));
    assert_eq!(rect(&mut d, "#x"), (100.0, 0.0, 200.0, 20.0));
}

// ---- #2443 in flex: no item, so no gap and no free-space share -----------

/// Chrome: `#f10a` at 0 and `#f10b` one 10px gap past it. rinch made the three
/// white-space runs zero-width items, each with a gap of its own.
#[test]
fn white_space_runs_take_no_flex_gap() {
    let mut d = html(
        r#"<div id=f10 style="display:flex; gap:10px"> <span id=f10a>a</span> <span id=f10b>x</span> </div>"#,
    );
    let a = rect(&mut d, "#f10a");
    let b = rect(&mut d, "#f10b");
    assert_eq!(a.0, 0.0);
    assert_eq!(b.0, a.2 + 10.0);
}

/// Chrome: `space-between` puts `#f12a` at the start and `#f12b` at the end.
#[test]
fn white_space_runs_take_no_share_of_justify_content() {
    let mut d = html(
        r#"<div id=f12 style="display:flex; justify-content:space-between; width:300px"> <span id=f12a>a</span> <span id=f12b>x</span> </div>"#,
    );
    let a = rect(&mut d, "#f12a");
    let b = rect(&mut d, "#f12b");
    assert_eq!(a.0, 0.0);
    assert_eq!(b.0 + b.2, 300.0);
}

/// The same in an `inline-flex`, the display `Button` and friends use: the
/// container shrink-wraps two items and one gap.
#[test]
fn white_space_runs_take_no_inline_flex_gap() {
    let mut d = html(
        r#"<div id=f18 style="display:inline-flex; gap:10px"> <span id=f18a>a</span> <span id=f18b>x</span> </div>"#,
    );
    let a = rect(&mut d, "#f18a");
    let b = rect(&mut d, "#f18b");
    assert_eq!(a.0, 0.0);
    assert_eq!(b.0, a.2 + 10.0);
    assert_eq!(rect(&mut d, "#f18").2, a.2 + 10.0 + b.2);
}

// ---- #2411: a grid's own text is broken at its track's width -------------

/// The issue's text: a space, a hyphen before a digit after a letter (`a-5`,
/// an opportunity) and one after a space (`-5`, none).
const TEXT: &str = "x -5 (-5) a-5";

/// Chrome 156, `grid-template-columns: 0px`: every break opportunity is
/// taken, five lines, 100px tall. rinch laid the text out as one 800px line.
#[test]
fn a_grids_only_text_breaks_at_its_track() {
    let mut d = html(&format!(
        r#"<div id=t style="display:grid; grid-template-columns:0px">{TEXT}</div>"#
    ));
    assert_eq!(item_lines(&d, "#t"), ["x ", "-5 ", "(-5) ", "a-", "5"]);
    assert_eq!(rect(&mut d, "#t"), (0.0, 0.0, 800.0, 100.0));
}

/// A 40px track, off the zero-width fixed point. Chrome: three lines, 60px.
#[test]
fn a_grids_only_text_breaks_at_a_40px_track() {
    let mut d = html(&format!(
        r#"<div id=t3 style="display:grid; grid-template-columns:40px">{TEXT}</div>"#
    ));
    assert_eq!(item_lines(&d, "#t3"), ["x -5 ", "(-5) ", "a-5"]);
    assert_eq!(rect(&mut d, "#t3"), (0.0, 0.0, 800.0, 60.0));
}

/// Text behind a `display: contents` wrapper joins the text after it in one
/// item, and the span after them takes the second column. Chrome: the item's
/// lines in the 40px track, `#x17` 100 wide at x 40, as tall as the row.
#[test]
fn text_behind_a_contents_wrapper_joins_one_grid_item() {
    let mut d = html(
        r#"<div id=g17 style="display:grid; grid-template-columns:40px 100px"><span style="display:contents">x -5</span> (-5) a-5<span id=x17>x</span></div>"#,
    );
    assert_eq!(item_lines(&d, "#g17"), ["x -5 ", "(-5) ", "a-5"]);
    assert_eq!(rect(&mut d, "#x17"), (40.0, 0.0, 100.0, 60.0));
}

// ---- review of #2455 -----------------------------------------------------

/// Blank text behind a `display: contents` wrapper is a unit of the flex
/// container above it, and is hidden by it: the wrapper, which is no
/// formatting container, must not hand it its `display` back on a
/// whole-document pass. rsx's `for`/`if`/component wrappers inside a
/// `Group`/`Stack` are this shape. Chrome: `#a1` 8.984 wide, `#b1` at 18.984.
#[test]
fn blank_text_behind_a_contents_wrapper_takes_no_flex_gap() {
    let mut d = html(
        r#"<div style="display:flex; gap:10px"><b id=a1>a</b><span style="display:contents"> </span><b id=b1>b</b></div>"#,
    );
    let a = rect(&mut d, "#a1");
    let b = rect(&mut d, "#b1");
    assert_eq!(a.0, 0.0);
    assert_eq!(b.0, a.2 + 10.0);
}

/// A form feed between items is dropped like a space. Chrome: `#b2` at
/// 18.984, one gap past `#a2`.
#[test]
fn a_form_feed_takes_no_flex_gap() {
    let mut d =
        html(r#"<div style="display:flex; gap:10px"><b id=a2>a</b>&#12;<b id=b2>b</b></div>"#);
    let a = rect(&mut d, "#a2");
    let b = rect(&mut d, "#b2");
    assert_eq!(b.0, a.2 + 10.0);
}

/// A no-break space is text, not white space: it is an item of its own, with
/// a gap on each side. Chrome: `#b3` at 33.484, two gaps and the space's
/// 4.5px advance past `#a3` (rinch's boxes are on whole pixels, so the
/// assertion is the structure, read off rinch's own advance).
#[test]
fn a_no_break_space_is_a_flex_item() {
    let mut d = html(
        r#"<div id=f3 style="display:flex; gap:10px"><b id=a3>a</b>&nbsp;<b id=b3>b</b></div>"#,
    );
    let a = rect(&mut d, "#a3");
    let b = rect(&mut d, "#b3");
    let f = d.query_selector("#f3").unwrap();
    let nbsp = d.tree.get(f.0).unwrap().children[1];
    let w = d.tree.get(nbsp).unwrap().layout.width;
    assert!(w > 0.0, "the no-break space has an advance");
    assert_eq!(b.0, a.2 + 10.0 + w + 10.0);
}
