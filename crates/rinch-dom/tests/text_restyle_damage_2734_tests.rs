//! A restyle or relayout names only the text it changed (#2734).
//!
//! A text child of an IFC root gets its box from the root's lines
//! (`write_inline_positions`: the root's width and the lines' height) when
//! they are shaped. Every later layout read it back from Taffy, where the
//! text is a measure leaf laid out `0x0`, so the first layout that kept the
//! lines flipped every paragraph's text box to `0x0` and pushed it
//! paint-dirty: a width change on one absolutely positioned box named every
//! paragraph on the page as damage.
//!
//! The second half is the other direction: what does change a text's paint —
//! an inherited colour or font, a `currentcolor` decoration or shadow, a theme
//! variable, a `:hover` rule — is still repainted, and the partial repaint is
//! the full paint of a fresh document in the same state.

#![cfg(feature = "software-renderer")]

use peniko::kurbo::{Affine, Rect};
use peniko::{Brush, Fill};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::{PaintShape, Painter};
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const VP: (f32, f32) = (400.0, 300.0);
const ROWS: usize = 6;

const CSS: &str = "
    html, body { margin: 0 }
    :root { --fg: rgb(0, 0, 0) }
    :root.dark { --fg: rgb(0, 120, 0) }
    #w { font: 16px/20px ProbeFace; color: var(--fg) }
    p { margin: 0 }
    p.hov:hover { color: rgb(200, 0, 0) }
    #w.boxhover:hover { color: rgb(0, 0, 200) }
    p.u { text-decoration: underline; text-decoration-color: currentcolor;
          text-shadow: 2px 2px currentcolor }
    #glass { position: absolute; left: 220px; top: 200px; width: 150px; height: 60px }
    #gauge { position: absolute; left: 0; top: 0; width: 120px; height: 10px;
             background: rgb(0, 0, 255) }
";

struct Page {
    d: RinchDocument,
    w: NodeId,
    rows: Vec<NodeId>,
    gauge: NodeId,
    label: NodeId,
}

fn page() -> Page {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut d = RinchDocument::new();
    d.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    d.load_css(CSS);
    let body = d.body();
    let w = d.create_element("div");
    d.set_attribute(w, "id", "w");
    d.append_child(body, w);
    let mut rows = Vec::new();
    for i in 0..ROWS {
        let p = d.create_element("p");
        d.set_attribute(p, "class", if i == 1 { "u hov" } else { "hov" });
        d.append_child(w, p);
        let t = d.create_text(&format!("Row {i} of text"));
        d.append_child(p, t);
        rows.push(p);
    }
    let glass = d.create_element("div");
    d.set_attribute(glass, "id", "glass");
    d.append_child(w, glass);
    let gauge = d.create_element("div");
    d.set_attribute(gauge, "id", "gauge");
    d.append_child(glass, gauge);
    let label_p = d.create_element("p");
    d.set_attribute(label_p, "style", "position:absolute;left:0;top:20px");
    d.append_child(glass, label_p);
    let label = d.create_text("42%");
    d.append_child(label_p, label);
    d.resolve_layout(VP.0, VP.1);
    Page {
        d,
        w,
        rows,
        gauge,
        label,
    }
}

fn text_ids(d: &RinchDocument) -> Vec<u64> {
    d.tree
        .nodes
        .iter()
        .filter(|(_, n)| n.is_text())
        .map(|(id, _)| id)
        .collect()
}

fn full(d: &mut RinchDocument, p: &mut TinySkiaPainter) {
    p.fill_white();
    let mut cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(&d.tree, p, 1.0, VP, &mut d.font_cx, &mut cx);
}

fn differing(a: &[u8], b: &[u8]) -> usize {
    a.as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .filter(|(x, y)| x != y)
        .count()
}

/// Paint the page, apply `op` and lay out again, then repaint only the damage
/// the frame names over the old pixels. Answers whether the damage was
/// partial. The result must be the full paint of the same document, and of a
/// fresh page that took the same `op`.
fn partial_matches_full(name: &str, op: impl Fn(&mut Page)) -> bool {
    let mut pg = page();
    let mut p = TinySkiaPainter::new(VP.0 as u32, VP.1 as u32);
    full(&mut pg.d, &mut p);
    pg.d.tree.consume_paint_dirty();
    op(&mut pg);
    pg.d.resolve_layout(VP.0, VP.1);
    let damage = rinch_dom::paint::compute_damage(&pg.d.tree, 1.0, VP.0 as f64, VP.1 as f64);
    let partial_damage = !damage.is_full();
    if partial_damage {
        let rects = damage.rects().to_vec();
        for r in &rects {
            let r = r.intersect(Rect::new(0.0, 0.0, VP.0 as f64, VP.1 as f64));
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
            rinch_dom::paint::paint_document(
                &pg.d.tree,
                &mut p,
                1.0,
                VP,
                &mut pg.d.font_cx,
                &mut cx,
            );
            p.pop_layer();
        }
        rinch_dom::paint::set_dirty_region(None);
    } else {
        full(&mut pg.d, &mut p);
    }
    let partial = p.pixels().to_vec();
    pg.d.tree.consume_paint_dirty();

    let mut q = TinySkiaPainter::new(VP.0 as u32, VP.1 as u32);
    full(&mut pg.d, &mut q);
    let n = differing(&partial, q.pixels());
    assert_eq!(n, 0, "{name}: {n} px differ from the full paint");

    let mut fresh = page();
    op(&mut fresh);
    fresh.d.resolve_layout(VP.0, VP.1);
    full(&mut fresh.d, &mut q);
    let n = differing(&partial, q.pixels());
    assert_eq!(n, 0, "{name}: {n} px differ from a fresh page's full paint");
    assert!(
        differing(&partial, &{
            let mut before = page();
            let mut b = TinySkiaPainter::new(VP.0 as u32, VP.1 as u32);
            full(&mut before.d, &mut b);
            b.pixels().to_vec()
        }) > 0,
        "{name}: positive control: the op changes the pixels"
    );
    partial_damage
}

fn widen_gauge(pg: &mut Page) {
    pg.d.set_attribute(pg.gauge, "style", "width:131px");
}

/// The issue's frame: one absolute box's width changes, and no text is
/// pushed. It pushed all seven paragraphs' text (#2734).
#[test]
fn an_unrelated_restyle_marks_no_text_paint_dirty() {
    let mut pg = page();
    pg.d.tree.consume_paint_dirty();
    widen_gauge(&mut pg);
    pg.d.resolve_layout(VP.0, VP.1);
    let texts = text_ids(&pg.d);
    let dirty: Vec<_> =
        pg.d.tree
            .paint_dirty_nodes
            .iter()
            .filter(|id| texts.contains(id))
            .collect();
    assert!(
        dirty.is_empty(),
        "text pushed: {dirty:?} (all: {:?})",
        pg.d.tree.paint_dirty_nodes
    );
    // The damage is the gauge's old and new box, nowhere near the rows.
    let damage = rinch_dom::paint::compute_damage(&pg.d.tree, 1.0, VP.0 as f64, VP.1 as f64);
    assert!(!damage.is_full());
    for r in damage.rects() {
        assert!(
            r.y0 >= 150.0,
            "damage reaches the rows: {:?}",
            damage.rects()
        );
    }
}

/// The same with the label's text set in the same frame: the label's own
/// paragraph is rebuilt, and no other text is pushed.
#[test]
fn a_set_text_beside_it_pushes_no_other_text() {
    let mut pg = page();
    pg.d.tree.consume_paint_dirty();
    widen_gauge(&mut pg);
    pg.d.set_text_content(pg.label, "43%");
    pg.d.resolve_layout(VP.0, VP.1);
    let row_texts: Vec<u64> = pg
        .rows
        .iter()
        .map(|r| pg.d.tree.nodes[r.0].children[0])
        .collect();
    let dirty: Vec<_> =
        pg.d.tree
            .paint_dirty_nodes
            .iter()
            .filter(|id| row_texts.contains(id))
            .collect();
    assert!(dirty.is_empty(), "row text pushed: {dirty:?}");
}

/// The text box keeps the size its lines gave it across a layout that keeps
/// them, and follows its root when the root's width changes.
#[test]
fn a_text_box_is_its_roots_width_and_its_lines_height() {
    let mut pg = page();
    let t = pg.d.tree.nodes[pg.rows[0].0].children[0];
    let first = pg.d.tree.nodes[t].layout;
    assert_eq!((first.width, first.height), (400.0, 20.0));
    widen_gauge(&mut pg);
    pg.d.resolve_layout(VP.0, VP.1);
    assert_eq!(pg.d.tree.nodes[t].layout, first);
    pg.d.set_attribute(pg.w, "style", "width:321px");
    pg.d.resolve_layout(VP.0, VP.1);
    let l = pg.d.tree.nodes[t].layout;
    assert_eq!((l.width, l.height), (321.0, 20.0));
}

/// Off the fixed point: a root shorter than its lines. The text box is the
/// lines' height, not the root's.
#[test]
fn a_text_box_is_its_lines_height_in_a_shorter_root() {
    let mut pg = page();
    pg.d.set_attribute(pg.rows[0], "style", "height:7px");
    pg.d.resolve_layout(VP.0, VP.1);
    let t = pg.d.tree.nodes[pg.rows[0].0].children[0];
    widen_gauge(&mut pg);
    pg.d.resolve_layout(VP.0, VP.1);
    let l = pg.d.tree.nodes[t].layout;
    assert_eq!((l.width, l.height), (400.0, 20.0));
    assert_eq!(pg.d.tree.nodes[pg.rows[0].0].layout.height, 7.0);
}

#[test]
fn an_unrelated_restyle_repaints_like_a_full_paint() {
    assert!(partial_matches_full("gauge width", widen_gauge));
    assert!(partial_matches_full("gauge width and label text", |pg| {
        widen_gauge(pg);
        pg.d.set_text_content(pg.label, "43%");
    }));
}

/// An inherited colour on the rows' ancestor repaints every row, the
/// `currentcolor` underline and shadow included.
#[test]
fn an_inherited_colour_change_repaints_the_text() {
    partial_matches_full("ancestor colour", |pg| {
        pg.d.set_attribute(pg.w, "style", "color: rgb(150, 0, 150)");
    });
    // With an unrelated relayout in the same frame.
    partial_matches_full("ancestor colour and gauge", |pg| {
        pg.d.set_attribute(pg.w, "style", "color: rgb(150, 0, 150)");
        widen_gauge(pg);
    });
}

/// An inherited font change relays the text out and repaints it.
#[test]
fn an_inherited_font_change_repaints_the_text() {
    partial_matches_full("ancestor font-size", |pg| {
        pg.d.set_attribute(pg.w, "style", "font-size: 19px");
    });
    partial_matches_full("ancestor letter-spacing", |pg| {
        pg.d.set_attribute(pg.w, "style", "letter-spacing: 2px");
    });
}

/// A theme change (a variable the text's colour reads, switched by a class on
/// the root).
#[test]
fn a_theme_change_repaints_the_text() {
    partial_matches_full("theme class", |pg| {
        let html = pg.d.tree.nodes[pg.d.body().0].parent.unwrap();
        pg.d.set_attribute(NodeId(html), "class", "dark");
    });
    partial_matches_full("theme css", |pg| {
        pg.d.set_theme_css("html:root { --fg: rgb(120, 60, 0) }");
    });
}

/// `:hover` on a text's own container, and on an ancestor of it.
#[test]
fn hover_on_a_text_container_repaints_the_text() {
    partial_matches_full("p:hover", |pg| {
        let mut changed = false;
        let row = pg.rows[1];
        pg.d.update_hover(Some(row.0), &mut changed);
    });
    partial_matches_full("ancestor :hover", |pg| {
        pg.d.set_attribute(pg.w, "class", "boxhover");
        pg.d.resolve_layout(VP.0, VP.1);
        let mut changed = false;
        let row = pg.rows[3];
        pg.d.update_hover(Some(row.0), &mut changed);
    });
}
