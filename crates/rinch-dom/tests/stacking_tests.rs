//! The one paint sequence, and both directions of reading it.
//!
//! `rinch_dom::stacking::stacking_paint_order` is what the painter walks
//! forwards and what hit testing walks backwards, so these tests pin the order
//! itself, then pin both readings of it: pixels for the forward one, a resolved
//! node for the reverse one.

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::computed_style::PositionValue;
use rinch_dom::node::{NodeTree, RawNodeId};
use rinch_dom::stacking::{PaintEntry, PaintKind, paints_at_stacking_root, stacking_paint_order};
use rinch_dom::{RinchDocument, node::LayoutResult};

/// The body's paint sequence: what `paint_children_with_stacking` walks for the
/// root, and what hit testing walks in reverse.
fn body_order(doc: &RinchDocument) -> rinch_dom::stacking::PaintOrder {
    stacking_paint_order(&doc.tree, doc.tree.body_id, 1.0, 0.0, 0.0)
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
    let check_children = !node.clips_overflow() || inside;

    {
        let (sx, sy) = (node.scroll_offset.0 as f32, node.scroll_offset.1 as f32);
        if id == tree.body_id || node.creates_stacking_context() {
            let order = stacking_paint_order(tree, id, 1.0, (nx - sx) as f64, (ny - sy) as f64);
            for entry in order.iter().rev() {
                // This root's own bounds gate, per entry, and a `position: fixed`
                // entry is exempt from it — this root is not its containing block
                // (#545). Mirrors `hit_test_node`; that exemption is ordering, not
                // coordinates, so it belongs in this reduced model even though the
                // viewport re-seed does not.
                let escapes_root_clip = tree
                    .get(entry.node_id)
                    .is_some_and(|c| c.computed_style.position == PositionValue::Fixed);
                if !check_children && !escapes_root_clip {
                    continue;
                }
                // The entry's clip chain: the clipping ancestors it was hoisted
                // past, which the gate above never sees because `overflow` is not
                // a stacking context (#324 stage B). Rect-only, like that gate.
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
        } else if check_children {
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
/// bug was found in. The FAB is `position: absolute` with no `z-index`, so it is
/// an Appendix E step-8 entry; the scroller is a plain in-flow box, which is a
/// step-4 one. It used to be a stacking context, because Rinch made one for
/// every `overflow` — #324 stage B stopped, so the two now sit in the CSS phases
/// they belong to rather than both landing at `z == 0`.
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
        "the FAB is a step-8 entry and the scroller a step-4 one, so the FAB \
         paints after it — this is the tap that used to fall through to the row"
    );
    assert_eq!(
        order[0].kind,
        PaintKind::InFlow,
        "the scroller is an ordinary in-flow child since #324 stage B: `overflow` \
         is a clip, not a stacking context"
    );
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

/// A positioned box beats an in-flow scroller **whatever the tree order** —
/// Appendix E puts in-flow non-positioned content (step 4) below positioned
/// `z-index: auto` content (step 8), and tree order only breaks ties *within* a
/// step.
///
/// This assertion is the inverse of the one #324 stage B replaced. It used to
/// read `[fab, scroller]` with "both enter at z == 0, so the one written later
/// wins", which was true only because `overflow` made the scroller a stacking
/// context and dragged it up into step 8 beside the FAB. A browser has always
/// painted the FAB on top here; rinch does now too.
#[test]
fn a_positioned_box_beats_an_in_flow_scroller_written_after_it() {
    let f = scroller_and_fab(true);
    let order = body_order(&f.doc);

    assert_eq!(
        ids(&order),
        vec![f.scroller, f.fab],
        "in-flow content paints below positioned content, markup order or not"
    );
    assert_eq!(
        resolve(&f.doc.tree, f.doc.tree.body_id, 0.0, 0.0, 170.0, 170.0),
        Some(f.fab),
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
    let order = stacking_paint_order(&doc.tree, doc.tree.body_id, 1.0, 0.0, 0.0);

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

    let root = stacking_paint_order(&doc.tree, doc.tree.body_id, 1.0, 0.0, 0.0);
    assert_eq!(ids(&root), vec![raw(spacer), raw(scroller), raw(modal)]);
    let entry = root.iter().find(|e| e.node_id == raw(modal)).unwrap();
    assert_eq!(
        (entry.offset_x, entry.offset_y),
        (0.0, 0.0),
        "a fixed box's layout is already viewport-relative, so it gets no \
         accumulated offset"
    );

    // …and nothing paints it a second time.
    //
    // This used to be checked by asking the *scroller* for a sequence and
    // requiring it to be empty, and that proved less than it looked. `overflow`
    // is not a stacking context (#324 stage B), so paint never asks a plain
    // scroller for a sequence at all — `paint_children_with_stacking` gates on
    // `is_body || creates_stacking_context()` — and the assertion was about a
    // call no consumer makes. It also only passed because of the very
    // fixed-box special case #545 removed; `stacking_paint_order` has never
    // filtered its answer by whether the caller *should* have asked, so the same
    // question about a `z-index: 5` child would always have answered "yes".
    //
    // What actually keeps a box from being painted twice is that exactly one
    // root paint asks claims it. So ask each candidate root the way paint
    // decides to.
    let claims_modal = |root: RawNodeId| {
        let node = doc.tree.get(root).unwrap();
        (root == doc.tree.body_id || node.creates_stacking_context())
            && stacking_paint_order(&doc.tree, root, 1.0, 0.0, 0.0)
                .iter()
                .any(|e| e.node_id == raw(modal))
    };
    let claimed: Vec<RawNodeId> = [doc.tree.body_id, raw(scroller)]
        .into_iter()
        .filter(|&root| claims_modal(root))
        .collect();
    assert_eq!(
        claimed,
        vec![doc.tree.body_id],
        "exactly one root paint asks owns the modal, and with no stacking \
         context between it and the body that root is the body"
    );
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

    let node = doc
        .tree
        .get(raw(text))
        .expect("the text node is in the tree");
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

    pub(super) fn pixel_at(painter: &TinySkiaPainter, x: u32, y: u32) -> [u8; 4] {
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

// ── #545: a fixed box is hoisted to its NEAREST ancestor stacking context ───
//
// A `position: fixed` box is viewport-*positioned*, not viewport-*stacked*. It
// used to be pulled out to the body's sequence whatever lay between, so a
// `z-index: 99` dismiss backdrop escaped a wrapper its `z-index: 100` panel
// could not, and the two numbers were compared across stacking contexts — which
// is what #324 exists to stop CSS never doing.
//
// Each fixture below is a popup under a wrapper, and the wrapper's style is the
// only thing that varies. Two rules for choosing one, learned the hard way in
// this repo: never give the wrapper `z-index: 0`, where the sort key decides
// nothing and a broken hoist and a correct one agree; and for a dismissal
// sample, never a point *inside* the popup's own panel, which both answers
// cover.

/// `body > wrapper > menu(position: relative) > { backdrop, panel > item }`.
///
/// The z-indexes are `DropdownMenu`'s own: the backdrop at 99 under the panel at
/// 100, so a tap on an item runs the item and a tap anywhere else dismisses.
/// Returns `(doc, wrapper, menu, backdrop, panel)`.
fn popup_under(wrapper_style: &str) -> (RinchDocument, RawNodeId, RawNodeId, RawNodeId, RawNodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let wrapper = doc.create_element("div");
    doc.set_attribute(wrapper, "style", wrapper_style);
    doc.append_child(body, wrapper);

    let menu = doc.create_element("div");
    doc.set_attribute(
        menu,
        "style",
        "position: relative; width: 120px; height: 32px",
    );
    doc.append_child(wrapper, menu);

    let backdrop = doc.create_element("div");
    doc.set_attribute(
        backdrop,
        "style",
        "position: fixed; top: 0; left: 0; right: 0; bottom: 0; z-index: 99",
    );
    doc.append_child(menu, backdrop);

    let panel = doc.create_element("div");
    doc.set_attribute(
        panel,
        "style",
        "position: absolute; top: 32px; left: 0; width: 160px; height: 120px; z-index: 100",
    );
    doc.append_child(menu, panel);

    doc.resolve_layout(800.0, 600.0);
    (doc, raw(wrapper), raw(menu), raw(backdrop), raw(panel))
}

/// A point on the panel, which is at (0, 32) 160x120 inside the wrapper.
const ON_PANEL: (f32, f32) = (80.0, 80.0);

#[test]
fn a_fixed_box_is_hoisted_no_further_than_its_nearest_stacking_context() {
    let (doc, wrapper, menu, backdrop, panel) = popup_under(
        "position: relative; opacity: 0.5; overflow: hidden; width: 400px; height: 300px",
    );

    assert_eq!(
        ids(&body_order(&doc)),
        vec![wrapper],
        "the body's sequence holds the wrapper and nothing from inside it — the \
         backdrop used to surface here, past a stacking context CSS says owns it"
    );

    let inner = stacking_paint_order(&doc.tree, wrapper, 1.0, 0.0, 0.0);
    assert_eq!(
        ids(&inner),
        vec![menu, backdrop, panel],
        "both boxes belong to the wrapper's sequence — the `position: relative` \
         menu at z == 0, then backdrop (99) painted before panel (100), i.e. \
         under it"
    );
}

/// The consequence, read the other way: the tap that #317 lost.
///
/// This is the exact fault that made `position: fixed` unusable for a dismiss
/// backdrop. Chromium answers the panel here (measured with `elementFromPoint`
/// on the same CSS); before #545 rinch answered the backdrop, because the 99 had
/// escaped to the body while the 100 stayed in the wrapper.
#[test]
fn a_tap_on_the_panel_beats_the_fixed_backdrop_under_an_opacity_wrapper() {
    let (doc, _, _, _, panel) = popup_under(
        "position: relative; opacity: 0.5; overflow: hidden; width: 400px; height: 300px",
    );
    let (x, y) = ON_PANEL;
    assert_eq!(
        resolve(&doc.tree, doc.tree.body_id, 0.0, 0.0, x, y),
        Some(panel),
    );
}

/// The same, with the wrapper's context spelled by an explicit `z-index`
/// instead — and deliberately **not** `z-index: 0`, which is the fixed point
/// here: at 0 the wrapper sorts identically to the body's other `z == 0` content
/// and a backdrop that wrongly escaped would land at 99 above it either way, so
/// the sample could not tell a fixed hoist from a correct one.
#[test]
fn a_tap_on_the_panel_beats_the_fixed_backdrop_under_a_z_index_wrapper() {
    let (doc, _, _, _, panel) = popup_under(
        "position: relative; z-index: 1; overflow: hidden; width: 400px; height: 300px",
    );
    let (x, y) = ON_PANEL;
    assert_eq!(
        resolve(&doc.tree, doc.tree.body_id, 0.0, 0.0, x, y),
        Some(panel),
    );
}

/// The other direction, and the one that hurts in practice: a wrapper whose own
/// `z` is **above** the backdrop's 99 — `Modal` (`z-index: 201`), `Drawer`
/// (201), `Notification` (300). A backdrop hoisted to the body at 99 sits
/// *under* the whole wrapper, so a tap anywhere inside it — the ordinary "click
/// somewhere else to dismiss" — reached nothing at all and the popup stayed open.
///
/// Sampled inside the wrapper and clear of the panel, which is where the two
/// behaviours differ; a point on the panel is the fixed point both cover.
#[test]
fn a_tap_inside_a_high_z_wrapper_still_reaches_the_dismiss_backdrop() {
    let (doc, _, _, backdrop, _) = popup_under(
        "position: relative; z-index: 201; overflow-y: auto; width: 400px; height: 300px",
    );
    assert_eq!(
        resolve(&doc.tree, doc.tree.body_id, 0.0, 0.0, 300.0, 250.0),
        Some(backdrop),
        "inside the wrapper and clear of the panel: the dismiss region must be \
         reachable, or a popup inside a Modal can never be dismissed"
    );
}

/// `body > clipper(overflow: hidden; z-index: 1) > modal(position: fixed)`,
/// with the clipper scrolled so the test is not sitting on the scroll-0 fixed
/// point. The modal is at (300, 300) 200x200 — entirely outside the clipper.
fn fixed_inside_a_clipping_context() -> (RinchDocument, RawNodeId, RawNodeId, RawNodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let clipper = doc.create_element("div");
    doc.set_attribute(
        clipper,
        "style",
        "position: relative; z-index: 1; overflow: hidden; width: 200px; height: 200px",
    );
    doc.append_child(body, clipper);

    let tall = doc.create_element("div");
    doc.set_attribute(
        tall,
        "style",
        "width: 10px; height: 800px; background-color: rgb(0, 0, 255)",
    );
    doc.append_child(clipper, tall);

    let modal = doc.create_element("div");
    doc.set_attribute(
        modal,
        "style",
        "position: fixed; left: 300px; top: 300px; width: 200px; height: 200px; \
         background-color: rgb(0, 200, 0)",
    );
    doc.append_child(clipper, modal);

    // Sorts *after* the fixed modal (z 5 against its z 0), so it is the entry
    // that proves the lifted bracket was put back: it is an absolute whose chain
    // is empty — the collecting root's own clip is never a chain link — so the
    // bracket is the only thing that can clip it.
    let after = doc.create_element("div");
    doc.set_attribute(
        after,
        "style",
        "position: absolute; left: 0; top: 100px; width: 600px; height: 20px; \
         z-index: 5; background-color: rgb(255, 0, 255)",
    );
    doc.append_child(clipper, after);

    doc.resolve_layout(800.0, 600.0);
    doc.tree.nodes[raw(clipper)].scroll_offset = (0.0, 50.0);
    (doc, raw(clipper), raw(tall), raw(modal))
}

/// A fixed box is not clipped by the stacking context that owns it — the
/// context is not its containing block.
///
/// This is #324 stage B's documented "Known gap" becoming live: the collecting
/// root's clip is a paint-time *bracket* around its whole sequence rather than
/// part of each entry's chain, and a fixed entry only ever landed at the body,
/// where its bracket is the viewport. Now that it can land under a clipper, the
/// bracket has to be lifted for it. Chromium answers the modal here; with the
/// gap open, rinch answered nothing at all.
#[test]
fn a_fixed_box_escapes_the_bounds_gate_of_the_context_that_owns_it() {
    let (doc, clipper, tall, modal) = fixed_inside_a_clipping_context();
    let after = doc.tree.get(clipper).unwrap().children[2];

    assert_eq!(
        stacking_paint_order(&doc.tree, clipper, 1.0, 0.0, 0.0)
            .iter()
            .map(|e| (e.node_id, e.clips.len()))
            .collect::<Vec<_>>(),
        vec![(tall, 0), (modal, 0), (after, 0)],
        "the clipper owns the modal, and imposes no chain link on it — its own \
         clip is the bracket paint opens, which is what has to be lifted"
    );
    assert_eq!(
        resolve(&doc.tree, doc.tree.body_id, 0.0, 0.0, 400.0, 400.0),
        Some(modal),
        "a point on the modal and far outside the clipper still resolves to it"
    );

    // Not vacuous: a point on neither answers the body, so the assertion above
    // is about this box and not about the walk answering `modal` for anything.
    assert_eq!(
        resolve(&doc.tree, doc.tree.body_id, 0.0, 0.0, 600.0, 100.0),
        Some(doc.tree.body_id),
    );
}

/// The forward reading of the same two rules: pixels.
///
/// The hit-test assertions above would all pass against a fix applied only to
/// hit testing, which would be a box drawn somewhere the taps do not go — the
/// exact drift #324 stages A and B existed to end. These are here so that
/// cannot ship.
mod painted_fixed {
    use super::*;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    const BACKDROP: [u8; 4] = [255, 0, 0, 255];
    const PANEL: [u8; 4] = [0, 0, 255, 255];
    const MODAL: [u8; 4] = [0, 200, 0, 255];
    const AFTER: [u8; 4] = [255, 0, 255, 255];

    pub(super) fn paint(doc: &mut RinchDocument) -> TinySkiaPainter {
        let mut painter = TinySkiaPainter::new(800, 600);
        let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut painter,
            1.0,
            (800.0, 600.0),
            &mut doc.font_cx,
            &mut layout_cx,
        );
        painter
    }

    fn pixel_at(painter: &TinySkiaPainter, x: u32, y: u32) -> [u8; 4] {
        let idx = ((y * painter.width() + x) * 4) as usize;
        let d = painter.pixels();
        [d[idx], d[idx + 1], d[idx + 2], d[idx + 3]]
    }

    /// `body > wrapper(z-index: 1) > menu > { backdrop(fixed, 99), panel(100) }`,
    /// with both overlays opaque so the comparison is exact — an `opacity`
    /// wrapper would blend and make the assertion about arithmetic instead of
    /// order.
    fn opaque_popup() -> RinchDocument {
        let mut doc = RinchDocument::new();
        let body = doc.body();

        let wrapper = doc.create_element("div");
        doc.set_attribute(
            wrapper,
            "style",
            "position: relative; z-index: 1; overflow: hidden; width: 400px; height: 300px",
        );
        doc.append_child(body, wrapper);

        let menu = doc.create_element("div");
        doc.set_attribute(
            menu,
            "style",
            "position: relative; width: 120px; height: 32px",
        );
        doc.append_child(wrapper, menu);

        let backdrop = doc.create_element("div");
        doc.set_attribute(
            backdrop,
            "style",
            "position: fixed; top: 0; left: 0; right: 0; bottom: 0; z-index: 99; \
             background-color: rgb(255, 0, 0)",
        );
        doc.append_child(menu, backdrop);

        let panel = doc.create_element("div");
        doc.set_attribute(
            panel,
            "style",
            "position: absolute; top: 32px; left: 0; width: 160px; height: 120px; \
             z-index: 100; background-color: rgb(0, 0, 255)",
        );
        doc.append_child(menu, panel);

        doc.resolve_layout(800.0, 600.0);
        doc
    }

    /// The panel is drawn over the backdrop, because the `99` and the `100` are
    /// in one sequence. Before #545 the backdrop escaped the wrapper to the body
    /// and was painted over the panel — the menu was invisible as well as
    /// untappable, the same pairing #324's own tests found.
    #[test]
    fn a_fixed_backdrop_is_drawn_under_the_panel_it_sits_with() {
        let painter = paint(&mut opaque_popup());
        assert_eq!(
            pixel_at(&painter, 80, 80),
            PANEL,
            "a point on the panel shows the panel"
        );
        assert_eq!(
            pixel_at(&painter, 300, 250),
            BACKDROP,
            "…and a point clear of it still shows the backdrop, so the panel is \
             not simply covering everything"
        );
    }

    /// A fixed box is painted outside the clip of the stacking context that owns
    /// it. `paint_node` opens that clip as a bracket around the whole sequence,
    /// so `paint_children_with_stacking` has to lift it for a fixed entry and put
    /// the same shape back.
    #[test]
    fn a_fixed_box_is_drawn_outside_the_clip_of_the_context_that_owns_it() {
        let (mut doc, ..) = fixed_inside_a_clipping_context();
        let painter = paint(&mut doc);

        assert_eq!(
            pixel_at(&painter, 400, 400),
            MODAL,
            "the modal is drawn at its viewport position, 100px beyond the \
             `overflow: hidden` box it lives in"
        );
        // The bracket is put back, not dropped. The `z-index: 5` box sorts
        // after the modal, is 600px wide in a 200px-wide clipper, and has an
        // empty clip chain — so the restored bracket is the only thing that can
        // cut it off. Visible inside the clipper, gone outside it.
        assert_eq!(
            pixel_at(&painter, 100, 60),
            AFTER,
            "the later entry is drawn inside the clipper"
        );
        assert_ne!(
            pixel_at(&painter, 400, 60),
            AFTER,
            "…and cut off at the clipper's edge, so the lifted bracket was \
             restored before it painted"
        );
    }

    /// A fixed box keeps the **body's** transform, not the transform of the
    /// stacking context that now owns it.
    ///
    /// This is the coupling that makes the paint half more than a clip change.
    /// `paint::compute_absolute_position_and_transform` — the shared answer to
    /// "where is this box", behind `ClickContext`, the MCP `absolute` contract
    /// and DevTools — deliberately drops every ancestor transform for a fixed
    /// box and seeds only the body's. Before #545 the two agreed for free,
    /// because a fixed box only ever painted in the body's sequence. Now that it
    /// can paint under a transformed root, `paint_children_with_stacking` has to
    /// keep handing it the body's transform or paint and every coordinate
    /// consumer disagree about the same box.
    ///
    /// **This is not what a browser does.** Chromium *contains* a fixed box in a
    /// transformed ancestor — position, clip and transform together — and
    /// answers `BODY` for the point below (measured with `elementFromPoint`).
    /// rinch models no containment at all: `out_of_flow::out_of_flow_kind`
    /// answers "the viewport" for every fixed box, so it was already wrong here
    /// before #545 and is exactly as wrong after. What this pins is that it is
    /// wrong *consistently* — the alternative, letting paint follow the
    /// transform while hit testing does not, is the drift #324 stages A and B
    /// exist to prevent. Containment is tracked separately, with #386 and #415.
    #[test]
    fn a_fixed_box_under_a_transformed_context_is_painted_where_it_is_reported() {
        let mut doc = RinchDocument::new();
        let body = doc.body();

        let moved = doc.create_element("div");
        doc.set_attribute(
            moved,
            "style",
            "position: relative; z-index: 1; transform: translate(120px, 90px); \
             width: 200px; height: 200px",
        );
        doc.append_child(body, moved);

        let modal = doc.create_element("div");
        doc.set_attribute(
            modal,
            "style",
            "position: fixed; left: 300px; top: 300px; width: 200px; height: 200px; \
             background-color: rgb(0, 200, 0)",
        );
        doc.append_child(moved, modal);

        doc.resolve_layout(800.0, 600.0);

        let (rx, ry, _) =
            rinch_dom::paint::compute_absolute_position_and_transform(&doc.tree, raw(modal), 1.0);
        assert_eq!(
            (rx, ry),
            (300.0, 300.0),
            "the shared position answer is the viewport one, transform dropped"
        );

        let painter = paint(&mut doc);
        assert_eq!(
            pixel_at(&painter, 400, 400),
            MODAL,
            "and paint puts it there too — not 120x90 further on, which is where \
             the wrapper's transform would have carried it"
        );
        assert_ne!(
            pixel_at(&painter, 520, 490),
            MODAL,
            "…which is that translated position, and must be empty"
        );
    }

    /// **Known deviation (#549), pinned so it is a decision and not a surprise:**
    /// a fixed box escapes the clip of the stacking context that owns it, but
    /// **not** the clips that context was itself hoisted past.
    ///
    /// `plain clipper > SC > fixed`. The fixed box is hoisted to the SC and its
    /// own entry chain is empty — but the *SC's* entry carries the clipper as a
    /// chain link, and paint pushes that chain around the SC's whole subtree.
    /// The lift in `paint_children_with_stacking` removes one clip, the
    /// collecting root's own bracket, and nothing above it; unwinding the rest
    /// would mean tracking every open layer up to the body, which paint does not
    /// do.
    ///
    /// So the box is drawn nowhere and tapped nowhere. **Chromium paints it**
    /// (measured), and so did rinch before #545 — a fixed box that reached the
    /// body escaped every clip on the way. Paint and hit testing agree here, so
    /// this is a consistent deviation and not the drift #324 exists to end, and
    /// nothing in-tree has the shape. It is still a regression, and the fix is
    /// architectural: tracked in #549 with #386 and #415.
    ///
    /// If this test starts failing because the box is painted, that is #549
    /// being fixed — invert it, do not delete it.
    #[test]
    fn a_fixed_box_does_not_escape_clips_above_the_context_that_owns_it() {
        let mut doc = RinchDocument::new();
        let body = doc.body();

        // Not a stacking context: `overflow` stopped creating one in stage B.
        let clipper = doc.create_element("div");
        doc.set_attribute(
            clipper,
            "style",
            "overflow: hidden; width: 200px; height: 200px",
        );
        doc.append_child(body, clipper);

        // …but this is, so it is what owns the fixed box.
        let owner = doc.create_element("div");
        doc.set_attribute(
            owner,
            "style",
            "position: relative; z-index: 1; width: 50px; height: 50px",
        );
        doc.append_child(clipper, owner);

        let modal = doc.create_element("div");
        doc.set_attribute(
            modal,
            "style",
            "position: fixed; left: 300px; top: 300px; width: 200px; height: 200px; \
             background-color: rgb(0, 200, 0)",
        );
        doc.append_child(owner, modal);

        doc.resolve_layout(800.0, 600.0);

        // The owner is hoisted to the body carrying the clipper in its chain,
        // which is the mechanism — stated as an assertion so a change in it
        // fails here rather than only in the pixels below.
        let owner_entry = body_order(&doc)
            .iter()
            .find(|e| e.node_id == raw(owner))
            .copied()
            .expect("the owner is an entry of the body's sequence");
        assert_eq!(
            owner_entry.clips.len(),
            1,
            "the owner was hoisted past the clipper, so its entry carries it"
        );
        assert_eq!(
            stacking_paint_order(&doc.tree, raw(owner), 1.0, 0.0, 0.0)
                .iter()
                .map(|e| (e.node_id, e.clips.len()))
                .collect::<Vec<_>>(),
            vec![(raw(modal), 0)],
            "…while the fixed box's own chain is empty, which is why the doc's \
             \"empty clip chain\" must not be read as \"escapes every clip\""
        );

        assert_eq!(
            painted_fixed::pixel_at(&painted_fixed::paint(&mut doc), 400, 400),
            [0, 0, 0, 0],
            "today the clipper the owner was hoisted past still cuts the box \
             away — Chromium paints it here (#549)"
        );
    }
}
