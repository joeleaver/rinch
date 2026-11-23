//! Partial repaints of a `filter` on an inline span (#2869).
//!
//! A filtered span is drawn in its own entry through an offscreen raster
//! (`paint::filter::paint_filtered_span`), so its ink reaches past its line
//! boxes by the filter's reach. A span has no box: its damage is its IFC
//! root's rect, which a span's own filter must grow by its reach now and as
//! it was painted (`PaintedState::filter_reach`), or adding, removing or
//! changing the filter leaves stale shadow pixels past the root (review of
//! #2895: a hover glow on a link in a shrink-wrapped label). A change inside
//! the span (its text) is grown by the span's reach as an ancestor's
//! (`filter::ancestor_reach`).
//!
//! Each case paints a frame, applies a change, repaints only the damage the
//! change names, and compares that with a full paint of the new state on a
//! fresh document.

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use peniko::kurbo::Rect;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const W: f32 = 300.0;
const H: f32 = 200.0;

/// A shrink-wrapped label holding a span, the label well inside the page so
/// a shadow past it lands on white.
const PAGE: &str = r#"<div style="position: absolute; left: 60px; top: 50px; width: max-content; font: 20px/30px sans-serif">go <span id="s" style="color: rgb(0, 0, 255)">link</span></div>"#;
const REL: &str = "position: relative; top: 4px; color: rgb(0, 0, 255);";
const GLOW: &str = "filter: drop-shadow(0 0 6px rgb(255, 0, 0));";
const FAR: &str = "filter: drop-shadow(24px 18px 0 rgb(255, 0, 0));";
const NEAR: &str = "filter: drop-shadow(3px 2px 0 rgb(255, 0, 0));";

fn document(page: &str, span_style: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_inner_html(body, page);
    let s = span(&doc);
    doc.set_attribute(s, "style", span_style);
    doc.resolve_layout(W, H);
    doc
}

fn span(doc: &RinchDocument) -> rinch_core::dom::NodeId {
    doc.query_selector_all("#s")[0]
}

fn paint(doc: &mut RinchDocument, p: &mut TinySkiaPainter, scale: f64) {
    let mut lcx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(&doc.tree, p, scale, (W, H), &mut doc.font_cx, &mut lcx);
}

/// Paint with the span styled `from`, apply `change`, repaint only the
/// damage, and compare with a fresh full paint after the same change.
#[track_caller]
fn partial_matches_full(from: &str, scale: f64, change: impl Fn(&mut RinchDocument)) {
    let mut doc = document(PAGE, from);
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
    let mut fresh = document(PAGE, from);
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

fn restyle(style: String) -> impl Fn(&mut RinchDocument) {
    move |doc: &mut RinchDocument| {
        let s = span(doc);
        doc.set_attribute(s, "style", &style);
    }
}

/// Each change, on a static and on a relative span, at scale 1 and 2.
fn each_base(f: impl Fn(&str, f64)) {
    for base in ["color: rgb(0, 0, 255);", REL] {
        for scale in [1.0, 2.0] {
            f(base, scale);
        }
    }
}

/// A filter added: the new shadow, past the label, is painted.
#[test]
fn a_filter_added_to_a_span_repaints_its_reach() {
    each_base(|base, scale| {
        partial_matches_full(base, scale, restyle(format!("{base} {GLOW}")));
        partial_matches_full(base, scale, restyle(format!("{base} {FAR}")));
    });
}

/// A filter removed (the hover glow going away): the old shadow is cleared.
#[test]
fn a_filter_removed_from_a_span_clears_its_old_reach() {
    each_base(|base, scale| {
        partial_matches_full(&format!("{base} {GLOW}"), scale, restyle(base.to_string()));
        partial_matches_full(&format!("{base} {FAR}"), scale, restyle(base.to_string()));
    });
}

/// A filter changed from a far shadow to a near one: the far one is cleared.
#[test]
fn a_filter_changed_on_a_span_clears_the_old_reach() {
    each_base(|base, scale| {
        partial_matches_full(
            &format!("{base} {FAR}"),
            scale,
            restyle(format!("{base} {NEAR}")),
        );
    });
}

/// The span's text changed under its shadow: the shadow of the old and the
/// new text, past the label, is repainted (the span's reach as the text's
/// ancestor's).
#[test]
fn text_changed_under_a_spans_shadow_repaints_the_shadow() {
    each_base(|base, scale| {
        partial_matches_full(&format!("{base} {FAR}"), scale, |doc| {
            let s = span(doc);
            let text = doc.get_children(s)[0];
            doc.set_text_content(text, "lnk");
        });
    });
}
