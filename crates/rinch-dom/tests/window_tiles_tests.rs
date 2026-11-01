//! A frame asks Vello for tiles over every clip's and layer's whole bounds, and
//! Vello's tile buffer is fixed: past it the frame is dropped and a transparent
//! window goes blank. Pimble's empty window at 3840x2128 asked for 2.81M tiles
//! (Vello holds 2.1M): 64 opacity layers over the whole plane for hidden, dimmed
//! elements (2.04M), and its window-sized clips re-opened around entries that
//! draw nothing (0.67M). These pin the paint-order fixes; `vello_painter`'s own
//! tests pin the third (a clip that cuts nothing is not pushed).

use peniko::kurbo::{Affine, Rect, Stroke};
use peniko::{Brush, Fill, FontData};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::{BlendMode, PaintGlyph, PaintImage, PaintShape, Painter};

/// Records the bounds of every clip and layer pushed.
#[derive(Default)]
struct Pushes {
    clips: Vec<Rect>,
    layers: Vec<Rect>,
    pops: usize,
    glyph_runs: usize,
}

impl Painter for Pushes {
    fn reset(&mut self) {
        *self = Self::default();
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
        self.glyph_runs += 1;
    }
    fn draw_image(&mut self, _: &PaintImage<'_>, _: Affine) {}
    fn push_clip(&mut self, _: Fill, t: Affine, shape: &PaintShape) {
        self.clips.push(t.transform_rect_bbox(shape.bounding_box()));
    }
    fn push_layer(&mut self, _: BlendMode, _: f32, t: Affine, shape: &PaintShape) {
        self.layers
            .push(t.transform_rect_bbox(shape.bounding_box()));
    }
    fn pop_layer(&mut self) {
        self.pops += 1;
    }
}

const VIEWPORT: (f32, f32) = (800.0, 600.0);

fn paint(doc: &mut RinchDocument) -> Pushes {
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    let mut painter = Pushes::default();
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        VIEWPORT,
        &mut doc.font_cx,
        &mut layout_cx,
    );
    assert_eq!(
        painter.clips.len() + painter.layers.len(),
        painter.pops,
        "every push is popped"
    );
    painter
}

fn el(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let e = doc.create_element("div");
    doc.set_attribute(e, "style", style);
    doc.append_child(parent, e);
    e
}

/// Dimmed elements under `display: none` (a hidden toolbar's disabled buttons,
/// a closed menu's disabled items) were hoisted into the paint sequence as
/// stacking contexts and each got a layer over the whole plane. Nothing under
/// `display: none` is painted now.
#[test]
fn dimmed_elements_under_display_none_are_not_painted() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let menu = el(&mut doc, body, "display: none");
    // With a child each: a childless `0x0` box is dismissed before any layer
    // is pushed, hoisted or not, and would pass without the fix.
    for _ in 0..3 {
        let dimmed = el(&mut doc, menu, "opacity: 0.5");
        el(&mut doc, dimmed, "");
    }
    el(
        &mut doc,
        body,
        "opacity: 0.5; width: 40px; height: 20px; background-color: red",
    );
    let p = paint(&mut doc);
    assert_eq!(
        p.layers.len(),
        1,
        "only the shown dimmed element is painted: {:?}",
        p.layers
    );
    assert!(
        p.layers
            .iter()
            .all(|r| r.width() < 1000.0 && r.height() < 1000.0),
        "no layer covers the plane: {:?}",
        p.layers
    );
}

/// The clips pushed for `content`'s rect with a `position: fixed` menu between
/// its two positioned children: closed (sized to nothing, its item hidden) or
/// open (drawing a box).
fn clips_around_a_fixed_menu(open_menu: bool) -> usize {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let content = el(
        &mut doc,
        body,
        "position: relative; left: 20px; top: 20px; width: 400px; height: 300px; overflow: hidden",
    );
    el(
        &mut doc,
        content,
        "position: relative; width: 50px; height: 20px; background-color: red",
    );
    let menu_style = if open_menu {
        "position: fixed; left: 500px; top: 0; width: 80px; height: 40px; background-color: white"
    } else {
        "position: fixed; left: 0; top: 0; width: 0; height: 0"
    };
    let menu = el(&mut doc, content, menu_style);
    let item = el(&mut doc, menu, if open_menu { "" } else { "display: none" });
    let text = doc.create_text("Cut");
    doc.append_child(item, text);
    el(
        &mut doc,
        content,
        "position: relative; width: 50px; height: 20px; background-color: blue",
    );
    let p = paint(&mut doc);
    let same = |a: f64, b: f64| (a - b).abs() < 0.5;
    p.clips
        .iter()
        .filter(|r| same(r.x0, 20.0) && same(r.y0, 20.0) && same(r.x1, 420.0) && same(r.y1, 320.0))
        .count()
}

/// A fixed entry has an empty clip chain: switching to one closes the clips
/// the entries around it share, and the next entry opens them all again. An
/// open menu needs that (it is drawn outside them); a closed one draws
/// nothing, and is skipped before the switch.
#[test]
fn a_closed_fixed_menu_does_not_reopen_the_clips_around_it() {
    let open = clips_around_a_fixed_menu(true);
    let closed = clips_around_a_fixed_menu(false);
    assert_eq!(closed + 1, open, "closed {closed}, open {open}");
}

/// A dimmed element that is shown but draws nothing (an empty, zero-size box)
/// gets an empty layer, not one over the whole plane.
#[test]
fn a_shown_dimmed_element_that_draws_nothing_gets_an_empty_layer() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let wrap = el(&mut doc, body, "width: 0; height: 0");
    // Its child keeps it past the dismissal of a childless `0x0` box, so a
    // layer really is pushed.
    let dimmed = el(&mut doc, wrap, "opacity: 0.5; position: relative");
    el(&mut doc, dimmed, "");
    let p = paint(&mut doc);
    assert_eq!(p.layers.len(), 1, "{:?}", p.layers);
    assert!(
        p.layers
            .iter()
            .all(|r| r.width() < 1000.0 && r.height() < 1000.0),
        "no layer covers the plane: {:?}",
        p.layers
    );
}

/// A box collapsed to nothing still draws the text directly inside it, which
/// overflows it, so it is not an entry that paints nothing.
#[test]
fn text_directly_in_a_zero_area_fixed_box_is_drawn_between_clipped_entries() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let content = el(
        &mut doc,
        body,
        "position: relative; width: 400px; height: 300px; overflow: hidden",
    );
    el(&mut doc, content, "position: relative; height: 20px");
    let label = el(
        &mut doc,
        content,
        "position: fixed; left: 0; top: 0; width: 0; height: 0; white-space: nowrap",
    );
    let text = doc.create_text("Cut");
    doc.append_child(label, text);
    el(&mut doc, content, "position: relative; height: 20px");
    assert_eq!(paint(&mut doc).glyph_runs, 1);
}

/// The layer of a dimmed box collapsed to nothing holds the text inside it:
/// Vello clips a layer to its bounds, so an empty one would blank the text.
#[test]
fn a_dimmed_zero_height_box_keeps_its_text_in_its_layer() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let dimmed = el(&mut doc, body, "height: 0; width: 200px; opacity: 0.5");
    let text = doc.create_text("Label");
    doc.append_child(dimmed, text);
    let p = paint(&mut doc);
    assert_eq!(p.glyph_runs, 1);
    assert_eq!(p.layers.len(), 1);
    let layer = p.layers[0];
    assert!(
        layer.width() > 1.0 && layer.height() > 1.0,
        "the text is inside the layer: {layer:?}"
    );
}
