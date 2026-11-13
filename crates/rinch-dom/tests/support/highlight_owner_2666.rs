//! #2666's page, shared by `highlight_owner_2666_tests` (software painter) and
//! `gpu_highlight_owner_2666` (in `rinch`, Vello).
//!
//! An editor-shaped stacking context (`position: relative; z-index: 0`) holds
//! a green `pre`, a paragraph whose inline `code` is blue, and three
//! `z-index: -1` washes (half-opaque red) the way the editor draws its
//! selection: one owned by the `pre`, one by the paragraph, and one with no
//! owner over a second green block. An owned wash is painted over its owner's
//! backgrounds and under its glyphs; the unowned one keeps its CSS place, under
//! the green.
#![allow(dead_code)]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

pub const W: u32 = 300;
pub const H: u32 = 100;

const BODY: &str = r#"
<div id=ed style="position:relative;z-index:0;width:300px;font:16px/24px ProbeFace;color:#000">
<pre id=pre style="margin:0;height:24px;background:rgb(0,128,0);font:16px/24px ProbeFace">MMM</pre>
<p id=para style="margin:0;height:24px"><code style="background:rgb(0,0,128);padding-left:20px;font:16px/24px ProbeFace">MMM</code></p>
<div id=plain style="height:24px;background:rgb(0,128,0)"></div>
<div id=w1 style="position:absolute;left:0;top:0;width:300px;height:24px;background:rgba(255,0,0,0.5);z-index:-1"></div>
<div id=w2 style="position:absolute;left:0;top:24px;width:300px;height:24px;background:rgba(255,0,0,0.5);z-index:-1"></div>
<div id=w3 style="position:absolute;left:0;top:48px;width:300px;height:24px;background:rgba(255,0,0,0.5);z-index:-1"></div>
</div>
"#;

fn id(doc: &RinchDocument, sel: &str) -> NodeId {
    doc.query_selector(sel)
        .unwrap_or_else(|| panic!("{sel} is in the page"))
}

/// The page, laid out, `owned` saying whether the first two washes are given
/// their owners.
pub fn document(owned: bool) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    doc.load_css("html, body { margin: 0; background: #fff; }");
    let b = doc.body();
    doc.set_inner_html(b, BODY);
    if owned {
        let (w1, w2) = (id(&doc, "#w1"), id(&doc, "#w2"));
        let (pre, para) = (id(&doc, "#pre"), id(&doc, "#para"));
        doc.set_highlight_owner(w1, Some(pre));
        doc.set_highlight_owner(w2, Some(para));
    }
    doc.resolve_layout(W as f32, H as f32);
    doc
}

fn near(p: [u8; 4], want: [u8; 3], tol: u8) -> bool {
    (0..3).all(|k| p[k].abs_diff(want[k]) <= tol)
}

fn darkest(px: &[[u8; 4]], pw: u32, y0: u32, y1: u32) -> u32 {
    (y0..y1)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .map(|(x, y)| {
            let p = px[(y * pw + x) as usize];
            p[0] as u32 + p[1] as u32 + p[2] as u32
        })
        .min()
        .unwrap()
}

/// Where `px` (the owned page, `pw` px wide, scale 1) is not as Chrome paints
/// a selection.
pub fn misses(px: &[[u8; 4]], pw: u32) -> Vec<String> {
    let at = |x: u32, y: u32| px[(y * pw + x) as usize];
    let mut bad = Vec::new();
    // Over the pre's green, right of its text: red over green.
    let p = at(280, 12);
    if !near(p, [128, 64, 0], 6) {
        bad.push(format!("pre: {p:?} at (280, 12), want red over green"));
    }
    // Over the code's blue padding: red over blue.
    let p = at(10, 36);
    if !near(p, [128, 0, 64], 6) {
        bad.push(format!("code: {p:?} at (10, 36), want red over blue"));
    }
    // Over the paragraph's bare part: red over white.
    let p = at(280, 36);
    if !near(p, [255, 128, 128], 6) {
        bad.push(format!("p: {p:?} at (280, 36), want red over white"));
    }
    // The glyphs are drawn over the wash: some ink stays black.
    for (what, y0) in [("pre", 0), ("code", 24)] {
        let d = darkest(px, pw, y0, y0 + 24);
        if d > 30 {
            bad.push(format!(
                "{what}: darkest ink {d}, want black text over the wash"
            ));
        }
    }
    // An unowned `z-index: -1` box keeps its place under the green.
    let p = at(150, 60);
    if !near(p, [0, 128, 0], 2) {
        bad.push(format!("unowned: {p:?} at (150, 60), want the bare green"));
    }
    bad
}

/// Where `px` (the page with no owners) is not CSS's order: every wash under
/// the green and blue.
pub fn unowned_misses(px: &[[u8; 4]], pw: u32) -> Vec<String> {
    let at = |x: u32, y: u32| px[(y * pw + x) as usize];
    let mut bad = Vec::new();
    for (what, x, y, want) in [
        ("pre", 280, 12, [0, 128, 0]),
        ("code", 10, 36, [0, 0, 128]),
        ("plain", 150, 60, [0, 128, 0]),
    ] {
        let p = at(x, y);
        if !near(p, want, 2) {
            bad.push(format!("{what}: {p:?} at ({x}, {y}), want {want:?}"));
        }
    }
    bad
}
