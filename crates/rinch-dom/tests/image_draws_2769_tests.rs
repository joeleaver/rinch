//! #2769 — the `image_draws` per-frame counter: one per `<img>` and per
//! `background-image` `url()` layer a paint actually draws, on both painters.
//!
//! The count is made in the paint walk both painters share, so the software
//! painter (`TinySkiaPainter`) and the GPU one (`VelloPainter`, which builds a
//! scene with no device) must agree on every case here. A tiled background
//! counts once per element and layer however many tiles or draw calls it
//! takes; an image outside the window or a partial repaint's damage, and one
//! not decoded yet, counts nothing.
#![cfg(feature = "software-renderer")]

use peniko::Brush;
use peniko::kurbo::Rect;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::Painter;
use rinch_dom::paint::vello_painter::VelloPainter;
use rinch_dom::perf::Counter;

const VW: f32 = 400.0;
const VH: f32 = 300.0;
const URL: &str = "test://image-draws-2769.png";

/// A 4x4 image, two colours so nothing about it is uniform.
fn image() -> rinch_dom::image_cache::DecodedImage {
    let mut data = Vec::new();
    for i in 0..16u8 {
        data.extend_from_slice(if i % 3 == 0 {
            &[200, 10, 10, 255]
        } else {
            &[10, 10, 200, 160]
        });
    }
    rinch_dom::image_cache::DecodedImage::new(data, 4, 4)
}

/// A document holding `html` under `css`, the image decoded at [`URL`].
fn doc_with(css: &str, html: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.tree.image_cache.insert_decoded(URL.into(), image());
    doc.load_css(&format!(
        "html, body {{ margin: 0; background: rgb(255, 255, 255); }} {css}"
    ));
    let body = doc.body();
    doc.set_inner_html(body, html);
    doc.resolve_layout(VW, VH);
    doc
}

#[derive(Clone, Copy, Debug)]
enum Which {
    Software,
    Gpu,
}

/// The `image_draws` of one paint of `doc` by `which`, under `damage`.
fn draws(doc: &mut RinchDocument, which: Which, damage: Option<Rect>) -> u64 {
    let mut cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    let mut software;
    let mut gpu;
    let painter: &mut dyn Painter = match which {
        Which::Software => {
            software = rinch_dom::paint::skia_painter::TinySkiaPainter::new(VW as u32, VH as u32);
            &mut software
        }
        Which::Gpu => {
            gpu = VelloPainter::new();
            &mut gpu
        }
    };
    rinch_dom::paint::set_dirty_region(damage);
    doc.tree.perf.reset();
    rinch_dom::paint::paint_document(&doc.tree, painter, 1.0, (VW, VH), &mut doc.font_cx, &mut cx);
    rinch_dom::paint::set_dirty_region(None);
    doc.tree.perf.end_frame().get(Counter::ImageDraws)
}

const BOTH: [Which; 2] = [Which::Software, Which::Gpu];

fn boxed(style: &str) -> String {
    format!(
        "<div style=\"position: absolute; left: 20px; top: 30px; width: 100px; height: 60px; {style}\"></div>"
    )
}

/// Every way a background layer is painted counts one: the one tile that is
/// the box, one clipped tile covering it, a repeating pattern (one fill
/// however many tiles), and tile by tile.
#[test]
fn a_background_image_counts_once_per_layer_on_both_painters() {
    let cases = [
        // 1. One tile, the box itself.
        (
            "stretched",
            format!("background-image: url({URL}); background-size: 100% 100%"),
        ),
        // 1. One tile covering a rounded box: one clipped draw.
        (
            "cover, rounded",
            format!("background-image: url({URL}); background-size: cover; border-radius: 9px"),
        ),
        // 2. A repeating 10px tile: dozens of tiles, one pattern.
        (
            "repeating",
            format!("background-image: url({URL}); background-size: 10px 10px"),
        ),
        // 2. Repeating on one axis only: the pattern clipped to the strip.
        (
            "repeat-x",
            format!(
                "background-image: url({URL}); background-size: 10px 10px; background-repeat: repeat-x"
            ),
        ),
        // 3. A no-repeat tile that does not cover the box: tile by tile.
        (
            "no-repeat, positioned",
            format!(
                "background-image: url({URL}); background-size: 16px 16px; background-repeat: no-repeat; background-position: 30px 20px"
            ),
        ),
    ];
    for which in BOTH {
        for (name, style) in &cases {
            let mut doc = doc_with("", &boxed(style));
            assert_eq!(draws(&mut doc, which, None), 1, "{name} on {which:?}");
        }
    }
}

/// Two `url()` layers on one element are two draws; a gradient layer is not
/// an image and counts nothing.
#[test]
fn each_url_layer_counts_and_a_gradient_does_not() {
    for which in BOTH {
        let mut doc = doc_with(
            "",
            &boxed(&format!(
                "background-image: url({URL}), linear-gradient(red, blue), url({URL}); background-size: 10px 10px, auto, 100% 100%"
            )),
        );
        assert_eq!(draws(&mut doc, which, None), 2, "{which:?}");
        let mut doc = doc_with("", &boxed("background-image: linear-gradient(red, blue)"));
        assert_eq!(draws(&mut doc, which, None), 0, "gradient on {which:?}");
        let mut doc = doc_with("", &boxed("background: rgb(1, 2, 3)"));
        assert_eq!(draws(&mut doc, which, None), 0, "colour on {which:?}");
    }
}

/// An `<img>` counts one; one whose image is not decoded yet (its
/// placeholder is drawn instead) counts nothing.
#[test]
fn an_img_counts_once_and_an_undecoded_one_not_at_all() {
    for which in BOTH {
        let mut doc = doc_with(
            "",
            &format!("<img src=\"{URL}\" style=\"width: 80px; height: 50px\">"),
        );
        assert_eq!(draws(&mut doc, which, None), 1, "{which:?}");
        let mut doc = doc_with(
            "",
            "<img src=\"test://not-loaded-2769.png\" style=\"width: 80px; height: 50px\">",
        );
        assert_eq!(draws(&mut doc, which, None), 0, "undecoded on {which:?}");
        let mut doc = doc_with(
            "",
            &format!(
                "<img src=\"{URL}\" style=\"width: 80px; height: 50px\"><img src=\"{URL}\" style=\"width: 8px; height: 5px\">"
            ),
        );
        assert_eq!(draws(&mut doc, which, None), 2, "two imgs on {which:?}");
    }
}

/// A partial frame whose damage misses the image does not count it, on
/// either painter; damage that touches it does. Checked for an `<img>` and
/// for each background path, including a tiled background whose element is
/// not culled (the damage is inside its box) but whose tiles all fall
/// outside the damage.
#[test]
fn a_partial_frame_counts_only_images_in_its_damage() {
    let miss = Rect::new(300.0, 200.0, 340.0, 240.0);
    let hit = Rect::new(60.0, 50.0, 70.0, 60.0);
    let docs = [
        (
            "img",
            format!(
                "<img src=\"{URL}\" style=\"position: absolute; left: 20px; top: 30px; width: 100px; height: 60px\">"
            ),
        ),
        (
            "stretched",
            boxed(&format!(
                "background-image: url({URL}); background-size: 100% 100%"
            )),
        ),
        (
            "repeating",
            boxed(&format!(
                "background-image: url({URL}); background-size: 10px 10px"
            )),
        ),
    ];
    for which in BOTH {
        for (name, html) in &docs {
            let mut doc = doc_with("", html);
            assert_eq!(
                draws(&mut doc, which, Some(miss)),
                0,
                "{name} missed, {which:?}"
            );
            assert_eq!(
                draws(&mut doc, which, Some(hit)),
                1,
                "{name} hit, {which:?}"
            );
        }
        // An `<img>` whose padding is in the damage and its picture (the
        // content box) is not: the element is painted, its image not shown.
        let mut doc = doc_with(
            "",
            &format!(
                "<img src=\"{URL}\" style=\"position: absolute; left: 20px; top: 30px; width: 40px; height: 30px; padding: 0 0 40px 0\">"
            ),
        );
        let padding_only = Rect::new(30.0, 80.0, 50.0, 95.0);
        assert_eq!(
            draws(&mut doc, which, Some(padding_only)),
            0,
            "img padding, {which:?}"
        );
        assert_eq!(
            draws(&mut doc, which, Some(Rect::new(30.0, 40.0, 50.0, 50.0))),
            1,
            "img content, {which:?}"
        );
        // The box is in the damage, its one no-repeat tile is not.
        let mut doc = doc_with(
            "",
            &boxed(&format!(
                "background-image: url({URL}); background-size: 16px 16px; background-repeat: no-repeat; background-position: 0 0"
            )),
        );
        let in_box_off_tile = Rect::new(90.0, 70.0, 110.0, 85.0);
        assert_eq!(
            draws(&mut doc, which, Some(in_box_off_tile)),
            0,
            "tile outside, {which:?}"
        );
        assert_eq!(
            draws(&mut doc, which, Some(Rect::new(22.0, 32.0, 30.0, 40.0))),
            1,
            "tile inside, {which:?}"
        );
        // repeat-x: the strip the tiles occupy is outside the damage.
        let mut doc = doc_with(
            "",
            &boxed(&format!(
                "background-image: url({URL}); background-size: 10px 10px; background-repeat: repeat-x"
            )),
        );
        assert_eq!(
            draws(&mut doc, which, Some(in_box_off_tile)),
            0,
            "strip outside, {which:?}"
        );
        assert_eq!(
            draws(&mut doc, which, Some(Rect::new(60.0, 32.0, 70.0, 38.0))),
            1,
            "strip inside, {which:?}"
        );
    }
}

/// An image off the window is culled and counts nothing.
#[test]
fn an_image_off_the_window_counts_nothing() {
    for which in BOTH {
        let mut doc = doc_with(
            "",
            &format!(
                "<img src=\"{URL}\" style=\"position: absolute; left: 20px; top: 900px; width: 100px; height: 60px\">\
                 <div style=\"position: absolute; left: 900px; top: 30px; width: 100px; height: 60px; background-image: url({URL})\"></div>"
            ),
        );
        assert_eq!(draws(&mut doc, which, None), 0, "{which:?}");
    }
}

/// A `mask-image` is not a painted picture: it shapes the element's paint
/// and counts nothing; the element's own background image still counts.
#[test]
fn a_mask_image_does_not_count() {
    for which in BOTH {
        let mut doc = doc_with(
            "",
            &boxed(&format!(
                "background: rgb(1, 2, 3); mask-image: url({URL}); mask-size: 10px 10px"
            )),
        );
        assert_eq!(draws(&mut doc, which, None), 0, "{which:?}");
    }
}

/// The counter is on the frame line under its name, and absent from it when
/// zero.
#[test]
fn the_frame_line_names_it_only_when_it_counted() {
    let mut doc = doc_with("", &boxed(&format!("background-image: url({URL})")));
    let _ = draws(&mut doc, Which::Software, None);
    let frame = doc.tree.perf.last_frame();
    assert!(
        frame.summary_line().contains("image_draws=1"),
        "{}",
        frame.summary_line()
    );
    let mut doc = doc_with("", &boxed("background: rgb(1, 2, 3)"));
    let _ = draws(&mut doc, Which::Software, None);
    let frame = doc.tree.perf.last_frame();
    assert!(
        !frame.summary_line().contains("image_draws"),
        "{}",
        frame.summary_line()
    );
}
