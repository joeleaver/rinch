//! Nothing outside the window is drawn — and nothing inside it is skipped.
//!
//! Card K43's second cut gives [`intersects_dirty_region`] a region that is
//! always there: the render target. A node whose box falls outside it cannot be
//! seen, and emitting it is not free — vello culls invisible paths in its coarse
//! stage, but only after flattening every one of them.
//!
//! # Why these are pixel assertions and the elision's are counts
//!
//! An elided clip paints the same picture, so only a count can see it. A culled
//! box paints *nothing*, so a pixel is exactly the right oracle — and the wrong
//! answer here is a box that has silently disappeared, which is the failure mode
//! this repo's pixel oracles exist for. Every assertion below reads a region
//! where the correct output is provably one colour or provably empty.
//!
//! # The fixed points these deliberately step off
//!
//! A box **entirely on-screen** and a box **entirely off-screen** are both
//! fixed points: a correct cull and a cull that never fires agree on the first,
//! and a correct cull and one that fires always agree on the second. Neither
//! discriminates. The cases that do are a box **straddling** the boundary and a
//! box inside the **ink margin** just past it, and both are here.
//!
//! Measured before these existed: `if true { return true; }` at the top of the
//! cull — deleting the whole optimisation — passed all 717 tests in this crate.

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 400.0;
const VH: f32 = 300.0;

fn paint(doc: &mut RinchDocument, painter: &mut TinySkiaPainter, scale: f64) {
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

fn pixel_at(p: &TinySkiaPainter, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * p.width() + x) * 4) as usize;
    let d = p.pixels();
    [d[i], d[i + 1], d[i + 2], d[i + 3]]
}

/// Painted pixels in a region — anything with a non-negligible alpha.
fn ink(p: &TinySkiaPainter, x0: u32, x1: u32, y0: u32, y1: u32) -> usize {
    let mut n = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            if pixel_at(p, x, y)[3] > 8 {
                n += 1;
            }
        }
    }
    n
}

/// A pixmap large enough to hold the window *and* a good deal past it, so that
/// "was it drawn" and "was it drawn where you can see it" stay separable.
fn painter(scale: f64) -> TinySkiaPainter {
    TinySkiaPainter::new((VW as f64 * scale) as u32, (VH as f64 * scale) as u32)
}

fn absolute_box(doc: &mut RinchDocument, style: &str) {
    let body = doc.body();
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", style);
    doc.append_child(body, d);
}

/// **The discriminating case.** A box with its top on the window and its bottom
/// past the far side of the margin band — so it crosses the very boundary the
/// cull compares against, rather than sitting wholly on one side of it.
///
/// That distinction is the test. The first version of this fixture stopped 24px
/// short of the cull rect's edge and was therefore *entirely inside* it, where a
/// correct cull and one demanding the whole box be inside give the same answer.
/// Measured: that fixture passed a mutant spelling the test
/// `x + w < cull.x1 && x > cull.x0 && …`, which loses every partially-visible
/// box on the page. This one fails it.
#[test]
fn a_box_straddling_the_bottom_edge_paints_the_part_that_is_on_screen() {
    let mut doc = RinchDocument::new();
    absolute_box(
        &mut doc,
        // 250 → 450, against a window of 300 and a cull edge at 364.
        "position: absolute; left: 50px; top: 250px; width: 100px; height: 200px; \
         background-color: rgb(255, 0, 0)",
    );
    doc.resolve_layout(VW, VH);
    let mut p = painter(1.0);
    paint(&mut doc, &mut p, 1.0);

    assert_eq!(
        ink(&p, 60, 140, 255, 295),
        80 * 40,
        "the on-window part of a box that crosses the cull boundary is not optional"
    );
}

/// The same, straddling the *right* edge. The other axis is a whole dimension
/// rather than another value of the same one, and a cull with a transposed or
/// missing comparison passes the test above and fails this.
#[test]
fn a_box_straddling_the_right_edge_paints_the_part_that_is_on_screen() {
    let mut doc = RinchDocument::new();
    absolute_box(
        &mut doc,
        // 300 → 500, against a window of 400 and a cull edge at 464.
        "position: absolute; left: 300px; top: 50px; width: 200px; height: 80px; \
         background-color: rgb(255, 0, 0)",
    );
    doc.resolve_layout(VW, VH);
    let mut p = painter(1.0);
    paint(&mut doc, &mut p, 1.0);

    assert_eq!(
        ink(&p, 305, 395, 55, 125),
        90 * 70,
        "the on-window part of a box that crosses the right cull boundary"
    );
}

/// A box wholly past the edge but inside the ink margin still paints. It is not
/// visible itself; what the margin protects is the ink it can throw *back* onto
/// the window — a shadow, an outline, a text run wider than its own box — none
/// of which is in the rect the cull tests.
///
/// 64 CSS px past the bottom at scale 1, against a 64px margin: inside, and the
/// next test is the same box one margin further out.
#[test]
fn a_box_inside_the_ink_margin_is_not_culled() {
    let mut doc = RinchDocument::new();
    absolute_box(
        &mut doc,
        "position: absolute; left: 50px; top: 340px; width: 100px; height: 20px; \
         background-color: rgb(255, 0, 0)",
    );
    doc.resolve_layout(VW, VH);
    // A pixmap tall enough to see past the window, since that is where this box
    // is: the question is whether paint emitted it, not whether it is visible.
    let mut p = TinySkiaPainter::new(VW as u32, 400);
    paint(&mut doc, &mut p, 1.0);

    assert!(
        ink(&p, 60, 140, 345, 355) > 0,
        "a box 40px past the edge is within the ink margin and must still be emitted"
    );
}

/// And the control: far enough out that nothing it can draw reaches the window.
/// Without this the two tests above would pass a cull that never fires at all.
#[test]
fn a_box_well_past_the_ink_margin_is_culled() {
    let mut doc = RinchDocument::new();
    absolute_box(
        &mut doc,
        "position: absolute; left: 50px; top: 900px; width: 100px; height: 20px; \
         background-color: rgb(255, 0, 0)",
    );
    doc.resolve_layout(VW, VH);
    let mut p = TinySkiaPainter::new(VW as u32, 1000);
    paint(&mut doc, &mut p, 1.0);

    assert_eq!(
        ink(&p, 60, 140, 905, 915),
        0,
        "a box 600px below the window has nothing to contribute and is not emitted"
    );
}

/// The control on the horizontal axis. Without it, a cull that dropped its `x`
/// comparisons entirely would still be killed by nothing: the vertical control
/// above culls its fixture on `y` alone, and the straddle tests only ever assert
/// that something *was* painted.
#[test]
fn a_box_well_past_the_right_edge_is_culled() {
    let mut doc = RinchDocument::new();
    absolute_box(
        &mut doc,
        "position: absolute; left: 900px; top: 50px; width: 100px; height: 20px; \
         background-color: rgb(255, 0, 0)",
    );
    doc.resolve_layout(VW, VH);
    let mut p = TinySkiaPainter::new(1100, VH as u32);
    paint(&mut doc, &mut p, 1.0);

    assert_eq!(
        ink(&p, 905, 995, 55, 65),
        0,
        "a box 500px to the right of the window is not emitted either"
    );
}

/// The margin is in **CSS** pixels, so it means the same thing at every scale.
/// It was a bare `96.0` in physical pixels, which is 96 CSS px on a scale-1
/// desktop and 32 on a scale-3 phone — the fixed point being `scale = 1.0`,
/// where the two spellings are indistinguishable.
///
/// This box is 40 CSS px past the edge, inside a 64 CSS px margin, at scale 3.
/// Under the physical-constant spelling the margin there is 32 CSS px and the
/// box is culled.
#[test]
fn the_ink_margin_is_the_same_size_in_css_pixels_at_every_scale() {
    let mut doc = RinchDocument::new();
    absolute_box(
        &mut doc,
        "position: absolute; left: 50px; top: 340px; width: 100px; height: 20px; \
         background-color: rgb(255, 0, 0)",
    );
    doc.resolve_layout(VW, VH);
    let mut p = TinySkiaPainter::new((VW * 3.0) as u32, 1200);
    paint(&mut doc, &mut p, 3.0);

    assert!(
        ink(&p, 180, 420, 1035, 1065) > 0,
        "40 CSS px past the edge is inside the margin whatever the DPI scale is"
    );
}

/// **A box outside says nothing about a box that is not in its coordinate
/// space.**
///
/// A `position: fixed` descendant is hoisted to its nearest stacking-context
/// ancestor and painted there in *viewport* space with zeroed offsets. When that
/// ancestor both clips and is off the window, the cull's subtree prune throws
/// away a box that is on screen — the fifth instance of #547's shape, *a walk
/// that stops early may not claim anything about what it did not visit*.
///
/// This was reachable on `main` before card K43, but only through a dirty
/// region, where the pruned root has to happen to fall outside the damaged
/// rect. The always-present render-target region is what made it fire on every
/// full repaint.
#[test]
fn a_fixed_box_survives_its_off_window_clipping_stacking_context() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let context = doc.create_element("div");
    doc.set_attribute(
        context,
        "style",
        // `z-index` makes it a stacking context, so the fixed box hoists to it
        // and no further; `overflow: hidden` is what used to make the cull
        // prune the whole subtree.
        "position: absolute; left: 0px; top: 3000px; width: 200px; height: 200px; \
         overflow: hidden; z-index: 3",
    );
    doc.append_child(body, context);

    let fixed = doc.create_element("div");
    doc.set_attribute(
        fixed,
        "style",
        "position: fixed; left: 100px; top: 60px; width: 120px; height: 100px; \
         background-color: rgb(0, 200, 0)",
    );
    doc.append_child(context, fixed);

    doc.resolve_layout(VW, VH);
    let mut p = painter(1.0);
    paint(&mut doc, &mut p, 1.0);

    assert_eq!(
        ink(&p, 110, 210, 70, 150),
        100 * 80,
        "a fixed box paints in viewport space; the box of the context that owns \
         it says nothing about whether it is on screen"
    );
}

/// The control for the case above, and the reason it is not simply "never prune
/// a clipping subtree". An off-window clipping box with ordinary in-flow content
/// — nothing hoisted, nothing in another coordinate space — is still pruned.
#[test]
fn an_off_window_clipper_with_nothing_hoisted_is_still_pruned() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let clipper = doc.create_element("div");
    doc.set_attribute(
        clipper,
        "style",
        "position: absolute; left: 0px; top: 3000px; width: 200px; height: 200px; \
         overflow: hidden",
    );
    doc.append_child(body, clipper);

    let child = doc.create_element("div");
    doc.set_attribute(
        child,
        "style",
        "width: 200px; height: 200px; background-color: rgb(255, 0, 0)",
    );
    doc.append_child(clipper, child);

    doc.resolve_layout(VW, VH);
    let mut p = TinySkiaPainter::new(VW as u32, 3400);
    paint(&mut doc, &mut p, 1.0);

    assert_eq!(
        ink(&p, 10, 190, 3010, 3190),
        0,
        "nothing in this subtree paints anywhere but inside a box that is off \
         the window, so the whole subtree is skipped"
    );
}

/// **The elision's blind spot, as a pixel.**
///
/// `paint_node` draws an IFC root's text at its *content* origin
/// (`ifc_root_content_origin` — padding plus border), and the first version of
/// the fit test measured it from the *border-box* origin. A `padding-left`
/// wider than the gap between the text and the box's right edge therefore read
/// as fitting, the clip was elided, and the text painted outside it.
///
/// Measured here: the whole 75px run starts at the clip's right edge, so a
/// correct clip removes all of it and the region past the box is provably
/// empty. Before the fit test was moved onto `layer_bounds` — which applies the
/// content origin at both places paint does — this region held 268 ink pixels.
#[test]
fn a_padded_ifc_roots_text_does_not_escape_through_an_elided_clip() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let c = doc.create_element("div");
    doc.set_attribute(
        c,
        "style",
        "position: absolute; left: 0; top: 0; width: 40px; padding-left: 100px; \
         overflow: hidden; color: rgb(255, 0, 0); font-size: 20px; line-height: 24px",
    );
    doc.append_child(body, c);
    let t = doc.create_text("MMMM");
    doc.append_child(c, t);

    doc.resolve_layout(VW, VH);
    let mut p = painter(1.0);
    paint(&mut doc, &mut p, 1.0);

    assert_eq!(
        ink(&p, 102, 260, 0, 40),
        0,
        "the text starts at the clip's right edge, so every pixel of it is cut"
    );
}

/// `paint_subtree` paints into a pixmap of its own, sized to one element, and
/// never sets the render target. So `paint_document` has to **clear** it rather
/// than leave it behind: otherwise the drag-ghost snapshot is culled against
/// whatever window the previous frame happened to have.
///
/// The fixture is a subtree whose root sits at the origin — where
/// `paint_subtree` puts it — with a descendant far enough below to be past the
/// last frame's cull rect. Correct output paints it; a stale target does not.
///
/// Deleting the clear is otherwise **silent**: it kills nothing else in the
/// crate, because `paint_document` re-sets the target on every call and
/// `paint_subtree` is the only other entry point into `paint_node`.
#[test]
fn paint_subtree_does_not_cull_against_the_last_frames_window() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    // In flow, both of them: an out-of-flow descendant would be hoisted into the
    // body's stacking sequence and `paint_subtree` would not draw it at all,
    // which would make this pass for the wrong reason.
    let root = doc.create_element("div");
    doc.set_attribute(root, "style", "width: 200px; height: 1000px");
    doc.append_child(body, root);

    // A spacer rather than a margin: the child's top margin would collapse into
    // its parent's and move the parent instead, leaving the box at the top.
    let spacer = doc.create_element("div");
    doc.set_attribute(spacer, "style", "width: 100px; height: 900px");
    doc.append_child(root, spacer);

    let far = doc.create_element("div");
    doc.set_attribute(
        far,
        "style",
        "width: 100px; height: 40px; background-color: rgb(0, 0, 255)",
    );
    doc.append_child(root, far);

    doc.resolve_layout(VW, VH);

    // A full document paint first, exactly as a real frame would run — this is
    // what leaves a 400x300 target behind if it is not cleared.
    let mut frame = painter(1.0);
    paint(&mut doc, &mut frame, 1.0);

    // Then the snapshot, which has no window of its own.
    let mut snapshot = TinySkiaPainter::new(200, 1000);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_subtree(
        &doc.tree,
        &mut snapshot,
        root.0,
        1.0,
        &mut doc.font_cx,
        &mut layout_cx,
    );

    assert!(
        ink(&snapshot, 10, 90, 905, 935) > 0,
        "a subtree snapshot paints its whole subtree; the window the last frame \
         used has nothing to do with the pixmap this one draws into"
    );
}
