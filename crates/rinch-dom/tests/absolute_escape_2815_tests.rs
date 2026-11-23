//! A `position: absolute` box is clipped only by the clippers between it
//! and its containing block (#2815).
//!
//! An absolute entry's clip chain is truncated at its containing block
//! (`stacking::Collector::span`), which is right while that block is inside
//! the stacking context collecting the entry. When the block is **above**
//! it, the context's own clip bracket and the chains of the contexts it was
//! hoisted through were left on around the box: under `containing block >
//! clipper > opacity context > absolute` the box was cut to the clipper,
//! where Chrome 158 paints it whole. Paint now lifts those clips around the
//! entry, as it does for a fixed box (#549), down to the clips of the
//! containing block and its ancestors, which stay.
//!
//! Every fixture is painted twice, as `fixed_escape_549_tests` does: lifting
//! in place (the software painter in an app) and through [`NoLift`] (the
//! route Vello takes: the clips popped and pushed back, the context's group
//! opened under them). The two must agree within two levels. The Chrome
//! values are Chrome 158's headless screenshot of the same markup, each
//! fixture at the page origin; Skia rounds an `opacity` composite up to two
//! levels away from tiny-skia here.

#![cfg(feature = "software-renderer")]

use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, Stroke};
use peniko::{Brush, Fill};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::{
    BlendMode, GlyphSynthesis, PaintGlyph, PaintImage, PaintShape, Painter,
};
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 600.0;
const VH: f32 = 400.0;

/// The software painter with [`Painter::lift_clips`] left at the trait's
/// default, so paint takes the route a painter that cannot lift in place
/// takes (Vello's): the clips a fixed or absolute box escapes are popped and pushed back,
/// and its context's group is opened under them (#549).
struct NoLift(TinySkiaPainter);

impl Painter for NoLift {
    fn reset(&mut self) {
        self.0.reset()
    }
    fn fill(&mut self, f: Fill, t: Affine, b: &Brush, s: &PaintShape) {
        self.0.fill(f, t, b, s)
    }
    fn set_coarse_curves(&mut self, coarse: bool) {
        self.0.set_coarse_curves(coarse)
    }
    fn fill_color(&mut self, f: Fill, t: Affine, c: AlphaColor<Srgb>, s: &PaintShape) {
        self.0.fill_color(f, t, c, s)
    }
    fn stroke(&mut self, st: &Stroke, t: Affine, b: &Brush, s: &PaintShape) {
        self.0.stroke(st, t, b, s)
    }
    fn stroke_color(&mut self, st: &Stroke, t: Affine, c: AlphaColor<Srgb>, s: &PaintShape) {
        self.0.stroke_color(st, t, c, s)
    }
    #[allow(clippy::too_many_arguments)]
    fn draw_glyphs(
        &mut self,
        font: &peniko::FontData,
        size: f32,
        t: Affine,
        gt: GlyphSynthesis,
        b: &Brush,
        hint: bool,
        coords: &[i16],
        glyphs: &[PaintGlyph],
    ) {
        self.0
            .draw_glyphs(font, size, t, gt, b, hint, coords, glyphs)
    }
    fn draw_image(&mut self, i: &PaintImage<'_>, t: Affine) {
        self.0.draw_image(i, t)
    }
    fn push_clip(&mut self, f: Fill, t: Affine, s: &PaintShape) {
        self.0.push_clip(f, t, s)
    }
    fn push_layer(&mut self, b: BlendMode, o: f32, t: Affine, s: &PaintShape) {
        self.0.push_layer(b, o, t, s)
    }
    fn push_isolated_layer(&mut self, o: f32, t: Affine, s: &PaintShape) {
        self.0.push_isolated_layer(o, t, s)
    }
    fn pop_layer(&mut self) {
        self.0.pop_layer()
    }
}

fn paint_with(doc: &mut RinchDocument, scale: f64, no_lift: bool) -> TinySkiaPainter {
    let painter = TinySkiaPainter::new((VW as f64 * scale) as u32, (VH as f64 * scale) as u32);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    let mut paint_into = |painter: &mut dyn Painter| {
        rinch_dom::paint::paint_document(
            &doc.tree,
            painter,
            scale,
            (VW, VH),
            &mut doc.font_cx,
            &mut layout_cx,
        )
    };
    if no_lift {
        let mut wrapped = NoLift(painter);
        paint_into(&mut wrapped);
        wrapped.0
    } else {
        let mut painter = painter;
        paint_into(&mut painter);
        painter
    }
}

/// `doc` painted by the software painter, which lifts an escaping box's clips in
/// place, after checking it against the same paint through [`NoLift`]. They
/// agree within two levels a channel: a clip applied inside a group rather
/// than over it rounds differently (`GroupLayer`'s doc).
fn paint(doc: &mut RinchDocument, scale: f64) -> TinySkiaPainter {
    let lifted = paint_with(doc, scale, false);
    let popped = paint_with(doc, scale, true);
    let w = lifted.width() as usize;
    for (i, (a, b)) in lifted.pixels().iter().zip(popped.pixels()).enumerate() {
        assert!(
            a.abs_diff(*b) <= 2,
            "lifted in place and popped disagree at ({}, {}) channel {}: {a} vs {b}",
            i / 4 % w,
            i / 4 / w,
            i % 4
        );
    }
    lifted
}

/// The pixel at CSS point `(x, y)`.
fn px(p: &TinySkiaPainter, (x, y): (u32, u32), scale: f64) -> [u8; 4] {
    let (x, y) = ((x as f64 * scale) as u32, (y as f64 * scale) as u32);
    let i = ((y * p.width() + x) * 4) as usize;
    let d = p.pixels();
    [d[i], d[i + 1], d[i + 2], d[i + 3]]
}

fn el(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let n = doc.create_element("div");
    doc.set_attribute(n, "style", style);
    doc.append_child(parent, n);
    n
}

/// The pixel at `at` composited over white, the page Chrome screenshots:
/// premultiplied `c + 255 - a`, per channel.
fn over_white(p: &TinySkiaPainter, at: (u32, u32), scale: f64) -> [u8; 3] {
    let [r, g, b, a] = px(p, at, scale);
    [r, g, b].map(|c| c.saturating_add(255 - a))
}

/// Within 2 of Chrome's pixel, per channel.
#[track_caller]
fn assert_near(got: [u8; 3], chrome: [u8; 3], what: &str) {
    let off = got
        .iter()
        .zip(chrome)
        .map(|(&g, c)| g.abs_diff(c))
        .max()
        .unwrap();
    assert!(off <= 2, "{what}: got {got:?}, Chrome draws {chrome:?}");
}

const WHITE: [u8; 3] = [255, 255, 255];
/// The green absolute box at `opacity: 0.5` over white, and over the
/// context's own blue background (it is drawn after it, in the same group).
const FADED_GREEN: [u8; 3] = [126, 227, 126];
/// The blue context background at `opacity: 0.5` over white.
const FADED_BLUE: [u8; 3] = [126, 126, 227];

/// A translucent context with a blue background, holding an absolute green
/// box that overlaps it and reaches far past the 100x100 clippers below.
const CONTEXT: &str = "opacity: 0.5; width: 50px; height: 50px; background-color: rgb(0, 0, 200)";
const ABSOLUTE: &str = "position: absolute; left: 25px; top: 25px; width: 200px; height: 200px; \
                        background-color: rgb(0, 200, 0)";
const CLIPPER: &str = "overflow: hidden; width: 100px; height: 100px";
const CB: &str = "position: relative; width: 300px; height: 300px";

/// Where the absolute box overlaps the context's background, inside every
/// clipper.
const OVERLAP: (u32, u32) = (40, 40);
/// The context's background alone.
const BG: (u32, u32) = (10, 10);
/// Inside the clippers, on the absolute box only.
const IN: (u32, u32) = (75, 75);
/// Outside the clippers, on the absolute box.
const OUT: (u32, u32) = (200, 200);

/// `body > s[0] > s[1] > … `, laid out; the last style is the innermost
/// box. Returns every node.
fn chain(styles: &[&str]) -> (RinchDocument, Vec<NodeId>) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let mut nodes = Vec::new();
    let mut parent = body;
    for s in styles {
        parent = el(&mut doc, parent, s);
        nodes.push(parent);
    }
    doc.resolve_layout(VW, VH);
    (doc, nodes)
}

/// **The issue's shape**: `containing block > clipper > opacity context >
/// absolute`. The clipper is not between the box and its containing block,
/// so Chrome paints the box whole, faded with its context.
///
/// Kills: no lift for an absolute entry (`OUT` is white: the box is cut to
/// the clipper, which is `main` before #2815); and, through the [`NoLift`]
/// comparison, the context's group not sunk under the clips for an
/// escaping absolute (the overlap is faded twice).
#[test]
fn an_absolute_box_escapes_a_clipper_below_its_containing_block() {
    for scale in [1.0, 2.0] {
        let (mut doc, _) = chain(&[CB, CLIPPER, CONTEXT, ABSOLUTE]);
        let p = paint(&mut doc, scale);
        assert_near(over_white(&p, BG, scale), FADED_BLUE, "the context");
        assert_near(
            over_white(&p, OVERLAP, scale),
            FADED_GREEN,
            "over the context",
        );
        assert_near(over_white(&p, IN, scale), FADED_GREEN, "inside the clipper");
        assert_near(
            over_white(&p, OUT, scale),
            FADED_GREEN,
            &format!("outside the clipper, scale {scale}"),
        );
    }
}

/// With no positioned ancestor the containing block is the initial one
/// (#204): the clipper is escaped all the same (Chrome 158).
#[test]
fn an_absolute_box_with_no_positioned_ancestor_escapes_the_clipper() {
    let (mut doc, _) = chain(&[CLIPPER, CONTEXT, ABSOLUTE]);
    let p = paint(&mut doc, 1.0);
    assert_near(
        over_white(&p, OVERLAP, 1.0),
        FADED_GREEN,
        "over the context",
    );
    assert_near(over_white(&p, OUT, 1.0), FADED_GREEN, "outside the clipper");
}

/// **The containing block's own clip, and a clipper above it, stay.** The
/// containing block is inside the clipper here: Chrome cuts the box to it.
///
/// Kills: a lift that takes every clip off (as a fixed box's does), or one
/// that reaches past the containing block.
#[test]
fn a_clipper_above_the_containing_block_still_clips() {
    let cb = "position: relative; width: 100px; height: 100px";
    let (mut doc, _) = chain(&[CLIPPER, cb, CONTEXT, ABSOLUTE]);
    let p = paint(&mut doc, 1.0);
    assert_near(over_white(&p, IN, 1.0), FADED_GREEN, "positive control");
    assert_eq!(over_white(&p, OUT, 1.0), WHITE, "outside the clipper");

    let cb = "position: relative; overflow: hidden; width: 100px; height: 100px";
    let (mut doc, _) = chain(&[cb, "width: 300px; height: 300px", CONTEXT, ABSOLUTE]);
    let p = paint(&mut doc, 1.0);
    assert_near(over_white(&p, IN, 1.0), FADED_GREEN, "positive control");
    assert_eq!(
        over_white(&p, OUT, 1.0),
        WHITE,
        "outside the containing block's own clip"
    );
}

/// The context clips its own overflow and is not the box's containing
/// block: its own clip is escaped too (Chrome 158). This was `stacking.rs`'s
/// "Known gap": the collecting root's bracket was applied to every entry.
///
/// Kills: a lift that stops at the collecting root's bracket.
#[test]
fn an_absolute_box_escapes_the_clip_of_its_own_stacking_context() {
    let context = "opacity: 0.5; overflow: hidden; width: 100px; height: 100px; \
                   background-color: rgb(0, 0, 200)";
    let (mut doc, _) = chain(&[CB, context, ABSOLUTE]);
    let p = paint(&mut doc, 1.0);
    assert_near(over_white(&p, BG, 1.0), FADED_BLUE, "the context");
    assert_near(
        over_white(&p, OVERLAP, 1.0),
        FADED_GREEN,
        "over the context",
    );
    assert_near(over_white(&p, OUT, 1.0), FADED_GREEN, "outside its clip");
}

/// A fixed box contained by a transformed ancestor reads `absolute`
/// (#1372) and escapes the clippers below that ancestor the same way, even
/// where a positioned box between would contain an absolute one (Chrome
/// 158: `transform > clipper > relative z-index context > fixed`). The
/// identity transform at the origin keeps rinch's viewport placement and
/// Chrome's the same box.
///
/// Kills: asking a contained fixed box for its absolute containing block
/// (the context itself, so nothing is lifted).
#[test]
fn a_fixed_box_contained_by_a_transform_escapes_a_clipper_below_it() {
    let (mut doc, _) = chain(&[
        "transform: translate(0px, 0px); width: 300px; height: 300px",
        CLIPPER,
        "position: relative; z-index: 1; width: 50px; height: 50px",
        "position: fixed; left: 50px; top: 50px; width: 200px; height: 200px; \
         background-color: rgb(0, 200, 0)",
    ]);
    let p = paint(&mut doc, 1.0);
    assert_eq!(over_white(&p, IN, 1.0), [0, 200, 0], "inside the clipper");
    assert_eq!(over_white(&p, OUT, 1.0), [0, 200, 0], "outside the clipper");
}

/// The context sits below its clipper's viewport, past a tall spacer, so it
/// is outside every cull; its absolute box, placed against the containing
/// block above the clipper, is on screen (Chrome 158). The box lies wholly
/// outside the clipper and past the cull's ink margin, so a cull measuring
/// the context's subtree against that clip finds nothing to draw.
///
/// Kills: the off-window cull pruning the context (`subtree_is_entirely_outside`
/// measuring the box against the clip it escapes).
#[test]
fn an_absolute_box_under_a_context_outside_its_clipper_is_drawn() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let cb = el(&mut doc, body, CB);
    let clipper = el(&mut doc, cb, CLIPPER);
    el(&mut doc, clipper, "width: 10px; height: 1000px");
    let context = el(&mut doc, clipper, CONTEXT);
    el(
        &mut doc,
        context,
        "position: absolute; left: 250px; top: 250px; width: 100px; height: 100px; \
         background-color: rgb(0, 200, 0)",
    );
    doc.resolve_layout(VW, VH);
    let p = paint(&mut doc, 1.0);
    assert_eq!(over_white(&p, IN, 1.0), WHITE, "nothing inside the clipper");
    assert_near(
        over_white(&p, (300, 300), 1.0),
        FADED_GREEN,
        "the box outside it",
    );
}

/// **What is painted after the absolute box is clipped again**: a later
/// translucent sibling of the context reaches past the clipper and stays cut
/// to it.
///
/// Kills: not putting the lifted clips back after the entry.
#[test]
fn what_paints_after_the_absolute_box_is_clipped_again() {
    let (mut doc, nodes) = chain(&[CB, CLIPPER, CONTEXT, ABSOLUTE]);
    el(
        &mut doc,
        nodes[1],
        "opacity: 0.99; width: 500px; height: 30px; background-color: rgb(0, 0, 200)",
    );
    doc.resolve_layout(VW, VH);
    let p = paint(&mut doc, 1.0);
    assert_ne!(over_white(&p, (90, 60), 1.0), WHITE, "positive control");
    assert_ne!(
        over_white(&p, (90, 60), 1.0),
        FADED_GREEN,
        "the later box is on top"
    );
    assert_eq!(
        over_white(&p, (300, 60), 1.0),
        WHITE,
        "the later box past the clip"
    );
}

/// **An element with a `clip-path` between the clipper and the box keeps
/// the clipper's clip on** (Chrome 158): `containing block > clipper >
/// clip-path > context > absolute`, and the context carrying the
/// `clip-path` itself (an `inset(-100px)` that cuts nothing near the box),
/// draw nothing past the clipper. rinch paints no clip path, so these are
/// the clipper's clip alone.
///
/// Kills: a lift that does not stop at a `clip-path` element (the box is
/// drawn out to 225).
#[test]
fn a_clip_path_between_the_clipper_and_the_box_keeps_the_clipper() {
    for styles in [
        &[
            CB,
            CLIPPER,
            "clip-path: inset(0); width: 150px; height: 150px",
            CONTEXT,
            ABSOLUTE,
        ][..],
        &[
            CB,
            CLIPPER,
            "opacity: 0.5; width: 50px; height: 50px; background-color: rgb(0, 0, 200); \
             clip-path: inset(-100px)",
            ABSOLUTE,
        ][..],
    ] {
        let (mut doc, _) = chain(styles);
        let p = paint(&mut doc, 1.0);
        assert_near(
            over_white(&p, IN, 1.0),
            FADED_GREEN,
            &format!("positive control, {styles:?}"),
        );
        assert_eq!(
            over_white(&p, (120, 120), 1.0),
            WHITE,
            "past the clipper, {styles:?}"
        );
        assert_eq!(over_white(&p, OUT, 1.0), WHITE, "far past it, {styles:?}");
    }
}

/// …and with the `clip-path` element **above** the clipper the box escapes
/// the clipper (Chrome 158; the clip path, `inset(0)` on a 300x300 box,
/// cuts nothing the probes reach).
#[test]
fn a_clip_path_above_the_clipper_does_not_keep_it() {
    let (mut doc, _) = chain(&[
        CB,
        "clip-path: inset(0); width: 300px; height: 300px",
        CLIPPER,
        CONTEXT,
        ABSOLUTE,
    ]);
    let p = paint(&mut doc, 1.0);
    assert_near(
        over_white(&p, (120, 120), 1.0),
        FADED_GREEN,
        "past the clipper",
    );
    assert_near(over_white(&p, OUT, 1.0), FADED_GREEN, "far past it");
}

/// **A masked element between the clipper and the box keeps the clipper's
/// clip on** (Chrome 158), and one above it does not. The mask's border-box
/// clip is on the paint stack with no owner, and the lift stops there: the
/// clipper below it is not lifted although it is escapable.
///
/// Kills: the lift walking on past a clip that is not escapable (`break`
/// turned into `continue`: the clipper is lifted, the box drawn at
/// `(120, 120)`).
#[test]
fn a_mask_between_the_clipper_and_the_box_keeps_the_clipper() {
    let (mut doc, _) = chain(&[
        CB,
        CLIPPER,
        "mask-image: linear-gradient(#000, #000); width: 150px; height: 150px",
        CONTEXT,
        ABSOLUTE,
    ]);
    let p = paint(&mut doc, 1.0);
    assert_near(over_white(&p, IN, 1.0), FADED_GREEN, "positive control");
    assert_eq!(over_white(&p, (120, 120), 1.0), WHITE, "past the clipper");

    let (mut doc, _) = chain(&[
        CB,
        "mask-image: linear-gradient(#000, #000); width: 300px; height: 300px",
        CLIPPER,
        CONTEXT,
        ABSOLUTE,
    ]);
    let p = paint(&mut doc, 1.0);
    assert_near(
        over_white(&p, (120, 120), 1.0),
        FADED_GREEN,
        "a mask above the clipper: past the clipper",
    );
    assert_near(over_white(&p, OUT, 1.0), FADED_GREEN, "far past it");
}
