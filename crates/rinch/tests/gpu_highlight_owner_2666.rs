//! #2666 on the GPU painter (Vello; lavapipe in CI): a box given a highlight
//! owner is painted over the owner's backgrounds and under its glyphs. The
//! page and its checks are `rinch-dom`'s `support/highlight_owner_2666.rs`,
//! which `highlight_owner_2666_tests` holds the software painter to.
//!
//! Needs an adapter, so `#[ignore]`d: run with
//! `cargo test -p rinch --features gpu --test gpu_highlight_owner_2666 -- --ignored`.
#![cfg(feature = "gpu")]

#[path = "support/vello_pixels.rs"]
mod pixels;
#[path = "../../rinch-dom/tests/support/highlight_owner_2666.rs"]
mod scene;

use peniko::Brush;
use rinch_dom::RinchDocument;
use rinch_dom::paint::vello_painter::VelloPainter;

fn render(doc: &mut RinchDocument) -> Vec<[u8; 4]> {
    let mut painter = VelloPainter::new();
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (scene::W as f32, scene::H as f32),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    pixels::render(painter.scene(), scene::W, scene::H)
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_an_owned_wash_is_painted_over_its_owners_backgrounds_and_under_its_text() {
    let px = render(&mut scene::document(true));
    let bad = scene::misses(&px, scene::W);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_an_unowned_wash_stays_under_the_backgrounds() {
    let px = render(&mut scene::document(false));
    let bad = scene::unowned_misses(&px, scene::W);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
