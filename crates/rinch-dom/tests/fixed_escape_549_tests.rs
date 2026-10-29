//! A `position: fixed` box escapes **every** clip between it and the viewport
//! — not only the clip of the stacking context that owns it (#549).
//!
//! A fixed box is hoisted to its nearest ancestor stacking context (#545), and
//! the paint walk used to lift exactly one clip around it: that context's own
//! bracket. The clips the context was itself hoisted past — the chain on its
//! entry, in whatever sequence hoisted it — stayed pushed around its whole
//! subtree, so under `plain clipper > stacking context > fixed` the fixed box
//! was cut away. Chrome 155 paints it (measured headless, `elementFromPoint`
//! and the screenshot's pixels, for every shape below).
//!
//! What still clips a fixed box is what CSS makes its containing block: an
//! ancestor with a `transform` (or a `filter`) contains it, so clips at and
//! above that ancestor still apply. rinch places such a box against the
//! viewport all the same (#1372); the fixture that pins the clip uses an
//! identity `translate(0, 0)`, where the two placements coincide. A
//! `mask-image` ancestor cuts it to its border box but is not its containing
//! block, so the masked box's own `overflow` does not clip it.
//!
//! An ancestor context's `opacity` applies to the fixed box, composited into
//! the one group with the rest of the context, as Chrome does; the second
//! round's fixtures pin that against Chrome's pixels.
//!
//! Every assertion reads the software painter. Off the fixed points: the
//! fixed box overlaps its clipper's edge rather than sitting wholly outside
//! it, probes are taken on both sides of the clip, and the main shape is
//! painted at scale 1 and 2.

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

const GREEN: [u8; 4] = [0, 200, 0, 255];
const BLUE: [u8; 4] = [0, 0, 200, 255];
const NOTHING: [u8; 4] = [0, 0, 0, 0];

/// The fixed box of every fixture: it straddles the 200x200 clippers' right
/// and bottom edges, so one probe is inside the clip and one is well outside.
const FIXED: &str = "position: fixed; left: 130px; top: 110px; width: 270px; height: 290px; \
                     background-color: rgb(0, 200, 0)";
/// Inside every clipper below, and on the fixed box.
const IN: (u32, u32) = (150, 130);
/// Outside every clipper below, and on the fixed box.
const OUT: (u32, u32) = (380, 380);

fn paint(doc: &mut RinchDocument, scale: f64) -> TinySkiaPainter {
    let mut painter = TinySkiaPainter::new((VW as f64 * scale) as u32, (VH as f64 * scale) as u32);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale,
        (VW, VH),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter
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

/// `body > s[0] > s[1] > … > fixed`, laid out. Returns every node, the fixed
/// box last.
fn chain(styles: &[&str]) -> (RinchDocument, Vec<NodeId>) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let mut nodes = Vec::new();
    let mut parent = body;
    for s in styles.iter().chain(std::iter::once(&FIXED)) {
        parent = el(&mut doc, parent, s);
        nodes.push(parent);
    }
    doc.resolve_layout(VW, VH);
    (doc, nodes)
}

const CLIPPER: &str = "overflow: hidden; width: 200px; height: 200px";
const OWNER: &str = "position: relative; z-index: 1; width: 50px; height: 50px";

/// **The issue's shape.** The owner is hoisted to the body past the clipper,
/// and the fixed box is painted inside the owner's sequence: Chrome paints it
/// outside the clipper too.
///
/// Kills: the lift removing only the owner's own bracket (the box is cut to
/// the clipper, `OUT` is empty) — which is `main` before #549.
#[test]
fn a_fixed_box_escapes_a_plain_clipper_above_its_stacking_context() {
    for scale in [1.0, 2.0] {
        let (mut doc, _) = chain(&[CLIPPER, OWNER]);
        let p = paint(&mut doc, scale);
        assert_eq!(
            px(&p, IN, scale),
            GREEN,
            "inside the clipper, scale {scale}"
        );
        assert_eq!(
            px(&p, OUT, scale),
            GREEN,
            "outside the clipper the fixed box is still drawn, scale {scale}"
        );
    }
}

/// The owner's own clip: escaped before #549 as well (#545), and still.
#[test]
fn a_fixed_box_escapes_the_clip_of_the_context_that_owns_it() {
    let (mut doc, _) = chain(&["position: relative; z-index: 1; overflow: hidden; \
                                width: 50px; height: 50px"]);
    let p = paint(&mut doc, 1.0);
    assert_eq!(px(&p, IN, 1.0), GREEN);
    assert_eq!(px(&p, OUT, 1.0), GREEN);
}

/// Two clippers above an `opacity` owner (`hidden` then `clip`): both are
/// escaped, and the owner's opacity still applies — the box is drawn exactly
/// as it is with no clipper at all.
///
/// Kills: re-pushing only the clips (the box is drawn opaque), and dropping
/// the opacity layer with the clips it sits above.
#[test]
fn a_fixed_box_escapes_two_clippers_and_keeps_its_contexts_opacity() {
    let owner = "opacity: 0.5; width: 50px; height: 50px";
    let (mut doc, _) = chain(&[
        CLIPPER,
        "overflow: clip; width: 150px; height: 150px",
        owner,
    ]);
    let p = paint(&mut doc, 1.0);
    let (mut bare, _) = chain(&[owner]);
    let reference = paint(&mut bare, 1.0);
    let faded = px(&reference, OUT, 1.0);
    assert!(
        faded[3] > 100 && faded[3] < 160 && faded[1] > 0,
        "positive control: the reference is the half-faded box, {faded:?}"
    );
    assert_eq!(px(&p, OUT, 1.0), faded, "outside both clippers");
    assert_eq!(px(&p, IN, 1.0), faded, "inside them");
}

/// A rounded clipper cuts its own corners; the fixed box covers the corner
/// and is not cut there.
///
/// Kills: the escape skipping rounded clips (a `RoundedRect` chain link left
/// pushed leaves the corner empty).
#[test]
fn a_fixed_box_is_not_cut_by_a_rounded_clipper_above_its_context() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let clipper = el(
        &mut doc,
        body,
        "overflow: hidden; border-radius: 60px; width: 200px; height: 200px",
    );
    let owner = el(&mut doc, clipper, OWNER);
    el(
        &mut doc,
        owner,
        "position: fixed; left: 0; top: 0; width: 300px; height: 300px; \
         background-color: rgb(0, 200, 0)",
    );
    doc.resolve_layout(VW, VH);
    let p = paint(&mut doc, 1.0);
    assert_eq!(px(&p, (4, 4), 1.0), GREEN, "in the clipper's cut corner");
    assert_eq!(px(&p, (100, 100), 1.0), GREEN, "inside the clipper");
    assert_eq!(px(&p, (250, 250), 1.0), GREEN, "outside it");
}

/// `clipper > owner > owner2 (clips) > fixed`: every level is escaped.
#[test]
fn a_fixed_box_escapes_every_level_of_nested_contexts() {
    let (mut doc, _) = chain(&[
        CLIPPER,
        "position: relative; z-index: 1; width: 100px; height: 100px",
        "position: relative; z-index: 1; overflow: hidden; width: 50px; height: 50px",
    ]);
    let p = paint(&mut doc, 1.0);
    assert_eq!(px(&p, IN, 1.0), GREEN);
    assert_eq!(px(&p, OUT, 1.0), GREEN);
}

/// **A transformed ancestor contains a fixed box**, so a clip above it still
/// applies: Chrome draws nothing at `OUT` here. The identity `translate`
/// keeps rinch's viewport placement and Chrome's transformed-ancestor
/// placement the same box, so this asserts the clip and nothing else.
///
/// Kills: the escape running past a transform (the box is drawn at `OUT`).
#[test]
fn a_transformed_ancestor_keeps_the_clip_above_it() {
    for between in [
        "transform: translate(0px, 0px); width: 100px; height: 100px",
        "filter: brightness(0.9); width: 100px; height: 100px",
    ] {
        let (mut doc, _) = chain(&[CLIPPER, between, OWNER]);
        let p = paint(&mut doc, 1.0);
        assert_ne!(px(&p, IN, 1.0), NOTHING, "positive control, {between}");
        assert_eq!(px(&p, OUT, 1.0), NOTHING, "clipped above {between}");
    }
}

/// A `mask-image` ancestor clips everything it paints to its border box,
/// fixed descendants included, and the clip above it stays with it.
#[test]
fn a_masked_ancestor_keeps_the_clip_above_it() {
    let (mut doc, _) = chain(&[
        CLIPPER,
        "mask-image: linear-gradient(#000, #000); width: 300px; height: 300px",
        OWNER,
    ]);
    let p = paint(&mut doc, 1.0);
    assert_eq!(px(&p, IN, 1.0), GREEN, "positive control");
    assert_eq!(px(&p, OUT, 1.0), NOTHING);
}

/// **What is painted after the fixed box is clipped again.** The owner's
/// later `z-index: 2` child and a body-level box after the clipper both reach
/// past the clipper; they stay cut to it.
///
/// Kills: not putting the lifted clips back after the fixed entry (the blue
/// boxes are drawn past the clipper).
#[test]
fn what_paints_after_the_fixed_box_is_clipped_again() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let clipper = el(&mut doc, body, CLIPPER);
    let owner = el(&mut doc, clipper, OWNER);
    el(&mut doc, owner, FIXED);
    el(
        &mut doc,
        owner,
        "position: absolute; z-index: 2; left: 0; top: 20px; width: 700px; height: 30px; \
         background-color: rgb(0, 0, 200)",
    );
    // A later sibling of the owner in the clipper, hoisted to the body too.
    el(
        &mut doc,
        clipper,
        "position: relative; z-index: 3; top: 100px; width: 700px; height: 30px; \
         background-color: rgb(0, 0, 200)",
    );
    doc.resolve_layout(VW, VH);
    let p = paint(&mut doc, 1.0);
    assert_eq!(
        px(&p, (100, 30), 1.0),
        BLUE,
        "positive control, owner's child"
    );
    assert_eq!(
        px(&p, (600, 30), 1.0),
        NOTHING,
        "the owner's child past the clip"
    );
    assert_eq!(
        px(&p, (100, 160), 1.0),
        BLUE,
        "positive control, later sibling"
    );
    assert_eq!(
        px(&p, (600, 160), 1.0),
        NOTHING,
        "the later sibling past the clip"
    );
    assert_eq!(
        px(&p, OUT, 1.0),
        GREEN,
        "the fixed box between them escaped"
    );
}

/// The clipper is scrolled so the owner's own box is above its viewport: the
/// owner is then outside every cull but its fixed box is on screen.
#[test]
fn a_fixed_box_under_a_context_scrolled_out_of_its_clipper_is_drawn() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let clipper = el(&mut doc, body, CLIPPER);
    let owner = el(&mut doc, clipper, OWNER);
    el(&mut doc, owner, FIXED);
    el(&mut doc, clipper, "width: 10px; height: 2000px");
    doc.resolve_layout(VW, VH);
    doc.tree.nodes[clipper.0].scroll_offset = (0.0, 700.0);
    let p = paint(&mut doc, 1.0);
    assert_eq!(px(&p, IN, 1.0), GREEN);
    assert_eq!(px(&p, OUT, 1.0), GREEN);
}

/// The pixel at `at` composited over white, the page Chrome screenshots:
/// premultiplied `c + 255 - a`, per channel.
fn over_white(p: &TinySkiaPainter, at: (u32, u32)) -> [u8; 3] {
    let [r, g, b, a] = px(p, at, 1.0);
    [r, g, b].map(|c| c.saturating_add(255 - a))
}

/// Within 1 of Chrome's pixel, per channel: Skia and tiny-skia round an
/// `opacity` composite differently by at most one.
#[track_caller]
fn assert_near(got: [u8; 3], chrome: [u8; 3], what: &str) {
    let off = got
        .iter()
        .zip(chrome)
        .map(|(&g, c)| g.abs_diff(c))
        .max()
        .unwrap();
    assert!(off <= 1, "{what}: rinch {got:?}, Chrome 155 {chrome:?}");
}

/// **A transformed or filtered box's own `overflow` clips a fixed descendant**
/// — it is the descendant's containing block — whether the box owns the fixed
/// box's stacking context or an ancestor of it does. Chrome 155 (headless
/// screenshot): `OUT` white in all four.
///
/// Kills: taking the fixed floor before the box's own clip bracket is pushed
/// (`OUT` drawn — what #2057's first round did for the owner-deeper shapes, and
/// what `main` did for the direct one, #545's lift).
#[test]
fn a_transformed_or_filtered_box_clips_a_fixed_descendant_to_its_own_overflow() {
    for own in [
        "transform: translate(0px, 0px); overflow: hidden; width: 200px; height: 200px",
        "filter: brightness(0.9); overflow: hidden; width: 200px; height: 200px",
    ] {
        for shape in [&[own, OWNER][..], &[own][..]] {
            let (mut doc, _) = chain(shape);
            let p = paint(&mut doc, 1.0);
            assert_ne!(px(&p, IN, 1.0), NOTHING, "positive control, {shape:?}");
            assert_eq!(px(&p, OUT, 1.0), NOTHING, "clipped to {shape:?}");
        }
    }
}

/// **A masked box's own `overflow` does not clip a fixed descendant**: a mask
/// is not a containing block. Chrome 155 draws the fixed box over the masked
/// box's 30px border, outside its padding box (`(185, 185)` green, in both
/// shapes), and still cuts it at the border box (`OUT` white). The fixed box
/// is placed against the viewport too (`(140, 130)` green, which a padding-box
/// containing block would have moved 30px away).
///
/// Kills: treating a mask like a transform (the floor after the box's own
/// bracket: `(185, 185)` blue).
#[test]
fn a_masked_box_does_not_clip_a_fixed_descendant_to_its_own_overflow() {
    let masked = "mask-image: linear-gradient(#000, #000); overflow: hidden; width: 200px; \
                  height: 200px; border: 30px solid rgb(0, 0, 200); box-sizing: border-box";
    for shape in [&[masked, OWNER][..], &[masked][..]] {
        let (mut doc, _) = chain(shape);
        let p = paint(&mut doc, 1.0);
        assert_eq!(
            px(&p, (10, 10), 1.0),
            BLUE,
            "positive control: the border, {shape:?}"
        );
        assert_eq!(px(&p, (185, 185), 1.0), GREEN, "over the border, {shape:?}");
        assert_eq!(
            px(&p, (140, 130), 1.0),
            GREEN,
            "placed in the viewport, {shape:?}"
        );
        assert_eq!(
            px(&p, OUT, 1.0),
            NOTHING,
            "cut to the border box, {shape:?}"
        );
    }
}

/// **The fixed box is composited into its context's `opacity` group, and the
/// group is composited once** — under a clipper above the context, which the
/// fixed box escapes. `A` is painted before the fixed entry and `B` after it,
/// overlapping each other far from the fixed box: Chrome 155 draws `B` over
/// `A` and fades the two together, `(128, 128, 228)`.
///
/// Kills: re-pushing the group layer around the fixed entry, which composites
/// `A` and `B` as two groups (`(113, 63, 163)`, #2057's first round).
#[test]
fn a_fixed_box_does_not_split_its_contexts_opacity_group() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let clipper = el(
        &mut doc,
        body,
        "overflow: hidden; width: 400px; height: 400px",
    );
    let ctx = el(
        &mut doc,
        clipper,
        "opacity: 0.5; width: 300px; height: 300px",
    );
    el(
        &mut doc,
        ctx,
        "width: 200px; height: 200px; background-color: rgb(200, 0, 0)",
    );
    el(
        &mut doc,
        ctx,
        "position: fixed; left: 600px; top: 500px; width: 20px; height: 20px; \
         background-color: rgb(0, 200, 0)",
    );
    el(
        &mut doc,
        ctx,
        "position: relative; z-index: 1; top: -100px; width: 200px; height: 100px; \
         background-color: rgb(0, 0, 200)",
    );
    doc.resolve_layout(VW, VH);
    let p = paint(&mut doc, 1.0);
    assert_near(over_white(&p, (100, 150)), [128, 128, 228], "B over A");
    assert_near(over_white(&p, (100, 50)), [228, 128, 128], "A alone");
    assert_near(over_white(&p, (610, 510)), [128, 228, 128], "the fixed box");
}

/// **A fading card in a scroller, holding a fixed popover** — rinch's own
/// `Popover`/`Select` dropdowns are `position: fixed` — mid-fade. The card's
/// background is painted before the fixed entry and its `z-index: 1` badge
/// after it; the popover reaches past the scroller. Every pixel is Chrome
/// 155's (headless screenshot of the same markup).
///
/// Kills: the group split (`(100, 200)`, the badge over the card's background,
/// fades twice) and a popover cut to the scroller (`(420, 100)`).
#[test]
fn a_fixed_popover_in_a_fading_card_in_a_scroller() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let scroller = el(
        &mut doc,
        body,
        "overflow: auto; width: 400px; height: 400px",
    );
    let card = el(
        &mut doc,
        scroller,
        "opacity: 0.6; width: 300px; height: 300px; background-color: rgb(200, 0, 0)",
    );
    el(
        &mut doc,
        card,
        "position: relative; z-index: 1; width: 200px; height: 100px; margin-top: 150px; \
         background-color: rgb(0, 0, 200)",
    );
    el(
        &mut doc,
        card,
        "position: fixed; left: 150px; top: 50px; width: 300px; height: 140px; \
         background-color: rgb(0, 200, 0)",
    );
    doc.resolve_layout(VW, VH);
    let p = paint(&mut doc, 1.0);
    for (at, chrome, what) in [
        ((170, 170), [102, 102, 222], "badge over popover over card"),
        ((100, 200), [102, 102, 222], "badge over card"),
        ((300, 100), [102, 222, 102], "popover"),
        ((420, 100), [102, 222, 102], "popover past the scroller"),
        (
            (50, 50),
            [255, 255, 255],
            "above the card (its margin collapsed)",
        ),
    ] {
        assert_near(over_white(&p, at), chrome, what);
    }
}

/// **A modal fading in, with a fixed tooltip in its scrolling dialog.** The
/// overlay is the `opacity` context and the dialog's clip is inside it, so the
/// lift pops only that clip. Every pixel is Chrome 155's.
///
/// Kills: a tooltip cut to the dialog (`(430, 280)`).
#[test]
fn a_fixed_tooltip_in_a_fading_modal() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let overlay = el(
        &mut doc,
        body,
        "position: fixed; left: 0; top: 0; width: 800px; height: 600px; opacity: 0.5; \
         background-color: rgba(0, 0, 0, 0.4)",
    );
    let dialog = el(
        &mut doc,
        overlay,
        "margin: 100px; width: 300px; height: 200px; overflow: auto; \
         background-color: rgb(255, 255, 255)",
    );
    el(
        &mut doc,
        dialog,
        "width: 280px; height: 300px; background-color: rgb(200, 0, 0)",
    );
    el(
        &mut doc,
        dialog,
        "position: fixed; left: 350px; top: 250px; width: 100px; height: 40px; \
         background-color: rgb(0, 200, 0)",
    );
    doc.resolve_layout(VW, VH);
    let p = paint(&mut doc, 1.0);
    for (at, chrome, what) in [
        ((370, 260), [128, 228, 128], "tooltip inside the dialog"),
        ((430, 280), [128, 228, 128], "tooltip past the dialog"),
        ((150, 150), [228, 128, 128], "dialog content"),
        ((50, 50), [204, 204, 204], "overlay"),
        ((390, 150), [255, 255, 255], "dialog background"),
    ] {
        assert_near(over_white(&p, at), chrome, what);
    }
}

/// With no clip to escape nothing is lifted: the fixed box is painted into its
/// context's group like any child. Chrome 155: `(102, 222, 102)` on white,
/// premultiplied `[0, 120, 0, 153]` on the transparent canvas. (#2057 review,
/// P6.)
///
/// Kills: a lift that pops from the floor up rather than from the lowest clip
/// above it, which pops the group layer with no clip above it and re-pushes it.
#[test]
fn an_opacity_owner_with_no_clip_to_escape_is_one_group() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let ctx = el(
        &mut doc,
        body,
        "opacity: 0.6; width: 300px; height: 300px; background-color: rgb(200, 0, 0)",
    );
    el(
        &mut doc,
        ctx,
        "position: fixed; left: 50px; top: 50px; width: 100px; height: 40px; \
         background-color: rgb(0, 200, 0)",
    );
    doc.resolve_layout(VW, VH);
    let p = paint(&mut doc, 1.0);
    assert_eq!(px(&p, (60, 60), 1.0), [0, 120, 0, 153]);
}

/// The fixed floor a transformed box sets is put back when paint leaves it: a
/// transformed box inside a clipper is painted first, then the issue's shape
/// elsewhere. (#2057 review, P7.)
///
/// Kills: a floor that is not restored (the later fixed box keeps the clipper,
/// `OUT` empty).
#[test]
fn the_fixed_floor_is_restored_after_a_transformed_box() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let a = el(
        &mut doc,
        body,
        "overflow: hidden; width: 20px; height: 20px; margin-top: 500px",
    );
    el(
        &mut doc,
        a,
        "transform: translate(0px, 0px); width: 10px; height: 10px",
    );
    let clipper = el(
        &mut doc,
        body,
        "overflow: hidden; width: 200px; height: 200px; position: absolute; left: 0; top: 0",
    );
    let owner = el(&mut doc, clipper, OWNER);
    el(&mut doc, owner, FIXED);
    doc.resolve_layout(VW, VH);
    let p = paint(&mut doc, 1.0);
    assert_eq!(px(&p, IN, 1.0), GREEN);
    assert_eq!(
        px(&p, OUT, 1.0),
        GREEN,
        "escapes the clipper after a transform elsewhere"
    );
}

/// A painter that draws nothing and counts the clips it is asked to push,
/// keeping every other operation at the trait's default: what a painter that
/// cannot reorder its own stack (Vello) pays, clip by clip.
#[derive(Default)]
struct Clips(u64);

impl rinch_dom::paint::painter::Painter for Clips {
    fn reset(&mut self) {
        self.0 = 0;
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
        self.0 += 1;
    }
    fn push_layer(
        &mut self,
        _: rinch_dom::paint::painter::BlendMode,
        _: f32,
        _: peniko::kurbo::Affine,
        _: &rinch_dom::paint::painter::PaintShape,
    ) {
    }
    fn pop_layer(&mut self) {}
}

/// The clips one paint pushes, at the painter interface.
fn clip_pushes(doc: &mut RinchDocument) -> u64 {
    let mut painter = Clips::default();
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter.0
}

/// The clip masks one software paint rasterises.
fn clip_masks(doc: &mut RinchDocument) -> u64 {
    paint(doc, 1.0).stats().clip_masks
}

/// **A fixed box that draws nothing costs nothing**: a closed popover
/// (`opacity: 0`) or a `visibility: hidden` leaf, inside an `opacity` context
/// under three rounded clippers. Neither the lift around the entry nor the
/// group layer's sinking pushes a clip for it, so a painter is asked for
/// exactly the clips of the same page without it.
///
/// One that draws is lifted out of the three (pushed again once, after it)
/// and has the group sunk under them (pushed again at the push and at the
/// pop): nine more pushes at a painter that takes the defaults. One off the
/// window is culled before the lift, but the sinking is decided from style
/// alone, before anything is placed: six. The software painter overrides all
/// three (a layer slid under its masks, masks set aside and put back) and
/// rasterises no mask again in any case.
///
/// Kills: lifting before the cull (the off-window box costs nine), sinking the
/// group for a box that draws nothing (six each), and the software painter
/// falling back to the defaults (masks rasterised again).
#[test]
fn a_fixed_box_that_draws_nothing_lifts_no_clip() {
    let build = |fixed: Option<&str>| {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        doc.set_attribute(body, "style", "margin: 0");
        let mut parent = body;
        for _ in 0..3 {
            parent = el(
                &mut doc,
                parent,
                "overflow: hidden; border-radius: 5px; padding: 2px; width: 300px",
            );
        }
        let ctx = el(
            &mut doc,
            parent,
            "opacity: 0.9; height: 20px; background-color: rgb(200, 0, 0)",
        );
        if let Some(style) = fixed {
            el(&mut doc, ctx, style);
        }
        doc.resolve_layout(VW, VH);
        doc
    };
    let none = clip_pushes(&mut build(None));
    let none_masks = clip_masks(&mut build(None));
    assert!(
        none >= 3 && none_masks >= 3,
        "positive control: the rounded clippers are pushed and are masks ({none}, {none_masks})"
    );
    let box_at = |extra: &str| {
        format!(
            "position: fixed; left: 500px; top: 300px; width: 40px; height: 40px; \
             background-color: rgb(0, 200, 0); {extra}"
        )
    };
    for (extra, more) in [
        ("opacity: 0", 0),
        ("visibility: hidden", 0),
        ("left: 2000px", 6),
        ("", 9),
    ] {
        let style = box_at(extra);
        assert_eq!(
            clip_pushes(&mut build(Some(&style))),
            none + more,
            "{extra:?}"
        );
        assert_eq!(
            clip_masks(&mut build(Some(&style))),
            none_masks,
            "software masks, {extra:?}"
        );
    }
}
