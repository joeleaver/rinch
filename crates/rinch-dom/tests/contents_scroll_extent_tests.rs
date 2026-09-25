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
type Ranges = ((f64, f64), (Option<f64>, Option<f64>), (f64, f64));

fn ranges(doc: &RinchDocument, id: usize) -> Ranges {
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

// ── Inline content: text a wrapper or an inline element holds ──────────────
//
// Text is the other half of #396. Every reactive `{|| text}` is a
// `display: contents` `<span>` around a text node, and a text node under a
// wrapper — or under a plain inline `<span>` — is given no box at all: only a
// text node that is the IFC root's *direct* child gets a proxy box from
// `write_inline_positions`. So `div { overflow-y: auto, {|| log.get()} }` had
// no scroll range. `content_extents` now reads an IFC root's inline layout
// itself.
//
// The text is fifteen `wwwwwwwwww` words at `font-size: 16px; line-height:
// 20px` in a 200px box: one word is 116-131px in every sans face in reach
// (Inter, DejaVu, Arial), so exactly one fits per line whatever the host's
// fonts, and the content is fifteen declared 20px lines, 300px. Chrome 153 on
// the same markup:
//
// | markup (200x40 `overflow: auto` container) | `scrollWidth`x`scrollHeight` |
// |---|---|
// | text | 200x300 |
// | `contents` span > text | 200x300 |
// | inline `span` > text | 200x300 |
// | `padding: 10px`, text | 200x320 |
// | `padding: 10px`, `contents` span > text | 200x320 |

const TEXT_SCROLLER: &str =
    "width: 200px; height: 40px; overflow: auto; font-size: 16px; line-height: 20px";

/// A text scroller: `wrapper` (a `span` style, or `None` for bare text).
fn text_scroller(container_style: &str, wrapper: Option<&str>) -> (RinchDocument, usize) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(container, "style", container_style);
    doc.append_child(body, container);
    let parent = match wrapper {
        Some(style) => {
            let span = doc.create_element("span");
            doc.set_attribute(span, "style", style);
            doc.append_child(container, span);
            span
        }
        None => container,
    };
    let text = doc.create_text(&"wwwwwwwwww ".repeat(15));
    doc.append_child(parent, text);
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    (doc, container.0)
}

/// `(max_scroll_x, max_scroll_y, scroll_height)`: the bars and the wheel's
/// range. The width is not asserted beyond "no horizontal bar" — it is the
/// longest line's advance, which is the host font's.
fn text_ranges(doc: &RinchDocument, id: usize) -> (Option<f64>, Option<f64>, f64) {
    let (x, y) = max_scroll(&scrollbars(&doc.tree, id, 1.0));
    (x, y, doc.scroll_height(NodeId(id)))
}

/// Bare text directly in the scroller: the positive control, and green before
/// #396 — the one shape `write_inline_positions`' proxy box covered.
#[test]
fn text_directly_in_a_scroller_scrolls_its_lines() {
    let (doc, id) = text_scroller(TEXT_SCROLLER, None);
    assert_eq!(
        text_ranges(&doc, id),
        (None, Some(260.0), 300.0),
        "Chrome: 300 - 40"
    );
}

/// Reactive text's own shape: a `display: contents` span around the text.
/// Unfixed: no range at all.
#[test]
fn text_in_a_contents_span_scrolls_its_lines() {
    let (doc, id) = text_scroller(TEXT_SCROLLER, Some("display: contents"));
    assert_eq!(
        text_ranges(&doc, id),
        (None, Some(260.0), 300.0),
        "Chrome: 300 - 40"
    );
}

/// An ordinary inline `<span>`: a flowed inline element owns no box either, so
/// this was no range too, with no `display: contents` anywhere.
#[test]
fn text_in_an_inline_span_scrolls_its_lines() {
    let (doc, id) = text_scroller(TEXT_SCROLLER, Some(""));
    assert_eq!(
        text_ranges(&doc, id),
        (None, Some(260.0), 300.0),
        "Chrome: 300 - 40"
    );
}

/// A padded text scroller, bare and wrapped. Chrome: 320 - 40 = 280px of
/// travel and no horizontal bar. The proxy box sat at the *border*-box origin
/// with the border-box width, so the bare case used to report 290px of content
/// (270px of travel) and a 190px-wide line in a 180px content box — a
/// horizontal bar over text that wraps.
#[test]
fn a_padded_text_scroller_measures_its_lines_from_the_content_box() {
    let padded = format!("{TEXT_SCROLLER}; padding: 10px");
    for wrapper in [None, Some("display: contents")] {
        let (doc, id) = text_scroller(&padded, wrapper);
        assert_eq!(
            text_ranges(&doc, id),
            (None, Some(280.0), 300.0),
            "Chrome: 320 - 40, no horizontal bar ({wrapper:?})"
        );
    }
}

/// Issue #873's scroll half: a layout pass that re-runs Taffy but skips this
/// IFC root (its text is clean, its width unchanged) resets the direct text
/// child's proxy box to Taffy's 0x0 and never writes it back. The range no
/// longer depends on that box, so an unrelated sibling's resize leaves it
/// alone.
#[test]
fn an_unrelated_layout_pass_keeps_a_text_scrollers_range() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(container, "style", TEXT_SCROLLER);
    doc.append_child(body, container);
    let text = doc.create_text(&"wwwwwwwwww ".repeat(15));
    doc.append_child(container, text);
    let sibling = doc.create_element("div");
    doc.set_attribute(sibling, "style", "width: 100px; height: 10px");
    doc.append_child(body, sibling);
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(
        text_ranges(&doc, container.0),
        (None, Some(260.0), 300.0),
        "first layout"
    );
    doc.set_attribute(sibling, "style", "width: 150px; height: 10px");
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(
        text_ranges(&doc, container.0),
        (None, Some(260.0), 300.0),
        "after a layout pass that did not rebuild the scroller's text"
    );
}

/// `white-space: nowrap` text scrolls **horizontally** by its own line's
/// advance — the width half of the inline-layout measure. Five
/// `wwwwwwwwww` words are over 580px in every sans face in reach (one word is
/// 116-131px), so the assertion holds on any host; Chrome 153: 596x100 of
/// scroll size in a 200x100 box. Unfixed, the bare case reported the proxy
/// box's 200px (no bar) and the wrapped one nothing.
///
/// Kills a measure that takes the inline layout's height and not its width.
#[test]
fn nowrap_text_scrolls_horizontally_by_its_line() {
    let style = "width: 200px; height: 100px; overflow: auto; font-size: 16px; \
                 line-height: 20px; white-space: nowrap";
    for wrapper in [None, Some("display: contents")] {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let container = doc.create_element("div");
        doc.set_attribute(container, "style", style);
        doc.append_child(body, container);
        let parent = match wrapper {
            Some(s) => {
                let span = doc.create_element("span");
                doc.set_attribute(span, "style", s);
                doc.append_child(container, span);
                span
            }
            None => container,
        };
        let text = doc.create_text(&"wwwwwwwwww ".repeat(5));
        doc.append_child(parent, text);
        doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
        let (x, y) = max_scroll(&scrollbars(&doc.tree, container.0, 1.0));
        assert!(
            x.is_some_and(|x| x > 380.0),
            "Chrome: 596 - 200 of horizontal travel ({wrapper:?}): got {x:?}"
        );
        assert_eq!(y, None, "one 20px line fits ({wrapper:?})");
        assert!(
            doc.scroll_width(container) > 580.0,
            "the wheel's range too ({wrapper:?})"
        );
    }
}

/// The walk goes **through** a `display: contents` wrapper and **stops at the
/// first real box**: a scroll container nested inside one is the outer's
/// content by its own 150x80 box, and its 700x500 content is its own to
/// scroll. Chrome 153: the outer reports 200x100 (no overflow).
///
/// Kills a descent that recurses into any child with children rather than
/// only into box-less wrappers (measured: the 700x500 leaks into the outer).
#[test]
fn the_walk_stops_at_a_nested_scroller() {
    let (doc, id) = scroller(
        SCROLLER,
        &[El(
            CONTENTS,
            vec![El(
                "width: 150px; height: 80px; overflow: auto",
                vec![El("width: 700px; height: 500px", vec![])],
            )],
        )],
    );
    assert_eq!(
        ranges(&doc, id),
        ((150.0, 80.0), (None, None), (150.0, 80.0)),
        "Chrome: 200x100, the inner scroller keeps its content"
    );
}

// ── Text beside a block child (issue #995) ──────────────────────────────────
//
// A container holding text **and** a block-level child is not an IFC root:
// each run of its inline content is laid out by an anonymous block box
// (CSS 2.1 §9.2.1.1), and inside a split inline (#513) by the box around its
// fragment. Neither box is in the element tree — no node's `children` holds
// it — so a walk of `children` never reached one, and the text nodes carry no
// box of their own. The range stopped at the last *element* box. The walk now
// reads the **box tree** (`RinchDocument::box_tree_children`).
//
// Same words as above: one `wwwwwwwwww` per 20px line in a 200px box. Chrome
// 153, `* { box-sizing: border-box; border-width: 0 }`, zero-size
// `::-webkit-scrollbar`, a 200x100 `overflow: auto` container with
// `font-size: 16px; line-height: 20px`, `B` a 100x50 `div`:
//
// | markup | `scrollWidth`x`scrollHeight` |
// |---|---|
// | 5 words, B, 5 words | 200x250 |
// | 5 words, B, `contents` span > 5 words | 200x250 |
// | inline `span` > (5 words, B, 5 words) | 200x250 |
// | `padding: 10px`, 5 words, B, 5 words | 200x270 |
// | `white-space: nowrap`, B, 5 words | 596x100 |
// | B, 3 words | 200x110 |

const WORD5: &str = "wwwwwwwwww wwwwwwwwww wwwwwwwwww wwwwwwwwww wwwwwwwwww ";

/// A piece of mixed content under one parent.
enum Mixed {
    Text(&'static str),
    Block,
    /// A `span` with this style, around these pieces.
    Span(&'static str, Vec<Mixed>),
}

fn build_mixed(doc: &mut RinchDocument, parent: NodeId, piece: &Mixed) {
    match piece {
        Mixed::Text(t) => {
            let text = doc.create_text(t);
            doc.append_child(parent, text);
        }
        Mixed::Block => {
            let div = doc.create_element("div");
            doc.set_attribute(div, "style", "width: 100px; height: 50px");
            doc.append_child(parent, div);
        }
        Mixed::Span(style, pieces) => {
            let span = doc.create_element("span");
            doc.set_attribute(span, "style", style);
            doc.append_child(parent, span);
            for p in pieces {
                build_mixed(doc, span, p);
            }
        }
    }
}

fn mixed_scroller(extra_style: &str, pieces: &[Mixed]) -> (RinchDocument, usize) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        &format!(
            "width: 200px; height: 100px; overflow: auto; font-size: 16px; \
             line-height: 20px; {extra_style}"
        ),
    );
    doc.append_child(body, container);
    for p in pieces {
        build_mixed(&mut doc, container, p);
    }
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    (doc, container.0)
}

/// Issue #995's own shape. Chrome: 250px of content, 150px of travel.
/// Unfixed: 150px of content (the block's bottom), 50px of travel — the
/// second run's 100px is out of reach of the wheel and the bar.
#[test]
fn text_after_a_block_child_is_the_containers_content() {
    let (doc, id) = mixed_scroller("", &[Mixed::Text(WORD5), Mixed::Block, Mixed::Text(WORD5)]);
    assert_eq!(
        text_ranges(&doc, id),
        (None, Some(150.0), 250.0),
        "Chrome: 250 - 100"
    );
}

/// The log-panel shape the issue names: the trailing run is reactive text, so
/// it sits in a `display: contents` span *and* an anonymous box.
#[test]
fn reactive_text_after_a_block_child_is_the_containers_content() {
    let (doc, id) = mixed_scroller(
        "",
        &[
            Mixed::Text(WORD5),
            Mixed::Block,
            Mixed::Span("display: contents", vec![Mixed::Text(WORD5)]),
        ],
    );
    assert_eq!(
        text_ranges(&doc, id),
        (None, Some(150.0), 250.0),
        "Chrome: 250 - 100"
    );
}

/// A split inline (#513): the block sits inside an inline `span`, so both runs
/// are fragments of that span, each laid out by an anonymous box.
#[test]
fn text_after_a_block_inside_a_split_inline_is_the_containers_content() {
    let (doc, id) = mixed_scroller(
        "",
        &[Mixed::Span(
            "",
            vec![Mixed::Text(WORD5), Mixed::Block, Mixed::Text(WORD5)],
        )],
    );
    assert_eq!(
        text_ranges(&doc, id),
        (None, Some(150.0), 250.0),
        "Chrome: 250 - 100"
    );
}

/// Off the zero-padding fixed point: an anonymous box sits in the content box,
/// so its rect is measured in the same frame as an element child's. Chrome:
/// 270px of content (10 + 250 + 10), 170px of travel.
#[test]
fn a_padded_mixed_scroller_measures_its_anonymous_boxes_in_the_content_frame() {
    let (doc, id) = mixed_scroller(
        "padding: 10px",
        &[Mixed::Text(WORD5), Mixed::Block, Mixed::Text(WORD5)],
    );
    assert_eq!(
        text_ranges(&doc, id),
        (None, Some(170.0), 250.0),
        "Chrome: 270 - 100"
    );
}

/// Off the "the run is taller than the block" point: three lines after the
/// block, 50 + 60 = 110px. Chrome: 10px of travel.
#[test]
fn a_short_run_after_a_block_adds_its_own_height() {
    let (doc, id) = mixed_scroller(
        "",
        &[
            Mixed::Block,
            Mixed::Text("wwwwwwwwww wwwwwwwwww wwwwwwwwww"),
        ],
    );
    assert_eq!(
        text_ranges(&doc, id),
        (None, Some(10.0), 110.0),
        "Chrome: 110 - 100"
    );
}

/// `white-space: nowrap` text in an anonymous box overflows the box's own
/// width, which is the container's: the line's advance is measured, as it is
/// for an IFC-root container. Chrome: 596x100. Five words are over 580px in
/// every sans face in reach, so the assertion holds on any host.
///
/// Kills a walk that reads an anonymous box's rect and not its lines.
#[test]
fn nowrap_text_after_a_block_scrolls_horizontally_by_its_line() {
    let (doc, id) = mixed_scroller("white-space: nowrap", &[Mixed::Block, Mixed::Text(WORD5)]);
    let (x, y) = max_scroll(&scrollbars(&doc.tree, id, 1.0));
    assert!(
        x.is_some_and(|x| x > 380.0),
        "Chrome: 596 - 200 of horizontal travel: got {x:?}"
    );
    assert_eq!(y, None, "50 + 20 fits in 100");
    assert!(
        doc.scroll_width(NodeId(id)) > 580.0,
        "the wheel's range too"
    );
}

/// An `absolute` box inside a flowed inline `span` beside a block child: the
/// box is hoisted out of the span into the container's box list (#591), so the
/// box-tree walk reaches it where the element walk stopped at the span's `0x0`
/// box. The containing-block rule still decides. Chrome 153, `B` then
/// `span > ("x", absolute 10x10 at top: 300px)`:
///
/// | container | `scrollWidth`x`scrollHeight` |
/// |---|---|
/// | `position: relative` | 200x310 |
/// | static | 200x100 (no overflow: 50 + one 20px line = 70) |
///
/// Unfixed, the relative container reported 50px (the block's bottom).
#[test]
fn a_hoisted_absolute_beside_a_block_counts_only_where_the_container_holds_it() {
    for (position, expected) in [("relative", (Some(210.0), 310.0)), ("static", (None, 70.0))] {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let container = doc.create_element("div");
        doc.set_attribute(
            container,
            "style",
            &format!(
                "width: 200px; height: 100px; overflow: auto; position: {position}; \
                 font-size: 16px; line-height: 20px"
            ),
        );
        doc.append_child(body, container);
        build_mixed(&mut doc, container, &Mixed::Block);
        let span = doc.create_element("span");
        doc.append_child(container, span);
        let x = doc.create_text("x");
        doc.append_child(span, x);
        let abs = doc.create_element("span");
        doc.set_attribute(
            abs,
            "style",
            "position: absolute; left: 0; top: 300px; width: 10px; height: 10px",
        );
        doc.append_child(span, abs);
        doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
        let (_, y) = max_scroll(&scrollbars(&doc.tree, container.0, 1.0));
        assert_eq!(
            (y, doc.scroll_height(container)),
            expected,
            "{position} container: Chrome 310, or no overflow at all"
        );
    }
}
