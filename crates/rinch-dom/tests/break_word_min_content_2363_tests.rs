//! `overflow-wrap: break-word` adds no soft wrap opportunity to the
//! min-content width (#2363; CSS Text 3 §5.2: "soft wrap opportunities
//! introduced by the break are not considered when calculating min-content
//! intrinsic sizes"). `overflow-wrap: anywhere` and `word-break: break-all`
//! do. So a box sized by its content (a float, an inline-block, an absolutely
//! positioned box, a flex or grid item, a table cell, `width: min-content`)
//! holding a word wider than its room takes the whole word's width and keeps
//! it on one line, where `anywhere` breaks the word at the room. A box whose
//! width is given still breaks the word under `break-word` when it lays its
//! lines out.
//!
//! rinch broke the word at the room: the measure laid the text out at the
//! room (or at 0, for a min-content width), where `break-word` let parley
//! break the word (an emergency break), and took the broken lines' width.
//!
//! Each number is Chrome 157's for the same markup (bundled Inter, 16px/20px,
//! in a 560px column). `Supercalifragilistic` is 139.70 wide; the rooms are
//! 100 and 83.
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const CSS: &str = "body { margin: 0; font-family: InterT; font-size: 16px; line-height: 20px; }";
const WORD: &str = "Supercalifragilistic";

fn doc(html: &str) -> RinchDocument {
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
    d.load_css(CSS);
    let body = d.body();
    d.set_inner_html(body, &format!("<div style='width:560px'>{html}</div>"));
    d.resolve_layout(800.0, 800.0);
    d
}

/// The border box size of `#id`.
fn w_h(d: &RinchDocument, id: &str) -> (f32, f32) {
    let n = d.query_selector(&format!("#{id}")).expect(id);
    let l = &d.tree.get(n.0).unwrap().layout;
    (l.width, l.height)
}

/// Within half a pixel: Chrome lays out in fractions, rinch rounds.
#[track_caller]
fn near(d: &RinchDocument, id: &str, chrome: (f32, f32)) {
    let got = w_h(d, id);
    assert!(
        (got.0 - chrome.0).abs() <= 0.5 && (got.1 - chrome.1).abs() <= 0.5,
        "#{id}: rinch (w, h) {got:?}, Chrome {chrome:?}"
    );
}

fn in_room(room: u32, inner: &str) -> String {
    format!("<div style='width:{room}px'>{inner}</div>")
}

// --- Floats --------------------------------------------------------------

/// The issue's case: the float is the word's width, on one line. It was
/// 100 x 40, the word broken at the room.
#[test]
fn a_break_word_float_takes_the_whole_word() {
    let d = doc(&in_room(
        100,
        &format!("<div id=f style='float:left;overflow-wrap:break-word'>{WORD}</div>"),
    ));
    near(&d, "f", (139.70, 20.0));
}

/// `anywhere` does count: the float takes the room and breaks the word.
#[test]
fn an_anywhere_float_takes_the_room() {
    let d = doc(&in_room(
        100,
        &format!("<div id=f style='float:left;overflow-wrap:anywhere'>{WORD}</div>"),
    ));
    near(&d, "f", (100.0, 40.0));
}

/// So does `word-break: break-all`.
#[test]
fn a_break_all_float_takes_the_room() {
    let d = doc(&in_room(
        100,
        &format!("<div id=f style='float:left;word-break:break-all'>{WORD}</div>"),
    ));
    near(&d, "f", (100.0, 40.0));
}

/// The word with more text after it, in 83: the float is the word's width
/// and the rest wraps at that width (two lines), not at the room's.
#[test]
fn a_break_word_float_lays_its_lines_out_at_the_word_width() {
    let d = doc(&in_room(
        83,
        &format!("<div id=f style='float:left;overflow-wrap:break-word'>{WORD} is a word</div>"),
    ));
    near(&d, "f", (139.70, 40.0));
}

/// `break-word` set on a span inside the float, not on the float. This
/// passed before the fix too, for another reason: rinch does not yet honour
/// `overflow-wrap` set on an inline span (#2625), so the word was never
/// broken. It pins Chrome's answer for when it does.
#[test]
fn a_break_word_span_in_a_float_counts_the_same() {
    let d = doc(&in_room(
        100,
        &format!(
            "<div id=f style='float:left'><span style='overflow-wrap:break-word'>{WORD}</span></div>"
        ),
    ));
    near(&d, "f", (139.70, 20.0));
}

// --- Inline-blocks and absolutely positioned boxes ----------------------

#[test]
fn a_break_word_inline_block_takes_the_whole_word() {
    let d = doc(&in_room(
        100,
        &format!("<span id=b style='display:inline-block;overflow-wrap:break-word'>{WORD}</span>"),
    ));
    near(&d, "b", (139.70, 20.0));
}

#[test]
fn an_anywhere_inline_block_takes_the_room() {
    let d = doc(&in_room(
        100,
        &format!("<span id=b style='display:inline-block;overflow-wrap:anywhere'>{WORD}</span>"),
    ));
    near(&d, "b", (100.0, 40.0));
}

#[test]
fn a_break_word_absolute_box_takes_the_whole_word() {
    let d = doc(&in_room(
        100,
        &format!(
            "<div style='position:relative;height:0'>\
             <div id=a style='position:absolute;overflow-wrap:break-word'>{WORD}</div></div>"
        ),
    ));
    near(&d, "a", (139.70, 20.0));
}

// --- Flex and grid items ---------------------------------------------------

/// A flex item's automatic minimum is its min-content width: it does not
/// shrink below the word.
#[test]
fn a_break_word_flex_item_does_not_shrink_below_the_word() {
    let d = doc(&format!(
        "<div style='width:100px;display:flex'>\
         <div id=i style='overflow-wrap:break-word'>{WORD}</div></div>"
    ));
    near(&d, "i", (139.70, 20.0));
}

#[test]
fn an_anywhere_flex_item_shrinks_to_the_room() {
    let d = doc(&format!(
        "<div style='width:100px;display:flex'>\
         <div id=i style='overflow-wrap:anywhere'>{WORD}</div></div>"
    ));
    near(&d, "i", (100.0, 40.0));
}

/// The flex container's own text (an anonymous item, a text leaf): one line
/// under `break-word`, so the container is 20 tall; two under `anywhere`.
#[test]
fn a_flex_containers_own_break_word_text_stays_on_one_line() {
    let d = doc(&format!(
        "<div id=c style='width:100px;display:flex;overflow-wrap:break-word'>{WORD}</div>"
    ));
    near(&d, "c", (100.0, 20.0));
}

#[test]
fn a_flex_containers_own_anywhere_text_breaks() {
    let d = doc(&format!(
        "<div id=c style='width:100px;display:flex;overflow-wrap:anywhere'>{WORD}</div>"
    ));
    near(&d, "c", (100.0, 40.0));
}

#[test]
fn a_break_word_grid_item_in_an_auto_track_takes_the_word() {
    let d = doc(&format!(
        "<div style='width:100px;display:grid;grid-template-columns:auto'>\
         <div id=i style='overflow-wrap:break-word'>{WORD}</div></div>"
    ));
    near(&d, "i", (139.70, 20.0));
}

#[test]
fn a_break_word_min_content_track_is_the_word() {
    let d = doc(&format!(
        "<div style='width:100px;display:grid;grid-template-columns:min-content'>\
         <div id=i style='overflow-wrap:break-word'>{WORD}</div></div>"
    ));
    near(&d, "i", (139.70, 20.0));
}

/// Under `anywhere` a min-content track is the widest letter: one per line.
#[test]
fn an_anywhere_min_content_track_is_one_letter() {
    let d = doc(&format!(
        "<div style='width:100px;display:grid;grid-template-columns:min-content'>\
         <div id=i style='overflow-wrap:anywhere'>{WORD}</div></div>"
    ));
    near(&d, "i", (10.27, 340.0));
}

// --- Table cells and min-content widths ----------------------------------

#[test]
fn a_break_word_table_cell_takes_the_word() {
    let d = doc(&in_room(
        100,
        &format!(
            "<table id=t style='border-collapse:collapse'><tr>\
             <td id=c style='padding:0;overflow-wrap:break-word'>{WORD}</td></tr></table>"
        ),
    ));
    near(&d, "c", (139.70, 20.0));
    near(&d, "t", (139.70, 20.0));
}

#[test]
fn an_anywhere_table_cell_takes_the_room() {
    let d = doc(&in_room(
        100,
        &format!(
            "<table id=t style='border-collapse:collapse'><tr>\
             <td id=c style='padding:0;overflow-wrap:anywhere'>{WORD}</td></tr></table>"
        ),
    ));
    near(&d, "c", (100.0, 40.0));
}

#[test]
fn a_break_word_min_content_width_is_the_word() {
    let d = doc(&format!(
        "<div id=m style='width:min-content;overflow-wrap:break-word'>{WORD}</div>"
    ));
    near(&d, "m", (139.70, 20.0));
}

#[test]
fn an_anywhere_min_content_width_is_one_letter() {
    let d = doc(&format!(
        "<div id=m style='width:min-content;overflow-wrap:anywhere'>{WORD}</div>"
    ));
    near(&d, "m", (10.27, 340.0));
}

// --- A given width still breaks the word ---------------------------------

/// `break-word` still breaks a word wider than a box whose width is given.
#[test]
fn a_break_word_block_of_given_width_breaks_the_word() {
    let d = doc(&format!(
        "<div id=b style='width:100px;overflow-wrap:break-word'>{WORD}</div>"
    ));
    near(&d, "b", (100.0, 40.0));
}

#[test]
fn a_break_word_block_of_given_width_breaks_the_word_among_others() {
    let d = doc(&format!(
        "<div id=b style='width:83px;overflow-wrap:break-word'>{WORD} is a word</div>"
    ));
    near(&d, "b", (83.0, 60.0));
}
