//! Where the read-only (`user-select: text`) selection highlight is painted.
//!
//! #1707 — the highlight of a scrolled IFC root follows its text. The text is painted at the content
//! origin less the root's own `scroll_offset`; the highlight used to be painted
//! at the content origin alone, so in a scrolled `overflow: auto` box the wash
//! stayed where the selected text had been before the scroll.
//!
//! #1692 — a flex or grid container's own text (a text leaf, in no IFC root)
//! carries a highlight of its own, painted under its glyphs.
//!
//! A local pixel oracle: the text is `color: transparent` on white, so the
//! only ink in the box is the wash. The face is the bundled Inter under a
//! private family name, and every size is declared.
#![cfg(feature = "software-renderer")]

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const VW: f32 = 300.0;
const VH: f32 = 200.0;
/// One line per row (`white-space: pre`); "line two" is bytes 9..17.
const TEXT: &str = "line one\nline two\nline three\nline four\nline five";

fn doc() -> RinchDocument {
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
    doc
}

/// The rows of column `x` the wash covers, given the box at `top = 20px`
/// holding "line two" selected and scrolled down by `scroll_y`.
fn wash_rows(scroll_y: f64, x: usize) -> Vec<usize> {
    let mut doc = doc();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0; background: white");
    let d = doc.create_element("div");
    doc.set_attribute(
        d,
        "style",
        "position: absolute; left: 20px; top: 20px; width: 160px; height: 60px; \
         overflow: auto; white-space: pre; user-select: text; \
         font-family: ProbeFace; font-size: 16px; line-height: 20px; color: transparent",
    );
    doc.append_child(body, d);
    let t = doc.create_text(TEXT);
    doc.append_child(d, t);
    doc.resolve_layout(VW, VH);
    for (k, v) in [
        ("data-text-sel", "true"),
        ("data-text-sel-start", "9"),
        ("data-text-sel-end", "17"),
    ] {
        doc.set_attribute(d, k, v);
    }
    doc.resolve_layout(VW + 1.0, VH);
    doc.tree.nodes[d.0].scroll_offset = (0.0, scroll_y);

    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    let px = painter.pixels().as_chunks::<4>().0.to_vec();
    (0..VH as usize)
        .filter(|&y| {
            let p = px[y * VW as usize + x];
            [p[0], p[1], p[2]] != [255, 255, 255]
        })
        .collect()
}

#[test]
fn an_unscrolled_selection_is_washed_over_its_line() {
    // The control: "line two" is the second line, rows 40..60.
    let rows = wash_rows(0.0, 22);
    assert_eq!(
        (rows.first(), rows.last()),
        (Some(&40), Some(&59)),
        "{rows:?}"
    );
}

#[test]
fn a_scrolled_selection_is_washed_where_its_text_now_is() {
    // Scrolled by 20px, "line two" is the box's first visible line: rows
    // 20..40. The wash used to stay at 40..60, over "line three".
    let rows = wash_rows(20.0, 22);
    assert_eq!(
        (rows.first(), rows.last()),
        (Some(&20), Some(&39)),
        "{rows:?}"
    );
}

#[test]
fn a_flex_containers_text_is_washed_where_it_is_selected() {
    // "hello world" in a flex container at (20, 20), 20px lines: "world"
    // (bytes 6..11) selected on the text leaf. The wash is on its line,
    // right of where "hello " ends, and nowhere left of it.
    let mut doc = doc();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0; background: white");
    let d = doc.create_element("div");
    doc.set_attribute(
        d,
        "style",
        "position: absolute; left: 20px; top: 20px; display: flex; user-select: text; \
         font-family: ProbeFace; font-size: 16px; line-height: 20px; color: transparent",
    );
    doc.append_child(body, d);
    let t = doc.create_text("hello world");
    doc.append_child(d, t);
    doc.resolve_layout(VW, VH);
    assert!(
        doc.tree.nodes[t.0].cached_text_parley.is_some(),
        "the text is a leaf"
    );
    for (k, v) in [
        ("data-text-sel", "true"),
        ("data-text-sel-start", "6"),
        ("data-text-sel-end", "11"),
    ] {
        doc.set_attribute(t, k, v);
    }
    doc.resolve_layout(VW + 1.0, VH);
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    let px = painter.pixels().as_chunks::<4>().0.to_vec();
    let inked = |x: usize, y: usize| {
        let p = px[y * VW as usize + x];
        [p[0], p[1], p[2]] != [255, 255, 255]
    };
    let row: Vec<usize> = (0..VW as usize).filter(|&x| inked(x, 30)).collect();
    let (first, last) = (row.first().copied(), row.last().copied());
    assert!(
        first.is_some_and(|x| x > 50) && last.is_some_and(|x| x < 120),
        "the wash covers \"world\" only: {first:?}..{last:?}"
    );
    let rows: Vec<usize> = (0..VH as usize)
        .filter(|&y| inked(first.unwrap() + 2, y))
        .collect();
    assert_eq!(
        (rows.first(), rows.last()),
        (Some(&20), Some(&39)),
        "{rows:?}"
    );
}
