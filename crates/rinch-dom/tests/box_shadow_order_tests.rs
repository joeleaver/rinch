//! #1009: of several **outer** box-shadows, the first is on top.
//!
//! css-backgrounds-3 §7.1: "The shadow effects are applied front-to-back: the
//! first shadow is on top and the others are layered behind." rinch painted
//! the outer shadows in list order with source-over fills, so the **last** one
//! ended up on top. (The inset path, `paint_inset_box_shadow`, already painted
//! in reverse; `box_shadow_inset_tests::the_first_inset_shadow_is_on_top` is
//! its pin.)
//!
//! Every expected colour below is what Chrome 153 paints for the same boxes
//! (headless screenshot at DPR 1, same pixels read). The boxes have no
//! background, so an outer shadow is visible only outside the border box.
//!
//! The fixtures are off the fixed point on purpose: a two-shadow list whose
//! first shadow is the *smaller* one passes against a "paint the biggest
//! spread first" sort as well as against the reversal, so the second fixture
//! puts the bigger one first.

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const BOX: i64 = 100;
const LEFT: i64 = 150;
const TOP: i64 = 150;

fn painted(style: &str) -> TinySkiaPainter {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        &format!(
            "position: absolute; left: {LEFT}px; top: {TOP}px; width: {BOX}px; height: {BOX}px; {style}"
        ),
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);
    let mut painter = TinySkiaPainter::new(800, 600);
    let mut paint_layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (800.0, 600.0),
        &mut doc.font_cx,
        &mut paint_layout_cx,
    );
    painter
}

/// `[r, g, b, a]` of one pixel, 0..=255 (premultiplied, as tiny-skia stores
/// it — exact for the opaque pixels read here).
fn rgba(p: &TinySkiaPainter, x: i64, y: i64) -> [u8; 4] {
    let idx = ((y as u32 * p.width() + x as u32) * 4) as usize;
    let px = &p.pixels()[idx..idx + 4];
    [px[0], px[1], px[2], px[3]]
}

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];
const NONE: [u8; 4] = [0, 0, 0, 0];

fn assert_px(p: &TinySkiaPainter, x: i64, y: i64, want: [u8; 4], what: &str) {
    assert_eq!(rgba(p, x, y), want, "{what} at ({x}, {y})");
}

/// The issue's example: a red 4px ring inside a blue 8px one. Chrome: red at
/// 2px outside the box, blue at 6px.
#[test]
fn the_first_outer_shadow_is_on_top() {
    let p = painted("box-shadow: 0 0 0 4px rgb(255,0,0), 0 0 0 8px rgb(0,0,255)");
    let mid = TOP + 50;
    assert_px(&p, LEFT - 2, mid, RED, "2px out: the first shadow");
    assert_px(&p, LEFT - 6, mid, BLUE, "6px out: the second shadow");
    assert_px(&p, LEFT - 9, mid, NONE, "9px out: neither");
}

/// The bigger shadow first hides the smaller one entirely. Chrome: blue at
/// 2px and at 6px.
#[test]
fn a_bigger_first_shadow_hides_a_smaller_second() {
    let p = painted("box-shadow: 0 0 0 8px rgb(0,0,255), 0 0 0 4px rgb(255,0,0)");
    let mid = TOP + 50;
    assert_px(&p, LEFT - 2, mid, BLUE, "2px out: the first shadow covers");
    assert_px(&p, LEFT - 6, mid, BLUE, "6px out: the first shadow");
}

/// Three offset shadows: each is visible exactly where no earlier one covers.
/// Chrome: red at 2px right of the box, green at 7px, blue at 15px.
#[test]
fn three_offset_shadows_stack_first_on_top() {
    let p = painted(
        "box-shadow: 5px 0 0 0 rgb(255,0,0), 10px 0 0 0 rgb(0,255,0), 20px 0 0 0 rgb(0,0,255)",
    );
    let mid = TOP + 50;
    let right = LEFT + BOX;
    assert_px(&p, right + 2, mid, RED, "all three cover: the first");
    assert_px(
        &p,
        right + 7,
        mid,
        GREEN,
        "the second and third cover: the second",
    );
    assert_px(&p, right + 15, mid, BLUE, "only the third covers");
    assert_px(&p, right + 21, mid, NONE, "none covers");
}

/// Outer and inset shadows interleaved in one list: each kind keeps its own
/// order, first on top, and neither disturbs the other. Chrome: red at 3px
/// outside, blue at 9px; green at depth 2, black at depth 7.
#[test]
fn interleaved_outer_and_inset_shadows_each_keep_first_on_top() {
    let p = painted(
        "box-shadow: inset 0 0 0 5px rgb(0,255,0), 0 0 0 6px rgb(255,0,0), \
         inset 0 0 0 10px rgb(0,0,0), 0 0 0 12px rgb(0,0,255)",
    );
    let mid = TOP + 50;
    assert_px(&p, LEFT - 3, mid, RED, "3px out: the first outer shadow");
    assert_px(&p, LEFT - 9, mid, BLUE, "9px out: the second outer shadow");
    assert_px(&p, LEFT + 2, mid, GREEN, "depth 2: the first inset shadow");
    assert_px(&p, LEFT + 7, mid, BLACK, "depth 7: the second inset shadow");
}
