//! An absolutely positioned box is part of the **scrollable overflow** of the
//! nearest scroll container at or above its **containing block**, wherever it
//! sits below that container (css-overflow-3 §2.2, issue #770) — not of the box
//! it happens to be laid out in. A scroll container *between* the box and its
//! containing block does not carry it (the box escapes it, #765); a box that is
//! not a scroll container passes it up, transformed by its transform.
//!
//! `content_extents` walks a container's child boxes and stops at the first
//! box, so before #770 a box laid out in any box but its scroll container was
//! measured by nobody: the issue's 700x1500 absolute under a static `overflow:
//! auto` div scrolled no box at all, where Chrome scrolls the page to it.
//!
//! Every number is **measured in Chrome 156** (headless, an 800x600 viewport)
//! on the same markup, under `* { box-sizing: border-box; margin: 0; padding:
//! 0; border-width: 0 }` and `::-webkit-scrollbar { width: 0; height: 0 }`, as
//! `scrollWidth - clientWidth` x `scrollHeight - clientHeight` of the box named.
//! That quantity is `ScrollbarTrack::max_scroll`, and the wheel's range.
//!
//! | row | shape | Chrome |
//! |---|---|---|
//! | c1 | the issue: static 200x100 scroller > absolute `0,0` 700x1500 | `html` 800x1500 (page 0x900), scroller 0x0 |
//! | c2 | padded `relative` scroller `r` > div > static scroller `s1` > absolute `30,40` 400x350 | `r` 138x196, `s1` 0x0 |
//! | c2s | c2 with `s1` holding 300px and scrolled 40, and `r` scrolled 50 | `r` 138x196 |
//! | c3 | padded static scroller > `relative` bordered div > div > absolute `60,70` 500x400 | 321x308 |
//! | c4 | c3 without the middle div (the containing block is the layout parent) | 321x308 |
//! | c4t | c3 with the `relative` div `translate(7px, 11px)` instead | 322x316 |
//! | c5 | scroller > `scale(2)` div > `relative; left: 5px` div > absolute `20,30` 150x120 | 60x100 |
//! | c6 | `relative` scroller `o` > static scroller `i` > div > absolute `250,260` 50x40 | `o` 0x100, `i` 0x0 |
//! | c6b | static scroller `o` > padded `relative` scroller `i` > div > absolute `150,120` 100x80 | `i` 50x100, `o` 0x0 |
//! | c7 | scroller > `overflow: hidden` 100x50 > `relative` > absolute 400x500 | scroller 0x0, hidden box 300x450 |
//! | c8 | c7 with `overflow: clip` | scroller 0x0 |
//! | c9 | scroller > `relative` > `fixed` 900x900 | 0x0, page 0x0 |
//! | c10 | `<body>` > absolute `30,900` 50x50 | page 0x350 |
//! | c11 | `<body>` > `relative; margin-top: 20px` 30px > div > absolute `top: 1000px` 100px | page 0x520 |
//!
//! **The page.** Chrome scrolls the viewport, whose range is
//! `documentElement.scrollWidth/Height`. rinch's page scroll is the `<body>`'s
//! (the UA sheet gives it `overflow-y: auto`, and its scroll is the one that
//! carries an initial-containing-block box), so the page rows are asserted on
//! the body: its client box is the 800x600 viewport, so its range is Chrome's
//! viewport range. With the body made `overflow: visible`, the `<html>` box —
//! the initial containing block — measures it instead.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::scrollbar::{content_extents, scrollbars};

const CSS: &str = "* { box-sizing: border-box; margin: 0; padding: 0; border-width: 0; }";

/// `<body>`'s content, laid out at 800x600.
fn doc_of(html: &str) -> RinchDocument {
    doc_with(CSS, html)
}

fn doc_with(css: &str, html: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css(css);
    let body = doc.body();
    doc.set_inner_html(body, html);
    doc.resolve_layout(800.0, 600.0);
    doc
}

fn by_id(doc: &RinchDocument, id: &str) -> NodeId {
    doc.query_selector(&format!("#{id}"))
        .unwrap_or_else(|| panic!("no #{id}"))
}

/// The bars' ranges of `id`, checked against the wheel's: the wheel and
/// `scroll_into_view` clamp to `scroll_* - client_*`, and it must be the
/// bar's range, or the thumb and the wheel disagree.
fn ranges(doc: &RinchDocument, id: NodeId) -> (f64, f64) {
    let bars = scrollbars(&doc.tree, id.0, 1.0);
    let x = bars.horizontal.map_or(0.0, |t| t.max_scroll);
    let y = bars.vertical.map_or(0.0, |t| t.max_scroll);
    let (wheel_x, wheel_y) = overflow(doc, id);
    assert!(
        (wheel_x - x).abs() < 1e-6 && (wheel_y - y).abs() < 1e-6,
        "the wheel's range ({wheel_x}, {wheel_y}) is the bars' ({x}, {y})"
    );
    (x, y)
}

/// `scroll_* - client_*`, for a box that paints no bars.
fn overflow(doc: &RinchDocument, id: NodeId) -> (f64, f64) {
    (
        (doc.scroll_width(id) - doc.client_width(id)).max(0.0),
        (doc.scroll_height(id) - doc.client_height(id)).max(0.0),
    )
}

#[track_caller]
fn assert_range(got: (f64, f64), want: (f64, f64)) {
    assert!(
        (got.0 - want.0).abs() < 0.01 && (got.1 - want.1).abs() < 0.01,
        "max scroll {got:?}, Chrome {want:?}"
    );
}

/// The issue's own shape (c1): the absolute escapes the static scroller it is
/// laid out in and lands on the initial containing block, so the page scrolls
/// 1500 - 600 to reach it. Before the fix nothing scrolled to it.
#[test]
fn an_absolute_escaping_a_static_scroller_scrolls_the_page() {
    let doc = doc_of(
        r#"<div id="n1" style="width:200px;height:100px;overflow:auto;"><div style="position:absolute;left:0;top:0;width:700px;height:1500px;"></div></div>"#,
    );
    assert_range(ranges(&doc, by_id(&doc, "n1")), (0.0, 0.0));
    assert_range(ranges(&doc, doc.body()), (0.0, 900.0));
}

/// c1 with a body that does not scroll: the `<html>` box is the initial
/// containing block and measures it, as `documentElement.scrollHeight` does
/// in Chrome (1500).
#[test]
fn with_a_body_that_does_not_scroll_the_html_box_measures_it() {
    let doc = doc_with(
        &format!("{CSS} body {{ overflow: visible; }}"),
        r#"<div id="n1" style="width:200px;height:100px;overflow:auto;"><div style="position:absolute;left:0;top:0;width:700px;height:1500px;"></div></div>"#,
    );
    assert_eq!(
        content_extents(&doc.tree, doc.tree.html_id),
        (800.0, 1500.0)
    );
}

const C2: &str = r#"<div id="r" style="position:relative;width:300px;height:200px;overflow:auto;border-style:solid;border-width:2px 5px 4px 3px;padding:7px 11px 13px 17px;"><div id="d" style="padding:9px 0 0 13px;width:150px;height:60px;"><div id="s1" style="width:120px;height:50px;overflow:auto;padding:3px;">CONTENT<div style="position:absolute;left:30px;top:40px;width:400px;height:350px;"></div></div></div></div>"#;

/// c2: a padded, bordered `relative` scroller is the containing block, and a
/// static scroller sits between it and the box. The box is the outer one's,
/// at its padding box's (30, 40): 430x390 of scroll against a 292x194 client
/// box. The inner scroller gets nothing.
#[test]
fn a_scroller_between_the_box_and_its_containing_block_does_not_get_it() {
    let doc = doc_of(&C2.replace("CONTENT", ""));
    assert_range(ranges(&doc, by_id(&doc, "r")), (138.0, 196.0));
    assert_range(ranges(&doc, by_id(&doc, "s1")), (0.0, 0.0));
}

/// c2s: the box does not move when the scroller it escapes scrolls (rinch
/// writes that scroll into its `layout`, `replace_after_scroll`), nor is its
/// own container's scroll part of its range. Chrome: unchanged at 430x390
/// with `s1` at 40 and `r` at 50.
#[test]
fn the_range_does_not_move_with_any_scroll() {
    let mut doc = doc_of(&C2.replace("CONTENT", r#"<div style="height:300px;"></div>"#));
    let (r, s1) = (by_id(&doc, "r"), by_id(&doc, "s1"));
    assert_range(ranges(&doc, s1), (0.0, 256.0));
    doc.set_scroll_top(s1, 40.0);
    doc.set_scroll_top(r, 50.0);
    assert_range(ranges(&doc, r), (138.0, 196.0));
}

const C3: &str = r#"<div id="s" style="width:300px;height:200px;overflow:auto;padding:10px 20px 30px 40px;"><div style="CB;margin:25px 0 0 15px;width:100px;height:50px;border-style:solid;border-width:BORDER;">MID<div style="position:absolute;left:60px;top:70px;width:500px;height:400px;"></div>END</div></div>"#;

fn c3(cb: &str, border: &str, middle: bool) -> String {
    let (mid, end) = if middle {
        (r#"<div style="width:50px;height:20px;">"#, "</div>")
    } else {
        ("", "")
    };
    C3.replace("CB", cb)
        .replace("BORDER", border)
        .replace("MID", mid)
        .replace("END", end)
}

/// c3: the containing block is a bordered `relative` box that is no scroll
/// container, inside a padded static scroller, and the box sits a level
/// below it. Its padding box is at (61, 38) of the scroller's, so the box
/// ends at (621, 508): 321x308 of travel.
#[test]
fn a_non_scrolling_containing_block_passes_the_box_up() {
    let doc = doc_of(&c3("position:relative", "3px 0 0 6px", true));
    assert_range(ranges(&doc, by_id(&doc, "s")), (321.0, 308.0));
}

/// c4: the same box as a direct child of its containing block — Taffy's own
/// answer, which the read-back never re-places, and which the scroller still
/// never reached.
#[test]
fn a_box_laid_out_in_its_containing_block_below_the_scroller_counts() {
    let doc = doc_of(&c3("position:relative", "3px 0 0 6px", false));
    assert_range(ranges(&doc, by_id(&doc, "s")), (321.0, 308.0));
}

/// c4t: a `transform` makes the box its containing block, and moves it:
/// `translate(7px, 11px)` puts the box at (122, 116), so 322x316.
#[test]
fn a_transformed_containing_block_moves_the_box_it_passes_up() {
    let doc = doc_of(&c3("transform:translate(7px,11px)", "0", true));
    assert_range(ranges(&doc, by_id(&doc, "s")), (322.0, 316.0));
}

/// c5: a transformed box *between* the containing block and the scroller
/// scales what it passes up: the box is at (25, 30) 150x120 inside a
/// `scale(2)` box at x = 10, so (60, 60) to (360, 300).
#[test]
fn a_transform_between_the_containing_block_and_the_scroller_applies() {
    let doc = doc_of(
        r#"<div id="s" style="width:300px;height:200px;overflow:auto;"><div style="transform:scale(2);transform-origin:0 0;width:100px;height:50px;margin-left:10px;"><div style="position:relative;left:5px;"><div style="position:absolute;left:20px;top:30px;width:150px;height:120px;"></div></div></div></div>"#,
    );
    assert_range(ranges(&doc, by_id(&doc, "s")), (60.0, 100.0));
}

/// c6 and c6b: of two nested scrollers, the box is the one's at or above its
/// containing block — the outer one under a `relative` outer scroller, the
/// inner one under a `relative` inner scroller (which clips it from the
/// outer).
#[test]
fn of_nested_scrollers_the_one_at_the_containing_block_gets_it() {
    let doc = doc_of(
        r#"<div id="o" style="position:relative;width:300px;height:200px;overflow:auto;"><div id="i" style="width:200px;height:100px;overflow:auto;margin:10px 0 0 20px;"><div style="width:50px;height:20px;"><div style="position:absolute;left:250px;top:260px;width:50px;height:40px;"></div></div></div></div>"#,
    );
    assert_range(ranges(&doc, by_id(&doc, "o")), (0.0, 100.0));
    assert_range(ranges(&doc, by_id(&doc, "i")), (0.0, 0.0));

    let doc = doc_of(
        r#"<div id="o" style="width:300px;height:200px;overflow:auto;"><div id="i" style="position:relative;width:200px;height:100px;overflow:auto;margin:10px 0 0 20px;padding:4px 6px 8px 12px;"><div style="width:50px;height:20px;"><div style="position:absolute;left:150px;top:120px;width:100px;height:80px;"></div></div></div></div>"#,
    );
    assert_range(ranges(&doc, by_id(&doc, "i")), (50.0, 100.0));
    assert_range(ranges(&doc, by_id(&doc, "o")), (0.0, 0.0));
}

/// c7 and c8: `overflow: hidden` is a scroll container, so it takes the box
/// (300x450 of `scroll - client`) and the scroller above gets nothing;
/// `overflow: clip` is not, and clips it away from the scroller all the same.
#[test]
fn a_hidden_box_takes_it_and_a_clip_box_clips_it() {
    let shape = r#"<div id="s" style="width:300px;height:200px;overflow:auto;"><div id="h" style="width:100px;height:50px;overflow:VALUE;"><div style="position:relative;"><div style="position:absolute;left:0;top:0;width:400px;height:500px;"></div></div></div></div>"#;
    let doc = doc_of(&shape.replace("VALUE", "hidden"));
    assert_range(ranges(&doc, by_id(&doc, "s")), (0.0, 0.0));
    assert_range(overflow(&doc, by_id(&doc, "h")), (300.0, 450.0));

    let doc = doc_of(&shape.replace("VALUE", "clip"));
    assert_range(ranges(&doc, by_id(&doc, "s")), (0.0, 0.0));
}

/// c9: a `fixed` box is the viewport's, which scrolls nowhere.
#[test]
fn a_fixed_box_scrolls_nothing() {
    let doc = doc_of(
        r#"<div id="s" style="width:300px;height:200px;overflow:auto;"><div style="position:relative;"><div style="position:fixed;left:0;top:0;width:900px;height:900px;"></div></div></div>"#,
    );
    assert_range(ranges(&doc, by_id(&doc, "s")), (0.0, 0.0));
    assert_range(ranges(&doc, doc.body()), (0.0, 0.0));
}

/// c10 and c11: the page reaches an absolute child of the body (the initial
/// containing block's), and one under a `relative` box further down that is
/// no scroll container.
#[test]
fn the_page_reaches_absolutes_below_the_body() {
    let doc =
        doc_of(r#"<div style="position:absolute;left:30px;top:900px;width:50px;height:50px;"></div>"#);
    assert_range(ranges(&doc, doc.body()), (0.0, 350.0));

    let doc = doc_of(
        r#"<div style="position:relative;margin-top:20px;height:30px;"><div style="height:10px;"><div style="position:absolute;left:0;top:1000px;width:10px;height:100px;"></div></div></div>"#,
    );
    assert_range(ranges(&doc, doc.body()), (0.0, 520.0));
}
