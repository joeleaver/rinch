//! A box sized fit-content whose lines wrap is as wide as its room, not its
//! widest line (#2330, #1276). Fit-content is `min(max-content,
//! max(min-content, available))` (CSS Sizing 3 §5.2): content that wraps
//! has a max-content width past the room, so the box takes the room, or its
//! min-content width when a piece that cannot break is wider still. Each
//! number is Chrome 156's for the same markup in a 560px column.
//!
//! The paragraph's measure answered the width of the lines it broke (a float
//! holding `alpha beta … kappa` in 300px was 277 wide, Chrome 300), wherever
//! a box takes that measure: a float, an absolutely positioned box, a flex
//! or grid item aligned to its start.
//!
//! Most content is two `vertical-align: top` inline-blocks with a space
//! between them, wider together than the room, so the line breaks at the
//! space and no width depends on a font; the text cases use the bundled Inter.
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const CSS: &str = "body { margin: 0; font-family: InterT; font-size: 16px; line-height: 20px; }";
const TEXT: &str = "alpha beta gamma delta epsilon zeta eta theta iota kappa";

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

/// The border box of `#id`: x and width, in page coordinates.
fn x_w(d: &RinchDocument, id: &str) -> (f32, f32) {
    let n = d.query_selector(&format!("#{id}")).expect(id);
    let w = d.tree.get(n.0).unwrap().layout.width;
    let mut x = 0.0;
    let mut cur = Some(n.0);
    while let Some(i) = cur {
        let node = d.tree.get(i).unwrap();
        x += node.layout.x;
        cur = node.parent;
    }
    (x, w)
}

/// Within half a pixel: Chrome lays out in fractions, rinch rounds.
#[track_caller]
fn near(got: (f32, f32), chrome: (f32, f32)) {
    assert!(
        (got.0 - chrome.0).abs() <= 0.5 && (got.1 - chrome.1).abs() <= 0.5,
        "rinch (x, w) {got:?}, Chrome {chrome:?}"
    );
}

fn ib(w: u32) -> String {
    format!("<span style='display:inline-block;vertical-align:top;width:{w}px;height:20px'></span>")
}

/// 170 + 170 with a breakable space between: wraps in anything under 344.
fn two() -> String {
    format!("{} {}", ib(170), ib(170))
}

// --- Floats -------------------------------------------------------------

/// The issue's shape: a right float holding a block of wrapping text sits at
/// the room's left edge, 300 wide. It was 277 wide at x 23.
#[test]
fn a_right_float_holding_wrapping_text_takes_its_room() {
    let d = doc(&format!(
        "<div style='width:300px'><div id=f style='float:right'><div>{TEXT}</div></div></div>"
    ));
    near(x_w(&d, "f"), (0.0, 300.0));
}

#[test]
fn a_left_float_holding_wrapping_boxes_takes_its_room() {
    let d = doc(&format!(
        "<div style='width:300px'><div id=f style='float:left'>{}</div></div>",
        two()
    ));
    near(x_w(&d, "f"), (0.0, 300.0));
}

/// Padding and border come out of the room: the border box is 300.
#[test]
fn padding_and_border_come_out_of_the_room() {
    let d = doc(&format!(
        "<div style='width:300px'>\
         <div id=f style='float:left;padding:0 13px;border:2px solid'>{}</div></div>",
        two()
    ));
    near(x_w(&d, "f"), (0.0, 300.0));
}

/// `max-width` caps the room: 200.
#[test]
fn max_width_caps_the_room() {
    let d = doc(&format!(
        "<div style='width:300px'><div id=f style='float:left;max-width:200px'>{}</div></div>",
        two()
    ));
    near(x_w(&d, "f"), (0.0, 200.0));
}

/// A piece wider than the room: the box is as wide as that piece (its
/// min-content width), 137 in 100.
#[test]
fn a_piece_wider_than_the_room_is_the_width() {
    let d = doc(&format!(
        "<div style='width:100px'><div id=f style='float:left'>{} {}</div></div>",
        ib(137),
        ib(50)
    ));
    near(x_w(&d, "f"), (0.0, 137.0));
}

/// **Control**: content that does not wrap is its max-content width.
#[test]
fn content_that_does_not_wrap_is_its_own_width() {
    let d = doc(&format!(
        "<div style='width:300px'><div id=f style='float:left'>{}</div></div>",
        ib(170)
    ));
    near(x_w(&d, "f"), (0.0, 170.0));
}

/// **Control**: a forced break is no wrap; the widest line is the width.
#[test]
fn a_forced_break_is_not_a_wrap() {
    let d = doc(&format!(
        "<div style='width:300px'><div id=f style='float:left'>{}<br>{}</div></div>",
        ib(170),
        ib(120)
    ));
    near(x_w(&d, "f"), (0.0, 170.0));
}

/// The #2326 shape: an inline-block capped at 300 holding a right float and
/// a block, each of wrapping content. The float takes the 300.
#[test]
fn a_float_in_a_capped_inline_block() {
    let d = doc(&format!(
        "<span id=i style='display:inline-block;max-width:300px'>\
         <div id=f style='float:right'><div>{t}</div></div><div>{t}</div></span>",
        t = two()
    ));
    near(x_w(&d, "i"), (0.0, 300.0));
    near(x_w(&d, "f"), (0.0, 300.0));
}

// --- Other fit-content boxes --------------------------------------------

/// An absolutely positioned box at `left: 37px` has 263 of its containing
/// block's 300.
#[test]
fn an_absolutely_positioned_box_takes_its_room() {
    let d = doc(&format!(
        "<div style='width:300px;position:relative;height:60px'>\
         <div id=a style='position:absolute;left:37px'>{}</div></div>",
        two()
    ));
    near(x_w(&d, "a"), (37.0, 263.0));
}

#[test]
fn a_column_flex_item_aligned_to_its_start_takes_its_room() {
    let d = doc(&format!(
        "<div style='display:flex;flex-direction:column;align-items:flex-start;width:300px'>\
         <div id=x>{}</div></div>",
        two()
    ));
    near(x_w(&d, "x"), (0.0, 300.0));
}

#[test]
fn a_grid_item_justified_to_its_start_takes_its_room() {
    let d = doc(&format!(
        "<div style='display:grid;justify-items:start;width:300px'><div id=g>{}</div></div>",
        two()
    ));
    near(x_w(&d, "g"), (0.0, 300.0));
}

/// **Control**: a row flex item's base size is its max-content width, then
/// it shrinks; unchanged. Chrome: 260 beside a 40px item.
#[test]
fn a_row_flex_item_still_shrinks_from_its_max_content() {
    let d = doc(&format!(
        "<div style='display:flex;width:300px'><div id=x>{}</div><div id=y>{}</div></div>",
        two(),
        ib(40)
    ));
    near(x_w(&d, "x"), (0.0, 260.0));
    near(x_w(&d, "y"), (260.0, 40.0));
}

/// Text that fits is centred in its own width (no wrap), 284.5 in Chrome.
#[test]
fn text_that_fits_keeps_its_own_width() {
    let d = doc(&format!(
        "<div style='width:300px'><div id=f style='float:left;text-align:center'>{} {}</div></div>",
        ib(170),
        ib(110)
    ));
    near(x_w(&d, "f"), (0.0, 284.5));
}
