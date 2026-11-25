//! Partial repaints of a box whose `transform` changed, still a transform.
//!
//! A transform carries the box's whole painted subtree with it, but only the
//! box is paint-dirty: its children did not move against it. A transform that
//! starts or stops (a new stacking context) damages the subtree's extent as a
//! reorder; one that changes from one translate to another (every frame of a
//! `transform` transition) damaged only the box, now and as painted, so what
//! the subtree paints past the box stayed where it was: a sortable list's
//! slot, drawn by the `::before` of the item after the gap, left a copy of
//! itself at the item's unshifted place while the item slid.
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

/// A `relative` box with an absolute child below it, translated.
const BELOW: &str = r#"<div id="b" style="position: relative; margin: 60px 0 0 20px; width: 80px; height: 30px; background: rgb(0, 0, 255); transform: translateY(10px)"><div style="position: absolute; left: 0; top: 40px; width: 80px; height: 30px; background: rgb(255, 0, 0)"></div></div>"#;

#[test]
fn an_absolute_child_past_the_box_moves_with_its_translate() {
    each_scale(
        BELOW,
        restyle(
            "b",
            "position: relative; margin: 60px 0 0 20px; width: 80px; height: 30px; background: rgb(0, 0, 255); transform: translateY(-40px)",
        ),
    );
}

/// A static box whose in-flow child overflows it, translated on both axes.
const OVERFLOW: &str = r#"<div id="b" style="margin: 40px 0 0 30px; width: 60px; height: 30px; background: rgb(0, 0, 255); transform: translate(5px, 5px)"><div style="width: 130px; height: 90px; background: rgb(255, 0, 0)"></div></div>"#;

#[test]
fn an_overflowing_child_moves_with_its_parents_translate() {
    each_scale(
        OVERFLOW,
        restyle(
            "b",
            "margin: 40px 0 0 30px; width: 60px; height: 30px; background: rgb(0, 0, 255); transform: translate(47px, 61px)",
        ),
    );
}

/// A change [`partials_match_full`] applies.
type Change = Box<dyn Fn(&mut RinchDocument)>;

/// The same translate, frame by frame, as a transition writes it.
#[test]
fn an_absolute_child_follows_a_translate_frame_by_frame() {
    let frames: Vec<String> = [0, -12, -27, -40]
        .iter()
        .map(|y| format!("position: relative; margin: 60px 0 0 20px; width: 80px; height: 30px; background: rgb(0, 0, 255); transform: translateY({y}px)", y = 10 + y))
        .collect();
    let changes: Vec<Change> = frames
        .into_iter()
        .map(|s| {
            Box::new(move |doc: &mut RinchDocument| {
                let n = by_id(doc, "b");
                doc.set_attribute(n, "style", &s);
            }) as Change
        })
        .collect();
    let refs: Vec<&dyn Fn(&mut RinchDocument)> = changes.iter().map(|c| c.as_ref()).collect();
    for scale in [1.0, 2.0] {
        partials_match_full(BELOW, scale, &refs[1..]);
    }
}
