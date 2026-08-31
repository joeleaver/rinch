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
    assert_eq!(c.clips, 1, "the child overhangs into pixels that are on screen");
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
    assert_eq!(c.clips, 1, "the child overhangs the box, so the clip is real");
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
