//! More shapes of an absolute box resolved against a non-parent containing
//! block (#386), from the review of PR #1409: the ones
//! `abs_containing_block_tests` sat on a fixed point for — two scrollers
//! between, a horizontal scroll, a `calc()` under a parent with an indefinite
//! axis — and the shapes probed and found right. Each fixture's doc names the
//! mutant it kills where it kills one.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

fn build(html: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let wrap = doc.create_element("div");
    doc.set_inner_html(wrap, html);
    doc.append_child(body, wrap);
    doc.resolve_layout(800.0, 600.0);
    doc
}

fn one(doc: &RinchDocument, selector: &str) -> usize {
    let found = query_selector(&doc.tree, selector);
    assert_eq!(found.len(), 1, "{selector} names exactly one node");
    found[0]
}

/// `m` relative to `cb`'s border box, with its size.
fn rect(doc: &RinchDocument, m: &str, cb: &str) -> [f32; 4] {
    let id = one(doc, m);
    let cb = one(doc, cb);
    let (x, y) = rinch_dom::paint::compute_absolute_position(&doc.tree, id, 1.0);
    let (cx, cy) = rinch_dom::paint::compute_absolute_position(&doc.tree, cb, 1.0);
    let l = doc.tree.get(id).unwrap().layout;
    [(x - cx) as f32, (y - cy) as f32, l.width, l.height]
}

fn size(doc: &RinchDocument, m: &str) -> (f32, f32) {
    let l = doc.tree.get(one(doc, m)).unwrap().layout;
    (l.width, l.height)
}

#[track_caller]
fn close(got: [f32; 4], want: [f32; 4], what: &str) {
    let ok = got.iter().zip(want).all(|(g, w)| (g - w).abs() < 0.01);
    assert!(ok, "{what}: got {got:?}, want {want:?}");
}

const CB: &str = "position: relative; width: 400px; height: 300px; margin: 13px 0 0 17px;";
const MID: &str = "width: 300px; height: 200px; margin: 0 0 0 30px;";

/// R1: the containing block is an AUTO-sized atomic inline (its size comes
/// from the detached compute, which may run at several widths).
#[test]
fn r1_auto_sized_inline_block_containing_block() {
    let doc = build(
        r#"<div style="width: 400px; font-size: 16px; line-height: 20px">
<span data-cb style="display: inline-block; position: relative; padding: 10px; border: 2px solid black"><div style="width: 120px; height: 40px"></div><div style="margin-left: 30px; width: 50px; height: 30px"><div data-a style="position: absolute; inset: 0"></div></div></span></div>"#,
    );
    let (cw, ch) = size(&doc, "[data-cb]");
    assert_eq!((cw, ch), (144.0, 94.0), "the inline-block itself");
    close(
        rect(&doc, "[data-a]", "[data-cb]"),
        [2.0, 2.0, 140.0, 90.0],
        "inset: 0 in an auto inline-block",
    );
}

/// R2: horizontal scroll of a scroller between, and two nested scrollers.
#[test]
fn r2_horizontal_and_nested_scrollers_between() {
    let mut doc = build(&format!(
        r#"<div data-cb style="{CB}"><div style="height: 20px"></div>
<div data-outer style="{MID} overflow: auto; height: 100px;">
  <div style="width: 900px; height: 40px"></div>
  <div data-inner style="overflow: auto; width: 200px; height: 60px; margin-left: 11px">
    <div style="width: 700px; height: 400px"></div>
    <div data-a style="position: absolute; left: 5px; top: 6px; width: 20px; height: 20px"></div>
    <div data-s style="position: absolute; width: 20px; height: 20px"></div>
  </div>
  <div style="height: 300px"></div>
</div></div>"#
    ));
    let a0 = rect(&doc, "[data-a]", "[data-cb]");
    let s0 = rect(&doc, "[data-s]", "[data-cb]");
    close(a0, [5.0, 6.0, 20.0, 20.0], "unscrolled inset box");
    let outer = one(&doc, "[data-outer]");
    let inner = one(&doc, "[data-inner]");
    doc.set_scroll_left(NodeId(inner), 37.0);
    doc.set_scroll_top(NodeId(inner), 23.0);
    close(rect(&doc, "[data-a]", "[data-cb]"), a0, "inner scrolled");
    close(
        rect(&doc, "[data-s]", "[data-cb]"),
        s0,
        "inner scrolled (static)",
    );
    doc.set_scroll_left(NodeId(outer), 51.0);
    doc.set_scroll_top(NodeId(outer), 17.0);
    assert_eq!(doc.tree.get(outer).unwrap().scroll_offset, (51.0, 17.0));
    assert_eq!(doc.tree.get(inner).unwrap().scroll_offset, (37.0, 23.0));
    close(rect(&doc, "[data-a]", "[data-cb]"), a0, "both scrolled");
    close(
        rect(&doc, "[data-s]", "[data-cb]"),
        s0,
        "both scrolled (static)",
    );
    doc.resolve_layout(800.0, 640.0);
    close(rect(&doc, "[data-a]", "[data-cb]"), a0, "laid out");
    close(rect(&doc, "[data-s]", "[data-cb]"), s0, "laid out (static)");
}

/// R5: a box hoisted out of a plain (non-positioned) inline span.
#[test]
fn r5_box_hoisted_out_of_a_plain_span() {
    let doc = build(&format!(
        r#"<div data-cb style="{CB}"><div style="height: 20px"></div>
<div style="{MID} font-size: 16px; line-height: 20px; padding: 7px 0 0 9px">lead <span>text<div data-a style="position: absolute; inset: 0"></div><div data-b style="position: absolute; right: 10px; bottom: 20px; width: 50px; height: 30px"></div></span></div></div>"#
    ));
    close(
        rect(&doc, "[data-a]", "[data-cb]"),
        [0.0, 0.0, 400.0, 300.0],
        "hoisted inset: 0",
    );
    close(
        rect(&doc, "[data-b]", "[data-cb]"),
        [340.0, 250.0, 50.0, 30.0],
        "hoisted right/bottom",
    );
}

/// R5b: the same under a SPLIT inline (#513): the span also holds a block.
#[test]
fn r5b_box_under_a_split_inline() {
    let doc = build(&format!(
        r#"<div data-cb style="{CB}"><div style="height: 20px"></div>
<div style="{MID} font-size: 16px; line-height: 20px;">lead <span>text<div style="height: 10px"></div>more<div data-a style="position: absolute; inset: 0"></div></span></div></div>"#
    ));
    close(
        rect(&doc, "[data-a]", "[data-cb]"),
        [0.0, 0.0, 400.0, 300.0],
        "inset: 0 under a split inline",
    );
}

/// R6: a `position: fixed` and a `position: sticky` grandparent.
#[test]
fn r6_fixed_and_sticky_grandparents() {
    let doc = build(&format!(
        r#"<div data-cb style="position: fixed; left: 40px; top: 50px; width: 400px; height: 300px"><div style="height: 20px"></div><div style="{MID}"><div data-a style="position: absolute; inset: 0"></div></div></div>"#
    ));
    close(
        rect(&doc, "[data-a]", "[data-cb]"),
        [0.0, 0.0, 400.0, 300.0],
        "fixed grandparent",
    );
    let doc = build(&format!(
        r#"<div data-cb style="position: sticky; top: 0; width: 400px; height: 300px"><div style="height: 20px"></div><div style="{MID}"><div data-a style="position: absolute; inset: 0"></div></div></div>"#
    ));
    close(
        rect(&doc, "[data-a]", "[data-cb]"),
        [0.0, 0.0, 400.0, 300.0],
        "sticky grandparent",
    );
}

/// R8: `<body>` positioned: it is an ancestor containing block like any other.
#[test]
fn r8_positioned_body() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(
        body,
        "style",
        "position: relative; padding: 10px; margin: 0",
    );
    let wrap = doc.create_element("div");
    doc.set_inner_html(
        wrap,
        r#"<div style="height: 20px"></div><div style="width: 300px; height: 200px; margin-left: 30px"><div data-a style="position: absolute; left: 3px; top: 4px; width: 50%; height: 10px"></div></div>"#,
    );
    doc.append_child(body, wrap);
    doc.resolve_layout(800.0, 600.0);
    let id = one(&doc, "[data-a]");
    let (x, y) = rinch_dom::paint::compute_absolute_position(&doc.tree, id, 1.0);
    let l = doc.tree.get(id).unwrap().layout;
    assert_eq!(
        (x as f32, y as f32, l.width, l.height),
        (3.0, 4.0, 400.0, 10.0)
    );
}

/// R9: a subtree shown after being `display: none`.
#[test]
fn r9_a_subtree_shown_later() {
    let mut doc = build(&format!(
        r#"<div data-cb style="{CB}"><div style="height: 20px"></div>
<div data-mid style="{MID} display: none"><div data-a style="position: absolute; inset: 0"><div data-k style="width: 50%; height: 50%"></div></div></div></div>"#
    ));
    let mid = one(&doc, "[data-mid]");
    doc.set_attribute(NodeId(mid), "style", MID);
    doc.resolve_layout(800.0, 600.0);
    close(
        rect(&doc, "[data-a]", "[data-cb]"),
        [0.0, 0.0, 400.0, 300.0],
        "shown",
    );
    assert_eq!(size(&doc, "[data-k]"), (200.0, 150.0));
}

/// R12: how deep can non-parent containing blocks nest before the first
/// layout is wrong (the fixpoint cap)?
fn nested(depth: usize) -> RinchDocument {
    let mut html = format!(r#"<div data-cb style="{CB}">"#);
    for i in 0..depth {
        html.push_str(&format!(
            r#"<div style="width: 10px; height: 10px"><div data-l="{i}" style="position: absolute; left: 5px; right: 5px; top: 0; height: 100%">"#
        ));
    }
    for _ in 0..depth {
        html.push_str("</div></div>");
    }
    html.push_str("</div>");
    build(&html)
}

#[test]
fn r12_nesting_depth_and_the_cap() {
    for depth in [3usize, 7, 8, 9, 12] {
        let mut doc = nested(depth);
        let want = 400.0 - 10.0 * depth as f32;
        let got = size(&doc, &format!("[data-l=\"{}\"]", depth - 1)).0;
        let passes = doc.tree.perf.frame();
        eprintln!(
            "depth {depth}: innermost width {got} (want {want}); first-frame counters: {passes:?}"
        );
        doc.resolve_layout(800.0, 640.0);
        let got2 = size(&doc, &format!("[data-l=\"{}\"]", depth - 1)).0;
        eprintln!("depth {depth}: after a second layout {got2}");
        if depth <= 7 {
            assert!((got - want).abs() < 1.0, "depth {depth} first layout");
        }
    }
}

/// R13: the `display: contents; position: absolute` element WITH an ICB
/// containing block, and a scroll, and a relayout.
#[test]
fn r13_contents_absolute_everywhere() {
    let mut doc = build(
        r#"<div data-s style="overflow: auto; width: 300px; height: 100px; margin: 20px 0 0 30px"><div style="height: 40px"></div>
<div data-w style="display: contents; position: absolute; right: 10px; bottom: 20px; width: 50%; padding: 10%"><div data-k style="width: 20px; height: 10px"></div></div><div style="height: 400px"></div></div>"#,
    );
    let k0 = rect(&doc, "[data-k]", "[data-s]");
    close(k0, [0.0, 40.0, 20.0, 10.0], "in flow");
    let s = one(&doc, "[data-s]");
    doc.set_scroll_top(NodeId(s), 25.0);
    let w = doc.tree.get(one(&doc, "[data-w]")).unwrap().layout;
    assert_eq!((w.x, w.y, w.width, w.height), (0.0, 0.0, 0.0, 0.0));
    close(
        rect(&doc, "[data-k]", "[data-s]"),
        [0.0, 15.0, 20.0, 10.0],
        "rides the scroller like any in-flow box",
    );
    doc.resolve_layout(800.0, 640.0);
    let w = doc.tree.get(one(&doc, "[data-w]")).unwrap().layout;
    assert_eq!((w.x, w.y, w.width, w.height), (0.0, 0.0, 0.0, 0.0));
    close(
        rect(&doc, "[data-k]", "[data-s]"),
        [0.0, 15.0, 20.0, 10.0],
        "after a layout",
    );
}

/// R14: an inset write through `set_style` (the #280 fast path) on a box with
/// PAIRED insets and an auto size — its size depends on the inset. The fast
/// path bakes the size for the new insets itself, so the write costs one
/// compute like any other inset write (it was two: the pass after the compute
/// found the old bake and ran the compute again).
#[test]
fn r14_fast_path_paired_insets() {
    let mut doc = build(&format!(
        r#"<div data-cb style="{CB}"><div style="height: 20px"></div>
<div style="{MID}"><div data-a style="position: absolute; left: 0; right: 0; top: 0; height: 10px"><div data-k style="width: 100%; height: 5px"></div></div></div></div>"#
    ));
    let a = one(&doc, "[data-a]");
    assert_eq!(size(&doc, "[data-a]").0, 400.0);
    use rinch_dom::perf::Counter;
    let counters = |doc: &RinchDocument| {
        (
            doc.tree.perf.get(Counter::TaffyRootComputes),
            doc.tree.perf.get(Counter::AbsContainingBlockPasses),
        )
    };
    let before = counters(&doc);
    doc.set_style(NodeId(a), "right", "100px");
    doc.resolve_layout(800.0, 600.0);
    let after = counters(&doc);
    assert_eq!(size(&doc, "[data-a]").0, 300.0);
    assert_eq!(size(&doc, "[data-k]").0, 300.0);
    assert_eq!(
        (after.0 - before.0, after.1 - before.1),
        (1, 0),
        "a `right` write on a left/right box: root computes, abs passes"
    );
    doc.set_style(NodeId(a), "top", "7px");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        counters(&doc).0 - after.0,
        1,
        "a `top` write, which the size does not depend on"
    );
    assert_eq!(size(&doc, "[data-a]").0, 300.0);
}

/// R15: the containing block changes size through a TRANSITION-free inline
/// write on an ancestor of the containing block (percentage chain).
#[test]
fn r15_percentage_containing_block_follows_the_viewport() {
    let mut doc = build(
        r#"<div data-cb style="position: relative; width: 50%; height: 100px"><div style="width: 10px; height: 10px; margin-left: 30px"><div data-a style="position: absolute; inset: 0"><div data-k style="width: 50%; height: 5px"></div></div></div></div>"#,
    );
    assert_eq!(size(&doc, "[data-a]"), (400.0, 100.0));
    doc.resolve_layout(1000.0, 600.0);
    assert_eq!(size(&doc, "[data-a]"), (500.0, 100.0));
    assert_eq!(size(&doc, "[data-k]").0, 250.0);
}

/// R16: removing the box, and removing the box between (the box becomes a
/// direct child of its containing block by a MOVE, no restyle).
#[test]
fn r16_moved_to_be_a_direct_child() {
    let mut doc = build(&format!(
        r#"<div data-cb style="{CB}"><div style="height: 20px"></div>
<div data-mid style="{MID}"><div data-a style="position: absolute; left: 0; top: 0; width: 50%; height: 50%; padding: 10%"></div></div></div>"#
    ));
    assert_eq!(size(&doc, "[data-a]"), (200.0, 150.0));
    let a = one(&doc, "[data-a]");
    let cb = one(&doc, "[data-cb]");
    doc.append_child(NodeId(cb), NodeId(a));
    doc.resolve_layout(800.0, 640.0);
    assert_eq!(size(&doc, "[data-a]"), (200.0, 150.0));
    close(
        rect(&doc, "[data-a]", "[data-cb]"),
        [0.0, 0.0, 200.0, 150.0],
        "direct child",
    );
}

/// R17: a real 0x0 box in an ICB chain under a scroller, the static axis.
#[test]
fn r17_icb_static_axis_under_two_scrollers() {
    let mut doc = build(
        r#"<div data-o style="overflow: auto; width: 300px; height: 120px; margin: 20px 0 0 30px"><div style="height: 30px"></div>
<div data-i style="overflow: auto; width: 200px; height: 60px"><div style="height: 25px"></div><div data-a style="position: absolute; width: 20px; height: 20px"></div><div style="height: 400px"></div></div><div style="height: 400px"></div></div>"#,
    );
    let id = one(&doc, "[data-a]");
    let pos = |doc: &RinchDocument| rinch_dom::paint::compute_absolute_position(&doc.tree, id, 1.0);
    let p0 = pos(&doc);
    doc.set_scroll_top(NodeId(one(&doc, "[data-i]")), 13.0);
    assert_eq!(pos(&doc), p0, "inner");
    doc.set_scroll_top(NodeId(one(&doc, "[data-o]")), 9.0);
    assert_eq!(pos(&doc), p0, "outer");
    doc.resolve_layout(800.0, 640.0);
    assert_eq!(pos(&doc), p0, "laid out");
    let body = doc.body();
    doc.set_attribute(body, "style", "overflow: auto");
}

/// K1 (kills M10): a `calc()` min-height on a box whose Taffy PARENT is
/// content-sized (indefinite) but whose containing block is definite.
#[test]
#[ignore = "fails at the PR head AND on main: calc() is never resolved inside an atomic inline (issue draft review-1409-1)"]
fn k1_calc_under_an_indefinite_parent() {
    let doc = build(&format!(
        r#"<div data-cb style="{CB}"><div style="height: 20px"></div>
<div style="margin-left: 30px; display: inline-block"><div style="width: 40px; height: 20px"></div><div data-a style="position: absolute; left: 0; top: 0; width: 10px; min-height: calc(50% - 20px); min-width: calc(25% + 3px); padding-top: calc(5% + 2px)"></div></div></div>"#
    ));
    assert_eq!(size(&doc, "[data-a]"), (103.0, 130.0));
}

const CALC_A: &str = "position: absolute; left: 0; top: 0; width: 10px; min-height: calc(50% - 20px); min-width: calc(25% + 3px); padding-top: calc(5% + 2px)";

/// K1b: the same with a block parent of auto height (indefinite on Y only).
#[test]
fn k1b_calc_under_an_auto_height_block() {
    let doc = build(&format!(
        r#"<div data-cb style="{CB}"><div style="height: 20px"></div>
<div style="margin-left: 30px;"><div style="width: 40px; height: 20px"></div><div data-a style="{CALC_A}"></div></div></div>"#
    ));
    assert_eq!(size(&doc, "[data-a]"), (103.0, 130.0));
}

/// K1c: a flex item parent of auto width (indefinite on X).
#[test]
fn k1c_calc_under_an_auto_width_flex_item() {
    let doc = build(&format!(
        r#"<div data-cb style="{CB}"><div style="display: flex; align-items: flex-start"><div><div style="width: 40px; height: 20px"></div><div data-a style="{CALC_A}"></div></div></div></div>"#
    ));
    assert_eq!(size(&doc, "[data-a]"), (103.0, 130.0));
}

/// K1d: control — the PLAIN percentage twins of K1 under the inline-block.
#[test]
fn k1d_plain_percentages_under_an_inline_block() {
    let doc = build(&format!(
        r#"<div data-cb style="{CB}"><div style="height: 20px"></div>
<div style="margin-left: 30px; display: inline-block"><div style="width: 40px; height: 20px"></div><div data-a style="position: absolute; left: 0; top: 0; width: 10px; min-height: 50%; min-width: 25%; padding-top: 5%"></div></div></div>"#
    ));
    assert_eq!(size(&doc, "[data-a]"), (100.0, 150.0));
}

/// K1e: control — is `calc()` resolved at all inside an atomic inline?
#[test]
#[ignore = "fails at the PR head AND on main: calc() is never resolved inside an atomic inline (issue draft review-1409-1)"]
fn k1e_calc_inside_an_inline_block_control() {
    let doc = build(
        r#"<div style="width: 400px"><div style="display: inline-block; width: 200px; position: relative"><div data-f style="width: calc(50% + 10px); height: 5px"></div><div data-a style="position: absolute; left: 0; top: 0; height: 5px; min-width: calc(25% + 3px)"></div></div></div>"#,
    );
    eprintln!(
        "in-flow {:?} abs-direct {:?}",
        size(&doc, "[data-f]"),
        size(&doc, "[data-a]")
    );
    assert_eq!(size(&doc, "[data-f]").0, 110.0);
    assert_eq!(size(&doc, "[data-a]").0, 53.0);
}
