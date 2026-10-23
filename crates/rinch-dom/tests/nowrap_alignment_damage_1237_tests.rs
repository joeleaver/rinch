//! The damage of a `nowrap` paragraph aligned within its box (#1237, review
//! of #1868): its lines are given the box's width as their alignment width,
//! and a line wider than the box overflows it. What the paragraph draws past
//! its box — text moved by `vertical-align`, a wavy underline — reaches as
//! far as the line's content, not the alignment width, or recolouring it
//! repaints only part of it.

#![cfg(feature = "software-renderer")]

use peniko::kurbo::Affine;
use peniko::{Brush, Fill};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::{PaintShape, Painter};
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const VP: (f32, f32) = (200.0, 80.0);

fn doc(root_style: &str, span_style: &str) -> (RinchDocument, NodeId) {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut d = RinchDocument::new();
    d.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    d.load_css("html, body { margin: 0 } body { padding: 30px 0 0 10px }");
    let body = d.body();
    let c = d.create_element("div");
    d.set_attribute(
        c,
        "style",
        &format!("width:50px;font:16px/16px ProbeFace;color:rgb(0,0,0);{root_style}"),
    );
    d.append_child(body, c);
    d.set_inner_html(c, "aaaaaaaaaa<span>bbb</span>");
    let span = d.tree.get(c.0).unwrap().children[1];
    let span = NodeId(span);
    d.set_attribute(span, "style", span_style);
    d.resolve_layout(VP.0, VP.1);
    (d, span)
}

fn full(d: &mut RinchDocument, p: &mut TinySkiaPainter) {
    p.fill_white();
    let mut cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(&d.tree, p, 1.0, VP, &mut d.font_cx, &mut cx);
}

/// Pixels that differ between two paints.
fn differing(a: &[u8], b: &[u8]) -> usize {
    a.as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .filter(|(x, y)| x != y)
        .count()
}

/// Recolour the span, then repaint only the damage the frame names over the
/// old paint: it must be the full paint of the new state.
fn recolour_repaints_the_span(root_style: &str, span_style: &str) {
    let (mut d, span) = doc(root_style, span_style);
    let mut p = TinySkiaPainter::new(VP.0 as u32, VP.1 as u32);
    full(&mut d, &mut p);
    d.tree.consume_paint_dirty();
    d.set_attribute(span, "style", &format!("{span_style};color:rgb(220,0,0)"));
    d.resolve_layout(VP.0, VP.1);
    let damage = rinch_dom::paint::compute_damage(&d.tree, 1.0, VP.0 as f64, VP.1 as f64);
    assert!(!damage.is_full(), "precondition: a partial repaint");
    let rects = damage.rects().to_vec();
    assert!(!rects.is_empty(), "precondition: the recolour names damage");
    for r in &rects {
        let r = r.intersect(peniko::kurbo::Rect::new(0.0, 0.0, VP.0 as f64, VP.1 as f64));
        p.clear_rect_white(
            r.x0 as u32,
            r.y0 as u32,
            r.width().ceil() as u32,
            r.height().ceil() as u32,
        );
    }
    rinch_dom::paint::set_dirty_rects(Some(&rects));
    for r in &rects {
        p.push_clip(Fill::NonZero, Affine::IDENTITY, &PaintShape::Rect(*r));
        let mut cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(&d.tree, &mut p, 1.0, VP, &mut d.font_cx, &mut cx);
        p.pop_layer();
    }
    rinch_dom::paint::set_dirty_region(None);
    let partial = p.pixels().to_vec();
    d.tree.consume_paint_dirty();
    full(&mut d, &mut p);
    let n = differing(&partial, p.pixels());
    assert_eq!(
        n, 0,
        "{root_style} / {span_style}: {n} px of the recoloured span were not repainted ({rects:?})"
    );
}

/// The span is raised off its line by `vertical-align`, past the box's right
/// edge: the paragraph's ink past its lines reaches it.
#[test]
fn a_raised_span_past_the_box_is_repainted_when_recoloured() {
    recolour_repaints_the_span("white-space:nowrap;text-align:right", "vertical-align:20px");
}
