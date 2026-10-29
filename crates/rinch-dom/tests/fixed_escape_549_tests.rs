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
//! identity `translate(0, 0)`, where the two placements coincide.
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
    let mut painter =
        TinySkiaPainter::new((VW as f64 * scale) as u32, (VH as f64 * scale) as u32);
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
        assert_eq!(px(&p, IN, scale), GREEN, "inside the clipper, scale {scale}");
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
    assert_eq!(px(&p, (100, 30), 1.0), BLUE, "positive control, owner's child");
    assert_eq!(px(&p, (600, 30), 1.0), NOTHING, "the owner's child past the clip");
    assert_eq!(px(&p, (100, 160), 1.0), BLUE, "positive control, later sibling");
    assert_eq!(px(&p, (600, 160), 1.0), NOTHING, "the later sibling past the clip");
    assert_eq!(px(&p, OUT, 1.0), GREEN, "the fixed box between them escaped");
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
