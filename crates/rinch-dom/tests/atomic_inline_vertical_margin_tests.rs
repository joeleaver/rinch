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

/// The boxes of `ids` in a document laid out from `html`.
fn boxes(doc: &RinchDocument, ids: &[&str]) -> Vec<(f32, f32, f32, f32)> {
    ids.iter()
        .map(|id| {
            let n = doc.query_selector(&format!("#{id}")).expect(id);
            let (x, y) = rinch_dom::paint::compute_absolute_position(&doc.tree, n.0, 1.0);
            let l = &doc.tree.get(n.0).unwrap().layout;
            (x as f32, y as f32, l.width, l.height)
        })
        .collect()
}

/// `#a`'s `style` changed from `before` to `after` once the document had been
/// laid out, against a document laid out with `after` from the start: a
/// margin change moves the line as a fresh layout lines it up. `template`
/// holds `STYLE` where `#a`'s style goes.
fn restyled_as_fresh(what: &str, template: &str, before: &str, after: &str, ids: &[&str]) {
    let mut doc = doc(&template.replace("STYLE", before));
    let a = doc.query_selector("#a").unwrap();
    doc.set_attribute(a, "style", after);
    doc.resolve_layout(800.0, 600.0);
    let fresh = self::doc(&template.replace("STYLE", after));
    assert_eq!(
        boxes(&doc, ids),
        boxes(&fresh, ids),
        "{what}: restyled, then fresh"
    );
}

#[test]
fn a_restyled_vertical_margin_moves_the_line_as_a_fresh_layout_does() {
    let line = format!(
        "<div id=line style='width:400px;font-size:0;line-height:0'>\
         <span id=a style='STYLE'></span><img id=b width=10 height=10></div>{NEXT}"
    );
    const BOX: &str = "display:inline-block;width:20px;height:24px";
    restyled_as_fresh(
        "an inline-block given a top margin",
        &line,
        BOX,
        &format!("{BOX};margin-top:20px"),
        &["line", "a", "b", "next"],
    );
    restyled_as_fresh(
        "a top margin moved to the bottom",
        &line,
        &format!("{BOX};margin:10px 0 0"),
        &format!("{BOX};margin:0 0 10px"),
        &["line", "a", "b", "next"],
    );
    restyled_as_fresh(
        "a margin taken away",
        &line,
        &format!("{BOX};margin:7px 0 9px"),
        BOX,
        &["line", "a", "b", "next"],
    );
    // An image alone in a table cell, in a line of text's height (Inter).
    restyled_as_fresh(
        "an image in a cell given margins",
        &format!(
            "<table id=t style='border-spacing:0;font:16px/20px ProbeFace'><tr><td id=c style='padding:0'>\
             <img id=a width=74 height=24 style='STYLE'></td></tr></table>{NEXT}"
        ),
        "",
        "margin:20px 0 35px",
        &["t", "c", "a", "next"],
    );
}

/// A `margin-top` transition on an inline-block moves its line every frame:
/// halfway through, the line is the one a fresh layout makes with the margin
/// at its halfway value, and at the end the one it makes with the end value.
#[test]
fn a_vertical_margin_transition_moves_the_line_each_frame() {
    let html = |style: &str| {
        format!(
            "<div id=line style='width:400px;font-size:0;line-height:0'><span id=a \
             style='display:inline-block;width:20px;height:24px;transition:margin-top 100ms linear;{style}'>\
             </span><img id=b width=10 height=10></div>{NEXT}"
        )
    };
    let ids = ["line", "a", "b", "next"];
    let mut doc = doc(&html("margin-top:0px"));
    doc.tree.transitions_enabled = true;
    doc.set_animation_clock_ms(Some(1_000_000.0));
    doc.resolve_layout(800.0, 600.0);
    let a = doc.query_selector("#a").unwrap();
    doc.set_attribute(
        a,
        "style",
        "display:inline-block;width:20px;height:24px;transition:margin-top 100ms linear;margin-top:20px",
    );
    doc.resolve_layout(800.0, 600.0);
    doc.set_animation_clock_ms(Some(1_000_050.0));
    doc.tick_transitions();
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        boxes(&doc, &ids),
        boxes(&self::doc(&html("margin-top:10px")), &ids),
        "halfway"
    );
    doc.set_animation_clock_ms(Some(1_000_200.0));
    doc.tick_transitions();
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        boxes(&doc, &ids),
        boxes(&self::doc(&html("margin-top:20px")), &ids),
        "finished"
    );
}

/// A negative top margin on an inline-block holding a line of text. Its
/// baseline is that line's, at the top of its content box however tall the
/// box is declared; counted up from the box's bottom (the record of its text
/// was measured at another width: Taffy does not measure a box whose size
/// is declared), the 40px box's baseline was 20px too low, which a zero top
/// margin happened to hide, and `margin-top: -10px` put the box 10px above
/// its line with what followed overlapping it (review of #2308). Chrome 156,
/// Inter at 16px/20px.
#[test]
fn a_negative_top_margin_on_an_inline_block_of_text() {
    const S: &str = "display:inline-block;height:40px;width:20px";
    let cases: &[Case] = &[
        (
            "no declared height",
            "x<span id=a style='display:inline-block;margin-top:-10px'>y</span>z",
            (0.0, 20.0),
            None,
            20.0,
        ),
        (
            "-10px, the box alone",
            &format!("<span id=a style='{S};margin-top:-10px'>y</span>"),
            (0.0, 40.0),
            None,
            40.0,
        ),
        (
            "-4px",
            &format!("x<span id=a style='{S};margin-top:-4px'>y</span>z"),
            (0.0, 40.0),
            None,
            40.0,
        ),
        (
            "-20px",
            &format!("x<span id=a style='{S};margin-top:-20px'>y</span>z"),
            (0.0, 40.0),
            None,
            40.0,
        ),
        (
            "no margin",
            &format!("x<span id=a style='{S}'>y</span>z"),
            (0.0, 40.0),
            None,
            40.0,
        ),
        (
            "-10px, an image on the baseline beside it",
            &format!(
                "x<span id=a style='{S};margin-top:-10px'>y</span><img id=b width=10 height=10>z"
            ),
            (0.0, 40.0),
            Some((6.0, 10.0)),
            40.0,
        ),
        (
            "a min-height in place of the height",
            "x<span id=a style='display:inline-block;min-height:40px;width:20px;margin-top:-10px'>y</span>\
             <img id=b width=10 height=10>z",
            (0.0, 40.0),
            Some((6.0, 10.0)),
            40.0,
        ),
    ];
    for (what, line, a, b, height) in cases {
        let doc = doc(&format!(
            "<div id=line style='font:16px/20px ProbeFace'>{line}</div>{NEXT}"
        ));
        assert_eq!(yh(&doc, "a"), *a, "{what}: #a");
        if let Some(b) = b {
            assert_eq!(yh(&doc, "b"), *b, "{what}: #b");
        }
        assert_eq!(yh(&doc, "line").1, *height, "{what}: the line");
        assert_eq!(yh(&doc, "next").0, *height, "{what}: what follows");
    }
    // In a line of no height of its own, beside a 30px box: Chrome lines the
    // two up by the text's baseline, 14px down.
    let doc = doc(&format!(
        "<div id=line style='font-size:0;line-height:0'><span id=a style='{S};margin-top:-10px;\
         font:16px/20px ProbeFace'>y</span><span id=b style='display:inline-block;width:10px;height:30px'>\
         </span></div>{NEXT}"
    ));
    assert_eq!(yh(&doc, "a"), (14.0, 40.0));
    assert_eq!(yh(&doc, "b"), (0.0, 30.0));
    assert_eq!(yh(&doc, "line").1, 54.0);
}
