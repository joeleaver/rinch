//! #1717's review probes (PR #1726). Each compares a document whose scroller
//! holds positioned rows (bars deferred to the stacking sequence) with a twin
//! whose rows are static (bars drawn by the scroller itself), unless it says
//! otherwise. Each pins a hunk the first round's suite left free: the nested
//! order (`p1`, Chrome 155: the outer bar over the inner), the bar's own clip
//! chain (`p13`, `p4`), the hidden-scroller check (`p10`), the clip switch
//! before the bar (`p2`, `p4`, `p13`), the sort by `after` (`p14`) and the cull
//! (`p3`).

#![cfg(feature = "software-renderer")]

use peniko::kurbo::{Affine, Rect};
use peniko::{Brush, Fill};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::{PaintShape, Painter};
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 600.0;
const VH: f32 = 400.0;

fn child(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", style);
    doc.append_child(parent, d);
    d
}

fn paint_at(doc: &mut RinchDocument, scale: f64, damage: Option<Rect>) -> TinySkiaPainter {
    let mut p = TinySkiaPainter::new((VW as f64 * scale) as u32, (VH as f64 * scale) as u32);
    let mut lcx = parley::LayoutContext::<Brush>::new();
    if let Some(d) = damage {
        rinch_dom::paint::set_dirty_rects(Some(&[d]));
        p.push_clip(Fill::NonZero, Affine::IDENTITY, &PaintShape::Rect(d));
    }
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut p,
        scale,
        (VW, VH),
        &mut doc.font_cx,
        &mut lcx,
    );
    if damage.is_some() {
        p.pop_layer();
        rinch_dom::paint::set_dirty_region(None);
    }
    p
}

fn paint(doc: &mut RinchDocument) -> TinySkiaPainter {
    paint_at(doc, 1.0, None)
}

fn px(p: &TinySkiaPainter, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * p.width() + x) * 4) as usize;
    let d = p.pixels();
    [d[i], d[i + 1], d[i + 2], d[i + 3]]
}

fn diff(a: &TinySkiaPainter, b: &TinySkiaPainter, r: Rect) -> Option<(u32, u32, [u8; 4], [u8; 4])> {
    for y in r.y0 as u32..r.y1 as u32 {
        for x in r.x0 as u32..r.x1 as u32 {
            let (pa, pb) = (px(a, x, y), px(b, x, y));
            if pa != pb {
                return Some((x, y, pa, pb));
            }
        }
    }
    None
}

const WHOLE: Rect = Rect::new(0.0, 0.0, VW as f64, VH as f64);

/// P1: an outer scroller holding an inner scroller whose rows are relative.
/// Both bars are deferred to the same `after`; the outer (blue) bar must be
/// drawn over the inner (green) one, as the twin draws it.
fn nested(pos: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let a = child(
        &mut doc,
        body,
        "position: absolute; left: 20px; top: 20px; width: 200px; height: 150px; \
         overflow-y: auto; --rinch-scrollbar-color: rgb(0, 0, 255)",
    );
    let b = child(
        &mut doc,
        a,
        "width: 200px; height: 140px; overflow-y: auto; --rinch-scrollbar-color: rgb(0, 200, 0)",
    );
    for _ in 0..8 {
        child(
            &mut doc,
            b,
            &format!("height: 40px; background: rgb(255,0,0); {pos}"),
        );
    }
    child(&mut doc, a, "height: 300px");
    doc.resolve_layout(VW, VH);
    doc.tree.nodes[b.0].scroll_offset = (0.0, 0.0);
    doc.tree.nodes[a.0].scroll_offset = (0.0, 0.0);
    doc.tree.hit_cache.invalidate();
    doc
}

#[test]
fn p1_nested_bar_order() {
    let mut a = nested("position: relative");
    let mut b = nested("");
    let pa = paint(&mut a);
    let pb = paint(&mut b);
    eprintln!(
        "twin at (215, 40) = {:?}, positioned = {:?}",
        px(&pb, 215, 40),
        px(&pa, 215, 40)
    );
    assert_eq!(diff(&pa, &pb, WHOLE), None);
}

/// P2: a static inline-block scroller after text (drawn by its IFC).
fn inline_block(pos: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(
        body,
        "style",
        "margin: 0; font-size: 16px; line-height: 20px",
    );
    let p = child(&mut doc, body, "padding: 10px 7px; border: 3px solid black");
    let t = doc.create_text("abc ");
    doc.append_child(p, t);
    let s = doc.create_element("div");
    doc.set_attribute(
        s,
        "style",
        "display: inline-block; width: 150px; height: 100px; overflow-y: auto; \
         padding: 5px 0 5px 5px; border: 4px solid rgb(0,0,0); --rinch-scrollbar-color: rgb(0,0,255)",
    );
    doc.append_child(p, s);
    for _ in 0..6 {
        child(
            &mut doc,
            s,
            &format!("height: 40px; background: rgb(255,0,0); {pos}"),
        );
    }
    doc.resolve_layout(VW, VH);
    doc.tree.nodes[s.0].scroll_offset = (0.0, 17.0);
    doc.tree.hit_cache.invalidate();
    doc
}

#[test]
fn p2_inline_block_scroller() {
    let mut a = inline_block("position: relative");
    let mut b = inline_block("");
    let pa = paint(&mut a);
    let pb = paint(&mut b);
    assert_eq!(diff(&pa, &pb, WHOLE), None);
}

/// P4: an inner scroller partly scrolled out of an outer one: its deferred
/// bar must be clipped by the outer's box.
fn clipped_inner(pos: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let a = child(
        &mut doc,
        body,
        "position: absolute; left: 20px; top: 20px; width: 300px; height: 150px; \
         overflow-y: auto; background: rgb(238,238,238)",
    );
    child(&mut doc, a, "height: 100px");
    let b = child(
        &mut doc,
        a,
        "width: 200px; height: 140px; overflow-y: auto; --rinch-scrollbar-color: rgb(0, 0, 255)",
    );
    for _ in 0..8 {
        child(
            &mut doc,
            b,
            &format!("height: 40px; background: rgb(255,0,0); {pos}"),
        );
    }
    child(&mut doc, a, "height: 300px");
    doc.resolve_layout(VW, VH);
    // b's box: y 120..260 in the page, a's box ends at 170.
    doc.tree.nodes[b.0].scroll_offset = (0.0, 68.0);
    doc.tree.hit_cache.invalidate();
    doc
}

#[test]
fn p4_inner_bar_clipped_by_outer() {
    let mut a = clipped_inner("position: relative");
    let mut b = clipped_inner("");
    let pa = paint(&mut a);
    let pb = paint(&mut b);
    // Below the outer box, at the inner thumb's column.
    for y in 171..280 {
        assert_eq!(px(&pa, 215, y)[2], px(&pb, 215, y)[2], "y {y}");
    }
    assert_eq!(diff(&pa, &pb, WHOLE), None);
}

/// P6: sticky rows.
fn sticky(pos: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let s = child(
        &mut doc,
        body,
        "position: absolute; left: 20px; top: 20px; width: 200px; height: 150px; \
         overflow-y: auto; --rinch-scrollbar-color: rgb(0,0,255)",
    );
    child(
        &mut doc,
        s,
        &format!("height: 40px; background: rgb(255,0,0); {pos}"),
    );
    for _ in 0..6 {
        child(&mut doc, s, "height: 40px; background: rgb(0,255,0)");
    }
    doc.resolve_layout(VW, VH);
    doc.tree.nodes[s.0].scroll_offset = (0.0, 30.0);
    doc.tree.hit_cache.invalidate();
    doc
}

#[test]
fn p6_sticky_row() {
    // The twin cannot be static (the row would scroll away); compare the
    // thumb's column at the sticky row's top against a row-less control:
    // the thumb is at the top (scroll 30 of 130).
    let mut a = sticky("position: sticky; top: 0");
    let pa = paint(&mut a);
    for y in (20..170).step_by(5) {
        eprintln!("y {y}: {:?}", px(&pa, 215, y));
    }
    let c = px(&pa, 215, 50);
    eprintln!("thumb pixel over sticky row: {c:?}");
    assert_eq!(c, [0, 0, 255, 255]);
}

/// P7: both bars and the corner.
fn both(pos: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let s = child(
        &mut doc,
        body,
        "position: absolute; left: 20px; top: 20px; width: 200px; height: 150px; \
         overflow: auto; --rinch-scrollbar-color: rgb(0,0,255) rgb(255,255,0)",
    );
    for _ in 0..6 {
        child(
            &mut doc,
            s,
            &format!("height: 40px; width: 400px; background: rgb(255,0,0); {pos}"),
        );
    }
    doc.resolve_layout(VW, VH);
    doc.tree.nodes[s.0].scroll_offset = (37.0, 23.0);
    doc.tree.hit_cache.invalidate();
    doc
}

#[test]
fn p7_both_axes_and_corner() {
    let mut a = both("position: relative");
    let mut b = both("");
    let pa = paint(&mut a);
    let pb = paint(&mut b);
    assert_eq!(diff(&pa, &pb, WHOLE), None);
}

/// P7b: scrollbar-width thin.
#[test]
fn p7b_thin() {
    let mk = |pos: &str| {
        let mut doc = both(pos);
        let s = doc.get_children(doc.body())[0];
        doc.set_style(s, "--rinch-scrollbar-width", "thin");
        doc.resolve_layout(VW + 1.0, VH);
        doc.tree.nodes[s.0].scroll_offset = (37.0, 23.0);
        doc.tree.hit_cache.invalidate();
        doc
    };
    let mut a = mk("position: relative");
    let mut b = mk("");
    let pa = paint(&mut a);
    let pb = paint(&mut b);
    assert_eq!(diff(&pa, &pb, WHOLE), None);
}

/// P8: rows behind a display: contents wrapper, the scroller itself behind
/// one too.
fn contents(pos: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let w0 = child(&mut doc, body, "display: contents");
    let s = child(
        &mut doc,
        w0,
        "position: absolute; left: 20px; top: 20px; width: 200px; height: 150px; \
         overflow-y: auto; --rinch-scrollbar-color: rgb(0,0,255)",
    );
    let w = child(&mut doc, s, "display: contents");
    for _ in 0..6 {
        child(
            &mut doc,
            w,
            &format!("height: 40px; background: rgb(255,0,0); {pos}"),
        );
    }
    doc.resolve_layout(VW, VH);
    doc.tree.nodes[s.0].scroll_offset = (0.0, 23.0);
    doc.tree.hit_cache.invalidate();
    doc
}

#[test]
fn p8_display_contents() {
    let mut a = contents("position: relative");
    let mut b = contents("");
    let pa = paint(&mut a);
    let pb = paint(&mut b);
    assert_eq!(diff(&pa, &pb, WHOLE), None);
}

/// P10: a hidden scroller whose rows are visible: no bar either way.
fn hidden(pos: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let s = child(
        &mut doc,
        body,
        "position: absolute; left: 20px; top: 20px; width: 200px; height: 150px; \
         overflow-y: auto; visibility: hidden; --rinch-scrollbar-color: rgb(0,0,255)",
    );
    for _ in 0..6 {
        child(
            &mut doc,
            s,
            &format!("height: 40px; background: rgb(255,0,0); visibility: visible; {pos}"),
        );
    }
    doc.resolve_layout(VW, VH);
    doc.tree.nodes[s.0].scroll_offset = (0.0, 23.0);
    doc.tree.hit_cache.invalidate();
    doc
}

#[test]
fn p10_hidden_scroller() {
    let mut a = hidden("position: relative");
    let mut b = hidden("");
    let pa = paint(&mut a);
    let pb = paint(&mut b);
    assert_eq!(diff(&pa, &pb, WHOLE), None);
}

/// P11: a partial repaint whose damage covers the thumb only (the `after`
/// row is outside it and culled).
#[test]
fn p11_damage_over_thumb_only() {
    let mut a = sticky("position: relative");
    let full = paint(&mut a);
    let dmg = Rect::new(210.0, 20.0, 220.0, 60.0);
    let part = paint_at(&mut a, 1.0, Some(dmg));
    assert_eq!(diff(&part, &full, dmg), None);
}

/// P9: the scroller's ancestor is a 0x0 `overflow: hidden` box (#1696) and
/// the scroller is relative (clipped away by it): nothing may show.
fn zero_clip(pos: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let z = child(&mut doc, body, "width: 0; height: 0; overflow: hidden");
    let s = child(
        &mut doc,
        z,
        "position: relative; left: 20px; top: 20px; width: 200px; height: 150px; \
         overflow-y: auto; --rinch-scrollbar-color: rgb(0,0,255)",
    );
    for _ in 0..6 {
        child(
            &mut doc,
            s,
            &format!("height: 40px; background: rgb(255,0,0); {pos}"),
        );
    }
    doc.resolve_layout(VW, VH);
    doc.tree.nodes[s.0].scroll_offset = (0.0, 23.0);
    doc.tree.hit_cache.invalidate();
    doc
}

#[test]
fn p9_zero_size_clipper() {
    let mut a = zero_clip("position: relative");
    let mut b = zero_clip("");
    let pa = paint(&mut a);
    let pb = paint(&mut b);
    assert_eq!(diff(&pa, &pb, WHOLE), None);
}

/// P3: offscreen scrollers. Count the thumb fills a frame encodes for
/// scrollers that are entirely below the window.
#[derive(Default)]
struct Recorder {
    fills: Vec<Rect>,
}
impl Painter for Recorder {
    fn reset(&mut self) {}
    fn fill(&mut self, _: Fill, _: Affine, _: &Brush, shape: &PaintShape) {
        self.fills.push(shape.bounding_box());
    }
    fn stroke(&mut self, _: &peniko::kurbo::Stroke, _: Affine, _: &Brush, _: &PaintShape) {}
    #[allow(clippy::too_many_arguments)]
    fn draw_glyphs(
        &mut self,
        _: &peniko::FontData,
        _: f32,
        _: Affine,
        _: Option<Affine>,
        _: &Brush,
        _: bool,
        _: &[i16],
        _: &[rinch_dom::paint::painter::PaintGlyph],
    ) {
    }
    fn draw_image(&mut self, _: &rinch_dom::paint::painter::PaintImage<'_>, _: Affine) {}
    fn push_clip(&mut self, _: Fill, _: Affine, _: &PaintShape) {}
    fn push_layer(
        &mut self,
        _: rinch_dom::paint::painter::BlendMode,
        _: f32,
        _: Affine,
        _: &PaintShape,
    ) {
    }
    fn pop_layer(&mut self) {}
}

fn cards(pos: &str, n: usize) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    for _ in 0..n {
        let s = child(
            &mut doc,
            body,
            "width: 200px; height: 40px; overflow-y: auto",
        );
        for _ in 0..3 {
            child(
                &mut doc,
                s,
                &format!("height: 20px; background: rgb(255,0,0); {pos}"),
            );
        }
    }
    doc.resolve_layout(VW, VH);
    doc
}

/// P3: 200 `overflow: auto` cards of 3 rows in a 400px window. A deferred
/// bar is culled as the scroller's own `paint_node` is: the relative rows cost
/// the fills the static ones do, and only the scrollers on screen (and in the
/// cull's ink margin) count a deferred bar.
#[test]
fn p3_offscreen_scroller_fills() {
    let mut fills = Vec::new();
    for pos in ["", "position: relative"] {
        let mut doc = cards(pos, 200);
        let mut r = Recorder::default();
        let mut lcx = parley::LayoutContext::<Brush>::new();
        doc.tree.perf.reset();
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut r,
            1.0,
            (VW, VH),
            &mut doc.font_cx,
            &mut lcx,
        );
        let below = r.fills.iter().filter(|f| f.y0 >= VH as f64).count();
        let deferred = doc
            .tree
            .perf
            .get(rinch_dom::perf::Counter::DeferredScrollbarsPainted);
        fills.push((r.fills.len(), below, deferred));
    }
    // 10 cards in the window and 2 in the cull's 64px ink margin are drawn;
    // with relative rows each of those defers its bar, and no other does (196
    // thumbs were filled below the window before the cull).
    assert_eq!(
        fills[0].0, fills[1].0,
        "same fills with relative rows: {fills:?}"
    );
    assert_eq!(
        fills[0].1, fills[1].1,
        "same fills below the window: {fills:?}"
    );
    assert_eq!(fills[0].2, 0, "{fills:?}");
    assert_eq!(
        fills[1].2, 12,
        "a deferred bar per card the cull lets through: {fills:?}"
    );
}

/// P13: an absolute scroller whose containing block (the body) is above a
/// static `overflow: hidden` box it sits in: the scroller escapes that box,
/// and so must its deferred bar.
fn escaping_scroller(pos: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let h = child(
        &mut doc,
        body,
        "width: 100px; height: 100px; overflow: hidden",
    );
    let s = child(
        &mut doc,
        h,
        "position: absolute; left: 20px; top: 20px; width: 200px; height: 150px; \
         overflow-y: auto; --rinch-scrollbar-color: rgb(0,0,255)",
    );
    for _ in 0..6 {
        child(
            &mut doc,
            s,
            &format!("height: 40px; background: rgb(255,0,0); {pos}"),
        );
    }
    doc.resolve_layout(VW, VH);
    doc.tree.nodes[s.0].scroll_offset = (0.0, 23.0);
    doc.tree.hit_cache.invalidate();
    doc
}

#[test]
fn p13_escaping_absolute_scroller() {
    let mut a = escaping_scroller("position: relative");
    let mut b = escaping_scroller("");
    let pa = paint(&mut a);
    let _ = &mut b;
    // The rows themselves are clipped at the hidden box (x 100) at base and
    // head alike: a pre-existing collector gap, not this PR (issue draft 2).
    for y in [45u32, 50, 55] {
        assert_eq!(px(&pa, 215, y), [0, 0, 255, 255], "y {y}");
    }
}

/// P14: two sibling scrollers whose deferred bars come in a different order
/// from the tree order (the first one's rows at `z-index: 5`, the second's at
/// `auto`): both bars are drawn.
fn two_scrollers(pos_a: &str, pos_b: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    for (left, pos) in [(20, pos_a), (260, pos_b)] {
        let s = child(
            &mut doc,
            body,
            &format!(
                "position: absolute; left: {left}px; top: 20px; width: 200px; height: 150px; \
                 overflow-y: auto; --rinch-scrollbar-color: rgb(0,0,255)"
            ),
        );
        for _ in 0..6 {
            child(
                &mut doc,
                s,
                &format!("height: 40px; background: rgb(255,0,0); {pos}"),
            );
        }
        doc.resolve_layout(VW, VH);
        doc.tree.nodes[s.0].scroll_offset = (0.0, 23.0);
    }
    doc.tree.hit_cache.invalidate();
    doc
}

#[test]
fn p14_bars_out_of_tree_order() {
    let mut a = two_scrollers("position: relative; z-index: 5", "position: relative");
    let mut b = two_scrollers("", "");
    let pa = paint(&mut a);
    let pb = paint(&mut b);
    assert_eq!(
        px(&pa, 455, 60),
        [0, 0, 255, 255],
        "the second scroller's thumb"
    );
    assert_eq!(
        px(&pa, 215, 60),
        [0, 0, 255, 255],
        "the first scroller's thumb"
    );
    assert_eq!(diff(&pa, &pb, WHOLE), None);
}
