//! #2719 review: the kept backdrops (`paint::raster_cache`,
//! `paint::backdrop::invalidate`) never show a stale picture. Each probe
//! paints twice (warm), applies a change, paints again, and compares with a
//! fresh document painted in the same state, every frame in the shell's
//! order (the change lists drained, then the paint). Adopted from the
//! review's probes.
#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const W: f32 = 360.0;
const H: f32 = 260.0;

fn part<'a>(page: &'a str, open: &str, close: &str) -> &'a str {
    let start = page.find(open).unwrap() + open.len();
    &page[start..start + page[start..].find(close).unwrap()]
}

fn document(page: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css(part(page, "<style>", "</style>"));
    let body = doc.body();
    doc.set_inner_html(body, part(page, "<body>", "</body>"));
    doc.resolve_layout(W, H);
    doc
}

fn by_id(doc: &RinchDocument, id: &str) -> rinch_core::dom::NodeId {
    doc.query_selector_all(&format!("#{id}"))[0]
}

/// One full frame in the shell's order: the change lists drained, then the
/// paint.
fn frame(doc: &mut RinchDocument, scale: f64) -> Vec<u8> {
    doc.tree.consume_paint_dirty();
    let (pw, ph) = ((W as f64 * scale) as u32, (H as f64 * scale) as u32);
    let mut p = TinySkiaPainter::new(pw, ph);
    p.fill_white();
    let mut lcx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(&doc.tree, &mut p, scale, (W, H), &mut doc.font_cx, &mut lcx);
    p.pixels().to_vec()
}

fn diff(a: &[u8], b: &[u8]) -> usize {
    a.chunks(4)
        .zip(b.chunks(4))
        .filter(|(x, y)| x.iter().zip(y.iter()).any(|(p, q)| p.abs_diff(*q) > 1))
        .count()
}

/// Paint twice (warm), apply `change`, paint, compare with a fresh document.
/// Returns the count of differing pixels, and asserts the change is visible.
fn probe(page: &str, scale: f64, change: impl Fn(&mut RinchDocument)) -> usize {
    let mut doc = document(page);
    let before = frame(&mut doc, scale);
    frame(&mut doc, scale);
    change(&mut doc);
    doc.resolve_layout(W, H);
    let after = frame(&mut doc, scale);
    let mut fresh = document(page);
    change(&mut fresh);
    fresh.resolve_layout(W, H);
    let want = frame(&mut fresh, scale);
    assert!(
        diff(&want, &before) > 0,
        "control: the change changed nothing"
    );
    diff(&after, &want)
}

const GLASS: &str = r#"<!doctype html><html><head><style>
html, body { margin: 0; background: #fff; }
body { width: 360px; height: 260px; position: relative; font: 16px/18px sans-serif; }
#bg { position: absolute; left: 0; top: 0; width: 360px; height: 260px;
      background: repeating-linear-gradient(90deg, #135 0 7px, #fd4 7px 13px); }
#mover { position: absolute; left: 20px; top: 20px; width: 30px; height: 20px; background: #e11; }
#shadowy { position: absolute; left: 20px; top: 200px; width: 30px; height: 20px; background: #111; }
#txt { position: absolute; left: 250px; top: 10px; width: 40px; height: 18px; margin: 0; color: #e11; white-space: nowrap; }
#sc { position: absolute; left: 100px; top: 100px; width: 160px; height: 120px; overflow: hidden; }
#sc > div { height: 600px; background: repeating-linear-gradient(0deg, #06c 0 6px, #fff 6px 12px); }
#glass { position: absolute; left: 90px; top: 90px; width: 180px; height: 100px;
         background: rgba(255, 255, 255, 0.2); backdrop-filter: blur(4px); }
#later { position: absolute; left: 0; top: 0; width: 1px; height: 1px; }
#neg { position: absolute; left: 91px; top: 91px; width: 40px; height: 30px; background: #0c0; z-index: -1; }
</style></head><body><div id="bg"></div><div id="mover"></div><div id="shadowy"></div><p id="txt">x</p><div id="sc"><div></div></div><div id="glass"></div><div id="later"><div id="neg"></div></div></body></html>"#;

#[test]
fn b1_mover_behind() {
    let n = probe(GLASS, 1.0, |doc| {
        let m = by_id(doc, "mover");
        doc.set_attribute(m, "style", "left: 66px; top: 78px");
    });
    assert_eq!(n, 0);
}

#[test]
fn b2_scroll_behind() {
    let n = probe(GLASS, 1.0, |doc| {
        let s = by_id(doc, "sc");
        doc.set_scroll_top(s, 3.0);
    });
    assert_eq!(n, 0);
}

#[test]
fn b3_box_shadow_grows_onto_glass() {
    let n = probe(GLASS, 1.0, |doc| {
        let s = by_id(doc, "shadowy");
        doc.set_attribute(s, "style", "box-shadow: 90px -40px 0 30px #0f0");
    });
    assert_eq!(n, 0);
}

#[test]
fn b4_overflowing_text_behind() {
    let n = probe(GLASS, 1.0, |doc| {
        let t = by_id(doc, "txt");
        doc.set_attribute(t, "style", "left: 60px; top: 120px; width: 10px");
        doc.set_text_content(t, "WWWWWWWWWWWW");
    });
    assert_eq!(n, 0);
}

#[test]
fn b4b_overflowing_text_changes_behind() {
    let page = GLASS.replace(
        "#txt { position: absolute; left: 250px; top: 10px;",
        "#txt { position: absolute; left: 40px; top: 120px;",
    );
    let page = page.replace(">x</p>", ">WWWWWWWWWW</p>");
    let n = probe(&page, 1.0, |doc| {
        let t = by_id(doc, "txt");
        doc.set_text_content(t, "MMMMMMMMMM");
    });
    assert_eq!(n, 0);
}

#[test]
fn b5_negative_z_in_later_positioned_sibling() {
    let page = GLASS.replace("<div id=\"bg\"></div>", "");
    let n = probe(&page, 1.0, |doc| {
        let s = by_id(doc, "neg");
        doc.set_attribute(s, "style", "background: #c0c");
    });
    assert_eq!(n, 0);
}

#[test]
fn b6_opacity_behind() {
    let n = probe(GLASS, 1.0, |doc| {
        let s = by_id(doc, "sc");
        doc.set_attribute(s, "style", "opacity: 0.4");
    });
    assert_eq!(n, 0);
}

#[test]
fn b7_transform_behind() {
    let n = probe(GLASS, 1.0, |doc| {
        let s = by_id(doc, "sc");
        doc.set_attribute(s, "style", "transform: rotate(7deg)");
    });
    assert_eq!(n, 0);
}

#[test]
fn b8_body_background() {
    let page = GLASS.replace("<div id=\"bg\"></div>", "");
    let n = probe(&page, 1.0, |doc| {
        let b = doc.body();
        doc.set_attribute(b, "style", "background: #39c");
    });
    assert_eq!(n, 0);
}

#[test]
fn b9_glass_moves_over_static() {
    let n = probe(GLASS, 1.0, |doc| {
        let g = by_id(doc, "glass");
        doc.set_attribute(g, "style", "left: 93.4px");
    });
    assert_eq!(n, 0);
}

#[test]
fn b10_ancestor_between_changes() {
    let page = GLASS.replace(
        "<div id=\"glass\"></div>",
        "<div id=\"mid\" style=\"position:absolute;inset:0\"><div id=\"glass\"></div></div>",
    );
    let n = probe(&page, 1.0, |doc| {
        let m = by_id(doc, "mid");
        doc.set_attribute(
            m,
            "style",
            "position:absolute;inset:0;background: rgba(0,200,0,0.3)",
        );
    });
    assert_eq!(n, 0);
}

// ── Freed ids ───────────────────────────────────────────────────────────────

/// A discarded paint-dirty node whose slot is reused before the next paint:
/// `invalidate` walks `paint_dirty_nodes` with `get_live` (debug-asserted).
/// A child of a blurred sibling changes: its own box misses the glass, the
/// blur spreads it onto the glass.
#[test]
fn b11_child_of_blurred_sibling_spreads_onto_glass() {
    let page = r#"<!doctype html><html><head><style>
html, body { margin: 0; background: #fff; }
body { width: 360px; height: 260px; position: relative; }
#glow { position: absolute; left: 20px; top: 90px; width: 50px; height: 60px; filter: blur(9px); }
#glow > b { position: absolute; left: 0; top: 0; width: 50px; height: 60px; background: #e11; }
#glass { position: absolute; left: 80px; top: 80px; width: 180px; height: 100px;
         background: rgba(255, 255, 255, 0.2); backdrop-filter: blur(2px); }
</style></head><body><div id="glow"><b></b></div><div id="glass"></div></body></html>"#;
    let n = probe(page, 1.0, |doc| {
        let g = by_id(doc, "glow");
        let b = doc.get_children(g)[0];
        doc.set_attribute(b, "style", "background: #11e");
    });
    assert_eq!(n, 0);
}

/// `paints_after` reads `z-index` on a non-positioned stacking context,
/// where it does not apply: `s` paints at layer 0, before `a` at layer 2.
#[test]
fn b12_ignored_z_index_on_later_sibling() {
    let page = r#"<!doctype html><html><head><style>
html, body { margin: 0; background: #fff; }
body { width: 360px; height: 260px; position: relative; }
#a { position: relative; z-index: 2; height: 0; }
#glass { position: absolute; left: 80px; top: 80px; width: 180px; height: 100px;
         background: rgba(255, 255, 255, 0.2); backdrop-filter: blur(3px); }
#s { opacity: 0.99; z-index: 3; margin-top: 100px; margin-left: 120px; width: 60px; height: 40px; background: #e11; }
</style></head><body><div id="a"><div id="glass"></div></div><div id="s"></div></body></html>"#;
    let n = probe(page, 1.0, |doc| {
        let s = by_id(doc, "s");
        doc.set_attribute(s, "style", "background: #11e");
    });
    assert_eq!(n, 0);
}

/// The glass hidden (`display: none`) while what is behind it changes, then
/// shown again where it was: its box was 0x0 when the change was checked.
#[test]
fn b13_glass_hidden_while_behind_changes() {
    let mut doc = document(GLASS);
    frame(&mut doc, 1.0);
    frame(&mut doc, 1.0);
    let g = by_id(&doc, "glass");
    doc.set_attribute(g, "style", "display: none");
    doc.resolve_layout(W, H);
    frame(&mut doc, 1.0);
    let m = by_id(&doc, "mover");
    doc.set_attribute(m, "style", "left: 66px; top: 78px");
    doc.resolve_layout(W, H);
    frame(&mut doc, 1.0);
    doc.set_attribute(g, "style", "");
    doc.resolve_layout(W, H);
    let got = frame(&mut doc, 1.0);
    let mut fresh = document(GLASS);
    let m = by_id(&fresh, "mover");
    fresh.set_attribute(m, "style", "left: 66px; top: 78px");
    fresh.resolve_layout(W, H);
    let want = frame(&mut fresh, 1.0);
    let n = diff(&got, &want);
    assert_eq!(n, 0);
}
