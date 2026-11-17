//! #2666: a box given a highlight owner (`DomDocument::set_highlight_owner`)
//! is painted over the owner's backgrounds, inline ones too, and under its
//! glyphs, as Chrome paints a selection. The page and its checks are in
//! `support/highlight_owner_2666.rs`, shared with `gpu_highlight_owner_2666`
//! (in `rinch`).
#![cfg(feature = "software-renderer")]

#[path = "support/highlight_owner_2666.rs"]
mod scene;

use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

fn render(doc: &mut RinchDocument) -> Vec<[u8; 4]> {
    let mut painter = TinySkiaPainter::new(scene::W, scene::H);
    let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (scene::W as f32, scene::H as f32),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter.pixels().as_chunks::<4>().0.to_vec()
}

#[test]
fn an_owned_wash_is_painted_over_its_owners_backgrounds_and_under_its_text() {
    let px = render(&mut scene::document(true));
    let bad = scene::misses(&px, scene::W);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// Positive control: without owners every wash is where CSS puts it, under
/// the green and blue backgrounds (what the editor's selection was, #2666).
#[test]
fn an_unowned_wash_stays_under_the_backgrounds() {
    let px = render(&mut scene::document(false));
    let bad = scene::unowned_misses(&px, scene::W);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// A wash whose owner was freed and whose slot a new node took is painted in
/// its place, under the green and the glyphs: never held for the new node
/// (which panicked in debug and drew it over the text in release).
#[test]
fn a_wash_whose_owner_was_freed_is_painted_in_its_place() {
    let px = render(&mut scene::document_with_dead_owner());
    let bad = scene::unowned_misses(&px, scene::W);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
