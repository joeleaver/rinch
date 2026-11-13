//! #2335, #2375, #2396 and #2373 on the GPU painter: the scenes and checks
//! of `rinch-dom/tests/paint_decorations_2396_tests.rs` (shared through
//! `rinch-dom/tests/support/paint_decorations_2396.rs`), painted by
//! `VelloPainter` and rendered by Vello (Area).
//!
//! Needs an adapter, so `#[ignore]`d: run with
//! `cargo test -p rinch --features gpu --test gpu_paint_decorations_2396 -- --ignored`.
#![cfg(feature = "gpu")]

#[path = "support/vello_pixels.rs"]
mod pixels;
#[path = "../../rinch-dom/tests/support/paint_decorations_2396.rs"]
mod scene;

use peniko::Brush;
use rinch_dom::RinchDocument;
use rinch_dom::paint::vello_painter::VelloPainter;

/// `doc` painted by Vello over white at `scale` on a `w` x `h` CSS px viewport.
fn render(doc: &mut RinchDocument, (w, h): (u32, u32), scale: f64) -> (u32, Vec<[u8; 4]>) {
    let (pw, ph) = (
        (w as f64 * scale).round() as u32,
        (h as f64 * scale).round() as u32,
    );
    let mut painter = VelloPainter::new();
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale,
        (w as f32, h as f32),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    (pw, pixels::render(painter.scene(), pw, ph))
}

fn assert_none(what: &str, bad: Vec<String>) {
    assert!(bad.is_empty(), "{what}:\n{}", bad.join("\n"));
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_an_img_draws_its_shadows_outline_and_background_image() {
    use scene::img::*;
    let (mut doc, _) = document(BOXES);
    let (_, px) = render(&mut doc, (W, H), 1.0);
    assert_none("#2335", misses(&px));
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_a_rounded_border_in_four_colours_covers_what_one_colour_does() {
    use scene::sides::*;
    for scale in [1.0, 2.0] {
        let (pw, one) = render(&mut document(ONE), (W, H), scale);
        let (_, four) = render(&mut document(FOUR), (W, H), scale);
        assert_none(
            &format!("#2375 at {scale}x"),
            misses(&one, &four, pw, GPU_TOL),
        );
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_an_inline_elements_dashed_dotted_and_groove_borders_are_chromes() {
    use scene::inline::*;
    let (_, px) = render(&mut document(), (W, H), 1.0);
    assert_none("#2396", misses(&px));
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_a_wrapped_inline_border_draws_no_side_at_the_break() {
    use scene::wrapped::*;
    let (_, px) = render(&mut document(), (W, H), 1.0);
    assert_none("#2396 wrapped", misses(&px));
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_a_line_through_and_an_underline_sit_on_whole_rows() {
    use scene::decorations::*;
    for scale in [1.0, 1.5] {
        let (pw, px) = render(&mut document(), (W, H), scale);
        assert_none(&format!("#2373 at {scale}x"), misses(&px, pw, scale));
    }
}
