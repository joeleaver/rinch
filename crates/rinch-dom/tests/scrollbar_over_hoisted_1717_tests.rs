//! An overlay scrollbar is drawn above the scroller's own positioned
//! descendants (#1717).
//!
//! A scroller that is no stacking context paints its bars at the end of its
//! own `paint_node`. Its positioned descendants (`position: relative`, a
//! `z-index`, an absolute whose containing block is inside it) are not painted
//! there: they are hoisted to the nearest stacking context's sequence and
//! painted after the scroller, so they used to cover its thumb. An app's tree
//! view with `position: relative` rows lost the left of its thumb under the
//! selected row.
//!
//! Chrome 155 (overlay scrollbars, measured with mobile emulation) draws the
//! bars after the last entry of that sequence the scroller *contains* — whose
//! containing block chain runs through the scroller, so the scroller clips it
//! — and before everything else:
//!
//! | case | Chrome 155 |
//! |---|---|
//! | `relative` rows, `z-index: auto` | bar over the rows |
//! | rows `z-index: 5` | bar over the rows |
//! | rows `z-index: -1` | bar over the rows (they are behind the scroller) |
//! | static scroller, `relative` rows | bar over the rows |
//! | absolute in a relative wrapper inside the scroller | bar over it |
//! | a positioned box nested in a non-clipping positioned wrapper | bar over it |
//! | a later positioned sibling of the scroller | **covers** the bar |
//! | an absolute inside a static scroller, containing block outside | **covers** the bar |
//! | rows `z-index: 5`, an outside sibling at `z-index: 3` over the bar | bar over the sibling |
//! | rows `z-index: auto`, an outside sibling at `z-index: 1` | **covers** the bar |
//!
//! The oracle for "over" is a twin document whose rows are not positioned:
//! nothing is hoisted there, the bars are drawn last by the scroller itself,
//! and the strip the bar sits in must be the same pixel for pixel. Every
//! scroller is scrolled off 0 (the thumb then sits where scroll 0 would not
//! put it) and its rows reach under the bar (no right padding).

#![cfg(feature = "software-renderer")]

use peniko::kurbo::{Affine, Rect};
use peniko::{Brush, Fill};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::{PaintShape, Painter};
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 600.0;
const VH: f32 = 400.0;
const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

fn child(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", style);
    doc.append_child(parent, d);
    d
}

/// Scroller styles: 200x150 at (20, 20), rows 40px tall and red, 6 of them.
const SCROLLER: &str = "position: absolute; left: 20px; top: 20px; width: 200px; \
                        height: 150px; overflow-y: auto; background-color: rgb(238, 238, 238)";

/// The bar strip of the scroller at (20, 20): x in [210, 220), its full height.
const STRIP: Rect = Rect::new(210.0, 20.0, 220.0, 170.0);

fn build(
    scroller_css: &str,
    row_css: &str,
    extra: impl FnOnce(&mut RinchDocument, NodeId, NodeId),
) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let s = child(&mut doc, body, scroller_css);
    for _ in 0..6 {
        child(
            &mut doc,
            s,
            &format!("height: 40px; background-color: rgb(255, 0, 0); {row_css}"),
        );
    }
    extra(&mut doc, body, s);
    doc.resolve_layout(VW, VH);
    doc.tree.nodes[s.0].scroll_offset = (0.0, 23.0);
    doc.tree.hit_cache.invalidate();
    (doc, s)
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

/// The thumb's centre, in logical px, for the scroller at its scroll offset.
fn thumb_centre(doc: &RinchDocument, s: NodeId) -> (u32, u32) {
    let bars = rinch_dom::paint::scrollbar::scrollbars(&doc.tree, s.0, 1.0);
    let t = bars.vertical.expect("the scroller has a vertical bar");
    let n = doc.tree.get(s.0).unwrap();
    let top = 20.0 + t.thumb_start(n.scroll_offset.1);
    let x = 20.0 + 200.0 - bars.thickness - bars.margin + bars.thickness / 2.0;
    (x as u32, (top + t.thumb_len / 2.0) as u32)
}

fn first_difference(
    a: &TinySkiaPainter,
    b: &TinySkiaPainter,
    region: Rect,
    scale: f64,
) -> Option<(u32, u32, [u8; 4], [u8; 4])> {
    for y in (region.y0 * scale) as u32..(region.y1 * scale) as u32 {
        for x in (region.x0 * scale) as u32..(region.x1 * scale) as u32 {
            let (pa, pb) = (px(a, x, y), px(b, x, y));
            if pa != pb {
                return Some((x, y, pa, pb));
            }
        }
    }
    None
}

/// The strip of `positioned` must equal the strip of the static-rows twin, and
/// the thumb's centre must not be the bare row colour.
fn assert_bar_on_top(
    positioned: (RinchDocument, NodeId),
    twin: (RinchDocument, NodeId),
    what: &str,
) {
    let (mut a, s) = positioned;
    let (mut b, _) = twin;
    let pa = paint(&mut a);
    let pb = paint(&mut b);
    let (cx, cy) = thumb_centre(&a, s);
    assert_ne!(
        px(&pa, cx, cy),
        RED,
        "{what}: the thumb at ({cx}, {cy}) is covered by a row"
    );
    assert_eq!(
        first_difference(&pa, &pb, STRIP, 1.0),
        None,
        "{what}: the bar strip differs from the static-rows twin (x, y, positioned, twin)"
    );
}

fn no_extra(_: &mut RinchDocument, _: NodeId, _: NodeId) {}

#[test]
fn the_bar_is_drawn_over_relative_rows() {
    assert_bar_on_top(
        build(SCROLLER, "position: relative", no_extra),
        build(SCROLLER, "", no_extra),
        "relative rows",
    );
}

#[test]
fn the_bar_is_drawn_over_rows_with_a_positive_z_index() {
    assert_bar_on_top(
        build(SCROLLER, "position: relative; z-index: 5", no_extra),
        build(SCROLLER, "", no_extra),
        "z-index: 5 rows",
    );
}

/// A static scroller: an in-flow entry of the body's sequence, its rows hoisted
/// to the z = 0 level after it.
#[test]
fn the_bar_of_a_static_scroller_is_drawn_over_its_relative_rows() {
    let css = "margin: 20px 0 0 20px; width: 200px; height: 150px; overflow-y: auto; \
               background-color: rgb(238, 238, 238)";
    assert_bar_on_top(
        build(css, "position: relative", no_extra),
        build(css, "", no_extra),
        "static scroller",
    );
}

/// The issue's own shape: rows reaching under the bar's left half only (4px of
/// right padding against a 6px bar with a 2px margin), one row selected.
#[test]
fn the_selected_row_does_not_cover_the_left_of_the_thumb() {
    let css = format!("{SCROLLER}; padding-right: 4px; box-sizing: border-box");
    let mk = |pos: &str| {
        build(&css, pos, |doc, _, s| {
            let rows = doc.get_children(s);
            doc.set_style(rows[1], "background-color", "rgb(0, 0, 255)");
        })
    };
    let (mut a, s) = mk("position: relative");
    let (mut b, _) = mk("");
    let pa = paint(&mut a);
    let pb = paint(&mut b);
    let (_, cy) = thumb_centre(&a, s);
    // The thumb's left column (x 212) is over the rows' 4px (they end at 216).
    assert!(
        matches!(px(&pa, 212, cy), p if p != RED && p != BLUE),
        "{:?}",
        px(&pa, 212, cy)
    );
    assert_eq!(first_difference(&pa, &pb, STRIP, 1.0), None);
}

/// An absolute box whose containing block is a positioned wrapper inside the
/// scroller is contained by it, and the bar is drawn over it.
///
/// The box ends at the scroller's right edge (140 + 60 = 200), under the bar.
/// Past it, it would overflow the scroller since #770 — a browser scrolls to
/// it — and add a horizontal bar that its in-flow twin (an in-flow grandchild
/// overflowing its parent, which rinch does not count yet, #2054) does not.
#[test]
fn the_bar_is_drawn_over_a_contained_absolute() {
    let mk = |abs: bool| {
        build(SCROLLER, "", move |doc, _, s| {
            let rows = doc.get_children(s);
            doc.set_style(rows[1], "position", "relative");
            let pos = if abs {
                "position: absolute; left: 140px; top: 0"
            } else {
                "margin-left: 140px"
            };
            child(
                doc,
                rows[1],
                &format!("{pos}; width: 60px; height: 30px; background-color: rgb(0, 0, 255)"),
            );
        })
    };
    // The twin draws the same blue box in flow, under the bar.
    let (mut a, s) = mk(true);
    let (mut b, _) = mk(false);
    let pa = paint(&mut a);
    let pb = paint(&mut b);
    // The wrapper row sits at content y 40, so on screen 20 + 40 - 23 = 37.
    let (cx, _) = thumb_centre(&a, s);
    assert_ne!(
        px(&pa, cx, 50),
        BLUE,
        "the thumb under the contained absolute is covered"
    );
    assert_eq!(first_difference(&pa, &pb, STRIP, 1.0), None);
}

/// A later positioned sibling of the scroller is not its content: Chrome draws
/// it over the bar, and so does rinch.
#[test]
fn a_later_positioned_sibling_still_covers_the_bar() {
    let (mut doc, s) = build(SCROLLER, "position: relative", |doc, body, _| {
        child(
            doc,
            body,
            "position: absolute; left: 200px; top: 20px; width: 40px; height: 150px; \
             background-color: rgb(0, 0, 255)",
        );
    });
    let p = paint(&mut doc);
    let (cx, cy) = thumb_centre(&doc, s);
    assert_eq!(px(&p, cx, cy), BLUE);
}

/// An absolute inside a **static** scroller whose containing block is the body
/// escapes the scroller: it is not its content, and covers the bar in Chrome.
#[test]
fn an_absolute_escaping_a_static_scroller_still_covers_the_bar() {
    let css = "margin: 20px 0 0 20px; width: 200px; height: 150px; overflow-y: auto; \
               background-color: rgb(238, 238, 238)";
    let (mut doc, s) = build(css, "position: relative", |doc, _, s| {
        let rows = doc.get_children(s);
        // The last row is static, so the body is the absolute's containing
        // block; it is hoisted after every relative row, so a bar placed
        // after the scroller's last *descendant* entry (rather than its last
        // *contained* one) would be drawn over it.
        doc.set_style(rows[5], "position", "static");
        child(
            doc,
            rows[5],
            "position: absolute; left: 200px; top: 20px; width: 40px; height: 150px; \
             background-color: rgb(0, 0, 255)",
        );
    });
    let p = paint(&mut doc);
    let (cx, cy) = thumb_centre(&doc, s);
    assert_eq!(px(&p, cx, cy), BLUE);
}

/// Rows at `z-index: 5` put the bar after them, so an outside box at
/// `z-index: 3` sits under the bar (Chrome 155); one at `z-index: 1` over rows
/// at `z-index: auto` covers it.
#[test]
fn the_bar_takes_the_place_of_its_last_contained_entry() {
    let sibling = |z: i32| {
        move |doc: &mut RinchDocument, body: NodeId, _: NodeId| {
            child(
                doc,
                body,
                &format!(
                    "position: absolute; left: 200px; top: 20px; width: 40px; height: 150px; \
                     z-index: {z}; background-color: rgb(0, 0, 255)"
                ),
            );
        }
    };
    let (mut over, s) = build(SCROLLER, "position: relative; z-index: 5", sibling(3));
    let p = paint(&mut over);
    let (cx, cy) = thumb_centre(&over, s);
    assert_ne!(
        px(&p, cx, cy),
        BLUE,
        "rows at z 5: the bar is above a z 3 sibling"
    );
    assert_ne!(px(&p, cx, cy), RED);

    let (mut under, s) = build(SCROLLER, "position: relative", sibling(1));
    let p = paint(&mut under);
    let (cx, cy) = thumb_centre(&under, s);
    assert_eq!(
        px(&p, cx, cy),
        BLUE,
        "rows at z auto: a z 1 sibling covers the bar"
    );
}

/// Two scrollers nested, the inner one positioned (hoisted) and holding
/// relative rows: both bars are over the rows, the outer one over the inner
/// one's.
#[test]
fn nested_scrollers_both_draw_over_their_rows() {
    let mk = |pos: &'static str| {
        build(SCROLLER, "", move |doc, _, s| {
            let rows = doc.get_children(s);
            for r in rows {
                doc.remove_child(s, r);
            }
            let inner = child(
                doc,
                s,
                &format!("{pos}; width: 200px; height: 120px; overflow-y: auto"),
            );
            for _ in 0..6 {
                child(
                    doc,
                    inner,
                    &format!("height: 40px; background-color: rgb(255, 0, 0); {pos}"),
                );
            }
            child(doc, s, "height: 200px");
        })
    };
    let (mut a, s) = mk("position: relative");
    let (mut b, _) = mk("");
    let inner = a.get_children(s)[0];
    a.tree.nodes[inner.0].scroll_offset = (0.0, 31.0);
    let inner_b = b.get_children(s)[0];
    b.tree.nodes[inner_b.0].scroll_offset = (0.0, 31.0);
    let pa = paint(&mut a);
    let pb = paint(&mut b);
    let (cx, cy) = thumb_centre(&a, s);
    assert_ne!(px(&pa, cx, cy), RED);
    assert_eq!(first_difference(&pa, &pb, STRIP, 1.0), None);
}

/// At scale 2 the deferred bar is placed by the same device-space origin.
#[test]
fn the_bar_is_drawn_over_relative_rows_at_scale_two() {
    let (mut a, _) = build(SCROLLER, "position: relative", no_extra);
    let (mut b, _) = build(SCROLLER, "", no_extra);
    let pa = paint_at(&mut a, 2.0, None);
    let pb = paint_at(&mut b, 2.0, None);
    assert_eq!(first_difference(&pa, &pb, STRIP, 2.0), None);
}

/// A partial repaint over one row (a selection change) redraws the bar on top
/// of it, as the full frame does (#1722 hands the GPU window the same damage).
#[test]
fn a_partial_repaint_over_a_row_keeps_the_bar_on_top() {
    let (mut a, _) = build(SCROLLER, "position: relative", no_extra);
    let full = paint(&mut a);
    let damage = Rect::new(20.0, 57.0, 220.0, 97.0);
    let part = paint_at(&mut a, 1.0, Some(damage));
    assert_eq!(first_difference(&part, &full, damage, 1.0), None);
    let (mut b, _) = build(SCROLLER, "", no_extra);
    let twin = paint(&mut b);
    assert_eq!(
        first_difference(&part, &twin, damage.intersect(STRIP), 1.0),
        None
    );
}

/// The command stream both painters are handed: the thumb's fill comes after
/// every row's. (Vello records exactly these calls; tiny-skia is pinned by the
/// pixel fixtures above.)
#[test]
fn the_thumb_is_the_last_fill_in_the_command_stream() {
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
    let (mut doc, _) = build(SCROLLER, "position: relative", no_extra);
    let mut r = Recorder::default();
    let mut lcx = parley::LayoutContext::<Brush>::new();
    rinch_dom::paint::paint_document(&doc.tree, &mut r, 1.0, (VW, VH), &mut doc.font_cx, &mut lcx);
    let last = r.fills.last().expect("something was filled");
    assert!(
        (last.x0 - 212.0).abs() < 0.01 && (last.x1 - 218.0).abs() < 0.01,
        "the last fill is not the thumb: {:?}",
        r.fills
    );
}

/// Rows at `z-index: -1` are drawn behind the scroller's own background, and
/// the bar with the scroller (Chrome 155): a negative-`z` entry is not one the
/// bar waits for, or it would be drawn before the background that covers it.
#[test]
fn rows_at_a_negative_z_index_do_not_defer_the_bar() {
    let (mut a, s) = build(SCROLLER, "position: relative; z-index: -1", no_extra);
    // The twin: the same content height, with rows that draw nothing.
    let (mut b, _) = build(SCROLLER, "background-color: transparent", no_extra);
    let pa = paint(&mut a);
    let pb = paint(&mut b);
    let (cx, cy) = thumb_centre(&a, s);
    assert_ne!(px(&pa, cx, cy), [238, 238, 238, 255], "the thumb is drawn");
    assert_eq!(first_difference(&pa, &pb, STRIP, 1.0), None);
}
