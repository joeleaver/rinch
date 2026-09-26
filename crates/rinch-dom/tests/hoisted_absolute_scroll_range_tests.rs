//! An absolute box hoisted out of a flowed inline element (#591) is measured by
//! the scroll container whose **containing-block chain** it is in (issue #1049).
//!
//! The box's CSS containing block can be a `position: relative` (or nested
//! inside one) inline span *between* it and the scroll container that hosts
//! it. `out_of_flow::out_of_flow_kind` walks DOM parents and stops at that
//! span; `contributes_to_scrollable_overflow` used to ask only whether the
//! scroll container itself was a containing block, so the two disagreed and
//! the box was measured by nobody under a static scroller.
//!
//! Chrome 153, under `* { box-sizing: border-box; border-width: 0; margin: 0;
//! padding: 0 }`, zero-size scrollbars, `font: 16px/20px sans-serif`, a
//! 200x100 `overflow: auto` scroller holding a 100x50 div and then
//! `span{…} > ("x", span{position: absolute; left: 0; top: 300px; 10x10})`:
//!
//! | scroller | outer span | `scrollWidth`x`scrollHeight` |
//! |---|---|---|
//! | static | `relative` | 200x361 |
//! | `relative` | `relative` | 200x361 |
//! | static | static | 200x100 |
//! | static, inside a `relative` div | static | 200x100 |
//! | static | `relative` > static span > the absolute | 200x361 |
//!
//! rinch places the box against its host, not the span (the #386 family), so
//! where Chrome says 361 rinch's extent is 310 (300 + 10). These fixtures pin
//! *whether* the box is counted — the bar and a range reaching past 300 — not
//! that offset.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::scrollbar::{content_extents, scrollbars};

const VIEWPORT: (f32, f32) = (800.0, 600.0);
const ABS: &str = "position: absolute; left: 0; top: 300px; width: 10px; height: 10px";

/// A 200x100 scroller under `wrapper_style`'s div (or under `<body>` when
/// `None`), holding a 100x50 div and then the `outer` spans nested in order
/// (outermost first) around `"x"` and the absolute box.
fn build(
    wrapper_style: Option<&str>,
    scroller_style: &str,
    outer: &[&str],
) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let parent = match wrapper_style {
        Some(style) => {
            let w = doc.create_element("div");
            doc.set_attribute(w, "style", style);
            doc.append_child(body, w);
            w
        }
        None => body,
    };
    let scroller = doc.create_element("div");
    doc.set_attribute(
        scroller,
        "style",
        &format!(
            "width: 200px; height: 100px; overflow: auto; font-size: 16px; line-height: 20px; {scroller_style}"
        ),
    );
    doc.append_child(parent, scroller);
    let block = doc.create_element("div");
    doc.set_attribute(block, "style", "width: 100px; height: 50px");
    doc.append_child(scroller, block);
    let mut at = scroller;
    for style in outer {
        let span = doc.create_element("span");
        doc.set_attribute(span, "style", style);
        doc.append_child(at, span);
        at = span;
    }
    let text = doc.create_text("x");
    doc.append_child(at, text);
    let abs = doc.create_element("span");
    doc.set_attribute(abs, "style", ABS);
    doc.append_child(at, abs);
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    (doc, scroller)
}

fn vertical_range(doc: &RinchDocument, scroller: NodeId) -> (f64, Option<f64>) {
    let (_, h) = content_extents(&doc.tree, scroller.0);
    let bar = scrollbars(&doc.tree, scroller.0, 1.0)
        .vertical
        .map(|t| t.max_scroll);
    (h, bar)
}

/// The issue's own row: a static scroller, a `position: relative` span. The
/// span is the box's containing block and the scroller is the span's, so the
/// box is the scroller's scrollable overflow — Chrome 200x361.
///
/// Fails at `2fa50a2f` with a 70px range and no bar (the container is not a
/// containing block, so the hoisted box was skipped).
#[test]
fn a_hoisted_absolute_under_a_relative_span_is_counted_by_a_static_scroller() {
    let (doc, s) = build(None, "", &["position: relative"]);
    let (h, bar) = vertical_range(&doc, s);
    assert!(h >= 310.0, "extent {h}: the hoisted box must be counted");
    assert!(bar.is_some_and(|m| m >= 210.0), "bar {bar:?}");
}

/// The containing block is the **outer** of two spans: the walk must go past
/// the box's DOM parent. Chrome 200x361.
///
/// Kills a fix that asks only the box's DOM parent.
#[test]
fn the_containing_span_need_not_be_the_boxs_parent() {
    let (doc, s) = build(None, "", &["position: relative", ""]);
    let (h, bar) = vertical_range(&doc, s);
    assert!(h >= 310.0, "extent {h}");
    assert!(bar.is_some(), "bar {bar:?}");
}

/// A positioned scroller still counts it, as it did before. Chrome 200x361.
#[test]
fn a_relative_scroller_still_counts_it() {
    let (doc, s) = build(None, "position: relative", &["position: relative"]);
    let (h, bar) = vertical_range(&doc, s);
    assert!(h >= 310.0, "extent {h}");
    assert!(bar.is_some(), "bar {bar:?}");
}

/// Negative controls: with only **static** spans the box's containing block is
/// above the scroller — the ICB, or a `relative` div wrapping the scroller —
/// and Chrome counts nothing (200x100). Kills a fix that counts every hoisted
/// absolute, or that walks past the scroller.
#[test]
fn a_static_span_leaves_it_to_the_containing_block_above() {
    for wrapper in [None, Some("position: relative")] {
        let (doc, s) = build(wrapper, "", &[""]);
        let (h, bar) = vertical_range(&doc, s);
        assert!(h <= 100.0, "wrapper {wrapper:?}: extent {h}");
        assert_eq!(bar, None, "wrapper {wrapper:?}");
    }
}
