//! A box collapsed to a real zero still clips what is inside it.
//!
//! Card K51 in the application this was found in: a group header's tap
//! toggled its collapsed signal correctly, the wrapper's height went to `0`,
//! and the rows underneath went on painting at full size in their old
//! position — forever, for as long as the height said not to.
//!
//! Two independent faults, both of them a zero being read as "nothing to do
//! here" rather than as "this is the most extreme value there is", and either
//! one alone is enough to produce the symptom. So there is a test for each:
//! one that drives a document the way a collapsed group actually is, and one
//! that puts a degenerate clip straight into the software painter, so that a
//! future change which fixes only half of it cannot go green.
//!
//! # Pinning the seam, not only the symptom
//!
//! Those two tests state the bug. They do **not** state the contract the fix
//! rests on. Measured against the 701 tests this crate already had, **four**
//! mutations of the new clip site survive both of them and all 701: passing
//! `None` for `root_clip`, never popping the bracket, popping it
//! unconditionally, and spelling the predicate the pre-#324 `overflow_y` way.
//! Each test below exists because one of those lived, and each was proved red
//! against its own mutant before being kept.
//!
//! Two candidates were dropped after measuring rather than kept on the strength
//! of the argument for them, and both are recorded so the next person does not
//! re-derive them:
//!
//! - **pushing the clip without asking `overflow`** is already killed by two
//!   pre-existing #142 tests (`test_zero_height_container_applies_transform`
//!   and `_applies_opacity`), so the test for it here is belt-and-braces at a
//!   different altitude — a pixel assertion rather than a transform one — not
//!   the only thing standing between that mutant and green;
//! - **deriving the clip at the *scrolled* origin** is an **equivalent**
//!   mutation on this branch. It is written up on the test that would have
//!   pinned it.
//!
//! They also deliberately step off the values a mutant can hide on
//! (`reference_test_fixed_point_blindness`): the pair above sample **scroll
//! offset 0**, **`scale = 1.0`**, **one** child under **one** clipper, and — the
//! one that is a whole axis rather than a value — a collapsed **height** every
//! time, never a collapsed width. The tests below carry a non-zero scroll
//! offset, a scale of `2.0`, a second sibling, and a `width: 0` box.
//!
//! Radius is the trap that cannot be stepped off, and the obvious argument for
//! why that is safe is **false**: `clip::border_radii` resolves against
//! `min(width, height)`, which is `0` here, so a *percentage* radius does come
//! back as `0` — but `LengthPercentageValue::resolve` ignores the basis for a
//! `Length`, and `border-radius: 10px` on a `height: 0` box hands back
//! `{10, 10, 10, 10}`. What makes a `RoundedRect` and a plain `Rect` identical
//! anyway is kurbo's clamp: `Rect::to_rounded_rect` caps every corner at half
//! the shorter side, which is `0`. Measured with `10px` and `50%`.
//!
//! **What none of this can see:** every assertion here reads the tiny-skia
//! pixmap. The DOM-side fix is renderer-agnostic and pushes the same clip on the
//! Vello path, and to be precise about what that means — `paint_tests.rs` drives
//! `VelloPainter` with no GPU, so a *structural* assertion that the clip reaches
//! the scene at all was writable and is simply not written here. What no
//! CPU-side test can answer is the question that actually matters: whether Vello
//! **honours** a zero-area clip once it rasterizes. `TinySkiaPainter::push_layer`
//! takes its `_bounds` and never reads them, which is how #550's GPU-only
//! divergence passed every software assertion, so a difference of that shape
//! would pass everything here too. The painter test below is a statement about
//! `TinySkiaPainter` alone and says so.

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 300.0;
const VH: f32 = 300.0;

fn paint(doc: &mut RinchDocument, painter: &mut TinySkiaPainter) {
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut layout_cx,
    );
}

fn pixel_at(painter: &TinySkiaPainter, x: u32, y: u32) -> [u8; 4] {
    let idx = ((y * painter.width() + x) * 4) as usize;
    let d = painter.pixels();
    [d[idx], d[idx + 1], d[idx + 2], d[idx + 3]]
}

/// The shape a closed group actually has: a wrapper whose height is `0` and
/// whose `overflow: hidden` is the whole reason the rows inside it should
/// stop being seen, with a real, in-flow, painted child still in it.
///
/// The child is red and 100px tall. If the wrapper clips, no red reaches the
/// pixmap at all. Before the fix, `paint_node`'s zero-in-one-dimension branch
/// — added for an *unclipped* auto-height wrapper whose only content is
/// absolutely positioned and already escaping it — returned before any of the
/// ordinary `overflow: hidden` handling further down could run, so the rows
/// painted straight through a box that had been collapsed precisely to hide
/// them.
#[test]
fn a_zero_height_overflow_hidden_wrapper_clips_its_in_flow_child() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let wrapper = doc.create_element("div");
    doc.set_attribute(
        wrapper,
        "style",
        "height: 0; overflow: hidden; width: 200px",
    );
    doc.append_child(body, wrapper);

    let row = doc.create_element("div");
    doc.set_attribute(
        row,
        "style",
        "background-color: rgb(255, 0, 0); width: 200px; height: 100px",
    );
    doc.append_child(wrapper, row);

    doc.resolve_layout(VW, VH);

    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    paint(&mut doc, &mut painter);

    for (x, y) in [(10, 10), (100, 40), (150, 80), (10, 99)] {
        let px = pixel_at(&painter, x, y);
        assert_ne!(
            [px[0], px[1], px[2]],
            [255, 0, 0],
            "a row inside a wrapper collapsed to height: 0 with overflow: hidden \
             painted at ({x}, {y}); the wrapper is what says it should not be seen"
        );
    }
}

/// The painter half, on its own. A clip path whose bounds round away to
/// nothing is the *strictest* clip there is, not the absence of one — and the
/// branch that noticed the degenerate path used to leave `clip_mask` at the
/// `None` that `.take()` had just put there, which is "clip nothing" and so
/// painted the subtree in full.
///
/// Driven through the trait rather than through a document so that it stays a
/// statement about the painter even if the DOM side above is rewritten.
#[test]
fn a_degenerate_clip_path_clips_everything_rather_than_nothing() {
    use peniko::{Fill, kurbo::Affine, kurbo::Rect};
    use rinch_dom::paint::painter::Painter;

    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);

    // A clip with no area at all — what a `height: 0` box's own rect is.
    let degenerate = Rect::new(0.0, 0.0, 200.0, 0.0);
    painter.push_clip(Fill::NonZero, Affine::IDENTITY, &degenerate.into());

    // Then paint something big and opaque inside it.
    let inside = Rect::new(0.0, 0.0, 200.0, 100.0);
    painter.fill_color(
        Fill::NonZero,
        Affine::IDENTITY,
        peniko::color::palette::css::RED,
        &inside.into(),
    );
    painter.pop_layer();

    for (x, y) in [(10, 10), (100, 40), (150, 80)] {
        let px = pixel_at(&painter, x, y);
        assert_ne!(
            [px[0], px[1], px[2]],
            [255, 0, 0],
            "a fill inside a zero-area clip reached ({x}, {y}); a clip that \
             encloses no area must admit nothing, not everything"
        );
    }
}

/// Helper for the seam tests below: a styled `<div>` under `parent`.
fn div(
    doc: &mut RinchDocument,
    parent: rinch_core::dom::NodeId,
    style: &str,
) -> rinch_core::dom::NodeId {
    let n = doc.create_element("div");
    doc.set_attribute(n, "style", style);
    doc.append_child(parent, n);
    n
}

/// Paint at an explicit scale, so `scale = 1.0` is not the only sample taken.
fn paint_at(doc: &mut RinchDocument, painter: &mut TinySkiaPainter, scale: f64) {
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        painter,
        scale,
        (VW, VH),
        &mut doc.font_cx,
        &mut layout_cx,
    );
}

/// **The seam.** A collapsed clipper that is also a stacking context owns any
/// `position: fixed` box below it (#545), and that box is *not* clipped by it:
/// `paint_children_with_stacking` lifts the bracket `paint_node` opened, paints
/// the entry, and puts the bracket back. It can only do that if it was told the
/// bracket exists — so the collapsed arm has to hand down its `root_clip` and
/// not the `None` that was true before it pushed anything.
///
/// This is the mutant that a clean three-way merge produced and that nothing
/// else in this crate's 620 tests notices: the fix's new `push_clip` landed
/// beside the `None` that #545 had made stale, and the only symptom is a fixed
/// box that silently stops being drawn.
#[test]
fn a_fixed_box_escapes_a_collapsed_clipper_that_owns_it() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    // `position: relative; z-index: 0` makes it a stacking context, so #545
    // hoists the fixed box no further than here; `height: 0; overflow: hidden`
    // makes it the collapsed clipper.
    let sc = div(
        &mut doc,
        body,
        "position: relative; z-index: 0; overflow: hidden; width: 200px; height: 0",
    );
    div(
        &mut doc,
        sc,
        "position: fixed; left: 150px; top: 150px; width: 80px; height: 80px; \
         background-color: rgb(0, 0, 255)",
    );

    doc.resolve_layout(VW, VH);
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    paint(&mut doc, &mut painter);

    let px = pixel_at(&painter, 190, 190);
    assert_eq!(
        [px[0], px[1], px[2]],
        [0, 0, 255],
        "a `position: fixed` box was swallowed by the collapsed clipper that \
         merely owns it; this root is not its containing block, so paint has to \
         lift its own bracket around it (#545) — which it can only do if the \
         collapsed arm declares that bracket rather than passing `None`"
    );
}

/// The bracket the collapsed arm opens has to be popped again, or every box
/// painted after it inherits a clip that encloses nothing. One clipper and one
/// child cannot show this: it needs a **second** subject, painted afterwards.
#[test]
fn the_collapsed_clippers_bracket_does_not_leak_onto_a_later_sibling() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let wrapper = div(&mut doc, body, "overflow: hidden; width: 200px; height: 0");
    div(
        &mut doc,
        wrapper,
        "width: 200px; height: 100px; background-color: rgb(255, 0, 0)",
    );
    div(
        &mut doc,
        body,
        "width: 200px; height: 200px; background-color: rgb(0, 255, 0)",
    );

    doc.resolve_layout(VW, VH);
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    paint(&mut doc, &mut painter);

    let px = pixel_at(&painter, 100, 100);
    assert_eq!(
        [px[0], px[1], px[2]],
        [0, 255, 0],
        "the sibling painted after the collapsed clipper was clipped by it, so \
         the bracket was pushed and never popped"
    );
}

/// The predicate is `Node::clips_overflow` — #324 stage A's one spelling — and
/// not the pre-stage-A `overflow_y` against `Hidden | Scroll | Auto`. The two
/// part company on exactly one value, so `overflow: clip` is the only sample
/// that can tell them apart.
#[test]
fn a_collapsed_box_clips_when_its_overflow_is_clip() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let wrapper = div(&mut doc, body, "overflow: clip; width: 200px; height: 0");
    div(
        &mut doc,
        wrapper,
        "width: 200px; height: 100px; background-color: rgb(255, 0, 0)",
    );

    doc.resolve_layout(VW, VH);
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    paint(&mut doc, &mut painter);

    let px = pixel_at(&painter, 100, 40);
    assert_ne!(
        [px[0], px[1], px[2]],
        [255, 0, 0],
        "`overflow: clip` did not clip — the predicate is one of the four \
         spellings #324 stage A deleted, and this is the value they disagree on"
    );
}

/// A collapsed clipper that is also a **scroller**, painted at a scale of
/// `2.0` — the two tests above sample `overflow: hidden` at `1.0` with no
/// scroll offset, and `scroll` is a third value of the predicate.
///
/// One thing this deliberately does **not** claim, because it was measured and
/// found false: it does not pin *where* the clip is taken. The clip is the box,
/// which does not move when its content scrolls, so it is derived at the node's
/// unscrolled origin while the children are painted at the scrolled one — and
/// deriving it at the scrolled origin instead is an **equivalent** mutation
/// here, not one this test lets through. A rect that encloses no area encloses
/// none wherever it is put, so on this branch the origin cannot change a single
/// pixel, on either backend. Anyone adding a pin for it is chasing a
/// distinction the zero has already erased; the origin only becomes observable
/// on the full-size bracket, where `paint_tests` covers it.
#[test]
fn a_scrolled_collapsed_clipper_still_clips_at_a_non_unit_scale() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let wrapper = div(&mut doc, body, "overflow: scroll; width: 100px; height: 0");
    div(
        &mut doc,
        wrapper,
        "width: 100px; height: 400px; background-color: rgb(255, 0, 0)",
    );

    doc.resolve_layout(VW, VH);
    doc.tree.nodes[wrapper.0].scroll_offset = (0.0, 120.0);

    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    paint_at(&mut doc, &mut painter, 2.0);

    for (x, y) in [(10, 10), (100, 60), (150, 150), (10, 250)] {
        let px = pixel_at(&painter, x, y);
        assert_ne!(
            [px[0], px[1], px[2]],
            [255, 0, 0],
            "a scrolled, collapsed clipper let its content paint at ({x}, {y}) \
             under a scale of 2.0"
        );
    }
}

/// The other half of the contract, and the one a clip pushed too eagerly would
/// break: a box collapsed on one axis whose `overflow` is `visible` still paints
/// its children in full. That is the case #142 added this branch for, and the
/// fix must leave it exactly as it was.
#[test]
fn a_collapsed_box_that_does_not_clip_still_paints_its_children() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let wrapper = div(&mut doc, body, "width: 200px; height: 0");
    div(
        &mut doc,
        wrapper,
        "width: 200px; height: 100px; background-color: rgb(0, 255, 0)",
    );

    doc.resolve_layout(VW, VH);
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    paint(&mut doc, &mut painter);

    let px = pixel_at(&painter, 100, 40);
    assert_eq!(
        [px[0], px[1], px[2]],
        [0, 255, 0],
        "a collapsed box with `overflow: visible` clipped its children; the \
         branch exists precisely so that it does not (#142)"
    );
}

/// A degenerate clip nested **inside** a real one, which is the shape every
/// software frame actually has: `RinchApp::build_pixels` pushes the dirty-region
/// clip first, so `previous_mask` is `Some(..)` for every DOM clip in a partial
/// repaint. The give-up branch used to `.take()` that mask and never put
/// anything back, which does not merely fail to add a clip — it drops the
/// enclosing one, and the subtree paints through both.
///
/// The two assertions are separate claims: nothing escapes while the degenerate
/// clip is open, and the enclosing clip is *back* once it is popped.
#[test]
fn a_degenerate_clip_does_not_take_the_enclosing_clip_with_it() {
    use peniko::{Fill, kurbo::Affine, kurbo::Rect};
    use rinch_dom::paint::painter::Painter;

    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    let red = |p: &mut TinySkiaPainter| {
        p.fill_color(
            Fill::NonZero,
            Affine::IDENTITY,
            peniko::color::palette::css::RED,
            &Rect::new(0.0, 0.0, VW as f64, VH as f64).into(),
        );
    };

    // The enclosing clip: nothing may ever reach (100, 100) through it.
    painter.push_clip(
        Fill::NonZero,
        Affine::IDENTITY,
        &Rect::new(0.0, 0.0, 50.0, 50.0).into(),
    );
    painter.push_clip(
        Fill::NonZero,
        Affine::IDENTITY,
        &Rect::new(0.0, 0.0, 200.0, 0.0).into(),
    );
    red(&mut painter);
    painter.pop_layer();

    let px = pixel_at(&painter, 100, 100);
    assert_ne!(
        [px[0], px[1], px[2]],
        [255, 0, 0],
        "a fill inside a degenerate clip escaped the *enclosing* 50x50 clip: \
         giving up on the inner clip dropped the outer one with it"
    );

    // …and the enclosing clip is still in force after the degenerate one pops.
    red(&mut painter);
    painter.pop_layer();

    let inside = pixel_at(&painter, 10, 10);
    assert_eq!(
        [inside[0], inside[1], inside[2]],
        [255, 0, 0],
        "the enclosing clip was not restored when the degenerate one popped"
    );
    let outside = pixel_at(&painter, 100, 100);
    assert_ne!(
        [outside[0], outside[1], outside[2]],
        [255, 0, 0],
        "the enclosing clip was restored too wide"
    );
}

/// The other axis. Every other fixture in this file collapses the **height**,
/// which leaves the whole `(width == 0) != (height == 0)` branch sampled on one
/// side of its own XOR — a mutation that read `layout.height` where it means
/// "the collapsed axis" would be invisible to all of them.
///
/// A `width: 0` clipper is not a hypothetical shape either: it is what a
/// horizontally collapsed pane or a `flex-basis: 0` sidebar leaves behind.
#[test]
fn a_zero_width_overflow_hidden_wrapper_clips_its_in_flow_child() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let wrapper = div(&mut doc, body, "overflow: hidden; width: 0; height: 100px");
    div(
        &mut doc,
        wrapper,
        "width: 200px; height: 100px; background-color: rgb(255, 0, 0)",
    );

    doc.resolve_layout(VW, VH);
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    paint(&mut doc, &mut painter);

    for (x, y) in [(10, 10), (100, 40), (150, 80)] {
        let px = pixel_at(&painter, x, y);
        assert_ne!(
            [px[0], px[1], px[2]],
            [255, 0, 0],
            "a child inside a wrapper collapsed to `width: 0` with \
             `overflow: hidden` painted at ({x}, {y}); the collapsed axis is not \
             always the height"
        );
    }
}

/// The bracket has to balance in **both** directions, and this is the half the
/// leak test above cannot see. A collapsed box that does *not* clip pushes
/// nothing, so a `pop_layer` that runs unconditionally pops whatever was below —
/// an ancestor's clip — and everything painted afterwards escapes it.
///
/// Found by measuring rather than by reading: dropping the `root_clip.is_some()`
/// guard around the pop survives #540's two tests, all 701 pre-existing ones,
/// *and* every other test in this file. It is the mirror image of
/// `the_collapsed_clippers_bracket_does_not_leak_onto_a_later_sibling` — one
/// pins the push without a pop, this one the pop without a push.
#[test]
fn a_collapsed_box_that_pushes_no_bracket_pops_none_either() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    // The ancestor clip that must survive the collapsed box below it.
    let clipper = div(
        &mut doc,
        body,
        "overflow: hidden; width: 60px; height: 60px",
    );

    // Collapsed on one axis, `overflow: visible`: it pushes no bracket at all.
    div(&mut doc, clipper, "width: 200px; height: 0");

    // Painted after it, and far outside the 60x60 clipper.
    div(
        &mut doc,
        clipper,
        "width: 200px; height: 200px; background-color: rgb(255, 0, 0)",
    );

    doc.resolve_layout(VW, VH);
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    paint(&mut doc, &mut painter);

    let px = pixel_at(&painter, 150, 150);
    assert_ne!(
        [px[0], px[1], px[2]],
        [255, 0, 0],
        "content escaped a 60x60 `overflow: hidden` ancestor: a collapsed box \
         that pushed no bracket popped one anyway, and took the ancestor's clip \
         off the stack with it"
    );
}
