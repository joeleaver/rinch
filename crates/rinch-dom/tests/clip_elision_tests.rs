//! A clip layer that provably removes no pixel is not pushed — card K43.
//!
//! Vello implements every clip as a blend-stack layer: for each 16x16 tile the
//! clip's bounding box touches, the fine stage saves the tile's pixels on the
//! way in and blends them back on the way out, whatever is drawn between. That
//! is two extra passes over the clipped area, paid whether or not the clip
//! removes anything. On the library screen of the app this came from, 39 of the
//! 41 clips in a frame removed nothing at all, and dropping them was 4.0ms of a
//! 17.8ms frame on an Adreno 619.
//!
//! **These tests exist because the optimisation is invisible in the pixels.**
//! An elided clip that was correct to elide paints exactly the picture the clip
//! would have painted — that is the definition of eliding it — so a pixel
//! oracle can prove the elision is *safe* and can never prove it *happened*.
//! `Clips` counts the `push_clip` calls instead, which is the only place the
//! difference is observable, and pairs each elision with a fixture that must
//! still push so that "never push anything" cannot pass.
//!
//! The rounded case is here because it was a real defect and not a hypothetical
//! one. "Nothing inside reaches past the box" does not mean "nothing is cut"
//! when the clip shape is a *rounded* box: the corners are cut from the box
//! itself, so the shape is smaller than the rect the fit was measured against.
//! The commit this came from tested the radius on the covers-the-window arm
//! only — where it reads as being about the bounding box — and a
//! `border-radius: 40px; overflow: hidden` box with a child exactly its own
//! size stopped cutting its corners.

use peniko::kurbo::{Affine, Stroke};
use peniko::{Brush, Fill, FontData};
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::{BlendMode, PaintGlyph, PaintImage, PaintShape, Painter};

/// A painter that draws nothing and counts the one call this is about.
///
/// `push_layer` is counted separately and deliberately: an opacity layer is not
/// a clip, and conflating the two would let an opacity regression read as a
/// clip elision.
#[derive(Default)]
struct Clips {
    clips: usize,
    layers: usize,
    pops: usize,
}

impl Painter for Clips {
    fn reset(&mut self) {
        self.clips = 0;
        self.layers = 0;
        self.pops = 0;
    }

    fn fill(&mut self, _: Fill, _: Affine, _: &Brush, _: &PaintShape) {}

    fn stroke(&mut self, _: &Stroke, _: Affine, _: &Brush, _: &PaintShape) {}

    #[allow(clippy::too_many_arguments)]
    fn draw_glyphs(
        &mut self,
        _: &FontData,
        _: f32,
        _: Affine,
        _: Option<Affine>,
        _: &Brush,
        _: bool,
        _: &[i16],
        _: &[PaintGlyph],
    ) {
    }

    fn draw_image(&mut self, _: &PaintImage<'_>, _: Affine) {}

    fn push_clip(&mut self, _: Fill, _: Affine, _: &PaintShape) {
        self.clips += 1;
    }

    fn push_layer(&mut self, _: BlendMode, _: f32, _: Affine, _: &PaintShape) {
        self.layers += 1;
    }

    fn pop_layer(&mut self) {
        self.pops += 1;
    }
}

impl Clips {
    /// Every push has to be matched, whether it was a clip or an opacity layer.
    /// A test that only counted pushes would pass a change that pushed
    /// correctly and popped twice.
    fn balanced(&self) -> bool {
        self.clips + self.layers == self.pops
    }
}

/// The render target every fixture below is painted into. A box of exactly this
/// size is the "clip covers the window" case; anything smaller is not.
const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// Paint `container_style` holding one child of `child_style`, and count.
fn count(container_style: &str, child_style: &str) -> Clips {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(container, "style", container_style);
    doc.append_child(body, container);
    let child = doc.create_element("div");
    doc.set_attribute(child, "style", child_style);
    doc.append_child(container, child);
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);

    let mut painter = Clips::default();
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        VIEWPORT,
        &mut doc.font_cx,
        &mut layout_cx,
    );
    assert!(painter.balanced(), "pushes and pops must match");
    painter
}

/// **The document root is itself an elided clip, and every count below sits on
/// top of that.** rinch's root pushes one clip over the whole window; it is the
/// covers-the-window case in its purest form, and it is the 1.2ms the app this
/// came from was paying to copy the screen out and copy it back. So a fixture
/// whose own clip is elided counts **0**, not 1, and this test is what says so
/// — without it the zeroes below could be read as the container never having
/// clipped at all.
#[test]
fn the_document_root_clip_over_the_whole_window_is_not_pushed() {
    let c = count("width: 100px; height: 100px", "width: 50px; height: 50px");
    assert_eq!(
        c.clips, 0,
        "nothing here clips but the root, and the root covers the window"
    );
}

/// A clip whose rectangle contains the whole render target can only remove
/// pixels that are off the window and discarded anyway.
///
/// The child overflows on purpose, so `subtree_fits` answers "no" and this
/// fixture can only be elided by the covers-the-window arm. Its control is
/// `a_clip_one_pixel_short_of_the_window_is_pushed`, which differs in nothing
/// else.
#[test]
fn a_clip_that_contains_the_window_is_not_pushed() {
    let c = count(
        "width: 800px; height: 600px; overflow: hidden",
        "width: 2000px; height: 2000px",
    );
    assert_eq!(
        c.clips, 0,
        "a clip over the whole 800x600 target removes nothing that is drawn"
    );
}

/// The control for the case above. One pixel short of the window in each
/// direction is a clip that really does cut something visible, so it is pushed
/// — which is what makes the zero above a statement about *covering the
/// window* rather than about big boxes.
#[test]
fn a_clip_one_pixel_short_of_the_window_is_pushed() {
    let c = count(
        "width: 799px; height: 599px; overflow: hidden",
        "width: 2000px; height: 2000px",
    );
    assert_eq!(
        c.clips, 1,
        "the child overhangs into pixels that are on screen"
    );
}

/// A box nothing inside reaches past has nothing left to clip.
#[test]
fn a_clip_whose_subtree_fits_is_not_pushed() {
    let c = count(
        "width: 100px; height: 100px; overflow: hidden",
        "width: 50px; height: 50px",
    );
    assert_eq!(c.clips, 0, "the child is inside the box on every side");
}

/// The control for the case above, differing only in the size of the child:
/// a clip that has work to do is still pushed. Without this, a change that
/// stopped clipping entirely would go green.
#[test]
fn a_clip_with_something_to_cut_is_still_pushed() {
    let c = count(
        "width: 100px; height: 100px; overflow: hidden",
        "width: 200px; height: 200px",
    );
    assert_eq!(
        c.clips, 1,
        "the child overhangs the box, so the clip is real"
    );
}

/// **The regression this pair of cases actually cost.**
///
/// A rounded clip cuts the corners of its own box, so a subtree that fits the
/// box is still cut by the shape. Fitting is not a reason to elide it.
#[test]
fn a_rounded_clip_whose_subtree_fits_is_still_pushed() {
    let c = count(
        "width: 100px; height: 100px; border-radius: 40px; overflow: hidden",
        "width: 100px; height: 100px",
    );
    assert_eq!(
        c.clips, 1,
        "the child fits the box and is still cut by the corners"
    );
}

/// And the same for a rounded box that covers the window: the corners are cut
/// from pixels that are on screen, so this one is not the discarded-anyway case
/// either.
#[test]
fn a_rounded_clip_over_the_whole_window_is_still_pushed() {
    let c = count(
        "width: 800px; height: 600px; border-radius: 40px; overflow: hidden",
        "width: 2000px; height: 2000px",
    );
    assert_eq!(c.clips, 1, "the window's own corners are visible pixels");
}

// ---------------------------------------------------------------------------
// Off the fixed points.
//
// Every fixture above sits on five at once: `scale = 1.0`, scroll offset `0`,
// the container at the window's own origin, exactly **one** child, and no text
// anywhere. The last is the one that cost something. The optimisation was
// written for ellipsised row titles — `overflow: hidden` over a string the
// ellipsis has already shortened — and measured against the suite as it stood,
// **deleting the fit test's entire text branch left all 717 tests green**. The
// defect that lived in it was real: an IFC root's text is drawn at its content
// origin and the branch measured it from the border box, so a padded box elided
// a clip its own text was entirely outside.
//
// `offscreen_cull_tests::a_padded_ifc_roots_text_does_not_escape_through_an_elided_clip`
// is that case as a pixel. These are the counting half.
// ---------------------------------------------------------------------------

/// Build whatever document you like, paint it at `scale`, and count.
fn count_doc(build: impl FnOnce(&mut RinchDocument), scale: f64) -> Clips {
    let mut doc = RinchDocument::new();
    build(&mut doc);
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);

    let mut painter = Clips::default();
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale,
        VIEWPORT,
        &mut doc.font_cx,
        &mut layout_cx,
    );
    assert!(painter.balanced(), "pushes and pops must match");
    painter
}

fn div(
    doc: &mut RinchDocument,
    parent: rinch_core::dom::NodeId,
    style: &str,
) -> rinch_core::dom::NodeId {
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", style);
    doc.append_child(parent, d);
    d
}

/// The motivating case, as a count: a title box whose text fits it. This is the
/// row title the optimisation exists for, and the elision it is claiming.
#[test]
fn a_clip_whose_text_fits_it_is_not_pushed() {
    let c = count_doc(
        |doc| {
            let body = doc.body();
            let title = div(
                doc,
                body,
                "width: 300px; height: 24px; overflow: hidden; font-size: 16px",
            );
            let t = doc.create_text("Track title");
            doc.append_child(title, t);
        },
        1.0,
    );
    assert_eq!(
        c.clips, 0,
        "the run is narrower than the box, so nothing is cut"
    );
}

/// **The control, and the test that was missing.** The same box with a run too
/// wide for it. Deleting the fit test's text branch makes this fixture elide,
/// and nothing else in the crate notices.
#[test]
fn a_clip_whose_text_overflows_it_is_still_pushed() {
    let c = count_doc(
        |doc| {
            let body = doc.body();
            let title = div(
                doc,
                body,
                "width: 60px; height: 24px; overflow: hidden; font-size: 16px",
            );
            let t = doc.create_text("MMMMMMMMMMMMMMMMMMMM");
            doc.append_child(title, t);
        },
        1.0,
    );
    assert_eq!(
        c.clips, 1,
        "an unbreakable run wider than its box really is cut"
    );
}

/// The same question with the overflow coming from the container's **padding**
/// rather than from the run being long. Paint draws the text at the content
/// origin; a fit test that measures from the border-box origin says this fits.
#[test]
fn a_padded_clip_whose_text_starts_past_its_edge_is_still_pushed() {
    let c = count_doc(
        |doc| {
            let body = doc.body();
            let title = div(
                doc,
                body,
                "width: 40px; padding-left: 100px; overflow: hidden; font-size: 20px",
            );
            let t = doc.create_text("MMMM");
            doc.append_child(title, t);
        },
        1.0,
    );
    assert_eq!(
        c.clips, 1,
        "the run begins where the clip ends, so the clip removes all of it"
    );
}

/// Scroll offset `0` is a fixed point: at zero, a fit test that forgot the
/// scroll offset and one that applies it agree. A scrolled container whose
/// content has moved *up* out of the box is the case that separates them.
#[test]
fn a_scrolled_clip_whose_content_has_moved_out_of_the_box_is_still_pushed() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let scroller = div(
        &mut doc,
        body,
        "width: 200px; height: 100px; overflow-y: auto",
    );
    div(&mut doc, scroller, "width: 200px; height: 400px");
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);

    // Scroll far enough that the child's top edge is well above the box.
    doc.tree.get_mut(scroller.0).unwrap().scroll_offset.1 = 150.0;

    let mut painter = Clips::default();
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        VIEWPORT,
        &mut doc.font_cx,
        &mut layout_cx,
    );
    assert!(painter.balanced(), "pushes and pops must match");
    assert_eq!(
        painter.clips, 1,
        "a scrolled child hangs off both ends of its scroller"
    );
}

/// `scale = 1.0` is a fixed point for anything that multiplies by it. The
/// covers-the-window arm compares a scaled box against a scaled target, so the
/// two have to be scaled the same way — at scale 1 a missing multiplication and
/// a present one are the same program.
#[test]
fn the_covers_the_window_arm_holds_at_a_non_unit_scale() {
    let c = count_doc(
        |doc| {
            let body = doc.body();
            div(doc, body, "width: 800px; height: 600px; overflow: hidden");
        },
        2.0,
    );
    assert_eq!(
        c.clips, 0,
        "an 800x600 box over an 800x600 logical window covers the target at any DPI"
    );
}

/// And its control at the same scale: one logical pixel short is still short
/// when both sides are multiplied by two.
#[test]
fn a_clip_short_of_the_window_is_pushed_at_a_non_unit_scale() {
    let c = count_doc(
        |doc| {
            let body = doc.body();
            let clipper = div(doc, body, "width: 799px; height: 599px; overflow: hidden");
            div(doc, clipper, "width: 2000px; height: 2000px");
        },
        2.0,
    );
    assert_eq!(
        c.clips, 1,
        "the child overhangs into pixels that are on screen"
    );
}

/// Cardinality 1 is a fixed point of its own: with a single child, nothing
/// order-dependent in the layer stack is exercised and "checks every child" and
/// "checks the first child" are the same program. Two siblings, the overflowing
/// one **last**, so a fit test that stops at the first answer is caught.
#[test]
fn a_clip_is_pushed_when_the_second_of_two_children_overflows() {
    let c = count_doc(
        |doc| {
            let body = doc.body();
            let clipper = div(doc, body, "width: 200px; height: 300px; overflow: hidden");
            div(doc, clipper, "width: 100px; height: 50px");
            div(doc, clipper, "width: 400px; height: 50px");
        },
        1.0,
    );
    assert_eq!(
        c.clips, 1,
        "the overflow is in the second child, not the first"
    );
}

/// The same pair with the overflow in the **first** child, which is the
/// direction a short-circuit gets right by accident. Both are needed: with only
/// this one, a fit test that looked at nothing but `children[0]` would pass.
#[test]
fn a_clip_is_pushed_when_the_first_of_two_children_overflows() {
    let c = count_doc(
        |doc| {
            let body = doc.body();
            let clipper = div(doc, body, "width: 200px; height: 300px; overflow: hidden");
            div(doc, clipper, "width: 400px; height: 50px");
            div(doc, clipper, "width: 100px; height: 50px");
        },
        1.0,
    );
    assert_eq!(
        c.clips, 1,
        "the overflow is in the first child, not the second"
    );
}

/// A grandchild, not a child. The fit test is a walk and this is the cheapest
/// statement that it is one: the direct child fits its parent exactly, and the
/// thing that overflows is a level below.
#[test]
fn a_clip_is_pushed_when_a_grandchild_overflows() {
    let c = count_doc(
        |doc| {
            let body = doc.body();
            let clipper = div(doc, body, "width: 200px; height: 200px; overflow: hidden");
            let inner = div(doc, clipper, "width: 200px; height: 200px");
            div(doc, inner, "width: 500px; height: 50px");
        },
        1.0,
    );
    assert_eq!(c.clips, 1, "the overflow is two levels down");
}

/// A clipping child bounds everything under it, so the walk may stop there —
/// and must not report the grandchild it never looked at as an overflow. This
/// is the elision the walk's `clipped_to` earns; without it a scroller inside a
/// card would keep the card's own redundant clip alive.
#[test]
fn a_grandchild_bounded_by_a_clipping_child_does_not_keep_the_outer_clip() {
    let c = count_doc(
        |doc| {
            let body = doc.body();
            let outer = div(doc, body, "width: 200px; height: 200px; overflow: hidden");
            let inner = div(doc, outer, "width: 200px; height: 200px; overflow: hidden");
            div(doc, inner, "width: 500px; height: 500px");
        },
        1.0,
    );
    assert_eq!(
        c.clips, 1,
        "the inner clip is real and the outer one has nothing left to do"
    );
}

/// A `position: fixed` descendant is painted in viewport space, not in this
/// box's, so a fit test may not conclude anything about it from its layout
/// position. The walk answers `Escapes` and the clip stays.
#[test]
fn a_fixed_descendant_keeps_the_clip() {
    let c = count_doc(
        |doc| {
            let body = doc.body();
            let clipper = div(doc, body, "width: 200px; height: 200px; overflow: hidden");
            div(
                doc,
                clipper,
                "position: fixed; left: 10px; top: 10px; width: 20px; height: 20px",
            );
        },
        1.0,
    );
    assert_eq!(
        c.clips, 1,
        "a fixed box's coordinates are the viewport's; nothing here can place it"
    );
}

/// A child's `box-shadow` reaches past its own layout box, and the box it
/// reaches into is the one being asked about. The walk models the shadow's
/// extent (`blur + spread`), so this is a statement that the fit test reads a
/// painted extent rather than a layout rect.
#[test]
fn a_childs_shadow_reaching_past_the_box_keeps_the_clip() {
    let c = count_doc(
        |doc| {
            let body = doc.body();
            let clipper = div(doc, body, "width: 200px; height: 200px; overflow: hidden");
            div(
                doc,
                clipper,
                "width: 200px; height: 200px; box-shadow: 0px 0px 40px rgb(0, 0, 0)",
            );
        },
        1.0,
    );
    assert_eq!(
        c.clips, 1,
        "the child fits, its shadow does not, and the clip is what removes it"
    );
}

/// **Pinning a guard, not fixing a defect.** `clip_cuts_nothing` answers
/// `false` for `Extent::Unknown` as well as for `Extent::Escapes`, and the two
/// arms were pinned as one — a mutant flipping both is killed by its `Escapes`
/// half alone, so nothing stood between the `Unknown` half and green.
///
/// `Unknown` has exactly one producer: a `position: sticky` descendant, whose
/// painted position `paint_node` derives by walking *up* to the nearest scroll
/// ancestor, so the subtree does not contain the answer. Here that descendant
/// fits its container's box, so a fit test that read `Unknown` as "fits" would
/// elide the clip.
///
/// **No pixel divergence has been constructed for that mutant, and this test
/// does not claim one.** A sticky box creates a stacking context, so it is
/// hoisted and carries its clipping ancestors in its own #324 clip chain — the
/// chain re-applies the clip the bracket would have applied, which is very
/// likely why the elision is invisible. Two answers to one question agreeing
/// today is not a reason to let one of them drift: the guard says a
/// not-knowing is never a fit, and this is what says so.
#[test]
fn a_sticky_descendant_keeps_the_clip() {
    let c = count_doc(
        |doc| {
            let body = doc.body();
            let clipper = div(doc, body, "width: 200px; height: 200px; overflow: hidden");
            div(
                doc,
                clipper,
                "position: sticky; top: 0px; width: 100px; height: 40px",
            );
        },
        1.0,
    );
    assert_eq!(
        c.clips, 2,
        "the clipper's own bracket, plus the clip the hoisted sticky entry \
         carries in its chain — a not-knowing is never a fit"
    );
}
