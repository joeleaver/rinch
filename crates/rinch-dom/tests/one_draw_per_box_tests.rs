//! One draw per box, at the IFC's position (#365, #407).
//!
//! The invariant: **a box carrying an `ifc_root` is painted exactly once, by
//! that IFC** — not by a tree-order walk, not by a stacking sequence. Two
//! independent terms of one predicate violated it.
//!
//! `already_drawn_inline` reads
//! `skip_ifc_children && kind != PaintKind::StackingContext && child.ifc_root == Some(node_id)`,
//! and has two ways to miss:
//!
//! - the **third** term can never match for a subtree hoisted to an *ancestor*
//!   (the box's `ifc_root` names its own IFC, not the node being painted);
//! - the **second** term excludes any inline-level box that is itself a
//!   stacking context, even as a direct child of the node being painted.
//!
//! Either way the box is drawn twice: once by `paint_inline_layout` at the IFC
//! root's **content** origin, once by the stacking sequence at its
//! **border-box** origin — two copies exactly one padding+border apart.
//!
//! The oracle is a **translucent fill**, and getting there took two attempts
//! worth recording, because the obvious oracles both fail on this bug.
//!
//! A *bounding box* cannot see it: two copies of an opaque fill that overlap
//! share one bbox. A *pixel count* sees it only while the copies are apart —
//! and #407's half of this fix moves them **on top of each other**, so once
//! the geometry is corrected an opaque count reads exactly the same for one
//! draw and for two. A mutation run caught that: reverting the #365 guard left
//! every count-based test green.
//!
//! A 50%-alpha fill composites differently for one draw than for two — one is
//! `rgb(255,127,127)` over white, two is `rgb(255,63,63)` — so it distinguishes
//! them *however* they are positioned. That is the property this file needs.

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

fn child_of(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let el = doc.create_element(tag);
    doc.set_attribute(el, "style", style);
    doc.append_child(parent, el);
    el
}

fn rasterize(doc: &mut RinchDocument) -> Vec<u8> {
    rasterize_scale(doc, 1.0)
}

/// Rasterize at a DPI scale. Layout stays in logical pixels; the pixmap and
/// every coordinate in it are physical.
fn rasterize_scale(doc: &mut RinchDocument, scale: f64) -> Vec<u8> {
    let pw = (VW as f64 * scale) as u32;
    let ph = (VH as f64 * scale) as u32;
    let mut painter = TinySkiaPainter::new(pw, ph);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale,
        (pw as f32, ph as f32),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter.pixels().to_vec()
}

/// How many pixels are exactly `rgb`. **The** assertion in this file: a second
/// copy doubles it, while the bounding box alone would not move if the copies
/// overlapped.
fn color_count(px: &[u8], rgb: (u8, u8, u8)) -> u32 {
    let mut n = 0;
    for i in (0..px.len()).step_by(4) {
        if (px[i], px[i + 1], px[i + 2]) == rgb {
            n += 1;
        }
    }
    n
}

fn color_bbox(px: &[u8], rgb: (u8, u8, u8)) -> Option<(u32, u32, u32, u32)> {
    color_bbox_in(px, VW as u32, VH as u32, rgb)
}

/// `color_bbox` over a pixmap of explicit physical dimensions (for scaled
/// rasterizations).
fn color_bbox_in(px: &[u8], w: u32, h: u32, rgb: (u8, u8, u8)) -> Option<(u32, u32, u32, u32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            if (px[i], px[i + 1], px[i + 2]) == rgb {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x0 == u32::MAX {
        None
    } else {
        Some((x0, y0, x1, y1))
    }
}

/// `<div padding:40 border:2>Press <button …></button></div>`, offset well
/// clear of the viewport edges so **neither** copy is clipped — the lead's
/// measurement got 2655 rather than 2880 because the ghost ran off the top of
/// a 600×300 canvas, and a clipped count cannot be reasoned about exactly.
fn padded_ifc_with_inline_block(button_style: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let spacer = child_of(&mut doc, body, "div", "height: 120px");
    let _ = spacer;
    let container = child_of(
        &mut doc,
        body,
        "div",
        "width: 400px; padding: 40px; border: 2px solid rgb(0, 0, 255); \
         font-size: 16px; background-color: rgb(255, 255, 255)",
    );
    let text = doc.create_text("Press ");
    doc.append_child(container, text);
    child_of(&mut doc, container, "button", button_style);
    doc
}

/// A **translucent** fill: one draw over white composites to `rgb(255,127,127)`,
/// two to `rgb(255,63,63)`. The colour, not the count, is what tells them apart
/// once #407 has moved the copies on top of each other.
const BTN: &str = "width: 60px; height: 24px; background-color: rgba(255, 0, 0, 0.5)";
/// The area of one 60×24 fill.
const ONE_DRAW: u32 = 60 * 24;
/// One 50% red over white.
const ONCE: (u8, u8, u8) = (255, 127, 127);

/// Assert `rgb` was painted **exactly once**.
///
/// Bounded on both sides, and neither bound is calibrated to the output. The
/// upper bound is exact arithmetic: one opaque 60×24 fill can match at most
/// its own area in pure `rgb` pixels, and an anti-aliased edge only ever
/// *reduces* that — so anything above the area is a second copy. The lower
/// bound catches the opposite failure, a box that stopped being drawn at all,
/// with enough slack for the blended edge columns of a fractional origin
/// (the measured draw lands at x=84.352, so one column of each edge blends).
fn assert_drawn_once(px: &[u8], _rgb: (u8, u8, u8), what: &str) {
    let once = color_count(px, ONCE);
    let twice = color_count(px, (255, 63, 63));
    assert_eq!(
        twice, 0,
        "{what}: {twice} pixels composited to rgb(255,63,63) — that is two \
         50% draws stacked. A count of the single-draw colour cannot catch \
         this once #407 puts the copies in the same place; the composited \
         value can."
    );
    assert!(
        once <= ONE_DRAW,
        "{what}: {once} pixels of the single-draw colour for a {ONE_DRAW}-pixel \
         box — two copies side by side. The composite check above only catches \
         copies that *overlap*; this catches copies that do not, and both \
         geometries occur depending on whether the offset is corrected."
    );
    assert!(
        once > ONE_DRAW * 3 / 4,
        "{what}: only {once} pixels of the single-draw colour, out of \
         {ONE_DRAW} — the box is barely drawn, or not at all"
    );
}

// ── #365: an inline-level stacking context is drawn twice ───────────────────

/// The measured repro. `position: relative` + `z-index` makes the button a
/// stacking context; it stays inline-block, because only out-of-flow
/// blockifies. The `kind != PaintKind::StackingContext` term then excludes it
/// from `already_drawn_inline`, so the stacking sequence draws it at the
/// container's **border-box** origin while `paint_inline_layout` draws it at
/// the **content** origin — two copies, one padding+border apart.
#[test]
fn an_inline_stacking_context_is_drawn_once() {
    let mut doc = padded_ifc_with_inline_block(&format!("position: relative; z-index: 1; {BTN}"));
    doc.resolve_layout(VW, VH);
    let px = rasterize(&mut doc);

    assert_drawn_once(&px, (255, 0, 0), "position: relative + z-index");
}

/// The general rule, stronger than the issue states: **no `z-index` is
/// needed.** `position: relative` alone makes an inline-block
/// `is_positioned_z_auto`, so it is `paints_at_stacking_root` and reaches the
/// same guard. `<button style="position: relative">` inside a padded paragraph
/// is the standard tooltip-anchor idiom.
#[test]
fn position_relative_alone_is_drawn_once() {
    let mut doc = padded_ifc_with_inline_block(&format!("position: relative; {BTN}"));
    doc.resolve_layout(VW, VH);
    let px = rasterize(&mut doc);

    assert_drawn_once(&px, (255, 0, 0), "position: relative alone");
}

/// `overflow: hidden` reaches it too — and per #324 that is a very common way
/// for a box to become a stacking context by accident.
#[test]
fn an_overflow_hidden_inline_block_is_drawn_once() {
    let mut doc =
        padded_ifc_with_inline_block(&format!("display: inline-block; overflow: hidden; {BTN}"));
    doc.resolve_layout(VW, VH);
    let px = rasterize(&mut doc);

    assert_drawn_once(&px, (255, 0, 0), "overflow: hidden");
}

/// The surviving draw is the **correctly positioned** one: at the IFC root's
/// content origin, inside the padding — not at its border box. A provably
/// empty region, which is what catches a ghost that a count alone could miss
/// if the two copies happened to coincide.
#[test]
fn the_surviving_draw_is_inside_the_padding() {
    let mut doc = padded_ifc_with_inline_block(&format!("position: relative; z-index: 1; {BTN}"));
    doc.resolve_layout(VW, VH);
    let px = rasterize(&mut doc);

    let (x0, y0, _, _) = color_bbox(&px, ONCE).expect("the button is painted at all");
    // The container starts at y=120 (the spacer); content begins one border +
    // one padding in on each axis.
    assert!(
        x0 >= 42,
        "the draw sits inside the container's border+padding, not at its \
         border-box origin; got x0={x0}"
    );
    assert!(y0 >= 120, "and below the spacer; got y0={y0}");
}

// ── the issue body's own repro, which the scoping pass doubted ──────────────

/// `<a style="position:relative"><img></a>` — the markup #365's body cites.
/// The `<a>` is `display: inline`, so `mark_inline_descendants` detaches it
/// from Taffy and `read_layout_results` keeps IFC positions only for
/// `InlineBlock`, leaving it 0×0 — and `paint_node`'s zero-size guard returns
/// before drawing anything. The scoping pass predicted **no second copy on
/// this markup**; this records which it is rather than leaving it asserted in
/// a comment.
#[test]
fn the_issue_bodys_inline_anchor_repro() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "width: 400px; padding: 40px; font-size: 16px",
    );
    let text = doc.create_text("Press ");
    doc.append_child(container, text);
    let anchor = child_of(&mut doc, container, "a", "position: relative");
    child_of(
        &mut doc,
        anchor,
        "div",
        &format!("display: inline-block; {BTN}"),
    );
    doc.resolve_layout(VW, VH);
    let px = rasterize(&mut doc);

    let n = color_count(&px, ONCE);
    assert!(
        n <= ONE_DRAW,
        "the inline <a> is detached and 0x0, so there is at most one draw \
         under it — got {n}, which would mean the issue body's repro does \
         double after all"
    );
}

// ── #407: the hoisted entry lands where the IFC put the box ────────────────

/// An absolutely positioned box inside an inline-block inside a **padded** IFC
/// root — the #407 fixture. The absolute is hoisted to the stacking root, and
/// the walk that positions it descends through the inline-block — whose
/// `layout.{x,y}` is relative to the IFC root's **content** box, while the
/// accumulated offset is that root's **border-box** origin. Without the
/// correction the hoisted box lands one padding+border up and to the left of
/// the span it belongs to.
fn anchored_absolute_doc(container_style: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    child_of(&mut doc, body, "div", "height: 60px");
    let container = child_of(&mut doc, body, "div", container_style);
    let text = doc.create_text("Press ");
    doc.append_child(container, text);
    let anchor = child_of(
        &mut doc,
        container,
        "span",
        "display: inline-block; position: relative; width: 80px; height: 40px; \
         background-color: rgb(0, 200, 0)",
    );
    child_of(
        &mut doc,
        anchor,
        "div",
        "position: absolute; left: 10px; top: 10px; width: 20px; height: 20px; \
         background-color: rgb(255, 0, 0)",
    );
    doc
}

/// Containment of the *painted* absolute box in the anchor's painted box,
/// which is the relationship that actually has to hold — rather than a
/// coordinate copied out of a previous run. `w`/`h` are the pixmap's physical
/// dimensions.
fn assert_hoisted_inside_anchor(px: &[u8], w: u32, h: u32, what: &str) {
    let anchor_box = color_bbox_in(px, w, h, (0, 200, 0)).expect("the inline-block is painted");
    let hoisted = color_bbox_in(px, w, h, (255, 0, 0)).expect("the absolute box is painted");
    assert!(
        hoisted.0 >= anchor_box.0
            && hoisted.1 >= anchor_box.1
            && hoisted.2 <= anchor_box.2
            && hoisted.3 <= anchor_box.3,
        "{what}: the hoisted box {hoisted:?} must land inside its anchor \
         {anchor_box:?} — `left: 10px; top: 10px` of an 80x40 anchor cannot \
         fall outside it. Landing up-and-left of the anchor is the \
         padding+border the descend walk failed to add (#407)."
    );
}

#[test]
fn a_hoisted_box_lands_inside_the_inline_block_that_anchors_it() {
    let mut doc = anchored_absolute_doc(
        "width: 400px; padding: 30px; border: 4px solid rgb(0, 0, 255); font-size: 16px",
    );
    doc.resolve_layout(VW, VH);
    let px = rasterize(&mut doc);
    assert_hoisted_inside_anchor(&px, VW as u32, VH as u32, "uniform padding");
}

/// The same relationship under **asymmetric** padding: dx = 30 + 4 = 34,
/// dy = 10 + 4 = 14. The uniform fixture above puts dx == dy — the one regime
/// where adding the x-offset to y and the y-offset to x reads identically, so
/// an axis-swap mutant of `descend`'s correction survived it (and the rest of
/// the workspace). Do not "simplify" this back to a uniform padding: the
/// asymmetry is the assertion.
#[test]
fn the_hoisted_box_lands_right_under_asymmetric_padding() {
    let mut doc = anchored_absolute_doc(
        "width: 400px; padding: 10px 50px 20px 30px; border: 4px solid rgb(0, 0, 255); \
         font-size: 16px",
    );
    doc.resolve_layout(VW, VH);
    let px = rasterize(&mut doc);
    assert_hoisted_inside_anchor(&px, VW as u32, VH as u32, "asymmetric padding");
}

/// The same relationship at paint scale 2.0. Every other test in the
/// workspace paints at scale 1.0 — the fixed point where
/// `layout.x * scale + dx` and `(layout.x + dx) * scale` agree — so only a
/// non-1 scale can tell whether `descend` adds the IFC offset inside the
/// multiply. It must: the offset is layout units, the accumulated sum is
/// physical pixels. A mutant hoisting the offset out of the `* scale`
/// survived the whole suite until this test.
#[test]
fn the_hoisted_box_lands_right_at_scale_2() {
    let mut doc = anchored_absolute_doc(
        "width: 400px; padding: 30px; border: 4px solid rgb(0, 0, 255); font-size: 16px",
    );
    doc.resolve_layout(VW, VH);
    let px = rasterize_scale(&mut doc, 2.0);
    assert_hoisted_inside_anchor(&px, VW as u32 * 2, VH as u32 * 2, "scale 2.0");
}

// ── the load-bearing term of `drawn_by_its_ifc` ─────────────────────────────

/// `drawn_by_its_ifc` skips a box only when its IFC root holds a **live**
/// inline layout (`text_layout.is_some()`). The predicate's doc calls that
/// term load-bearing — a root with no cached layout draws nothing, so
/// skipping its children would make them vanish rather than double — but
/// nothing in the workspace could tell: dropping the term survived every
/// suite. This pins it, by clearing the root's cached layout after layout
/// resolution (the state virtualization's `estimated_height` roots are in)
/// and asserting the hoisted child still reaches the screen through the
/// stacking sequence.
#[test]
fn children_of_an_ifc_root_with_no_live_layout_still_paint() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    child_of(&mut doc, body, "div", "height: 120px");
    let container = child_of(
        &mut doc,
        body,
        "div",
        "width: 400px; padding: 40px; border: 2px solid rgb(0, 0, 255); \
         font-size: 16px; background-color: rgb(255, 255, 255)",
    );
    let text = doc.create_text("Press ");
    doc.append_child(container, text);
    child_of(
        &mut doc,
        container,
        "button",
        &format!("position: relative; {BTN}"),
    );
    doc.resolve_layout(VW, VH);
    doc.tree
        .get_mut(container.0)
        .expect("the container exists")
        .text_layout = None;
    let px = rasterize(&mut doc);

    let once = color_count(&px, ONCE);
    assert!(
        once > ONE_DRAW / 2,
        "with no live layout on its IFC root, the hoisted button must still \
         be drawn by the stacking sequence — got {once} pixels of the \
         single-draw colour. Zero means `drawn_by_its_ifc` skipped a box \
         whose IFC cannot draw it."
    );
    assert!(
        once <= ONE_DRAW,
        "and drawn once, not doubled — got {once} pixels for a \
         {ONE_DRAW}-pixel box"
    );
}
