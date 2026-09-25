//! The review fixtures of #904 (PR #969): an `inline-flex` / `inline-grid`
//! label painted from its cached layout against paint's on-demand fallback
//! (byte-identical at scale 1, 1.5 and 2), against a fresh document across a
//! theme change, a move and a display flip, plus `#[ignore]`d release-mode
//! benches of a colour hover / colour transition over 500 rows.
//!
//! One case is deliberately absent: `text-overflow: ellipsis` on an
//! `inline-flex`'s own text. The cached layout carries the "…" that
//! `copy_cached_text_layouts` inserts — what a block-level `flex` already did —
//! where the old fallback clipped, so the two paths differ by design there.

#![cfg(feature = "software-renderer")]
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VP: (f32, f32) = (800.0, 600.0);
const BASE_CSS: &str = "
    body { margin: 0; font-family: sans-serif; font-size: 16px; line-height: 20px;
           color: rgb(10, 20, 30); }
";

fn settle(doc: &mut RinchDocument) {
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
}

fn paint_at(doc: &mut RinchDocument, scale: f32) -> Vec<[u8; 4]> {
    let mut painter = rinch_dom::paint::skia_painter::TinySkiaPainter::new(
        (VP.0 * scale) as u32,
        (VP.1 * scale) as u32,
    );
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale as f64,
        VP,
        &mut doc.font_cx,
        &mut doc.layout_cx,
    );
    painter.pixels().as_chunks::<4>().0.to_vec()
}

fn red(px: &[[u8; 4]]) -> usize {
    px.iter()
        .filter(|p| p[3] > 0 && p[0] as i32 > p[1] as i32 + 80)
        .count()
}
fn ink(px: &[[u8; 4]]) -> usize {
    px.iter()
        .filter(|p| p[3] > 0 && (p[0] < 200 || p[1] < 200))
        .count()
}

fn chip(doc: &mut RinchDocument, css: &str, class: &str, label: &str) -> (NodeId, NodeId) {
    doc.load_css(BASE_CSS);
    doc.load_css(css);
    let body = doc.body();
    let p = doc.create_element("div");
    let c = doc.create_element("span");
    doc.set_attribute(c, "class", class);
    let t = doc.create_text(label);
    doc.append_child(c, t);
    doc.append_child(p, c);
    doc.append_child(body, p);
    (c, t)
}

fn cached_vs_fallback(css: &str, class: &str, label: &str, scale: f32) {
    let mut d = RinchDocument::new();
    let (c, t) = chip(&mut d, css, class, label);
    settle(&mut d);
    let lay = d.tree.get(c.0).unwrap().layout;
    let cached_lines = d
        .tree
        .get(t.0)
        .unwrap()
        .cached_text_parley
        .as_ref()
        .map(|l| (l.len(), l.width(), l.height()));
    let a = paint_at(&mut d, scale);
    d.tree.nodes[t.0].cached_text_parley = None;
    let b = paint_at(&mut d, scale);
    let diff = a.iter().zip(&b).filter(|(x, y)| x != y).count();
    eprintln!(
        "[{class} @{scale}] box {}x{} cached={cached_lines:?} ink cached={} fallback={} diffpx={diff}",
        lay.width,
        lay.height,
        ink(&a),
        ink(&b)
    );
    assert!(diff == 0, "cached vs fallback differ: {diff}px");
}

const CSS: &str = ".ic { display: inline-flex; padding: 2px; color: rgb(220,0,0); }
    .ic.narrow { max-width: 60px; }
    .ic.w { width: 60px; }
    .ic.ell { max-width: 60px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .ic.ls { letter-spacing: 2px; word-spacing: 3px; font-weight: 700; }
    .ic.it { font-style: italic; text-decoration: underline; text-transform: uppercase; }
";

#[test]
fn p3_plain_1() {
    cached_vs_fallback(CSS, "ic", "chip label text", 1.0);
}
#[test]
fn p3_plain_15() {
    cached_vs_fallback(CSS, "ic", "chip label text", 1.5);
}
#[test]
fn p3_plain_2() {
    cached_vs_fallback(CSS, "ic", "chip label text", 2.0);
}
#[test]
fn p2_narrow_1() {
    cached_vs_fallback(CSS, "ic narrow", "a long chip label text that wraps", 1.0);
}
#[test]
fn p2_narrow_15() {
    cached_vs_fallback(CSS, "ic narrow", "a long chip label text that wraps", 1.5);
}
#[test]
fn p2_width_1() {
    cached_vs_fallback(CSS, "ic w", "a long chip label text that wraps", 1.0);
}
#[test]
fn p2_ls_1() {
    cached_vs_fallback(CSS, "ic ls", "chip label text", 1.0);
}
#[test]
fn p2_it_1() {
    cached_vs_fallback(CSS, "ic it", "chip label text", 1.0);
}

// P1: theme colour toggle via #967's full restyle keep path.
fn theme_case(disp: &str) {
    let css = format!(".ic {{ display: {disp}; color: var(--tc); }}");
    let make = |theme: &str| {
        let mut d = RinchDocument::new();
        d.set_theme_css(theme);
        let (_c, _t) = chip(&mut d, &css, "ic", "chip label text");
        settle(&mut d);
        d
    };
    let a = ":root { --tc: rgb(10, 20, 30); }";
    let b = ":root { --tc: rgb(220, 0, 0); }";
    let mut d = make(a);
    assert_eq!(red(&paint_at(&mut d, 1.0)), 0);
    d.update_theme_variables(b);
    d.recompute_all_styles_full();
    d.resolve_layout(VP.0, VP.1);
    let inc = paint_at(&mut d, 1.0);
    let mut f = make(b);
    let fr = paint_at(&mut f, 1.0);
    eprintln!("[theme {disp}] red inc={} fresh={}", red(&inc), red(&fr));
    assert!(red(&fr) > 0);
    assert!(inc == fr, "theme toggle: incremental pixels differ");
}
#[test]
fn p1_theme_inline_flex() {
    theme_case("inline-flex");
}
#[test]
fn p1_theme_flex() {
    theme_case("flex");
}
#[test]
fn p1_theme_inline_grid() {
    theme_case("inline-grid");
}

// P5: move a label between two inline-flex parents of different colours.
#[test]
fn p5_move_label() {
    let css = ".a { display: inline-flex; color: rgb(10,20,30); } .b { display: inline-flex; color: rgb(220,0,0); }";
    let build = |d: &mut RinchDocument, in_b: bool| {
        d.load_css(BASE_CSS);
        d.load_css(css);
        let body = d.body();
        let a = d.create_element("span");
        d.set_attribute(a, "class", "a");
        let b = d.create_element("span");
        d.set_attribute(b, "class", "b");
        let ta = d.create_text("x");
        d.append_child(a, ta);
        let tb = d.create_text("y");
        d.append_child(b, tb);
        let t = d.create_text("moved label");
        if in_b {
            d.append_child(b, t)
        } else {
            d.append_child(a, t)
        }
        d.append_child(body, a);
        d.append_child(body, b);
        (b, t)
    };
    let mut d = RinchDocument::new();
    let (b, t) = build(&mut d, false);
    settle(&mut d);
    let _ = paint_at(&mut d, 1.0);
    d.append_child(b, t);
    d.resolve_layout(VP.0, VP.1);
    let inc = paint_at(&mut d, 1.0);
    let mut f = RinchDocument::new();
    let _ = build(&mut f, true);
    settle(&mut f);
    let fr = paint_at(&mut f, 1.0);
    eprintln!("[move] red inc={} fresh={}", red(&inc), red(&fr));
    assert!(inc == fr);
}

// P6: display flip inline-flex -> block -> inline-flex with colour changed while block.
#[test]
fn p6_display_flip() {
    let css = ".ic { display: inline-flex; } .ic.blk { display: block; } .ic.hot { color: rgb(220,0,0); }";
    let mut d = RinchDocument::new();
    let (c, _t) = chip(&mut d, css, "ic", "flip label");
    settle(&mut d);
    let _ = paint_at(&mut d, 1.0);
    d.set_attribute(c, "class", "ic blk");
    d.resolve_layout(VP.0, VP.1);
    let _ = paint_at(&mut d, 1.0);
    d.set_attribute(c, "class", "ic blk hot");
    d.resolve_layout(VP.0, VP.1);
    let _ = paint_at(&mut d, 1.0);
    d.set_attribute(c, "class", "ic hot");
    d.resolve_layout(VP.0, VP.1);
    let inc = paint_at(&mut d, 1.0);
    let mut f = RinchDocument::new();
    let _ = chip(&mut f, css, "ic hot", "flip label");
    settle(&mut f);
    let fr = paint_at(&mut f, 1.0);
    eprintln!("[flip] red inc={} fresh={}", red(&inc), red(&fr));
    assert!(inc == fr);
}

// P10: mid-transition frames reach the label (not only the finished one).
#[test]
fn p10_mid_transition_frame() {
    let css = ".ic { display: inline-flex; transition: color 1000ms linear; color: rgb(0,0,0); } .ic.hot { color: rgb(240,0,0); }";
    let mut d = RinchDocument::new();
    let (c, t) = chip(&mut d, css, "ic", "mid label text");
    settle(&mut d);
    let _ = paint_at(&mut d, 1.0);
    d.set_attribute(c, "class", "ic hot");
    d.resolve_layout(VP.0, VP.1);
    let _ = paint_at(&mut d, 1.0);
    for tr in d
        .tree
        .active_transitions
        .get_mut(&c.0)
        .unwrap()
        .values_mut()
    {
        tr.start_time_ms -= 500.0;
    }
    d.tick_transitions();
    d.resolve_layout(VP.0, VP.1);
    let brush = d
        .tree
        .get(t.0)
        .unwrap()
        .cached_text_parley
        .as_ref()
        .map(|l| {
            let mut v = vec![];
            for line in l.lines() {
                for it in line.items() {
                    if let parley::layout::PositionedLayoutItem::GlyphRun(g) = it {
                        v.push(format!("{:?}", g.style().brush));
                    }
                }
            }
            v
        });
    let col = d.tree.get(c.0).unwrap().computed_style.color;
    eprintln!("[mid] style={col:?} leaf brush={brush:?}");
}

fn bench_doc(disp: &str, n: usize, trans: bool) -> (RinchDocument, Vec<NodeId>) {
    let mut d = RinchDocument::new();
    d.load_css(BASE_CSS);
    let t = if trans {
        "transition: color 150ms linear;"
    } else {
        ""
    };
    d.load_css(&format!(".list {{ display: flex; flex-direction: column; }} .row {{ display: {disp}; padding: 2px; {t} }} .row.hot {{ color: rgb(220,0,0); }}"));
    let body = d.body();
    let list = d.create_element("div");
    d.set_attribute(list, "class", "list");
    let mut rows = vec![];
    for i in 0..n {
        let r = d.create_element("div");
        d.set_attribute(r, "class", "row");
        let tx = d.create_text(&format!("row {i} label text here"));
        d.append_child(r, tx);
        d.append_child(list, r);
        rows.push(r);
    }
    d.append_child(body, list);
    settle(&mut d);
    (d, rows)
}

fn hover_bench(disp: &str) {
    let (mut d, rows) = bench_doc(disp, 500, false);
    let r = rows[250];
    let mut best = f64::MAX;
    let c0 = d.tree.taffy_computes;
    for k in 0..200 {
        d.set_attribute(r, "class", if k % 2 == 0 { "row hot" } else { "row" });
        let t = std::time::Instant::now();
        d.resolve_layout(VP.0, VP.1);
        best = best.min(t.elapsed().as_secs_f64() * 1e6);
    }
    eprintln!(
        "[bench hover {disp}] best {best:.1}us computes/200={}",
        d.tree.taffy_computes - c0
    );
}
#[test]
#[ignore]
fn bench_hover_flex() {
    hover_bench("flex");
}
#[test]
#[ignore]
fn bench_hover_iflex() {
    hover_bench("inline-flex");
}
#[test]
#[ignore]
fn bench_hover_block() {
    hover_bench("block");
}

fn trans_bench(disp: &str) {
    let (mut d, rows) = bench_doc(disp, 500, true);
    let r = rows[250];
    d.set_attribute(r, "class", "row hot");
    d.resolve_layout(VP.0, VP.1);
    let c0 = d.tree.taffy_computes;
    let mut best = f64::MAX;
    let mut sum = 0.0;
    for tr in d
        .tree
        .active_transitions
        .get_mut(&r.0)
        .unwrap()
        .values_mut()
    {
        tr.start_time_ms -= 1.0;
        tr.duration_ms = 1.0e12;
    }
    for _ in 0..100 {
        let t = std::time::Instant::now();
        d.tick_transitions();
        d.resolve_layout(VP.0, VP.1);
        let e = t.elapsed().as_secs_f64() * 1e6;
        best = best.min(e);
        sum += e;
    }
    eprintln!(
        "[bench trans {disp}] best {best:.1}us mean {:.1}us computes/100={}",
        sum / 100.0,
        d.tree.taffy_computes - c0
    );
}
#[test]
#[ignore]
fn bench_trans_flex() {
    trans_bench("flex");
}
#[test]
#[ignore]
fn bench_trans_iflex() {
    trans_bench("inline-flex");
}
#[test]
#[ignore]
fn bench_trans_block() {
    trans_bench("block");
}

#[test]
#[ignore]
fn bench_hover_span_label() {
    let mut d = RinchDocument::new();
    d.load_css(BASE_CSS);
    d.load_css(".list { display: flex; flex-direction: column; } .row { display: flex; padding: 2px; } .row.hot { color: rgb(220,0,0); }");
    let body = d.body();
    let list = d.create_element("div");
    d.set_attribute(list, "class", "list");
    let mut rows = vec![];
    for i in 0..500 {
        let r = d.create_element("div");
        d.set_attribute(r, "class", "row");
        let s = d.create_element("span");
        let tx = d.create_text(&format!("row {i} label"));
        d.append_child(s, tx);
        d.append_child(r, s);
        d.append_child(list, r);
        rows.push((r, tx));
    }
    d.append_child(body, list);
    settle(&mut d);
    let (r, tx) = rows[250];
    eprintln!("leaf? ifc_root={:?}", d.tree.get(tx.0).unwrap().ifc_root);
    let c0 = d.tree.taffy_computes;
    let mut best = f64::MAX;
    for k in 0..200 {
        d.set_attribute(r, "class", if k % 2 == 0 { "row hot" } else { "row" });
        let t = std::time::Instant::now();
        d.resolve_layout(VP.0, VP.1);
        best = best.min(t.elapsed().as_secs_f64() * 1e6);
    }
    eprintln!(
        "[bench span-label hover] best {best:.1}us computes={}",
        d.tree.taffy_computes - c0
    );
}
