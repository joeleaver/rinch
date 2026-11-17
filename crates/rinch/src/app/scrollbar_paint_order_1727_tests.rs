//! A press on a scroll container's overlay bar goes to the bar only when the
//! bar is the topmost thing painted there (#1727), and the scrollbar search and
//! the wheel's geometric fallback agree with `hit_test` on what reaches the
//! point: the containing-block clip chain, `mask-image`, `visibility: hidden`
//! and `pointer-events: none` (#2684).
//!
//! Every geometry here is set with `position: absolute`/`fixed` from the
//! viewport origin, so the numbers are the CSS. A vertical bar's hit strip is
//! the last 16px of the scroller's width; the presses are 8px in from the right
//! edge.
use super::{find_scroll_container_at_point, find_scrollbar_hit, hit_test};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::perf::Counter;

fn el(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let e = doc.create_element("div");
    doc.set_attribute(e, "style", style);
    doc.append_child(parent, e);
    e
}

/// An `overflow-y: auto` box with content taller than it.
fn scroller(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let s = el(doc, parent, &format!("overflow-y: auto; {style}"));
    el(doc, s, "height: 2000px");
    s
}

fn bar(doc: &RinchDocument, x: f32, y: f32) -> Option<u64> {
    find_scrollbar_hit(&doc.tree, x, y).map(|h| h.node_id)
}

fn layout(doc: &mut RinchDocument) {
    doc.resolve_layout(800.0, 600.0);
}

/// The issue's repro: a later absolute sibling drawn over the scroller's
/// bar takes the press; the bar below it, past the sibling, is still the bar.
#[test]
fn a_later_absolute_sibling_over_the_bar_takes_the_press() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let s = scroller(
        &mut doc,
        body,
        "position: absolute; left: 20px; top: 20px; width: 200px; height: 150px",
    );
    let over = el(
        &mut doc,
        body,
        "position: absolute; left: 200px; top: 20px; width: 40px; height: 60px",
    );
    layout(&mut doc);

    assert_eq!(hit_test(&doc.tree, 212.0, 50.0), Some(over.0));
    assert_eq!(bar(&doc, 212.0, 50.0), None, "the sibling is drawn on top");
    assert_eq!(bar(&doc, 212.0, 120.0), Some(s.0), "below the sibling");
}

/// A `position: fixed` modal backdrop over the page: the scroller's bar under
/// it is covered.
#[test]
fn a_modal_backdrop_over_the_bar_takes_the_press() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let s = scroller(
        &mut doc,
        body,
        "position: absolute; left: 20px; top: 20px; width: 200px; height: 150px",
    );
    let backdrop = el(
        &mut doc,
        body,
        "position: fixed; left: 0; top: 0; width: 800px; height: 600px; z-index: 10; \
         background: rgba(0, 0, 0, 0.5)",
    );
    layout(&mut doc);

    assert_eq!(hit_test(&doc.tree, 212.0, 50.0), Some(backdrop.0));
    assert_eq!(bar(&doc, 212.0, 50.0), None);

    // The same backdrop with `pointer-events: none` lets the press through.
    doc.set_attribute(
        backdrop,
        "style",
        "position: fixed; left: 0; top: 0; width: 800px; height: 600px; z-index: 10; \
         pointer-events: none",
    );
    layout(&mut doc);
    assert_eq!(bar(&doc, 212.0, 50.0), Some(s.0));
}

/// A popover earlier in the tree than the scroller, drawn over it by its
/// `z-index`: tree order is not paint order.
#[test]
fn an_earlier_z_indexed_popover_over_the_bar_takes_the_press() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let popover = el(
        &mut doc,
        body,
        "position: absolute; z-index: 5; left: 200px; top: 20px; width: 40px; height: 60px",
    );
    let s = scroller(
        &mut doc,
        body,
        "position: absolute; left: 20px; top: 20px; width: 200px; height: 150px",
    );
    layout(&mut doc);

    assert_eq!(hit_test(&doc.tree, 212.0, 50.0), Some(popover.0));
    assert_eq!(bar(&doc, 212.0, 50.0), None);
    assert_eq!(bar(&doc, 212.0, 120.0), Some(s.0));
}

/// The shape #2681 widened: a scroller wider than its (non-clipping) parent,
/// under a `position: fixed` header.
#[test]
fn a_fixed_header_over_a_scroller_wider_than_its_parent_takes_the_press() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let parent = el(
        &mut doc,
        body,
        "position: absolute; left: 20px; top: 0; width: 100px; height: 400px",
    );
    let s = scroller(&mut doc, parent, "width: 300px; height: 300px");
    let header = el(
        &mut doc,
        body,
        "position: fixed; left: 0; top: 0; width: 800px; height: 40px; z-index: 1",
    );
    layout(&mut doc);

    // The strip is x in [304, 320].
    assert_eq!(hit_test(&doc.tree, 312.0, 20.0), Some(header.0));
    assert_eq!(bar(&doc, 312.0, 20.0), None, "under the header");
    assert_eq!(bar(&doc, 312.0, 100.0), Some(s.0), "below the header");
}

/// A scroller that is no stacking context draws its bars after its last
/// contained positioned entry (#1717), so over a positioned box earlier in
/// the sequence that it does not contain. That box is what `hit_test` answers
/// at the strip, and the bar is still on top of it: "covered" is decided by
/// paint order, not by whether the hit is inside the scroller.
#[test]
fn a_deferred_bar_over_an_earlier_positioned_box_keeps_the_press() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let cb = el(
        &mut doc,
        body,
        "position: absolute; left: 0; top: 0; width: 400px; height: 400px",
    );
    let under = el(
        &mut doc,
        cb,
        "position: absolute; left: 180px; top: 20px; width: 40px; height: 100px",
    );
    let s = el(
        &mut doc,
        cb,
        "margin: 20px 0 0 20px; width: 200px; height: 150px; overflow-y: auto",
    );
    // Narrow, so nothing contained is under the strip.
    el(
        &mut doc,
        s,
        "position: relative; width: 50px; height: 2000px",
    );
    layout(&mut doc);

    assert_eq!(hit_test(&doc.tree, 212.0, 50.0), Some(under.0));
    assert_eq!(bar(&doc, 212.0, 50.0), Some(s.0));
}

/// An absolute that escapes the scroller (its containing block is outside it)
/// and comes after the scroller's last contained entry is drawn over the bar.
/// One before that entry is drawn under it.
#[test]
fn an_escaping_absolute_covers_the_bar_only_after_the_last_contained_entry() {
    for escaping_first in [false, true] {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let cb = el(
            &mut doc,
            body,
            "position: absolute; left: 0; top: 0; width: 400px; height: 400px",
        );
        let s = el(
            &mut doc,
            cb,
            "margin: 20px 0 0 20px; width: 200px; height: 150px; overflow-y: auto",
        );
        let escaping_style = "position: absolute; left: 200px; top: 20px; width: 40px; \
                              height: 60px";
        let row_style = "position: relative; width: 50px; height: 2000px";
        let escaping = if escaping_first {
            let e = el(&mut doc, s, escaping_style);
            el(&mut doc, s, row_style);
            e
        } else {
            el(&mut doc, s, row_style);
            el(&mut doc, s, escaping_style)
        };
        layout(&mut doc);

        assert_eq!(hit_test(&doc.tree, 212.0, 50.0), Some(escaping.0));
        let expected = escaping_first.then_some(s.0);
        assert_eq!(
            bar(&doc, 212.0, 50.0),
            expected,
            "escaping absolute first in tree order: {escaping_first}"
        );
    }
}

/// A scroller standing past a masked ancestor's border box is masked away,
/// and `hit_test` does not reach it (#1798): neither does a press on its bar
/// nor the wheel's fallback (#2684 item 1).
#[test]
fn a_bar_past_a_masked_ancestors_box_is_not_found() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let m = el(
        &mut doc,
        body,
        "position: absolute; left: 0; top: 0; width: 100px; height: 100px; \
         mask-image: linear-gradient(#000, #000)",
    );
    let s = scroller(
        &mut doc,
        m,
        "position: absolute; left: 150px; top: 0; width: 200px; height: 150px",
    );
    layout(&mut doc);

    assert_ne!(hit_test(&doc.tree, 250.0, 50.0), Some(s.0));
    assert_eq!(bar(&doc, 342.0, 50.0), None);
    assert_eq!(find_scroll_container_at_point(&doc.tree, 250.0, 50.0), None);

    // Control: without the mask the same scroller is found by both.
    doc.set_attribute(
        m,
        "style",
        "position: absolute; left: 0; top: 0; width: 100px; height: 100px",
    );
    layout(&mut doc);
    assert_eq!(bar(&doc, 342.0, 50.0), Some(s.0));
    assert_eq!(
        find_scroll_container_at_point(&doc.tree, 250.0, 50.0),
        Some(s.0)
    );
}

/// An absolute scroller inside a static `overflow: hidden` box that is not
/// its containing block is not clipped by it, as `hit_test` agrees: its bar
/// is pressed and the wheel's fallback finds it (#2684 item 2).
#[test]
fn an_absolute_scroller_escaping_a_static_clip_is_found() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let cb = el(
        &mut doc,
        body,
        "position: absolute; left: 0; top: 0; width: 600px; height: 400px",
    );
    let clip = el(
        &mut doc,
        cb,
        "overflow: hidden; width: 100px; height: 100px",
    );
    let s = scroller(
        &mut doc,
        clip,
        "position: absolute; left: 150px; top: 0; width: 200px; height: 150px",
    );
    layout(&mut doc);

    let content = doc.tree.get(s.0).unwrap().children[0];
    assert_eq!(hit_test(&doc.tree, 250.0, 50.0), Some(content));
    assert_eq!(bar(&doc, 342.0, 50.0), Some(s.0));
    assert_eq!(
        find_scroll_container_at_point(&doc.tree, 250.0, 50.0),
        Some(s.0)
    );
}

/// A `position: fixed` scroller inside an `overflow: hidden; z-index` panel:
/// the panel is not its containing block (#2684 item 2).
#[test]
fn a_fixed_scroller_inside_a_clipping_stacking_panel_is_found() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let panel = el(
        &mut doc,
        body,
        "position: absolute; left: 0; top: 0; z-index: 1; overflow: hidden; width: 100px; \
         height: 100px",
    );
    let s = scroller(
        &mut doc,
        panel,
        "position: fixed; left: 300px; top: 100px; width: 200px; height: 150px",
    );
    layout(&mut doc);

    let content = doc.tree.get(s.0).unwrap().children[0];
    assert_eq!(hit_test(&doc.tree, 400.0, 150.0), Some(content));
    assert_eq!(bar(&doc, 492.0, 150.0), Some(s.0));
    assert_eq!(
        find_scroll_container_at_point(&doc.tree, 400.0, 150.0),
        Some(s.0)
    );
}

/// The wheel's fallback does not find a scroller under a `visibility:
/// hidden` ancestor, which paints nothing and is not hit (#2684 item 3); a
/// scroller that sets `visibility: visible` again is found, and its bar
/// pressed.
#[test]
fn a_hidden_scroller_is_not_found_by_the_wheel_fallback() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let wrap = el(
        &mut doc,
        body,
        "visibility: hidden; position: absolute; left: 0; top: 0",
    );
    let s = scroller(&mut doc, wrap, "width: 200px; height: 150px");
    layout(&mut doc);

    assert_eq!(find_scroll_container_at_point(&doc.tree, 100.0, 50.0), None);
    assert_eq!(bar(&doc, 192.0, 50.0), None);

    doc.set_attribute(
        s,
        "style",
        "overflow-y: auto; width: 200px; height: 150px; visibility: visible",
    );
    layout(&mut doc);
    assert_eq!(
        find_scroll_container_at_point(&doc.tree, 100.0, 50.0),
        Some(s.0)
    );
    assert_eq!(bar(&doc, 192.0, 50.0), Some(s.0));
}

/// A `pointer-events: none` scroller offers no bar to press (#2684 item 3),
/// while a descendant that sets `pointer-events: auto` again is still hit.
#[test]
fn a_pointer_events_none_scrollers_bar_is_not_pressed() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let s = el(
        &mut doc,
        body,
        "position: absolute; left: 0; top: 0; width: 200px; height: 150px; \
         overflow-y: auto; pointer-events: none",
    );
    let inner = el(&mut doc, s, "height: 2000px; pointer-events: auto");
    layout(&mut doc);

    assert_eq!(hit_test(&doc.tree, 100.0, 50.0), Some(inner.0));
    assert_eq!(bar(&doc, 192.0, 50.0), None);
    assert_eq!(find_scroll_container_at_point(&doc.tree, 100.0, 50.0), None);
}

/// A bordered, padded parent that clips on y only: the bar of a scroller
/// overflowing it is pressed past its box on the open x axis, inside its
/// padding box on y, and not in its bottom border or below it (#2684 item 5).
#[test]
fn a_bar_under_a_padded_y_clip_is_pressed_only_inside_the_padding_box() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let parent = el(
        &mut doc,
        body,
        "position: absolute; left: 0; top: 0; width: 100px; height: 100px; \
         border: 5px solid black; padding: 10px; overflow-x: visible; overflow-y: clip",
    );
    // Content box at (15, 15); padding box (5, 5)-(125, 125); border box
    // (0, 0)-(130, 130). The scroller covers (15, 15)-(315, 315), so its
    // strip is x in [299, 315].
    let s = scroller(&mut doc, parent, "width: 300px; height: 300px");
    layout(&mut doc);

    assert_eq!(bar(&doc, 307.0, 50.0), Some(s.0), "past the box on open x");
    assert_eq!(bar(&doc, 307.0, 122.0), Some(s.0), "inside the padding box");
    assert_eq!(bar(&doc, 307.0, 128.0), None, "in the bottom border");
    assert_eq!(bar(&doc, 307.0, 200.0), None, "below the box on clipped y");
}

/// The search is the hit test's own walk, so it costs what `hit_test` costs
/// at the point: a press beside a long list whose rows each hold an absolute
/// box visits no row (#2684 item 4).
#[test]
fn the_bar_search_beside_a_long_list_visits_what_the_hit_test_visits() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let list = el(
        &mut doc,
        body,
        "position: absolute; left: 0; top: 0; width: 200px",
    );
    for _ in 0..1500 {
        let row = el(&mut doc, list, "position: relative; height: 20px");
        el(
            &mut doc,
            row,
            "position: absolute; left: 0; top: 0; width: 10px; height: 10px",
        );
    }
    let s = scroller(
        &mut doc,
        body,
        "position: absolute; left: 400px; top: 20px; width: 200px; height: 150px",
    );
    layout(&mut doc);

    let before = doc.tree.perf.get(Counter::HitTestNodesVisited);
    assert_eq!(hit_test(&doc.tree, 700.0, 300.0), Some(body.0));
    let hit_visits = doc.tree.perf.get(Counter::HitTestNodesVisited) - before;

    let before = doc.tree.perf.get(Counter::ScrollProbeNodesVisited);
    assert_eq!(bar(&doc, 700.0, 300.0), None);
    let probe_visits = doc.tree.perf.get(Counter::ScrollProbeNodesVisited) - before;
    assert!(probe_visits > 0, "the probe counts its visits");
    assert_eq!(probe_visits, hit_visits);
    assert!(probe_visits < 10, "{probe_visits} nodes visited");

    // And the scroller's bar is still found there.
    assert_eq!(bar(&doc, 592.0, 50.0), Some(s.0));
}

/// The sequences the search walks are cached (`HitCache::order`, with its
/// `bars` flag): a box inserted over the bar after a press, and removed again,
/// is seen by the next press.
#[test]
fn a_press_after_an_insert_and_a_remove_sees_the_change() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let s = scroller(
        &mut doc,
        body,
        "position: absolute; left: 20px; top: 20px; width: 200px; height: 150px",
    );
    layout(&mut doc);
    assert_eq!(bar(&doc, 212.0, 50.0), Some(s.0), "cached press");

    let over = el(
        &mut doc,
        body,
        "position: absolute; left: 200px; top: 20px; width: 40px; height: 60px",
    );
    layout(&mut doc);
    assert_eq!(bar(&doc, 212.0, 50.0), None, "after the insert");

    doc.remove_child(body, over);
    layout(&mut doc);
    assert_eq!(bar(&doc, 212.0, 50.0), Some(s.0), "after the remove");
}

/// A deferred inner scroller's bar is placed by its sequence entry's origin,
/// which an outer scroll moves (#911 keeps extents across a scroll but drops
/// every sequence): after the outer scroller scrolls, the bar is found where
/// it now is, and not where the cached sequence put it.
#[test]
fn a_deferred_inner_bar_moves_with_an_outer_scroll() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let outer = el(
        &mut doc,
        body,
        "position: absolute; left: 0; top: 0; width: 300px; height: 300px; overflow-y: auto",
    );
    el(&mut doc, outer, "height: 100px");
    // Not a stacking context, with a contained positioned row: its bars are
    // deferred to the body's sequence (#1717).
    let inner = el(
        &mut doc,
        outer,
        "width: 200px; height: 150px; overflow-y: auto",
    );
    el(
        &mut doc,
        inner,
        "position: relative; width: 50px; height: 2000px",
    );
    el(&mut doc, outer, "height: 1000px");
    layout(&mut doc);

    // The inner box is y 100..250; its strip is x in [184, 200].
    assert_eq!(bar(&doc, 192.0, 240.0), Some(inner.0), "before the scroll");

    doc.set_scroll_top(outer, 80.0);
    // Now y 20..170.
    assert_ne!(bar(&doc, 192.0, 240.0), Some(inner.0), "the old place");
    assert_eq!(bar(&doc, 192.0, 160.0), Some(inner.0), "the new place");
}

/// A deferred bar is drawn inside its root's clip bracket: a scroller wider
/// than an `overflow: hidden; z-index: 0` root offers no bar past the root's
/// clip, where paint draws none.
#[test]
fn a_deferred_bar_past_its_roots_clip_is_not_pressed() {
    for clips in [true, false] {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let overflow = if clips { "hidden" } else { "visible" };
        let root = el(
            &mut doc,
            body,
            &format!(
                "position: absolute; left: 0; top: 0; width: 200px; height: 200px; \
                 z-index: 0; overflow: {overflow}"
            ),
        );
        let s = el(
            &mut doc,
            root,
            "width: 300px; height: 150px; overflow-y: auto",
        );
        el(
            &mut doc,
            s,
            "position: relative; width: 50px; height: 2000px",
        );
        layout(&mut doc);

        // The strip is x in [284, 300], past the root's x 0..200.
        let expected = (!clips).then_some(s.0);
        assert_eq!(bar(&doc, 292.0, 50.0), expected, "root clips: {clips}");
    }
}
