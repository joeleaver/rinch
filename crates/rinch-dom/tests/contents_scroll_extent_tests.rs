//! A `display: contents` wrapper's children are a scroll container's content
//! (issue #396).
//!
//! A `display: contents` element generates no box: its children are laid out
//! in its parent's formatting context, and Taffy lays them out as children of
//! that parent (`sync_display_contents` splices them in). So they are the
//! parent's scrollable overflow exactly as if the wrapper were not there.
//! `paint::scrollbar::content_extents` walked only the container's **direct**
//! DOM children, found the wrapper's own `0x0` box, and decided the content fit:
//! no bar, and — through `DomDocument::scroll_height`, a second copy of the
//! same walk — a wheel that moved nothing. `rsx!` emits one such wrapper per
//! reactive text, `if`, `match`, `for` and embedded `Vec<NodeHandle>`, so an
//! `overflow: auto` region whose rows come from a `for` did not scroll at all.
//!
//! Every expectation is **measured in Chrome 153** on the same markup under
//! `* { box-sizing: border-box; border-width: 0 }` and zero-size
//! `::-webkit-scrollbar`, as in `scrollable_overflow_tests.rs`:
//!
//! | markup (200x100 `overflow: auto` container) | `scrollWidth`x`scrollHeight` |
//! |---|---|
//! | `contents` > 700x500 | 700x500 |
//! | 100x50, then `contents` > `contents` > 700x500 | 700x550 |
//! | `contents` > (`fixed` inset 0, 100x50) | 200x100 |
//! | `contents` > `absolute` 700x500 (container static) | 200x100 |
//! | `contents` > `absolute` 700x500 (container `relative`) | 700x500 |
//! | `padding: 20px`, `contents` > 160x260 | 200x300 |
//! | `fixed` inset 0 child, 100x50 child (no wrapper) | 200x100 |

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::scrollbar::{Scrollbars, content_extents, scrollbars};

const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// A tree to build under a scroll container: a `div` with a style, and its
/// children.
struct El(&'static str, Vec<El>);

fn build(doc: &mut RinchDocument, parent: NodeId, el: &El) {
    let node = doc.create_element("div");
    doc.set_attribute(node, "style", el.0);
    doc.append_child(parent, node);
    for child in &el.1 {
        build(doc, node, child);
    }
}

/// One `div` scroll container under `<body>`, holding `children`, laid out at
/// the viewport.
fn scroller(container_style: &str, children: &[El]) -> (RinchDocument, usize) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(container, "style", container_style);
    doc.append_child(body, container);
    for child in children {
        build(&mut doc, container, child);
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

/// Every reader of a scroll range, in one tuple: the shared extent, the bars
/// paint and hit testing use, and the `DomDocument` pair the wheel clamps to.
fn ranges(doc: &RinchDocument, id: usize) -> ((f64, f64), (Option<f64>, Option<f64>), (f64, f64)) {
    (
        content_extents(&doc.tree, id),
        max_scroll(&scrollbars(&doc.tree, id, 1.0)),
        (doc.scroll_width(NodeId(id)), doc.scroll_height(NodeId(id))),
    )
}

const SCROLLER: &str = "width: 200px; height: 100px; overflow: auto";
const CONTENTS: &str = "display: contents";

/// Issue #396's own shape: the container's only child is a `display: contents`
/// wrapper around content bigger than the container on both axes.
///
/// Chrome: 700x500 of scroll size in a 200x100 client box, so 500x400 of
/// travel. Kills the unfixed walk (which reads the wrapper's `0x0` box and
/// reports no content) in all three readers — the bars *and*
/// `scroll_width`/`scroll_height`, which the wheel clamps to.
#[test]
fn a_contents_wrappers_child_is_the_containers_content() {
    let (doc, id) = scroller(
        SCROLLER,
        &[El(
            CONTENTS,
            vec![El("width: 700px; height: 500px", vec![])],
        )],
    );
    assert_eq!(
        ranges(&doc, id),
        ((700.0, 500.0), (Some(500.0), Some(400.0)), (700.0, 500.0)),
        "Chrome: scrollWidth x scrollHeight = 700x500 in a 200x100 client box"
    );
}

/// Two wrappers deep, and **off the origin**: a 100x50 sibling first, so the
/// flattened child is laid out at `y = 50` in the container's space.
///
/// Kills a one-level descent (the inner wrapper's `0x0` box is all it sees:
/// content 100x50, no horizontal bar) and a descent that re-bases a flattened
/// child's rect on its wrapper's own (zero) origin by accident of the fixture —
/// the wrapper sits at the sibling's end, not at `0,0`.
#[test]
fn nested_wrappers_are_walked_through_to_the_boxes_they_hold() {
    let (doc, id) = scroller(
        SCROLLER,
        &[
            El("width: 100px; height: 50px", vec![]),
            El(
                CONTENTS,
                vec![El(
                    CONTENTS,
                    vec![El("width: 700px; height: 500px", vec![])],
                )],
            ),
        ],
    );
    assert_eq!(
        ranges(&doc, id),
        ((700.0, 550.0), (Some(500.0), Some(450.0)), (700.0, 550.0)),
        "Chrome: 700x550"
    );
}

/// The containing-block rule applies to a box reached **through** a wrapper,
/// against the scroll container: a wrapper generates no box, so it is nobody's
/// containing block.
///
/// Kills a descent that skips `contributes_to_scrollable_overflow` for the
/// flattened children (measured: the fixed box makes the content 800x600, the
/// #765 phantom bars reached one wrapper down).
#[test]
fn a_fixed_box_inside_a_wrapper_is_still_no_part_of_the_range() {
    let (doc, id) = scroller(
        SCROLLER,
        &[El(
            CONTENTS,
            vec![
                El(
                    "position: fixed; top: 0; left: 0; right: 0; bottom: 0",
                    vec![],
                ),
                El("width: 100px; height: 50px", vec![]),
            ],
        )],
    );
    assert_eq!(
        ranges(&doc, id),
        ((100.0, 50.0), (None, None), (100.0, 50.0)),
        "Chrome: 200x100, no bars"
    );
}

/// An absolute box inside a wrapper counts exactly when the **scroll
/// container** is its containing block — the rule asked of the container, not
/// of the wrapper.
///
/// The pair kills both wrong parents for the predicate: asking it of the
/// wrapper (static, so the `relative` case loses its content) and dropping it
/// (the static case gains 700x500).
#[test]
fn an_absolute_box_inside_a_wrapper_counts_only_where_the_container_holds_it() {
    let absolute = || {
        El(
            CONTENTS,
            vec![El(
                "position: absolute; left: 0; top: 0; width: 700px; height: 500px",
                vec![],
            )],
        )
    };
    let (doc, id) = scroller(SCROLLER, &[absolute()]);
    assert_eq!(
        ranges(&doc, id),
        ((0.0, 0.0), (None, None), (0.0, 0.0)),
        "static container: Chrome 200x100"
    );

    let (doc, id) = scroller(&format!("{SCROLLER}; position: relative"), &[absolute()]);
    assert_eq!(
        ranges(&doc, id),
        ((700.0, 500.0), (Some(500.0), Some(400.0)), (700.0, 500.0)),
        "relative container: Chrome 700x500"
    );
}

/// A padded container: the flattened child sits at the container's content
/// origin exactly as a direct child would, so the same padding subtraction
/// applies. Chrome: 200x300 scroll size, 200x100 client — 200px of vertical
/// travel, none horizontal.
#[test]
fn a_wrapped_child_of_a_padded_container_is_measured_like_a_direct_one() {
    let (doc, id) = scroller(
        &format!("{SCROLLER}; padding: 20px"),
        &[El(
            CONTENTS,
            vec![El("width: 160px; height: 260px", vec![])],
        )],
    );
    assert_eq!(
        ranges(&doc, id),
        ((160.0, 260.0), (None, Some(200.0)), (160.0, 260.0)),
        "Chrome: 200x300"
    );
}

/// `scroll_width`/`scroll_height` apply the #765 containing-block rule too:
/// they are what the wheel clamps to, so a bar and the wheel cannot disagree
/// about what is content. Before #396 they were a second copy of the walk
/// without the filter, and a `fixed` child gave a 200x100 box 600px of wheel
/// travel over content that fits and paints no bar.
#[test]
fn the_wheels_range_skips_a_fixed_child_as_the_bars_do() {
    let (doc, id) = scroller(
        SCROLLER,
        &[
            El(
                "position: fixed; top: 0; left: 0; right: 0; bottom: 0",
                vec![],
            ),
            El("width: 100px; height: 50px", vec![]),
        ],
    );
    assert_eq!(
        ranges(&doc, id),
        ((100.0, 50.0), (None, None), (100.0, 50.0)),
        "Chrome: scrollHeight == clientHeight"
    );
}
