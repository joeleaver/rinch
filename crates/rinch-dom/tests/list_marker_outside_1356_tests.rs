//! #1356: an `outside` list marker is a box of its own beside the item's first
//! line, hung in the list's padding, not an inline span on a line of its own.
//!
//! Before, the marker was an ordinary inline `<span>` at the start of the
//! `<li>`. An item whose first child is a block (`<li><div>`, `<li><p>`,
//! `<li><table>`, which Outlook and Gmail write for every item) put that span
//! on an anonymous line of its own above the block, so the item was one line
//! taller than in Chrome (40px against 20 at `line-height: 20px`), and every
//! item's text started a marker's width right of the list's padding edge.
//!
//! Each case below is drawn in Chrome 156 and in rinch's software painter with
//! the bundled Inter at a declared `font: 16px/20px`, the item red and its
//! content black, and compares the item's box and the ink of the marker (the
//! red pixels) and of the content (the dark ones). The Chrome numbers were
//! measured with Playwright on this exact markup (a `400px` host at the
//! origin, `body { margin: 0 }`).
//!
//! Known differences, allowed for case by case below and nowhere else:
//! - a number is drawn with Chrome's `::marker { font-variant-numeric:
//!   tabular-nums }`, which rinch does not apply: its digits are narrower, so
//!   the number's far edge (left, or right in RTL) is up to 3px in;
//! - an `inside` disc is a glyph here and a shape in Chrome, and the glyph's
//!   side bearing puts its ink about 0.12em right of Chrome's.

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use rinch_dom::testing::{get_node_summary, query_selector};

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

type Ink = Option<[i32; 4]>;

/// `(name, markup, Chrome's <li> box [x, y, w, h], Chrome's marker ink,
/// Chrome's content ink)`, inks as `[x0, y0, x1, y1]`.
const CASES: &[(&str, &str, [i32; 4], Ink, Ink)] = &[
    (
        "text",
        "<ul style='margin:0'><li><span style='color:#000'>plain text</span></li></ul>",
        [40, 0, 360, 20],
        Some([23, 9, 28, 14]),
        Some([42, 4, 108, 16]),
    ),
    (
        "div",
        "<ul style='margin:0'><li><div style='color:#000'>block child</div></li></ul>",
        [40, 0, 360, 20],
        Some([23, 9, 28, 14]),
        Some([42, 4, 120, 16]),
    ),
    (
        "div_ws",
        "<ul style='margin:0'><li>\n  <div style='color:#000'>block child</div>\n</li></ul>",
        [40, 0, 360, 20],
        Some([23, 9, 28, 14]),
        Some([42, 4, 120, 16]),
    ),
    (
        "p",
        "<ul style='margin:0'><li><p style='color:#000'>para</p></li></ul>",
        [40, 16, 360, 20],
        Some([23, 25, 28, 30]),
        Some([42, 23, 72, 32]),
    ),
    (
        "p0",
        "<ul style='margin:0'><li><p style='margin:0;color:#000'>para</p></li></ul>",
        [40, 0, 360, 20],
        Some([23, 9, 28, 14]),
        Some([42, 7, 72, 16]),
    ),
    (
        "table",
        "<ul style='margin:0'><li><table style='border-spacing:0;color:#000'><tr><td style='padding:0'>cell</td></tr></table></li></ul>",
        [40, 0, 360, 20],
        Some([23, 13, 28, 18]),
        Some([41, 4, 61, 16]),
    ),
    (
        "table_pad",
        "<ul style='margin:0'><li><table style='border-spacing:4px;color:#000'><tr><td style='padding:6px'>cell</td></tr></table></li></ul>",
        [40, 0, 360, 40],
        Some([23, 23, 28, 28]),
        Some([51, 14, 71, 26]),
    ),
    (
        "table_2rows",
        "<ul style='margin:0'><li><table style='border-spacing:0;color:#000'><tr><td style='padding:0'>a</td></tr><tr><td style='padding:0'>b</td></tr></table></li></ul>",
        [40, 0, 360, 40],
        Some([23, 13, 28, 18]),
        Some([41, 7, 49, 36]),
    ),
    (
        "picture",
        "<ul style='margin:0'><li><div style='width:50px;height:30px;background:#00f'></div></li></ul>",
        [40, 0, 360, 30],
        Some([23, 9, 28, 14]),
        None,
    ),
    (
        "img_block",
        "<ul style='margin:0'><li><img style='display:block;width:50px;height:30px;background:#00f'></li></ul>",
        [40, 0, 360, 30],
        Some([23, 9, 28, 14]),
        None,
    ),
    (
        "inline_block",
        "<ul style='margin:0'><li><span style='display:inline-block;width:50px;height:30px;background:#00f'></span></li></ul>",
        [40, 0, 360, 34],
        Some([23, 23, 28, 28]),
        None,
    ),
    (
        "nested",
        "<ul style='margin:0'><li><ul><li><span style='color:#000'>Wide</span></li></ul></li></ul>",
        [40, 0, 360, 20],
        Some([23, 9, 68, 14]),
        Some([81, 4, 118, 16]),
    ),
    (
        "empty_div",
        "<ul style='margin:0'><li><div></div></li></ul>",
        [40, 0, 360, 20],
        Some([23, 9, 28, 14]),
        None,
    ),
    (
        "empty_li",
        "<ul style='margin:0'><li></li></ul>",
        [40, 0, 360, 20],
        Some([23, 9, 28, 14]),
        None,
    ),
    (
        "div_padding",
        "<ul style='margin:0'><li><div style='padding:10px;color:#000'>padded</div></li></ul>",
        [40, 0, 360, 40],
        Some([23, 19, 28, 24]),
        Some([52, 14, 106, 26]),
    ),
    (
        "div_border",
        "<ul style='margin:0'><li><div style='border:7px solid #0f0;color:#000'>bordered</div></li></ul>",
        [40, 0, 360, 34],
        Some([23, 16, 28, 21]),
        Some([49, 14, 114, 23]),
    ),
    (
        "div_margin",
        "<ul style='margin:0'><li><div style='margin-top:12px;color:#000'>m</div></li></ul>",
        [40, 12, 360, 20],
        Some([23, 21, 28, 26]),
        Some([42, 19, 52, 21]),
    ),
    (
        "none",
        "<ul style='margin:0;list-style-type:none'><li><div style='color:#000'>block child</div></li></ul>",
        [40, 0, 360, 20],
        None,
        Some([42, 4, 120, 16]),
    ),
    (
        "inside_block",
        "<ul style='margin:0;list-style-position:inside'><li><div style='color:#000'>block child</div></li></ul>",
        [40, 0, 360, 40],
        Some([40, 9, 45, 14]),
        Some([42, 24, 120, 36]),
    ),
    (
        "inside_text",
        "<ul style='margin:0;list-style-position:inside'><li><span style='color:#000'>plain text</span></li></ul>",
        [40, 0, 360, 20],
        Some([40, 9, 45, 14]),
        Some([64, 4, 130, 16]),
    ),
    (
        "ol",
        "<ol style='margin:0'><li><div style='color:#000'>numbered</div></li></ol>",
        [40, 0, 360, 20],
        Some([23, 4, 34, 16]),
        Some([42, 7, 115, 16]),
    ),
    (
        "ol_text",
        "<ol style='margin:0'><li><span style='color:#000'>numbered</span></li></ol>",
        [40, 0, 360, 20],
        Some([23, 4, 34, 16]),
        Some([42, 7, 115, 16]),
    ),
    (
        "ol_start_12",
        "<ol style='margin:0' start='12'><li><div style='color:#000'>numbered</div></li></ol>",
        [40, 0, 360, 20],
        Some([13, 4, 34, 16]),
        Some([42, 7, 115, 16]),
    ),
    (
        "rtl",
        "<ul style='margin:0;direction:rtl'><li><div style='color:#000'>block child</div></li></ul>",
        [0, 0, 360, 20],
        Some([372, 9, 377, 14]),
        Some([280, 4, 358, 16]),
    ),
    (
        "rtl_ol",
        "<ol style='margin:0;direction:rtl'><li><div style='color:#000'>numbered</div></li></ol>",
        [0, 0, 360, 20],
        Some([366, 4, 377, 16]),
        Some([285, 7, 358, 16]),
    ),
    (
        "wrapped",
        "<ul style='margin:0;width:120px'><li><span style='color:#000'>one two three four five six</span></li></ul>",
        [40, 0, 120, 40],
        Some([23, 9, 28, 14]),
        Some([41, 4, 143, 36]),
    ),
    (
        "big_line",
        "<ul style='margin:0'><li><div style='font-size:32px;line-height:40px;color:#000'>Big</div></li></ul>",
        [40, 0, 360, 40],
        Some([23, 24, 28, 29]),
        Some([43, 7, 85, 38]),
    ),
    (
        "two_divs",
        "<ul style='margin:0'><li><div style='color:#000'>a</div><div style='color:#000'>b</div></li></ul>",
        [40, 0, 360, 40],
        Some([23, 9, 28, 14]),
        Some([41, 7, 49, 36]),
    ),
    (
        "text_then_div",
        "<ul style='margin:0'><li><span style='color:#000'>lead</span><div style='color:#000'>b</div></li></ul>",
        [40, 0, 360, 40],
        Some([23, 9, 28, 14]),
        Some([42, 7, 70, 36]),
    ),
    (
        "flex_child",
        "<ul style='margin:0'><li><div style='display:flex;color:#000'><span>flex</span></div></li></ul>",
        [40, 0, 360, 20],
        Some([23, 9, 28, 14]),
        Some([41, 4, 66, 16]),
    ),
    (
        "box_then_text",
        "<ul style='margin:0'><li><div style='height:30px;background:#00f'></div><div style='color:#000'>text</div></li></ul>",
        [40, 0, 360, 50],
        Some([23, 39, 28, 44]),
        Some([41, 37, 67, 46]),
    ),
    (
        "float_first",
        "<ul style='margin:0'><li><div style='float:left;width:20px;height:50px;background:#00f'></div><div style='color:#000'>text</div></li></ul>",
        [40, 0, 360, 20],
        Some([23, 9, 28, 14]),
        Some([61, 7, 87, 16]),
    ),
    (
        "hr_first",
        "<ul style='margin:0'><li><hr style='margin:0'><div style='color:#000'>text</div></li></ul>",
        [40, 0, 360, 22],
        Some([23, 11, 28, 16]),
        Some([41, 9, 67, 18]),
    ),
    (
        "li_padding",
        "<ul style='margin:0'><li style='padding:10px;border:3px solid #0f0'><div style='color:#000'>t</div></li></ul>",
        [40, 0, 360, 46],
        Some([23, 22, 28, 27]),
        Some([54, 20, 57, 29]),
    ),
    (
        "li_padding_ol",
        "<ol style='margin:0'><li style='padding:10px;border:3px solid #0f0'><div style='color:#000'>t</div></li></ol>",
        [40, 0, 360, 46],
        Some([23, 17, 34, 29]),
        Some([54, 20, 57, 29]),
    ),
    (
        "li_padding_empty",
        "<ul style='margin:0'><li style='padding:10px'></li></ul>",
        [40, 0, 360, 40],
        Some([23, 19, 28, 24]),
        None,
    ),
    (
        "li_height_5",
        "<ul style='margin:0'><li style='height:5px'></li></ul>",
        [40, 0, 360, 5],
        Some([23, 9, 28, 14]),
        None,
    ),
    (
        "li_border_box",
        "<ul style='margin:0'><li style='box-sizing:border-box;padding:4px'></li></ul>",
        [40, 0, 360, 28],
        Some([23, 13, 28, 18]),
        None,
    ),
    (
        "size_12",
        "<ul style='margin:0;font-size:12px;line-height:15px'><li><span style='color:#000'>x</span></li></ul>",
        [40, 0, 360, 15],
        Some([25, 6, 29, 10]),
        Some([41, 5, 45, 11]),
    ),
    (
        "size_24",
        "<ul style='margin:0;font-size:24px;line-height:30px'><li><span style='color:#000'>x</span></li></ul>",
        [40, 0, 360, 30],
        Some([18, 12, 26, 20]),
        Some([42, 10, 52, 23]),
    ),
    (
        "size_32",
        "<ul style='margin:0;font-size:32px;line-height:40px'><li><span style='color:#000'>x</span></li></ul>",
        [40, 0, 360, 40],
        Some([13, 16, 23, 26]),
        Some([42, 13, 56, 31]),
    ),
    (
        "size_32_inside",
        "<ul style='margin:0;font-size:32px;line-height:40px;list-style-position:inside'><li><span style='color:#000'>x</span></li></ul>",
        [40, 0, 360, 40],
        Some([40, 16, 50, 26]),
        Some([85, 13, 99, 31]),
    ),
];

/// How far each edge of the marker's ink may sit from Chrome's, `[x0, y0, x1,
/// y1]`: one pixel of raster difference, and the two known differences of
/// the module doc.
fn marker_tolerance(name: &str) -> [i32; 4] {
    match name {
        // Tabular digits (module doc): the number's far edge.
        "ol" | "ol_text" | "li_padding_ol" => [3, 1, 1, 1],
        "ol_start_12" => [4, 1, 1, 1],
        "rtl_ol" => [1, 1, 4, 1],
        // An inside disc's glyph side bearing (module doc).
        "inside_block" | "inside_text" => [2, 1, 2, 1],
        "size_32_inside" => [4, 1, 4, 1],
        _ => [1, 1, 1, 1],
    }
}

fn document(markup: &str) -> RinchDocument {
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
    doc.load_css("body { margin: 0; font: 16px/20px ProbeFace; } li { color: rgb(255, 0, 0); }");
    let body = doc.body();
    let host = doc.create_element("div");
    doc.set_attribute(host, "style", "width: 400px; color: #000");
    doc.append_child(body, host);
    doc.set_inner_html(host, markup);
    doc.resolve_layout(800.0, 600.0);
    doc
}

/// The red (marker) and dark (content) ink in the top 400x200 of a software
/// paint, classified as the Chrome measurement classified its screenshot.
fn ink(doc: &mut RinchDocument) -> (Ink, Ink) {
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
    let (mut red, mut dark): (Ink, Ink) = (None, None);
    for y in 0..200usize {
        for x in 0..400usize {
            let i = (y * w + x) * 4;
            let (r, g, b, a) = (d[i], d[i + 1], d[i + 2], d[i + 3]);
            if a < 128 {
                continue;
            }
            let slot = if r > 200 && g < 90 && b < 90 {
                &mut red
            } else if r < 100 && g < 100 && b < 100 {
                &mut dark
            } else {
                continue;
            };
            let (x, y) = (x as i32, y as i32);
            let e = slot.get_or_insert([x, y, x + 1, y + 1]);
            e[0] = e[0].min(x);
            e[1] = e[1].min(y);
            e[2] = e[2].max(x + 1);
            e[3] = e[3].max(y + 1);
        }
    }
    (red, dark)
}

fn li_box(doc: &RinchDocument) -> [i32; 4] {
    let li = query_selector(&doc.tree, "li")[0];
    let abs = &get_node_summary(&doc.tree, li).unwrap()["absolute"];
    let at = |k: &str| abs[k].as_f64().unwrap().round() as i32;
    [at("x"), at("y"), at("width"), at("height")]
}

/// Every case against Chrome: the item's box exactly, the marker's ink to
/// within its tolerance, and the content's first ink column to within 2px (a
/// glyph's raster edge differs between the two painters).
#[test]
fn outside_markers_sit_where_chrome_156_draws_them() {
    let mut wrong = Vec::new();
    for &(name, markup, chrome_li, chrome_marker, chrome_text) in CASES {
        let mut doc = document(markup);
        let li = li_box(&doc);
        let (marker, text) = ink(&mut doc);
        let mut why = Vec::new();
        if li != chrome_li {
            why.push(format!("<li> {li:?}, Chrome {chrome_li:?}"));
        }
        match (marker, chrome_marker) {
            (Some(m), Some(c)) => {
                let tol = marker_tolerance(name);
                if (0..4).any(|i| (m[i] - c[i]).abs() > tol[i]) {
                    why.push(format!("marker ink {m:?}, Chrome {c:?} (± {tol:?})"));
                }
            }
            (None, None) => {}
            (m, c) => why.push(format!("marker ink {m:?}, Chrome {c:?}")),
        }
        match (text, chrome_text) {
            (Some(t), Some(c)) if (t[0] - c[0]).abs() <= 2 => {}
            (None, None) => {}
            (t, c) => why.push(format!("content ink {t:?}, Chrome {c:?}")),
        }
        if !why.is_empty() {
            wrong.push(format!("{name}: {}", why.join("; ")));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {} cases differ from Chrome 156:\n{}",
        wrong.len(),
        CASES.len(),
        wrong.join("\n")
    );
}

/// An item with no line box is one marker line high (Chrome 156: `<li></li>`
/// is 20px), and stops being so when it gains a line: here a 10px one, which
/// leaves it 10px high, as in Chrome (the marker's own line is 10px there
/// too). A floor kept from the empty item would hold it at 20.
#[test]
fn the_empty_items_marker_line_goes_when_the_item_gains_a_line() {
    let mut doc = document("<ul style='margin:0'><li></li></ul>");
    assert_eq!(
        li_box(&doc),
        [40, 0, 360, 20],
        "an empty item is one line high"
    );
    let li = query_selector(&doc.tree, "li")[0];
    let li = rinch_core::dom::NodeId(li);
    doc.set_attribute(li, "style", "line-height: 10px");
    let text = doc.create_text("x");
    doc.append_child(li, text);
    doc.resolve_layout(801.0, 600.0);
    assert_eq!(li_box(&doc), [40, 0, 360, 10]);
    // And back: the text gone, the item is one (now 10px) marker line high.
    doc.remove_child(li, text);
    doc.resolve_layout(802.0, 600.0);
    assert_eq!(li_box(&doc), [40, 0, 360, 10]);
    doc.set_attribute(li, "style", "");
    doc.resolve_layout(803.0, 600.0);
    assert_eq!(li_box(&doc), [40, 0, 360, 20]);
}

/// `list-style-position` read at run time: `inside` puts the marker back on
/// a line of its own above a block child (Chrome 156: 40px), `outside` takes
/// it off again (20px).
#[test]
fn list_style_position_switches_between_the_two_markers() {
    let mut doc =
        document("<ul style='margin:0'><li><div style='color:#000'>block child</div></li></ul>");
    assert_eq!(li_box(&doc), [40, 0, 360, 20]);
    let ul = rinch_core::dom::NodeId(query_selector(&doc.tree, "ul")[0]);
    doc.set_attribute(ul, "style", "margin: 0; list-style-position: inside");
    doc.resolve_layout(801.0, 600.0);
    assert_eq!(li_box(&doc), [40, 0, 360, 40]);
    let (marker, _) = ink(&mut doc);
    assert_eq!(marker.map(|m| m[1]), Some(9), "on the first line");
    doc.set_attribute(ul, "style", "margin: 0");
    doc.resolve_layout(802.0, 600.0);
    assert_eq!(li_box(&doc), [40, 0, 360, 20]);
    let (marker, _) = ink(&mut doc);
    assert_eq!(marker, Some([23, 9, 28, 14]));
}

/// The marker follows the item's first line when the line moves: the block
/// above it grows, at the same viewport, through a style change only.
#[test]
fn the_marker_follows_the_first_line_when_it_moves() {
    let mut doc = document(
        "<ul style='margin:0'><li><div id='gap' style='height:30px'></div><div style='color:#000'>text</div></li></ul>",
    );
    let (marker, _) = ink(&mut doc);
    assert_eq!(marker, Some([23, 39, 28, 44]));
    let gap = rinch_core::dom::NodeId(query_selector(&doc.tree, "#gap")[0]);
    doc.set_attribute(gap, "style", "height: 47px");
    doc.resolve_layout(800.0, 600.0);
    let (marker, _) = ink(&mut doc);
    assert_eq!(marker, Some([23, 56, 28, 61]));
}

/// A layout that does not move the item's first line leaves the marker where
/// it is and does not repaint it: the read-back keeps the place
/// `list_marker` wrote rather than Taffy's static position, which would move
/// it there and back (and repaint it) on every layout.
#[test]
fn a_layout_that_moves_nothing_does_not_repaint_the_marker() {
    let mut doc = document(
        "<ul style='margin:0'><li><div style='color:#000'>block child</div></li></ul><div id='other'>x</div>",
    );
    let li = query_selector(&doc.tree, "li")[0];
    let marker = doc.tree.get(li).unwrap().children[0];
    assert!(doc.tree.get(marker).unwrap().is_pseudo_element);
    let other = rinch_core::dom::NodeId(query_selector(&doc.tree, "#other")[0]);
    doc.tree.paint_dirty_nodes.clear();
    doc.set_attribute(other, "style", "height: 37px");
    doc.resolve_layout(800.0, 600.0);
    assert!(
        doc.tree.paint_dirty_nodes.contains(&other.0),
        "precondition: the layout ran and the other box moved"
    );
    assert!(
        !doc.tree.paint_dirty_nodes.contains(&marker),
        "the marker did not move"
    );
    let (marker_ink, _) = ink(&mut doc);
    assert_eq!(marker_ink, Some([23, 9, 28, 14]));
}

/// An empty item restyled off `list-item` loses its marker and the marker's
/// line with it: an empty `display: block` box is 0px high (Chrome 156), and
/// `list-style-position: inside` gives the marker's own line back as content.
#[test]
fn an_empty_item_that_loses_its_outside_marker_loses_its_line() {
    let mut doc = document("<ul style='margin:0'><li></li></ul>");
    assert_eq!(li_box(&doc), [40, 0, 360, 20]);
    let li = rinch_core::dom::NodeId(query_selector(&doc.tree, "li")[0]);
    doc.set_attribute(li, "style", "display: block");
    doc.resolve_layout(801.0, 600.0);
    assert_eq!(li_box(&doc), [40, 0, 360, 0]);
    doc.set_attribute(li, "style", "");
    doc.resolve_layout(802.0, 600.0);
    assert_eq!(li_box(&doc), [40, 0, 360, 20]);
}

/// Review of PR #2271: shapes measured in Chrome 156 by the reviewer (same
/// font and size; the item `#t` red, its content `.k` black, a 400px host).
/// The content ink is looked for inside `#t`'s box only.
mod review_2271 {
    use super::*;
    use peniko::Fill;
    use peniko::kurbo::{Affine, Rect};
    use rinch_core::dom::NodeId;
    use rinch_dom::paint::painter::{PaintShape, Painter};

    const VP: (f32, f32) = (800.0, 600.0);

    fn doc(markup: &str, css: &str, scroll: Option<f64>) -> RinchDocument {
        use parley::fontique::{Blob, FontInfoOverride};
        let mut doc = RinchDocument::new();
        doc.font_cx.collection.register_fonts(
            Blob::new(std::sync::Arc::new(FACE)),
            Some(FontInfoOverride {
                family_name: Some("ProbeFace"),
                ..Default::default()
            }),
        );
        doc.load_css(&format!(
            "html, body {{ margin: 0; padding: 0 }} body {{ font: 16px/20px ProbeFace; color: #000 }} \
             #t {{ color: rgb(255,0,0) }} .k {{ color: #000 }} {css}"
        ));
        let body = doc.body();
        let host = doc.create_element("div");
        doc.set_attribute(host, "style", "width: 400px");
        doc.append_child(body, host);
        doc.set_inner_html(host, markup);
        doc.resolve_layout(VP.0, VP.1);
        if let Some(top) = scroll {
            let sc = NodeId(query_selector(&doc.tree, "#sc")[0]);
            doc.set_scroll_top(sc, top);
            doc.resolve_layout(VP.0 + 1.0, VP.1);
        }
        doc
    }

    fn painter(doc: &mut RinchDocument) -> TinySkiaPainter {
        let mut p = TinySkiaPainter::new(VP.0 as u32, VP.1 as u32);
        p.fill_white();
        let mut cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(&doc.tree, &mut p, 1.0, VP, &mut doc.font_cx, &mut cx);
        p
    }

    fn t_box(doc: &RinchDocument) -> [i32; 4] {
        let t = query_selector(&doc.tree, "#t")[0];
        let abs = &get_node_summary(&doc.tree, t).unwrap()["absolute"];
        let at = |k: &str| abs[k].as_f64().unwrap().round() as i32;
        [at("x"), at("y"), at("width"), at("height")]
    }

    /// Red ink anywhere in the top 400x200, and dark ink inside `rect`.
    fn inks(p: &TinySkiaPainter, rect: [i32; 4]) -> (Ink, Ink) {
        let (d, w) = (p.pixels(), p.width() as usize);
        let (mut red, mut dark): (Ink, Ink) = (None, None);
        for y in 0..200usize {
            for x in 0..400usize {
                let i = (y * w + x) * 4;
                let (r, g, b) = (d[i] as i32, d[i + 1] as i32, d[i + 2] as i32);
                let (xi, yi) = (x as i32, y as i32);
                let inside = xi >= rect[0]
                    && xi < rect[0] + rect[2]
                    && yi >= rect[1]
                    && yi < rect[1] + rect[3];
                let slot = if r > 200 && g < 90 && b < 90 {
                    &mut red
                } else if r < 100 && g < 100 && b < 100 && inside {
                    &mut dark
                } else {
                    continue;
                };
                let e = slot.get_or_insert([xi, yi, xi + 1, yi + 1]);
                e[0] = e[0].min(xi);
                e[1] = e[1].min(yi);
                e[2] = e[2].max(xi + 1);
                e[3] = e[3].max(yi + 1);
            }
        }
        (red, dark)
    }

    /// `(name, markup, extra css, scroll of #sc, Chrome's #t box, marker ink,
    /// content ink)`.
    #[allow(clippy::type_complexity)]
    const CASES: &[(&str, &str, &str, Option<f64>, [i32; 4], Ink, Ink)] = &[
        // An outside marker is clipped by the boxes that clip its item (1).
        (
            "scroll_under_header",
            "<div style='height:50px'></div><div id=sc style='height:60px;overflow:auto;width:300px'><ul id=t style='margin:0'><li><span class=k>a</span></li><li><span class=k>b</span></li><li><span class=k>c</span></li><li><span class=k>d</span></li><li><span class=k>e</span></li><li><span class=k>f</span></li></ul></div>",
            "",
            Some(40.0),
            [0, 10, 285, 120],
            Some([23, 59, 28, 104]),
            Some([41, 57, 48, 106]),
        ),
        (
            "scrolled",
            "<div id=sc style='height:40px;overflow:auto;width:300px'><ul style='margin:0'><li><span class=k>a</span></li><li id=t><span class=k>b</span></li><li><span class=k>c</span></li><li><span class=k>d</span></li></ul></div>",
            "",
            Some(25.0),
            [40, -5, 245, 20],
            Some([23, 4, 28, 9]),
            Some([42, 2, 49, 11]),
        ),
        (
            "hidden_under_header",
            "<div style='height:50px'></div><div style='height:30px;overflow:hidden;width:300px'><ul id=t style='margin:0;margin-top:-35px'><li><span class=k>a</span></li><li><span class=k>b</span></li><li><span class=k>c</span></li></ul></div>",
            "",
            None,
            [0, 15, 300, 60],
            Some([23, 64, 28, 69]),
            Some([41, 50, 47, 71]),
        ),
        (
            "clip_parent",
            "<div style='height:15px;overflow:hidden'><ul style='margin:0'><li><span class=k>a</span></li><li id=t><span class=k>b</span></li></ul></div>",
            "",
            None,
            [40, 20, 360, 20],
            None,
            None,
        ),
        // ... including the item's own `overflow: hidden` (6).
        (
            "li_overflow_hidden",
            "<ul style='margin:0'><li id=t style='overflow:hidden'><span class=k>hid</span></li></ul>",
            "",
            None,
            [40, 0, 360, 20],
            None,
            Some([42, 4, 61, 16]),
        ),
        // It paints in its item's flow, under a later-painted positioned box (2).
        (
            "positioned_cover",
            "<div style='position:relative;height:0'><div style='position:absolute;top:0;left:0;width:40px;height:20px;background:#fff'></div></div><ul style='margin:0'><li id=t><span class=k>under</span></li></ul>",
            "",
            None,
            [40, 0, 360, 20],
            None,
            Some([42, 7, 83, 16]),
        ),
        // An author rule for the item's children does not reach the marker,
        // as none reaches a `::marker` (4).
        (
            "li_font_rule",
            "<ul style='margin:0'><li id=t><span class=k>x</span></li></ul>",
            "li span:first-child{font-size:30px}",
            None,
            [40, 0, 360, 25],
            Some([23, 14, 28, 19]),
            Some([42, 4, 55, 21]),
        ),
        (
            "li_star_rule",
            "<ul style='margin:0'><li id=t><span class=k>x</span></li></ul>",
            "li *{font-size:30px}",
            None,
            [40, 0, 360, 25],
            Some([23, 14, 28, 19]),
            Some([42, 4, 55, 21]),
        ),
        // An inside marker starts an RTL line (5).
        (
            "rtl_inside",
            "<ul style='margin:0;direction:rtl;list-style-position:inside'><li id=t><span class=k>in</span></li></ul>",
            "",
            None,
            [0, 0, 360, 20],
            Some([355, 9, 360, 14]),
            Some([326, 4, 336, 16]),
        ),
    ];

    #[test]
    fn the_reviews_shapes_match_chrome_156() {
        let mut wrong = Vec::new();
        for &(name, markup, css, scroll, chrome_box, chrome_marker, chrome_text) in CASES {
            let mut d = doc(markup, css, scroll);
            let b = t_box(&d);
            let p = painter(&mut d);
            let (m, t) = inks(&p, b);
            let mut why = Vec::new();
            // Not the width: in a scroller Chrome takes a 15px classic scrollbar
            // out of it, and rinch's scrollbar overlays the content (older than
            // this; the marker hangs from the left edge either way).
            if [b[0], b[1], b[3]] != [chrome_box[0], chrome_box[1], chrome_box[3]] {
                why.push(format!("#t {b:?}, Chrome {chrome_box:?}"));
            }
            match (m, chrome_marker) {
                (Some(m), Some(c)) if (0..4).all(|i| (m[i] - c[i]).abs() <= 1) => {}
                (None, None) => {}
                (m, c) => why.push(format!("marker ink {m:?}, Chrome {c:?}")),
            }
            match (t, chrome_text) {
                (Some(t), Some(c)) if (t[0] - c[0]).abs() <= 2 && (t[1] - c[1]).abs() <= 3 => {}
                (None, None) => {}
                (t, c) => why.push(format!("content ink {t:?}, Chrome {c:?}")),
            }
            if !why.is_empty() {
                wrong.push(format!("{name}: {}", why.join("; ")));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    /// A damage-only repaint after a scroll paints what a full repaint does:
    /// the markers move and are clipped with the scroller's content.
    #[test]
    fn a_partial_repaint_after_a_scroll_equals_a_full_one() {
        let markup = "<div style='height:50px'></div><div id=sc style='height:60px;overflow:auto;width:300px'><ul style='margin:0'><li><span class=k>a</span></li><li><span class=k>b</span></li><li><span class=k>c</span></li><li><span class=k>d</span></li><li><span class=k>e</span></li><li><span class=k>f</span></li></ul></div>";
        let mut d = doc(markup, "li { color: rgb(255,0,0) }", None);
        let mut p = painter(&mut d);
        d.tree.consume_paint_dirty();
        let sc = NodeId(query_selector(&d.tree, "#sc")[0]);
        d.set_scroll_top(sc, 40.0);
        d.resolve_layout(VP.0, VP.1);
        let damage = rinch_dom::paint::compute_damage(&d.tree, 1.0, VP.0 as f64, VP.1 as f64);
        assert!(!damage.is_full(), "precondition: a partial repaint");
        let rects = damage.rects().to_vec();
        for r in &rects {
            let r = r.intersect(Rect::new(0.0, 0.0, VP.0 as f64, VP.1 as f64));
            if r.width() <= 0.0 || r.height() <= 0.0 {
                continue;
            }
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
        let full = painter(&mut d);
        let differing = partial
            .chunks(4)
            .zip(full.pixels().chunks(4))
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(differing, 0, "pixels a partial repaint got wrong");
        let (marker, _) = inks(&full, [0, 0, 0, 0]);
        assert_eq!(marker, Some([23, 59, 28, 104]), "Chrome 156's marker ink");
    }
}
