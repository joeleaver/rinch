//! #2597 and #2607 on the software painter (tiny-skia); the scene and
//! Chrome's boxes are in `support/inline_fragments_2597.rs`, shared with
//! `gpu_inline_fragments_2597` (in `rinch`), which holds the GPU painter to
//! the same checks.
//!
//! - #2597: an inline element whose only content is an atomic inline drew
//!   no background and no border. A line holding no text has parley's text
//!   range `usize::MAX..0`, which the search for an element's lines never
//!   found, and an element with neither text nor edge boxes recorded no box
//!   at all; nor did an element's box take in the atomic inlines it holds.
//! - #2607: a wrapped inline element's fragment took in the white space its
//!   line ends with, which Chrome hangs; its edges were not on whole device
//!   pixels; and a side beside an open end (a solid one, drawn with a round
//!   cap) reached a pixel past the fragment.

#![cfg(feature = "software-renderer")]

#[path = "support/inline_fragments_2597.rs"]
mod scene;

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use rinch_dom::testing::query_selector;

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

/// Every fragment of the scene is drawn where Chrome draws it, with the
/// sides Chrome gives it, at 1x and 2x.
#[test]
fn inline_fragments_are_drawn_as_chrome_draws_them() {
    let mut doc = scene::document();
    for scale in [1.0, 2.0] {
        let (pw, px) = paint(&mut doc, (scene::W, scene::H), scale);
        assert_none("#2597/#2607", scene::misses(&px, pw, scale));
    }
}

/// `getBoundingClientRect` of each element is the union of the fragments
/// Chrome reports, which is the union of the fragments painted: the white
/// space that ends `h`'s line is in neither. `g` too, a wrapped
/// right-to-left element, its border on the right of its first fragment and
/// the left of its last (#2616).
#[test]
fn the_bounding_rect_is_the_union_of_the_fragments() {
    let mut doc = scene::document();
    let mut bad = Vec::new();
    for (id, _) in scene::CHROME {
        let node = query_selector(&doc.tree, &format!("#{id}"))[0];
        let r = doc.bounding_rect(NodeId(node)).expect("a box");
        let (x, y, w, h) = scene::chrome_union(id);
        let got = [r.x, r.y, r.width, r.height].map(f64::from);
        if got
            .iter()
            .zip([x, y, w, h])
            .any(|(g, c)| (g - c).abs() > 0.01)
        {
            bad.push(format!("#{id}: {got:?}, Chrome {:?}", (x, y, w, h)));
        }
    }
    assert_none("bounding rects", bad);
}

/// A point over the white space a line hangs finds no text, as Chrome finds
/// the block there; a point over the element's glyphs finds its text.
#[test]
fn the_hung_white_space_is_no_text_a_point_finds() {
    let doc = scene::document();
    let tree = &doc.tree;
    let mut bad = Vec::new();
    for &(x, y, want) in scene::HITS {
        // The block whose lines hold the point.
        let root = query_selector(tree, "div")
            .into_iter()
            .filter_map(|id| tree.get(id))
            .find(|n| {
                let (l, t) = (n.layout.x, n.layout.y);
                (l..l + n.layout.width).contains(&x) && (t..t + n.layout.height).contains(&y)
            })
            .expect("a block under the point");
        let inline = root.text_layout.as_ref().expect("its lines");
        let (ox, oy) = rinch_dom::paint::ifc_root_content_origin(root);
        let got = rinch_dom::text_query::glyph_cluster_in_inline(
            inline,
            x - root.layout.x - ox,
            y - root.layout.y - oy,
        )
        .and_then(|c| {
            inline
                .text_ranges
                .iter()
                .find(|r| (r.flat_start..r.flat_end).contains(&c.start))
        })
        .and_then(|r| tree.get(r.node_id)?.parent)
        .and_then(|p| doc.get_attribute(NodeId(p), "id"));
        if got.as_deref() != want {
            bad.push(format!("({x}, {y}): {got:?}, Chrome {want:?}"));
        }
    }
    assert_none("hits", bad);
}

/// A solid side's ends: no cap past a lone side, and no seam where the sides
/// of boxes meeting at a fractional device pixel meet (review of #2623), at
/// 1x and 1.5x.
#[test]
fn solid_sides_end_on_whole_pixels_without_seams() {
    use scene::seams::*;
    let mut doc = document();
    for scale in [1.0, 1.5] {
        let (pw, px) = paint(&mut doc, (W, H), scale);
        assert_none(&format!("seams at {scale}x"), misses(&px, pw, scale));
    }
}
