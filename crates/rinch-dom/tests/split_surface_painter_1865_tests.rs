//! #1865 PR 2: the Vello painter's split of GPU surfaces out of the scene
//! (`docs/design/split-layer-gpu-surfaces.md`, *Painter split*).
//!
//! No GPU: what each scene **encodes** — which draws went where, which clips
//! were pushed, whether the window's scene is untouched when nothing splits.
//! The pixels these scenes make, composited, are pinned against in-scene by
//! the GPU oracle in `rinch` (`shell/split_compositor_1865_tests.rs`, and the
//! runtime's in PR 3).
use std::collections::HashMap;
use std::sync::Arc;

use parley::LayoutContext;
use peniko::kurbo::Rect;
use peniko::{Blob, Brush, ImageAlphaType, ImageData, ImageFormat};
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::Painter;
use rinch_dom::paint::slicing::{SliceCoverage, SurfaceSlice};
use rinch_dom::paint::vello_painter::VelloPainter;
use vello::Scene;

const VIEWPORT: (f32, f32) = (200.0, 120.0);

/// `vello_encoding::DrawTag` values (its field is public, its type is not
/// re-exported): an image fill, a clip's begin.
const IMAGE: u32 = 0x28C;
const BEGIN_CLIP: u32 = 0x49;

/// A page, one surface (id 1, its content box at (20, 12), 40x26) and
/// whatever `extra` adds after it, in the surface's `wrap` (a style for a
/// wrapper around it, empty for none).
fn document(surface_style: &str, wrap: &str, extra: &[&str]) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let page = doc.create_element("div");
    doc.set_attribute(
        page,
        "style",
        "position: absolute; left: 0; top: 0; width: 200px; height: 120px; \
         background: rgb(60, 70, 90)",
    );
    doc.append_child(body, page);
    let parent = if wrap.is_empty() {
        body
    } else {
        let w = doc.create_element("div");
        doc.set_attribute(w, "style", wrap);
        doc.append_child(body, w);
        w
    };
    let surface = doc.create_element("div");
    doc.set_attribute(surface, "data-render-surface", "1");
    doc.set_attribute(
        surface,
        "style",
        &format!(
            "position: absolute; left: 20px; top: 12px; width: 40px; height: 26px; {surface_style}"
        ),
    );
    doc.append_child(parent, surface);
    for style in extra {
        let e = doc.create_element("div");
        doc.set_attribute(e, "style", style);
        doc.append_child(body, e);
    }
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    doc
}

/// A GPU frame as `sync_surface_images` hands paint one: no pixels here.
fn frame(w: u32, h: u32) -> ImageData {
    ImageData {
        data: Blob::new(Arc::new([])),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
        width: w,
        height: h,
    }
}

struct Painted {
    below: Scene,
    slices: Vec<SurfaceSlice>,
}

fn paint(doc: &mut RinchDocument, slicing: bool) -> Painted {
    rinch_dom::paint::set_surface_gpu_images(Some(HashMap::from([(1, frame(40, 26))])));
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
    rinch_dom::paint::set_surface_gpu_images(None);
    Painted {
        below: painter.scene().clone(),
        slices,
    }
}

fn tags(scene: &Scene, tag: u32) -> usize {
    scene
        .encoding()
        .draw_tags
        .iter()
        .filter(|t| t.0 == tag)
        .count()
}

/// Every draw: the encoded draw objects that are not clip ends or begins.
fn draws(scene: &Scene) -> usize {
    scene
        .encoding()
        .draw_tags
        .iter()
        .filter(|t| t.0 != BEGIN_CLIP && t.0 != 0x21) // END_CLIP
        .count()
}

/// The two scenes encode the same thing, byte for byte.
fn same_encoding(a: &Scene, b: &Scene) -> bool {
    let (a, b) = (a.encoding(), b.encoding());
    a.path_tags == b.path_tags
        && a.path_data == b.path_data
        && a.draw_tags == b.draw_tags
        && a.draw_data == b.draw_data
        && format!("{:?}", a.transforms) == format!("{:?}", b.transforms)
        && a.n_paths == b.n_paths
        && a.n_clips == b.n_clips
        // Glyph runs live in the resources (#1894 review: compared too).
        && format!("{:?}", a.resources.glyphs) == format!("{:?}", b.resources.glyphs)
        && a.resources.glyph_runs.len() == b.resources.glyph_runs.len()
        && a.resources
            .glyph_runs
            .iter()
            .zip(&b.resources.glyph_runs)
            .all(|(x, y)| {
                format!("{:?}{:?}", x.transform, x.glyph_transform)
                    == format!("{:?}{:?}", y.transform, y.glyph_transform)
                    && x.font_size == y.font_size
                    && x.hint == y.hint
                    && x.glyphs == y.glyphs
                    && x.normalized_coords == y.normalized_coords
                    && x.font.data.id() == y.font.data.id()
            })
        && a.resources.normalized_coords == b.resources.normalized_coords
}

/// Append `text` in a paragraph at `style` to the page, and lay it out again.
fn add_text(doc: &mut RinchDocument, style: &str, text: &str) {
    let body = doc.body();
    let p = doc.create_element("p");
    doc.set_attribute(p, "style", style);
    let t = doc.create_text(text);
    doc.append_child(p, t);
    doc.append_child(body, p);
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
}

/// A box after the surface, its style `style`.
fn after(doc: &mut RinchDocument, style: &str) {
    let body = doc.body();
    let e = doc.create_element("div");
    doc.set_attribute(e, "style", style);
    doc.append_child(body, e);
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
}

fn balanced(scene: &Scene) -> bool {
    scene.encoding().n_open_clips == 0
}

/// A frame the runtime does not open for slicing paints the surface in the
/// scene, as #1861 does; one it opens, with nothing to split, encodes the
/// window's scene byte for byte as it did before.
#[test]
fn slicing_changes_nothing_where_nothing_splits() {
    // No surface frame at all: the slicing painter's scene is the plain one.
    let mut doc = document(
        "",
        "",
        &[
            "position: absolute; left: 70px; top: 30px; width: 50px; height: 30px; background: rgba(0, 0, 0, 0.5)",
        ],
    );
    add_text(
        &mut doc,
        "position: absolute; left: 4px; top: 70px; margin: 0; font: 13px sans-serif",
        "Glyphs too: the quick brown fox",
    );
    rinch_dom::paint::set_surface_gpu_images(None);
    let mut plain = VelloPainter::new();
    let mut sliced = VelloPainter::new();
    sliced.begin_slicing(Rect::new(0.0, 0.0, 200.0, 120.0));
    for p in [&mut plain, &mut sliced] {
        let mut layout_cx: LayoutContext<Brush> = LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            p,
            1.0,
            VIEWPORT,
            &mut doc.font_cx,
            &mut layout_cx,
        );
    }
    assert!(sliced.take_slices().is_empty());
    assert!(same_encoding(plain.scene(), sliced.scene()));

    // A refused surface (inside an opacity group): the same scene as a frame
    // not opened for slicing, the frame drawn in it.
    let mut doc = document("", "opacity: 0.5", &[]);
    let off = paint(&mut doc, false);
    let on = paint(&mut doc, true);
    assert!(on.slices.is_empty(), "a surface in an opacity group split");
    assert_eq!(tags(&off.below, IMAGE), 1, "the frame is drawn in-scene");
    assert!(same_encoding(&off.below, &on.below));
}

/// A plain surface leaves the scene: one slice, its region the content box,
/// a whole-pixel coverage rect, nothing above it; the window's scene no
/// longer draws the frame and pushes no inverse clip.
#[test]
fn a_plain_surface_leaves_the_scene() {
    let mut doc = document(
        "",
        "",
        &[
            "position: absolute; left: 150px; top: 0; width: 50px; height: 120px; background: rgb(0, 0, 128)",
        ],
    );
    let off = paint(&mut doc, false);
    let on = paint(&mut doc, true);
    assert_eq!(on.slices.len(), 1);
    let s = &on.slices[0];
    assert_eq!(s.surface_id, 1);
    // The content box, and the pixel left of it Vello's antialiasing of a
    // fractional edge can reach (#1894 review).
    assert_eq!(s.region, (19, 12, 41, 26));
    assert!(matches!(s.own, SliceCoverage::Rect([20, 12, 60, 38])));
    assert!(matches!(s.ancestors, SliceCoverage::Rect([19, 12, 60, 38])));
    assert!(
        s.above_in.is_none(),
        "the sidebar is nowhere near the surface"
    );
    assert!(s.above_out.is_none());
    assert!(!s.tainted);
    assert_eq!(s.extents, (40, 26));
    assert_eq!(tags(&off.below, IMAGE), 1);
    assert_eq!(tags(&on.below, IMAGE), 0, "the frame left the scene");
    assert_eq!(
        tags(&on.below, BEGIN_CLIP),
        tags(&off.below, BEGIN_CLIP),
        "a draw away from R is not clipped"
    );
    assert_eq!(draws(&on.below), draws(&off.below) - 1);
    assert!(balanced(&on.below));
}

/// A bar across the surface's bottom edge, painted after it: drawn in the
/// above slice and in the window's scene, there inside one inverse clip.
#[test]
fn a_bar_across_the_surface_goes_above_and_below() {
    let mut doc = document(
        "",
        "",
        &[
            "position: absolute; left: 10px; top: 30px; width: 70px; height: 14px; background: rgba(0, 0, 0, 0.55)",
        ],
    );
    let off = paint(&mut doc, false);
    let on = paint(&mut doc, true);
    let s = &on.slices[0];
    assert!(s.above_out.is_none());
    let above = s.above_in.as_ref().expect("the bar is over the surface");
    assert_eq!(draws(above), 1);
    assert!(balanced(above));
    assert_eq!(
        tags(&on.below, BEGIN_CLIP),
        tags(&off.below, BEGIN_CLIP) + 1,
        "one inverse clip around the bar"
    );
    assert_eq!(
        draws(&on.below),
        draws(&off.below) - 1,
        "the bar is still below"
    );
    assert!(balanced(&on.below));
}

/// A badge wholly inside the surface's region: above only.
#[test]
fn a_badge_inside_the_region_goes_above_only() {
    let mut doc = document(
        "",
        "",
        &[
            "position: absolute; left: 24px; top: 14px; width: 10px; height: 8px; background: rgb(255, 255, 255)",
        ],
    );
    let off = paint(&mut doc, false);
    let on = paint(&mut doc, true);
    let above = on.slices[0].above_in.as_ref().expect("the badge");
    assert_eq!(draws(above), 1);
    assert_eq!(
        draws(&on.below),
        draws(&off.below) - 2,
        "neither frame nor badge below"
    );
    assert_eq!(tags(&on.below, BEGIN_CLIP), tags(&off.below, BEGIN_CLIP));
}

/// Its own rounded corners make the coverage a mask; the clip `paint_surface_frame`
/// would push around the frame is not pushed in the window's scene.
#[test]
fn rounded_corners_make_a_mask() {
    let mut doc = document("border-radius: 8px", "", &[]);
    let off = paint(&mut doc, false);
    let on = paint(&mut doc, true);
    assert!(matches!(on.slices[0].ancestors, SliceCoverage::Rect(_)));
    let SliceCoverage::Mask(mask) = &on.slices[0].own else {
        panic!("rounded corners on a coverage rect");
    };
    assert_eq!(tags(mask, BEGIN_CLIP), 1, "the inner-radius clip");
    assert!(balanced(mask));
    assert_eq!(
        tags(&on.below, BEGIN_CLIP) + 1,
        tags(&off.below, BEGIN_CLIP)
    );
}

/// A whole-pixel scroller around the surface: rect coverages, the
/// ancestors' cut to the scroller.
#[test]
fn a_whole_pixel_scroller_cuts_the_coverage_rect() {
    let mut doc = document(
        "",
        "position: absolute; left: 0; top: 20px; width: 100px; height: 60px; overflow: hidden",
        &[],
    );
    let on = paint(&mut doc, true);
    let s = &on.slices[0];
    // The surface is positioned against the scroller now: (20, 32).
    assert_eq!(s.region, (19, 32, 41, 26));
    assert!(matches!(s.own, SliceCoverage::Rect([20, 32, 60, 58])));
    assert!(matches!(s.ancestors, SliceCoverage::Rect([19, 32, 60, 58])));

    // Scrolled so the scroller cuts R's bottom: the ancestors' rect is cut,
    // the surface's own is not.
    let mut doc = document(
        "",
        "position: absolute; left: 0; top: 20px; width: 100px; height: 30px; overflow: hidden",
        &[],
    );
    let s = &paint(&mut doc, true).slices[0];
    assert!(matches!(s.own, SliceCoverage::Rect([20, 32, 60, 58])));
    assert!(matches!(s.ancestors, SliceCoverage::Rect([19, 32, 60, 50])));
}

/// The card's case (#1887 review): a rounded `overflow: hidden` card, the
/// surface in it, a control bar after it inside the card and a tooltip after
/// the card. The card's coverage is a mask; the bar is in `above_in`
/// without the card's clip; the tooltip in `above_out`; nothing is tainted.
#[test]
fn a_rounded_card_puts_its_content_in_above_in_and_after_it_in_above_out() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let card = doc.create_element("div");
    doc.set_attribute(
        card,
        "style",
        "position: absolute; left: 10px; top: 6px; width: 60px; height: 40px; \
         overflow: hidden; border-radius: 10px; background: rgb(240, 240, 240)",
    );
    doc.append_child(body, card);
    let surface = doc.create_element("div");
    doc.set_attribute(surface, "data-render-surface", "1");
    doc.set_attribute(
        surface,
        "style",
        "position: absolute; left: 10px; top: 6px; width: 40px; height: 26px",
    );
    doc.append_child(card, surface);
    let bar = doc.create_element("div");
    doc.set_attribute(
        bar,
        "style",
        "position: absolute; left: 0; top: 24px; width: 60px; height: 10px; \
         background: rgba(0, 0, 0, 0.5)",
    );
    doc.append_child(card, bar);
    let tip = doc.create_element("div");
    doc.set_attribute(
        tip,
        "style",
        "position: absolute; left: 40px; top: 8px; width: 40px; height: 10px; \
         background: rgb(255, 255, 220)",
    );
    doc.append_child(body, tip);
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    let on = paint(&mut doc, true);
    let s = &on.slices[0];
    assert_eq!(s.region, (19, 12, 41, 26));
    assert!(matches!(s.own, SliceCoverage::Rect([20, 12, 60, 38])));
    let SliceCoverage::Mask(card_mask) = &s.ancestors else {
        panic!("a rounded card's coverage as a rect");
    };
    assert!(tags(card_mask, BEGIN_CLIP) >= 1);
    assert!(balanced(card_mask));
    let above_in = s.above_in.as_ref().expect("the bar is over the surface");
    assert_eq!(draws(above_in), 1);
    assert_eq!(
        tags(above_in, BEGIN_CLIP),
        0,
        "the card's clip is not replayed"
    );
    let above_out = s
        .above_out
        .as_ref()
        .expect("the tooltip is over the surface");
    assert_eq!(draws(above_out), 1);
    assert!(balanced(above_in) && balanced(above_out));
    assert!(!s.tainted);
}

/// Content painted after an inner whole-pixel wrapper closes but still
/// inside an outer rounded card, over the surface: the card's ring would be
/// applied to it separately — tainted. The control, an outer card with
/// square corners, is not.
#[test]
fn content_between_two_closes_over_a_ring_taints() {
    for (radius, tainted) in [("10px", true), ("0", false)] {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        doc.set_attribute(body, "style", "margin: 0");
        let card = doc.create_element("div");
        doc.set_attribute(
            card,
            "style",
            &format!(
                "position: absolute; left: 10px; top: 6px; width: 60px; height: 40px; \
                 overflow: hidden; border-radius: {radius}"
            ),
        );
        doc.append_child(body, card);
        let wrap = doc.create_element("div");
        doc.set_attribute(
            wrap,
            "style",
            "position: absolute; left: 0; top: 0; width: 60px; height: 40px; overflow: hidden",
        );
        doc.append_child(card, wrap);
        let surface = doc.create_element("div");
        doc.set_attribute(surface, "data-render-surface", "1");
        doc.set_attribute(
            surface,
            "style",
            "position: absolute; left: 10px; top: 6px; width: 40px; height: 26px",
        );
        doc.append_child(wrap, surface);
        let bar = doc.create_element("div");
        doc.set_attribute(
            bar,
            "style",
            "position: absolute; left: 0; top: 24px; width: 60px; height: 10px; \
             background: rgba(0, 0, 0, 0.5)",
        );
        doc.append_child(card, bar);
        doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
        let on = paint(&mut doc, true);
        assert_eq!(on.slices.len(), 1);
        assert_eq!(on.slices[0].tainted, tainted, "radius {radius}");
        let out = on.slices[0]
            .above_out
            .as_ref()
            .expect("the bar is above_out");
        // The card's clip, still open, replayed at above_out's start.
        assert!(
            tags(out, BEGIN_CLIP) >= 1,
            "radius {radius}: the still-open card is not replayed into above_out"
        );
        assert!(balanced(out));
    }
}

/// The refusals: a rotation, and a second surface over the first.
#[test]
fn rotation_and_overlap_stay_in_the_scene() {
    let mut doc = document("transform: rotate(10deg)", "", &[]);
    let on = paint(&mut doc, true);
    assert!(on.slices.is_empty(), "a rotated surface split");
    assert_eq!(tags(&on.below, IMAGE), 1);

    // Two surfaces, the second over the first: the first splits, the second
    // is drawn in its above slice.
    let mut doc = document("", "", &[]);
    let body = doc.body();
    let second = doc.create_element("div");
    doc.set_attribute(second, "data-render-surface", "2");
    doc.set_attribute(
        second,
        "style",
        "position: absolute; left: 40px; top: 20px; width: 40px; height: 26px",
    );
    doc.append_child(body, second);
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    rinch_dom::paint::set_surface_gpu_images(Some(HashMap::from([
        (1, frame(40, 26)),
        (2, frame(40, 26)),
    ])));
    let mut painter = VelloPainter::new();
    painter.begin_slicing(Rect::new(0.0, 0.0, 200.0, 120.0));
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
    rinch_dom::paint::set_surface_gpu_images(None);
    assert_eq!(slices.len(), 1);
    assert_eq!(slices[0].surface_id, 1);
    let above = slices[0]
        .above_in
        .as_ref()
        .expect("the second surface is over the first");
    assert_eq!(tags(above, IMAGE), 1, "drawn in-scene, in the above slice");
    assert_eq!(
        tags(painter.scene(), IMAGE),
        1,
        "and below, where it is outside R"
    );
}

/// Grayscale over the surface reads a backdrop without it: tainted (D5a).
/// The control, brightness (an ordinary fill), is not.
#[test]
fn grayscale_over_the_surface_taints_it() {
    let over = "position: absolute; left: 10px; top: 20px; width: 70px; height: 30px; \
                background: rgba(255, 255, 255, 0.2); filter: grayscale(1)";
    let mut doc = document("", "", &[over]);
    assert!(paint(&mut doc, true).slices[0].tainted);
    let bright = "position: absolute; left: 10px; top: 20px; width: 70px; height: 30px; \
                  background: rgba(255, 255, 255, 0.2); filter: brightness(0.5)";
    let mut doc = document("", "", &[bright]);
    let on = paint(&mut doc, true);
    assert!(!on.slices[0].tainted);
    assert!(on.slices[0].above_in.is_some());
}

/// The image transform is the in-scene fill's, centre sampling included.
#[test]
fn the_slice_carries_the_in_scene_image_transform() {
    let mut doc = document("", "", &[]);
    let on = paint(&mut doc, true);
    // translate(20, 12), then half a pixel up and left (#1027).
    assert_eq!(
        on.slices[0].device_from_image,
        [1.0, 0.0, 0.0, 1.0, 19.5, 11.5]
    );
}

/// A box whose left edge is R's right edge and whose top is fractional:
/// Vello paints the pixel left of its bounds, inside R, so it is routed
/// across R, not to below only (#1894 review, 9 levels).
#[test]
fn a_box_touching_r_from_the_right_is_routed_across_it() {
    let mut doc = document("", "", &[]);
    after(
        &mut doc,
        "position: absolute; left: 60px; top: 20.4px; width: 20px; height: 10px; \
         background: rgb(0, 0, 0)",
    );
    let on = paint(&mut doc, true);
    let above = on.slices[0]
        .above_in
        .as_ref()
        .expect("the box's left antialiased pixel is in R");
    assert_eq!(draws(above), 1);
}

/// Routing goes by **device** bounds: a box laid out far from the surface
/// and moved over it by a transform is above it.
#[test]
fn a_transformed_box_is_routed_where_it_lands() {
    let mut doc = document("", "", &[]);
    after(
        &mut doc,
        "position: absolute; left: 150px; top: 20px; width: 20px; height: 10px; \
         background: rgb(0, 0, 0); transform: translateX(-120px)",
    );
    let on = paint(&mut doc, true);
    assert!(
        on.slices[0].above_in.is_some(),
        "the box moved over the surface is not above it"
    );
}

/// Two surfaces, a draw across the first between them (opening below's
/// inverse clip), then a draw across the second: the inverse clip is
/// opened again after the second split, with its hole (#1894 review: the
/// first one, without it, was still open: 49 levels).
#[test]
fn a_second_split_reopens_the_inverse_clip_with_its_hole() {
    let mut doc = document(
        "",
        "",
        &[
            "position: absolute; left: 10px; top: 30px; width: 70px; height: 14px; \
           background: rgba(0, 0, 0, 0.5)",
        ],
    );
    let body = doc.body();
    let second = doc.create_element("div");
    doc.set_attribute(second, "data-render-surface", "2");
    doc.set_attribute(
        second,
        "style",
        "position: absolute; left: 100px; top: 60px; width: 40px; height: 26px",
    );
    doc.append_child(body, second);
    after(
        &mut doc,
        "position: absolute; left: 90px; top: 80px; width: 70px; height: 14px; \
         background: rgba(0, 0, 0, 0.5)",
    );
    let images = HashMap::from([(1, frame(40, 26)), (2, frame(40, 26))]);
    let mut count = |slicing: bool| {
        rinch_dom::paint::set_surface_gpu_images(Some(images.clone()));
        let mut painter = VelloPainter::new();
        if slicing {
            painter.begin_slicing(Rect::new(0.0, 0.0, 200.0, 120.0));
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
        rinch_dom::paint::set_surface_gpu_images(None);
        (tags(painter.scene(), BEGIN_CLIP), slices.len())
    };
    let (plain, _) = count(false);
    let (sliced, n) = count(true);
    assert_eq!(n, 2);
    assert_eq!(
        sliced,
        plain + 2,
        "one inverse clip around each bar, the second cut for both regions"
    );
}
