//! #468, review round 2 of PR #1143 (N1): a build without the software
//! painter (`embed`) has no rasteriser for the pattern path, so it tiles one
//! fill per tile — and must never drop a layer, however many tiles it has.
//! Round 2 dropped any layer past 4096 tiles: the 800x600 dot grid emitted
//! one draw (its underlay). Run with `--no-default-features`.
#![cfg(not(feature = "software-renderer"))]
use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::vello_painter::VelloPainter;

fn draws(style: &str) -> (usize, usize) {
    let (all, grads, _) = draws_with_images(style);
    (all, grads)
}

/// A 3x2 image of distinct, partly translucent (straight-alpha) pixels.
fn dots() -> rinch_dom::image_cache::DecodedImage {
    let data: Vec<u8> = (0..6u8)
        .flat_map(|i| [40 * i, 200 - 30 * i, 90, 255 - 20 * i])
        .collect();
    rinch_dom::image_cache::DecodedImage::new(data, 3, 2)
}

/// Every draw, the gradient draws, and the image draws.
fn draws_with_images(style: &str) -> (usize, usize, usize) {
    let mut doc = RinchDocument::new();
    doc.tree
        .image_cache
        .insert_decoded("test://dots.png".into(), dots());
    doc.load_css("html, body { margin: 0; }");
    let body = doc.body();
    let node = doc.create_element("div");
    doc.set_attribute(
        node,
        "style",
        &format!("position:absolute;left:0;top:0;{style}"),
    );
    doc.append_child(body, node);
    doc.resolve_layout(900.0, 700.0);
    let mut vp = VelloPainter::new();
    let mut lcx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut vp,
        1.0,
        (900.0, 700.0),
        &mut doc.font_cx,
        &mut lcx,
    );
    let e = vp.scene().encoding();
    // DrawTag::RADIAL_GRADIENT / LINEAR_GRADIENT / SWEEP_GRADIENT (vello_encoding 0.7)
    let grads = e
        .draw_tags
        .iter()
        .filter(|t| t.0 == 0x29c || t.0 == 0x114 || t.0 == 0x254)
        .count();
    // DrawTag::IMAGE
    let images = e.draw_tags.iter().filter(|t| t.0 == 0x28C).count();
    (e.draw_tags.len(), grads, images)
}

#[test]
fn a_many_tile_layer_is_drawn_without_the_software_painter() {
    let (all, _) = draws(
        "width:800px;height:600px;background-color:rgb(250,250,250);background-image:radial-gradient(rgb(0,0,0) 20%, transparent 20%);background-size:10px 10px",
    );
    assert!(
        all >= 4800,
        "80x60 tiles, one draw each (plus the underlay): {all}"
    );
}

#[test]
fn embed_probe() {
    eprintln!(
        "dot grid 800x600 10px: {:?}",
        draws(
            "width:800px;height:600px;background-color:rgb(250,250,250);background-image:radial-gradient(rgb(0,0,0) 20%, transparent 20%);background-size:10px 10px"
        )
    );
    eprintln!(
        "dot grid 400x300 10px: {:?}",
        draws(
            "width:400px;height:300px;background-color:rgb(250,250,250);background-image:radial-gradient(rgb(0,0,0) 20%, transparent 20%);background-size:10px 10px"
        )
    );
    eprintln!(
        "stripes 300x16: {:?}",
        draws(
            "width:300px;height:16px;background-image:linear-gradient(45deg, rgba(255,255,255,.15) 25%, transparent 25%, transparent 50%, rgba(255,255,255,.15) 50%, rgba(255,255,255,.15) 75%, transparent 75%, transparent);background-size:16px 16px"
        )
    );
}

/// #1158: a `url()` layer needs no rasteriser. The decoded image is one
/// `Extend::Repeat` image brush, however many tiles it covers: 800x600 of a
/// 3x2 image is 80,000 tiles, which was 80,000 image draws.
#[test]
fn a_repeating_url_layer_is_one_image_draw_without_the_software_painter() {
    for style in [
        // Natural size, both axes repeat.
        "width:800px;height:600px;background-image:url(test://dots.png)",
        // Scaled tile, off the identity.
        "width:800px;height:600px;background-image:url(test://dots.png);background-size:7px 5px",
        // One axis repeats: clipped to the strip.
        "width:800px;height:600px;background-image:url(test://dots.png);background-repeat:repeat-x;background-position:0 13px",
        // Rounded box (a BezPath / RoundedRect shape).
        "width:800px;height:600px;border-radius:20px;background-image:url(test://dots.png);background-position:1.5px 2.5px",
    ] {
        let (all, _, images) = draws_with_images(style);
        assert_eq!(images, 1, "{style}: {all} draws, {images} images");
    }
}

/// Gradients still go tile by tile: the image arm must not take them.
#[test]
fn a_gradient_layer_is_still_tiled_without_the_software_painter() {
    let (all, grads, images) = draws_with_images(
        "width:100px;height:100px;background-image:radial-gradient(red 20%, transparent 20%);background-size:10px 10px",
    );
    assert_eq!(images, 0, "{all}");
    assert_eq!(grads, 100, "{all}");
}
