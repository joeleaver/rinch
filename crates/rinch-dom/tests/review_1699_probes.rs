//! Reviewer probes for PR #1699 (#1696). Each prints what it measures and
//! asserts the Chrome 155 answer where one is stated.

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 300.0;
const VH: f32 = 300.0;

fn doc_from(html: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    doc.set_inner_html(body, html);
    doc.resolve_layout(VW, VH);
    doc
}

fn paint(doc: &mut RinchDocument) -> (TinySkiaPainter, u64) {
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    let before = doc
        .tree
        .perf
        .total()
        .get(rinch_dom::perf::Counter::PaintNodesVisited);
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    let after = doc
        .tree
        .perf
        .total()
        .get(rinch_dom::perf::Counter::PaintNodesVisited);
    (painter, after - before)
}

fn ink(p: &TinySkiaPainter, (x0, x1, y0, y1): (u32, u32, u32, u32)) -> usize {
    let d = p.pixels();
    let mut n = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let i = ((y * p.width() + x) * 4) as usize;
            if d[i + 3] > 0 {
                n += 1;
            }
        }
    }
    n
}

fn green(p: &TinySkiaPainter, (x0, x1, y0, y1): (u32, u32, u32, u32)) -> usize {
    let d = p.pixels();
    let mut n = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let i = ((y * p.width() + x) * 4) as usize;
            if d[i] == 0 && d[i + 1] == 128 && d[i + 2] == 0 && d[i + 3] == 255 {
                n += 1;
            }
        }
    }
    n
}

const ALL: (u32, u32, u32, u32) = (0, 300, 0, 300);

fn report(name: &str, doc: &mut RinchDocument) -> (usize, usize, u64) {
    let (p, visits) = paint(doc);
    let (i, g) = (ink(&p, ALL), green(&p, ALL));
    eprintln!("PROBE {name}: ink={i} green={g} visits={visits}");
    (i, g, visits)
}

const TXT: &str = "font-size: 16px; line-height: 20px; color: rgb(0,0,0)";

// --- text / inline content of a 0x0 box (Chrome paints it, overflowing) ---

#[test]
fn p01_static_zero_box_with_text() {
    let mut d = doc_from(&format!(
        "<div style=\"margin: 20px; width: 0; height: 0; {TXT}\">Hello</div>"
    ));
    let (i, _, _) = report("p01 static 0x0 text", &mut d);
    assert!(i > 20, "Chrome paints 'Hello' overflowing the 0x0 box");
}

#[test]
fn p02_abs_zero_sc_with_text() {
    let mut d = doc_from(&format!(
        "<div style=\"position: absolute; left: 20px; top: 20px; width: 0; height: 0; z-index: 2; {TXT}\">Hello</div>"
    ));
    let (i, _, _) = report("p02 abs 0x0 SC text", &mut d);
    assert!(i > 20, "Chrome paints the text");
}

#[test]
fn p03_abs_zero_sc_with_inline_block_child() {
    let mut d = doc_from(
        "<div style=\"position: absolute; left: 20px; top: 20px; width: 0; height: 0; z-index: 2\">\
         <span style=\"display: inline-block; width: 18px; height: 18px; background-color: rgb(0,128,0)\"></span></div>",
    );
    let (_, g, _) = report("p03 abs 0x0 SC > inline-block", &mut d);
    assert_eq!(g, 18 * 18, "Chrome paints the inline-block");
}

#[test]
fn p04_abs_zero_sc_with_inline_flex_badge_child() {
    let mut d = doc_from(&format!(
        "<div style=\"position: absolute; left: 20px; top: 20px; width: 0; height: 0; z-index: 2; {TXT}\">\
         <span style=\"display: inline-flex; width: 18px; height: 18px; background-color: rgb(0,128,0)\"></span></div>"
    ));
    let (_, g, _) = report("p04 abs 0x0 SC > inline-flex", &mut d);
    assert_eq!(g, 18 * 18, "Chrome paints the inline-flex");
}

#[test]
fn p05_height_zero_box_with_text_1d_preexisting() {
    let mut d = doc_from(&format!(
        "<div style=\"margin: 20px; width: 100px; height: 0; {TXT}\">Hello</div>"
    ));
    let (i, _, _) = report("p05 1D (height:0) text, pre-existing branch", &mut d);
    assert!(i > 20, "Chrome paints 'Hello' overflowing the height:0 box");
}

// --- flex / grid / inline-block 0x0 containers ---

#[test]
fn p06_zero_flex_container() {
    let mut d = doc_from(
        "<div style=\"margin: 20px; display: flex; width: 0; height: 0\">\
         <div style=\"flex: none; width: 30px; height: 30px; background-color: rgb(0,128,0)\"></div></div>",
    );
    let (_, g, _) = report("p06 0x0 flex", &mut d);
    assert_eq!(g, 900);
}

#[test]
fn p07_zero_grid_container() {
    let mut d = doc_from(
        "<div style=\"margin: 20px; display: grid; grid-template-columns: 30px; width: 0; height: 0\">\
         <div style=\"width: 30px; height: 30px; background-color: rgb(0,128,0)\"></div></div>",
    );
    let (_, g, _) = report("p07 0x0 grid", &mut d);
    assert_eq!(g, 900);
}

#[test]
fn p08_zero_inline_block_in_paragraph() {
    let mut d = doc_from(&format!(
        "<p style=\"margin: 20px; {TXT}\">a<span style=\"display: inline-block; width: 0; height: 0\">\
         <span style=\"position: absolute; width: 18px; height: 18px; background-color: rgb(0,128,0)\"></span>\
         <div style=\"width: 20px; height: 10px; background-color: rgb(0,128,0)\"></div></span>b</p>"
    ));
    let (_, g, _) = report("p08 0x0 inline-block (abs + in-flow child)", &mut d);
    assert_eq!(
        g,
        18 * 18 + 20 * 10 - 18 * 10,
        "Chrome paints both children (they overlap 18x10)"
    );
}

// --- clips ---

#[test]
fn p09_zero_box_overflow_hidden_rounded() {
    let mut d = doc_from(
        "<div style=\"position: relative; margin: 20px; width: 0; height: 0; overflow: hidden; border-radius: 10px\">\
         <div style=\"width: 30px; height: 30px; background-color: rgb(0,128,0)\"></div></div>",
    );
    let (i, _, _) = report("p09 0x0 overflow hidden radius", &mut d);
    assert_eq!(i, 0);
}

#[test]
fn p10_zero_box_overflow_visible_vs_hidden_1d() {
    let mut d = doc_from(
        "<div style=\"margin: 20px; width: 0; height: 0; overflow: visible\">\
         <div style=\"width: 30px; height: 30px; background-color: rgb(0,128,0)\"></div></div>",
    );
    let (_, g, _) = report("p10 0x0 overflow visible", &mut d);
    assert_eq!(g, 900);
}

#[test]
fn p11_zero_box_with_filter() {
    let mut d = doc_from(
        "<div style=\"margin: 20px; width: 0; height: 0; filter: brightness(0.5)\">\
         <div style=\"width: 30px; height: 30px; background-color: rgb(0,128,0)\"></div></div>",
    );
    let (_, g, _) = report(
        "p11 0x0 filter brightness(0.5) (Chrome: child darkened, green=0)",
        &mut d,
    );
    let _ = g;
}

#[test]
fn p11b_nonzero_box_with_filter_overflowing_child_preexisting() {
    let mut d = doc_from(
        "<div style=\"margin: 20px; width: 1px; height: 1px; filter: brightness(0.5)\">\
         <div style=\"width: 30px; height: 30px; background-color: rgb(0,128,0)\"></div></div>",
    );
    let (_, g, _) = report("p11b 1x1 filter (Chrome green≈0)", &mut d);
    let _ = g;
}

// --- visibility / display ---

#[test]
fn p12_hidden_zero_box_visible_descendant() {
    let mut d = doc_from(
        "<div style=\"margin: 20px; width: 0; height: 0; visibility: hidden\">\
         <div style=\"visibility: visible; width: 30px; height: 30px; background-color: rgb(0,128,0)\"></div></div>",
    );
    let (_, g, _) = report("p12 hidden 0x0 > visible child", &mut d);
    assert_eq!(g, 900, "Chrome paints a visibility: visible descendant");
}

#[test]
fn p13_zero_box_display_none_child() {
    let mut d = doc_from(
        "<div style=\"margin: 20px; width: 0; height: 0\">\
         <div style=\"display: none; width: 30px; height: 30px; background-color: rgb(0,128,0)\"></div></div>",
    );
    let (i, _, _) = report("p13 0x0 > display none", &mut d);
    assert_eq!(i, 0);
}

#[test]
fn p14_display_none_ancestor_of_zero_box() {
    let mut d = doc_from(
        "<div style=\"display: none\"><div style=\"width: 0; height: 0\">\
         <div style=\"width: 30px; height: 30px; background-color: rgb(0,128,0)\"></div></div></div>",
    );
    let (i, _, _) = report("p14 display:none > 0x0 > child", &mut d);
    assert_eq!(i, 0);
}

// --- replaced/control elements at 0x0 ---

#[test]
fn p15_zero_select_with_options() {
    let mut d = doc_from(&format!(
        "<select style=\"margin: 20px; width: 0; height: 0; border: 0; padding: 0; {TXT}\">\
         <option>Option one long label</option><option>two</option></select>"
    ));
    let (i, _, _) = report("p15 0x0 select (Chrome: nothing drawn)", &mut d);
    let _ = i;
}

#[test]
fn p16_zero_textarea_with_text() {
    let mut d = doc_from(&format!(
        "<textarea style=\"margin: 20px; width: 0; height: 0; border: 0; padding: 0; {TXT}\">Hello there</textarea>"
    ));
    let (i, _, _) = report("p16 0x0 textarea (Chrome: nothing)", &mut d);
    let _ = i;
}

#[test]
fn p17_zero_viewport_with_children() {
    let mut d = doc_from(
        "<div data-viewport=\"x\" style=\"margin: 20px; width: 0; height: 0\">\
         <div style=\"width: 30px; height: 30px; background-color: rgb(0,128,0)\"></div></div>",
    );
    let (_, g, _) = report("p17 0x0 data-viewport > child", &mut d);
    let _ = g;
}

// --- perf: 0x0 boxes holding big subtrees ---

fn rows(n: usize, inner: &str) -> String {
    let mut s = String::new();
    for _ in 0..n {
        s.push_str(inner);
    }
    s
}

#[test]
fn p18_perf_zero_overflow_hidden_with_big_subtree() {
    let inner = rows(
        500,
        "<div style=\"height: 20px; background-color: rgb(0,128,0)\"></div>",
    );
    let mut d = doc_from(&format!(
        "<div style=\"width: 0; height: 0; overflow: hidden\">{inner}</div>"
    ));
    let (i, _, v) = report("p18 0x0 overflow:hidden > 500 rows", &mut d);
    assert_eq!(i, 0);
    let _ = v;
}

#[test]
fn p19_perf_many_zero_wrappers_with_hidden_children() {
    let inner = rows(
        300,
        "<div style=\"width: 0; height: 0\"><div style=\"display: none\"><span>x</span><span>y</span></div><div style=\"display: none\"></div></div>",
    );
    let mut d = doc_from(&inner);
    let (i, _, v) = report("p19 300 0x0 wrappers each > 2 display:none", &mut d);
    assert_eq!(i, 0);
    let _ = v;
}

#[test]
fn p20_perf_zero_box_offscreen_big_subtree() {
    let inner = rows(
        500,
        "<div style=\"height: 20px; width: 30px; background-color: rgb(0,128,0)\"></div>",
    );
    let mut d = doc_from(&format!(
        "<div style=\"position: absolute; top: 1000px; left: 0; width: 0; height: 0; z-index: 1\">{inner}</div>"
    ));
    let (i, _, v) = report("p20 off-screen 0x0 SC > 500 rows", &mut d);
    assert_eq!(i, 0);
    let _ = v;
}

#[test]
fn p15b_zero_select_overflow_visible() {
    let mut d = doc_from(&format!(
        "<select style=\"margin: 20px; width: 0; height: 0; border: 0; padding: 0; overflow: visible; {TXT}\">\
         <option>Option one long label</option><option>two</option></select>"
    ));
    let (i, _, _) = report("p15b 0x0 select overflow visible (Chrome: nothing)", &mut d);
    assert_eq!(i, 0);
}

#[test]
fn p21_zero_svg_sprite_sheet() {
    let mut d = doc_from(
        "<svg width=\"0\" height=\"0\" style=\"position: absolute\"><rect width=\"20\" height=\"20\" fill=\"rgb(0,128,0)\" style=\"background-color: rgb(0,128,0)\"></rect><g><rect width=\"20\" height=\"20\" fill=\"green\"></rect></g></svg>",
    );
    let (i, _, _) = report("p21 0x0 svg sprite (Chrome: nothing)", &mut d);
    assert_eq!(i, 0);
}

#[test]
fn p22_zero_button_with_label() {
    let mut d = doc_from(&format!(
        "<button style=\"margin: 20px; width: 0; height: 0; border: 0; padding: 0; {TXT}\">Click</button>"
    ));
    let (i, _, _) = report("p22 0x0 button label (Chrome 155: 299 ink px)", &mut d);
    assert!(i > 0, "the label overflows the 0x0 button and is drawn");
}

#[test]
fn p18b_perf_zero_overflow_hidden_with_sized_rows() {
    let inner = rows(
        500,
        "<div style=\"width: 30px; height: 20px; background-color: rgb(0,128,0)\"></div>",
    );
    let mut d = doc_from(&format!(
        "<div style=\"width: 0; height: 0; overflow: hidden\">{inner}</div>"
    ));
    let (i, _, v) = report("p18b 0x0 overflow:hidden > 500 30px rows", &mut d);
    assert_eq!(i, 0);
    let _ = v;
}

#[test]
fn p18c_perf_height_zero_overflow_hidden_rows_1d_preexisting() {
    let inner = rows(
        500,
        "<div style=\"height: 20px; background-color: rgb(0,128,0)\"></div>",
    );
    let mut d = doc_from(&format!(
        "<div style=\"width: 100px; height: 0; overflow: hidden\">{inner}</div>"
    ));
    let (i, _, v) = report(
        "p18c 1D height:0 overflow:hidden > 500 rows (main's cost)",
        &mut d,
    );
    assert_eq!(i, 0);
    let _ = v;
}
