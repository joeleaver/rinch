//! Hand-built documents in the shapes the prune and the cache have to get
//! right, each probed against the oracle.

use super::super::hit_test;
use super::{assert_agree, reference_hit_test};
use rinch_core::dom::{DomContent, DomScroll, DomTree, NodeId};
use rinch_dom::RinchDocument;

/// A long list in a scroller — the shape the prune exists for — scrolled to
/// the middle, with positioned rows and a nested clipper so the clip chain and
/// the `PositionedAuto` entries are exercised too.
#[test]
fn a_scrolled_list_hits_identically() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let scroller = doc.create_element("div");
    doc.set_attribute(
        scroller,
        "style",
        "height: 400px; overflow-y: auto; margin: 10px",
    );
    doc.append_child(body, scroller);
    for i in 0..300 {
        let row = doc.create_element("div");
        let pos = if i % 7 == 0 {
            "position: relative; "
        } else {
            ""
        };
        doc.set_attribute(
            row,
            "style",
            &format!("{pos}height: 20px; margin-left: {}px", i % 5),
        );
        let t = doc.create_text(&format!("row {i}"));
        doc.append_child(row, t);
        doc.append_child(scroller, row);
        if i % 11 == 0 {
            let over = doc.create_element("div");
            doc.set_attribute(
                over,
                "style",
                "position: relative; left: -30px; top: 35px; width: 50px; height: 50px",
            );
            doc.append_child(row, over);
        }
    }
    doc.resolve_layout(800.0, 600.0);
    for top in [0.0, 1234.5, 5000.0] {
        doc.set_scroll_top(scroller, top);
        assert_agree(&doc, &format!("scrolled to {top}"));
    }
}

/// Rows that are stacking contexts (#1719), which the prune now skips on their
/// flow extent when nothing under them is hoisted — and must not when
/// something is: every fifth row holds an absolute box that escapes a clipping
/// child to hang 30px below the row (the row is its containing block), every
/// seventh a `position: relative; z-index: 1` badge, every ninth a rotated
/// leaf. Probed scrolled to three places against the unpruned walk.
///
/// Kills: the hoisting bit dropped (every stacking context pruned on its flow
/// extent: the escaping absolutes go unhit), and a clipping box answering
/// "hoists nothing" without looking (the absolute under the clipper).
#[test]
fn stacking_context_rows_hit_identically() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let scroller = doc.create_element("div");
    doc.set_attribute(
        scroller,
        "style",
        "height: 400px; overflow-y: auto; margin: 10px; position: relative; z-index: 0",
    );
    doc.append_child(body, scroller);
    let div = |doc: &mut RinchDocument, parent: NodeId, style: &str| {
        let d = doc.create_element("div");
        doc.set_attribute(d, "style", style);
        doc.append_child(parent, d);
        d
    };
    for i in 0..300 {
        let row = div(
            &mut doc,
            scroller,
            &format!(
                "position: relative; z-index: 0; width: 90px; height: 20px; margin-left: {}px",
                i % 5
            ),
        );
        div(&mut doc, row, "width: 40px; height: 10px");
        if i % 5 == 0 {
            let clip = div(&mut doc, row, "overflow: hidden; width: 60px; height: 10px");
            div(
                &mut doc,
                clip,
                "position: absolute; left: 100px; top: 30px; width: 30px; height: 15px",
            );
        }
        if i % 7 == 0 {
            div(
                &mut doc,
                row,
                "position: relative; z-index: 1; left: 200px; top: -5px; width: 20px; height: 20px",
            );
        }
        if i % 9 == 0 {
            div(
                &mut doc,
                row,
                "transform: rotate(30deg); width: 30px; height: 6px; margin-left: 300px",
            );
        }
    }
    doc.resolve_layout(800.0, 600.0);
    for top in [0.0, 1234.5, 5000.0] {
        doc.set_scroll_top(scroller, top);
        assert_agree(&doc, &format!("scrolled to {top}"));
    }
    // Positive control: an escaping absolute is hit where it hangs, below its
    // row (row 250 at 5000: content y 0, the box at 30..45, x 100..130 plus
    // the scroller's 10px margin; the rows are 90px wide, so none covers it).
    let hit = hit_test(&doc.tree, 125.0, 52.0).expect("a hit");
    let style = doc.tree.nodes[hit]
        .attributes
        .get("style")
        .cloned()
        .unwrap_or_default();
    assert!(style.contains("absolute"), "hit {hit}: {style}");
}

/// Scrollers inside scrollers, each holding `sticky`, `fixed`, `absolute`
/// and `relative` children, scrolled one at a time with the cache warm
/// between — the shape #911's partial invalidation has to get right. A scroll
/// drops every stacking sequence (the positioned rows are entries of the
/// body's, at offsets that moved), and keeps every extent unless the scrolled
/// box's own extent reads its scroll offset.
///
/// A real scroller clips, so its extent is its own box and no extent anywhere
/// moves when it scrolls. The other half is reached only by a box that does
/// **not** clip and still has a scroll offset — `set_scroll_top` gives one
/// here (the wheel can too, on an inline `overflow: auto` span, which never
/// clips) — so `wrapper` is that box; its scroll must drop its own extent
/// and `plain`'s, its parent, or `plain` answers from before the scroll. (It
/// drops every extent: tracking the chain cost the cold pointer move 5%.)
///
/// Every probe grid runs on a warm cache: the previous grid filled it, and
/// the `resolve_layout` between some rounds takes the paint-only path, which
/// keeps it.
#[test]
fn nested_scrollers_scrolled_one_at_a_time_hit_identically() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let mut scrollers = Vec::new();
    // Index 3: not a scroll container — no `overflow` — but scrolled anyway.
    // It sits in a plain div of its own so that its extent is folded into
    // one that is kept (`<body>` is a stacking root and keeps none).
    let plain = doc.create_element("div");
    // Both are shorter than what they hold, so the content overflows their
    // own boxes: a box's extent always includes its own box, and content
    // that only ever moved inside it could not tell a stale extent apart.
    doc.set_attribute(plain, "style", "height: 150px; padding-left: 4px");
    doc.append_child(body, plain);
    let wrapper = doc.create_element("div");
    doc.set_attribute(
        wrapper,
        "style",
        "height: 130px; padding-top: 6px; margin-left: 9px",
    );
    doc.append_child(plain, wrapper);
    let mut parent = wrapper;
    // Three levels: body > outer > middle > inner, each a scroller of rows
    // and positioned children, the next level nested in its third row.
    for level in 0..3 {
        let sc = doc.create_element("div");
        let (top, h) = (10 + level * 7, 420 - level * 90);
        doc.set_attribute(
            sc,
            "style",
            &format!(
                "height: {h}px; width: {}px; overflow-y: auto; overflow-x: auto; \
                 margin-top: {top}px; margin-left: {}px",
                700 - level * 120,
                level * 13
            ),
        );
        doc.append_child(parent, sc);
        scrollers.push(sc);
        let mut next_parent = sc;
        for i in 0..40 {
            let row = doc.create_element("div");
            let style = match i % 9 {
                0 => "position: relative; height: 18px; width: 900px".to_string(),
                3 => "position: sticky; top: 0px; height: 16px; background: red".to_string(),
                5 => "height: 22px; margin-left: -15px".to_string(),
                _ => format!("height: {}px; margin-left: {}px", 14 + i % 4, i % 6),
            };
            doc.set_attribute(row, "style", &style);
            doc.append_child(sc, row);
            if i % 7 == 2 {
                let abs = doc.create_element("div");
                doc.set_attribute(
                    abs,
                    "style",
                    "position: absolute; left: 40px; top: 30px; width: 60px; height: 25px",
                );
                doc.append_child(row, abs);
            }
            if i % 13 == 4 {
                let fixed = doc.create_element("div");
                doc.set_attribute(
                    fixed,
                    "style",
                    &format!(
                        "position: fixed; left: {}px; top: {}px; width: 30px; height: 30px",
                        500 + level * 40,
                        20 + i * 7
                    ),
                );
                doc.append_child(row, fixed);
            }
            if i % 5 == 1 {
                // An in-flow child that overflows its row to the left and
                // down, so the row's extent is not its own box.
                let over = doc.create_element("div");
                doc.set_attribute(
                    over,
                    "style",
                    "margin-left: -25px; margin-top: 4px; width: 45px; height: 40px",
                );
                doc.append_child(row, over);
            }
            if i == 2 {
                next_parent = row;
            }
        }
        parent = next_parent;
    }
    scrollers.push(wrapper);
    doc.resolve_layout(800.0, 600.0);
    assert_agree(&doc, "as laid out");

    // Deepest first, then outwards and back in, off every fixed point.
    let steps: [(usize, f64, f64); 12] = [
        (2, 37.0, 0.0),
        (1, 55.5, 11.0),
        (3, 17.0, 5.0),
        (0, 91.0, 0.0),
        (2, 140.0, 23.0),
        (3, 90.0, 0.0),
        (0, 12.0, 7.0),
        (1, 0.0, 0.0),
        (2, 3.0, 0.0),
        (3, 0.0, 31.0),
        (1, 210.0, 40.0),
        (0, 260.0, 0.0),
    ];
    let mut hits_seen = 0;
    for (i, &(which, top, left)) in steps.iter().enumerate() {
        doc.set_scroll_top(scrollers[which], top);
        doc.set_scroll_left(scrollers[which], left);
        if i % 3 == 2 {
            doc.resolve_layout(800.0, 600.0);
        }
        assert_agree(
            &doc,
            &format!("step {i}: scroller {which} to ({left}, {top})"),
        );
        hits_seen += (0..600)
            .step_by(23)
            .filter(|&v| hit_test(&doc.tree, v as f32 * 1.3, v as f32).is_some())
            .count();
        if which == 3 {
            // Rebuild the memo while `wrapper` is scrolled, so `plain`'s
            // extent is the *scrolled* one — too short at the bottom for the
            // next scroll back up. Scrolling further down only ever leaves a
            // stale extent too large, and a too-large extent merely fails to
            // prune: this is the direction that hides a hit.
            doc.set_attribute(plain, "data-step", &i.to_string());
            assert_agree(&doc, &format!("step {i}: rebuilt while scrolled"));
        }
    }
    assert!(hits_seen > 50, "positive control: hits seen {hits_seen}");
}

/// A viewport resize moves percentage-sized boxes with no restyle and no
/// `DomDocument` write — only the layout pass sees it. Since #911 that pass
/// keeps the hit memo on its paint-only path, so this pins that the path which
/// does move boxes still drops it: the probes before the resize fill the cache
/// with extents at the old width.
#[test]
fn a_resize_that_moves_percentage_boxes_drops_the_memo() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    for i in 0..12 {
        let row = doc.create_element("div");
        doc.set_attribute(
            row,
            "style",
            &format!(
                "width: {}%; height: 30px; margin-left: {}%",
                20 + i * 5,
                i * 3
            ),
        );
        doc.append_child(body, row);
        let over = doc.create_element("div");
        // Overflows the row to the right by a percentage of the row, so the
        // row's extent — not just its box — moves with the viewport.
        doc.set_attribute(over, "style", "width: 150%; height: 12px");
        doc.append_child(row, over);
    }
    // Narrow first, then grow: the extents cached at the narrow width are
    // too *small* for the wide layout, which is the direction a stale memo
    // prunes a real hit away in. (Shrinking leaves them too large — merely
    // conservative — and passes with the invalidation removed.)
    doc.resolve_layout(530.0, 600.0);
    let before = reference_hit_test(&doc.tree, 700.0, 20.0);
    assert_agree(&doc, "at 530");
    doc.resolve_layout(800.0, 600.0);
    assert_agree(&doc, "at 800");
    // Positive control: the resize did move a box across a probed point.
    assert_ne!(
        reference_hit_test(&doc.tree, 700.0, 20.0),
        before,
        "the resize moved nothing under (700, 20)"
    );
}

/// A stacking-context row, cache warm with "hoists nothing" (#1719), then a
/// child restyled into an absolute box hanging far below the row: the restyle
/// drops the memo, and the box is hit where it hangs (review of #1720).
#[test]
fn a_stacking_row_that_starts_hoisting_is_hit() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let row = doc.create_element("div");
    doc.set_attribute(
        row,
        "style",
        "position: relative; z-index: 0; width: 100px; height: 20px; margin-top: 10px",
    );
    doc.append_child(body, row);
    let kid = doc.create_element("div");
    doc.set_attribute(kid, "style", "width: 50px; height: 10px");
    doc.append_child(row, kid);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        hit_test(&doc.tree, 300.0, 300.0).map(|h| h == kid.0),
        Some(false)
    );
    doc.set_attribute(
        kid,
        "style",
        "position: absolute; left: 280px; top: 280px; width: 50px; height: 50px",
    );
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(hit_test(&doc.tree, 300.0, 300.0), Some(kid.0));
    assert_agree(&doc, "after hoisting");
}
