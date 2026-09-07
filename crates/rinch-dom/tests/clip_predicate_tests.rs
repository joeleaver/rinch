//! One answer to "does this box clip, and to what shape" — #324 stage A.
//!
//! Seven sites used to hold four different clip predicates: paint,
//! `layer_bounds` and `creates_stacking_context` matched `overflow_y` against
//! `Hidden | Scroll | Auto`; the dirty-region prune matched the same three on
//! either axis; hit testing and `viewport_clip_rect` asked `!= Visible` on
//! either axis; `viewport_rect_with_radius` asked it on `overflow_y` alone.
//! The tests below are in three groups:
//!
//! * **Which spelling is reachable** — `overflow: clip`, and only
//!   `overflow: clip`, can put a clipping value on one axis and `visible` on
//!   the other, so `clip` is where the two spellings actually disagreed and
//!   `overflow-x: hidden` is not. That is measured against Stylo here rather
//!   than argued from the spec.
//! * **What the disagreement cost** — an `overflow: clip` box clipped clicks
//!   and painted unclipped. The pixel oracles below are read against the
//!   painter's zeroed pixmap, so "not painted" is a provable `[0, 0, 0, 0]`
//!   rather than an eyeballed absence.
//! * **That the shape stays tied to the box** — `clip_shape` re-derives its
//!   rect instead of receiving the one its caller holds, which is a degree of
//!   freedom the extraction created and stage B will build a second caller on
//!   top of. `the_clip_rect_is_the_box_the_background_paints` and
//!   `the_clip_rect_scales_with_the_dpi_scale` pin it.

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::computed_style::OverflowValue;
use rinch_dom::node::{NodeTree, RawNodeId};
use rinch_dom::stacking::{paints_at_stacking_root, stacking_paint_order};
use rinch_dom::{RinchDocument, node::LayoutResult};

/// The computed overflow pair Stylo hands back for `style` on a plain div.
fn computed_overflow(style: &str) -> (OverflowValue, OverflowValue) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", &format!("width: 100px; height: 100px; {style}"));
    doc.append_child(body, d);
    doc.resolve_layout(800.0, 600.0);
    let n = doc.tree.get(d.0).unwrap();
    (n.computed_style.overflow_x, n.computed_style.overflow_y)
}

fn clips(style: &str) -> bool {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", &format!("width: 100px; height: 100px; {style}"));
    doc.append_child(body, d);
    doc.resolve_layout(800.0, 600.0);
    doc.tree.get(d.0).unwrap().clips_overflow()
}

/// Whether a plain div carrying `style` creates a stacking context.
fn sc(style: &str) -> bool {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", &format!("width: 100px; height: 100px; {style}"));
    doc.append_child(body, d);
    doc.resolve_layout(800.0, 600.0);
    doc.tree.get(d.0).unwrap().creates_stacking_context()
}

// ── Which spelling is reachable ─────────────────────────────────────────────

/// The reason the old `overflow_y`-only spelling never actually misfired for
/// `hidden`/`scroll`/`auto`: css-overflow-3 §3 makes a `visible` compute to
/// `auto` when the other axis is neither `visible` nor `clip`, and **Stylo's
/// style adjuster implements it**. Measured, because the scoping for #324 could
/// only infer it — nothing in `from_stylo` enforces it and nothing asserted it.
///
/// So the asymmetry the issue names (`overflow-x: hidden; overflow-y: visible`)
/// does not exist by the time a `ComputedStyle` is built, in either direction,
/// and unifying those three values across both axes is a pure refactor. If a
/// Stylo bump ever stops doing this, that becomes a live rendering bug and this
/// test is what says so.
#[test]
fn stylo_pairs_a_non_visible_axis_with_auto() {
    use OverflowValue::{Auto, Hidden, Scroll};

    assert_eq!(computed_overflow("overflow-x: hidden"), (Hidden, Auto));
    assert_eq!(computed_overflow("overflow-y: hidden"), (Auto, Hidden));
    assert_eq!(
        computed_overflow("overflow-x: hidden; overflow-y: visible"),
        (Hidden, Auto),
        "an author's explicit `visible` is overridden too — this is a computed \
         value rule, not a defaulting one"
    );
    assert_eq!(
        computed_overflow("overflow-y: visible; overflow-x: scroll"),
        (Scroll, Auto),
        "and it holds for `scroll` as well as `hidden`, in either source order"
    );
}

/// `clip` is the exception the same rule carves out, and therefore the one
/// value that can leave the two axes genuinely asymmetric. This is what makes
/// the shared predicate read both axes rather than mirroring paint's old
/// `overflow_y`-only spelling: under that one, `overflow-x: clip` clipped
/// nothing at all while hit testing clipped both axes.
#[test]
fn stylo_leaves_clip_beside_visible() {
    use OverflowValue::{Clip, Visible};

    assert_eq!(computed_overflow("overflow-x: clip"), (Clip, Visible));
    assert_eq!(computed_overflow("overflow-y: clip"), (Visible, Clip));
    assert_eq!(computed_overflow("overflow: clip"), (Clip, Clip));
}

/// The predicate itself, sampled off every fixed point that hides a mutant:
/// both axes separately (an `overflow_y`-only reading fails the x rows), and
/// `clip` separately from `hidden`/`scroll`/`auto` (a `Hidden | Scroll | Auto`
/// reading fails the `clip` rows).
#[test]
fn a_box_clips_when_either_axis_is_not_visible() {
    assert!(!clips(""), "the initial value clips nothing");
    assert!(!clips("overflow: visible"));

    assert!(clips("overflow: hidden"));
    assert!(clips("overflow: scroll"));
    assert!(clips("overflow: auto"));
    assert!(clips("overflow-x: hidden"));
    assert!(clips("overflow-y: hidden"));

    assert!(
        clips("overflow: clip"),
        "`clip` is a clipping value — paint used to disagree with hit testing \
         about this, and paint was wrong"
    );
    assert!(
        clips("overflow-x: clip"),
        "and it reaches the predicate through the x axis alone, which is the \
         one arrangement `overflow_y`-only could never see"
    );
    assert!(clips("overflow-y: clip"));
}

/// Clipping is **not** stacking, since #324 stage B — the whole point of the
/// clip chain. Stage A had to make every clipping box a stacking context so the
/// bracket it opened enclosed the boxes it was entitled to clip; stage B gives
/// each hoisted entry the chain instead, and this is the pin that the arm is
/// gone rather than quietly re-added the next time something escapes a clip.
///
/// Sampled across the whole clip predicate — `hidden`, `scroll`, `auto`, `clip`
/// and single-axis `clip` — because a partial re-add is the plausible
/// regression, not a wholesale one.
#[test]
fn a_clipping_box_is_not_a_stacking_context() {
    assert!(!sc(""), "the initial value forms nothing either");
    assert!(!sc("overflow: hidden"));
    assert!(!sc("overflow: scroll"));
    assert!(!sc("overflow: auto"));
    assert!(!sc("overflow: clip"));
    assert!(!sc("overflow-x: clip"));

    // The clip is still there — this is a change to stacking alone.
    assert!(clips("overflow: hidden"));
    assert!(clips("overflow: clip"));
}

/// The CSS list, as far as `ComputedStyle` can express it: a positioned box with
/// an explicit `z-index`, `opacity < 1`, a transform — and `position: fixed` or
/// `sticky` whatever the `z-index`, which #324 stage B added alongside dropping
/// the `overflow` arm because both are the same three lines.
///
/// `position: relative` with no `z-index` is the row that matters most: it forms
/// no stacking context, but it *is* hoisted (`paints_at_stacking_root`), which
/// is why dropping the `overflow` arm moved every one of them out to the body.
#[test]
fn the_stacking_context_creators_are_the_css_ones() {
    assert!(!sc("position: relative"), "no z-index, no context");
    assert!(!sc("position: absolute"));
    assert!(!sc("z-index: 5"), "a static box ignores z-index");

    assert!(sc("position: relative; z-index: 5"));
    assert!(sc("position: absolute; z-index: 0"), "an explicit 0 counts");
    assert!(sc("opacity: 0.5"));
    assert!(sc("transform: translateX(10px)"));

    assert!(
        sc("position: fixed"),
        "a fixed box is viewport-level content: its descendants travel with it"
    );
    assert!(sc("position: sticky"));
}

/// The trap dropping the `overflow` arm springs, stated as a test rather than a
/// paragraph: a `position: relative` box with no `z-index` forms no stacking
/// context and is hoisted anyway. Every one of those in the component library —
/// `.rinch-slider`, the ColorPicker panels, `.rinch-navlink`, any `Paper` with a
/// badge — moved out to the body when the arm went, which is why the clip chain
/// had to arrive in the same change.
#[test]
fn a_relative_box_with_no_z_index_hoists_without_forming_a_context() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", "position: relative; width: 10px; height: 10px");
    doc.append_child(body, d);
    doc.resolve_layout(800.0, 600.0);
    let node = doc.tree.get(d.0).unwrap();

    assert!(!node.creates_stacking_context());
    assert!(
        paints_at_stacking_root(node),
        "hoisted regardless — Appendix E step 8"
    );
}

// ── What the disagreement cost ──────────────────────────────────────────────

/// Hit testing's `check_children` gate, reduced to the part these tests are
/// about. This is `rinch`'s `hit_test_node` with transforms, `position: fixed`,
/// visibility and `pointer-events` taken out — the same reduction
/// `stacking_tests::resolve` makes, kept here because what this file is pinning
/// is that the gate and the painter read one predicate.
///
/// # What this cannot catch
///
/// **It is a *model* of hit testing, not hit testing.** It lives in
/// `rinch-dom`, which has no hit testing in it at all; the real walk is
/// `rinch`'s `hit_test_node`, and the two are held together only by both
/// calling [`rinch_dom::node::Node::clips_overflow`]. So
/// `a_clipped_pixel_and_a_clipped_tap_are_the_same_set` pins "paint and a
/// faithful model of hit testing agree" — it would **not** go red if
/// `hit_test_node` itself regressed, only if the shared predicate did.
/// Anything pinning the real walk has to live in the `rinch` crate.
fn resolve(tree: &NodeTree, id: RawNodeId, ox: f32, oy: f32, x: f32, y: f32) -> Option<RawNodeId> {
    let node = tree.get(id)?;
    let LayoutResult {
        x: lx,
        y: ly,
        width,
        height,
    } = node.layout;
    let (nx, ny) = (ox + lx, oy + ly);
    let inside = x >= nx && x <= nx + width && y >= ny && y <= ny + height;

    if !node.clips_overflow() || inside {
        let (sx, sy) = (node.scroll_offset.0 as f32, node.scroll_offset.1 as f32);
        let is_body = id == tree.body_id;
        if is_body || node.creates_stacking_context() {
            let order =
                stacking_paint_order(tree, id, is_body, 1.0, (nx - sx) as f64, (ny - sy) as f64);
            for entry in order.iter().rev() {
                // The entry's clip chain: the clipping ancestors it was hoisted
                // past, which `check_children` above never sees because
                // `overflow` is not a stacking context (#324 stage B). Rect-only,
                // like that gate.
                if !order
                    .clips_for(entry)
                    .iter()
                    .all(|c| c.contains(x as f64, y as f64))
                {
                    continue;
                }
                if let Some(hit) = resolve(
                    tree,
                    entry.node_id,
                    entry.offset_x as f32,
                    entry.offset_y as f32,
                    x,
                    y,
                ) {
                    return Some(hit);
                }
            }
        } else {
            for &child_id in node.children.iter().rev() {
                let Some(child) = tree.get(child_id) else {
                    continue;
                };
                if paints_at_stacking_root(child) {
                    continue;
                }
                if let Some(hit) = resolve(tree, child_id, nx - sx, ny - sy, x, y) {
                    return Some(hit);
                }
            }
        }
    }

    inside.then_some(id)
}

#[cfg(feature = "software-renderer")]
mod painted {
    use super::*;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    /// Nothing was drawn here. `Pixmap::new` zeroes, and no fixture below gives
    /// the document a background, so an unpainted pixel is exactly this — the
    /// local oracle a whole-screen comparison cannot provide.
    const NOTHING: [u8; 4] = [0, 0, 0, 0];
    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    fn paint(doc: &mut RinchDocument, painter: &mut TinySkiaPainter) {
        paint_at(doc, painter, 1.0);
    }

    /// Rasterise at an explicit DPI scale. Layout is in CSS px either way; only
    /// the painter's units change, which is the whole point of the scale-2
    /// fixture below.
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

    fn pixel_at(painter: &TinySkiaPainter, x: u32, y: u32) -> [u8; 4] {
        let idx = ((y * painter.width() + x) * 4) as usize;
        let d = painter.pixels();
        [d[idx], d[idx + 1], d[idx + 2], d[idx + 3]]
    }

    /// A 100x100 container at the document origin holding one 200x200 red
    /// child, so the child overhangs on **both** axes and by more than it fits.
    /// Overhanging is the whole point: a child entirely inside its container is
    /// the fixed point where a clip and no clip paint the same picture.
    fn overhang(container_style: &str) -> (RinchDocument, RawNodeId, RawNodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let container = doc.create_element("div");
        doc.set_attribute(
            container,
            "style",
            &format!("width: 100px; height: 100px; {container_style}"),
        );
        doc.append_child(body, container);
        let child = doc.create_element("div");
        doc.set_attribute(
            child,
            "style",
            "width: 200px; height: 200px; background-color: rgb(255, 0, 0)",
        );
        doc.append_child(container, child);
        doc.resolve_layout(800.0, 600.0);
        (doc, raw(container), raw(child))
    }

    /// [`overhang`] with a background on the container and a 10px left margin
    /// on the child, so that the container's own fill stays visible in a strip
    /// the child cannot cover. That strip is what makes a clip-rect error
    /// legible: the background is painted before the bracket opens, so a clip
    /// that stops short of the border box shows blue where the child should be.
    fn boxed_overhang(container_style: &str) -> (RinchDocument, RawNodeId, RawNodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let container = doc.create_element("div");
        doc.set_attribute(
            container,
            "style",
            &format!(
                "width: 100px; height: 100px; background-color: rgb(0, 0, 255); \
                 {container_style}"
            ),
        );
        doc.append_child(body, container);
        let child = doc.create_element("div");
        doc.set_attribute(
            child,
            "style",
            "width: 200px; height: 200px; margin-left: 10px; \
             background-color: rgb(255, 0, 0)",
        );
        doc.append_child(container, child);
        doc.resolve_layout(800.0, 600.0);
        (doc, raw(container), raw(child))
    }

    fn raw(id: rinch_core::dom::NodeId) -> RawNodeId {
        id.0
    }

    /// The headline defect. `overflow: clip` reached hit testing's
    /// `overflow != Visible` and missed paint's `Hidden | Scroll | Auto`, so
    /// the overhang was drawn and was not clickable.
    #[test]
    fn overflow_clip_clips_pixels_on_both_axes() {
        let (mut doc, _container, _child) = overhang("overflow: clip");
        let mut painter = TinySkiaPainter::new(300, 300);
        paint(&mut doc, &mut painter);

        assert_eq!(
            pixel_at(&painter, 50, 50),
            RED,
            "the part of the child inside the container is still painted"
        );
        assert_eq!(
            pixel_at(&painter, 150, 50),
            NOTHING,
            "the horizontal overhang is clipped — this was red"
        );
        assert_eq!(
            pixel_at(&painter, 50, 150),
            NOTHING,
            "and so is the vertical overhang"
        );
        assert_eq!(pixel_at(&painter, 150, 150), NOTHING);
    }

    /// One axis is enough, and it is the axis the old predicate could not see.
    ///
    /// The vertical assertion pins a **known deviation, not the CSS answer**:
    /// `overflow-y` computes to `visible` here (css-overflow-3 pairs `clip`
    /// with `visible`), so CSS would paint the vertical overhang. rinch clips
    /// both axes with one rect, which hit testing has always done and paint now
    /// matches. Per-axis clipping is #535; when it lands, this assertion
    /// becomes `RED` and the comment goes away.
    #[test]
    fn overflow_x_clip_clips_without_the_other_axis_saying_anything() {
        let (mut doc, container, _child) = overhang("overflow-x: clip");
        assert_eq!(
            doc.tree.get(container).unwrap().computed_style.overflow_y,
            OverflowValue::Visible,
            "the fixture is only interesting while the y axis stays visible"
        );

        let mut painter = TinySkiaPainter::new(300, 300);
        paint(&mut doc, &mut painter);

        assert_eq!(pixel_at(&painter, 50, 50), RED);
        assert_eq!(
            pixel_at(&painter, 150, 50),
            NOTHING,
            "the x axis says clip, so the horizontal overhang goes"
        );
        assert_eq!(
            pixel_at(&painter, 50, 150),
            NOTHING,
            "known deviation: rinch clips both axes with one rect, so the \
             vertical overhang goes too. CSS would paint it."
        );
    }

    /// Paint and hit testing now answer the same question at every probe. Read
    /// as a pair rather than as two assertions: before the predicates were
    /// unified, (150, 50) was painted red *and* unreachable, which is the
    /// "drawn and not clickable" this issue is about.
    #[test]
    fn a_clipped_pixel_and_a_clipped_tap_are_the_same_set() {
        let (mut doc, _container, child) = overhang("overflow: clip");
        let mut painter = TinySkiaPainter::new(300, 300);
        paint(&mut doc, &mut painter);

        for (px, py) in [(50u32, 50u32), (150, 50), (50, 150), (150, 150), (95, 95)] {
            let drawn = pixel_at(&painter, px, py) == RED;
            let tapped =
                resolve(&doc.tree, doc.tree.body_id, 0.0, 0.0, px as f32, py as f32) == Some(child);
            assert_eq!(
                drawn, tapped,
                "at ({px}, {py}) the painter says drawn={drawn} and hit testing \
                 says reachable={tapped}"
            );
        }
    }

    /// The clip is the box's *rounded* border box, not its square one. This is
    /// the mutation guard for `clip_shape` losing its radii: at radius 0 a rect
    /// clip and a rounded clip agree everywhere, so the probe is a corner.
    ///
    /// (2, 2) is 53.7px from the corner circle's centre at (40, 40), so it is
    /// outside a 40px radius by a wide enough margin that antialiasing has
    /// nothing to say about it.
    #[test]
    fn a_rounded_clip_cuts_its_corners() {
        for overflow in ["overflow: hidden", "overflow: clip"] {
            let mut doc = RinchDocument::new();
            let body = doc.body();
            let container = doc.create_element("div");
            doc.set_attribute(
                container,
                "style",
                &format!("width: 100px; height: 100px; border-radius: 40px; {overflow}"),
            );
            doc.append_child(body, container);
            let child = doc.create_element("div");
            doc.set_attribute(
                child,
                "style",
                "width: 100px; height: 100px; background-color: rgb(255, 0, 0)",
            );
            doc.append_child(container, child);
            doc.resolve_layout(800.0, 600.0);

            let mut painter = TinySkiaPainter::new(300, 300);
            paint(&mut doc, &mut painter);

            assert_eq!(
                pixel_at(&painter, 50, 50),
                RED,
                "{overflow}: the middle of the box is filled"
            );
            assert_eq!(
                pixel_at(&painter, 2, 2),
                NOTHING,
                "{overflow}: the top-left corner is outside a 40px radius"
            );
        }
    }

    /// A clipping box owns its hoisted descendants, so gaining a clip has to
    /// mean gaining a stacking context. Before, an `overflow: clip` box was not
    /// one: its `z-index: 5` panel was collected into the **body's** sequence
    /// and painted there, outside any bracket the container might open.
    ///
    /// `z-index: 5` rather than `auto` on purpose — every entry at 0 is the
    /// fixed point where the sort key decides nothing.
    ///
    /// # The two halves, after stage B
    ///
    /// * The **pixel** assertions are the invariant, and are **unchanged** from
    ///   stage A. "A z-indexed descendant of a clipping box is clipped by it" is
    ///   true in CSS, was true through the stacking context, and is true now
    ///   through the hoisted entry's clip chain. They are the guard that stage B
    ///   did not silently un-fix stage A: drop the chain and keep the dropped
    ///   arm and (130, 130) goes green.
    /// * The **ordering** assertion is the one stage B inverted, exactly as
    ///   stage A predicted it would. The container is no longer a stacking
    ///   context, so the panel *is* collected into the body's sequence — and
    ///   carries a one-link chain naming the container, which is what clips it
    ///   there.
    #[test]
    fn a_z_indexed_child_of_a_clip_box_is_clipped_by_it() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let container = doc.create_element("div");
        doc.set_attribute(
            container,
            "style",
            "position: relative; width: 100px; height: 100px; overflow: clip",
        );
        doc.append_child(body, container);
        let panel = doc.create_element("div");
        doc.set_attribute(
            panel,
            "style",
            "position: absolute; left: 50px; top: 50px; width: 100px; height: 100px; \
             z-index: 5; background-color: rgb(0, 255, 0)",
        );
        doc.append_child(container, panel);
        doc.resolve_layout(800.0, 600.0);

        // ── The ordering assertion: inverted by stage B, chain and all ──
        let body_order = stacking_paint_order(&doc.tree, doc.tree.body_id, true, 1.0, 0.0, 0.0);
        let entry = body_order.iter().find(|e| e.node_id == raw(panel)).expect(
            "the panel is hoisted to the body now: an `overflow: clip` box \
                 is not a stacking context and does not stop the walk",
        );
        let chain = body_order.clips_for(entry);
        assert_eq!(
            chain.len(),
            1,
            "and it carries exactly the one clipping ancestor it passed"
        );
        assert_eq!(
            chain[0].rect,
            peniko::kurbo::Rect::new(0.0, 0.0, 100.0, 100.0),
            "which is the container's own border box"
        );

        // ── The pixel assertions: the invariant, which must survive stage B ──
        let mut painter = TinySkiaPainter::new(300, 300);
        paint(&mut doc, &mut painter);

        assert_eq!(
            pixel_at(&painter, 75, 75),
            GREEN,
            "the panel still paints where it overlaps its container"
        );
        assert_eq!(
            pixel_at(&painter, 130, 130),
            NOTHING,
            "and stops at the container's edge instead of escaping to the body. \
             After stage B this must hold through the entry's clip chain — if \
             it goes green-to-red there, the arm was dropped without one"
        );
    }

    /// The clip rect must be the box the background paints, to the pixel.
    ///
    /// `clip_shape` **re-derives** its rect from `node.layout` and `scale`
    /// rather than being handed the `rect` its caller already holds, so the two
    /// can drift apart — a degree of freedom that did not exist before #324
    /// stage A extracted the function, because there was only ever one rect.
    /// Measured by the review: a 3px error in `clip_shape` killed **nothing**,
    /// while the same 3px error in `paint_node`'s shared `rect` killed 18
    /// tests. This pins it, and pins it for stage B, which adds a second caller
    /// that computes a clip for a node it holds no rect for at all — which is
    /// exactly why `clip_shape` derives rather than receives, and why the
    /// agreement has to be a test instead of a signature.
    ///
    /// **The container's own background is the oracle.** It is painted *before*
    /// the clip bracket opens and covers the whole border box, so a clip that
    /// stops short leaves a strip of it showing, and a clip that overreaches
    /// lets the child paint onto bare pixmap past the box. One probe either
    /// side of the edge catches both, at ±1px rather than the ±5px the scrolled
    /// fixture's outside probe manages.
    #[test]
    fn the_clip_rect_is_the_box_the_background_paints() {
        let (mut doc, _c, _ch) = boxed_overhang("overflow: hidden");
        let mut painter = TinySkiaPainter::new(300, 300);
        paint(&mut doc, &mut painter);

        assert_eq!(
            pixel_at(&painter, 5, 50),
            BLUE,
            "the child's 10px left margin leaves the container's background \
             showing — if this is not blue the fixture never painted and the \
             edge probes below prove nothing"
        );

        assert_eq!(
            pixel_at(&painter, 99, 50),
            RED,
            "the clip reaches the box's right edge: a rect even 1px short \
             would show the container's blue background here"
        );
        assert_eq!(
            pixel_at(&painter, 100, 50),
            NOTHING,
            "…and stops there: a rect even 1px long would paint the child onto \
             bare pixmap outside the container"
        );

        assert_eq!(
            pixel_at(&painter, 50, 99),
            RED,
            "the same, on the bottom edge — a rect wrong on one axis only is \
             the mutant a single-axis probe misses"
        );
        assert_eq!(pixel_at(&painter, 50, 100), NOTHING);
    }

    /// The clip rect is in painter units, so it scales with the DPI scale.
    ///
    /// `scale = 1.0` is the fixed point where a dropped `* scale` is invisible,
    /// and every other fixture in this file sits on it — as does every pixel
    /// fixture in the repo. (`paint_tests::test_paint_at_scale` is the only
    /// scale-2 test there is, and it asserts a Vello scene is non-empty, which
    /// no clip-rect error can disturb.) So this is the one that has to leave it.
    ///
    /// At scale 2 the container's border box is 200 physical px and the child
    /// reaches 400. Drop `* scale` from `clip_shape` and the clip stays 100
    /// wide while the background it is supposed to match is 200: the outer half
    /// of the box turns blue.
    #[test]
    fn the_clip_rect_scales_with_the_dpi_scale() {
        let (mut doc, _c, _ch) = boxed_overhang("overflow: hidden");
        let mut painter = TinySkiaPainter::new(500, 500);
        paint_at(&mut doc, &mut painter, 2.0);

        assert_eq!(
            pixel_at(&painter, 10, 100),
            BLUE,
            "the 10px margin is 20 physical px, so the background still shows"
        );
        assert_eq!(
            pixel_at(&painter, 150, 100),
            RED,
            "an unscaled clip rect would end at physical x = 100 and leave the \
             container's background showing from here out"
        );
        assert_eq!(pixel_at(&painter, 199, 100), RED, "right up to the edge");
        assert_eq!(
            pixel_at(&painter, 200, 100),
            NOTHING,
            "and no further — the box is 200 physical px, not 100 and not 400"
        );
        assert_eq!(pixel_at(&painter, 100, 199), RED, "and on the other axis");
        assert_eq!(pixel_at(&painter, 100, 200), NOTHING);
    }

    /// #408: the culled-node branch paints its children at the **scrolled**
    /// content origin, like every other call in `paint_node`.
    ///
    /// The branch is unreachable with a real scroll offset — a node only
    /// carries one if it scrolls, and a node that scrolls answers
    /// `clips_overflow`, so the guard above returns first — which is why this
    /// was filed as a latent trap rather than a symptom, and why the only test
    /// that can see it is a white-box one. The offset is written straight onto
    /// an `overflow: visible` node here to construct the state the guard
    /// forbids, so that the arithmetic is pinned by something other than that
    /// guard staying where it is. Anyone who relaxes it — to let a clipping
    /// container's out-of-flow descendants paint while the container is culled,
    /// say — inherits a correct call instead of a whole subtree displaced by
    /// `scroll_offset`.
    #[test]
    fn a_culled_node_paints_its_children_at_the_scrolled_origin() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        // 300x10, so its own box misses the dirty region below and it takes the
        // culled branch — while its overflowing child reaches into the region.
        let outer = doc.create_element("div");
        doc.set_attribute(outer, "style", "width: 300px; height: 10px");
        doc.append_child(body, outer);
        let child = doc.create_element("div");
        doc.set_attribute(
            child,
            "style",
            "width: 40px; height: 240px; background-color: rgb(255, 0, 0)",
        );
        doc.append_child(outer, child);
        doc.resolve_layout(800.0, 600.0);

        assert!(
            !doc.tree.get(raw(outer)).unwrap().clips_overflow(),
            "the fixture only reaches the branch while `outer` does not clip"
        );
        doc.tree.nodes[raw(outer)].scroll_offset = (0.0, 50.0);

        rinch_dom::paint::set_dirty_region(Some(peniko::kurbo::Rect::new(
            0.0, 100.0, 300.0, 300.0,
        )));
        let mut painter = TinySkiaPainter::new(300, 300);
        paint(&mut doc, &mut painter);
        rinch_dom::paint::set_dirty_region(None);

        assert_eq!(
            pixel_at(&painter, 20, 150),
            RED,
            "the child is painted at all — if it is not, the fixture stopped \
             reaching the branch and the assertion below proves nothing"
        );
        assert_eq!(
            pixel_at(&painter, 20, 210),
            NOTHING,
            "scrolled 50px up, the 240px child ends at y = 190; painting it at \
             the unscrolled origin would carry it to y = 240"
        );
    }

    /// The clip rect is the container's own border box, not the scrolled origin
    /// its children are laid out against. Every probe here is at a **non-zero**
    /// scroll offset, because at offset 0 the two origins coincide and a clip
    /// computed from the wrong one is invisible — #439's exact shape.
    ///
    /// `overflow: auto` and unchanged by this PR: this is the refactor's pin,
    /// not the fix's.
    #[test]
    fn a_scrolled_container_clips_at_its_own_box() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let container = doc.create_element("div");
        doc.set_attribute(
            container,
            "style",
            "width: 100px; height: 100px; overflow: auto",
        );
        doc.append_child(body, container);
        for colour in ["rgb(255, 0, 0)", "rgb(0, 255, 0)"] {
            let block = doc.create_element("div");
            doc.set_attribute(
                block,
                "style",
                &format!("width: 60px; height: 100px; background-color: {colour}"),
            );
            doc.append_child(container, block);
        }
        doc.resolve_layout(800.0, 600.0);
        doc.tree.nodes[raw(container)].scroll_offset = (0.0, 50.0);

        let mut painter = TinySkiaPainter::new(300, 300);
        paint(&mut doc, &mut painter);

        // Scrolled 50px down: red now occupies y in [0, 50), green y in [50, 150).
        assert_eq!(pixel_at(&painter, 30, 25), RED);
        assert_eq!(
            pixel_at(&painter, 30, 75),
            GREEN,
            "a clip taken at the scrolled origin would end at y = 50 and cut \
             this away"
        );
        assert_eq!(
            pixel_at(&painter, 30, 95),
            GREEN,
            "…right up to the container's own bottom edge"
        );
        assert_eq!(
            pixel_at(&painter, 30, 105),
            NOTHING,
            "and no further: the green block reaches y = 150 unclipped"
        );
    }
}
