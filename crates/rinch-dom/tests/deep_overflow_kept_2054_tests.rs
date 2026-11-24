//! #2054: a scroll container's kept descendant walk, brought up to date in
//! place, against a fresh document given the same changes; and what each
//! change costs it (`testing::deep_overflow_walks`, `deep_overflow_measured`).
//! Random sequences of edits are in `deep_overflow_kept_fuzz_2054_tests.rs`.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

// ── The kept walk against a fresh document (#2054 review) ─────────────────
//
// Each scenario primes the scroller's kept walk (a read, and a paint
// consuming the tree), makes its changes with a layout and a read after
// each, and reads the range in three orders; each must equal what a fresh
// document given the same changes, never read in between, answers.

type Step = fn(&mut RinchDocument);

fn kept_doc(html: &str) -> RinchDocument {
    let mut d = RinchDocument::new();
    d.load_css("body { margin: 0 } div { margin: 0; padding: 0 }");
    let body = d.body();
    d.set_inner_html(body, html);
    d.resolve_layout(800.0, 600.0);
    d
}

fn range_of(d: &RinchDocument) -> (f64, f64) {
    let s = d.query_selector("#s").expect("#s");
    (d.scroll_width(s), d.scroll_height(s))
}

fn by_id(d: &RinchDocument, id: &str) -> rinch_core::dom::NodeId {
    d.query_selector(&format!("#{id}")).expect(id)
}

fn kept_matches_fresh(what: &str, html: &str, steps: &[Step]) {
    let fresh = {
        let mut d = kept_doc(html);
        for step in steps {
            step(&mut d);
            d.resolve_layout(800.0, 600.0);
        }
        range_of(&d)
    };
    for order in 0..3 {
        let mut d = kept_doc(html);
        let _ = range_of(&d);
        d.tree.consume_paint_dirty();
        let _ = range_of(&d);
        for step in steps {
            step(&mut d);
            d.resolve_layout(800.0, 600.0);
            if order != 1 {
                let _ = range_of(&d);
            }
        }
        let got = match order {
            0 => range_of(&d),
            1 => {
                d.tree.consume_paint_dirty();
                range_of(&d)
            }
            _ => {
                d.tree.consume_paint_dirty();
                let _ = range_of(&d);
                range_of(&d)
            }
        };
        assert_eq!(got, fresh, "{what}, read order {order}");
    }
}

const TALL: &str = r#"<div id="s" style="width:200px;height:100px;overflow:auto"><div id="c" style="height:50px;width:100px"><div id="g" style="height:300px;width:150px"></div></div></div>"#;
const TALL_HIDDEN: &str = r#"<div id="s" style="width:200px;height:100px;overflow:auto"><div id="c" style="height:50px;width:100px;overflow:hidden"><div id="g" style="height:300px;width:150px"></div></div></div>"#;

#[test]
fn a_box_made_to_clip_drops_the_kept_walk() {
    kept_matches_fresh(
        "overflow: hidden",
        TALL,
        &[|d| {
            let c = by_id(d, "c");
            d.set_style(c, "overflow", "hidden");
        }],
    );
    kept_matches_fresh(
        "overflow: clip",
        TALL,
        &[|d| {
            let c = by_id(d, "c");
            d.set_style(c, "overflow", "clip");
        }],
    );
}

#[test]
fn a_box_that_stops_clipping_drops_the_kept_walk() {
    kept_matches_fresh(
        "hidden to visible",
        TALL_HIDDEN,
        &[|d| {
            let c = by_id(d, "c");
            d.set_style(c, "overflow", "visible");
        }],
    );
}

/// A change and its undoing between two paints: the walk taken in between
/// must not stand.
#[test]
fn a_change_and_back_before_a_paint_drops_the_kept_walk() {
    kept_matches_fresh(
        "900 then 300",
        TALL,
        &[
            |d| {
                let g = by_id(d, "g");
                d.set_style(g, "width", "900px");
            },
            |d| {
                let g = by_id(d, "g");
                d.set_style(g, "width", "150px");
            },
        ],
    );
}

/// More of the family: a grandchild's lines re-shaped wider, a transform
/// given with no layout, a box taken out of the display, a box added under
/// one the walk entered, and a block collapsed to an estimated height (its
/// content, laid out before, is not entered).
#[test]
fn other_changes_under_the_scroller_drop_the_kept_walk() {
    kept_matches_fresh(
        "lines re-shaped",
        r#"<div id="s" style="width:200px;height:100px;overflow:auto"><div style="width:100px;height:20px"><div id="p" style="width:100px;white-space:nowrap;font-size:0">x</div></div></div>"#,
        &[|d| {
            let p = by_id(d, "p");
            d.set_inner_html(
                p,
                r#"<span style="display:inline-block;width:400px;height:5px"></span>"#,
            );
        }],
    );
    kept_matches_fresh(
        "a transform given",
        TALL,
        &[|d| {
            let g = by_id(d, "g");
            d.set_style(g, "transform", "translateX(300px)");
        }],
    );
    kept_matches_fresh(
        "display: none",
        TALL,
        &[|d| {
            let g = by_id(d, "g");
            d.set_style(g, "display", "none");
        }],
    );
    kept_matches_fresh(
        "a box added",
        TALL,
        &[|d| {
            let c = by_id(d, "c");
            let n = d.create_element("div");
            d.set_attribute(n, "style", "width:700px;height:5px");
            d.append_child(c, n);
        }],
    );
    kept_matches_fresh(
        "a block collapsed",
        TALL,
        &[|d| {
            let c = by_id(d, "c");
            d.set_estimated_height(c.0, Some(50.0));
        }],
    );
}

/// A block collapsed by virtualization stands for its content at its
/// estimate: what it held, laid out before, is not in the range.
#[test]
fn a_collapsed_blocks_content_is_not_in_the_range() {
    let mut d = kept_doc(TALL);
    assert_eq!(range_of(&d).1, 300.0, "control: the grandchild is reached");
    let c = by_id(&d, "c");
    d.set_estimated_height(c.0, Some(50.0));
    d.resolve_layout(800.0, 600.0);
    assert_eq!(range_of(&d).1, 50.0, "the estimate alone");
}

// ── A change inside the scroller's first box keeps the walk (#2961) ───────
//
// The perf review of #2961: a page of paragraphs in the viewport's scroller
// walked every box again on each layout that resized one of them
// (`shell::relayout_beside_text`, +9%). What is inside a top box's border
// box adds nothing to the range, so a change there keeps the walk.

const ROWS: &str = r#"<div id="s" style="width:300px;height:100px;overflow:auto"><div id="t" style="position:relative;width:300px;height:400px"><div id="wide" style="height:10px;width:600px"></div><div id="a" style="width:200px;height:20px"></div><div id="b" style="width:100px;height:20px"></div><div id="gauge" style="position:absolute;left:150px;top:200px;width:100px;height:10px"></div></div></div>"#;

/// A row resized and an absolute box resized, each inside the top box, and
/// a row made to reach past it: the range is brought up to date from the
/// kept walk, measuring only the boxes the layout moved and the top box
/// above them, and is right.
#[test]
fn a_change_inside_the_top_box_keeps_the_walk() {
    let mut d = kept_doc(ROWS);
    let s = by_id(&d, "s");
    let walks = |d: &RinchDocument| rinch_dom::testing::deep_overflow_walks(&d.tree);
    let measured = |d: &RinchDocument| rinch_dom::testing::deep_overflow_measured(&d.tree);
    assert_eq!(
        range_of(&d),
        (600.0, 400.0),
        "control: the wide row is reached"
    );
    d.tree.consume_paint_dirty();
    let _ = range_of(&d);
    let gauge = by_id(&d, "gauge");
    d.set_style(gauge, "width", "111px");
    d.resolve_layout(800.0, 600.0);
    let (before, measured_before) = (walks(&d), measured(&d));
    assert_eq!(range_of(&d), (600.0, 400.0));
    let _ = rinch_dom::paint::scrollbar::content_extents(&d.tree, s.0);
    assert_eq!(
        walks(&d),
        before,
        "the absolute box moved inside the top box"
    );
    assert_eq!(
        measured(&d) - measured_before,
        2,
        "the absolute box and the top box"
    );
    d.tree.consume_paint_dirty();
    let a = by_id(&d, "a");
    d.set_style(a, "height", "60px");
    d.resolve_layout(800.0, 600.0);
    let (before, measured_before) = (walks(&d), measured(&d));
    assert_eq!(range_of(&d), (600.0, 400.0));
    assert_eq!(walks(&d), before, "the rows moved inside the top box");
    assert_eq!(
        measured(&d) - measured_before,
        3,
        "the row resized, the row it moved, and the top box"
    );
    // A row made to reach past the top box: the range follows it, from the
    // records.
    d.tree.consume_paint_dirty();
    let b = by_id(&d, "b");
    d.set_style(b, "width", "700px");
    d.resolve_layout(800.0, 600.0);
    let (before, measured_before) = (walks(&d), measured(&d));
    assert_eq!(range_of(&d), (700.0, 400.0));
    assert_eq!(walks(&d), before, "brought up to date, not walked");
    assert_eq!(measured(&d) - measured_before, 2, "the row and the top box");
}

/// Kept against fresh, for the changes a kept walk now stands through or
/// must drop on: a box moved inside, a counted box brought back inside, a
/// box carrying a child that reaches
/// past the top box, the top box shrunk under its content, and a child
/// grown inside before its parent moves (the parent's record must hold the
/// child's new extent).
#[test]
fn changes_inside_the_top_box_match_a_fresh_document() {
    kept_matches_fresh(
        "rows resized inside",
        ROWS,
        &[
            |d| {
                let a = by_id(d, "a");
                d.set_style(a, "height", "60px");
            },
            |d| {
                let g = by_id(d, "gauge");
                d.set_style(g, "top", "300px");
            },
        ],
    );
    kept_matches_fresh(
        "a counted row comes back inside",
        ROWS,
        &[|d| {
            let w = by_id(d, "wide");
            d.set_style(w, "width", "200px");
        }],
    );
    const CARRIER: &str = r#"<div id="s" style="width:200px;height:50px;overflow:auto"><div id="t" style="width:150px;height:100px"><div id="a" style="width:100px;height:10px"><div id="g" style="width:50px;height:40px"></div></div></div></div>"#;
    kept_matches_fresh(
        "a parent moves its child past the top box",
        CARRIER,
        &[|d| {
            let a = by_id(d, "a");
            d.set_style(a, "margin-top", "70px");
        }],
    );
    kept_matches_fresh(
        "the top box shrinks under its content",
        CARRIER,
        &[|d| {
            let t = by_id(d, "t");
            d.set_style(t, "height", "30px");
        }],
    );
    kept_matches_fresh(
        "the top box grows",
        CARRIER,
        &[|d| {
            let t = by_id(d, "t");
            d.set_style(t, "height", "300px");
        }],
    );
    const GROWS: &str = r#"<div id="s" style="width:200px;height:50px;overflow:auto"><div id="t" style="width:150px;height:200px"><div id="a" style="width:100px;height:10px"><div id="g" style="width:50px;height:10px"></div></div></div></div>"#;
    kept_matches_fresh(
        "a child grows inside, then its parent moves",
        GROWS,
        &[
            |d| {
                let g = by_id(d, "g");
                d.set_style(g, "height", "150px");
            },
            |d| {
                let a = by_id(d, "a");
                d.set_style(a, "margin-top", "80px");
            },
        ],
    );
}

/// A box moved inside the top box adds what it now holds to the record of
/// each box above it (the ancestor join in `moved_inside`): `g` is moved past
/// its parent `a`, still inside `t`, and then `a` moves, taking `g` past `t`.
/// Without the join `a`'s record still holds `g` where it was, so the second
/// move is judged inside and the kept range stays at `t`'s 150.
#[test]
fn a_child_moved_past_its_parent_then_the_parent_moves() {
    const NESTED: &str = r#"<div id="s" style="width:100px;height:50px;overflow:auto"><div id="t" style="width:150px;height:200px"><div id="a" style="width:100px;height:10px"><div id="g" style="width:50px;height:10px"></div></div></div></div>"#;
    kept_matches_fresh(
        "a child moved past its parent, then the parent moves",
        NESTED,
        &[
            |d| {
                let g = by_id(d, "g");
                d.set_style(g, "margin-left", "90px");
            },
            |d| {
                let a = by_id(d, "a");
                d.set_style(a, "margin-left", "30px");
            },
        ],
    );
}

/// Padding moves what a box lays out from its content box — its lines, and
/// the atomic inlines its inline formatting context places — without moving
/// or resizing the box, so the kept walk's record of the box must say where
/// its content starts. Here `r` keeps its 280px border box and its
/// inline-block keeps its layout (relative to `r`'s content box), but the
/// padding takes the inline-block to 150..350, past `t`.
const PADDED: &str = r#"<div id="s" style="width:200px;height:50px;overflow:auto"><div id="t" style="width:300px;height:100px"><div id="r" style="box-sizing:border-box;width:280px"><div style="display:inline-block;width:200px;height:20px;vertical-align:top"></div></div></div></div>"#;

#[test]
fn padding_that_moves_an_inline_block_drops_the_kept_walk() {
    kept_matches_fresh(
        "padding moves an inline-block",
        PADDED,
        &[|d| {
            let r = by_id(d, "r");
            d.set_style(r, "padding-left", "150px");
        }],
    );
}

/// The same for a line: `lines_span` measures from the content origin.
#[test]
fn padding_that_moves_a_line_drops_the_kept_walk() {
    const TEXT: &str = r#"<div id="s" style="width:200px;height:50px;overflow:auto"><div id="t" style="width:300px;height:100px"><div id="r" style="box-sizing:border-box;width:280px;white-space:nowrap;font-size:16px;line-height:20px">wwwwwwwwwwww wwwwwwwwwwww</div></div></div>"#;
    kept_matches_fresh(
        "padding moves a line",
        TEXT,
        &[|d| {
            let r = by_id(d, "r");
            d.set_style(r, "padding-left", "150px");
        }],
    );
}

/// And for a border, the other half of the content origin.
#[test]
fn a_border_that_moves_an_inline_block_drops_the_kept_walk() {
    kept_matches_fresh(
        "a border moves an inline-block",
        PADDED,
        &[|d| {
            let r = by_id(d, "r");
            d.set_style(r, "border-left", "150px solid black");
        }],
    );
}

/// A `filter` makes its box the containing block of an absolute box below
/// it (#2719), which is then the container's overflow; taken off, the box's
/// containing block is above the container, and it is not. The absolute box
/// has no insets, so it stays where it is and nothing of it changes: only
/// the filtered box's record can say so. (Taking the filter off is the case
/// the walk alone answers: while it is on, the container's own measure lists
/// the absolute box too, `NodeTree::overflow_absolutes`.)
#[test]
fn a_filter_taken_off_an_absolute_boxs_containing_block_drops_the_kept_walk() {
    const ABS: &str = r#"<div id="s" style="width:200px;height:50px;overflow:auto"><div id="t" style="width:150px;height:100px"><div id="a" style="width:100px;height:10px;filter:blur(0px)"><div style="position:absolute;width:400px;height:10px"></div></div></div></div>"#;
    kept_matches_fresh(
        "a filter taken off an absolute box's containing block",
        ABS,
        &[|d| {
            let a = by_id(d, "a");
            d.set_style(a, "filter", "none");
        }],
    );
}

/// A frame that takes a box out from under the scroller walks nothing: the
/// read before the paint measures the box it left (pushed as a parent a
/// subtree left) and nothing below it, and the paint's consume, which records
/// the bars it drew, measures nothing more. A consume once checked the
/// parents the subtree left as new, after the read had already checked them
/// (`row_shift_damage` in `rinch-bench` walked its 4,000 boxes twice a
/// frame).
#[test]
fn a_removal_walks_once_per_frame() {
    let mut d = kept_doc(ROWS);
    let walks = |d: &RinchDocument| rinch_dom::testing::deep_overflow_walks(&d.tree);
    let measured = |d: &RinchDocument| rinch_dom::testing::deep_overflow_measured(&d.tree);
    assert_eq!(range_of(&d), (600.0, 400.0), "control: the wide row");
    d.tree.consume_paint_dirty();
    let (t, b) = (by_id(&d, "t"), by_id(&d, "b"));
    d.remove_child(t, b);
    d.resolve_layout(800.0, 600.0);
    let (before, measured_before) = (walks(&d), measured(&d));
    assert_eq!(range_of(&d), (600.0, 400.0));
    assert_eq!(walks(&d), before, "the removal walks nothing");
    let after_read = measured(&d);
    assert_eq!(after_read - measured_before, 1, "the box it left");
    d.tree.consume_paint_dirty();
    assert_eq!(range_of(&d), (600.0, 400.0));
    assert_eq!(
        walks(&d),
        before,
        "the consume and the next read walk nothing"
    );
    assert_eq!(measured(&d), after_read, "nor measure anything");
}

/// An absolute box counts nothing, its content included, while it has no
/// area (`deep_walk::walk_out`): emptied, it leaves the range; given an area
/// back, it is in it again. Nothing of its kind changes, so only its size,
/// read again where it is, can say so.
#[test]
fn an_absolute_box_emptied_and_filled_again_matches_a_fresh_document() {
    const ABS: &str = r#"<div id="s" style="width:200px;height:100px;overflow:auto"><div style="position:relative;width:150px;height:40px"><div id="a" style="position:absolute;left:10px;top:20px;width:40px;height:10px"><div style="width:330px;height:20px"></div></div></div></div>"#;
    kept_matches_fresh(
        "an absolute box emptied",
        ABS,
        &[|d| {
            let a = by_id(d, "a");
            d.set_style(a, "width", "0px");
        }],
    );
    kept_matches_fresh(
        "an absolute box emptied, then given an area again",
        ABS,
        &[
            |d| {
                let a = by_id(d, "a");
                d.set_style(a, "height", "0px");
            },
            |d| {
                let a = by_id(d, "a");
                d.set_style(a, "height", "12px");
            },
        ],
    );
}

/// The scroller's own style decides which absolute boxes below it are its
/// overflow: one it stops containing (its `position` or its `filter` taken
/// off) is contained above it, and leaves the range. Nothing below it is
/// pushed: only what the walk read of the container itself can say so.
#[test]
fn the_scroller_no_longer_containing_an_absolute_box_matches_a_fresh_document() {
    const HELD: &str = r#"<div id="s" style="width:200px;height:100px;overflow:auto;position:relative;filter:blur(0px)"><div style="width:150px;height:40px"><div style="position:absolute;left:10px;top:20px;width:400px;height:10px"></div></div></div>"#;
    kept_matches_fresh(
        "the scroller's position and filter taken off",
        HELD,
        &[|d| {
            let s = by_id(d, "s");
            d.set_style(s, "position", "static");
            d.set_style(s, "filter", "none");
        }],
    );
}
