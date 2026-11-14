//! Soft hyphen follow-ups (#2654) to #2614's hyphen:
//!
//! 1. `letter-spacing` is not added to a soft hyphen, drawn or not;
//! 2. a line broken at the spaces right after a soft hyphen draws its hyphen
//!    (`Extra&shy; ordinary` is `Extra- ` / `ordinary`), and is fitted with it;
//! 3. the soft hyphen's selection rect starts after an element's start edge
//!    (a left border) right before it;
//! 4. rinch's own breaks (the seams pass) look past an element's end edge for
//!    the hyphen where parley's break does not stand in for them.
//!
//! Every expectation is Chrome 157's for the same markup, measured with the
//! bundled Inter as `ProbeFace`, `font: 16px/20px`. A line is written as
//! rinch's text of it, a soft hyphen as `~`; a width is the line's advance.
//! Inter's U+2010 is 5.73px at 16px; `Extra` is 38.91, a space 4.5.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// Text that does not wrap right before wrapping text: a text seam (#2390),
/// which sends the paragraph through rinch's own line breaks.
const TEXT_SEAM: &str = "<span style='white-space:nowrap'>abc-</span>123 ";

/// Inter's hyphen at 16px, Chrome 157.
const HYPHEN: f32 = 5.73;

/// `Extra`, Chrome 157.
const EXTRA: f32 = 38.91;

/// A space, Chrome 157.
const SPACE: f32 = 4.5;

fn html(markup: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut d = RinchDocument::new();
    let registered = d.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    d.load_css("* { margin: 0; padding: 0 } body { font: 16px/20px ProbeFace }");
    let body = d.body();
    d.set_inner_html(body, markup);
    d.resolve_layout(800.0, 800.0);
    d
}

fn by_id(d: &RinchDocument, id: &str) -> NodeId {
    d.query_selector(&format!("#{id}")).expect(id)
}

struct Laid {
    advances: Vec<f32>,
    doc: RinchDocument,
}

impl Laid {
    fn layout(&self) -> &parley::Layout<peniko::Brush> {
        let n = self.doc.tree.get(by_id(&self.doc, "a").0).unwrap();
        &n.text_layout.as_ref().unwrap().layout
    }

    /// The selection rects of the first soft hyphen: `(x, y, width)`.
    fn soft_hyphen_rects(&self) -> Vec<(f32, f32, f32)> {
        let n = self.doc.tree.get(by_id(&self.doc, "a").0).unwrap();
        let il = n.text_layout.as_ref().unwrap();
        let at = il.text_content.find('\u{ad}').expect("a soft hyphen");
        rinch_dom::text_query::selection_rects_for_layout(&il.layout, at, at + 2)
            .into_iter()
            .map(|(x, y, w, _)| (x, y, w))
            .collect()
    }
}

/// `markup` in a `width`px div styled `style`; asserts Chrome's lines and
/// height.
#[track_caller]
fn breaks(markup: &str, width: u32, style: &str, chrome: &[&str]) -> Laid {
    let doc = html(&format!(
        "<div id=a style='width:{width}px;{style}'>{markup}</div>"
    ));
    let n = doc.tree.get(by_id(&doc, "a").0).unwrap();
    let il = n.text_layout.as_ref().expect("an IFC root");
    let mut lines = Vec::new();
    let mut advances = Vec::new();
    for l in il.layout.lines() {
        lines.push(
            il.text_content
                .get(l.text_range())
                .unwrap_or("")
                .replace('\u{ad}', "~"),
        );
        advances.push(l.metrics().advance);
    }
    let height = n.layout.height;
    assert_eq!(lines, chrome, "{width}px {style}: {markup}");
    let tall = 20.0 * chrome.len() as f32;
    assert!(
        (height - tall).abs() < 0.5,
        "{width}px {style}: {markup}: rinch {height}, Chrome {tall}"
    );
    Laid { advances, doc }
}

#[track_caller]
fn close(rinch: f32, chrome: f32, what: &str) {
    assert!(
        (rinch - chrome).abs() < 0.05,
        "{what}: rinch {rinch}, Chrome {chrome}"
    );
}

/// (1) Chrome adds no letter spacing to a soft hyphen (Blink's
/// `ShapeResultSpacing` skips a zero-width character), nor to the hyphen it
/// draws: with `letter-spacing: 2px`, `Extra-` is 54.64 (each letter of
/// `Extra` 2px wider, the hyphen 5.73) in 51px and 1px, and the word on one
/// line is 127.64 wide, as wide as without the soft hyphen. rinch gave the
/// soft hyphen's cluster 2px: `Extra-` 56.64, the word 129.63.
#[test]
fn letter_spacing_skips_the_soft_hyphen() {
    for width in [51, 1] {
        let laid = breaks(
            "Extra&shy;ordinary",
            width,
            "letter-spacing:2px",
            &["Extra~", "ordinary"],
        );
        close(
            laid.advances[0],
            EXTRA + 5.0 * 2.0 + HYPHEN,
            &format!("Extra- in {width}px"),
        );
        close(laid.advances[1], 78.73, &format!("ordinary in {width}px"));
        let rects = laid.soft_hyphen_rects();
        close(rects[0].0, EXTRA + 10.0, "the hyphen after `Extra`");
        close(rects[0].2, HYPHEN, "the hyphen, no spacing");
    }
    for (markup, chrome) in [
        ("Extra&shy;ordinary", 127.64),
        ("Extraordinary", 127.64),
        ("Ex&shy;tra&shy;or&shy;di&shy;nary", 127.64),
    ] {
        let d = html(&format!(
            "<div style='width:max-content;letter-spacing:2px'><span id=a>{markup}</span></div>"
        ));
        let span = d.tree.get(by_id(&d, "a").0).unwrap();
        let w = d.tree.get(span.parent.unwrap()).unwrap().layout.width;
        assert!(
            (w - chrome).abs() < 0.5,
            "max-content of {markup}: rinch {w}, Chrome {chrome}"
        );
    }
    // A min-content piece that ends at a soft hyphen: `Extraordinary`
    // spaced, then the hyphen, unspaced (Chrome 157: 133.38).
    let d = html(
        "<div style='width:min-content;letter-spacing:2px'><span id=a>Extraordinary&shy;x</span></div>",
    );
    let span = d.tree.get(by_id(&d, "a").0).unwrap();
    let w = d.tree.get(span.parent.unwrap()).unwrap().layout.width;
    assert!(
        (w - 133.38).abs() < 0.5,
        "min-content: rinch {w}, Chrome 133.38"
    );
}

/// (2) A soft hyphen right before the spaces a line breaks at draws its
/// hyphen (Chrome 157): `Extra&shy; ordinary` is `Extra- ` / `ordinary` in
/// 60px, and in 40px and 30px, where the line overflows. The soft hyphen's
/// selection rect is the hyphen, x 38.91, 5.73 wide. rinch drew none: the
/// line was `Extra` and a space, 43.41.
#[test]
fn a_hyphen_before_a_space_break() {
    for width in [60, 40, 30] {
        let laid = breaks("Extra&shy; ordinary", width, "", &["Extra~ ", "ordinary"]);
        close(
            laid.advances[0],
            EXTRA + HYPHEN + SPACE,
            &format!("Extra-, then the space, in {width}px"),
        );
        let rects = laid.soft_hyphen_rects();
        close(rects[0].0, EXTRA, "the hyphen's left");
        close(rects[0].2, HYPHEN, "the hyphen's width");
        close(rects[0].1, 0.0, "on the first line");
    }
    // Two spaces (collapsed to one), a preserved space and a tab after it:
    // the hyphen, then the white space.
    let laid = breaks("Extra&shy;  ordinary", 60, "", &["Extra~ ", "ordinary"]);
    close(
        laid.soft_hyphen_rects()[0].2,
        HYPHEN,
        "two spaces: the hyphen",
    );
    let laid = breaks(
        "Extra&shy; ordinary",
        60,
        "white-space:pre-wrap",
        &["Extra~ ", "ordinary"],
    );
    let rects = laid.soft_hyphen_rects();
    close(rects[0].0, EXTRA, "pre-wrap: the hyphen's left");
    close(rects[0].2, HYPHEN, "pre-wrap: the hyphen's width");
    // Known difference: Chrome draws the hyphen before a preserved tab too
    // (`Extra-`, then the tab from 44.64). rinch lays a tab out as a space
    // of a style of its own, widened to its stop (#1122), and a space of
    // another style keeps no hyphen (below), so it draws none there.
    let laid = breaks(
        "Extra&shy;\tordinary",
        60,
        "white-space:pre-wrap",
        &["Extra~ ", "ordinary"],
    );
    assert!(
        laid.soft_hyphen_rects()[0].2 <= 1.0,
        "known difference: no hyphen before a tab"
    );
    // Only the soft hyphen before the break draws one.
    let laid = breaks(
        "Ex&shy; tra&shy; ordinary",
        50,
        "",
        &["Ex~ tra~ ", "ordinary"],
    );
    close(laid.advances[0], 43.09 + HYPHEN + SPACE, "Ex tra- ");
    // Chrome draws it under `hyphens: none` too: the break is the space's,
    // which `hyphens` does not take away.
    let laid = breaks(
        "Extra&shy; ordinary",
        60,
        "hyphens:none",
        &["Extra~ ", "ordinary"],
    );
    close(laid.advances[0], EXTRA + HYPHEN + SPACE, "hyphens:none");
    // Not when the space is another element's, bold or not. (Chrome asks
    // whether it is the soft hyphen's own text node, and draws none after
    // `Extra&shy;` and ` ordinary` as two text nodes either; rinch asks
    // whether the space has the soft hyphen's style, and draws one there.)
    for markup in [
        "Extra&shy;<b> </b>ordinary",
        "Extra&shy;<span> </span>ordinary",
    ] {
        let laid = breaks(markup, 60, "", &["Extra~ ", "ordinary"]);
        close(
            laid.advances[0],
            EXTRA + SPACE,
            &format!("{markup}: a space of its own"),
        );
    }
    // Nor when the line does not break there.
    let laid = breaks("Extra&shy; ordinary", 200, "", &["Extra~ ordinary"]);
    close(laid.advances[0], 106.14, "one line");
    // Before an ideographic space Chrome draws the hyphen too (`Extra-`,
    // then U+3000 from 44.64). Inter has no U+3000, and whether rinch draws
    // it depends on the host's fonts: parley draws the hyphen only when the
    // soft hyphen is in the line's last run, and a fallback font may shape
    // the space as a run of its own (a known difference; on nico-top, whose
    // fonts are CI's, it is drawn). Only the lines are pinned.
    breaks(
        "Extra&shy;&#x3000;ordinary",
        60,
        "",
        &["Extra~\u{3000}", "ordinary"],
    );
}

/// (2), fitting: The line is fitted with the hyphen, its spaces hanging:
/// `123 Extra- ` (content 75.3) stays on one line in 76px, and in 75px
/// `Extra` moves down; without the soft hyphen 70px is enough. Through
/// rinch's own breaks too (a text seam before it).
#[test]
fn a_hyphen_before_a_space_break_is_fitted() {
    let markup = "123 Extra&shy; ordinary";
    breaks(markup, 76, "", &["123 Extra~ ", "ordinary"]);
    breaks(markup, 75, "", &["123 ", "Extra~ ", "ordinary"]);
    breaks(markup, 73, "", &["123 ", "Extra~ ", "ordinary"]);
    breaks("123 Extra ordinary", 70, "", &["123 Extra ", "ordinary"]);
    // `abc-123 Extra` is 104.84, `abc-123 Extra- ` 110.57 less its space.
    let seamed = format!("{TEXT_SEAM}Extra&shy; ordinary");
    breaks(&seamed, 110, "", &["abc-123 ", "Extra~ ordinary"]);
    let laid = breaks(&seamed, 113, "", &["abc-123 Extra~ ", "ordinary"]);
    close(
        laid.advances[0],
        110.57 + SPACE,
        "abc-123 Extra- after the seam",
    );
    // Through rinch's own breaks, a space of another element keeps no
    // hyphen: `abc-123 Extra` and the space fit 108px. Known difference:
    // Chrome fits that line with the hyphen it does not draw, and in 108px
    // puts `Extra` on the next line (`abc-123 ` / `Extra~ ordinary`; it keeps
    // `abc-123 Extra~ ` from 111px).
    breaks(
        &format!("{TEXT_SEAM}Extra&shy;<b> </b>ordinary"),
        108,
        "",
        &["abc-123 Extra~ ", "ordinary"],
    );
    // Nor does a `break-spaces` space, which never hangs (Chrome 157: in
    // 110px `abc-123 Extra ` is 109.35 with its space, no hyphen; in 109px
    // `Extra` moves down).
    let spaces = format!("{TEXT_SEAM}Extra&shy; ordinary");
    let laid = breaks(
        &spaces,
        110,
        "white-space:break-spaces",
        &["abc-123 Extra~ ", "ordinary"],
    );
    close(laid.advances[0], 109.35, "break-spaces: no hyphen");
    assert!(
        laid.soft_hyphen_rects()[0].2 <= 1.0,
        "break-spaces: none drawn"
    );
    breaks(
        &spaces,
        109,
        "white-space:break-spaces",
        &["abc-123 ", "Extra~ ordinary"],
    );
    let laid = breaks(
        &format!("{TEXT_SEAM}Ex&shy; ordinary"),
        63,
        "",
        &["abc-123 ", "Ex~ ", "ordinary"],
    );
    close(
        laid.advances[1],
        18.36 + HYPHEN + SPACE,
        "Ex- after the seam",
    );
    // A min-content piece that ends at the space counts the hyphen
    // (Chrome 157: 107.38 against `Extraordinary`'s 101.64), unless the
    // space is another element's.
    for (markup, chrome) in [
        ("Extraordinary&shy; x", 107.38),
        ("Extraordinary x", 101.64),
        ("Extraordinary&shy;<b> </b>x", 101.64),
    ] {
        let d = html(&format!(
            "<div style='width:min-content'><span id=a>{markup}</span></div>"
        ));
        let span = d.tree.get(by_id(&d, "a").0).unwrap();
        let w = d.tree.get(span.parent.unwrap()).unwrap().layout.width;
        assert!(
            (w - chrome).abs() < 0.5,
            "min-content of {markup}: rinch {w}, Chrome {chrome}"
        );
    }
}

/// (3) The soft hyphen's selection rect starts after an element's start
/// border: `Extra<span border-left:3px>&shy;ordinary</span>` in 50px, Chrome
/// 157's rect of the soft hyphen is x 41.91, 5.73 wide. rinch's took in the
/// border, x 38.91, 8.73 wide. The same for a letter after the border.
#[test]
fn the_soft_hyphens_rect_starts_after_a_start_border() {
    let laid = breaks(
        "Extra<span style='border-left:3px solid red'>&shy;ordinary</span>",
        50,
        "",
        &["Extra~", "ordinary"],
    );
    let rects = laid.soft_hyphen_rects();
    assert_eq!(rects.len(), 1, "{rects:?}");
    close(
        rects[0].0,
        EXTRA + 3.0,
        "the hyphen's left, after the border",
    );
    close(rects[0].2, HYPHEN, "the hyphen's width");
    // Border and padding.
    let laid = breaks(
        "Ext<span style='border-left:3px solid red;padding-left:2px'>ra&shy;ordinary</span>",
        50,
        "",
        &["Extra~", "ordinary"],
    );
    let rects = laid.soft_hyphen_rects();
    close(
        rects[0].0,
        43.92,
        "after `ra` behind the border and padding",
    );
    close(rects[0].2, HYPHEN, "the hyphen's width");
    let ra = rinch_dom::text_query::selection_rects_for_layout(laid.layout(), 3, 5);
    close(ra[0].0, 28.91, "`ra` starts after the border and padding");
}

/// (4) rinch's own breaks find the hyphen past an element's end edge where
/// parley's break does not decide: a text seam, then a soft hyphen at the
/// end of a bordered span whose line overflows only with the hyphen and the
/// border (Chrome 157: `abc-123 Ex<span border-right:3px>tra&shy;</span>
/// ordinaryordinary` in 110px is `abc-123 ` / `Extra-` / `ordinaryordinary`,
/// in 114px `abc-123 Extra-` / `ordinaryordinary`: `abc-123 Extra-` and the
/// border are 113.57, 107.84 without the hyphen).
#[test]
fn the_seams_pass_looks_past_an_end_edge() {
    let markup = format!(
        "{TEXT_SEAM}Ex<span style='border-right:3px solid red'>tra&shy;</span>ordinaryordinary"
    );
    let laid = breaks(&markup, 114, "", &["abc-123 Extra~", "ordinaryordinary"]);
    close(laid.advances[0], 113.57, "abc-123 Extra- and the border");
    breaks(
        &markup,
        110,
        "",
        &["abc-123 ", "Extra~", "ordinaryordinary"],
    );
}
