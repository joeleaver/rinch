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
    doc.set_attribute(wrapper, "style", "height: 0; overflow: hidden; width: 200px");
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
