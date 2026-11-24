//! #2276: CSS 2.1 Appendix E paints a stacking context's in-flow,
//! non-positioned descendants in phases — every block background and border
//! (step 4), then the floats (step 5), then every line's content: text,
//! inline backgrounds, atomic inlines, list markers (step 7). So a later
//! block's background never covers an earlier block's text. rinch painted
//! each in-flow child whole, in tree order, and a later block pulled up by a
//! negative margin, or one an earlier box's text overflows into, hid it.
//!
//! Each case is drawn in Chrome 158 and in rinch's software painter with the
//! bundled Inter at a declared `font: 16px/20px` in a `400px` host at the
//! origin (`body { margin: 0 }`), and compares the ink of each colour class:
//! dark (text), red (a list marker), blue (a box the case expects on top or
//! hidden). The Chrome numbers were measured with headless Chrome on this
//! exact markup (a screenshot of a 400x200 window).
//!
//! The phases are decided in the paint walk both painters are driven by
//! (`paint::paint_children_with_stacking`), not in a painter, so the Vello
//! scene is handed the same calls in the same order.

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// `[x0, y0, x1, y1, count]` of one colour class.
type Ink = Option<[i32; 5]>;

/// `(name, markup, Chrome's dark ink, Chrome's red ink, Chrome's blue ink)`.
const CASES: &[(&str, &str, Ink, Ink, Ink)] = &[
    // The issue's first case: a later block pulled up over a line of text.
    (
        "neg_margin_text",
        "<div>abc</div><div style='margin-top:-20px;height:20px;background:#ff0'></div>",
        Some([1, 4, 26, 16, 74]),
        None,
        None,
    ),
    // The issue's second: an outside marker under a later block's background.
    (
        "marker",
        "<ul style='margin:0'><li style='color:#f00'>a</li></ul><div style='margin-top:-20px;height:20px;width:40px;background:#ff0'></div>",
        None,
        Some([23, 7, 47, 16, 46]),
        None,
    ),
    // Text overflowing a fixed-height box into the next block's background.
    (
        "overflow_text",
        "<div style='height:20px'>one<br>two</div><div style='height:40px;background:#ff0'></div>",
        Some([1, 7, 27, 36, 126]),
        None,
        None,
    ),
    // The same one level down: the overflow is a grandchild's.
    (
        "deep_overflow",
        "<div><div style='height:20px'>one<br>two</div></div><div style='height:40px;background:#ff0'></div>",
        Some([1, 7, 27, 36, 126]),
        None,
        None,
    ),
    // A float overhangs the next block, whose background is painted before
    // it (step 4 before step 5).
    (
        "float_next_bg",
        "<div><div style='float:left;width:30px;height:60px;background:#00f'></div>x</div><p style='margin:0;height:60px;background:#ff0'>yy</p>",
        Some([31, 7, 47, 39, 55]),
        None,
        Some([0, 0, 30, 60, 1800]),
    ),
    // An inline-block is line content: atomic, step 7.
    (
        "inline_block",
        "<div><span style='display:inline-block;width:30px;height:20px;background:#00f'></span></div><div style='margin-top:-20px;height:20px;background:#ff0'></div>",
        None,
        None,
        Some([0, 0, 30, 20, 600]),
    ),
    // Flex items paint as inline blocks, each whole (css-flexbox-1 §5.4):
    // the later item's background covers the earlier item's text.
    (
        "flex_atomic",
        "<div style='display:flex'><div style='width:100px'>abc</div><div style='margin-left:-100px;width:100px;height:20px;background:#ff0'></div></div>",
        None,
        None,
        None,
    ),
    // The earlier box clips: its text is drawn in the content phase under its
    // own clip.
    (
        "clip_earlier",
        "<div style='overflow:hidden;height:20px'>abc</div><div style='margin-top:-20px;height:20px;background:#ff0'></div>",
        Some([1, 4, 26, 16, 74]),
        None,
        None,
    ),
    // A scroll container is no stacking context either: its lines are
    // drawn in the content pass, inside its clip and scrolled.
    (
        "scroller",
        "<div style='overflow:auto;height:20px'>abc<br>def</div><div style='margin-top:-20px;height:20px;width:300px;background:#ff0'></div>",
        Some([1, 4, 26, 16, 74]),
        None,
        None,
    ),
    // Both boxes inside a clip: the phases run inside it.
    (
        "clip_parent",
        "<div style='overflow:hidden;height:30px'><div>abc</div><div style='margin-top:-20px;height:20px;background:#ff0'></div></div>",
        Some([1, 4, 26, 16, 74]),
        None,
        None,
    ),
    // The control: later text over an earlier background is the same in
    // either order.
    (
        "later_text_over",
        "<div style='background:#00f;height:20px'></div><div style='margin-top:-20px'>abc</div>",
        Some([0, 4, 26, 16, 85]),
        None,
        Some([0, 0, 400, 20, 7855]),
    ),
];

fn document(host_style: &str, markup: &str) -> RinchDocument {
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
    doc.load_css("body { margin: 0; font: 16px/20px ProbeFace; }");
    let body = doc.body();
    let host = doc.create_element("div");
    doc.set_attribute(
        host,
        "style",
        &format!("width: 400px; color: #000; {host_style}"),
    );
    doc.append_child(body, host);
    doc.set_inner_html(host, markup);
    doc.resolve_layout(800.0, 600.0);
    doc
}

/// The dark, red and blue ink in the top 400x200 of a software paint,
/// classified as the Chrome measurement classified its screenshot.
fn ink(doc: &mut RinchDocument) -> [Ink; 3] {
    let mut painter = TinySkiaPainter::new(800, 600);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (800.0, 600.0),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    let (d, w) = (painter.pixels(), painter.width() as usize);
    let mut out: [Ink; 3] = [None; 3];
    for y in 0..200usize {
        for x in 0..400usize {
            let i = (y * w + x) * 4;
            let (r, g, b, a) = (d[i], d[i + 1], d[i + 2], d[i + 3]);
            if a < 128 {
                continue;
            }
            let slot = if r > 200 && g < 90 && b < 90 {
                1
            } else if b > 200 && r < 90 && g < 90 {
                2
            } else if r < 100 && g < 100 && b < 100 {
                0
            } else {
                continue;
            };
            let (x, y) = (x as i32, y as i32);
            let e = out[slot].get_or_insert([x, y, x + 1, y + 1, 0]);
            e[0] = e[0].min(x);
            e[1] = e[1].min(y);
            e[2] = e[2].max(x + 1);
            e[3] = e[3].max(y + 1);
            e[4] += 1;
        }
    }
    out
}

/// Whether `got` is Chrome's `want`: both absent, or both present with every
/// edge within 2px (a glyph's raster edge differs between the painters) and
/// the count within a factor of two. Text is anti-aliased differently by the
/// two (rinch's glyphs classify more pixels dark: 178 against 126 for two
/// lines here), so the count says "drawn, not mostly covered", and the edges
/// say where.
fn agrees(got: Ink, want: Ink) -> bool {
    match (got, want) {
        (None, None) => true,
        (Some(g), Some(w)) => {
            (0..4).all(|i| (g[i] - w[i]).abs() <= 2) && g[4] * 2 >= w[4] && g[4] <= w[4] * 2
        }
        _ => false,
    }
}

/// Every case against Chrome 158.
///
/// Mutants (each fails the named cases): painting each in-flow child whole,
/// in tree order (the pre-#2276 walk) — `neg_margin_text`, `marker`,
/// `overflow_text`, `deep_overflow`, `float_next_bg`, `inline_block`,
/// `clip_earlier`, `clip_parent`; splitting across a flex container's items
/// — `flex_atomic`; deciding the split from the children's own boxes only,
/// not their descendants' — `deep_overflow`.
fn check(host_style: &str) {
    let mut wrong = Vec::new();
    for &(name, markup, dark, red, blue) in CASES {
        let mut doc = document(host_style, markup);
        let got = ink(&mut doc);
        for (class, g, w) in [
            ("dark", got[0], dark),
            ("red", got[1], red),
            ("blue", got[2], blue),
        ] {
            if !agrees(g, w) {
                wrong.push(format!("{name} {class}: rinch {g:?}, Chrome {w:?}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn in_flow_content_paints_in_appendix_e_phases_as_chrome_158_does() {
    check("");
}

/// The same run as a stacking context's in-flow entries (the sequence's
/// step 2), where `paint_children_with_stacking` walks it from the order.
/// Chrome 158 draws every case the same with the host at `z-index: 0`.
#[test]
fn a_stacking_contexts_in_flow_entries_paint_in_phases_too() {
    check("position: relative; z-index: 0");
}

/// The split is memoised per layout (`Node::phase_split`), so a style write that
/// makes a later block overlap an earlier one must reach it: paint once with
/// the blocks stacked (no split), pull the later one up, lay out and paint
/// again. The earlier text shows, as in Chrome 158 (`neg_margin_text`).
///
/// Mutant: keeping a node's split memo across a `layout_epoch` change
/// — the second paint reuses "no split" and the text is hidden.
#[test]
fn a_style_write_that_makes_blocks_overlap_reaches_the_memoised_split() {
    let mut doc = document(
        "",
        "<div>abc</div><div id=b style='height:20px;background:#ff0'></div>",
    );
    let before = ink(&mut doc);
    assert!(
        agrees(before[0], Some([1, 4, 26, 16, 74])),
        "positive control: the text shows while the blocks are stacked: {:?}",
        before[0]
    );
    let b = rinch_dom::testing::query_selector(&doc.tree, "#b")[0];
    doc.set_attribute(
        rinch_core::dom::NodeId(b),
        "style",
        "margin-top:-20px;height:20px;background:#ff0",
    );
    doc.resolve_layout(800.0, 600.0);
    let after = ink(&mut doc);
    assert!(
        agrees(after[0], Some([1, 4, 26, 16, 74])),
        "the earlier text under the pulled-up block: rinch {:?}, Chrome {:?}",
        after[0],
        Some([1, 4, 26, 16, 74])
    );
}
