//! Recascade-only restyles (#2571).
//!
//! An element whose only reason to restyle is an inherited value its parent
//! passed down, or a `color` / `font-size` animation sample of its own, keeps
//! the rule node its last selector match built and is only cascaded again
//! (`elements_recascaded`); anything that can change what it matches still
//! matches (`elements_cascaded`). These pin both halves: the values a
//! recascade produces are the ones a full match produces, and every input
//! that can change matching reaches the element as a match.
//!
//! The counts and timings of the 1000-child cases are in
//! `perf_counter_baselines.rs`.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::perf::Counter::*;

const T0: f64 = 1_000_000.0;

type Rgb = (u8, u8, u8);

fn new_doc(css: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.set_animation_clock_ms(Some(T0));
    doc.load_css(&format!(
        "html, body {{ margin: 0; }} body {{ font-size: 16px; line-height: 20px; }} {css}"
    ));
    doc
}

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, class: &str) -> NodeId {
    let e = doc.create_element(tag);
    if !class.is_empty() {
        doc.set_attribute(e, "class", class);
    }
    doc.append_child(parent, e);
    e
}

fn colour_of(doc: &RinchDocument, id: NodeId) -> Rgb {
    let c = doc
        .tree
        .get(id.0)
        .unwrap()
        .computed_style
        .color
        .unwrap()
        .to_rgba8();
    (c.r, c.g, c.b)
}

fn settle(doc: &mut RinchDocument) {
    doc.resolve_layout(800.0, 600.0);
    doc.resolve_layout(800.0, 600.0);
}

/// The inset fast path (`set_styles` of `left`/`top` on an absolute box)
/// writes the inset to layout and replaces the inline `style` block without
/// a cascade. A later inherited change above the box recascades it: that
/// cascade must read the new block, not the rule node of the last match,
/// which still holds the old one — the box would jump back to `left: 0`.
/// (Mutant: drop the `inline_style_unmatched` check from `reuse` in
/// `resolve.rs` — the box is at x = 0.)
#[test]
fn an_inherited_change_after_the_inset_fast_path_keeps_the_new_inset() {
    let mut doc = new_doc(
        ".rel { position: relative; width: 300px; height: 100px; }
         .p { position: absolute; left: 0; top: 0; width: 10px; height: 10px; }",
    );
    let body = doc.body();
    let rel = el(&mut doc, body, "div", "rel");
    let p = el(&mut doc, rel, "div", "p");
    settle(&mut doc);
    doc.set_style(p, "left", "40px");
    settle(&mut doc);
    assert_eq!(
        doc.tree.get(p.0).unwrap().layout.x,
        40.0,
        "positive control: the fast path moved the box"
    );

    doc.tree.perf.reset();
    doc.set_style(rel, "color", "rgb(200, 0, 0)");
    settle(&mut doc);
    assert_eq!(colour_of(&doc, p), (200, 0, 0), "the colour is inherited");
    assert_eq!(
        doc.tree.get(p.0).unwrap().layout.x,
        40.0,
        "the recascade kept the inset the fast path wrote"
    );
    let f = doc.tree.perf.frame();
    assert_eq!(
        (f.get(ElementsCascaded), f.get(ElementsRecascaded)),
        (2, 0),
        "`rel` (its style attribute) and `p` (its inline block is unmatched) both match"
    );

    // Matched once, the box recascades again on the next inherited change.
    doc.tree.perf.reset();
    doc.set_style(rel, "color", "rgb(0, 200, 0)");
    settle(&mut doc);
    assert_eq!(colour_of(&doc, p), (0, 200, 0));
    assert_eq!(doc.tree.get(p.0).unwrap().layout.x, 40.0);
    let f = doc.tree.perf.frame();
    assert_eq!((f.get(ElementsCascaded), f.get(ElementsRecascaded)), (1, 1));
}

/// A class change on the parent that moves an inherited value **and** the
/// child's matching (`.p.on > .c`): the invalidator gives the child a hint
/// of its own, so it matches; a sibling no rule ties to the class only
/// inherits, and is recascaded. (Mutant: reuse the rule node whatever the
/// hint — `.c` keeps `width: auto`.)
#[test]
fn a_parent_class_that_changes_a_child_rule_still_matches_the_child() {
    let mut doc = new_doc(
        ".p.on { color: rgb(0, 0, 200); }
         .p.on > .c { width: 50px; }
         .c, .d { height: 10px; }",
    );
    let body = doc.body();
    let p = el(&mut doc, body, "div", "p");
    let c = el(&mut doc, p, "div", "c");
    let d = el(&mut doc, p, "div", "d");
    settle(&mut doc);
    assert_eq!(doc.tree.get(c.0).unwrap().layout.width, 800.0);

    doc.tree.perf.reset();
    doc.set_attribute(p, "class", "p on");
    settle(&mut doc);
    assert_eq!(doc.tree.get(c.0).unwrap().layout.width, 50.0);
    assert_eq!(colour_of(&doc, c), (0, 0, 200));
    assert_eq!(colour_of(&doc, d), (0, 0, 200));
    let f = doc.tree.perf.frame();
    assert_eq!(
        (f.get(ElementsCascaded), f.get(ElementsRecascaded)),
        (2, 1),
        "`.p` and `.c` match; `.d` recascades"
    );
}

/// A colour animation on a node with a nested subtree: each frame recascades
/// the node and every descendant, none matches, and the deepest one
/// inherits the sample. (Chrome, 500ms of a 1000ms linear `rgb(0,0,0)` →
/// `rgb(200,100,0)` animation: `rgb(100, 50, 0)` all the way down.)
#[test]
fn an_animated_colour_recascades_a_nested_subtree_without_matching() {
    let mut doc = new_doc(
        "@keyframes k2571 { from { color: rgb(0, 0, 0); } to { color: rgb(200, 100, 0); } }
         .a { animation: k2571 1000ms linear; }",
    );
    let body = doc.body();
    let a = el(&mut doc, body, "div", "a");
    let b = el(&mut doc, a, "div", "");
    let c = el(&mut doc, b, "span", "");
    let t = doc.create_text("x");
    doc.append_child(c, t);
    settle(&mut doc);

    doc.tree.perf.reset();
    doc.set_animation_clock_ms(Some(T0 + 500.0));
    doc.tick_animations();
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(colour_of(&doc, a), (100, 50, 0));
    assert_eq!(colour_of(&doc, c), (100, 50, 0), "Chrome: rgb(100, 50, 0)");
    let f = doc.tree.perf.frame();
    assert_eq!((f.get(ElementsCascaded), f.get(ElementsRecascaded)), (0, 3));
}

/// An animated node whose class changes in the same frame as a tick: the
/// class change's hint outranks the tick's `RECASCADE_SELF`, so the node
/// matches and picks up the new rule. (Mutant: `mark_recascade` replacing
/// the hint, or `reuse` accepting any hint — the width stays `auto`.)
#[test]
fn a_class_change_in_a_tick_frame_still_matches_the_animated_node() {
    let mut doc = new_doc(
        "@keyframes k2571b { from { color: rgb(0, 0, 0); } to { color: rgb(200, 100, 0); } }
         .a { animation: k2571b 1000ms linear; height: 10px; }
         .a.w { width: 70px; }",
    );
    let body = doc.body();
    let a = el(&mut doc, body, "div", "a");
    let _kid = el(&mut doc, a, "span", "");
    settle(&mut doc);
    doc.set_attribute(a, "class", "a w");
    doc.set_animation_clock_ms(Some(T0 + 500.0));
    doc.tick_animations();
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(doc.tree.get(a.0).unwrap().layout.width, 70.0);
    assert_eq!(colour_of(&doc, a), (100, 50, 0));
}

/// An animated `font-size` reaches an `em` length on a child that
/// recascades: the cascade resolves it against the new parent size.
/// (Chrome: `font-size` 40px at 500ms of 32px → 48px; child `width: 2em` →
/// 80px.)
#[test]
fn an_animated_font_size_recascades_a_childs_em_width() {
    let mut doc = new_doc(
        "@keyframes k2571c { from { font-size: 32px; } to { font-size: 48px; } }
         .a { animation: k2571c 1000ms linear; }
         .k { width: 2em; height: 10px; }",
    );
    let body = doc.body();
    let a = el(&mut doc, body, "div", "a");
    let k = el(&mut doc, a, "div", "k");
    settle(&mut doc);
    doc.tree.perf.reset();
    doc.set_animation_clock_ms(Some(T0 + 500.0));
    doc.tick_animations();
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(doc.tree.get(k.0).unwrap().layout.width, 80.0);
    let f = doc.tree.perf.frame();
    assert_eq!((f.get(ElementsCascaded), f.get(ElementsRecascaded)), (0, 2));
}

/// A late sheet whose selector Stylo invalidates as a subtree
/// (`.p .c > div`: `.p` gets `RESTYLE_DESCENDANTS`), in the same frame as an
/// inherited change on `.p`: the descendants arrive with the parent asking
/// `Subtree`, carry no hint of their own, and must match — a recascade would
/// keep the rules from before the sheet. Compared with a fresh document.
/// (Mutant: drop `inherited != ChildCascade::Subtree` from `reuse` — the
/// `div` keeps `width: auto`.)
#[test]
fn a_subtree_hint_from_a_late_sheet_still_matches_the_descendants() {
    const CSS: &str = ".c > div { height: 10px; }";
    const LATE: &str = ".p .c > div { width: 50px; }";
    let build = |doc: &mut RinchDocument| {
        let body = doc.body();
        let p = el(doc, body, "div", "p");
        let c = el(doc, p, "div", "c");
        let d = el(doc, c, "div", "");
        (p, d)
    };

    let mut doc = new_doc(CSS);
    let (p, d) = build(&mut doc);
    settle(&mut doc);
    assert_eq!(
        doc.tree.get(d.0).unwrap().layout.width,
        800.0,
        "positive control: no width rule yet"
    );
    doc.load_css(LATE);
    doc.set_style(p, "color", "rgb(0, 0, 200)");
    settle(&mut doc);

    let mut fresh = new_doc(CSS);
    fresh.load_css(LATE);
    let (fp, fd) = build(&mut fresh);
    fresh.set_style(fp, "color", "rgb(0, 0, 200)");
    settle(&mut fresh);

    assert_eq!(fresh.tree.get(fd.0).unwrap().layout.width, 50.0);
    assert_eq!(
        doc.tree.get(d.0).unwrap().layout.width,
        50.0,
        "as the fresh document"
    );
    assert_eq!(colour_of(&doc, d), colour_of(&fresh, fd));
}
