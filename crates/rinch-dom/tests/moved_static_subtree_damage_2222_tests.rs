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
    let mut doc = document(page);
    let (pw, ph) = ((W as f64 * scale) as u32, (H as f64 * scale) as u32);
    let mut p = TinySkiaPainter::new(pw, ph);
    p.fill_white();
    paint(&mut doc, &mut p, scale);
    let before = p.pixels().to_vec();
    doc.tree.consume_paint_dirty();
    change(&mut doc);
    doc.resolve_layout(W, H);
    let damage = rinch_dom::paint::compute_damage(&doc.tree, scale, pw as f64, ph as f64);
    assert!(!damage.is_full(), "the change damaged the whole frame");
    let rects: Vec<Rect> = damage.rects().to_vec();
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
    let partial = p.pixels().to_vec();
    let mut fresh = document(page);
    change(&mut fresh);
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
        "{} pixels of the partial repaint are not the new state, first {:?} (damage {rects:?})",
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
