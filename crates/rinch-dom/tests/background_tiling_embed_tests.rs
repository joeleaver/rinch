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
    let mut doc = RinchDocument::new();
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
    // DrawTag::RADIAL_GRADIENT / LINEAR_GRADIENT
    let grads = e
        .draw_tags
        .iter()
        .filter(|t| t.0 == 0x229 || t.0 == 0x114 || t.0 == 0x654)
        .count();
    (e.draw_tags.len(), grads)
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
