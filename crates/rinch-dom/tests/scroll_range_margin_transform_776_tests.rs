//! How each child box contributes to a scroll container's **scrollable
//! overflow** (issues #776 and #775): an in-flow child by its **margin box**,
//! a transformed child also by its **transformed** border box, an `absolute`
//! child by its border box alone, and a `relative` child at both its static
//! and its offset position.
//!
//! Every expectation is **measured in Chrome 156** (headless, 800x600) on the
//! same markup, under `* { box-sizing: border-box; margin: 0; padding: 0;
//! border-width: 0 }` and `::-webkit-scrollbar { width: 0; height: 0 }`, as
//! `scrollWidth - clientWidth` x `scrollHeight - clientHeight`. That quantity
//! is what `ScrollbarTrack::max_scroll` is. Chrome's numbers are integers
//! (snapped); where the box has fractional edges the expected value below is
//! the exact one, and the comment gives Chrome's.
//!
//! Most rows use the padded, bordered container `PB` — every padding and every
//! border different (`border-width: 2px 5px 4px 3px; padding: 7px 11px 13px
//! 17px`), so a rule that confuses start and end padding, or padding and
//! border, gets a different number. Its client box is 192x94 and its content
//! box 164x74. Chrome measures in the padding-box frame and rinch in the
//! content-box frame; the two agree on an in-flow box (Chrome extends the
//! in-flow area by the end padding and the paddings cancel) and differ by the
//! end padding for a box that is not in flow (`absolute`, or the transformed or
//! offset position of an in-flow one), which Chrome does **not** extend.
//!
//! | row | child | Chrome max scroll |
//! |---|---|---|
//! | m1 | `PB`: 100x50, `margin-right: 150px; margin-bottom: 70px` | 86x46 |
//! | m2 | 200x100: 300x50, `margin-right: 40px` | 140x0 |
//! | m3 | 200x100: 100x150, `margin-bottom: -30px` | 0x50 |
//! | m4 | `PB`: 100x50, `margin-right: 60%` | 34x0 (34.4) |
//! | m5 | `PB` flex: m1's child, `flex-shrink: 0` | 86x46 |
//! | m6 | `PB` grid: m1's child | 86x46 (here 0x0: a known gap) |
//! | m7 | `PB`: 100x50 `mb 70px`, then 100x20 `mb 9px` | 0x75 |
//! | t1 | `PB`: 100x50, `translate(500px, 300px)` | 425x263 |
//! | t2 | `PB`: 180x80, `rotate(30deg)` | 16x33 (32.64) |
//! | t3 | `PB`: 120x60, `transform-origin: 0 0; scale(2)` | 65x33 |
//! | t4 | `PB`: m1's child plus `translate(50px, 10px)` | 86x46 |
//! | t5 | `PB` flex: 100x50, `translate(170px, 90px)` | 95x53 |
//! | a1 | `PB`: `absolute; left: 0; top: 0` 210x50 | 18x0 |
//! | a2 | `PB`: `absolute; inset: 0` | 0x0 |
//! | a3 | `PB`: `absolute; left: 0; top: 0` 100x50 with m1's margins | 0x0 |
//! | a4 | `PB`: `absolute; left: 0; top: 0` 100x50, `translate(170px, 90px)` | 78x46 |
//! | a5 | `PB`: `absolute; left: 0; top: 0` 300x150, `translate(-150px, -100px)` | 0x0 |
//! | a6 | `PB`: `absolute` (no insets) 300x200 | 125x113 |
//! | r1 | `PB`: `relative; left: 40px; top: 5px` 210x50 | 75x0 |
//! | r2 | 200x100: `relative; left: -40px` 240x50 | 40x0 |
//! | r3 | 200x100: `relative; top: -40px` 100x130 | 0x30 |
//! | r4 | `PB`: `relative; left: 10px; top: 20px` with m1's size and margins | 86x46 |
//! | r5 | `PB`: `relative; right: 30px; bottom: 10px` 250x120 | 86x46 |
//! | r6 | `PB`: `relative; left: 10%; top: 10%` 150x60 | 0x0 |
//! | r7 | `PB`: `relative; left: 20px` 150x60, `translate(30px, 40px)` | 25x13 |
//! | r8 | `PB`: `relative; top: -10%` 100x90 | 0x16 |
//! | r9 | `PB`: `relative; left: -10%` 180x50 | 16x0 |
//! | n1 | 200x100: 300x50, `margin-right: -50px` | 100x0 |
//! | r10 | 200x100: `relative; top: 60px` 100x80 | 0x40 |
//! | i1 | 200x100: `<span style="margin-bottom: 200px">x</span>` | 0x0 |
//! | i2 | 200x100: `<b style="margin-bottom: 150px">x</b><i>y</i>` | 0x0 |
//! | i3 | 200x100: `<span style="margin-right: 300px">x</span>` | 107x0 (the line) |
//! | i4 | 200x100: `<img>` 50x20 `vertical-align: top`, `margin-bottom: 200px` | 0x120 |
//! | i5 | the same `<img>`, `margin-right: 200px` | 50x0 |
//! | i6 | 200x100: `inline-block` 50x20 `vertical-align: top`, `margin-bottom: 200px` | 0x120 |
//! | i7 | the same as `inline-flex` / `inline-grid` with `margin-right: 200px` | 50x0 |
//! | g1 | 200x100 grid `100px 100px`: 50x20, `margin-right: 100%` | 0x0 |
//! | g2 | g1 with `margin-right: 200%` | 0x0 |
//! | g3 | 200x100 grid (auto column): 50x20, `margin-right: 150%` | 0x0 |
//! | g4 | g1's grid: 50x20, `margin-bottom: 200%` | 0x120 (here 0: a known gap) |
//! | g5 | g1's grid: 50x20, `margin-right: 300px` | 0x0 |
//!
//! A grid item counts its border box alone (as before #776): Chrome counts
//! the grid's tracks, which layout does not keep, so m6 and g4 — a margin
//! growing an `auto` track — are known gaps, pinned at rinch's numbers.
//!
//! A `position: sticky` child is laid out by Taffy as a `relative` one at its
//! inset (`top: 20px` scrolls 0x20 here, 0x0 in Chrome, before #776 too), so
//! no row pins sticky: it would pin rinch's own wrong layout.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::scrollbar::scrollbars;

const CSS: &str = "* { box-sizing: border-box; margin: 0; padding: 0; border-width: 0; }";

/// A plain 200x100 scroll container.
const B: &str = "width: 200px; height: 100px; overflow: auto;";
/// [`B`] with every border and padding different, and `position: relative` so
/// it is its absolutes' containing block.
const PB: &str = "width: 200px; height: 100px; overflow: auto; border-style: solid; \
                  border-width: 2px 5px 4px 3px; padding: 7px 11px 13px 17px; position: relative;";

/// One scroll container under `<body>` holding one `div` per entry of
/// `children`, laid out at 800x600.
fn scroller(container_style: &str, children: &[&str]) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(container, "style", container_style);
    doc.append_child(body, container);
    for style in children {
        let child = doc.create_element("div");
        doc.set_attribute(child, "style", style);
        doc.append_child(container, child);
    }
    doc.resolve_layout(800.0, 600.0);
    (doc, container)
}

/// `(max_scroll_x, max_scroll_y)` of the bars, `0.0` for an axis with none.
fn max_scroll(container_style: &str, children: &[&str]) -> (f64, f64) {
    let (doc, id) = scroller(container_style, children);
    ranges(&doc, id)
}

/// [`max_scroll`] of a scroll container whose content is `html`, with
/// `line-height: 16px; font-size: 14px` declared so a line's height does not
/// come from the host's fonts.
fn max_scroll_html(container_style: &str, html: &str) -> (f64, f64) {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        &format!("{container_style} line-height: 16px; font-size: 14px;"),
    );
    doc.append_child(body, container);
    doc.set_inner_html(container, html);
    doc.resolve_layout(800.0, 600.0);
    ranges(&doc, container)
}

/// The bars' ranges of `id`, checked against the wheel's.
fn ranges(doc: &RinchDocument, id: NodeId) -> (f64, f64) {
    let bars = scrollbars(&doc.tree, id.0, 1.0);
    let x = bars.horizontal.map_or(0.0, |t| t.max_scroll);
    let y = bars.vertical.map_or(0.0, |t| t.max_scroll);
    // The wheel and `scroll_into_view` clamp to `scroll_* - client_*`; it
    // must be the bar's range, or the thumb and the wheel disagree.
    let wheel_x = (doc.scroll_width(id) - doc.client_width(id)).max(0.0);
    let wheel_y = (doc.scroll_height(id) - doc.client_height(id)).max(0.0);
    assert!(
        (wheel_x - x).abs() < 1e-6 && (wheel_y - y).abs() < 1e-6,
        "the wheel's range ({wheel_x}, {wheel_y}) is the bars' ({x}, {y})"
    );
    (x, y)
}

#[track_caller]
fn assert_range(got: (f64, f64), want: (f64, f64)) {
    assert!(
        (got.0 - want.0).abs() < 0.01 && (got.1 - want.1).abs() < 0.01,
        "max scroll {got:?}, Chrome {want:?}"
    );
}

// ── In-flow children: the margin box (#776) ─────────────────────────────

/// Issue #776's own shape, off the fixed points: a padded, bordered container,
/// and end margins on both axes. Before the fix rinch measured the 100x50
/// border box, which fits, and drew no bar at all.
#[test]
fn an_in_flow_childs_end_margins_are_scrollable() {
    let child = "width: 100px; height: 50px; margin-right: 150px; margin-bottom: 70px";
    assert_range(max_scroll(PB, &[child]), (86.0, 46.0));
    // The same box as a flex item. (As a grid item it is a known gap:
    // `a_grid_items_margins_count_nothing`.)
    assert_range(
        max_scroll(
            &format!("{PB} display: flex;"),
            &[&format!("{child}; flex-shrink: 0")],
        ),
        (86.0, 46.0),
    );
}

/// The margin counts as **declared**: an over-constrained block's used
/// `margin-right` is negative (it is what absorbs the over-constraint), but
/// Chrome counts the 40px the author wrote — 300 + 40 = 340.
#[test]
fn an_overconstrained_blocks_declared_end_margin_counts() {
    assert_range(
        max_scroll(B, &["width: 300px; height: 50px; margin-right: 40px"]),
        (140.0, 0.0),
    );
}

/// A negative end margin pulls the margin box inside the border box, which
/// still counts: 150px tall with `margin-bottom: -30px` scrolls 50, not 20.
#[test]
fn a_negative_end_margin_does_not_shrink_the_border_box() {
    assert_range(
        max_scroll(B, &["width: 100px; height: 150px; margin-bottom: -30px"]),
        (0.0, 50.0),
    );
}

/// A percentage margin resolves against the containing block's width — the
/// container's 164px content box: 100 + 0.6 * 164 = 198.4, so 34.4 of travel
/// (Chrome: 34).
#[test]
fn a_percentage_end_margin_resolves_against_the_content_box() {
    assert_range(
        max_scroll(PB, &["width: 100px; height: 50px; margin-right: 60%"]),
        (34.4, 0.0),
    );
}

/// Only the last child's block-end margin reaches the end; an earlier one is
/// already in the next child's position. 50 + 70 + 20 + 9 = 149 against 74.
#[test]
fn the_last_childs_end_margin_counts_after_its_siblings() {
    assert_range(
        max_scroll(
            PB,
            &[
                "width: 100px; height: 50px; margin-bottom: 70px",
                "width: 100px; height: 20px; margin-bottom: 9px",
            ],
        ),
        (0.0, 75.0),
    );
}

// ── Transformed children (#776) ─────────────────────────────────────────

/// Issue #776's second shape, padded: a translated box counts where it is
/// drawn, in the padding-box frame and **without** the end padding — 600 - 11
/// = 589 against 164, and 350 - 13 = 337 against 74.
#[test]
fn a_translated_child_is_scrollable_where_it_is_drawn() {
    assert_range(
        max_scroll(
            PB,
            &["width: 100px; height: 50px; transform: translate(500px, 300px)"],
        ),
        (425.0, 263.0),
    );
}

/// A rotation that is not a multiple of 90°: the transformed box's bounding
/// box is 187.94 x 119.64 about the 180x80 box's centre. Horizontally the
/// untransformed in-flow box (180, extended by the end padding) wins, 16;
/// vertically the rotated one, 119.64 - 13 - 74 = 32.64 (Chrome: 33).
#[test]
fn a_rotated_childs_bounding_box_is_scrollable() {
    assert_range(
        max_scroll(
            PB,
            &["width: 180px; height: 80px; transform: rotate(30deg)"],
        ),
        (16.0, 32.64),
    );
}

/// A scale about the top-left corner doubles the box to 240x120.
#[test]
fn a_scaled_child_is_scrollable_at_its_scaled_size() {
    assert_range(
        max_scroll(
            PB,
            &["width: 120px; height: 60px; transform-origin: 0 0; transform: scale(2)"],
        ),
        (65.0, 33.0),
    );
}

/// A transformed in-flow box contributes **both** its untransformed margin
/// box (in flow, end padding included) and its transformed border box: here
/// the margin box wins on both axes.
#[test]
fn a_transformed_childs_margin_box_still_counts_untransformed() {
    assert_range(
        max_scroll(
            PB,
            &[
                "width: 100px; height: 50px; margin-right: 150px; margin-bottom: 70px; \
               transform: translate(50px, 10px)",
            ],
        ),
        (86.0, 46.0),
    );
}

/// A transformed flex item, as for a block child.
#[test]
fn a_translated_flex_item_is_scrollable_where_it_is_drawn() {
    assert_range(
        max_scroll(
            &format!("{PB} display: flex;"),
            &["flex-shrink: 0; width: 100px; height: 50px; transform: translate(170px, 90px)"],
        ),
        (95.0, 53.0),
    );
}

// ── `absolute` children: the border box, no end padding, no margins (#775) ──

/// `left: 0` is the padding edge: the box ends at 210 in the padding frame,
/// against the 192px client box — 18. rinch used to add the end padding: 29.
#[test]
fn an_absolute_childs_border_box_counts_without_the_end_padding() {
    assert_range(
        max_scroll(
            PB,
            &["position: absolute; left: 0; top: 0; width: 210px; height: 50px"],
        ),
        (18.0, 0.0),
    );
}

/// `inset: 0` fills the padding box exactly, so nothing overflows. rinch used
/// to report the end paddings, 11x13, and paint two bars (issue #775's
/// `LoadingOverlay` shape).
#[test]
fn an_inset_zero_absolute_paints_no_bars() {
    let (doc, id) = scroller(PB, &["position: absolute; inset: 0"]);
    let bars = scrollbars(&doc.tree, id.0, 1.0);
    assert!(
        bars.horizontal.is_none() && bars.vertical.is_none(),
        "Chrome: no overflow; got {bars:?}"
    );
}

/// An absolute box's margins are no part of the scrollable area.
#[test]
fn an_absolute_childs_margins_do_not_count() {
    assert_range(
        max_scroll(
            PB,
            &[
                "position: absolute; left: 0; top: 0; width: 100px; height: 50px; \
               margin-right: 150px; margin-bottom: 70px",
            ],
        ),
        (0.0, 0.0),
    );
}

/// A transformed absolute counts where it is drawn: (-17 + 170 + 100) - 11 =
/// 242 against 164, and (-7 + 90 + 50) - 13 = 120 against 74.
#[test]
fn a_translated_absolute_counts_where_it_is_drawn() {
    assert_range(
        max_scroll(
            PB,
            &[
                "position: absolute; left: 0; top: 0; width: 100px; height: 50px; \
               transform: translate(170px, 90px)",
            ],
        ),
        (78.0, 46.0),
    );
}

/// …and **only** there: an absolute translated back inside the padding box
/// overflows nothing, though its untransformed 300x150 box would.
#[test]
fn an_absolute_translated_back_inside_overflows_nothing() {
    assert_range(
        max_scroll(
            PB,
            &[
                "position: absolute; left: 0; top: 0; width: 300px; height: 150px; \
               transform: translate(-150px, -100px)",
            ],
        ),
        (0.0, 0.0),
    );
}

/// An absolute with no insets sits at its static position, the content edge:
/// 300 - 11 - 164 = 125 and 200 - 13 - 74 = 113.
#[test]
fn an_absolute_at_its_static_position_counts_without_the_end_padding() {
    assert_range(
        max_scroll(PB, &["position: absolute; width: 300px; height: 200px"]),
        (125.0, 113.0),
    );
}

// ── `relative` children: static and offset positions (#775) ─────────────

/// Toward the end: the offset box (250 wide, unextended: 250 - 11 = 239)
/// beats the static one (210). rinch used to extend the offset box: 86.
#[test]
fn a_relative_offset_toward_the_end_counts_without_the_end_padding() {
    assert_range(
        max_scroll(
            PB,
            &["position: relative; left: 40px; top: 5px; width: 210px; height: 50px"],
        ),
        (75.0, 0.0),
    );
}

/// Toward the start, with no padding at all: the static box still counts.
/// rinch used to read only the offset box, which fits, so content Chrome
/// scrolls to was unreachable.
#[test]
fn a_relative_offset_toward_the_start_keeps_the_static_box() {
    assert_range(
        max_scroll(
            B,
            &["position: relative; left: -40px; width: 240px; height: 50px"],
        ),
        (40.0, 0.0),
    );
    assert_range(
        max_scroll(
            B,
            &["position: relative; top: -40px; width: 100px; height: 130px"],
        ),
        (0.0, 30.0),
    );
}

/// `right`/`bottom` offsets, and a relative box's margins at its static
/// position.
#[test]
fn a_relative_childs_static_margin_box_counts() {
    assert_range(
        max_scroll(
            PB,
            &[
                "position: relative; left: 10px; top: 20px; width: 100px; height: 50px; \
               margin-right: 150px; margin-bottom: 70px",
            ],
        ),
        (86.0, 46.0),
    );
    assert_range(
        max_scroll(
            PB,
            &["position: relative; right: 30px; bottom: 10px; width: 250px; height: 120px"],
        ),
        (86.0, 46.0),
    );
}

/// Percentage offsets resolve against the content box (16.4 x 7.4): the
/// static 150x60 box fits, and so does the offset one once the end padding
/// is not added (166.4 - 11 < 164). Resolving the offset to anything else
/// moves the static box and overflows.
#[test]
fn a_percentage_relative_offset_overflows_nothing() {
    assert_range(
        max_scroll(
            PB,
            &["position: relative; left: 10%; top: 10%; width: 150px; height: 60px"],
        ),
        (0.0, 0.0),
    );
}

/// An offset and a transform together: the transformed box at the offset
/// position, 20 + 30 + 150 - 11 = 189 against 164, and 40 + 60 - 13 = 87
/// against 74.
#[test]
fn a_relative_and_translated_child_counts_where_it_is_drawn() {
    assert_range(
        max_scroll(
            PB,
            &[
                "position: relative; left: 20px; width: 150px; height: 60px; \
               transform: translate(30px, 40px)",
            ],
        ),
        (25.0, 13.0),
    );
}

/// Start-ward percentage offsets: the static box is what overflows, so the
/// offset taken off the layout rect must be the one Taffy applied — `top`
/// against the content box's **height** (-7.4), `left` against its width
/// (-16.4). A 90px-tall box scrolls 90 - 74 = 16; a 180px-wide one 16.
#[test]
fn a_startward_percentage_offset_keeps_the_static_box() {
    assert_range(
        max_scroll(
            PB,
            &["position: relative; top: -10%; width: 100px; height: 90px"],
        ),
        (0.0, 16.0),
    );
    assert_range(
        max_scroll(
            PB,
            &["position: relative; left: -10%; width: 180px; height: 50px"],
        ),
        (16.0, 0.0),
    );
}

// ── Review of #2004: the remaining branches, inline boxes, grid items ───

/// The floor on the **inline** axis too: a 300px block with `margin-right:
/// -50px` scrolls its whole border box, 100. Without the floor it was 50.
#[test]
fn a_negative_end_margin_does_not_shrink_the_border_box_inline() {
    assert_range(
        max_scroll(B, &["width: 300px; height: 50px; margin-right: -50px"]),
        (100.0, 0.0),
    );
}

/// A `relative` offset on the **block** axis alone still counts where the box
/// is drawn: 60 + 80 - 100 = 40. Counting the drawn box only for an x offset
/// gave 0.
#[test]
fn a_relative_offset_down_counts_where_it_is_drawn() {
    assert_range(
        max_scroll(
            B,
            &["position: relative; top: 60px; width: 100px; height: 80px"],
        ),
        (0.0, 40.0),
    );
}

/// A non-replaced `display: inline` box's vertical margins do nothing (CSS
/// 2.1 §10.6.1), so they add no range: Chrome 0x0 for both. Counting its
/// margin box (the first cut of #776) gave 0x100 and 0x50 — a bar main never
/// drew.
#[test]
fn an_inline_boxs_vertical_margin_adds_no_range() {
    assert_range(
        max_scroll_html(B, "<span style='margin-bottom: 200px'>x</span>"),
        (0.0, 0.0),
    );
    assert_range(
        max_scroll_html(B, "<b style='margin-bottom: 150px'>x</b><i>y</i>"),
        (0.0, 0.0),
    );
}

/// Its inline-axis margin is in its line, which the container's inline
/// layout measures: 300 + the glyph's width - 200. Chrome 107; the glyph's
/// width is the host font's, so only the margin's share is pinned.
#[test]
fn an_inline_boxs_end_margin_is_in_its_line() {
    let (x, y) = max_scroll_html(B, "<span style='margin-right: 300px'>x</span>");
    assert!((100.0..120.0).contains(&x) && y == 0.0, "({x}, {y})");
}

/// A replaced element at `display: inline` is an atomic inline, and its
/// margins count on both axes: 20 + 200 - 100 = 120 and 50 + 200 - 200 = 50
/// (Chrome 0x120, 50x0).
#[test]
fn an_inline_images_margins_still_count() {
    let img = "width: 50px; height: 20px; vertical-align: top";
    assert_range(
        max_scroll_html(B, &format!("<img style='{img}; margin-bottom: 200px'>")),
        (0.0, 120.0),
    );
    assert_range(
        max_scroll_html(B, &format!("<img style='{img}; margin-right: 200px'>")),
        (50.0, 0.0),
    );
}

/// So do an `inline-block`'s, an `inline-flex`'s and an `inline-grid`'s
/// (Chrome 0x120, 50x0, 50x0).
#[test]
fn an_atomic_inlines_margins_still_count() {
    let b = "width: 50px; height: 20px; vertical-align: top";
    assert_range(
        max_scroll_html(
            B,
            &format!("<div style='display: inline-block; {b}; margin-bottom: 200px'></div>"),
        ),
        (0.0, 120.0),
    );
    for display in ["inline-flex", "inline-grid"] {
        assert_range(
            max_scroll_html(
                B,
                &format!("<div style='display: {display}; {b}; margin-right: 200px'></div>"),
            ),
            (50.0, 0.0),
        );
    }
}

/// A grid item's margins count nothing: Chrome counts the grid's tracks, and
/// a margin past a fixed track is not in them. `margin-right: 100%` or `200%`
/// (of the 100px area) on a 50px item in a 100px column, `150%` in an auto
/// column, and `margin-right: 300px` in the 100px column all scroll 0 in
/// Chrome. The item's margin box gave 50, 250, 150 and 150.
#[test]
fn a_grid_items_margins_count_nothing() {
    let item = "<div style='width: 50px; height: 20px; margin-right: {}'></div>";
    let two = format!("{B} display: grid; grid-template-columns: 100px 100px;");
    for margin in ["100%", "200%", "300px"] {
        assert_range(
            max_scroll_html(&two, &item.replace("{}", margin)),
            (0.0, 0.0),
        );
    }
    assert_range(
        max_scroll_html(&format!("{B} display: grid;"), &item.replace("{}", "150%")),
        (0.0, 0.0),
    );
}

/// **Known gaps**, pinned at rinch's numbers (main's too) so a fix shows: a
/// margin that grows an `auto` track is in Chrome's tracks. m1's child as the
/// item of an auto grid (Chrome 86x46), and `margin-bottom: 200%` of the
/// 100px area in an auto row (Chrome 0x120).
#[test]
fn a_grid_items_margin_growing_an_auto_track_known_gaps() {
    let child = "width: 100px; height: 50px; margin-right: 150px; margin-bottom: 70px";
    assert_range(
        max_scroll(&format!("{PB} display: grid;"), &[child]),
        (0.0, 0.0),
    );
    assert_range(
        max_scroll_html(
            &format!("{B} display: grid; grid-template-columns: 100px 100px;"),
            "<div style='width: 50px; height: 20px; margin-bottom: 200%'></div>",
        ),
        (0.0, 0.0),
    );
}
