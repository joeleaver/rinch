//! Four paint issues on the software painter (tiny-skia); the scenes and
//! their expected pixels are in `support/paint_decorations_2396.rs`, shared
//! with `gpu_paint_decorations_2396` (in `rinch`), which holds the GPU
//! painter to the same checks.
//!
//! - #2335: an `<img>` drew only a colour background, its picture and its
//!   border: no `box-shadow`, `outline` or `background-image`.
//! - #2375: a rounded border in several colours was a round-capped stroke
//!   per side, and the caps overlapped on each corner's diagonal, covering
//!   its edge pixels twice.
//! - #2396: an inline element's border was four solid strips whatever its
//!   style.
//! - #2373: a line-through (and an underline) was drawn at its fractional
//!   position, blended over two rows, where Chrome snaps it to whole rows.

#![cfg(feature = "software-renderer")]

#[path = "support/paint_decorations_2396.rs"]
mod scene;

use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

/// `doc` painted over white at `scale` on a `w` x `h` CSS px viewport.
fn paint(doc: &mut RinchDocument, (w, h): (u32, u32), scale: f64) -> (u32, Vec<[u8; 4]>) {
    let (pw, ph) = (
        (w as f64 * scale).round() as u32,
        (h as f64 * scale).round() as u32,
    );
    let mut painter = TinySkiaPainter::new(pw, ph);
    painter.fill_white();
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale,
        (w as f32, h as f32),
        &mut doc.font_cx,
        &mut cx,
    );
    (pw, painter.pixels().as_chunks::<4>().0.to_vec())
}

fn assert_none(what: &str, bad: Vec<String>) {
    assert!(bad.is_empty(), "{what}:\n{}", bad.join("\n"));
}

#[test]
fn an_img_draws_its_shadows_outline_and_background_image_as_chrome_does() {
    use scene::img::*;
    let (mut doc, _) = document(BOXES);
    let (_, px) = paint(&mut doc, (W, H), 1.0);
    assert_none("#2335", misses(&px));
}

/// The frame that gives an `<img>` a shadow and an outline damages where
/// they paint, and the frame that takes them away damages where they were.
#[test]
fn an_imgs_new_shadow_and_outline_are_damaged() {
    use scene::img::*;
    let plain = "left: 10px; top: 10px";
    let decorated = "left: 10px; top: 10px; box-shadow: 30px 20px 0 rgb(0, 0, 255); \
                     outline: 3px solid rgb(0, 128, 0)";
    let (mut doc, ids) = document(&[plain]);
    paint(&mut doc, (W, H), 1.0);
    // What the painted frame drew, recorded as a frame records it.
    doc.tree.consume_paint_dirty();
    for (style, what) in [(decorated, "added"), (plain, "removed")] {
        rinch_core::dom::DomDocument::set_attribute(&mut doc, ids[0], "style", style);
        doc.resolve_layout(W as f32, H as f32);
        let region = rinch_dom::paint::compute_dirty_region(&doc.tree, 1.0, W as f64, H as f64)
            .unwrap_or_else(|| panic!("{what}: no damage"));
        // The shadow is the box moved by (30, 20): 40..60 x 30..50. The
        // outline is 3px around the box: 7..33 x 7..33.
        for (x, y) in [(59.5, 49.5), (7.5, 7.5), (32.5, 7.5)] {
            assert!(
                region.contains(peniko::kurbo::Point::new(x, y)),
                "{what}: the damage {region:?} misses ({x}, {y})"
            );
        }
        paint(&mut doc, (W, H), 1.0);
        doc.tree.consume_paint_dirty();
    }
}

fn sides(scale: f64) {
    use scene::sides::*;
    let (pw, one) = paint(&mut document(ONE), (W, H), scale);
    let (_, four) = paint(&mut document(FOUR), (W, H), scale);
    assert_none(&format!("#2375 at {scale}x"), misses(&one, &four, pw));
}

#[test]
fn a_rounded_border_in_four_colours_covers_what_one_colour_does_at_1x() {
    sides(1.0);
}

#[test]
fn a_rounded_border_in_four_colours_covers_what_one_colour_does_at_2x() {
    sides(2.0);
}

#[test]
fn an_inline_elements_dashed_dotted_and_groove_borders_are_chromes() {
    use scene::inline::*;
    let (_, px) = paint(&mut document(), (W, H), 1.0);
    assert_none("#2396", misses(&px));
}

fn decorations(scale: f64) {
    use scene::decorations::*;
    let (pw, px) = paint(&mut document(), (W, H), scale);
    assert_none(&format!("#2373 at {scale}x"), misses(&px, pw, scale));
}

#[test]
fn a_line_through_and_an_underline_sit_on_whole_rows_at_1x() {
    decorations(1.0);
}

#[test]
fn a_line_through_and_an_underline_sit_on_whole_rows_at_1_5x() {
    decorations(1.5);
}
