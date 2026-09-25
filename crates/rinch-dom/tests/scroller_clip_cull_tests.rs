//! Paint does not visit what a scroller's clip cuts away — and draws everything
//! the clip lets through (#910).
//!
//! The window and dirty-region culls say what nothing can be *seen* outside.
//! A scroller's rows below its viewport are on the window, often inside the
//! dirty rect, and are clipped away all the same: paint used to enter every one
//! of them, so a frame cost grew with the list, not the viewport. The clip the
//! painter has open is now a third cull (`ClipTrackingPainter` mirrors the
//! painter's own clip stack), and a row that would paint nothing is dismissed
//! by its parent's loop without a `paint_node` visit.
//!
//! The visit counts are the positive side; every other fixture here is a local
//! pixel oracle on the negative side — a region where the correct output is
//! provably one colour — and each names the mutant it kills. The fixed points
//! they step off: a row wholly inside the clip and a row far outside it agree
//! with any cull, so the discriminating rows **straddle** the cull's edge, sit
//! inside its **ink margin**, or are reached only through a **transform** or
//! the #545 **lift**.

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use rinch_dom::paint::vello_painter::VelloPainter;
use rinch_dom::perf::Counter;

const VW: f32 = 400.0;
const VH: f32 = 800.0;

fn raw(id: NodeId) -> usize {
    id.0
}

fn layout_cx() -> parley::LayoutContext<Brush> {
    parley::LayoutContext::new()
}

/// Paint `doc` in full on the software painter and return the painter and the
/// number of `paint_node` visits it cost.
fn paint(doc: &mut RinchDocument) -> (TinySkiaPainter, u64) {
    let mut p = TinySkiaPainter::new(VW as u32, VH as u32);
    let mut lcx = layout_cx();
    doc.tree.perf.reset();
    rinch_dom::paint::paint_document(&doc.tree, &mut p, 1.0, (VW, VH), &mut doc.font_cx, &mut lcx);
    let visits = doc.tree.perf.get(Counter::PaintNodesVisited);
    (p, visits)
}

/// The same paint on the GPU painter, which only records a scene: its visit
/// count, which must match the software one's (both go through
/// `paint_document`, so both are culled by the clip).
fn vello_visits(doc: &mut RinchDocument) -> u64 {
    let mut p = VelloPainter::new();
    let mut lcx = layout_cx();
    doc.tree.perf.reset();
    rinch_dom::paint::paint_document(&doc.tree, &mut p, 1.0, (VW, VH), &mut doc.font_cx, &mut lcx);
    doc.tree.perf.get(Counter::PaintNodesVisited)
}

fn pixel_at(p: &TinySkiaPainter, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * p.width() + x) * 4) as usize;
    let d = p.pixels();
    [d[i], d[i + 1], d[i + 2], d[i + 3]]
}

/// Pixels in a region that are exactly `rgb`, opaque.
fn count_rgb(p: &TinySkiaPainter, x0: u32, x1: u32, y0: u32, y1: u32, rgb: [u8; 3]) -> usize {
    let mut n = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let px = pixel_at(p, x, y);
            if px[0] == rgb[0] && px[1] == rgb[1] && px[2] == rgb[2] && px[3] == 255 {
                n += 1;
            }
        }
    }
    n
}

fn child(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", style);
    doc.append_child(parent, d);
    d
}

const ROW: &str = "height: 20px; background-color: rgb(0, 0, 255)";

/// A 200x200 scroller at the top-left of the window holding `rows` 20px rows,
/// each with a line of text (drawn by the row's own IFC, like a real list).
/// `extra` is appended to the scroller's style.
fn scroller(doc: &mut RinchDocument, rows: usize, extra: &str) -> NodeId {
    let body = doc.body();
    let s = child(
        doc,
        body,
        &format!("width: 200px; height: 200px; overflow-y: auto; {extra}"),
    );
    for i in 0..rows {
        let r = child(
            doc,
            s,
            &format!("{ROW}; font-size: 14px; line-height: 20px; color: rgb(0, 0, 255)"),
        );
        let t = doc.create_text(&format!("row {i}"));
        doc.append_child(r, t);
    }
    s
}

/// **The count this issue is about.** 200 rows, ten on screen: paint enters
/// the rows the clip (plus its ink margin) lets through, not all 200. Before
/// #910 this was 204 (html, body, scroller, and every row).
///
/// Kills: the clip test removed from `intersects_dirty_region` (every row is on
/// the window, so nothing else culls it: 204), and the parent-side
/// `paints_nothing_without_visit` call removed from the tree-order loop (each
/// row is entered to be culled on arrival: 204 again).
#[test]
fn a_scroller_paints_only_the_rows_its_clip_shows() {
    let mut doc = RinchDocument::new();
    scroller(&mut doc, 200, "");
    doc.resolve_layout(VW, VH);
    let (p, visits) = paint(&mut doc);
    // 200px clip + 64px margin below = rows 0..=13 intersect; plus the chain.
    assert!(
        visits < 25,
        "paint visited {visits} nodes for a scroller showing 10 of its 200 rows"
    );
    // Positive control: the rows on screen were drawn, the last one included.
    assert_eq!(count_rgb(&p, 150, 190, 182, 198, [0, 0, 255]), 40 * 16);
    // And nothing below the scroller: the clip still clips.
    assert_eq!(count_rgb(&p, 0, 200, 202, 400, [0, 0, 255]), 0);
}

/// The same through a stacking-context scroller, whose rows are entries of its
/// own paint sequence rather than a tree-order walk.
///
/// Kills: the `paints_nothing_without_visit` call removed from the sequence
/// loop (204).
#[test]
fn a_stacking_context_scroller_paints_only_the_rows_its_clip_shows() {
    let mut doc = RinchDocument::new();
    scroller(&mut doc, 200, "position: relative; z-index: 0");
    doc.resolve_layout(VW, VH);
    let (p, visits) = paint(&mut doc);
    assert!(
        visits < 25,
        "paint visited {visits} nodes for a stacking scroller showing 10 rows"
    );
    assert_eq!(count_rgb(&p, 150, 190, 182, 198, [0, 0, 255]), 40 * 16);
}

/// The GPU scene is built by the same walk, so it is pruned the same: equal
/// visits on both painters. The GPU path is where the prune is worth most —
/// vello flattens every path it is handed before its coarse stage culls it.
///
/// Kills: the clip tracking moved out of `paint_document` into a
/// software-only caller (the Vello scene visits all 204 again). Also killed by
/// either mutant above.
#[test]
fn the_gpu_scene_is_pruned_like_the_software_frame() {
    let mut doc = RinchDocument::new();
    scroller(&mut doc, 200, "");
    doc.resolve_layout(VW, VH);
    let (_, sw) = paint(&mut doc);
    let gpu = vello_visits(&mut doc);
    assert_eq!(
        gpu, sw,
        "the GPU scene visited {gpu}, the software frame {sw}"
    );
    assert!(gpu < 25);
}

/// **Scrolled.** The rows the clip shows are the ones the scroll brought in,
/// and the rows scrolled off the *top* are pruned like the ones below — the
/// cull is a rect on both sides, not a "past the bottom" test.
///
/// Kills: the clip test removed, and the tree-loop call removed (both leave
/// all 200 rows visited).
#[test]
fn a_scrolled_scroller_paints_the_rows_scrolled_into_view() {
    let mut doc = RinchDocument::new();
    let s = scroller(&mut doc, 200, "");
    doc.resolve_layout(VW, VH);
    doc.tree.nodes[raw(s)].scroll_offset = (0.0, 2000.0);
    doc.tree.hit_cache.invalidate();
    let (p, visits) = paint(&mut doc);
    assert!(visits < 30, "visited {visits}");
    assert_eq!(count_rgb(&p, 150, 190, 2, 198, [0, 0, 255]), 40 * 196);
}

/// **Straddling the cull's edge.** A 200px-tall row from 150 to 350 crosses
/// the edge of the clip's cull band (200 + 64 = 264): its top 50px are on
/// screen and must be drawn.
///
/// Kills: a cull demanding the box lie wholly inside the clip rather than
/// merely touch it.
#[test]
fn a_row_straddling_the_clip_draws_its_visible_part() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let s = child(
        &mut doc,
        body,
        "width: 200px; height: 200px; overflow-y: auto",
    );
    child(&mut doc, s, "height: 150px");
    child(
        &mut doc,
        s,
        "height: 200px; background-color: rgb(255, 0, 0)",
    );
    for _ in 0..50 {
        child(&mut doc, s, ROW);
    }
    doc.resolve_layout(VW, VH);
    let (p, _) = paint(&mut doc);
    assert_eq!(count_rgb(&p, 10, 190, 152, 198, [255, 0, 0]), 180 * 46);
}

/// **Inside the ink margin.** A row 30px below the scroller's clip casts a
/// shadow 50px up, into the scroller's last 20px. The row's box is outside the
/// clip; its ink is not, and the clip lets that ink through.
///
/// Kills: the clip's cull rect taken without the ink margin (the row is culled
/// and its shadow with it).
#[test]
fn a_shadow_cast_back_into_the_clip_by_a_row_past_it_is_drawn() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let s = child(
        &mut doc,
        body,
        "width: 200px; height: 200px; overflow-y: auto",
    );
    child(&mut doc, s, "height: 230px");
    child(
        &mut doc,
        s,
        "height: 20px; box-shadow: 0px -50px 0px 0px rgb(255, 0, 0)",
    );
    for _ in 0..50 {
        child(&mut doc, s, "height: 20px");
    }
    doc.resolve_layout(VW, VH);
    let (p, _) = paint(&mut doc);
    // The shadow covers 180..200 inside the clip; 250..270 is clipped away.
    assert_eq!(count_rgb(&p, 10, 190, 182, 198, [255, 0, 0]), 180 * 16);
    assert_eq!(count_rgb(&p, 0, 400, 202, 400, [255, 0, 0]), 0);
}

/// **Through a transform.** The scroller is moved 300px down by a transform, so
/// its clip is pushed under that transform. The cull must compare the rows'
/// on-screen boxes against the clip's on-screen box.
///
/// Kills: the clip's bounds taken in the shape's own space, untransformed —
/// the cull band would be 0..264 while the rows paint at 300..500, and every
/// row past 264 would be lost.
#[test]
fn a_transformed_scroller_culls_in_screen_space() {
    let mut doc = RinchDocument::new();
    scroller(&mut doc, 200, "transform: translateY(300px)");
    doc.resolve_layout(VW, VH);
    let (p, visits) = paint(&mut doc);
    assert_eq!(count_rgb(&p, 150, 190, 302, 498, [0, 0, 255]), 40 * 196);
    assert!(visits < 25, "visited {visits}");
}

/// **The #545 lift.** A `position: fixed` box inside a stacking-context
/// scroller is hoisted to the scroller's own sequence and painted with the
/// scroller's bracket *lifted* — popped, and pushed back after. The fixed box
/// sits at y=400, far outside the scroller's clip and on the window. The cull
/// must drop the clip for the lift exactly as the painter does.
///
/// Kills: the tracking wrapper's `pop_layer` not popping the cull (the fixed
/// box is judged against the scroller's clip and never drawn).
#[test]
fn a_fixed_box_lifted_out_of_a_scrollers_bracket_is_drawn() {
    let mut doc = RinchDocument::new();
    let s = scroller(&mut doc, 50, "position: relative; z-index: 0");
    let row = doc.get_children(s)[30];
    child(
        &mut doc,
        row,
        "position: fixed; left: 250px; top: 400px; width: 50px; height: 50px; \
         background-color: rgb(0, 255, 0)",
    );
    doc.resolve_layout(VW, VH);
    let (p, _) = paint(&mut doc);
    assert_eq!(count_rgb(&p, 252, 298, 402, 448, [0, 255, 0]), 46 * 46);
}

/// **What the prune does not reach.** A row far below the clip holds a
/// `position: relative` child offset back up into view. The child is an entry
/// of the body's sequence, painted there under the scroller's clip chain — the
/// row's pruning is not its business.
///
/// This guards the shape of the prune (it dismisses only what the tree-order
/// walk would have drawn) rather than a particular mutant; it fails if the
/// prune ever starts taking hoisted content with it.
#[test]
fn a_hoisted_child_of_a_pruned_row_is_still_drawn() {
    let mut doc = RinchDocument::new();
    let s = scroller(&mut doc, 50, "");
    let row = doc.get_children(s)[40]; // at y = 800
    child(
        &mut doc,
        row,
        "position: relative; top: -700px; height: 20px; background-color: rgb(0, 255, 0)",
    );
    doc.resolve_layout(VW, VH);
    let (p, _) = paint(&mut doc);
    // The row is at 800..820 and its child is laid out below the row's text
    // line (820..840), moved up 700: 120..140.
    assert_eq!(count_rgb(&p, 150, 190, 122, 138, [0, 255, 0]), 40 * 16);
}

/// **Nested clips intersect.** An inner 150x400 scroller sits at y=150 inside
/// an outer 200x200 one, so the outer clip cuts it off at 200. Its rows past
/// the *outer* clip are pruned too: the cull is the intersection of every clip
/// open, not the innermost one.
///
/// Kills: a clip pushed onto the cull stack without intersecting the one below
/// (the inner clip alone lets the inner rows down to 614 through: 28 visits).
#[test]
fn a_scroller_inside_a_scroller_is_culled_by_both_clips() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let outer = child(
        &mut doc,
        body,
        "width: 200px; height: 200px; overflow-y: auto",
    );
    child(&mut doc, outer, "height: 150px");
    let inner = child(
        &mut doc,
        outer,
        "width: 150px; height: 400px; overflow-y: auto",
    );
    for _ in 0..100 {
        child(&mut doc, inner, ROW);
    }
    doc.resolve_layout(VW, VH);
    let (p, visits) = paint(&mut doc);
    assert_eq!(count_rgb(&p, 10, 140, 152, 198, [0, 0, 255]), 130 * 46);
    assert_eq!(visits, VISITS_NESTED, "paint visited {visits} nodes");
}
const VISITS_NESTED: u64 = 10;

/// **A layer is not a clip, and does not forget one.** A translucent block
/// (`opacity: 0.5`, so painted through a layer) holding 100 rows sits in a
/// 200px scroller. Inside the layer the scroller's clip still culls: only the
/// rows it lets through are visited.
///
/// Kills: `push_layer` resetting the cull instead of carrying the one below
/// (every row inside the layer is visited: 47).
#[test]
fn rows_inside_a_translucent_block_in_a_scroller_are_culled() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let s = child(
        &mut doc,
        body,
        "width: 200px; height: 200px; overflow-y: auto",
    );
    let layer = child(&mut doc, s, "opacity: 0.5");
    for _ in 0..100 {
        child(&mut doc, layer, ROW);
    }
    doc.resolve_layout(VW, VH);
    let (p, visits) = paint(&mut doc);
    // Positive control: the visible rows were drawn, at half strength.
    let px = pixel_at(&p, 100, 190);
    assert!(px[2] > 100 && px[2] < 255 && px[0] < 10, "{px:?}");
    assert_eq!(visits, VISITS_LAYER, "paint visited {visits} nodes");
}
const VISITS_LAYER: u64 = 17;
