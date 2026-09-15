//! What counts as a scroll container's **scrollable overflow** (issue #765).
//!
//! `paint::scrollbar::content_extents` measures a container's scroll range from
//! its direct children's layout rects, and used to measure *every* one of them.
//! Taffy lays an out-of-flow box out against its direct parent whatever CSS says
//! its containing block is, so that made a `position: fixed` child — which
//! resolves against the viewport — into its Taffy parent's scrollable content: a
//! closed `Drawer` (`position: fixed`, and since #751/#761 still rendered while
//! closed) inside an `overflow: auto` div reported 800x600 of content and grew
//! two scrollbars over a div holding 100x50 of real content.
//!
//! Every expectation here is **measured in Chrome 141** on the same markup,
//! under `* { box-sizing: border-box; border-width: 0 }` — rinch's own defaults,
//! which its UA sheet and Taffy supply — and with `::-webkit-scrollbar { width:
//! 0; height: 0 }` so Chrome's classic bars do not eat 15px of `clientWidth`
//! that rinch's overlay bars never take. `scrollWidth - clientWidth` is the
//! quantity compared against `ScrollbarTrack::max_scroll`; Chrome measures both
//! in the container's **padding**-box frame and rinch in its **content**-box
//! frame, which cancels for **in-flow** children — every padded row below is
//! one. It does not cancel for a positioned child of a padded container, where
//! rinch over-reports by the end padding; `content_extents`' own doc has the
//! numbers, and nothing here pins that case.
//!
//! The Chrome numbers, for whoever re-derives these:
//!
//! | markup | `scrollWidth`x`scrollHeight` | `clientWidth`x`clientHeight` |
//! |---|---|---|
//! | 200x100 `auto`, `fixed` child + 100x50 child | 200x100 | 200x100 |
//! | 200x100 `auto` static, `absolute` 700x500 child | 200x100 | 200x100 |
//! | 200x100 `auto` **`relative`**, `absolute` 700x500 child | 700x500 | 200x100 |
//! | 200x100 `auto`, 700x500 `visibility: hidden` child | 700x500 | 200x100 |
//! | 200x100 `auto`, 700x500 `display: none` child | 200x100 | 200x100 |
//! | 200x100 `auto` `padding: 20px`, 160x60 child | 200x100 | 200x100 |
//! | 200x100 `auto` `padding: 20px`, 160x260 child | 200x300 | 200x100 |
//! | 200x100 `auto` `padding: 20px`, 161x61 child | 201x101 | 200x100 |
//! | 200x100 `auto` `border: 5px` `padding: 20px`, 150x250 child | 190x290 | 190x90 |

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::scrollbar::{Scrollbars, content_extents, scrollbars};

const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// One `div` scroll container under `<body>`, holding one `div` per entry of
/// `children`, laid out at the viewport.
fn scroller(container_style: &str, children: &[&str]) -> (RinchDocument, usize) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(container, "style", container_style);
    doc.append_child(body, container);
    for style in children {
        let child = doc.create_element("div");
        doc.set_attribute(child, "style", style);
        doc.append_child(container, child);
    }
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    (doc, container.0)
}

/// `(max_scroll_x, max_scroll_y)`, with `None` for an axis carrying no bar.
fn max_scroll(bars: &Scrollbars) -> (Option<f64>, Option<f64>) {
    (
        bars.horizontal.map(|t| t.max_scroll),
        bars.vertical.map(|t| t.max_scroll),
    )
}

/// A `position: fixed` child resolves against the viewport, so it is no part of
/// its Taffy parent's scroll range — issue #765's own shape.
///
/// The fixture is off every fixed point that could hide the bug: the container
/// is smaller than the viewport on both axes, so a counted fixed child
/// overflows it on both; and there is a real in-flow child that fits, so
/// "content is 100x50" is a positive statement about the fixed one being
/// skipped rather than about the container being empty.
///
/// Kills the mutant that drops the `PositionValue::Fixed => false` arm from
/// `out_of_flow::contributes_to_scrollable_overflow` (measured: content becomes
/// 800x600 and both bars appear, max_scroll 600/500).
#[test]
fn a_fixed_child_is_no_part_of_its_parents_scroll_range() {
    let (doc, id) = scroller(
        "width: 200px; height: 100px; overflow: auto",
        &[
            "position: fixed; top: 0; left: 0; right: 0; bottom: 0",
            "width: 100px; height: 50px",
        ],
    );
    assert_eq!(
        content_extents(&doc.tree, id),
        (100.0, 50.0),
        "only the in-flow child is scrollable content"
    );
    let bars = scrollbars(&doc.tree, id, 1.0);
    assert_eq!(
        max_scroll(&bars),
        (None, None),
        "Chrome: scrollWidth == clientWidth and scrollHeight == clientHeight, no bars"
    );
}

/// Same shape one level up: a `position: fixed` child of `<body>`, which the UA
/// sheet makes `overflow-y: auto`.
///
/// Deliberately **off** the fixed point the ordinary drawer sits on: a drawer
/// pinned `top: 0; bottom: 0` is exactly as tall as the body, so counting it
/// happens to change nothing. This one starts 40px down and is a full viewport
/// tall, so its bottom edge is 640 against a 600px body — counted, it grows a
/// body scrollbar with 40px of travel, which is what this assertion denies.
#[test]
fn a_fixed_child_does_not_give_the_body_a_scrollbar() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let fixed = doc.create_element("div");
    doc.set_attribute(
        fixed,
        "style",
        "position: fixed; top: 40px; left: 0; width: 320px; height: 600px",
    );
    doc.append_child(body, fixed);
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(
        content_extents(&doc.tree, body.0).1,
        0.0,
        "the body holds no in-flow content at all"
    );
    assert!(
        scrollbars(&doc.tree, body.0, 1.0).vertical.is_none(),
        "the body does not scroll to reach a viewport-anchored box"
    );
}

/// A `position: absolute` direct child of `<body>` belongs to the initial
/// containing block, not to `<body>`, so it gives `<body>` no scroll range.
///
/// `<body>` is `static` under the UA sheet, so it is not the box's containing
/// block, and it is not the initial containing block either — in rinch that is
/// `<html>`, the arm `an_absolute_child_of_html_resolves_against_html_and_counts`
/// pins from the positive side. Nothing pinned the negative side, and an
/// overlay mounted straight under `<body>` is the shape #765 is about. Chrome
/// 150, same markup: `<body>` reports `scrollHeight == clientHeight`, and the
/// 1500px lands on `documentElement.scrollHeight` instead.
///
/// Off the fixed point by construction: the box is 1500px tall against a 600px
/// viewport, so counting it would give the body a bar with 900px of travel.
///
/// Kills the mutant that widens the initial-containing-block arm of
/// `out_of_flow::contributes_to_scrollable_overflow` with `|| parent_id ==
/// tree.body_id` (measured: content 700x1500, vertical `max_scroll` 900), which
/// every other fixture here passes.
#[test]
fn an_absolute_child_of_body_gives_the_body_no_scroll_range() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let child = doc.create_element("div");
    doc.set_attribute(
        child,
        "style",
        "position: absolute; left: 0; top: 0; width: 700px; height: 1500px",
    );
    doc.append_child(body, child);
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(
        content_extents(&doc.tree, body.0),
        (0.0, 0.0),
        "a static <body> is not the containing block of its absolute child"
    );
    assert!(
        scrollbars(&doc.tree, body.0, 1.0).vertical.is_none(),
        "Chrome: body scrollHeight == clientHeight; the box is <html>'s"
    );
}

/// A `position: absolute` child counts only where the container is its
/// containing block. Here the container is `static`, so the box resolves
/// against the initial containing block (#204) and escapes.
///
/// Kills the mutant that treats every absolute like an in-flow child (measured:
/// content 700x500, max_scroll 500/400 where Chrome reports none).
#[test]
fn an_absolute_child_of_a_static_container_escapes_its_scroll_range() {
    let (doc, id) = scroller(
        "width: 200px; height: 100px; overflow: auto",
        &[
            "position: absolute; left: 0; top: 0; width: 700px; height: 500px",
            "width: 100px; height: 50px",
        ],
    );
    assert_eq!(content_extents(&doc.tree, id), (100.0, 50.0));
    assert_eq!(
        max_scroll(&scrollbars(&doc.tree, id, 1.0)),
        (None, None),
        "Chrome: 200x100 of scrollWidth/scrollHeight, no bars"
    );
}

/// ...and the same when the containing block is a positioned **ancestor** of
/// the container rather than the initial containing block. Chrome agrees the
/// scroller sees nothing (`scrollWidth == clientWidth`); it attributes the box
/// to the `position: relative` wrapper instead.
///
/// This is the arm a `parent.establishes_abs_containing_block()`-free mutant
/// (one keying only on "does the box have *any* positioned ancestor") would
/// get wrong while still passing the test above.
#[test]
fn an_absolute_child_whose_containing_block_is_an_ancestor_escapes_too() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let wrapper = doc.create_element("div");
    doc.set_attribute(wrapper, "style", "position: relative");
    doc.append_child(body, wrapper);
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "width: 200px; height: 100px; overflow: auto",
    );
    doc.append_child(wrapper, container);
    for style in [
        "position: absolute; left: 0; top: 0; width: 700px; height: 500px",
        "width: 100px; height: 50px",
    ] {
        let child = doc.create_element("div");
        doc.set_attribute(child, "style", style);
        doc.append_child(container, child);
    }
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(content_extents(&doc.tree, container.0), (100.0, 50.0));
    assert_eq!(
        max_scroll(&scrollbars(&doc.tree, container.0, 1.0)),
        (None, None)
    );
}

/// The other half, without which the fix would be "absolutes never count": a
/// `position: relative` container **is** its absolute child's containing block,
/// so that child is its scrollable content, exactly as in Chrome (700x500 of
/// `scrollWidth`/`scrollHeight` against a 200x100 client box).
///
/// Kills the mutant that answers `false` for every `PositionValue::Absolute`.
#[test]
fn an_absolute_child_of_a_positioned_container_is_counted() {
    let (doc, id) = scroller(
        "width: 200px; height: 100px; overflow: auto; position: relative",
        &[
            "position: absolute; left: 0; top: 0; width: 700px; height: 500px",
            "width: 100px; height: 50px",
        ],
    );
    assert_eq!(content_extents(&doc.tree, id), (700.0, 500.0));
    assert_eq!(
        max_scroll(&scrollbars(&doc.tree, id, 1.0)),
        (Some(500.0), Some(400.0)),
        "Chrome: 700 - 200 and 500 - 100"
    );
}

/// A `transform` makes a box the containing block of its absolute descendants
/// even at `position: static`, which is the second half of
/// `Node::establishes_abs_containing_block` and the half a mutant keying only
/// on `position` would drop.
#[test]
fn a_transformed_container_is_a_containing_block_too() {
    let (doc, id) = scroller(
        "width: 200px; height: 100px; overflow: auto; transform: translateX(10px)",
        &["position: absolute; left: 0; top: 0; width: 700px; height: 500px"],
    );
    assert_eq!(content_extents(&doc.tree, id), (700.0, 500.0));
    assert_eq!(
        max_scroll(&scrollbars(&doc.tree, id, 1.0)),
        (Some(500.0), Some(400.0))
    );
}

/// `<html>`'s box **is** the initial containing block in rinch, so an absolute
/// child of it resolves against the box it is a child of and counts — the arm
/// `out_of_flow::out_of_flow_kind` stops its walk on, mirrored here because the
/// two must not drift apart (nothing but this suite holds them together).
///
/// `<html>`'s own `overflow` is `visible`, so this is asserted on
/// `content_extents` rather than on a bar: `scrollbars` returns early for a
/// container that scrolls on neither axis, and would answer "no bars" whatever
/// the extent.
#[test]
fn an_absolute_child_of_html_resolves_against_html_and_counts() {
    let mut doc = RinchDocument::new();
    let html = NodeId(doc.tree.html_id);
    let child = doc.create_element("div");
    doc.set_attribute(
        child,
        "style",
        "position: absolute; left: 0; top: 0; width: 700px; height: 900px",
    );
    doc.append_child(html, child);
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(
        content_extents(&doc.tree, doc.tree.html_id).1,
        900.0,
        "the initial containing block owns an absolute that escaped everything else"
    );
}

/// `visibility: hidden` hides a box; it does not remove it. Chrome still
/// reports 700x500 of `scrollWidth`/`scrollHeight`, and so must rinch — which
/// matters because #761 is what made the closed `Drawer` `visibility: hidden`,
/// and the temptation is to make *that* the skip rule rather than `position:
/// fixed`.
///
/// Kills a mutant that skips hidden children (measured: no bars at all).
#[test]
fn a_visibility_hidden_child_still_counts() {
    let (doc, id) = scroller(
        "width: 200px; height: 100px; overflow: auto",
        &["width: 700px; height: 500px; visibility: hidden"],
    );
    assert_eq!(content_extents(&doc.tree, id), (700.0, 500.0));
    assert_eq!(
        max_scroll(&scrollbars(&doc.tree, id, 1.0)),
        (Some(500.0), Some(400.0))
    );
}

/// `display: none` generates no box, and its zeroed layout rect contributes
/// nothing without a special case. Pinned so the absence of that special case
/// is a decision rather than an oversight.
#[test]
fn a_display_none_child_contributes_nothing() {
    let (doc, id) = scroller(
        "width: 200px; height: 100px; overflow: auto",
        &[
            "width: 700px; height: 500px; display: none",
            "width: 100px; height: 50px",
        ],
    );
    assert_eq!(content_extents(&doc.tree, id), (100.0, 50.0));
    assert_eq!(max_scroll(&scrollbars(&doc.tree, id, 1.0)), (None, None));
}

/// A padded container measures its content from its **content** box, not its
/// border box (issue #480).
///
/// Taffy's `child.layout.{x,y}` are relative to the parent's border box, so a
/// `padding: 20px` container's first child sits at (20, 20); `content_extents`
/// subtracts that leading offset. Zero the subtraction out and this fixture's
/// exactly-fitting child reports 180x80 against a 160x60 content box and grows
/// two phantom bars — measured, and the survivor #480 was filed for, which no
/// test in the repo discriminated before this one.
///
/// Three samples, because the first alone sits on no fixed point but the
/// mutant's *slope* does not show there: exact fit (no bars), 1px over (1px of
/// travel on each axis — the smallest overflow that exists at all), and a
/// genuinely overflowing child (200px of travel, not the mutant's 220).
#[test]
fn a_padded_container_measures_its_content_box() {
    const PADDED: &str = "width: 200px; height: 100px; padding: 20px; overflow: auto";

    let (doc, id) = scroller(PADDED, &["width: 160px; height: 60px"]);
    assert_eq!(
        content_extents(&doc.tree, id),
        (160.0, 60.0),
        "a child that exactly fills the content box is 160x60 of content, not 180x80"
    );
    assert_eq!(
        max_scroll(&scrollbars(&doc.tree, id, 1.0)),
        (None, None),
        "Chrome: scrollWidth 200 == clientWidth 200"
    );

    let (doc, id) = scroller(PADDED, &["width: 161px; height: 61px"]);
    assert_eq!(
        max_scroll(&scrollbars(&doc.tree, id, 1.0)),
        (Some(1.0), Some(1.0)),
        "Chrome: scrollWidth 201, scrollHeight 101, client 200x100"
    );

    let (doc, id) = scroller(PADDED, &["width: 160px; height: 260px"]);
    assert_eq!(
        content_extents(&doc.tree, id),
        (160.0, 260.0),
        "not 180x280"
    );
    assert_eq!(
        max_scroll(&scrollbars(&doc.tree, id, 1.0)),
        (None, Some(200.0)),
        "Chrome: scrollHeight 300 - clientHeight 100; the mutant's is 220"
    );
}

/// The border half of the same subtraction, which the padding fixture above
/// cannot discriminate on its own: a mutant that subtracts the padding and
/// forgets the border passes every assertion there and fails here.
#[test]
fn a_bordered_container_measures_past_its_border_too() {
    const BORDERED: &str =
        "width: 200px; height: 100px; border: 5px solid; padding: 20px; overflow: auto";

    let (doc, id) = scroller(BORDERED, &["width: 150px; height: 50px"]);
    assert_eq!(
        content_extents(&doc.tree, id),
        (150.0, 50.0),
        "the child sits at (25, 25) and exactly fills the 150x50 content box"
    );
    assert_eq!(
        max_scroll(&scrollbars(&doc.tree, id, 1.0)),
        (None, None),
        "Chrome: scrollWidth 190 == clientWidth 190"
    );

    let (doc, id) = scroller(BORDERED, &["width: 150px; height: 250px"]);
    assert_eq!(
        max_scroll(&scrollbars(&doc.tree, id, 1.0)),
        (None, Some(200.0)),
        "Chrome: scrollHeight 290 - clientHeight 90"
    );
}
