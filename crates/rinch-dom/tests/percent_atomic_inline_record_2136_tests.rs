//! #2136 — a percentage-sized atomic inline is resolved against its
//! containing block once, not on every layout.
//!
//! `resolve_percentage_inline_blocks` sizes an atomic inline whose `width`,
//! `min-width` or `max-width` is a percentage after the root compute, against
//! its containing block's inner width. It used to do that — two detached
//! computes, and a re-shape of the box's text — after **every** root compute,
//! whatever had moved: a `max-width: 100%` pill on each of 430 grid cards was
//! 860 `inline_block_computes` on a layout that changed nothing in the grid.
//! It is now recorded (`NodeTree::percent_inline_cb_width`) at the width it
//! was resolved against, as the `fit-content`/`stretch` boxes have been since
//! #1281, and skipped while that width stands. The record is dropped by every
//! measure of the box as `auto` — a change inside it or to its own style, a
//! structural pass — and by the node being freed.
//!
//! So every fixture here is a **history** compared with a **fresh layout** of
//! the state the history ends in: the record must never serve a size the
//! fresh layout would not give. Each one first checks that the box's size
//! actually moved (the positive control) — a fixture whose box keeps its size
//! passes against a record that is never dropped. The counts that the record
//! saves are pinned in `perf_regression_scenarios.rs`
//! (`percentage_pills_in_a_grid_*`).
//!
//! No number here is a glyph measurement: each assertion compares two layouts
//! of the same document on the same host, or a box against a declared length.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VP: (f32, f32) = (800.0, 600.0);

/// Words enough that the pill's text, which wraps, is wider than any block
/// here, so `max-width: 100%` is what sizes it. (Wrapping text: a pill whose
/// content cannot get narrower — `white-space: nowrap`, or a 400px box — is
/// left at its min-content width by rinch where Chrome caps it, a separate
/// bug, so a fixture on it would not exercise the cap.)
const LONG: usize = 200;

const CSS: &str = "body { margin: 0; font-size: 16px; line-height: 20px }
    .cb { width: 300px }
    .pill { display: inline-flex; max-width: 100% }
    .half { display: inline-block; width: 50%; height: 10px }
    .chip { display: inline-block }";

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, class: &str) -> NodeId {
    let e = doc.create_element(tag);
    if !class.is_empty() {
        doc.set_attribute(e, "class", class);
    }
    doc.append_child(parent, e);
    e
}

fn text(doc: &mut RinchDocument, parent: NodeId, s: &str) -> NodeId {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
    t
}

fn size(doc: &RinchDocument, id: NodeId) -> (f32, f32) {
    let n = &doc.tree.nodes[id.0];
    (n.layout.width, n.layout.height)
}

fn settle(doc: &mut RinchDocument) {
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
}

/// The fixture's boxes: a containing block holding a `max-width: 100%` pill
/// (its text wider than the block, unless it is one word), a `width: 50%` box, and
/// an `auto` chip holding a percentage box.
struct Doc {
    doc: RinchDocument,
    cb: NodeId,
    pill: NodeId,
    pill_text: NodeId,
    half: NodeId,
    chip: NodeId,
}

fn build(cb_width: &str, pill_words: usize) -> Doc {
    build_with(cb_width, pill_words, "")
}

fn build_with(cb_width: &str, pill_words: usize, extra_css: &str) -> Doc {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    if !extra_css.is_empty() {
        doc.load_css(extra_css);
    }
    let body = doc.body();
    let cb = el(&mut doc, body, "div", "cb");
    doc.set_style(cb, "width", cb_width);
    let pill = el(&mut doc, cb, "span", "pill");
    let pill_text = text(&mut doc, pill, &"word ".repeat(pill_words));
    text(&mut doc, cb, " ");
    let half = el(&mut doc, cb, "span", "half");
    text(&mut doc, cb, " ");
    let chip = el(&mut doc, cb, "span", "chip");
    el(&mut doc, chip, "span", "half");
    settle(&mut doc);
    Doc {
        doc,
        cb,
        pill,
        pill_text,
        half,
        chip,
    }
}

#[track_caller]
fn assert_matches_fresh(got: &Doc, fresh: &Doc, what: &str) {
    for (name, a, b) in [
        ("pill", got.pill, fresh.pill),
        ("half", got.half, fresh.half),
        ("chip", got.chip, fresh.chip),
    ] {
        assert_eq!(
            size(&got.doc, a),
            size(&fresh.doc, b),
            "{what}: the {name} after the history differs from a fresh layout"
        );
    }
}

/// The containing block narrows: the record is at the old width, so every
/// percentage box is resolved again. A record keyed on nothing (a mutant that
/// skips any box it has an entry for) keeps the 300px answers.
#[test]
fn a_narrower_containing_block_resolves_the_percentage_boxes_again() {
    let mut d = build("300px", LONG);
    let before = (size(&d.doc, d.pill), size(&d.doc, d.half));
    assert_eq!(before.0.0, 300.0, "the pill is capped at its block");
    assert_eq!(before.1.0, 150.0, "the half box is 50% of 300px");

    d.doc.set_style(d.cb, "width", "120px");
    settle(&mut d.doc);
    let fresh = build("120px", LONG);
    assert_eq!(size(&fresh.doc, fresh.pill).0, 120.0, "positive control");
    assert_eq!(size(&fresh.doc, fresh.half).0, 60.0, "positive control");
    assert_matches_fresh(&d, &fresh, "300px -> 120px");

    // And back: the record now says 120px.
    d.doc.set_style(d.cb, "width", "300px");
    settle(&mut d.doc);
    let fresh = build("300px", LONG);
    assert_matches_fresh(&d, &fresh, "120px -> 300px");
}

/// The pill's own content grows past its block at the same block width: the
/// text write measures the pill as `auto`, which drops its record, so it is
/// capped again. A mutant that keeps the record through an `auto` measure
/// leaves the pill at its uncapped `auto` width.
#[test]
fn new_content_in_a_percentage_box_is_resolved_at_an_unchanged_block_width() {
    let mut d = build("300px", 1);
    let narrow = size(&d.doc, d.pill);
    assert!(narrow.0 < 300.0, "the one-word pill fits ({narrow:?})");

    d.doc.set_text_content(d.pill_text, &"word ".repeat(LONG));
    settle(&mut d.doc);
    let fresh = build("300px", LONG);
    assert_eq!(
        size(&fresh.doc, fresh.pill).0,
        300.0,
        "positive control: the long pill is capped"
    );
    assert_matches_fresh(&d, &fresh, "a long text in the pill");

    d.doc.set_text_content(d.pill_text, "word ");
    settle(&mut d.doc);
    let fresh = build("300px", 1);
    assert_eq!(size(&d.doc, d.pill), narrow, "back to one word");
    assert_matches_fresh(&d, &fresh, "back to one word");
}

/// The box's own percentage changes at the same block width: the restyle
/// queues it for an `auto` measure, which drops the record.
#[test]
fn a_changed_percentage_at_an_unchanged_block_width_is_resolved_again() {
    let mut d = build("300px", LONG);
    assert_eq!(size(&d.doc, d.pill).0, 300.0);
    d.doc.set_style(d.pill, "max-width", "50%");
    d.doc.set_style(d.half, "width", "25%");
    settle(&mut d.doc);

    let fresh = build_with(
        "300px",
        LONG,
        ".cb > .pill { max-width: 50% } .cb > .half { width: 25% }",
    );
    assert_eq!(
        size(&fresh.doc, fresh.pill).0,
        150.0,
        "positive control: 50% of 300px"
    );
    assert_eq!(size(&fresh.doc, fresh.half).0, 75.0, "positive control");
    assert_matches_fresh(&d, &fresh, "max-width 100% -> 50%, width 50% -> 25%");
}

/// A change elsewhere leaves every percentage box at the size it was
/// resolved to, and lays none of them out again: the record is what is served
/// (`inline_block_computes` stays 0 — on main, each of the pill and the two
/// `width: 50%` boxes was laid out again), and it must be the right size.
#[test]
fn an_unrelated_change_keeps_every_percentage_box_at_its_resolved_size() {
    use rinch_dom::perf::Counter::InlineBlockComputes;
    let mut d = build("300px", LONG);
    let before: Vec<_> = [d.pill, d.half, d.chip]
        .iter()
        .map(|&n| size(&d.doc, n))
        .collect();
    let body = d.doc.body();
    let other = el(&mut d.doc, body, "div", "");
    let t = text(&mut d.doc, other, "elsewhere");
    settle(&mut d.doc);
    d.doc.tree.perf.reset();
    d.doc.set_text_content(t, "elsewhere, changed");
    d.doc.resolve_layout(VP.0, VP.1);
    let s = d.doc.tree.perf.end_frame();
    assert_eq!(
        s.get(InlineBlockComputes),
        0,
        "no percentage box is laid out again for a change elsewhere"
    );
    settle(&mut d.doc);
    let after: Vec<_> = [d.pill, d.half, d.chip]
        .iter()
        .map(|&n| size(&d.doc, n))
        .collect();
    assert_eq!(before, after);
}

/// A percentage box moved to another block of the **same** width, whose
/// inherited `font-size` is twice as large: the block width alone would
/// match the record, so only the move's `auto` measure dropping it makes the
/// pill wrap its (now wider) text again. Compared with the pill laid out
/// fresh in that block.
#[test]
fn a_percentage_box_moved_to_a_same_width_block_is_resolved_again() {
    let mut d = build("300px", LONG);
    let body = d.doc.body();
    let other = el(&mut d.doc, body, "div", "cb");
    d.doc.set_style(other, "font-size", "32px");
    settle(&mut d.doc);
    let small = size(&d.doc, d.pill);
    assert_eq!(small.0, 300.0, "capped in the 16px block");
    d.doc.append_child(other, d.pill);
    settle(&mut d.doc);

    let mut fresh = RinchDocument::new();
    fresh.load_css(CSS);
    let fbody = fresh.body();
    let fcb = el(&mut fresh, fbody, "div", "cb");
    fresh.set_style(fcb, "font-size", "32px");
    let fpill = el(&mut fresh, fcb, "span", "pill");
    text(&mut fresh, fpill, &"word ".repeat(LONG));
    settle(&mut fresh);
    let want = size(&fresh, fpill);
    assert_eq!(want.0, 300.0, "positive control: capped in the 32px block");
    assert!(
        want.1 > small.1,
        "positive control: twice the font wraps to more lines ({want:?} vs {small:?})"
    );
    assert_eq!(
        size(&d.doc, d.pill),
        want,
        "the moved pill vs a fresh layout"
    );
}

/// A `calc()` size inside a percentage box, resolved against the box's own
/// width (review of #2136): `resolve_layout_calcs` rewrites the child's
/// Taffy style after the root compute, which never reaches the box's
/// detached subtree, so the box is laid out again around it. With the
/// record and nothing else the child stayed at its `calc` seed (10px), and a
/// narrower block left it at its old width; before the record it was right
/// one layout late.
#[test]
fn a_calc_child_inside_a_percentage_box_follows_the_box() {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    doc.load_css(".c { display: block; width: calc(50% + 10px); height: 5px }");
    let body = doc.body();
    let cb = el(&mut doc, body, "div", "cb");
    let half = el(&mut doc, cb, "span", "half");
    doc.set_style(half, "height", "auto");
    let c = el(&mut doc, half, "div", "c");
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(size(&doc, half).0, 150.0);
    assert_eq!(
        size(&doc, c).0,
        85.0,
        "50% of 150px + 10px, on the first layout"
    );
    doc.set_style(cb, "width", "200px");
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(size(&doc, half).0, 100.0);
    assert_eq!(size(&doc, c).0, 60.0, "50% of 100px + 10px");
    settle(&mut doc);
    assert_eq!(size(&doc, c).0, 60.0, "and it stays");
}
