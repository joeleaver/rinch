//! An atomic inline's top and bottom margins take part in its line (#1405,
//! its block axis): the line lines up the box's **margin box** (CSS 2.1
//! §10.8), so the margins grow the line box, the border box sits inside them,
//! and a box with no baseline (an `<img>`, an empty `inline-block`) puts its
//! bottom margin edge on the baseline. rinch lined up the border box, so an
//! `<img style="margin: 20px 0 35px">` logo in a cell of its own took 55px
//! less than in Chrome (a mail's header was that much shorter).
//!
//! The horizontal margins are still left out (#1405 stays open for them).
//!
//! Every expected number is Chrome 156's for the same markup (Playwright,
//! `getBoundingClientRect`, `body { margin: 0 }` at 800x600). The lines are
//! `font-size: 0; line-height: 0` and hold only boxes, so no number depends
//! on a font, but for the one line with text, set in the bundled Inter.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const INTER: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn doc(html: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(INTER)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    doc.load_css("body { margin: 0; font: 16px/20px sans-serif; }");
    let body = doc.body();
    doc.set_inner_html(body, html);
    doc.resolve_layout(800.0, 600.0);
    doc
}

/// `(y, height)` of `#id`'s border box in the document.
fn yh(doc: &RinchDocument, id: &str) -> (f32, f32) {
    let n = doc.query_selector(&format!("#{id}")).expect(id);
    let (_, y) = rinch_dom::paint::compute_absolute_position(&doc.tree, n.0, 1.0);
    let l = &doc.tree.get(n.0).unwrap().layout;
    (y as f32, l.height)
}

/// What, the line's content, `(y, height)` of `#a`, of `#b` when there is
/// one, and the line's height.
type Case<'a> = (&'a str, &'a str, (f32, f32), Option<(f32, f32)>, f32);

/// What follows the line: where its block ends.
const NEXT: &str = "<div id=next style='height:5px'></div>";

#[test]
fn the_vertical_margins_grow_the_line() {
    // (what, line content, Chrome's (y, height) of #a, of #b when there is
    // one, and the line's block's height)
    let cases: &[Case] = &[
        (
            "an empty inline-block: margins 5 and 7, a 50px box beside it",
            "<div id=a style='display:inline-block;width:100px;height:50px;margin:5px 0 7px'></div>\
             <div id=b style='display:inline-block;width:50px;height:50px'></div>",
            (5.0, 50.0),
            // Both bottom margin edges on the baseline, at 62.
            Some((12.0, 50.0)),
            62.0,
        ),
        (
            "an image's bottom margin",
            "<img id=a width=74 height=24 style='margin-bottom:16px'>",
            (0.0, 24.0),
            None,
            40.0,
        ),
        (
            "both margins of an image, a smaller one beside it",
            "<img id=a width=74 height=24 style='margin:7px 0 16px'><img id=b width=10 height=10>",
            (7.0, 24.0),
            Some((37.0, 10.0)),
            47.0,
        ),
        (
            "vertical-align: top lines the margin box up",
            "<img id=a width=74 height=24 style='margin:9px 0 16px;vertical-align:top'>\
             <img id=b width=10 height=60 style='vertical-align:top'>",
            (9.0, 24.0),
            Some((0.0, 60.0)),
            60.0,
        ),
        (
            "a negative margin shrinks the margin box",
            "<img id=a width=74 height=24 style='margin-bottom:-10px'><img id=b width=10 height=30>",
            (16.0, 24.0),
            Some((0.0, 30.0)),
            30.0,
        ),
        (
            "a margin box of negative height: its top edge is under its bottom",
            "<img id=a width=74 height=24 style='margin-bottom:-30px'><img id=b width=10 height=30>",
            (36.0, 24.0),
            Some((0.0, 30.0)),
            30.0,
        ),
    ];
    for (what, line, a, b, height) in cases {
        let doc = doc(&format!(
            "<div id=line style='width:400px;font-size:0;line-height:0'>{line}</div>{NEXT}"
        ));
        assert_eq!(yh(&doc, "a"), *a, "{what}: #a");
        if let Some(b) = b {
            assert_eq!(yh(&doc, "b"), *b, "{what}: #b");
        }
        assert_eq!(yh(&doc, "line").1, *height, "{what}: the line");
        assert_eq!(yh(&doc, "next").0, *height, "{what}: what follows");
    }
}

/// An inline-block with a line of text has a baseline: its text sits on the
/// line's baseline, its border box its top margin below the margin box's
/// top, and the line grows by both margins. Chrome 156, Inter at 16px/20px:
/// the box at y 6, the line 35px tall, and an image beside it, which sits on
/// the baseline, at 12 (the baseline at 22).
#[test]
fn an_inline_block_with_a_baseline_keeps_it_inside_its_margins() {
    let doc = doc(&format!(
        "<div id=line style='font:16px/20px ProbeFace'>x <span id=a style='display:inline-block;margin:6px 0 9px'>\
         <span>y</span></span><img id=b width=10 height=10></div>{NEXT}"
    ));
    assert_eq!(yh(&doc, "a"), (6.0, 20.0));
    assert_eq!(yh(&doc, "line").1, 35.0);
    assert_eq!(yh(&doc, "b"), (12.0, 10.0), "on the line's baseline");
}
