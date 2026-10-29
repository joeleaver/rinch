//! #1866 on the split path: the Vello painter takes a `GameViewport` out of
//! the scene as a **base** slice (`docs/design/split-layer-gpu-surfaces.md`,
//! *How `GameViewport` fits on the split path*), with no erase in the scene,
//! and says when it could not (`take_base_refused`), so the GPU window falls
//! back to the layered path for that frame.
//!
//! No GPU: what the scenes encode and what the slices hold. The pixels are
//! pinned by `rinch`'s `app::game_viewport_base_layer_1866_tests` (desktop)
//! and `tests/embed_game_viewport_base_layer_1866.rs` (embed overlay).
use std::collections::HashMap;

use parley::LayoutContext;
use peniko::Brush;
use peniko::kurbo::Rect;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::Painter;
use rinch_dom::paint::slicing::{SliceCoverage, SurfaceSlice};
use rinch_dom::paint::vello_painter::VelloPainter;
use rinch_dom::paint::{BASE_WITHOUT_GAME_IDS, BaseViewport};
use vello::Scene;

const VIEWPORT: (f32, f32) = (200.0, 120.0);
/// `vello_encoding::DrawTag::BEGIN_CLIP`: a clip's or a layer's begin.
const BEGIN_CLIP: u32 = 0x49;

/// A green page, a red box under the viewport's top-left, and the viewport
/// `g` at (20, 12), 40x26, styled `style`, inside a wrapper styled `wrap`.
fn document(style: &str, wrap: &str) -> (RinchDocument, rinch_core::dom::NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let page = doc.create_element("div");
    doc.set_attribute(
        page,
        "style",
        "position: absolute; left: 0; top: 0; width: 200px; height: 120px; \
         background: rgb(0, 128, 0)",
    );
    doc.append_child(body, page);
    let red = doc.create_element("div");
    doc.set_attribute(
        red,
        "style",
        "position: absolute; left: 10px; top: 5px; width: 30px; height: 20px; \
         background: rgb(255, 0, 0)",
    );
    doc.append_child(body, red);
    let wrapper = doc.create_element("div");
    doc.set_attribute(wrapper, "style", wrap);
    doc.append_child(body, wrapper);
    let viewport = doc.create_element("div");
    doc.set_attribute(viewport, "data-viewport", "g");
    doc.set_attribute(
        viewport,
        "style",
        &format!("position: absolute; left: 20px; top: 12px; width: 40px; height: 26px; {style}"),
    );
    doc.append_child(wrapper, viewport);
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    (doc, viewport)
}

struct Painted {
    below: Scene,
    slices: Vec<SurfaceSlice>,
    refused: bool,
}

/// Paint `doc` as the GPU window does with `g` a base viewport (`named`), or
/// as the embed overlay does (`!named`: every viewport, no game).
fn paint(doc: &mut RinchDocument, slicing: bool, named: bool) -> Painted {
    rinch_dom::paint::set_active_viewports(Some(["g".to_string()].into_iter().collect()));
    if named {
        rinch_dom::paint::set_base_viewports(Some(HashMap::from([(
            "g".to_string(),
            BaseViewport {
                surface_id: 7,
                extents: (40, 13),
                letterbox: true,
            },
        )])));
    } else {
        rinch_dom::paint::set_bases_without_game(true);
    }
    let _ = rinch_dom::paint::take_base_refused();
    let mut painter = VelloPainter::new();
    painter.reset();
    if slicing {
        painter.begin_slicing(Rect::new(0.0, 0.0, VIEWPORT.0 as f64, VIEWPORT.1 as f64));
    }
    let mut layout_cx: LayoutContext<Brush> = LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        VIEWPORT,
        &mut doc.font_cx,
        &mut layout_cx,
    );
    let slices = painter.take_slices();
    rinch_dom::paint::set_base_viewports(None);
    rinch_dom::paint::set_bases_without_game(false);
    rinch_dom::paint::set_active_viewports(None);
    Painted {
        below: painter.scene().clone(),
        slices,
        refused: rinch_dom::paint::take_base_refused(),
    }
}

fn layers(scene: &Scene) -> usize {
    scene
        .encoding()
        .draw_tags
        .iter()
        .filter(|t| t.0 == BEGIN_CLIP)
        .count()
}

/// Taken: one base slice keyed by the viewport's id, R its box, the clear
/// its box on whole pixels, black under the letterboxed frame, the frame
/// fitted `contain` (40x13 into 40x26: 6.5px bars), and nothing of it in
/// the window's scene: no layer at all (the erase was a `DestinationOut`
/// layer).
#[test]
fn a_game_viewport_leaves_the_scene_as_a_base_slice() {
    let (mut doc, _) = document("", "");
    let p = paint(&mut doc, true, true);
    assert!(!p.refused);
    assert_eq!(p.slices.len(), 1);
    let s = &p.slices[0];
    assert_eq!(s.surface_id, 7);
    assert_eq!(s.region, (20, 12, 40, 26));
    let base = s.base.as_ref().expect("a base slice");
    assert!(base.black_backdrop);
    assert!(matches!(base.clear, SliceCoverage::Rect([20, 12, 60, 38])));
    // The image is the fitted frame, not the box: a mask (its bars fall on
    // half pixels).
    assert!(matches!(s.own, SliceCoverage::Mask(_)));
    assert_eq!(
        s.device_from_image[5].fract(),
        0.0,
        "{:?}",
        s.device_from_image
    );
    assert_eq!(layers(&p.below), 0, "no erase in the scene");
}

/// A rounded viewport's clear is a mask (Vello's coverage of the radius).
#[test]
fn a_rounded_viewports_clear_is_a_mask() {
    let (mut doc, _) = document("border-radius: 6px", "");
    let p = paint(&mut doc, true, true);
    assert!(!p.refused);
    let base = p.slices[0].base.as_ref().expect("a base slice");
    assert!(matches!(base.clear, SliceCoverage::Mask(_)));
    assert_eq!(layers(&p.below), 0);
}

/// Refused — inside an opacity group, or on a frame not opened for slicing
/// — the box is erased in the scene (a layer) and the refusal is reported,
/// so the window composites this frame as layers (R1).
#[test]
fn a_refused_base_is_erased_in_the_scene_and_reported() {
    let (mut doc, _) = document("", "opacity: 0.5");
    let p = paint(&mut doc, true, true);
    assert!(p.refused, "inside an opacity group");
    assert!(p.slices.is_empty());
    assert_eq!(layers(&p.below), 2, "the opacity layer and the erase");

    let (mut doc, _) = document("", "");
    let p = paint(&mut doc, false, true);
    assert!(p.refused, "a frame not opened for slicing");
    assert_eq!(layers(&p.below), 1, "the erase");
}

/// A viewport that does not punch (no layer behind it) is offered nothing
/// and refuses nothing.
#[test]
fn a_viewport_that_does_not_punch_is_not_offered() {
    let (mut doc, viewport) = document("", "");
    doc.set_attribute(viewport, "data-viewport", "other");
    doc.resolve_layout(VIEWPORT.0 + 1.0, VIEWPORT.1);
    let p = paint(&mut doc, true, true);
    assert!(!p.refused);
    assert!(p.slices.is_empty());
}

/// The embed overlay: every punching viewport is a base with no game,
/// keyed past `BASE_WITHOUT_GAME_IDS` by its node id.
#[test]
fn the_embed_overlay_splits_every_viewport_with_no_game() {
    let (mut doc, viewport) = document("", "");
    let p = paint(&mut doc, true, false);
    assert!(!p.refused);
    assert_eq!(p.slices.len(), 1);
    assert_eq!(
        p.slices[0].surface_id,
        BASE_WITHOUT_GAME_IDS + viewport.index()
    );
    let base = p.slices[0].base.as_ref().expect("a base slice");
    assert!(!base.black_backdrop);
    assert!(matches!(base.clear, SliceCoverage::Rect([20, 12, 60, 38])));
    assert_eq!(layers(&p.below), 0);
}
