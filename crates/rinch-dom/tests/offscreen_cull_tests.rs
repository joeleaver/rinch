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

// ---------------------------------------------------------------------------
// #562: the prune is gated on what the subtree PAINTS, not on the node's box.
// ---------------------------------------------------------------------------

/// A painter that draws nothing and counts the group layers it is asked for.
///
/// The pixel oracles below can prove an off-window sheet is invisible; they
/// cannot prove it was *cheap*, because a layer that is composited back with
/// nothing on screen in it leaves no pixel either way. In the software painter
/// a `push_layer` is a whole-surface pixmap allocated, filled and composited —
/// the entire cost #562 is about — so counting the pushes is the only place the
/// difference is observable.
#[derive(Default)]
struct Layers {
    layers: usize,
    pops: usize,
}

impl rinch_dom::paint::painter::Painter for Layers {
    fn reset(&mut self) {
        self.layers = 0;
        self.pops = 0;
    }
    fn fill(
        &mut self,
        _: peniko::Fill,
        _: peniko::kurbo::Affine,
        _: &Brush,
        _: &rinch_dom::paint::painter::PaintShape,
    ) {
    }
    fn stroke(
        &mut self,
        _: &peniko::kurbo::Stroke,
        _: peniko::kurbo::Affine,
        _: &Brush,
        _: &rinch_dom::paint::painter::PaintShape,
    ) {
    }
    #[allow(clippy::too_many_arguments)]
    fn draw_glyphs(
        &mut self,
        _: &peniko::FontData,
        _: f32,
        _: peniko::kurbo::Affine,
        _: Option<peniko::kurbo::Affine>,
        _: &Brush,
        _: bool,
        _: &[i16],
        _: &[rinch_dom::paint::painter::PaintGlyph],
    ) {
    }
    fn draw_image(
        &mut self,
        _: &rinch_dom::paint::painter::PaintImage<'_>,
        _: peniko::kurbo::Affine,
    ) {
    }
    fn push_clip(
        &mut self,
        _: peniko::Fill,
        _: peniko::kurbo::Affine,
        _: &rinch_dom::paint::painter::PaintShape,
    ) {
    }
    fn push_layer(
        &mut self,
        _: rinch_dom::paint::painter::BlendMode,
        _: f32,
        _: peniko::kurbo::Affine,
        _: &rinch_dom::paint::painter::PaintShape,
    ) {
        self.layers += 1;
    }
    fn pop_layer(&mut self) {
        self.pops += 1;
    }
}

fn count_layers(doc: &mut RinchDocument) -> Layers {
    let mut p = Layers::default();
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut p,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    p
}

/// A full-screen translucent sheet parked below the fold, holding ordinary
/// in-flow content — the shape #562 was filed for.
fn off_window_sheet(doc: &mut RinchDocument, sheet_style: &str) {
    let body = doc.body();
    let sheet = doc.create_element("div");
    doc.set_attribute(sheet, "style", sheet_style);
    doc.append_child(body, sheet);
    let row = doc.create_element("div");
    doc.set_attribute(
        row,
        "style",
        "width: 400px; height: 200px; background-color: rgb(255, 0, 0)",
    );
    doc.append_child(sheet, row);
}

/// **The regression #562 reports, and it is a count because it cannot be a
/// pixel.** An `opacity: 0.5` sheet parked below the fold has nothing hoisted
/// in it and nothing that paints anywhere but inside its own off-window box,
/// so the whole subtree is skipped — and the skip has to happen *before* the
/// group layer is opened, or it has saved nothing at all.
///
/// A pixel oracle cannot see this. The sheet's own children are off-window too,
/// so the inner per-node cull already discards each of them and the composited
/// layer comes back empty either way: a 400x3400 pixmap sampled at the sheet's
/// own address reads 0 ink with the prune and 0 ink without it. What the prune
/// actually saves is the layer itself, which in the software painter is a
/// whole-surface pixmap allocated, filled and composited back — 26.7ms against
/// 0.8ms on three such sheets at 1080x2460. `push_layer` is the only place that
/// is observable.
#[test]
fn an_off_window_opacity_sheet_opens_no_group_layer() {
    let mut doc = RinchDocument::new();
    off_window_sheet(
        &mut doc,
        "position: absolute; left: 0px; top: 3000px; width: 400px; height: 300px; \
         opacity: 0.5",
    );
    doc.resolve_layout(VW, VH);
    let counted = count_layers(&mut doc);
    assert_eq!(
        (counted.layers, counted.pops),
        (0, 0),
        "the sheet is pruned before its opacity layer is pushed"
    );
}

/// The control for the pair above, and what stops "never open a layer" from
/// passing them: the very same sheet **on** the window still composites.
#[test]
fn an_on_window_opacity_sheet_still_opens_its_group_layer() {
    let mut doc = RinchDocument::new();
    off_window_sheet(
        &mut doc,
        "position: absolute; left: 0px; top: 0px; width: 400px; height: 300px; \
         opacity: 0.5",
    );
    doc.resolve_layout(VW, VH);
    let counted = count_layers(&mut doc);
    assert_eq!(
        (counted.layers, counted.pops),
        (1, 1),
        "an on-window translucent sheet is composited as it always was"
    );
}

/// **The counterexample that killed #562's own proposal.** An off-window
/// **non-clipping** `opacity: 0.5` stacking context owning an on-screen fixed
/// box: the fixed box is painted *inside* that group layer, so it must come out
/// at the sheet's opacity and not at full strength.
///
/// Asserting the **alpha** and not merely the presence is the whole point.
/// #562's proposed `clips_overflow()` narrowing sends this node down the
/// skip-draw-and-recurse arm, which sits before every `push_layer` in the
/// function: the box still paints, at `[0, 200, 0, 255]` instead of
/// `[0, 100, 0, 128]`. Every one of the 951 tests in this crate stayed green
/// for it.
#[test]
fn an_off_window_opacity_layer_keeps_its_fixed_descendant_faded() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let sheet = doc.create_element("div");
    doc.set_attribute(
        sheet,
        "style",
        "position: absolute; left: 0px; top: 3000px; width: 200px; height: 200px; \
         opacity: 0.5",
    );
    doc.append_child(body, sheet);

    let fixed = doc.create_element("div");
    doc.set_attribute(
        fixed,
        "style",
        "position: fixed; left: 100px; top: 60px; width: 120px; height: 100px; \
         background-color: rgb(0, 200, 0)",
    );
    doc.append_child(sheet, fixed);

    doc.resolve_layout(VW, VH);
    let mut p = painter(1.0);
    paint(&mut doc, &mut p, 1.0);

    assert_eq!(
        pixel_at(&p, 150, 100),
        [0, 100, 0, 128],
        "the fixed box is painted through its owner's opacity layer, and the \
         owner's own box being off-window says nothing about either"
    );
}

/// The same, with the fixed box **two levels down**, so the answer has to come
/// from a walk of the subtree rather than from a glance at the direct children.
#[test]
fn a_fixed_box_nested_inside_an_off_window_stacking_context_still_paints() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let sheet = doc.create_element("div");
    doc.set_attribute(
        sheet,
        "style",
        "position: absolute; left: 0px; top: 3000px; width: 200px; height: 200px; \
         opacity: 0.5",
    );
    doc.append_child(body, sheet);

    let mid = doc.create_element("div");
    doc.set_attribute(mid, "style", "width: 200px; height: 50px");
    doc.append_child(sheet, mid);

    let inner = doc.create_element("div");
    doc.set_attribute(inner, "style", "width: 200px; height: 50px");
    doc.append_child(mid, inner);

    let fixed = doc.create_element("div");
    doc.set_attribute(
        fixed,
        "style",
        "position: fixed; left: 100px; top: 60px; width: 120px; height: 100px; \
         background-color: rgb(0, 200, 0)",
    );
    doc.append_child(inner, fixed);

    doc.resolve_layout(VW, VH);
    let mut p = painter(1.0);
    paint(&mut doc, &mut p, 1.0);

    assert_eq!(
        pixel_at(&p, 150, 100),
        [0, 100, 0, 128],
        "a fixed box two levels inside an off-window stacking context is still \
         painted, and still faded"
    );
}

/// **The extent is what is asked, not the box.** An off-window stacking context
/// whose ordinary in-flow child reaches back onto the window — here by a
/// negative margin — paints that child.
///
/// The node's own border box is off-window in both this fixture and the pruned
/// one above, so a gate that reads the box alone cannot tell them apart.
#[test]
fn an_off_window_stacking_context_whose_child_reaches_the_window_is_not_pruned() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let sheet = doc.create_element("div");
    doc.set_attribute(
        sheet,
        "style",
        "position: absolute; left: 0px; top: 3000px; width: 200px; height: 200px; \
         opacity: 0.5",
    );
    doc.append_child(body, sheet);

    let reaching = doc.create_element("div");
    doc.set_attribute(
        reaching,
        "style",
        "margin-top: -2940px; margin-left: 40px; width: 120px; height: 100px; \
         background-color: rgb(0, 200, 0)",
    );
    doc.append_child(sheet, reaching);

    doc.resolve_layout(VW, VH);
    let mut p = painter(1.0);
    paint(&mut doc, &mut p, 1.0);

    assert_eq!(
        pixel_at(&p, 100, 100),
        [0, 100, 0, 128],
        "the child paints on the window, so the subtree that owns it is not \
         off-window whatever its own box says"
    );
}

/// **#550's hole, as a cull.** `stacking::Collector::span` truncates an
/// absolute's clip chain at its containing block, so an absolute escapes any
/// clipper *below* that block — while a subtree walk narrows it at every
/// clipping ancestor it descends through. A prune keyed on an under-measured
/// extent would discard this on-screen box.
///
/// The shape: an off-window positioned stacking context (the containing block),
/// a **static** `overflow: hidden` box inside it (not in the absolute's chain),
/// and an absolute a level below *that*, placed back on the window.
#[test]
fn an_absolute_escaping_a_clipper_below_its_containing_block_survives_the_cull() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let context = doc.create_element("div");
    doc.set_attribute(
        context,
        "style",
        "position: absolute; left: 0px; top: 3000px; width: 200px; height: 200px; \
         z-index: 3",
    );
    doc.append_child(body, context);

    let clipper = doc.create_element("div");
    doc.set_attribute(
        clipper,
        "style",
        "width: 200px; height: 40px; overflow: hidden",
    );
    doc.append_child(context, clipper);

    // One level between the clipper and the absolute, so the clip has to be
    // carried *down* the descent rather than noticed at the parent.
    let inner = doc.create_element("div");
    doc.set_attribute(inner, "style", "width: 200px");
    doc.append_child(clipper, inner);

    let escaping = doc.create_element("div");
    doc.set_attribute(
        escaping,
        "style",
        "position: absolute; left: 40px; top: -2940px; width: 120px; height: 100px; \
         background-color: rgb(0, 200, 0)",
    );
    doc.append_child(inner, escaping);

    doc.resolve_layout(VW, VH);
    let mut p = painter(1.0);
    paint(&mut doc, &mut p, 1.0);

    assert_eq!(
        ink(&p, 45, 155, 65, 155),
        110 * 90,
        "paint does not clip this absolute by a static clipper below its \
         containing block, so nothing that measures the subtree may assume it \
         is bounded by one"
    );
}

/// **The other not-knowing, and it is not `Escapes`.** `paint_node` places a
/// `position: sticky` box by walking *up* to its nearest scroll ancestor, which
/// may be above the subtree being measured — so a walk of the subtree alone
/// cannot say where the box lands, and answers `Extent::Unknown`.
///
/// Here the stacking context is parked 500px above the window and its sticky
/// child is painted at `y = 100`, on it. The large `top` is only what makes the
/// divergence visible in a unit fixture; the mechanism is the ancestor walk,
/// and a scrolled container reaches the same place with an ordinary `top: 0`.
///
/// A gate that treated `Unknown` as prunable — the obvious simplification,
/// since `Extent::clipped_to` narrows it happily everywhere else — deletes this
/// box.
#[test]
fn an_off_window_stacking_context_with_a_sticky_descendant_is_not_pruned() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let sheet = doc.create_element("div");
    doc.set_attribute(
        sheet,
        "style",
        "position: absolute; left: 0px; top: -500px; width: 200px; height: 200px; \
         opacity: 0.5",
    );
    doc.append_child(body, sheet);

    let stuck = doc.create_element("div");
    doc.set_attribute(
        stuck,
        "style",
        "position: sticky; top: 600px; margin-left: 40px; width: 120px; height: 100px; \
         background-color: rgb(0, 200, 0)",
    );
    doc.append_child(sheet, stuck);

    doc.resolve_layout(VW, VH);
    let mut p = painter(1.0);
    paint(&mut doc, &mut p, 1.0);

    assert_eq!(
        pixel_at(&p, 100, 150),
        [0, 100, 0, 128],
        "a sticky box is placed by an ancestor walk, so the subtree that holds \
         it does not contain the answer and may not be pruned on one"
    );
}

/// **The extent comes back in the node's own untransformed space**, like the
/// bounds `layer_bounds` hands `push_layer`, so the cull has to apply the
/// node's composed transform to it before comparing — exactly as the box test
/// three lines above it already does (#143).
///
/// Both boxes here are off-window *after* the transform except the child, which
/// the transform carries back to the top of the window. A comparison made in
/// the wrong space finds the subtree at `y = 2000` and deletes it.
#[test]
fn the_subtree_extent_is_compared_in_screen_space_not_layout_space() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let sheet = doc.create_element("div");
    doc.set_attribute(
        sheet,
        "style",
        "position: absolute; left: 0px; top: 3000px; width: 200px; height: 200px; \
         opacity: 0.5; transform: translateY(-2000px)",
    );
    doc.append_child(body, sheet);

    let reaching = doc.create_element("div");
    doc.set_attribute(
        reaching,
        "style",
        "margin-top: -1000px; margin-left: 40px; width: 120px; height: 100px; \
         background-color: rgb(0, 200, 0)",
    );
    doc.append_child(sheet, reaching);

    doc.resolve_layout(VW, VH);
    let mut p = painter(1.0);
    paint(&mut doc, &mut p, 1.0);

    assert_eq!(
        pixel_at(&p, 100, 50),
        [0, 100, 0, 128],
        "the child lands at the top of the window once the sheet's transform is \
         applied; a cull that compares the untransformed extent loses it"
    );
}
