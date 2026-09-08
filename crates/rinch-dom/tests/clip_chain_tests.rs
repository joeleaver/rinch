//! A hoisted box is clipped by the ancestors it was hoisted past — #324 stage B.
//!
//! Stage B dropped `overflow` from `creates_stacking_context`, which is where it
//! never belonged: two `z-index` values in different stacking contexts were being
//! compared, and that produced the same user-visible bug twice (#317's dropdown
//! backdrops, #534's menu-bar overlay). What held the clip together before was an
//! accident of that deviation — the clip was a bracket around one stacking
//! context's sequence, so making every clipping box a stacking context made the
//! bracket enclose the right set of boxes.
//!
//! It is a **clip chain** now: every hoisted [`PaintEntry`] records the clipping
//! ancestors between it and the collecting root, and paint pushes them while hit
//! testing rejects a point outside them. These tests pin the chain, off every
//! fixed point that hides a mutant in it:
//!
//! * **Nothing is at scroll 0.** A clip taken at the scrolled origin instead of
//!   the box's own is identical there, which is #439's exact shape. Every fixture
//!   below carries a non-zero scroll offset, and every one has a probe on each
//!   side of the difference.
//! * **Nothing is entirely inside its clip.** Correct and broken agree for a box
//!   that never overhangs, so each hoisted box reaches well past the clip and the
//!   assertions are read in pairs: one pixel provably outside the clip and inside
//!   the box's unclipped extent, one inside both.
//! * **The truncation fixture has three levels.** `position: absolute` is clipped
//!   by its containing-block chain and not by an `overflow` box below it, and that
//!   rule only bites with **two** clips and a containing block between them.
//! * **One fixture has a radius and probes a corner**, where a rect clip and a
//!   rounded one part company and nowhere else.
//! * **One fixture paints at scale 2**, where a dropped `* scale` stops being the
//!   no-op it is at every other pixel fixture in this repo.
//! * **No fixture puts every entry at `z == 0`**, where the `(z_index, dom_order)`
//!   sort key decides nothing.
//!
//! The oracle is the painter's own zeroed pixmap: no fixture gives the document a
//! background, so "not painted here" is a provable `[0, 0, 0, 0]` rather than an
//! eyeballed absence. That is the local oracle a whole-screen comparison cannot
//! provide (`reference_visual_regression_gap`).

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::node::RawNodeId;
use rinch_dom::stacking::{PaintOrder, stacking_paint_order};

/// `DomDocument`'s handle, as the tree's own index.
fn raw(id: rinch_core::dom::NodeId) -> RawNodeId {
    id.0
}

fn body_order(doc: &RinchDocument) -> PaintOrder {
    stacking_paint_order(&doc.tree, doc.tree.body_id, 1.0, 0.0, 0.0)
}

/// The chain recorded for `node`, in the body's sequence.
fn chain_of(order: &PaintOrder, node: RawNodeId) -> Vec<(f64, f64, f64, f64)> {
    let entry = order
        .iter()
        .find(|e| e.node_id == node)
        .unwrap_or_else(|| panic!("node {node} is not an entry of this sequence: {order:?}"));
    order
        .clips_for(entry)
        .iter()
        .map(|c| (c.rect.x0, c.rect.y0, c.rect.x1, c.rect.y1))
        .collect()
}

fn div(doc: &mut RinchDocument, parent: rinch_core::dom::NodeId, style: &str) -> RawNodeId {
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", style);
    doc.append_child(parent, d);
    raw(d)
}

// ── The three-level fixture: what an absolute's containing block decides ─────

/// `outer` clips (and is `abs`'s containing-block ancestor), `mid` is the
/// containing block, `inner` clips but is **below** the containing block — so
/// `inner` does not clip `abs` and `outer` does.
///
/// This is real CSS, not a rinch nicety: `<div style="position:relative"><div
/// style="overflow:hidden"><div style="position:absolute">` is the idiom every
/// tooltip and popover in the library is built on, and the absolute escapes the
/// `overflow: hidden` in every browser.
///
/// `outer` is scrolled, so the fixture also pins that a chain rect is taken at
/// the clipping box's **own** origin rather than at the scrolled one its children
/// are laid out against.
fn three_levels() -> (RinchDocument, RawNodeId, RawNodeId, RawNodeId, RawNodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "position: relative");

    let outer = doc.create_element("div");
    doc.set_attribute(
        outer,
        "style",
        "position: relative; overflow: hidden; width: 200px; height: 200px",
    );
    doc.append_child(body, outer);

    let mid = doc.create_element("div");
    doc.set_attribute(
        mid,
        "style",
        "position: relative; width: 100px; height: 100px",
    );
    doc.append_child(outer, mid);

    let inner = doc.create_element("div");
    doc.set_attribute(
        inner,
        "style",
        "overflow: hidden; width: 50px; height: 50px",
    );
    doc.append_child(mid, inner);

    let abs = doc.create_element("div");
    doc.set_attribute(
        abs,
        "style",
        "position: absolute; left: 0; top: 0; width: 300px; height: 300px; \
         z-index: 5; background-color: rgb(255, 0, 0)",
    );
    doc.append_child(inner, abs);

    doc.resolve_layout(800.0, 600.0);
    // Scrolled, so a clip taken at the scrolled origin is a different rect.
    doc.tree.nodes[raw(outer)].scroll_offset = (0.0, 30.0);

    (doc, raw(outer), raw(mid), raw(inner), raw(abs))
}

#[test]
fn an_absolute_chain_stops_at_its_containing_block() {
    let (doc, outer, mid, inner, abs) = three_levels();

    // The premise: `inner` really does clip, and really is not a containing
    // block. Without both, the truncation rule is not being exercised at all.
    assert!(doc.tree.get(inner).unwrap().clips_overflow());
    assert!(
        !doc.tree
            .get(inner)
            .unwrap()
            .establishes_abs_containing_block()
    );
    assert!(
        doc.tree
            .get(mid)
            .unwrap()
            .establishes_abs_containing_block()
    );

    let order = body_order(&doc);
    assert_eq!(
        chain_of(&order, abs),
        vec![(0.0, 0.0, 200.0, 200.0)],
        "`outer` clips the absolute and `inner` does not: `inner` sits below the \
         containing block, so it is not in the absolute's containing-block chain"
    );
    assert_eq!(
        chain_of(&order, mid),
        vec![(0.0, 0.0, 200.0, 200.0)],
        "`mid` is not absolute, so it takes the whole chain — the same one link \
         here, which is why the absolute row above needs `inner` beside it"
    );
    assert_eq!(
        chain_of(&order, outer),
        Vec::<(f64, f64, f64, f64)>::new(),
        "and a box hoisted past nothing has no chain"
    );
}

#[cfg(feature = "software-renderer")]
mod painted {
    use super::*;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    /// Nothing was drawn here. `Pixmap::new` zeroes and no fixture gives the
    /// document a background, so an unpainted pixel is exactly this.
    const NOTHING: [u8; 4] = [0, 0, 0, 0];
    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    fn paint_at(doc: &mut RinchDocument, painter: &mut TinySkiaPainter, scale: f64) {
        let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            painter,
            scale,
            (800.0, 600.0),
            &mut doc.font_cx,
            &mut layout_cx,
        );
    }

    fn paint(doc: &mut RinchDocument, painter: &mut TinySkiaPainter) {
        paint_at(doc, painter, 1.0);
    }

    fn pixel_at(painter: &TinySkiaPainter, x: u32, y: u32) -> [u8; 4] {
        let idx = ((y * painter.width() + x) * 4) as usize;
        let d = painter.pixels();
        [d[idx], d[idx + 1], d[idx + 2], d[idx + 3]]
    }

    /// The same three-level fixture, in pixels — which is what a user sees and
    /// what the span assertions above cannot prove on their own (a chain that is
    /// recorded and never pushed reads identically there).
    ///
    /// `outer` is scrolled 30px, so `abs` paints from y = -30 to y = 270 and the
    /// clip stays at `outer`'s own box, y in [0, 200).
    #[test]
    fn an_absolute_paints_outside_a_clip_below_its_containing_block() {
        let (mut doc, ..) = three_levels();
        let mut painter = TinySkiaPainter::new(400, 400);
        paint(&mut doc, &mut painter);

        assert_eq!(
            pixel_at(&painter, 25, 25),
            RED,
            "inside every box: if this is not red the fixture never painted and \
             nothing below proves anything"
        );
        assert_eq!(
            pixel_at(&painter, 150, 150),
            RED,
            "outside `inner`'s 50x50 box and inside `outer` — `inner` is below the \
             containing block, so it does not clip the absolute. Put `inner` in \
             the chain and this goes red-to-nothing"
        );
        assert_eq!(
            pixel_at(&painter, 150, 180),
            RED,
            "…and still inside `outer`, whose clip is its own box and not the \
             30px-scrolled origin its children are laid out against"
        );
        assert_eq!(
            pixel_at(&painter, 150, 210),
            NOTHING,
            "past `outer`'s bottom edge at y = 200, though the absolute reaches \
             y = 270 unclipped"
        );
        assert_eq!(
            pixel_at(&painter, 250, 100),
            NOTHING,
            "and past its right edge — drop the chain entirely and both of these \
             go red"
        );
    }

    /// A scrolled container clips a box hoisted out of it **at its own border
    /// box**, which the container's scroll offset does not move.
    ///
    /// Every probe is on one side or the other of the 50px difference between the
    /// two candidate origins, because at scroll 0 they coincide (#439's shape) and
    /// a probe in the overlap discriminates nothing.
    fn scrolled_scroller() -> RinchDocument {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        doc.set_attribute(body, "style", "position: relative");

        // Not at the document origin either: a clip rect that forgets the
        // accumulated parent offset is a no-op against a container at y = 0.
        div(&mut doc, body, "width: 300px; height: 60px");

        let scroller = doc.create_element("div");
        doc.set_attribute(
            scroller,
            "style",
            "position: relative; overflow: auto; width: 200px; height: 100px",
        );
        doc.append_child(body, scroller);

        div(
            &mut doc,
            scroller,
            "width: 100px; height: 400px; background-color: rgb(0, 0, 255)",
        );

        let panel = doc.create_element("div");
        doc.set_attribute(
            panel,
            "style",
            "position: absolute; left: 0; top: 0; width: 60px; height: 400px; \
             z-index: 5; background-color: rgb(0, 255, 0)",
        );
        doc.append_child(scroller, panel);

        doc.resolve_layout(800.0, 600.0);
        doc.tree.nodes[raw(scroller)].scroll_offset = (0.0, 50.0);
        doc
    }

    #[test]
    fn a_scrolled_container_clips_a_hoisted_box_at_its_own_box() {
        let mut doc = scrolled_scroller();
        let mut painter = TinySkiaPainter::new(400, 400);
        paint(&mut doc, &mut painter);

        // The scroller's box is y in [60, 160). Scrolled 50, the panel paints
        // from y = 10 to y = 410, so it overhangs at both ends.
        assert_eq!(
            pixel_at(&painter, 30, 75),
            GREEN,
            "the panel paints inside the scroller, over the blue content"
        );
        assert_eq!(
            pixel_at(&painter, 80, 75),
            BLUE,
            "and only where it is: the panel is 60 wide, the content 100"
        );
        assert_eq!(
            pixel_at(&painter, 30, 30),
            NOTHING,
            "above the scroller's own top edge at y = 60. A clip taken at the \
             scrolled origin would start at y = 10 and paint here"
        );
        assert_eq!(
            pixel_at(&painter, 30, 155),
            GREEN,
            "…right down to its bottom edge at y = 160, which the same wrong clip \
             would have cut away at y = 110"
        );
        assert_eq!(
            pixel_at(&painter, 30, 170),
            NOTHING,
            "and no further, though the panel reaches y = 410 unclipped"
        );
    }

    /// The chain rect is in painter units, so it scales with the DPI scale.
    ///
    /// Every other clip-chain fixture here sits at scale 1, where dropping a
    /// `* scale` is a literal no-op. At scale 2 the scroller's box is y in
    /// [120, 320) physical and the panel reaches y = 820.
    #[test]
    fn the_chain_scales_with_the_dpi_scale() {
        let mut doc = scrolled_scroller();
        let mut painter = TinySkiaPainter::new(600, 600);
        paint_at(&mut doc, &mut painter, 2.0);

        assert_eq!(pixel_at(&painter, 60, 150), GREEN, "inside, doubled");
        assert_eq!(pixel_at(&painter, 160, 150), BLUE, "the content, doubled");
        assert_eq!(
            pixel_at(&painter, 60, 60),
            NOTHING,
            "an unscaled origin would put the clip's top at y = 60 and paint here"
        );
        assert_eq!(
            pixel_at(&painter, 60, 310),
            GREEN,
            "the box is 200 physical px tall, so the clip reaches y = 320"
        );
        assert_eq!(
            pixel_at(&painter, 60, 340),
            NOTHING,
            "and an unscaled height would have ended it at y = 220"
        );
    }

    /// A chain link is the clipping box's **rounded** border box. At radius 0 a
    /// rect clip and a rounded one agree everywhere, so the probe is a corner —
    /// and a second probe at the same x, halfway down, says the corner probe is
    /// not just "the left edge is cut".
    #[test]
    fn a_rounded_chain_link_cuts_its_corners() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        doc.set_attribute(body, "style", "position: relative");

        let round = doc.create_element("div");
        doc.set_attribute(
            round,
            "style",
            "position: relative; overflow: hidden; border-radius: 40px; \
             width: 100px; height: 100px",
        );
        doc.append_child(body, round);

        let panel = doc.create_element("div");
        doc.set_attribute(
            panel,
            "style",
            "position: absolute; left: 0; top: 0; width: 100px; height: 200px; \
             z-index: 5; background-color: rgb(255, 0, 0)",
        );
        doc.append_child(round, panel);

        doc.resolve_layout(800.0, 600.0);
        doc.tree.nodes[raw(round)].scroll_offset = (0.0, 20.0);

        let mut painter = TinySkiaPainter::new(300, 300);
        paint(&mut doc, &mut painter);

        assert_eq!(pixel_at(&painter, 50, 50), RED, "the middle is filled");
        assert_eq!(
            pixel_at(&painter, 2, 50),
            RED,
            "and so is the left edge halfway down, where the radius has curved \
             back — without this the corner probe below proves only that \
             something on the left is cut"
        );
        assert_eq!(
            pixel_at(&painter, 2, 2),
            NOTHING,
            "(2, 2) is 53.7px from the corner circle's centre at (40, 40), well \
             outside a 40px radius. A chain link that dropped its radii would \
             paint here"
        );
        assert_eq!(
            pixel_at(&painter, 50, 95),
            RED,
            "the clip is the box, scroll offset or not: the panel paints from \
             y = -20 to y = 180 and is cut at y = 100"
        );
        assert_eq!(pixel_at(&painter, 50, 105), NOTHING);
    }

    /// A `position: fixed` box takes an **empty** chain: it resolves against the
    /// viewport, so no `overflow` ancestor between it and the body is in its
    /// containing-block chain.
    #[test]
    fn a_fixed_box_takes_no_chain_at_all() {
        let mut doc = RinchDocument::new();
        let body = doc.body();

        let scroller = doc.create_element("div");
        doc.set_attribute(
            scroller,
            "style",
            "position: relative; overflow: auto; width: 100px; height: 100px",
        );
        doc.append_child(body, scroller);

        div(
            &mut doc,
            scroller,
            "width: 100px; height: 400px; background-color: rgb(0, 0, 255)",
        );

        let floating = doc.create_element("div");
        doc.set_attribute(
            floating,
            "style",
            "position: fixed; left: 150px; top: 150px; width: 50px; height: 50px; \
             z-index: 5; background-color: rgb(0, 255, 0)",
        );
        doc.append_child(scroller, floating);

        doc.resolve_layout(800.0, 600.0);
        doc.tree.nodes[raw(scroller)].scroll_offset = (0.0, 50.0);

        assert_eq!(
            chain_of(&body_order(&doc), raw(floating)),
            Vec::<(f64, f64, f64, f64)>::new()
        );

        let mut painter = TinySkiaPainter::new(300, 300);
        paint(&mut doc, &mut painter);
        assert_eq!(
            pixel_at(&painter, 170, 170),
            GREEN,
            "the fixed box paints at its viewport position, well outside the \
             100x100 scroller it is written inside — give it the chain and it \
             disappears entirely"
        );
    }

    /// The payoff, in the shape #324 was filed for: a `z-index` inside a scroller
    /// and a `z-index` outside it are compared **in one sequence**, and the
    /// higher one wins.
    ///
    /// The scroller used to be a stacking context, so its badge was never
    /// compared with anything outside it — the badge's `2` lost to the box's `1`
    /// because what was really being sorted was the *scroller's* implicit 0.
    #[test]
    fn a_z_index_inside_a_scroller_is_compared_with_one_outside_it() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        doc.set_attribute(body, "style", "position: relative");

        let scroller = doc.create_element("div");
        doc.set_attribute(
            scroller,
            "style",
            "position: relative; overflow: auto; width: 200px; height: 200px",
        );
        doc.append_child(body, scroller);
        div(
            &mut doc,
            scroller,
            "width: 100px; height: 600px; background-color: rgb(0, 0, 255)",
        );

        let badge = doc.create_element("div");
        doc.set_attribute(
            badge,
            "style",
            "position: absolute; left: 0; top: 60px; width: 100px; height: 100px; \
             z-index: 2; background-color: rgb(0, 255, 0)",
        );
        doc.append_child(scroller, badge);

        // Written *after* the scroller and at a lower z: tree order would put it
        // on top, z-index must not.
        let over = doc.create_element("div");
        doc.set_attribute(
            over,
            "style",
            "position: absolute; left: 0; top: 0; width: 200px; height: 200px; \
             z-index: 1; background-color: rgb(255, 0, 0)",
        );
        doc.append_child(body, over);

        doc.resolve_layout(800.0, 600.0);
        doc.tree.nodes[raw(scroller)].scroll_offset = (0.0, 20.0);

        let order = body_order(&doc);
        let pos = |id| order.iter().position(|e| e.node_id == id).unwrap();
        assert!(
            pos(raw(badge)) > pos(raw(over)),
            "z 2 beats z 1 across the scroller's boundary: {:?}",
            order
                .iter()
                .map(|e| (e.node_id, e.z_index))
                .collect::<Vec<_>>()
        );

        let mut painter = TinySkiaPainter::new(400, 400);
        paint(&mut doc, &mut painter);
        assert_eq!(
            pixel_at(&painter, 50, 60),
            GREEN,
            "the badge paints over the box outside the scroller — scrolled 20, it \
             occupies y in [40, 140)"
        );
        assert_eq!(
            pixel_at(&painter, 50, 190),
            RED,
            "and only where it is: below it, the z 1 box is on top of the scroller"
        );
        assert_eq!(
            pixel_at(&painter, 50, 210),
            NOTHING,
            "…and the badge is still clipped to the scroller it came out of, \
             which reaches y = 200"
        );
    }

    /// Two scrollers side by side: each hoisted box carries **its own**
    /// container's clip.
    ///
    /// The collector reuses the tail of the clip table when it already holds the
    /// chain being asked for — fifty rows under one scroller would otherwise copy
    /// the same rect fifty times. This is the fixture where a reuse test that
    /// compared lengths instead of contents would hand the second panel the first
    /// scroller's clip: the two containers are deliberately different sizes, and
    /// each panel is probed *inside its own* container and *inside its sibling's*.
    #[test]
    fn sibling_scrollers_do_not_share_a_chain() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        doc.set_attribute(body, "style", "position: relative");

        let make = |doc: &mut RinchDocument, style: &str, colour: &str| {
            let s = doc.create_element("div");
            doc.set_attribute(s, "style", style);
            doc.append_child(body, s);
            let p = doc.create_element("div");
            doc.set_attribute(
                p,
                "style",
                &format!(
                    "position: absolute; left: 0; top: 0; width: 300px; \
                     height: 300px; z-index: 5; background-color: {colour}"
                ),
            );
            doc.append_child(s, p);
            (raw(s), raw(p))
        };

        let (a, pa) = make(
            &mut doc,
            "position: relative; overflow: hidden; width: 60px; height: 60px",
            "rgb(255, 0, 0)",
        );
        let (b, pb) = make(
            &mut doc,
            "position: relative; overflow: hidden; width: 160px; height: 160px",
            "rgb(0, 255, 0)",
        );

        doc.resolve_layout(800.0, 600.0);
        doc.tree.nodes[a].scroll_offset = (0.0, 10.0);
        doc.tree.nodes[b].scroll_offset = (0.0, 10.0);

        let order = body_order(&doc);
        assert_eq!(chain_of(&order, pa), vec![(0.0, 0.0, 60.0, 60.0)]);
        assert_eq!(chain_of(&order, pb), vec![(0.0, 60.0, 160.0, 220.0)]);

        let mut painter = TinySkiaPainter::new(400, 400);
        paint(&mut doc, &mut painter);

        assert_eq!(pixel_at(&painter, 30, 30), RED, "inside the small one");
        assert_eq!(
            pixel_at(&painter, 100, 30),
            NOTHING,
            "and not past it — a chain that reused the *larger* sibling's rect \
             would paint here"
        );
        assert_eq!(pixel_at(&painter, 100, 150), GREEN, "inside the large one");
        assert_eq!(
            pixel_at(&painter, 100, 230),
            NOTHING,
            "and not past it either — reusing the *smaller* sibling's rect would \
             have cut this one short instead"
        );
    }

    /// The other side of the truncation rule, and the one a fixture full of
    /// absolutes cannot see: a **`position: relative`** box is clipped by every
    /// clipping ancestor, containing block or not.
    ///
    /// The clip here sits *below* the nearest containing-block establisher, which
    /// is exactly where truncating would drop it. Applying the absolute rule to
    /// everything survived the rest of this file — every other fixture happens to
    /// put its clips at or above the containing block, which is the arrangement
    /// where the two answers agree.
    #[test]
    fn a_relative_box_is_clipped_by_an_ancestor_below_its_containing_block() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        doc.set_attribute(body, "style", "position: relative");

        // Establishes a containing block and clips nothing, so the clip below it
        // is the whole of the chain.
        let cb = doc.create_element("div");
        doc.set_attribute(
            cb,
            "style",
            "position: relative; width: 200px; height: 200px",
        );
        doc.append_child(body, cb);

        let clipbox = doc.create_element("div");
        doc.set_attribute(
            clipbox,
            "style",
            "overflow: hidden; width: 100px; height: 100px",
        );
        doc.append_child(cb, clipbox);

        let rel = doc.create_element("div");
        doc.set_attribute(
            rel,
            "style",
            "position: relative; z-index: 5; width: 300px; height: 300px; \
             background-color: rgb(255, 0, 0)",
        );
        doc.append_child(clipbox, rel);

        doc.resolve_layout(800.0, 600.0);
        doc.tree.nodes[raw(clipbox)].scroll_offset = (0.0, 20.0);

        assert_eq!(
            chain_of(&body_order(&doc), raw(rel)),
            vec![(0.0, 0.0, 100.0, 100.0)],
            "the clipping box is below the containing block, and a relative box \
             is clipped by it all the same"
        );

        let mut painter = TinySkiaPainter::new(400, 400);
        paint(&mut doc, &mut painter);

        assert_eq!(pixel_at(&painter, 50, 50), RED, "inside the clipping box");
        assert_eq!(
            pixel_at(&painter, 50, 95),
            RED,
            "right down to its bottom edge — the box does not move with its own \
             20px scroll offset"
        );
        assert_eq!(
            pixel_at(&painter, 50, 110),
            NOTHING,
            "and stops there, though the relative box reaches y = 280"
        );
        assert_eq!(
            pixel_at(&painter, 150, 50),
            NOTHING,
            "and at its right edge. Truncate a relative box's chain the way an \
             absolute's is truncated and both of these paint red"
        );
    }

    /// **Every** link of the chain is pushed, not just the outermost.
    ///
    /// Two nested clipping boxes, and the probes that matter are between them:
    /// inside the outer, outside the inner. Every other fixture in this file has
    /// a single *effective* link — the three-level one records two but the
    /// absolute only takes one — which is the fixed point where "push the first
    /// and stop" is indistinguishable from the truth.
    #[test]
    fn every_link_of_a_two_clip_chain_is_pushed() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        doc.set_attribute(body, "style", "position: relative");

        let outer = doc.create_element("div");
        doc.set_attribute(
            outer,
            "style",
            "overflow: hidden; width: 300px; height: 300px",
        );
        doc.append_child(body, outer);
        let inner = doc.create_element("div");
        doc.set_attribute(
            inner,
            "style",
            "overflow: hidden; width: 100px; height: 100px",
        );
        doc.append_child(outer, inner);

        let panel = doc.create_element("div");
        doc.set_attribute(
            panel,
            "style",
            "position: relative; z-index: 5; width: 400px; height: 400px; \
             background-color: rgb(255, 0, 0)",
        );
        doc.append_child(inner, panel);

        doc.resolve_layout(800.0, 600.0);
        doc.tree.nodes[raw(inner)].scroll_offset = (0.0, 20.0);

        assert_eq!(
            chain_of(&body_order(&doc), raw(panel)),
            vec![(0.0, 0.0, 300.0, 300.0), (0.0, 0.0, 100.0, 100.0)],
            "outermost first"
        );

        let mut painter = TinySkiaPainter::new(400, 400);
        paint(&mut doc, &mut painter);

        assert_eq!(pixel_at(&painter, 50, 50), RED, "inside both clips");
        assert_eq!(
            pixel_at(&painter, 50, 95),
            RED,
            "right down to the inner box's own bottom edge, which its 20px \
             scroll offset does not move"
        );
        assert_eq!(
            pixel_at(&painter, 150, 150),
            NOTHING,
            "outside the inner clip and inside the outer one. Push only the \
             first link and the panel paints here"
        );
        assert_eq!(pixel_at(&painter, 350, 150), NOTHING, "and outside both");
    }

    /// A run of chain clips is popped when the sequence ends, so the **next**
    /// thing the painter draws is not still inside it.
    ///
    /// Every other fixture here has its chained entry last in the last sequence
    /// painted, where a leaked clip has nothing left to spoil — which is exactly
    /// the fixed point that let "never pop at the end of the loop" survive a green
    /// run of this file. The chained entry is therefore inside a **nested**
    /// stacking context, and a box at a higher `z-index` in the *outer* sequence
    /// is painted after it, outside the scroller's box.
    #[test]
    fn a_clip_run_is_popped_before_the_next_sequence_entry() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        doc.set_attribute(body, "style", "position: relative");

        let sc = doc.create_element("div");
        doc.set_attribute(
            sc,
            "style",
            "position: relative; z-index: 1; width: 200px; height: 200px",
        );
        doc.append_child(body, sc);

        let scroller = doc.create_element("div");
        doc.set_attribute(
            scroller,
            "style",
            "position: relative; overflow: hidden; width: 100px; height: 100px",
        );
        doc.append_child(sc, scroller);

        let panel = doc.create_element("div");
        doc.set_attribute(
            panel,
            "style",
            "position: relative; z-index: 5; width: 300px; height: 300px; \
             background-color: rgb(255, 0, 0)",
        );
        doc.append_child(scroller, panel);

        // Painted after `sc` in the body's sequence, and clear of the scroller.
        let later = doc.create_element("div");
        doc.set_attribute(
            later,
            "style",
            "position: absolute; left: 150px; top: 150px; width: 150px; \
             height: 150px; z-index: 2; background-color: rgb(0, 255, 0)",
        );
        doc.append_child(body, later);

        doc.resolve_layout(800.0, 600.0);
        doc.tree.nodes[raw(scroller)].scroll_offset = (0.0, 20.0);

        let order = body_order(&doc);
        let pos = |id| order.iter().position(|e| e.node_id == id).unwrap();
        assert!(
            pos(raw(later)) > pos(raw(sc)),
            "the fixture only bites while `later` is painted after the sequence \
             that opened the clips"
        );
        assert_eq!(
            chain_of(&order, raw(later)),
            Vec::<(f64, f64, f64, f64)>::new(),
            "`later` has no chain of its own, so anything clipping it is a leak"
        );

        let mut painter = TinySkiaPainter::new(400, 400);
        paint(&mut doc, &mut painter);

        assert_eq!(
            pixel_at(&painter, 50, 50),
            RED,
            "the panel, inside its clip"
        );
        assert_eq!(
            pixel_at(&painter, 150, 50),
            NOTHING,
            "and clipped by the scroller it was hoisted past"
        );
        assert_eq!(
            pixel_at(&painter, 200, 200),
            GREEN,
            "…and the next entry in the outer sequence is painted with that clip \
             gone. Leave the run pushed and this is nothing at all"
        );
    }

    /// The collecting root's own clip is applied by paint's bracket, not by the
    /// chain, so it must not be in one — or it is pushed twice.
    ///
    /// Pushing a clip twice is idempotent, so no pixel can see this; the assertion
    /// is on the spans. It is worth its own test because "why is nothing in this
    /// chain" is exactly the question the next person will ask of
    /// `stacking_paint_order`'s first line.
    #[test]
    fn a_stacking_context_root_keeps_its_own_clip_out_of_its_childrens_chains() {
        let mut doc = RinchDocument::new();
        let body = doc.body();

        let root = doc.create_element("div");
        doc.set_attribute(
            root,
            "style",
            "position: relative; z-index: 1; overflow: hidden; \
             width: 100px; height: 100px",
        );
        doc.append_child(body, root);

        let panel = doc.create_element("div");
        doc.set_attribute(
            panel,
            "style",
            "position: absolute; left: 0; top: 0; width: 300px; height: 300px; \
             z-index: 5; background-color: rgb(255, 0, 0)",
        );
        doc.append_child(root, panel);

        doc.resolve_layout(800.0, 600.0);
        doc.tree.nodes[raw(root)].scroll_offset = (0.0, 10.0);

        assert!(
            doc.tree.get(raw(root)).unwrap().creates_stacking_context(),
            "the fixture needs a real stacking-context root — z-index does it, \
             `overflow` deliberately no longer does"
        );
        let inner = stacking_paint_order(&doc.tree, raw(root), 1.0, 0.0, 0.0);
        assert_eq!(
            chain_of(&inner, raw(panel)),
            Vec::<(f64, f64, f64, f64)>::new(),
            "the root's clip is the bracket paint already opened around this \
             whole sequence"
        );

        // …and it is still applied, by that bracket.
        let mut painter = TinySkiaPainter::new(300, 300);
        paint(&mut doc, &mut painter);
        assert_eq!(pixel_at(&painter, 50, 50), RED);
        assert_eq!(pixel_at(&painter, 150, 50), NOTHING);
    }

    /// A chain link under a **transform**, which nothing else here covers.
    ///
    /// The collecting root is transformed, so every link is recorded in that
    /// root's own untransformed space and paint pushes it under
    /// `node_transform` with no per-clip composition. The module docs assert
    /// that; this measures it. Its twin in
    /// `rinch/src/app/hit_testing.rs`
    /// (`a_tap_on_a_chained_box_under_a_transform_lands_where_it_paints`) reads
    /// the same fixture from the input side, and the two are the only thing
    /// standing between the chain and a probe point taken in the wrong space.
    ///
    /// The transform is a 100px translate, so a link left in screen space —
    /// or a probe point taken in viewport space — is off by exactly that.
    #[test]
    fn a_chain_link_under_a_transform_clips_where_the_transform_puts_it() {
        let mut doc = RinchDocument::new();
        let body = doc.body();

        let tx = doc.create_element("div");
        doc.set_attribute(
            tx,
            "style",
            "position: relative; z-index: 1; transform: translate(100px, 100px); \
             width: 400px; height: 400px",
        );
        doc.append_child(body, tx);

        let clipbox = doc.create_element("div");
        doc.set_attribute(
            clipbox,
            "style",
            "overflow: hidden; width: 100px; height: 100px",
        );
        doc.append_child(tx, clipbox);

        let panel = doc.create_element("div");
        doc.set_attribute(
            panel,
            "style",
            "position: relative; z-index: 5; width: 300px; height: 300px; \
             background-color: rgb(255, 0, 0)",
        );
        doc.append_child(clipbox, panel);

        doc.resolve_layout(800.0, 600.0);
        doc.tree.nodes[raw(clipbox)].scroll_offset = (0.0, 20.0);

        let inner = stacking_paint_order(&doc.tree, raw(tx), 1.0, 0.0, 0.0);
        assert_eq!(
            chain_of(&inner, raw(panel)),
            vec![(0.0, 0.0, 100.0, 100.0)],
            "the link is in the transformed root's own space, not on screen"
        );

        let mut painter = TinySkiaPainter::new(500, 500);
        paint(&mut doc, &mut painter);

        assert_eq!(
            pixel_at(&painter, 150, 150),
            RED,
            "inside the clip, which the transform has moved to (100,100)-(200,200)"
        );
        assert_eq!(
            pixel_at(&painter, 120, 120),
            RED,
            "…and near its top-left corner"
        );
        assert_eq!(
            pixel_at(&painter, 250, 250),
            NOTHING,
            "outside the clip and inside the transformed root, where the panel \
             reaches unclipped: a link pushed without the transform paints here"
        );
        assert_eq!(
            pixel_at(&painter, 50, 50),
            NOTHING,
            "and above the transform entirely, where a link left in screen space \
             would have put the clip"
        );
    }
}

/// The run-length reuse is not decoration: it is what keeps the full-surface
/// `Mask` that `TinySkiaPainter::push_clip` allocates from being paid once per
/// hoisted box. `sibling_scrollers_do_not_share_a_chain` pins that the reuse is
/// never *wrong*; nothing pinned that it happens at all, so a refactor could
/// quietly turn it off and leave every pixel identical.
///
/// Counted the way `paint_children_with_stacking` counts: a run starts wherever
/// an entry's `ClipSpan` differs from the one currently open.
///
/// Note the 200-row row is a **fixed point**: one scroller means the table only
/// ever holds one chain, so a collector that reused far less would still answer
/// 1 there. The 50-scroller row is the one that discriminates — reusing only
/// when the whole table matches takes it to 197.
#[test]
fn consecutive_entries_that_share_a_chain_share_one_clip_push() {
    fn pushes(order: &PaintOrder) -> (usize, usize) {
        let mut open = rinch_dom::stacking::ClipSpan::EMPTY;
        let mut n = 0usize;
        for e in order.iter() {
            if e.clips != open {
                open = e.clips;
                n += open.len();
            }
        }
        (n, order.clips.len())
    }

    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "position: relative");
    let s = doc.create_element("div");
    doc.set_attribute(
        s,
        "style",
        "position: relative; overflow: auto; width: 300px; height: 400px",
    );
    doc.append_child(body, s);
    for _ in 0..200 {
        div(
            &mut doc,
            s,
            "position: relative; width: 300px; height: 20px",
        );
    }
    doc.resolve_layout(800.0, 600.0);
    doc.tree.nodes[raw(s)].scroll_offset = (0.0, 100.0);
    assert_eq!(
        pushes(&body_order(&doc)),
        (1, 1),
        "200 positioned rows under one scroller are one push and one table entry"
    );

    let mut doc2 = RinchDocument::new();
    let body2 = doc2.body();
    doc2.set_attribute(body2, "style", "position: relative");
    for _ in 0..50 {
        let s = doc2.create_element("div");
        doc2.set_attribute(
            s,
            "style",
            "position: relative; overflow: auto; width: 300px; height: 40px",
        );
        doc2.append_child(body2, s);
        for _ in 0..4 {
            div(
                &mut doc2,
                s,
                "position: relative; width: 300px; height: 20px",
            );
        }
    }
    doc2.resolve_layout(800.0, 600.0);
    assert_eq!(
        pushes(&body_order(&doc2)),
        (50, 50),
        "50 scrollers of 4 rows are one push each, not one per row — this is the \
         row a length-only or whole-table reuse takes to 197"
    );
}
