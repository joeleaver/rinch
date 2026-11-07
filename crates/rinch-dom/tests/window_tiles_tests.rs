//! A frame asks Vello for tiles over every clip's and layer's whole bounds, and
//! Vello's tile buffer is fixed: past it the frame is dropped and a transparent
//! window goes blank. Pimble's empty window at 3840x2128 asked for 2.81M tiles
//! (Vello holds 2.1M): 64 opacity layers over the whole plane for hidden, dimmed
//! elements (2.04M), and its window-sized clips re-opened around entries that
//! draw nothing (0.67M). These pin the paint-order fixes; `vello_painter`'s own
//! tests pin the third (a clip that cuts nothing is not pushed).
//!
//! The `tile_cost_*` tests put a number on each, through a real `VelloPainter`:
//! `clip_tiles`, the estimate behind the `vello_clip_tiles` counter. Every box
//! in them has declared geometry and no text, so the numbers do not depend on
//! the host's fonts. A window here is 800x600: 50x38, 1900 tiles.

use peniko::kurbo::{Affine, Rect, Stroke};
use peniko::{Brush, Fill, FontData};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::{BlendMode, PaintGlyph, PaintImage, PaintShape, Painter};
use rinch_dom::paint::vello_painter::VelloPainter;

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
        _: rinch_dom::paint::painter::GlyphSynthesis,
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

/// What a frame of `doc` asks Vello for, in `VIEWPORT`.
fn clip_tiles(doc: &mut RinchDocument) -> u64 {
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    let mut painter = VelloPainter::new();
    painter.reset();
    painter.set_tile_target(Rect::new(0.0, 0.0, VIEWPORT.0 as f64, VIEWPORT.1 as f64));
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        VIEWPORT,
        &mut doc.font_cx,
        &mut layout_cx,
    );
    assert!(painter.take_slices().is_empty());
    painter.clip_tiles()
}

fn page() -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    (doc, body)
}

/// Dimmed elements under `display: none` cost nothing. Each holds a fixed box,
/// which no bound on the dimmed one can contain: reached, it gets a layer over
/// the whole plane, 1900 tiles here (without one, its layer is proved empty and
/// costs none either way). The shown one costs its own box: x 24..64 is tiles
/// 1..4, y 0..20 is tiles 0..2.
#[test]
fn tile_cost_of_a_hidden_dimmed_subtree_is_nothing() {
    let (mut doc, body) = page();
    let menu = el(&mut doc, body, "display: none");
    for _ in 0..3 {
        let dimmed = el(&mut doc, menu, "opacity: 0.5");
        el(
            &mut doc,
            dimmed,
            "position: fixed; width: 100px; height: 100px; background-color: red",
        );
    }
    el(
        &mut doc,
        body,
        "opacity: 0.5; margin-left: 24px; width: 40px; height: 20px; background-color: red",
    );
    assert_eq!(clip_tiles(&mut doc), 3 * 2);
}

/// `content`'s clip around two positioned entries with a fixed menu between
/// them, closed or open. The clip covers x 20..420 (tiles 1..27) and y 20..320
/// (tiles 1..20): 494.
fn tiles_around_a_fixed_menu(open_menu: bool) -> u64 {
    let (mut doc, body) = page();
    let content = el(
        &mut doc,
        body,
        "position: relative; left: 20px; top: 20px; width: 400px; height: 300px; overflow: hidden",
    );
    let entry = "position: relative; width: 50px; height: 20px; background-color: red";
    el(&mut doc, content, entry);
    let menu_style = if open_menu {
        "position: fixed; left: 500px; top: 0; width: 80px; height: 40px; background-color: white"
    } else {
        "position: fixed; left: 0; top: 0; width: 0; height: 0"
    };
    let menu = el(&mut doc, content, menu_style);
    let item = el(&mut doc, menu, if open_menu { "" } else { "display: none" });
    el(
        &mut doc,
        item,
        "width: 10px; height: 10px; background-color: green",
    );
    el(&mut doc, content, entry);
    clip_tiles(&mut doc)
}

/// The clip opens for the box's own paint and for its entries. A closed menu
/// between the entries leaves that alone; an open one, drawn outside the clip,
/// closes it, and the next entry pays for it again.
#[test]
fn tile_cost_of_a_closed_fixed_menu_between_clipped_entries_is_nothing() {
    assert_eq!(tiles_around_a_fixed_menu(false), 2 * 494);
    assert_eq!(tiles_around_a_fixed_menu(true), 3 * 494);
}

/// Panes inside panes, each its own stacking context and each clipping to the
/// same box, around one small entry that crosses their edge (paint opens no
/// clip around what fits in it). The outermost clip pays: x 8..792 is 50
/// columns, y 8..592 is 37 rows. The clip paint re-opens inside it cuts
/// nothing, and is not pushed; pushed, it would cost those 1850 tiles again.
#[test]
fn tile_cost_of_a_window_sized_clip_is_paid_once() {
    let (mut doc, body) = page();
    let mut pane = el(
        &mut doc,
        body,
        "position: absolute; left: 8px; top: 8px; width: 784px; height: 584px; overflow: hidden",
    );
    for _ in 0..3 {
        pane = el(
            &mut doc,
            pane,
            "position: relative; z-index: 1; width: 100%; height: 100%; overflow: hidden",
        );
    }
    el(
        &mut doc,
        pane,
        "position: absolute; left: 770px; top: 100px; width: 40px; height: 20px; \
         background-color: red",
    );
    assert_eq!(clip_tiles(&mut doc), 50 * 37);
}

/// `pairs` rows in each of two unrelated panes, their `z-index` alternating
/// between the panes, so consecutive entries share no clip.
fn tiles_under_two_clippers(pairs: u32) -> u64 {
    let (mut doc, body) = page();
    let pane = |doc: &mut RinchDocument, left: u32| {
        let style = format!(
            "position: absolute; left: {left}px; top: 0; width: 392px; height: 600px; \
             overflow: hidden"
        );
        el(doc, body, &style)
    };
    let (a, b) = (pane(&mut doc, 8), pane(&mut doc, 400));
    for i in 0..pairs {
        for (parent, z) in [(a, 2 * i), (b, 2 * i + 1)] {
            let row =
                format!("position: relative; z-index: {z}; height: 10px; background-color: red");
            el(&mut doc, parent, &row);
        }
    }
    clip_tiles(&mut doc)
}

/// joeleaver/rinch#1645: rows of two panes alternating by `z-index` used to close
/// one pane's clip and open the other's per entry, 25x38 = 950 tiles each (x
/// 8..400 and x 400..792 are 25 columns): 19000 at 10 pairs, 38000 at 20. The
/// panes share no pixel, so the rows are gathered by pane, and the number no
/// longer grows with them. It is three clips and not two because pane B's own
/// entry sits between row `z-index: 0` and the rest: it has no clip to prove it
/// clear of A, so A's first row keeps its side of it.
#[test]
fn tile_cost_of_entries_under_two_unrelated_clippers_does_not_grow_with_them() {
    assert_eq!(tiles_under_two_clippers(10), 3 * 950);
    assert_eq!(tiles_under_two_clippers(20), 3 * 950);
}
