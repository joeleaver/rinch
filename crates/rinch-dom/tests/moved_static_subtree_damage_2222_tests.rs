//! Partial repaints of a static box that a margin change moved (#2222).
//!
//! A box moved by layout carries its whole painted subtree with it, but only
//! the box itself is paint-dirty: its children did not move against it. What
//! the subtree paints past the box — an overflowing child, an absolutely
//! positioned descendant it contains, a child's shadow — has to be in the
//! damage where it was and where it is, or the old pixels stay on screen.
//! Positioned boxes have been walked for that since #880; a static one was
//! not, so its old overflow was left behind.
//!
//! Each case paints a frame, applies a change, repaints only the damage the
//! change names, and compares that with a full paint of the new state on a
//! fresh document, at scale 1 and 2.

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use peniko::kurbo::Rect;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const W: f32 = 400.0;
const H: f32 = 300.0;

fn document(page: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_inner_html(body, page);
    doc.resolve_layout(W, H);
    doc
}

fn by_id(doc: &RinchDocument, id: &str) -> rinch_core::dom::NodeId {
    doc.query_selector_all(&format!("#{id}"))[0]
}

fn paint(doc: &mut RinchDocument, p: &mut TinySkiaPainter, scale: f64) {
    let mut lcx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(&doc.tree, p, scale, (W, H), &mut doc.font_cx, &mut lcx);
}

/// Paint `page`, apply `change`, repaint only the damage, and compare with a
/// fresh full paint after the same change.
#[track_caller]
fn partial_matches_full(page: &str, scale: f64, change: &dyn Fn(&mut RinchDocument)) {
    partials_match_full(page, scale, &[change]);
}

/// [`partial_matches_full`] for a run of changes, each followed by a partial
/// repaint of its damage alone, compared once all have been applied.
#[track_caller]
fn partials_match_full(page: &str, scale: f64, changes: &[&dyn Fn(&mut RinchDocument)]) {
    let mut doc = document(page);
    let (pw, ph) = ((W as f64 * scale) as u32, (H as f64 * scale) as u32);
    let mut p = TinySkiaPainter::new(pw, ph);
    p.fill_white();
    paint(&mut doc, &mut p, scale);
    let before = p.pixels().to_vec();
    let mut rects: Vec<Rect> = Vec::new();
    for change in changes {
        doc.tree.consume_paint_dirty();
        change(&mut doc);
        doc.resolve_layout(W, H);
        let damage = rinch_dom::paint::compute_damage(&doc.tree, scale, pw as f64, ph as f64);
        assert!(!damage.is_full(), "the change damaged the whole frame");
        rects = damage.rects().to_vec();
        assert!(!rects.is_empty(), "the change named no damage");
        for r in &rects {
            let r = r
                .intersect(Rect::new(0.0, 0.0, pw as f64, ph as f64))
                .expand();
            p.clear_rect_white(
                r.x0 as u32,
                r.y0 as u32,
                r.width() as u32,
                r.height() as u32,
            );
        }
        p.push_damage_clip(&damage);
        rinch_dom::paint::set_dirty_rects(Some(&rects));
        paint(&mut doc, &mut p, scale);
        rinch_dom::paint::set_dirty_region(None);
        // Closed before the next frame's, as the shell's frame does.
        rinch_dom::paint::painter::Painter::pop_layer(&mut p);
    }
    let partial = p.pixels().to_vec();
    let mut fresh = document(page);
    for change in changes {
        change(&mut fresh);
    }
    fresh.resolve_layout(W, H);
    let mut full = TinySkiaPainter::new(pw, ph);
    full.fill_white();
    paint(&mut fresh, &mut full, scale);
    assert_ne!(full.pixels(), &before[..], "the change changed nothing");
    let bad: Vec<(usize, usize)> = partial
        .chunks(4)
        .zip(full.pixels().chunks(4))
        .enumerate()
        .filter(|(_, (a, b))| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 1))
        .map(|(i, _)| (i % pw as usize, i / pw as usize))
        .collect();
    assert!(
        bad.is_empty(),
        "{} pixels of the partial repaint are not the new state, first {:?} (last damage {rects:?})",
        bad.len(),
        &bad[..bad.len().min(5)]
    );
}

fn each_scale(page: &str, change: impl Fn(&mut RinchDocument)) {
    for scale in [1.0, 2.0] {
        partial_matches_full(page, scale, &change);
    }
}

/// Set `#id`'s `style` to `style`.
fn restyle(id: &'static str, style: &'static str) -> impl Fn(&mut RinchDocument) {
    move |doc: &mut RinchDocument| {
        let n = by_id(doc, id);
        doc.set_attribute(n, "style", style);
    }
}

/// A static box whose child overflows it below and to the right; the box
/// moves by its own margin.
const OVERFLOW: &str = r#"<div style="margin: 0; padding: 0"><div id="b" style="margin: 13px 0 0 21px; width: 60px; height: 30px; background: rgb(0, 0, 255)"><div style="width: 130px; height: 90px; background: rgb(255, 0, 0)"></div></div></div>"#;

#[test]
fn an_overflowing_child_moves_with_its_static_parent() {
    each_scale(
        OVERFLOW,
        restyle(
            "b",
            "margin: 47px 0 0 69px; width: 60px; height: 30px; background: rgb(0, 0, 255)",
        ),
    );
}

/// A static box moved by its **previous sibling's** margin: the sibling is
/// the restyled node, the box only reflowed.
const SHIFTED: &str = r#"<div id="a" style="margin-top: 7px; height: 20px; background: rgb(0, 128, 0)"></div><div style="margin-left: 31px; width: 60px; height: 30px; background: rgb(0, 0, 255)"><div style="width: 150px; height: 70px; background: rgb(255, 0, 0)"></div></div>"#;

#[test]
fn an_overflowing_child_moves_with_a_reflow_shifted_parent() {
    each_scale(
        SHIFTED,
        restyle(
            "a",
            "margin-top: 53px; height: 20px; background: rgb(0, 128, 0)",
        ),
    );
}

/// A static box holding a `relative` wrapper with an absolute badge past its
/// corner: the wrapper is the badge's containing block and does not move
/// against the static box, so neither is dirty.
const BADGE: &str = r#"<div id="b" style="margin: 11px 0 0 17px; width: 80px; height: 40px; background: rgb(0, 0, 255)"><div style="position: relative; width: 80px; height: 40px"><div style="position: absolute; left: 70px; top: 30px; width: 50px; height: 45px; background: rgb(255, 0, 0)"></div></div></div>"#;

#[test]
fn a_contained_absolute_descendant_moves_with_its_static_ancestor() {
    each_scale(
        BADGE,
        restyle(
            "b",
            "margin: 59px 0 0 83px; width: 80px; height: 40px; background: rgb(0, 0, 255)",
        ),
    );
}

/// A static box whose in-flow child casts a shadow past it (#880's probe J2).
const SHADOW: &str = r#"<div id="b" style="margin: 9px 0 0 23px; width: 90px; height: 40px"><div style="width: 90px; height: 40px; background: rgb(0, 0, 255); box-shadow: 17px 21px 0 rgb(255, 0, 0)"></div></div>"#;

#[test]
fn a_childs_shadow_moves_with_its_static_parent() {
    each_scale(
        SHADOW,
        restyle("b", "margin: 51px 0 0 67px; width: 90px; height: 40px"),
    );
}

/// A static box whose text child's long unbreakable line overflows it.
const TEXT: &str = r#"<div id="b" style="margin: 5px 0 0 15px; width: 40px; height: 20px; font: 16px/20px sans-serif; white-space: nowrap; color: rgb(255, 0, 0)"><div>overflowing words</div></div>"#;

#[test]
fn overflowing_text_moves_with_its_static_parent() {
    each_scale(
        TEXT,
        restyle(
            "b",
            "margin: 43px 0 0 61px; width: 40px; height: 20px; font: 16px/20px sans-serif; white-space: nowrap; color: rgb(255, 0, 0)",
        ),
    );
}

/// A static scroller of more rows than the reach walk's budget, moved by its
/// margin: it clips its rows on both axes, so its own rect holds them and the
/// frame stays partial (walking it would give up and repaint the window).
#[test]
fn a_moved_clipping_scroller_of_many_rows_stays_a_partial_frame() {
    let rows: String = (0..600)
        .map(|i| {
            format!(
                r#"<div style="height: 10px; background: rgb({}, 0, 0)"></div>"#,
                i % 256
            )
        })
        .collect();
    let page = format!(
        r#"<div id="b" style="margin: 7px 0 0 9px; width: 120px; height: 80px; overflow: hidden">{rows}</div>"#
    );
    let page: &'static str = Box::leak(page.into_boxed_str());
    each_scale(
        page,
        restyle(
            "b",
            "margin: 29px 0 0 41px; width: 120px; height: 80px; overflow: hidden",
        ),
    );
}

/// The box starts clipping in the frame it moves: its child's old overflow,
/// painted unclipped, is cleared (the clip skip asks how it was painted too).
#[test]
fn a_box_that_starts_clipping_as_it_moves_clears_its_old_overflow() {
    each_scale(
        OVERFLOW,
        restyle(
            "b",
            "margin: 47px 0 0 69px; width: 60px; height: 30px; background: rgb(0, 0, 255); overflow: hidden",
        ),
    );
}

/// The box starts clipping where it is: its child's overflow is cleared.
#[test]
fn a_box_that_starts_clipping_in_place_clears_its_old_overflow() {
    each_scale(
        OVERFLOW,
        restyle(
            "b",
            "margin: 13px 0 0 21px; width: 60px; height: 30px; background: rgb(0, 0, 255); overflow: hidden",
        ),
    );
}

/// The box stops clipping as it moves: its child's overflow is drawn.
#[test]
fn a_box_that_stops_clipping_as_it_moves_draws_its_overflow() {
    const CLIPPED: &str = r#"<div style="margin: 0; padding: 0"><div id="b" style="margin: 13px 0 0 21px; width: 60px; height: 30px; background: rgb(0, 0, 255); overflow: hidden"><div style="width: 130px; height: 90px; background: rgb(255, 0, 0)"></div></div></div>"#;
    each_scale(
        CLIPPED,
        restyle(
            "b",
            "margin: 47px 0 0 69px; width: 60px; height: 30px; background: rgb(0, 0, 255)",
        ),
    );
}

/// A static box holding a `position: fixed` child, with an overflowing child
/// beside it: the fixed one does not move with the box, so the move stays a
/// partial frame (the walk used to give up on it and repaint the window), and
/// the overflow is still repainted.
#[test]
fn a_fixed_child_does_not_make_a_moved_static_box_repaint_the_window() {
    const PAGE: &str = r#"<div id="b" style="margin: 13px 0 0 21px; width: 60px; height: 30px; background: rgb(0, 0, 255)"><div style="position: fixed; left: 250px; top: 200px; width: 40px; height: 30px; background: rgb(0, 128, 0)"></div><div style="width: 130px; height: 90px; background: rgb(255, 0, 0)"></div></div>"#;
    each_scale(
        PAGE,
        restyle(
            "b",
            "margin: 47px 0 0 69px; width: 60px; height: 30px; background: rgb(0, 0, 255)",
        ),
    );
}

/// A static box that clips one axis only (`overflow-x: clip`), with its child
/// overflowing on the other: it is walked, and the vertical overflow moves.
#[test]
fn a_box_clipping_one_axis_carries_its_overflow_on_the_other() {
    const PAGE: &str = r#"<div id="b" style="margin: 13px 0 0 21px; width: 60px; height: 30px; background: rgb(0, 0, 255); overflow-x: clip"><div style="width: 130px; height: 90px; background: rgb(255, 0, 0)"></div></div>"#;
    each_scale(
        PAGE,
        restyle(
            "b",
            "margin: 47px 0 0 69px; width: 60px; height: 30px; background: rgb(0, 0, 255); overflow-x: clip",
        ),
    );
}

/// The same box moved twice, its subtree unchanged between: the second move
/// is damaged from the reach the first paint recorded, with no walk.
#[test]
fn a_second_move_is_damaged_from_the_recorded_reach() {
    for scale in [1.0, 2.0] {
        partial_matches_full_twice(
            OVERFLOW,
            scale,
            &restyle(
                "b",
                "margin: 31px 0 0 45px; width: 60px; height: 30px; background: rgb(0, 0, 255)",
            ),
            &restyle(
                "b",
                "margin: 59px 0 0 7px; width: 60px; height: 30px; background: rgb(0, 0, 255)",
            ),
        );
    }
}

/// A move with a change under the box in the same frame: the recorded reach
/// describes the old subtree only, so the new one is walked.
#[test]
fn a_move_with_a_change_under_the_box_walks_the_new_subtree() {
    const PAGE: &str = r#"<div id="b" style="margin: 13px 0 0 21px; width: 60px; height: 30px; background: rgb(0, 0, 255)"><div id="c" style="width: 50px; height: 20px; background: rgb(255, 0, 0)"></div></div>"#;
    for scale in [1.0, 2.0] {
        partial_matches_full_twice(
            PAGE,
            scale,
            &restyle(
                "b",
                "margin: 31px 0 0 45px; width: 60px; height: 30px; background: rgb(0, 0, 255)",
            ),
            &|doc: &mut RinchDocument| {
                let b = by_id(doc, "b");
                doc.set_attribute(
                    b,
                    "style",
                    "margin: 59px 0 0 7px; width: 60px; height: 30px; background: rgb(0, 0, 255)",
                );
                let c = by_id(doc, "c");
                doc.set_attribute(
                    c,
                    "style",
                    "width: 170px; height: 110px; background: rgb(255, 0, 0)",
                );
            },
        );
    }
}

/// A change under the box in one frame, a move of the box in the next: the
/// first frame's paint drops the box's recorded reach, so the move walks the
/// grown subtree and clears its old overflow.
#[test]
fn a_move_after_a_change_under_the_box_reads_no_stale_reach() {
    const PAGE: &str = r#"<div id="b" style="margin: 13px 0 0 21px; width: 60px; height: 30px; background: rgb(0, 0, 255)"><div id="c" style="width: 50px; height: 20px; background: rgb(255, 0, 0)"></div></div>"#;
    for scale in [1.0, 2.0] {
        partial_matches_full_twice(
            PAGE,
            scale,
            &|doc: &mut RinchDocument| {
                let c = by_id(doc, "c");
                doc.set_attribute(
                    c,
                    "style",
                    "width: 170px; height: 110px; background: rgb(255, 0, 0)",
                );
            },
            &restyle(
                "b",
                "margin: 59px 0 0 87px; width: 60px; height: 30px; background: rgb(0, 0, 255)",
            ),
        );
    }
}

/// A box below the window whose child reaches up into it: the move is not
/// skipped as off the surface, since the box's reach is on it.
#[test]
fn an_off_window_box_reaching_into_the_window_is_not_skipped() {
    const PAGE: &str = r#"<div style="height: 320px"></div><div id="b" style="margin-left: 21px; width: 60px; height: 30px; background: rgb(0, 0, 255)"><div style="position: relative; top: -70px; width: 130px; height: 40px; background: rgb(255, 0, 0)"></div></div>"#;
    each_scale(
        PAGE,
        restyle(
            "b",
            "margin-left: 69px; width: 60px; height: 30px; background: rgb(0, 0, 255)",
        ),
    );
}

/// [`partial_matches_full`] for a second change after a first one was
/// painted partially and consumed.
#[track_caller]
fn partial_matches_full_twice(
    page: &str,
    scale: f64,
    first: &dyn Fn(&mut RinchDocument),
    second: &dyn Fn(&mut RinchDocument),
) {
    let mut doc = document(page);
    let (pw, ph) = ((W as f64 * scale) as u32, (H as f64 * scale) as u32);
    let mut p = TinySkiaPainter::new(pw, ph);
    p.fill_white();
    paint(&mut doc, &mut p, scale);
    doc.tree.consume_paint_dirty();
    for change in [first, second] {
        change(&mut doc);
        doc.resolve_layout(W, H);
        let damage = rinch_dom::paint::compute_damage(&doc.tree, scale, pw as f64, ph as f64);
        assert!(!damage.is_full(), "the change damaged the whole frame");
        let rects: Vec<Rect> = damage.rects().to_vec();
        for r in &rects {
            let r = r
                .intersect(Rect::new(0.0, 0.0, pw as f64, ph as f64))
                .expand();
            p.clear_rect_white(
                r.x0 as u32,
                r.y0 as u32,
                r.width() as u32,
                r.height() as u32,
            );
        }
        p.push_damage_clip(&damage);
        rinch_dom::paint::set_dirty_rects(Some(&rects));
        paint(&mut doc, &mut p, scale);
        rinch_dom::paint::set_dirty_region(None);
        rinch_dom::paint::painter::Painter::pop_layer(&mut p);
        doc.tree.consume_paint_dirty();
    }
    let mut fresh = document(page);
    first(&mut fresh);
    fresh.resolve_layout(W, H);
    second(&mut fresh);
    fresh.resolve_layout(W, H);
    let mut full = TinySkiaPainter::new(pw, ph);
    full.fill_white();
    paint(&mut fresh, &mut full, scale);
    let bad = p
        .pixels()
        .chunks(4)
        .zip(full.pixels().chunks(4))
        .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 1))
        .count();
    assert_eq!(
        bad, 0,
        "{bad} pixels of the second partial repaint are not the new state"
    );
}

/// A box moved whose child's `scale(12)` magnifies what snapping does at a
/// fractional scale (#2368): paint snaps the child's edges to the device
/// grid in its own space, before the transform, so half a device pixel there
/// is six on screen. The reach the moved box recorded (#2222) is measured at
/// no particular scale and read at 1.25x and 1.5x, so it has to carry what
/// that half pixel becomes through the transform; grown by half a pixel
/// after the transform, it left 338 stale pixels at 1.25x (458 after a
/// second move). The child's 1.5px margin puts its edges off the grid.
const SCALED_CHILD: &str = r#"<div style="margin: 0; padding: 0"><div id="b" style="margin: 13px 0 0 40px; width: 60px; height: 30px"><div style="margin-left: 1.5px; width: 11px; height: 10px; transform: scale(12); background: rgb(255, 0, 0)"></div></div></div>"#;

/// [`SCALED_CHILD`] one level down, inside a plain box: the moved box's reach
/// is measured from that box's recorded one, which has to carry the scaled
/// box's gain up with it.
const SCALED_GRANDCHILD: &str = r#"<div style="margin: 0; padding: 0"><div id="b" style="margin: 13px 0 0 40px; width: 60px; height: 30px"><div style="width: 20px; height: 12px"><div style="margin-left: 1.5px; width: 11px; height: 10px; transform: scale(12); background: rgb(255, 0, 0)"></div></div></div></div>"#;

#[test]
fn a_scaled_up_childs_snapped_edges_move_with_its_parent_at_a_fractional_scale() {
    let first = restyle("b", "margin: 13px 0 0 80px; width: 60px; height: 30px");
    let second = restyle("b", "margin: 21px 0 0 57px; width: 60px; height: 30px");
    for page in [SCALED_CHILD, SCALED_GRANDCHILD] {
        for scale in [1.25, 1.5] {
            partial_matches_full(page, scale, &first);
            partials_match_full(page, scale, &[&first, &second]);
        }
    }
}
