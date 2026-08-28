//! The one paint sequence, and both directions of reading it.
//!
//! `rinch_dom::stacking::stacking_paint_order` is what the painter walks
//! forwards and what hit testing walks backwards, so these tests pin the order
//! itself, then pin both readings of it: pixels for the forward one, a resolved
//! node for the reverse one.

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::computed_style::OverflowValue;
use rinch_dom::node::{NodeTree, RawNodeId};
use rinch_dom::stacking::{PaintEntry, PaintKind, paints_at_stacking_root, stacking_paint_order};
use rinch_dom::{RinchDocument, node::LayoutResult};

/// The body's paint sequence: what `paint_children_with_stacking` walks for the
/// root, and what hit testing walks in reverse.
fn body_order(doc: &RinchDocument) -> Vec<PaintEntry> {
    stacking_paint_order(&doc.tree, doc.tree.body_id, true, 1.0, 0.0, 0.0)
}

fn ids(order: &[PaintEntry]) -> Vec<RawNodeId> {
    order.iter().map(|e| e.node_id).collect()
}

/// `DomDocument`'s handle, as the tree's own index.
fn raw(id: rinch_core::dom::NodeId) -> RawNodeId {
    id.0
}

/// Hit testing, reduced to the part these tests are about: walk the shared
/// sequence backwards and take the first box that answers. This is
/// `rinch`'s `hit_test_node` with transforms, `position: fixed`, visibility and
/// `pointer-events` taken out — everything that remains is the ordering.
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

    let clips = !matches!(node.computed_style.overflow_x, OverflowValue::Visible)
        || !matches!(node.computed_style.overflow_y, OverflowValue::Visible);
    if !clips || inside {
        let (sx, sy) = (node.scroll_offset.0 as f32, node.scroll_offset.1 as f32);
        let is_body = id == tree.body_id;
        if is_body || node.creates_stacking_context() {
            let order =
                stacking_paint_order(tree, id, is_body, 1.0, (nx - sx) as f64, (ny - sy) as f64);
            for entry in order.iter().rev() {
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

/// A scrolling list with a floating action button over it — the arrangement the
/// bug was found in. The scroller is a stacking context (Rinch makes one for
/// `overflow`), the FAB is `position: absolute` with no `z-index`, and the FAB
/// comes second in the markup.
struct ScrollerAndFab {
    doc: RinchDocument,
    scroller: RawNodeId,
    row: RawNodeId,
    fab: RawNodeId,
}

fn scroller_and_fab(fab_first: bool) -> ScrollerAndFab {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "position: relative");

    let fab = doc.create_element("div");
    doc.set_attribute(
        fab,
        "style",
        "position: absolute; left: 150px; top: 150px; width: 56px; height: 56px; \
         background-color: rgb(255, 0, 0)",
    );

    let scroller = doc.create_element("div");
    doc.set_attribute(
        scroller,
        "style",
        "overflow: auto; width: 200px; height: 200px; background-color: rgb(0, 0, 255)",
    );
    let row = doc.create_element("div");
    doc.set_attribute(
        row,
        "style",
        "width: 200px; height: 400px; background-color: rgb(0, 0, 255)",
    );
    doc.append_child(scroller, row);

    if fab_first {
        doc.append_child(body, fab);
        doc.append_child(body, scroller);
    } else {
        doc.append_child(body, scroller);
        doc.append_child(body, fab);
    }

    doc.resolve_layout(800.0, 600.0);
    ScrollerAndFab {
        doc,
        scroller: raw(scroller),
        row: raw(row),
        fab: raw(fab),
    }
}

#[test]
fn a_positioned_z_auto_box_is_ordered_above_an_earlier_scroller() {
    let f = scroller_and_fab(false);
    let order = body_order(&f.doc);

    assert_eq!(
        ids(&order),
        vec![f.scroller, f.fab],
        "the FAB is a step-8 entry at z == 0 and comes later in the markup, so it \
         paints after the scroller — not before it, and not in a phase the \
         scroller's z == 0 stacking context is allowed to paint over"
    );
    assert_eq!(order[0].kind, PaintKind::StackingContext);
    assert_eq!(order[1].kind, PaintKind::PositionedAuto);
    assert_eq!(order[1].z_index, 0, "`z-index: auto` enters at z == 0");
}

#[test]
fn a_tap_over_the_positioned_box_resolves_to_it_and_not_the_scroller() {
    let f = scroller_and_fab(false);

    // (170, 170) is inside the FAB *and* inside the scroller's 400px-tall row.
    assert_eq!(
        resolve(&f.doc.tree, f.doc.tree.body_id, 0.0, 0.0, 170.0, 170.0),
        Some(f.fab),
        "reading the same sequence backwards must reach the FAB first — this is \
         the tap that fell through to the row underneath"
    );
    // …and a point clear of the FAB still reaches the row, so the FAB is not
    // simply swallowing the whole overlap.
    assert_eq!(
        resolve(&f.doc.tree, f.doc.tree.body_id, 0.0, 0.0, 40.0, 40.0),
        Some(f.row),
    );
}

#[test]
fn tree_order_decides_between_a_positioned_box_and_a_scroller_at_the_same_level() {
    let f = scroller_and_fab(true);
    let order = body_order(&f.doc);

    assert_eq!(
        ids(&order),
        vec![f.fab, f.scroller],
        "both enter at z == 0, so the one written later wins"
    );
    assert_eq!(
        resolve(&f.doc.tree, f.doc.tree.body_id, 0.0, 0.0, 170.0, 170.0),
        Some(f.row),
        "and the reverse read agrees with the forward one"
    );
}

#[test]
fn a_positioned_z_auto_box_is_ordered_above_in_flow_content_written_after_it() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "position: relative");

    let fab = doc.create_element("div");
    doc.set_attribute(
        fab,
        "style",
        "position: absolute; left: 20px; top: 20px; width: 40px; height: 40px",
    );
    doc.append_child(body, fab);

    let block = doc.create_element("div");
    doc.set_attribute(block, "style", "width: 200px; height: 100px");
    doc.append_child(body, block);

    doc.resolve_layout(800.0, 600.0);
    let order = stacking_paint_order(&doc.tree, doc.tree.body_id, true, 1.0, 0.0, 0.0);

    assert_eq!(
        ids(&order),
        vec![raw(block), raw(fab)],
        "in-flow content is step 4 and a positioned box is step 8, whatever the \
         markup order says"
    );
    assert_eq!(order[0].kind, PaintKind::InFlow);
    assert_eq!(
        resolve(&doc.tree, doc.tree.body_id, 0.0, 0.0, 30.0, 30.0),
        Some(raw(fab)),
    );
}

#[test]
fn a_negative_z_index_context_stays_below_in_flow_content() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "position: relative");

    let behind = doc.create_element("div");
    doc.set_attribute(
        behind,
        "style",
        "position: absolute; z-index: -1; left: 0; top: 0; width: 100px; height: 100px",
    );
    doc.append_child(body, behind);

    let block = doc.create_element("div");
    doc.set_attribute(block, "style", "width: 200px; height: 100px");
    doc.append_child(body, block);

    let above = doc.create_element("div");
    doc.set_attribute(
        above,
        "style",
        "position: absolute; z-index: 5; left: 0; top: 0; width: 100px; height: 100px",
    );
    doc.append_child(body, above);

    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        ids(&stacking_paint_order(
            &doc.tree,
            doc.tree.body_id,
            true,
            1.0,
            0.0,
            0.0
        )),
        vec![raw(behind), raw(block), raw(above)],
    );
}

#[test]
fn a_stacking_context_under_a_positioned_z_auto_box_belongs_to_the_ancestor() {
    // Appendix E step 8's second half: a positioned `z-index: auto` box is
    // entered as if it made a stacking context, "but any positioned descendants
    // and descendants which actually create a new stacking context should be
    // considered part of the parent stacking context". So the nested z-index: 3
    // box must surface in the *body's* sequence, above everything at z == 0 —
    // not be trapped inside the box it is written in.
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "position: relative");

    let scroller = doc.create_element("div");
    doc.set_attribute(
        scroller,
        "style",
        "overflow: auto; width: 200px; height: 200px",
    );
    doc.append_child(body, scroller);

    let panel = doc.create_element("div");
    doc.set_attribute(
        panel,
        "style",
        "position: absolute; left: 0; top: 0; width: 100px; height: 100px",
    );
    doc.append_child(body, panel);

    let badge = doc.create_element("div");
    doc.set_attribute(
        badge,
        "style",
        "position: absolute; z-index: 3; left: 0; top: 0; width: 20px; height: 20px",
    );
    doc.append_child(panel, badge);

    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        ids(&stacking_paint_order(
            &doc.tree,
            doc.tree.body_id,
            true,
            1.0,
            0.0,
            0.0
        )),
        vec![raw(scroller), raw(panel), raw(badge)],
    );
}

#[test]
fn a_fixed_box_inside_a_scroller_is_hoisted_to_the_viewport_with_no_offset() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let spacer = doc.create_element("div");
    doc.set_attribute(spacer, "style", "width: 10px; height: 120px");
    doc.append_child(body, spacer);

    let scroller = doc.create_element("div");
    doc.set_attribute(
        scroller,
        "style",
        "overflow: auto; width: 200px; height: 200px",
    );
    doc.append_child(body, scroller);

    let modal = doc.create_element("div");
    doc.set_attribute(
        modal,
        "style",
        "position: fixed; left: 300px; top: 300px; width: 100px; height: 100px",
    );
    doc.append_child(scroller, modal);

    doc.resolve_layout(800.0, 600.0);

    let root = stacking_paint_order(&doc.tree, doc.tree.body_id, true, 1.0, 0.0, 0.0);
    assert_eq!(ids(&root), vec![raw(spacer), raw(scroller), raw(modal)]);
    let entry = root.iter().find(|e| e.node_id == raw(modal)).unwrap();
    assert_eq!(
        (entry.offset_x, entry.offset_y),
        (0.0, 0.0),
        "a fixed box's layout is already viewport-relative, so it gets no \
         accumulated offset"
    );

    // …and the scroller does not paint it a second time inside its own clip.
    let inner = stacking_paint_order(&doc.tree, raw(scroller), false, 1.0, 0.0, 120.0);
    assert!(inner.is_empty(), "the body already has it: {inner:?}");
}

/// A text node is not a box, so it cannot be a positioned descendant, so it is
/// never hoisted out of the parent that flows it.
///
/// This is the invariant card K20 turned out to be about. Style resolution runs
/// on elements only, so a text node keeps `ComputedStyle::default()` for its
/// whole life — and `PositionValue` used to default to `Relative`, which made
/// `is_positioned_z_auto` answer `true` for every text node in the document.
/// Each one was then hoisted into the nearest stacking-context ancestor's
/// sequence, where the guard that stops an IFC root's children being painted a
/// second time (`already_drawn_inline`, which only recognises a child of the
/// node it is called on) cannot reach it. The result was that every run of text
/// in an inline formatting context was painted twice — see
/// `text_in_a_padded_ifc_root_is_painted_once` for the picture that made.
#[test]
fn a_text_node_is_never_hoisted_out_of_the_box_that_flows_it() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let para = doc.create_element("div");
    doc.set_attribute(para, "style", "font-size: 20px");
    doc.append_child(body, para);
    let text = doc.create_text("Solid");
    doc.append_child(para, text);

    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(raw(text)).expect("the text node is in the tree");
    assert!(
        !paints_at_stacking_root(node),
        "a text node paints in its parent's tree-order run; hoisting it puts it \
         somewhere `already_drawn_inline` cannot skip it and the run is painted twice"
    );
    assert_eq!(
        ids(&body_order(&doc)),
        vec![raw(para)],
        "the body's sequence is the paragraph and nothing else — the text inside \
         it belongs to the paragraph's inline layout, not to this list"
    );
}

// ── The forward read: pixels ────────────────────────────────────────────────

#[cfg(feature = "software-renderer")]
mod painted {
    use super::*;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    fn paint(doc: &mut RinchDocument, painter: &mut TinySkiaPainter) {
        let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            painter,
            1.0,
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

    /// The bug report held that only taps were affected — that "the painter
    /// agrees with CSS, so this only ever showed up as a dead button and never
    /// as a wrong picture". It did not: with the FAB in step 4 and the scroller
    /// in the z >= 0 phase that runs after it, the scroller was painted *over*
    /// the FAB. The button was invisible as well as untappable.
    #[test]
    fn a_scroller_does_not_paint_over_a_later_positioned_box() {
        let mut f = scroller_and_fab(false);
        let mut painter = TinySkiaPainter::new(300, 300);
        paint(&mut f.doc, &mut painter);

        assert_eq!(
            pixel_at(&painter, 170, 170),
            [255, 0, 0, 255],
            "the FAB is on top of the scroller it overlaps"
        );
        assert_eq!(
            pixel_at(&painter, 40, 40),
            [0, 0, 255, 255],
            "and the scroller is otherwise undisturbed"
        );
    }

    /// The same gap without any `overflow` involved: an `opacity` stacking
    /// context is a z == 0 entry too, and an earlier one must not cover a
    /// positioned box written after it.
    #[test]
    fn an_opacity_context_does_not_paint_over_a_later_positioned_box() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        doc.set_attribute(body, "style", "position: relative");

        let faded = doc.create_element("div");
        doc.set_attribute(
            faded,
            "style",
            "opacity: 0.99; width: 200px; height: 200px; background-color: rgb(0, 0, 255)",
        );
        doc.append_child(body, faded);

        let fab = doc.create_element("div");
        doc.set_attribute(
            fab,
            "style",
            "position: absolute; left: 150px; top: 150px; width: 56px; height: 56px; \
             background-color: rgb(255, 0, 0)",
        );
        doc.append_child(body, fab);

        doc.resolve_layout(800.0, 600.0);
        let mut painter = TinySkiaPainter::new(300, 300);
        paint(&mut doc, &mut painter);

        let p = pixel_at(&painter, 170, 170);
        assert!(
            p[0] > 200 && p[2] < 50,
            "the FAB paints over the faded box, got {p:?}"
        );
    }

    /// In-flow content written *after* a positioned box still paints below it.
    #[test]
    fn in_flow_content_does_not_paint_over_an_earlier_positioned_box() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        doc.set_attribute(body, "style", "position: relative");

        let fab = doc.create_element("div");
        doc.set_attribute(
            fab,
            "style",
            "position: absolute; left: 20px; top: 20px; width: 40px; height: 40px; \
             background-color: rgb(255, 0, 0)",
        );
        doc.append_child(body, fab);

        let block = doc.create_element("div");
        doc.set_attribute(
            block,
            "style",
            "width: 200px; height: 100px; background-color: rgb(0, 0, 255)",
        );
        doc.append_child(body, block);

        doc.resolve_layout(800.0, 600.0);
        let mut painter = TinySkiaPainter::new(300, 300);
        paint(&mut doc, &mut painter);

        assert_eq!(pixel_at(&painter, 30, 30), [255, 0, 0, 255]);
    }

    /// One run of text, painted once — the fault behind card K20.
    ///
    /// An IFC root draws its text out of the Parley layout at its *content-box*
    /// origin, inside the padding. The standalone text path in `paint_node`
    /// draws the raw DOM string at the node's own layout position, which
    /// `write_inline_positions` zeroes to the root's *border-box* origin. So
    /// when a text node was wrongly hoisted (see
    /// `a_text_node_is_never_hoisted_out_of_the_box_that_flows_it`) and both
    /// paths ran, the second copy landed a whole padding to the left of the
    /// first — a chip with `padding: 6px 12px` drew its label twice, a line and
    /// a padding apart, which is how this was first seen on a phone.
    ///
    /// The padding strip is therefore the oracle: it is the one place on the
    /// screen where the correct render puts no ink at all.
    #[test]
    fn text_in_a_padded_ifc_root_is_painted_once() {
        let mut doc = RinchDocument::new();
        let body = doc.body();

        let chip = doc.create_element("div");
        doc.set_attribute(
            chip,
            "style",
            "padding-left: 40px; font-size: 20px; color: rgb(255, 0, 0)",
        );
        doc.append_child(body, chip);
        let text = doc.create_text("Solid");
        doc.append_child(chip, text);

        doc.resolve_layout(800.0, 600.0);
        let mut painter = TinySkiaPainter::new(300, 60);
        paint(&mut doc, &mut painter);

        let ink = |x0: u32, x1: u32| {
            let mut n = 0;
            for y in 0..painter.height() {
                for x in x0..x1 {
                    if pixel_at(&painter, x, y)[3] > 0 {
                        n += 1;
                    }
                }
            }
            n
        };

        assert!(
            ink(40, 300) > 0,
            "the text itself is painted, inside the padding — if this is zero the \
             test is measuring an empty document and proves nothing"
        );
        assert_eq!(
            ink(0, 39),
            0,
            "nothing is painted in the padding strip; ink here is the second, \
             un-transformed copy of the run, drawn at the border-box origin"
        );
    }
}
