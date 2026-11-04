//! A `visibility: hidden` subtree with nothing visible in it is skipped by
//! paint and by the stacking collector (#2137).
//!
//! `visibility` inherits and a descendant may set it back to `visible`, which
//! is drawn (#829, #843), so the skip is taken only where nothing below the
//! hidden node is visible (`rinch_dom::visibility::shows_below`, the hit
//! tester's memo). It is a pure saving: every picture here is painted twice,
//! with the skip and without it (`set_hidden_subtree_cull`), and the two must
//! be byte for byte the same. The counters say the saving is real.

#![cfg(feature = "software-renderer")]

use peniko::kurbo::{Affine, Stroke};
use peniko::{Brush, Fill, FontData};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::{BlendMode, PaintGlyph, PaintImage, PaintShape, Painter};
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use rinch_dom::perf::Counter;
use rinch_dom::visibility::set_hidden_subtree_cull;

const VP: (f32, f32) = (320.0, 240.0);

fn el(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let e = doc.create_element("div");
    doc.set_attribute(e, "style", style);
    doc.append_child(parent, e);
    e
}

fn text(doc: &mut RinchDocument, parent: NodeId, s: &str) {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
}

fn page() -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css("body { margin: 0; font-size: 14px; line-height: 18px; }");
    let body = doc.body();
    (doc, body)
}

/// Restores the skip however a test ends, so a failing test cannot leave the
/// next one on this thread painting without it.
struct CullOff;
impl CullOff {
    fn new() -> Self {
        set_hidden_subtree_cull(false);
        Self
    }
}
impl Drop for CullOff {
    fn drop(&mut self) {
        set_hidden_subtree_cull(true);
    }
}

/// What a paint drew and what it cost: the pixels, `paint_nodes_visited`, and
/// the layers pushed.
struct Shot {
    pixels: Vec<u8>,
    visited: u64,
    layers: u64,
}

fn shoot(doc: &mut RinchDocument, scale: f64) -> Shot {
    doc.resolve_layout(VP.0, VP.1);
    let (w, h) = ((VP.0 as f64 * scale) as u32, (VP.1 as f64 * scale) as u32);
    let mut painter = TinySkiaPainter::new(w, h);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    let visited = doc.tree.perf.get(Counter::PaintNodesVisited);
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale,
        VP,
        &mut doc.font_cx,
        &mut layout_cx,
    );
    let visited = doc.tree.perf.get(Counter::PaintNodesVisited) - visited;
    let mut counting = Commands::default();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut counting,
        scale,
        VP,
        &mut doc.font_cx,
        &mut layout_cx,
    );
    Shot {
        pixels: painter.pixels().to_vec(),
        visited,
        layers: counting.layers,
    }
}

/// The document painted with the skip, then without it.
fn both(doc: &mut RinchDocument, scale: f64) -> (Shot, Shot) {
    let on = shoot(doc, scale);
    let off = {
        let _off = CullOff::new();
        shoot(doc, scale)
    };
    (on, off)
}

fn assert_same_picture(on: &Shot, off: &Shot, what: &str) {
    if on.pixels != off.pixels {
        let differing = on
            .pixels
            .chunks(4)
            .zip(off.pixels.chunks(4))
            .filter(|(a, b)| a != b)
            .count();
        panic!("{what}: the skip changed {differing} pixels");
    }
}

fn ink(px: &[u8]) -> usize {
    px.chunks(4).filter(|p| p[3] > 0).count()
}

/// Counts the commands a paint emits that open a layer, and records the
/// watched nodes it starts.
#[derive(Default)]
struct Commands {
    layers: u64,
    started: Vec<u64>,
}

impl Painter for Commands {
    fn reset(&mut self) {}
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
    fn push_clip(&mut self, _: Fill, _: Affine, _: &PaintShape) {}
    fn push_layer(&mut self, _: BlendMode, _: f32, _: Affine, _: &PaintShape) {
        self.layers += 1;
    }
    fn pop_layer(&mut self) {}
    fn note_node_start(&mut self, node: u64) {
        self.started.push(node);
    }
}

// ── What a closed overlay costs ────────────────────────────────────────────

/// The two shapes of a closed overlay rinch's components take.
#[derive(Clone, Copy, Debug)]
enum Overlay {
    /// A `Drawer`: a fixed root, its panel transformed — a stacking context,
    /// so what it holds is hoisted into its own sequence.
    Drawer,
    /// A `Popover` or menu: an absolutely positioned root and no stacking
    /// context, so what it holds is hoisted into the **body's** sequence.
    Popover,
    /// A collapsed panel in flow (an accordion's, a menu's submenu): the
    /// root is no entry of any sequence, and paint meets it in its parent's
    /// child loop.
    InFlow,
}

/// A closed overlay of `rows` rows of text, each row `position: relative`
/// with a translucent positioned badge and a `z-index`ed one: the boxes the
/// collector hoists.
fn closed_overlay(doc: &mut RinchDocument, body: NodeId, rows: usize, shape: Overlay) -> NodeId {
    let root = match shape {
        Overlay::Drawer => el(
            doc,
            body,
            "position: fixed; left: 0; top: 0; width: 200px; height: 240px; \
             visibility: hidden",
        ),
        Overlay::Popover => el(
            doc,
            body,
            "position: absolute; left: 10px; top: 10px; width: 200px; visibility: hidden",
        ),
        Overlay::InFlow => {
            let wrapper = el(doc, body, "width: 200px");
            el(doc, wrapper, "visibility: hidden")
        }
    };
    let panel = el(
        doc,
        root,
        match shape {
            Overlay::Drawer => {
                "transform: translateX(-10px); background: rgb(250, 250, 250); height: 240px; \
                 overflow-y: auto"
            }
            Overlay::Popover | Overlay::InFlow => "background: rgb(250, 250, 250)",
        },
    );
    for i in 0..rows {
        let row = el(
            doc,
            panel,
            "position: relative; height: 2px; border-bottom: 1px solid rgb(0, 0, 0); \
             font-size: 1px; line-height: 1px",
        );
        text(doc, row, &format!("row {i} "));
        let span = doc.create_element("span");
        doc.set_attribute(span, "style", "background: rgb(0, 120, 0)");
        text(doc, span, "label");
        doc.append_child(row, span);
        el(
            doc,
            row,
            "position: absolute; right: 2px; top: 0; width: 8px; height: 2px; \
             opacity: 0.5; background: rgb(200, 0, 0)",
        );
        el(
            doc,
            row,
            "position: absolute; right: 12px; top: 0; width: 8px; height: 2px; \
             z-index: 2; background: rgb(0, 0, 200)",
        );
    }
    root
}

fn visible_page(doc: &mut RinchDocument, body: NodeId) {
    let p = el(doc, body, "width: 300px; background: rgb(230, 230, 255)");
    text(doc, p, "The page under the overlay.");
    el(
        doc,
        body,
        "position: absolute; left: 220px; top: 100px; width: 40px; height: 40px; \
         opacity: 0.6; background: rgb(0, 0, 0)",
    );
}

/// A closed overlay of 60 rows (all on screen) costs paint **nothing**: the
/// same visits and the same layers as the page with no overlay at all, and the
/// same pixels as a paint that walks it. Walked, every node in it was entered
/// and each translucent badge pushed an empty group layer.
#[test]
fn a_closed_overlay_costs_paint_nothing() {
    for shape in [Overlay::Drawer, Overlay::Popover, Overlay::InFlow] {
        // The page alone, with the in-flow panel's visible wrapper, which the
        // walk enters, and whose height is the panel's (a hidden box keeps
        // its box).
        let (mut bare, body) = page();
        if let Overlay::InFlow = shape {
            el(&mut bare, body, "width: 200px; height: 180px");
        }
        visible_page(&mut bare, body);
        let bare = shoot(&mut bare, 1.0);

        let (mut doc, body) = page();
        closed_overlay(&mut doc, body, 60, shape);
        visible_page(&mut doc, body);
        let (on, off) = both(&mut doc, 1.0);
        assert_same_picture(&on, &off, &format!("closed {shape:?}"));
        assert_eq!(on.pixels, bare.pixels, "a closed {shape:?} shows nothing");
        assert!(
            off.visited > bare.visited + 150 && off.layers >= bare.layers + 60,
            "positive control: walked, the {shape:?} costs {} visits and {} layers over the \
             page's {} and {}",
            off.visited,
            off.layers,
            bare.visited,
            bare.layers
        );
        assert_eq!(
            (on.visited, on.layers),
            (bare.visited, bare.layers),
            "skipped, the {shape:?} costs no visit and no layer"
        );
    }
}

/// The stacking collector hoists nothing out of a closed `Popover`: the body's
/// sequence is the page's own, for paint and for hit testing. Walked, it held
/// the overlay's root, its 60 rows and their 120 badges.
#[test]
fn a_closed_overlay_hoists_nothing() {
    let entries = |overlay: bool| {
        let (mut doc, body) = page();
        visible_page(&mut doc, body);
        if overlay {
            closed_overlay(&mut doc, body, 60, Overlay::Popover);
        }
        doc.resolve_layout(VP.0, VP.1);
        let order =
            rinch_dom::stacking::stacking_paint_order(&doc.tree, doc.tree.body_id, 1.0, 0.0, 0.0);
        let hit =
            rinch_dom::stacking::stacking_hit_order(&doc.tree, doc.tree.body_id, 1.0, 0.0, 0.0);
        let walked = {
            let _off = CullOff::new();
            rinch_dom::stacking::stacking_paint_order(&doc.tree, doc.tree.body_id, 1.0, 0.0, 0.0)
                .len()
        };
        (order.len(), hit.len(), walked)
    };
    let (bare, bare_hit, _) = entries(false);
    let (with, with_hit, walked) = entries(true);
    assert_eq!(
        walked,
        bare + 1 + 60 + 120,
        "positive control: walked, the overlay's root, rows and badges are entries"
    );
    assert_eq!((with, with_hit), (bare, bare_hit));
}

// ── What must still paint ──────────────────────────────────────────────────

/// A hidden subtree with a `visibility: visible` descendant is walked, and the
/// descendant paints — in flow, absolutely positioned against a box outside
/// the subtree, fixed, inside a hidden IFC, and under a hidden
/// `display: contents` wrapper — while a hidden sibling subtree with nothing
/// visible is skipped.
#[test]
fn visible_descendants_of_hidden_subtrees_still_paint() {
    let (mut doc, body) = page();
    let cb = el(
        &mut doc,
        body,
        "position: relative; width: 300px; height: 230px",
    );
    let hidden = el(
        &mut doc,
        cb,
        "visibility: hidden; background: rgb(255, 0, 0); padding: 4px",
    );
    let inner = el(&mut doc, hidden, "height: 20px; background: rgb(255, 0, 0)");
    el(
        &mut doc,
        inner,
        "visibility: visible; width: 30px; height: 10px; background: rgb(0, 0, 255)",
    );
    // Absolute against `cb`, outside the hidden subtree.
    el(
        &mut doc,
        inner,
        "visibility: visible; position: absolute; left: 200px; top: 150px; width: 20px; \
         height: 20px; background: rgb(0, 160, 0)",
    );
    // Fixed, against the viewport.
    el(
        &mut doc,
        inner,
        "visibility: visible; position: fixed; left: 280px; top: 200px; width: 20px; \
         height: 20px; z-index: 3; background: rgb(0, 160, 160)",
    );
    let p = el(&mut doc, hidden, "background: rgb(255, 0, 0)");
    text(&mut doc, p, "hidden ");
    let span = doc.create_element("span");
    doc.set_attribute(
        span,
        "style",
        "visibility: visible; background: rgb(160, 0, 160)",
    );
    text(&mut doc, span, "shown");
    doc.append_child(p, span);
    let contents = el(&mut doc, hidden, "display: contents; visibility: hidden");
    el(
        &mut doc,
        contents,
        "visibility: visible; width: 40px; height: 10px; background: rgb(160, 160, 0)",
    );
    // A hidden sibling with nothing visible, holding a translucent hoisted box.
    let none = el(
        &mut doc,
        cb,
        "visibility: hidden; height: 30px; background: rgb(255, 0, 0)",
    );
    el(
        &mut doc,
        none,
        "position: absolute; left: 100px; top: 100px; width: 30px; height: 30px; \
         opacity: 0.5; background: rgb(255, 0, 0)",
    );

    let (on, off) = both(&mut doc, 1.0);
    assert_same_picture(&on, &off, "visible descendants");
    let has = |rgb: [u8; 3]| {
        on.pixels
            .chunks(4)
            .any(|p| p[0] == rgb[0] && p[1] == rgb[1] && p[2] == rgb[2] && p[3] == 255)
    };
    for (rgb, what) in [
        ([0, 0, 255], "the in-flow visible child"),
        ([0, 160, 0], "the absolute one"),
        ([0, 160, 160], "the fixed one"),
        ([160, 0, 160], "the visible span's background"),
        ([160, 160, 0], "the child of the hidden contents wrapper"),
    ] {
        assert!(has(rgb), "{what} paints");
    }
    assert!(!has([255, 0, 0]), "nothing hidden paints");
    assert!(
        on.layers < off.layers,
        "the hidden sibling's layer is skipped"
    );
}

/// A scroller's bars are drawn after its last contained entry (#1717). A
/// hidden entry can be that last one, and the collector keeps it there, so a
/// box that sorts between it and the visible entries stays under the bars.
#[test]
fn a_hidden_last_entry_keeps_its_scrollers_bars_where_they_were() {
    let bars_over = |hidden_last: bool| {
        let (mut doc, body) = page();
        let s = el(
            &mut doc,
            body,
            "position: relative; left: 10px; top: 10px; width: 100px; height: 100px; \
             overflow-y: scroll",
        );
        el(&mut doc, s, "height: 400px");
        el(
            &mut doc,
            s,
            "position: absolute; left: 0; top: 0; width: 100px; height: 100px; z-index: 1; \
             background: rgb(0, 160, 0)",
        );
        if hidden_last {
            let h = el(
                &mut doc,
                s,
                "position: absolute; left: 0; top: 0; width: 10px; height: 10px; z-index: 3; \
                 visibility: hidden; background: rgb(255, 0, 0)",
            );
            el(
                &mut doc,
                h,
                "width: 5px; height: 5px; background: rgb(255, 0, 0)",
            );
        }
        // Over the bar, between the two entries.
        el(
            &mut doc,
            body,
            "position: absolute; left: 90px; top: 10px; width: 40px; height: 100px; \
             z-index: 2; background: rgb(0, 0, 200)",
        );
        let (on, off) = both(&mut doc, 1.0);
        assert_same_picture(&on, &off, "hidden last entry");
        // The column the bar is drawn in.
        let w = VP.0 as usize;
        (12..30)
            .map(|y| {
                let i = (y * w + 105) * 4;
                [on.pixels[i], on.pixels[i + 1], on.pixels[i + 2]]
            })
            .collect::<Vec<_>>()
    };
    let bar_over_box = bars_over(true);
    assert!(
        bar_over_box.iter().any(|&p| p != [0, 0, 200]),
        "control: the bar is drawn over the box: {bar_over_box:?}"
    );
    assert!(
        bars_over(false).iter().all(|&p| p == [0, 0, 200]),
        "control: with no entry past the box, the box is over the bar"
    );
}

/// While a paint-order watch is on (#1865 S4: the runtime placing a change
/// against a split GPU surface), a watched node in a hidden subtree is still
/// started, where it always was: the skip stands down for that paint.
#[test]
fn a_watched_hidden_node_is_still_started() {
    let (mut doc, body) = page();
    let hidden = el(
        &mut doc,
        body,
        "visibility: hidden; width: 50px; height: 50px",
    );
    let kid = el(
        &mut doc,
        hidden,
        "width: 20px; height: 20px; background: red",
    );
    doc.resolve_layout(VP.0, VP.1);
    let started = |doc: &mut RinchDocument| {
        let mut c = Commands::default();
        let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut c,
            1.0,
            VP,
            &mut doc.font_cx,
            &mut layout_cx,
        );
        c.started
    };
    rinch_dom::paint::set_paint_order_watch(&doc.tree, &[kid.0], true);
    let watched = started(&mut doc);
    rinch_dom::paint::set_paint_order_watch(&doc.tree, &[kid.0], false);
    assert_eq!(watched, vec![kid.0], "the watched node is started");
    let _ = hidden;
}

/// A panel built closed and opened by a class change, holding bare text beside
/// a block: the text sits in an anonymous block box, which copied the panel's
/// `hidden` at its layout and keeps it, since opening is paint-only. The text
/// shows once the panel opens.
#[test]
fn bare_text_of_a_panel_built_closed_shows_when_it_opens() {
    let (mut doc, body) = page();
    doc.load_css(".shut { visibility: hidden; } .kid { width: 40px; height: 10px; }");
    let panel = doc.create_element("div");
    doc.set_attribute(panel, "class", "shut");
    doc.append_child(body, panel);
    text(&mut doc, panel, "Hello");
    el(&mut doc, panel, "width: 40px; height: 10px");
    let closed = shoot(&mut doc, 1.0);
    assert_eq!(ink(&closed.pixels), 0, "closed");
    doc.set_attribute(panel, "class", "");
    let (on, off) = both(&mut doc, 1.0);
    assert!(ink(&off.pixels) > 100, "control: walked, the text shows");
    assert_same_picture(&on, &off, "opened");
}

/// While a paint-order watch is on, a watched **positioned** node in a hidden
/// subtree is started too: the collector, which would otherwise not hoist
/// it, stands down with paint.
#[test]
fn a_watched_hidden_positioned_node_is_still_started() {
    let (mut doc, body) = page();
    let hidden = el(
        &mut doc,
        body,
        "visibility: hidden; width: 50px; height: 50px",
    );
    let kid = el(
        &mut doc,
        hidden,
        "position: absolute; left: 5px; top: 5px; width: 20px; height: 20px; \
         z-index: 1; background: red",
    );
    doc.resolve_layout(VP.0, VP.1);
    let mut c = Commands::default();
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::set_paint_order_watch(&doc.tree, &[kid.0], true);
    rinch_dom::paint::paint_document(&doc.tree, &mut c, 1.0, VP, &mut doc.font_cx, &mut layout_cx);
    rinch_dom::paint::set_paint_order_watch(&doc.tree, &[kid.0], false);
    assert_eq!(
        c.started,
        vec![kid.0],
        "the watched positioned node is started"
    );
}

// ── Transitions: the memo moves with the value ─────────────────────────────

const TRANSITION_CSS: &str = "
    .root { visibility: hidden; width: 200px; height: 60px; }
    .kid { visibility: hidden; width: 40px; height: 20px; background: rgb(0, 0, 220); }
    .kid.on { visibility: visible; transition: visibility 100ms linear; }
    .panel { width: 200px; height: 60px; background: rgb(0, 120, 0); }
    .panel.shut { visibility: hidden; transition: visibility 0s linear 300ms; }
    .fill { width: 50px; height: 20px; background: rgb(200, 0, 0); }
";

fn tick(doc: &mut RinchDocument, at_ms: f64) {
    rinch_dom::transition::tick_transitions(&mut doc.tree, at_ms);
}

fn start_of(doc: &RinchDocument, node: NodeId) -> f64 {
    doc.tree.active_transitions[&node.0][&rinch_dom::transition::TransitionProperty::Visibility]
        .start_time_ms
}

fn blue(px: &[u8]) -> usize {
    px.chunks(4)
        .filter(|p| p[0] == 0 && p[1] == 0 && p[2] == 220)
        .count()
}

/// A hidden root whose hidden child turns `visible` by a transition **tick**:
/// the root stays hidden and its memo said "nothing below" at every paint
/// before, so the next paint must see the tick's value. The tick moves the hit
/// cache's generation (`HitStyleKey` holds `visibility`); `resolve_layout`
/// after it takes the paint-only path, which moves nothing, so that tick is
/// the only thing that can.
#[test]
fn a_tick_that_reveals_a_descendant_reaches_paint() {
    let (mut doc, body) = page();
    doc.load_css(TRANSITION_CSS);
    doc.tree.transitions_enabled = true;
    let root = doc.create_element("div");
    doc.set_attribute(root, "class", "root");
    doc.append_child(body, root);
    let kid = doc.create_element("div");
    doc.set_attribute(kid, "class", "kid");
    doc.append_child(root, kid);
    let shot = shoot(&mut doc, 1.0);
    assert_eq!(blue(&shot.pixels), 0, "closed");

    doc.set_attribute(kid, "class", "kid on");
    let shot = shoot(&mut doc, 1.0);
    assert_eq!(
        blue(&shot.pixels),
        0,
        "at progress 0 the kid is still hidden"
    );

    let start = start_of(&doc, kid);
    tick(&mut doc, start + 10.0);
    let (on, off) = both(&mut doc, 1.0);
    assert_eq!(
        blue(&off.pixels),
        40 * 20,
        "control: the tick made the kid visible"
    );
    assert_same_picture(&on, &off, "after the revealing tick");
}

/// The overlay idiom (`Drawer`, `Popover`): closing holds the root `visible`
/// for the 300ms delay, everything under it painted; the tick at the end hides
/// it, and the next paint skips it all. Reopening shows it at once.
#[test]
fn a_closing_overlay_paints_through_its_delay_and_is_skipped_after() {
    let (mut doc, body) = page();
    doc.load_css(TRANSITION_CSS);
    doc.tree.transitions_enabled = true;
    let panel = doc.create_element("div");
    doc.set_attribute(panel, "class", "panel");
    doc.append_child(body, panel);
    for _ in 0..20 {
        let f = doc.create_element("div");
        doc.set_attribute(f, "class", "fill");
        doc.append_child(panel, f);
    }
    let open = shoot(&mut doc, 1.0);
    assert!(ink(&open.pixels) > 0);

    doc.set_attribute(panel, "class", "panel shut");
    let (on, off) = both(&mut doc, 1.0);
    assert_same_picture(&on, &off, "closing, held visible");
    assert_eq!(on.pixels, open.pixels, "held visible through the delay");
    let start = start_of(&doc, panel);
    tick(&mut doc, start + 150.0);
    let (on, off) = both(&mut doc, 1.0);
    assert_same_picture(&on, &off, "mid-delay");
    assert_eq!(on.pixels, open.pixels);

    tick(&mut doc, start + 310.0);
    let (on, off) = both(&mut doc, 1.0);
    assert_same_picture(&on, &off, "closed by the tick");
    assert_eq!(ink(&on.pixels), 0, "closed");
    assert!(
        on.visited < off.visited,
        "positive control: the closed panel is skipped ({} against {})",
        on.visited,
        off.visited
    );

    doc.set_attribute(panel, "class", "panel");
    let (on, off) = both(&mut doc, 1.0);
    assert_same_picture(&on, &off, "reopened");
    assert_eq!(on.pixels, open.pixels, "reopened at once");
}

// ── A render surface in a closed overlay ───────────────────────────────────

/// A `RenderSurface` with a frame, inside a closed overlay, draws nothing — it
/// is its own visual (#829) — and a visible one beside it draws its frame.
#[test]
fn a_hidden_render_surface_draws_nothing() {
    use std::collections::HashMap;
    use std::sync::Arc;
    let (mut doc, body) = page();
    let hidden = el(&mut doc, body, "visibility: hidden");
    for (parent, id) in [(hidden, "1"), (body, "2")] {
        let s = doc.create_element("div");
        doc.set_attribute(s, "data-render-surface", id);
        doc.set_attribute(s, "style", "width: 40px; height: 40px");
        doc.append_child(parent, s);
    }
    let frame = rinch_dom::paint::SurfacePixelData {
        data: Arc::new([0u8, 200, 0, 255].repeat(16 * 16)),
        width: 16,
        height: 16,
        opaque: true,
    };
    rinch_dom::paint::set_surface_pixels(Some(HashMap::from([(1, frame.clone()), (2, frame)])));
    let (on, off) = both(&mut doc, 1.0);
    rinch_dom::paint::set_surface_pixels(None);
    assert_same_picture(&on, &off, "surfaces");
    assert_eq!(
        ink(&on.pixels),
        40 * 40,
        "the visible surface's frame, only"
    );
}

// ── Random documents ───────────────────────────────────────────────────────

struct Rng(u64);
impl Rng {
    fn pick(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }
    fn of<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.pick(xs.len() as u64) as usize]
    }
}

fn color(r: &mut Rng) -> String {
    format!(
        "rgba({}, {}, {}, 0.{})",
        r.pick(256),
        r.pick(256),
        r.pick(256),
        4 + r.pick(6)
    )
}

/// Hidden often, `visible` now and then (an override under a hidden
/// ancestor), unset otherwise.
fn visibility(r: &mut Rng) -> &'static str {
    r.of(&[
        "",
        "",
        "visibility: hidden;",
        "visibility: hidden;",
        "visibility: collapse;",
        "visibility: visible;",
    ])
}

fn random_box(doc: &mut RinchDocument, r: &mut Rng, parent: NodeId, depth: u32) {
    let kind = r.pick(if depth >= 3 { 3 } else { 8 });
    let vis = visibility(r);
    let bg = color(r);
    match kind {
        // A paragraph with a span that may differ in visibility.
        0 => {
            let p = el(doc, parent, &format!("{vis} background: {bg}"));
            text(doc, p, "Lorem ipsum ");
            let span = doc.create_element("span");
            let sv = visibility(r);
            let sb = color(r);
            doc.set_attribute(span, "style", &format!("{sv} background: {sb}"));
            text(doc, span, "dolor sit");
            doc.append_child(p, span);
            text(doc, p, " amet.");
        }
        // Bare text beside a block: an anonymous block box holds the text,
        // with a copy of its container's `visibility` taken at layout.
        2 => {
            let c = el(doc, parent, &format!("{vis} background: {bg}"));
            text(doc, c, "Bare text ");
            el(
                doc,
                c,
                &format!(
                    "{} width: {}px; height: 6px; background: {}",
                    visibility(r),
                    10 + r.pick(60),
                    color(r)
                ),
            );
        }
        // A leaf box with a shadow or an outline.
        1 => {
            let deco = r.of(&[
                "",
                "box-shadow: 3px 3px 4px rgb(0, 80, 80);",
                "outline: 2px solid rgb(90, 0, 0);",
                "border: 2px solid rgb(0, 0, 0); border-radius: 5px;",
            ]);
            el(
                doc,
                parent,
                &format!(
                    "{vis} width: {}px; height: {}px; background: {bg}; {deco}",
                    10 + r.pick(80),
                    4 + r.pick(30)
                ),
            );
        }
        // A positioned box: hoisted, maybe a stacking context.
        3 | 4 => {
            let position = r.of(&["relative", "absolute", "absolute", "fixed"]);
            let z = r.of(&["", "z-index: -1;", "z-index: 0;", "z-index: 2;"]);
            let opacity = r.of(&["", "", "opacity: 0.6;"]);
            let b = el(
                doc,
                parent,
                &format!(
                    "{vis} position: {position}; left: {}px; top: {}px; width: {}px; \
                     height: {}px; background: {bg}; {z} {opacity}",
                    r.pick(200) as i64 - 20,
                    r.pick(160) as i64 - 20,
                    10 + r.pick(90),
                    10 + r.pick(60)
                ),
            );
            for _ in 0..r.pick(3) {
                random_box(doc, r, b, depth + 1);
            }
        }
        // A scroller, which draws bars.
        5 => {
            let s = el(
                doc,
                parent,
                &format!(
                    "{vis} width: {}px; height: {}px; overflow: {}; background: {bg}",
                    40 + r.pick(120),
                    30 + r.pick(60),
                    r.of(&["auto", "scroll", "hidden"])
                ),
            );
            el(doc, s, "height: 300px; width: 10px");
            for _ in 0..1 + r.pick(3) {
                random_box(doc, r, s, depth + 1);
            }
        }
        // A `display: contents` wrapper.
        6 => {
            let c = el(doc, parent, &format!("{vis} display: contents"));
            for _ in 0..1 + r.pick(3) {
                random_box(doc, r, c, depth + 1);
            }
        }
        // A container, maybe transformed or translucent.
        _ => {
            let extra = r.of(&[
                "",
                "transform: translate(3px, 2px);",
                "opacity: 0.8;",
                "display: inline-block; width: 60px;",
                "padding: 3px;",
            ]);
            let c = el(doc, parent, &format!("{vis} background: {bg}; {extra}"));
            for _ in 0..1 + r.pick(4) {
                random_box(doc, r, c, depth + 1);
            }
        }
    }
}

fn random_doc(seed: u64) -> RinchDocument {
    let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x2137);
    let (mut doc, body) = page();
    for _ in 0..2 + r.pick(4) {
        random_box(&mut doc, &mut r, body, 0);
    }
    doc
}

/// The random documents again, changed after their first layout: visibility
/// toggled by class and by `style` on random elements, opened, then closed
/// again, each state painted with the skip and without it. A document whose
/// first layout saw a subtree hidden keeps state from that layout (an
/// anonymous block box's copy of its container's `visibility`), which a
/// document painted once can never exercise.
#[test]
fn random_documents_changed_after_layout_paint_the_same() {
    let mut changed = 0;
    for seed in 0..120u64 {
        let mut doc = random_doc(seed);
        doc.load_css(".h { visibility: hidden; } .v { visibility: visible; }");
        let scale = [1.0, 1.5][seed as usize % 2];
        let (on, off) = both(&mut doc, scale);
        assert_same_picture(&on, &off, &format!("seed {seed}, as built"));
        let mut r = Rng(seed ^ 0x5eed);
        let elements: Vec<NodeId> = doc
            .tree
            .nodes
            .iter()
            .filter(|(id, n)| n.is_element() && *id != doc.tree.body_id && n.parent.is_some())
            .map(|(id, _)| NodeId(id))
            .collect();
        let picked: Vec<NodeId> = (0..1 + r.pick(4))
            .map(|_| elements[r.pick(elements.len() as u64) as usize])
            .collect();
        for (step, (class, style)) in [
            ("v", "visible"),
            ("h", "hidden"),
            ("", "visible"),
            ("h", "inherit"),
        ]
        .into_iter()
        .enumerate()
        {
            for (i, &e) in picked.iter().enumerate() {
                if (i + step) % 2 == 0 {
                    doc.set_attribute(e, "class", class);
                } else {
                    doc.set_style(e, "visibility", style);
                }
            }
            let before = on.pixels.clone();
            let (on2, off2) = both(&mut doc, scale);
            assert_same_picture(&on2, &off2, &format!("seed {seed}, step {step}"));
            if on2.pixels != before {
                changed += 1;
            }
        }
    }
    assert!(
        changed >= 100,
        "the changes moved the picture {changed} times"
    );
}

/// Random documents of hidden and visible subtrees — `visible` overrides under
/// hidden ancestors, hoisted and fixed boxes, stacking contexts, scrollers,
/// `display: contents`, IFC spans — painted with the skip and without it at
/// four scales: the pictures are identical. Positive controls: the skip fired
/// on many of them, and many hid something that a descendant overrode.
#[test]
fn random_documents_paint_the_same_with_and_without_the_skip() {
    let (mut skipped, mut overridden) = (0, 0);
    for seed in 0..200u64 {
        let mut doc = random_doc(seed);
        let scale = [1.0, 1.5, 2.0, 1.25][seed as usize % 4];
        let (on, off) = both(&mut doc, scale);
        assert_same_picture(&on, &off, &format!("seed {seed}"));
        assert!(on.visited <= off.visited, "seed {seed}");
        if on.visited < off.visited {
            skipped += 1;
        }
        let tree = &doc.tree;
        if tree.nodes.iter().any(|(id, n)| {
            !n.is_text()
                && rinch_dom::visibility::hides_own(tree, n)
                && rinch_dom::visibility::shows_below(tree, id)
        }) {
            overridden += 1;
        }
    }
    assert!(skipped >= 60, "the skip fired on {skipped} of 200");
    assert!(
        overridden >= 40,
        "{overridden} of 200 had a visible override"
    );
}

/// Two translucent rows under one rounded clip, `z` 1 and 3, with a hidden box
/// at `z` 2 under another clip between them. Walked, the hidden entry splits
/// the rows into a clip layer each; skipped, they share one. On the software
/// painter the picture is the same either way (on Vello the shared layer's
/// edge antialiases once: `gpu_a_hidden_entry_between_two_runs_of_one_clip`).
#[test]
fn a_hidden_entry_between_two_runs_of_one_clip_paints_the_same() {
    let (mut doc, body) = page();
    let row = "position: absolute; left: 0; top: 0; width: 100%; height: 100%";
    let a = el(
        &mut doc,
        body,
        "position: absolute; left: 13px; top: 9px; width: 181px; height: 170px; \
         overflow: hidden; border-radius: 30px",
    );
    let b = el(
        &mut doc,
        body,
        "position: absolute; left: 150px; top: 9px; width: 90px; height: 170px; \
         overflow: hidden",
    );
    el(
        &mut doc,
        a,
        &format!("{row}; z-index: 1; background: rgba(0, 160, 0, 0.6)"),
    );
    el(
        &mut doc,
        b,
        &format!("{row}; z-index: 2; visibility: hidden; background: red"),
    );
    el(
        &mut doc,
        a,
        &format!("{row}; z-index: 3; background: rgba(0, 0, 200, 0.6)"),
    );
    for scale in [1.0, 1.5] {
        let (on, off) = both(&mut doc, scale);
        assert_same_picture(&on, &off, &format!("scale {scale}"));
        assert!(ink(&on.pixels) > 20_000, "control: the rows paint");
    }
}
