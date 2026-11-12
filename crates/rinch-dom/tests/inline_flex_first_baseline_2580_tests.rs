//! An `inline-flex`'s baseline when its first item has no line box (#2580).
//!
//! A flex container's first baseline is its first item's, and an item with
//! no baseline has one synthesized from its border box: its bottom edge
//! (css-flexbox-1 §8.5, css-align-3 §9.1). An item holding only collapsible
//! white space has no line box (CSS 2.1 §9.4.2), so no baseline either.
//! rinch took the empty IFC's one parley line (baseline 0, the line grew to
//! 36) and, for an empty item, went on to the next item's line (the line
//! stayed 20), where Chrome lines both up by the first item's bottom.
//!
//! Every number is Chrome 157's (headless, the bundled Inter at 16px/20px, a
//! 360px block holding `word` and the chip): the line's height and the
//! chip's top in it. The text's baseline is 16px down its 20px line, so a
//! chip whose baseline is `b` px down stands `16 - b` px from the line top
//! when that is below it, and grows the line otherwise.
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn doc(chip: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut d = RinchDocument::new();
    let registered = d.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("InterT"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    d.load_css(
        "body { margin: 0; font-family: InterT; font-size: 16px; line-height: 20px; } p { margin: 0 }",
    );
    let body = d.body();
    d.set_inner_html(
        body,
        &format!("<div style='width:560px'><div id=c style='width:360px'>word{chip}</div></div>"),
    );
    d.resolve_layout(800.0, 800.0);
    d
}

/// The line's height (the block holds one line) and the chip `#t`'s top in it.
#[track_caller]
fn line_and_chip(chip: &str, chrome: (f32, f32)) {
    let mut d = doc(chip);
    let c = d.query_selector("#c").unwrap();
    let t = d.query_selector("#t").unwrap();
    let line = d.bounding_rect(c).unwrap();
    let got = (line.height, d.bounding_rect(t).unwrap().y - line.y);
    assert!(
        (got.0 - chrome.0).abs() <= 0.5 && (got.1 - chrome.1).abs() <= 0.5,
        "{chip}: rinch (line height, chip top) {got:?}, Chrome {chrome:?}"
    );
}

/// The issue's two shapes, which Chrome lines up alike: the first item's
/// bottom, 20px down the stretched chip, on the baseline. (rinch: 36 / 16
/// with the white space, 20 / 0 empty.)
#[test]
fn a_first_item_of_white_space_or_nothing_synthesizes_its_baseline_2580() {
    for inner in [" ", ""] {
        line_and_chip(
            &format!(
                "<div id=t style='display:inline-flex'><p style='display:inline'>{inner}</p>a<em style='display:flex;flex-direction:column'><p></p></em></div>"
            ),
            (24.0, 0.0),
        );
    }
}

/// Padding on the white-space item: still its border-box bottom, the
/// chip's 20. (rinch: 33 / 13.)
#[test]
fn a_padded_white_space_item_synthesizes_from_its_border_box_2580() {
    line_and_chip(
        "<div id=t style='display:inline-flex'><p style='display:inline;padding-top:3px'> </p>a</div>",
        (24.0, 0.0),
    );
}

/// An empty 7px item: the baseline 7px down the chip, which stands 9px
/// down a 29px line. (rinch: the `a`'s line, 20 / 0.)
#[test]
fn an_empty_first_item_lines_up_by_its_bottom_2580() {
    line_and_chip(
        "<div id=t style='display:inline-flex'><div style='height:7px'></div>a</div>",
        (29.0, 9.0),
    );
}

/// Its margins are not part of its border box: 4 + 7 down the chip.
/// (rinch: 20 / 0.)
#[test]
fn an_empty_first_item_with_margins_lines_up_by_its_border_box_2580() {
    line_and_chip(
        "<div id=t style='display:inline-flex'><div style='height:7px;margin-top:4px;margin-bottom:6px'></div>a</div>",
        (25.0, 5.0),
    );
}

/// A column's first item likewise. (rinch: 27 / 0, by the `a`'s line below
/// it.)
#[test]
fn a_column_whose_first_item_is_empty_2580() {
    line_and_chip(
        "<div id=t style='display:inline-flex;flex-direction:column'><div style='height:7px'></div>a</div>",
        (36.0, 9.0),
    );
}

/// A control: a first item with a line keeps it, past an empty second one.
#[test]
fn a_first_item_with_a_line_keeps_its_baseline_2580() {
    line_and_chip(
        "<div id=t style='display:inline-flex'><div>b</div><div style='height:7px'></div>a</div>",
        (20.0, 0.0),
    );
}

/// A control: white space before the first item makes no item.
#[test]
fn white_space_before_the_first_item_is_no_item_2580() {
    line_and_chip(
        "<div id=t style='display:inline-flex'>\n  <div>b</div><div style='height:7px'></div>a</div>",
        (20.0, 0.0),
    );
}
