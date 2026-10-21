//! A scroller's positioned rows that are scrolled out of view are not visited
//! by paint, and the frame is the same pixel for pixel (#1719).
//!
//! #910 made the clips the painter has open a cull, and a row that would paint
//! nothing is dismissed by its parent's loop without a `paint_node` visit
//! (`paints_nothing_without_visit`). That helper answered `false` for every
//! stacking context, so two list shapes were walked whole on every frame:
//!
//! - **(a)** rows `position: relative` (`z-index: auto`, hoisted to the outer
//!   sequence with the scroller in their clip chain) each holding positioned
//!   `z-index: 1` children, which are stacking contexts and entries of the
//!   same sequence;
//! - **(b)** rows that are stacking contexts themselves
//!   (`position: relative; z-index: 0`), in a scroller that is one too.
//!
//! The counts are pinned exactly in `perf_regression_scenarios.rs`
//! (`positioned_rows_*`); this file holds the pixel half. Its oracle is a
//! second document in which every row far from the scroller's viewport is
//! replaced by an empty spacer of the same height: whatever the cull
//! dismisses, the two frames must agree, at every scroll offset tried and on
//! a partial repaint as well as a full one.

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use peniko::kurbo::{Affine, Rect};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::{PaintShape, Painter};
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use rinch_dom::paint::vello_painter::VelloPainter;
use rinch_dom::perf::Counter;

const VW: f32 = 400.0;
const VH: f32 = 600.0;
const ROWS: usize = 500;
const ROW_H: f32 = 20.0;
const VIEW_H: f32 = 400.0;

#[derive(Clone, Copy, Debug)]
enum Shape {
    /// (a): `position: relative` rows, three `z-index: 1` children each.
    HoistedRowsWithLayeredChildren,
    /// (b): stacking-context rows in a stacking-context scroller.
    StackingRows,
    /// (b) in a scroller that is no stacking context: the rows are entries of
    /// the body's sequence, with the scroller in their clip chain.
    StackingRowsInPlainScroller,
}

fn child(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", style);
    doc.append_child(parent, d);
    d
}

fn colour(i: usize) -> String {
    format!(
        "rgb({}, {}, {})",
        (i * 37) % 256,
        (i * 91) % 256,
        255 - (i * 13) % 256
    )
}

/// A 200x400 scroller at (20, 20) holding 500 rows of `shape`, scrolled to
/// `scroll`. With `keep: Some((top, bottom))`, a row lying wholly outside
/// `top..bottom` (content coordinates) is an empty spacer instead: the oracle.
///
/// Each row casts a shadow 30px **up**, so the row just below the viewport
/// paints into its bottom edge: a cull that dropped a row on its box rather
/// than its ink would lose that strip.
fn list(shape: Shape, scroll: f32, keep: Option<(f32, f32)>) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let scroller_css = match shape {
        Shape::StackingRows => "position: relative; z-index: 0;",
        _ => "",
    };
    let s = child(
        &mut doc,
        body,
        &format!(
            "position: absolute; left: 20px; top: 20px; width: 200px; height: {VIEW_H}px; \
             overflow-y: auto; background-color: rgb(250, 250, 250); {scroller_css}"
        ),
    );
    for i in 0..ROWS {
        let top = i as f32 * ROW_H;
        if let Some((a, b)) = keep
            && (top + ROW_H <= a || top >= b)
        {
            child(&mut doc, s, &format!("height: {ROW_H}px"));
            continue;
        }
        let row_css = match shape {
            Shape::HoistedRowsWithLayeredChildren => "position: relative;",
            _ => "position: relative; z-index: 0;",
        };
        let r = child(
            &mut doc,
            s,
            &format!(
                "{row_css} height: {ROW_H}px; background-color: {}; \
                 box-shadow: 0 -30px 0 0 {}",
                colour(i),
                colour(i + 1000)
            ),
        );
        let kids = match shape {
            Shape::HoistedRowsWithLayeredChildren => 3,
            _ => 1,
        };
        for k in 0..kids {
            let kid_css = match shape {
                Shape::HoistedRowsWithLayeredChildren => "position: relative; z-index: 1;",
                _ => "",
            };
            child(
                &mut doc,
                r,
                &format!(
                    "{kid_css} margin-left: {}px; width: 30px; height: 5px; background-color: {}",
                    10 + 40 * k,
                    colour(i * 3 + k + 2000)
                ),
            );
        }
    }
    doc.resolve_layout(VW, VH);
    doc.tree.nodes[s.0].scroll_offset = (0.0, scroll as f64);
    doc.tree.hit_cache.invalidate();
    (doc, s)
}

/// Paint `doc` (in full, or over `damage` only) and return the painter and the
/// `paint_node` visits it cost.
fn paint(doc: &mut RinchDocument, damage: Option<Rect>) -> (TinySkiaPainter, u64) {
    let mut p = TinySkiaPainter::new(VW as u32, VH as u32);
    let mut lcx = parley::LayoutContext::<Brush>::new();
    doc.tree.perf.reset();
    if let Some(d) = damage {
        rinch_dom::paint::set_dirty_rects(Some(&[d]));
        p.push_clip(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            &PaintShape::Rect(d),
        );
    }
    rinch_dom::paint::paint_document(&doc.tree, &mut p, 1.0, (VW, VH), &mut doc.font_cx, &mut lcx);
    if damage.is_some() {
        p.pop_layer();
        rinch_dom::paint::set_dirty_region(None);
    }
    (p, doc.tree.perf.get(Counter::PaintNodesVisited))
}

fn vello_visits(doc: &mut RinchDocument) -> u64 {
    let mut p = VelloPainter::new();
    let mut lcx = parley::LayoutContext::<Brush>::new();
    doc.tree.perf.reset();
    rinch_dom::paint::paint_document(&doc.tree, &mut p, 1.0, (VW, VH), &mut doc.font_cx, &mut lcx);
    doc.tree.perf.get(Counter::PaintNodesVisited)
}

/// Index and value of the first pixel that differs, inside `region`.
fn first_difference(
    a: &TinySkiaPainter,
    b: &TinySkiaPainter,
    region: Rect,
) -> Option<(u32, u32, [u8; 4], [u8; 4])> {
    let (pa, pb) = (a.pixels(), b.pixels());
    for y in region.y0 as u32..region.y1 as u32 {
        for x in region.x0 as u32..region.x1 as u32 {
            let i = ((y * a.width() + x) * 4) as usize;
            if pa[i..i + 4] != pb[i..i + 4] {
                return Some((
                    x,
                    y,
                    [pa[i], pa[i + 1], pa[i + 2], pa[i + 3]],
                    [pb[i], pb[i + 1], pb[i + 2], pb[i + 3]],
                ));
            }
        }
    }
    None
}

const SHAPES: [Shape; 3] = [
    Shape::HoistedRowsWithLayeredChildren,
    Shape::StackingRows,
    Shape::StackingRowsInPlainScroller,
];

/// Scroll offsets off every fixed point: the top, a whole row, a fraction of
/// one, and the end of the range.
const SCROLLS: [f32; 4] = [0.0, 2000.0, 3337.5, ROWS as f32 * ROW_H - VIEW_H];

/// **The frame is unchanged.** Against the oracle that keeps only the rows
/// within 100px of the viewport, every pixel of the window agrees, for each
/// shape at each scroll offset — and the culled frame really did skip the rest
/// (its visits are bounded by the rows on screen, not the list).
///
/// Kills: the stacking-context branch answering `true` on the box alone
/// without asking `subtree_is_entirely_outside` (the row below the viewport
/// whose shadow reaches up into it has a box outside the clip and is dropped:
/// its strip at the viewport's bottom differs); and the branch removed (visits
/// over 500 — the bound below).
#[test]
fn culled_positioned_rows_paint_the_same_frame() {
    for shape in SHAPES {
        for scroll in SCROLLS {
            let (mut doc, _) = list(shape, scroll, None);
            let (mut oracle, _) = list(
                shape,
                scroll,
                Some((scroll - 100.0, scroll + VIEW_H + 100.0)),
            );
            let (got, visits) = paint(&mut doc, None);
            let (want, _) = paint(&mut oracle, None);
            let window = Rect::new(0.0, 0.0, VW as f64, VH as f64);
            assert_eq!(
                first_difference(&got, &want, window),
                None,
                "{shape:?} at scroll {scroll}: (x, y, culled, oracle)"
            );
            // 400px of 20px rows, plus the ink-margin band each side, times
            // the boxes per row.
            let per_row = match shape {
                Shape::HoistedRowsWithLayeredChildren => 4,
                _ => 2,
            };
            assert!(
                visits <= 4 + 28 * per_row,
                "{shape:?} at scroll {scroll}: {visits} visits"
            );
            // Positive control: the oracle frame has rows in it.
            assert_ne!(
                first_difference(
                    &want,
                    &TinySkiaPainter::new(VW as u32, VH as u32),
                    Rect::new(20.0, 20.0, 220.0, 420.0)
                ),
                None
            );
        }
    }
}

/// **A partial repaint** culls against the damage as well as the clip: a
/// 200x40 damage over the scroller's bottom edge repaints the rows there —
/// shadow strip included — exactly as the full frame does.
#[test]
fn a_partial_repaint_over_culled_rows_matches_the_full_frame() {
    let damage = Rect::new(20.0, 390.0, 220.0, 430.0);
    for shape in SHAPES {
        for scroll in SCROLLS {
            let (mut doc, _) = list(shape, scroll, None);
            let (full, _) = paint(&mut doc, None);
            let (part, _) = paint(&mut doc, Some(damage));
            assert_eq!(
                first_difference(&part, &full, damage),
                None,
                "{shape:?} at scroll {scroll}: (x, y, partial, full)"
            );
        }
    }
}

/// The GPU scene is built by the same walk: the same visits as the software
/// frame, for each shape.
#[test]
fn the_gpu_scene_culls_positioned_rows_like_the_software_frame() {
    for shape in SHAPES {
        let (mut doc, _) = list(shape, 2000.0, None);
        let (_, sw) = paint(&mut doc, None);
        let gpu = vello_visits(&mut doc);
        assert_eq!(gpu, sw, "{shape:?}");
        assert!(gpu < 120, "{shape:?}: {gpu}");
    }
}

/// **What escapes the row is still drawn.** A stacking-context row far below
/// the viewport holds a `position: fixed` box placed inside the scroller's
/// viewport: the row's box is outside every cull and its subtree is not, so
/// the row must be entered (`subtree_is_entirely_outside` answers "escapes").
///
/// Kills: the stacking-context branch answering on the row's box alone.
#[test]
fn a_fixed_box_in_a_culled_stacking_row_is_drawn() {
    for shape in [Shape::StackingRows, Shape::StackingRowsInPlainScroller] {
        let (mut doc, s) = list(shape, 0.0, None);
        let row = doc.get_children(s)[300];
        child(
            &mut doc,
            row,
            "position: fixed; left: 60px; top: 100px; width: 40px; height: 40px; \
             background-color: rgb(0, 255, 0)",
        );
        doc.resolve_layout(VW, VH);
        let (p, _) = paint(&mut doc, None);
        let px = p.pixels();
        let mut green = 0;
        for y in 100..140u32 {
            for x in 60..100u32 {
                let i = ((y * p.width() + x) * 4) as usize;
                if px[i..i + 4] == [0, 255, 0, 255] {
                    green += 1;
                }
            }
        }
        assert_eq!(green, 40 * 40, "{shape:?}");
    }
}

// ── What the cull looks through (#1719) ─────────────────────────────────────

/// A 200x400 scroller at (20, 20) of 100 plain 20px rows; `far` is row 60,
/// well below the viewport and its ink margin.
fn plain_list() -> (RinchDocument, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let s = child(
        &mut doc,
        body,
        "position: absolute; left: 20px; top: 20px; width: 200px; height: 400px; overflow-y: auto",
    );
    for i in 0..100 {
        child(
            &mut doc,
            s,
            &format!(
                "display: flex; height: 20px; background-color: {}",
                colour(i)
            ),
        );
    }
    let far = doc.get_children(s)[60];
    (doc, s, far)
}

/// Opaque `rgb(0, 255, 0)` pixels in the 40x40 square at (60, 100).
fn green_square(doc: &mut RinchDocument) -> usize {
    doc.resolve_layout(VW, VH);
    let (p, _) = paint(doc, None);
    let px = p.pixels();
    let mut n = 0;
    for y in 100..140u32 {
        for x in 60..100u32 {
            let i = ((y * p.width() + x) * 4) as usize;
            if px[i..i + 4] == [0, 255, 0, 255] {
                n += 1;
            }
        }
    }
    n
}

/// The square drawn by a box `margin-top: -1120px` under `parent` in row 60
/// (row 60's top is at 1200 in the scroller's content, 1220 on screen): it
/// lands at y = 100 on screen, x = 60. Every box between is a flex container,
/// so the negative margin collapses with nothing.
fn reaching_square(doc: &mut RinchDocument, parent: NodeId) {
    child(
        doc,
        parent,
        "margin-top: -1120px; margin-left: 40px; width: 40px; height: 40px; \
         background-color: rgb(0, 255, 0)",
    );
}

/// **A `display: contents` wrapper is looked through, not trusted.** Row 60
/// holds a wrapper whose child reaches back up into the viewport: the cull
/// asks the child, and the child is drawn.
///
/// Kills: a contents wrapper answering "paints nothing" without asking its
/// children (the square is not drawn: 0 of 1600).
#[test]
fn content_reaching_back_through_a_contents_wrapper_is_drawn() {
    let (mut doc, _, far) = plain_list();
    let wrapper = child(&mut doc, far, "display: contents");
    reaching_square(&mut doc, wrapper);
    assert_eq!(green_square(&mut doc), 1600);
}

/// **A box degenerate in one axis is entered**, as `paint_node` enters it
/// (#142): row 60 holds a `height: 0` box whose child reaches back up.
///
/// Kills: every zero-area box answering "paints nothing" (0 of 1600).
#[test]
fn content_reaching_back_from_a_zero_height_box_is_drawn() {
    let (mut doc, _, far) = plain_list();
    let flat = child(&mut doc, far, "display: flex; height: 0; width: 200px");
    reaching_square(&mut doc, flat);
    assert_eq!(green_square(&mut doc), 1600);
}

/// **Past the budget, the cull visits.** Row 60 holds a chain of 140 nested
/// boxes (more than the 128 a question may look at), the last of which
/// reaches back up: the question gives up, paint enters, and the square is
/// drawn.
///
/// Kills: a spent budget answering "paints nothing" (0 of 1600).
#[test]
fn content_reaching_back_past_the_budget_is_drawn() {
    let (mut doc, _, far) = plain_list();
    let mut at = far;
    for _ in 0..140 {
        at = child(&mut doc, at, "display: flex");
    }
    reaching_square(&mut doc, at);
    assert_eq!(green_square(&mut doc), 1600);
}

/// A tree of open folders like rinch's `Tree`: `li > div.content > span.icon >
/// svg > rect`, an empty `0x0` box, a `display: contents` wrapper around the
/// label, and a
/// nested list under every folder. `branch` children per folder, `depth`
/// levels. With `keep`, a row outside `keep` (content coordinates) keeps its
/// box and loses its content: the oracle.
fn open_folders(scroll: f32, keep: Option<(f32, f32)>) -> (RinchDocument, usize) {
    fn level(
        doc: &mut RinchDocument,
        ul: NodeId,
        depth: usize,
        prefix: usize,
        keep: Option<(f32, f32)>,
        row: &mut usize,
    ) {
        for b in 0..3 {
            let li = child(doc, ul, "");
            let top = *row as f32 * ROW_H;
            *row += 1;
            let content = child(
                doc,
                li,
                &format!(
                    "display: flex; height: {ROW_H}px; padding-left: {}px; \
                     box-shadow: 0 -24px 0 0 {}",
                    depth * 12,
                    colour(prefix * 3 + b + 3000)
                ),
            );
            let shown = keep.is_none_or(|(a, z)| top + ROW_H > a && top < z);
            if shown {
                let icon = doc.create_element("span");
                doc.set_attribute(icon, "style", "width: 16px; height: 16px");
                doc.append_child(content, icon);
                let svg = doc.create_element("svg");
                doc.set_attribute(svg, "viewBox", "0 0 16 16");
                doc.set_attribute(svg, "style", "width: 16px; height: 16px");
                doc.append_child(icon, svg);
                let path = doc.create_element("rect");
                doc.set_attribute(path, "x", "2");
                doc.set_attribute(path, "y", "2");
                doc.set_attribute(path, "width", "12");
                doc.set_attribute(path, "height", "12");
                doc.set_attribute(path, "fill", &colour(prefix * 3 + b));
                doc.append_child(svg, path);
                // An empty `0x0` box, as an icon's `<path>` or an empty text
                // node lays out: it paints nothing and must not make the row
                // around it look as if it might.
                child(doc, content, "width: 0; height: 0; flex-shrink: 0");
                let wrap = child(doc, content, "display: contents");
                child(
                    doc,
                    wrap,
                    &format!(
                        "width: 80px; height: 12px; background-color: {}",
                        colour(prefix * 7 + b)
                    ),
                );
            }
            if depth < 4 {
                let sub = child(doc, li, "");
                level(doc, sub, depth + 1, prefix * 3 + b, keep, row);
            }
        }
    }
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let s = child(
        &mut doc,
        body,
        &format!(
            "position: absolute; left: 20px; top: 20px; width: 200px; height: {VIEW_H}px; \
             overflow-y: auto; background-color: rgb(250, 250, 250)"
        ),
    );
    let ul = child(&mut doc, s, "");
    let mut rows = 0;
    level(&mut doc, ul, 0, 0, keep, &mut rows);
    doc.resolve_layout(VW, VH);
    doc.tree.nodes[s.0].scroll_offset = (0.0, scroll as f64);
    doc.tree.hit_cache.invalidate();
    (doc, rows)
}

/// **Open folders paint the same frame, and only the rows on screen.** Each
/// folder row casts a shadow 24px up, so the row just below the viewport
/// paints into it. Against the oracle that empties every row more than 100px
/// from the viewport, every pixel agrees at each scroll offset; and the
/// culled frame visits a bounded number of nodes, not the 363 rows' 2000.
///
/// Kills (with the three fixtures above): the old depth cap (every row deeper
/// than three levels, and every open folder, visited: over 900), a contents
/// wrapper or a `0x0` box answering "visit me" (the same).
#[test]
fn open_folders_paint_the_same_frame_and_only_the_rows_on_screen() {
    for scroll in [0.0, 1500.0, 3337.5, 6860.0] {
        let (mut doc, rows) = open_folders(scroll, None);
        assert_eq!(rows, 363);
        let (mut oracle, _) = open_folders(scroll, Some((scroll - 100.0, scroll + VIEW_H + 100.0)));
        let (got, visits) = paint(&mut doc, None);
        let (want, _) = paint(&mut oracle, None);
        assert_eq!(
            first_difference(&got, &want, Rect::new(0.0, 0.0, VW as f64, VH as f64)),
            None,
            "scroll {scroll}: (x, y, culled, oracle)"
        );
        assert!(visits < 260, "scroll {scroll}: {visits} visits");
    }
}
