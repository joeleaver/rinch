//! #2098 — `image-rendering: pixelated` / `crisp-edges` — and #2099 —
//! `mask-image: url(…)` with `mask-size`, `mask-position` and `mask-repeat`
//! — on the software painter.
//!
//! Every expected pixel is Chrome 156's (headless, `--force-device-scale-factor=1`,
//! the same markup with the images as `data:` PNGs), unless a test says
//! otherwise. The GPU twin is `crates/rinch/tests/gpu_image_rendering_mask_2098.rs`.
#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::image_cache::DecodedImage;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const W: u32 = 400;
const H: u32 = 220;

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];
const CLEAR: [u8; 4] = [0, 0, 0, 0];

/// 2x2: red, green / blue, white.
fn quad() -> DecodedImage {
    DecodedImage::new([RED, GREEN, BLUE, WHITE].concat(), 2, 2)
}

/// 4x4: the left half opaque, the right half transparent.
fn half() -> DecodedImage {
    let px: Vec<[u8; 4]> = (0..16)
        .map(|i| if i % 4 < 2 { BLACK } else { CLEAR })
        .collect();
    DecodedImage::new(px.concat(), 4, 4)
}

/// 4x4: opaque, with a transparent 2x2 hole in the middle and a half-alpha
/// top-left texel.
fn holed() -> DecodedImage {
    let px: Vec<[u8; 4]> = (0..16)
        .map(|i| {
            let (x, y) = (i % 4, i / 4);
            if (1..=2).contains(&x) && (1..=2).contains(&y) {
                CLEAR
            } else if (x, y) == (0, 0) {
                [0, 0, 0, 128]
            } else {
                BLACK
            }
        })
        .collect();
    DecodedImage::new(px.concat(), 4, 4)
}

fn new_doc() -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css(
        "html, body { margin: 0; background: #fff; } .a { position: absolute; display: block; }",
    );
    let cache = &mut doc.tree.image_cache;
    cache.insert_decoded("q2099.png".into(), quad());
    cache.insert_decoded("m2099.png".into(), half());
    cache.insert_decoded("h2099.png".into(), holed());
    doc
}

fn add(doc: &mut RinchDocument, tag: &str, style: &str) -> NodeId {
    let body = doc.body();
    let n = doc.create_element(tag);
    doc.set_attribute(n, "class", "a");
    doc.set_attribute(n, "style", style);
    if tag == "img" {
        doc.set_attribute(n, "src", "q2099.png");
    }
    doc.append_child(body, n);
    n
}

fn paint(doc: &mut RinchDocument) -> TinySkiaPainter {
    doc.resolve_layout(W as f32, H as f32);
    let mut painter = TinySkiaPainter::new(W, H);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (W as f32, H as f32),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter
}

fn px(p: &TinySkiaPainter, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * p.width() + x) * 4) as usize;
    let d = p.pixels();
    [d[i], d[i + 1], d[i + 2]]
}

/// Each `(point, want)` within `tol` per channel.
fn expect(p: &TinySkiaPainter, want: &[((u32, u32), [u8; 3])], tol: u8) {
    let bad: Vec<_> = want
        .iter()
        .filter_map(|&((x, y), w)| {
            let got = px(p, x, y);
            (got.iter().zip(w).any(|(a, b)| a.abs_diff(b) > tol)).then_some(((x, y), got, w))
        })
        .collect();
    assert!(bad.is_empty(), "(point, rinch, chrome): {bad:?}");
}

fn row(x0: u32, y: u32, want: &[[u8; 3]]) -> Vec<((u32, u32), [u8; 3])> {
    want.iter()
        .enumerate()
        .map(|(i, &w)| ((x0 + i as u32, y), w))
        .collect()
}

const R: [u8; 3] = [255, 0, 0];
const G: [u8; 3] = [0, 255, 0];
const WH: [u8; 3] = [255, 255, 255];

/// `image-rendering` reaches the computed style, inherited, and Stylo's
/// servo build keeps the two keywords apart.
#[test]
fn image_rendering_is_computed_and_inherited() {
    use rinch_dom::computed_style::ImageRenderingValue as V;
    let mut doc = new_doc();
    let parent = add(&mut doc, "div", "image-rendering: pixelated");
    let child = doc.create_element("img");
    doc.append_child(parent, child);
    let crisp = add(&mut doc, "div", "image-rendering: crisp-edges");
    let moz = add(&mut doc, "div", "image-rendering: -moz-crisp-edges");
    let plain = add(&mut doc, "div", "");
    doc.resolve_layout(W as f32, H as f32);
    let ir = |n: NodeId| doc.tree.nodes[n.0].computed_style.image_rendering;
    assert_eq!(ir(parent), V::Pixelated);
    assert_eq!(ir(child), V::Pixelated, "inherited");
    assert_eq!(ir(crisp), V::CrispEdges);
    assert_eq!(ir(moz), V::CrispEdges);
    assert_eq!(ir(plain), V::Auto);
}

/// A 2x2 image at 40x40 with `pixelated` and `crisp-edges`: hard 20px
/// texels (Chrome 156 draws both nearest). The software painter draws an
/// `<img>` nearest under `auto` too, so this pins that nearest stays nearest
/// rather than failing on `main`.
#[test]
fn a_pixelated_img_upscales_nearest() {
    let mut doc = new_doc();
    add(
        &mut doc,
        "img",
        "left: 50px; top: 0; width: 40px; height: 40px; image-rendering: pixelated",
    );
    add(
        &mut doc,
        "img",
        "left: 100px; top: 0; width: 40px; height: 40px; image-rendering: crisp-edges",
    );
    let p = paint(&mut doc);
    for x0 in [50, 100] {
        expect(&p, &row(x0 + 15, 5, &[R, R, R, R, R, G, G, G, G, G]), 0);
        expect(&p, &[((x0 + 5, 25), [0, 0, 255]), ((x0 + 35, 35), WH)], 0);
    }
}

/// A repeating `background-image` at a fractional `background-position`
/// (`0.25px`): `auto` samples the pattern bilinearly (#1151), `pixelated`
/// nearest, so the tile edges stay hard. Chrome 156: 200..209 red,
/// 210..219 green, 220.. red. On `main` the edge pixels blended.
#[test]
fn a_pixelated_repeating_background_samples_nearest_at_a_fractional_position() {
    let mut doc = new_doc();
    add(
        &mut doc,
        "div",
        "left: 200px; top: 100px; width: 80px; height: 40px; \
         background: url(q2099.png) 0.25px 0 / 20px 20px repeat; image-rendering: pixelated",
    );
    let p = paint(&mut doc);
    let mut want = vec![R; 10];
    want.extend([G; 10]);
    want.extend([R; 4]);
    expect(&p, &row(200, 105, &want), 0);
}

/// `mask-image: url(…)` with `mask-size: 100% 100%` (and as
/// `-webkit-mask-image`, which Stylo maps to it): the left half of a red box
/// shows, the right half is masked away. Chrome blends the middle columns
/// (its `auto` is smooth; the software painter's is nearest, so they are not
/// compared). On `main` the url layer was dropped and the box was all red.
#[test]
fn a_url_mask_tints_an_icon() {
    let mut doc = new_doc();
    add(
        &mut doc,
        "div",
        "left: 0; top: 150px; width: 40px; height: 40px; background: red; \
         mask-image: url(m2099.png); mask-size: 100% 100%",
    );
    add(
        &mut doc,
        "div",
        "left: 50px; top: 150px; width: 40px; height: 40px; background: red; \
         -webkit-mask-image: url(m2099.png); -webkit-mask-size: 100% 100%",
    );
    let p = paint(&mut doc);
    for x0 in [0, 50] {
        expect(&p, &row(x0 + 2, 170, &[R; 12]), 0);
        expect(&p, &row(x0 + 27, 170, &[WH; 12]), 0);
        expect(&p, &row(x0 + 2, 152, &[R; 12]), 0);
    }
}

/// `pixelated` applies to a mask image too (Chrome 156): the half mask
/// scaled 10x has a hard edge at x = 20.
#[test]
fn a_pixelated_url_mask_has_a_hard_edge() {
    let mut doc = new_doc();
    add(
        &mut doc,
        "div",
        "left: 250px; top: 150px; width: 40px; height: 40px; background: red; \
         mask-image: url(m2099.png); mask-size: 100% 100%; image-rendering: pixelated",
    );
    let p = paint(&mut doc);
    expect(
        &p,
        &row(264, 170, &[R, R, R, R, R, R, WH, WH, WH, WH, WH, WH]),
        0,
    );
}

/// `mask-size: auto` is the image's own 4x4, repeated (`mask-repeat`'s
/// initial `repeat`) from the border box's corner (`mask-origin:
/// border-box`): a 2x2 hole every 4px, and the half-alpha corner texel
/// blends red with white. Exact pixels: nothing is scaled.
#[test]
fn a_url_mask_repeats_at_its_natural_size() {
    let mut doc = new_doc();
    add(
        &mut doc,
        "div",
        "left: 100px; top: 150px; width: 40px; height: 40px; background: red; mask-image: url(h2099.png)",
    );
    let p = paint(&mut doc);
    let half = [255, 127, 127];
    expect(
        &p,
        &row(100, 150, &[half, R, R, R, half, R, R, R, half, R]),
        1,
    );
    expect(&p, &row(100, 151, &[R, WH, WH, R, R, WH, WH, R, R, WH]), 0);
    expect(&p, &row(136, 189, &[R, R, R, R]), 0);
    expect(&p, &row(136, 187, &[R, WH, WH, R]), 0);
}

/// `mask-repeat: no-repeat; mask-position: 8px 8px`: the one 4x4 tile at
/// (158, 158), nothing else of the box.
#[test]
fn a_url_mask_takes_its_position_and_repeat() {
    let mut doc = new_doc();
    add(
        &mut doc,
        "div",
        "left: 150px; top: 150px; width: 40px; height: 40px; background: red; \
         mask-image: url(h2099.png); mask-repeat: no-repeat; mask-position: 8px 8px",
    );
    let p = paint(&mut doc);
    let half = [255, 127, 127];
    for (y, want) in [
        (157, [WH, WH, WH, WH, WH, WH, WH]),
        (158, [WH, WH, half, R, R, R, WH]),
        (159, [WH, WH, R, WH, WH, R, WH]),
        (160, [WH, WH, R, WH, WH, R, WH]),
        (161, [WH, WH, R, R, R, R, WH]),
        (162, [WH, WH, WH, WH, WH, WH, WH]),
    ] {
        expect(&p, &row(156, y, &want), 1);
    }
    expect(
        &p,
        &[((152, 152), WH), ((185, 185), WH), ((170, 170), WH)],
        0,
    );
}

/// The `mask` shorthand (`url(…) 4px 4px / 20px 20px no-repeat`): a 20px
/// tile at (4, 4) of the box, its hole 10px wide in the middle. Points away
/// from the scaled texels' edges (where Chrome blends and software does not).
#[test]
fn the_mask_shorthand_sets_size_and_position() {
    let mut doc = new_doc();
    add(
        &mut doc,
        "div",
        "left: 300px; top: 150px; width: 40px; height: 40px; background: red; \
         mask: url(h2099.png) 4px 4px / 20px 20px no-repeat",
    );
    let p = paint(&mut doc);
    expect(
        &p,
        &[
            ((302, 164), WH),
            ((306, 164), R),
            ((314, 164), WH),
            ((321, 164), R),
            ((326, 164), WH),
            ((306, 156), [255, 127, 127]),
            ((314, 180), WH),
        ],
        1,
    );
}

/// A mask image that does not load counts as a transparent black layer
/// (css-masking-1 §7.1): the box is masked away entirely, as in Chrome. On
/// `main` the layer was dropped and the box painted red.
#[test]
fn a_url_mask_that_never_loads_hides_the_box() {
    let mut doc = new_doc();
    add(
        &mut doc,
        "div",
        "left: 200px; top: 150px; width: 40px; height: 40px; background: red;          mask-image: url(/nonexistent-2099/heart.png)",
    );
    let p = paint(&mut doc);
    expect(&p, &[((220, 170), WH), ((201, 151), WH)], 0);
}

/// The half mask as a PNG file's bytes.
fn half_png() -> Vec<u8> {
    let d = half();
    let mut bytes = Vec::new();
    image::write_buffer_with_format(
        &mut std::io::Cursor::new(&mut bytes),
        &d.data,
        d.width,
        d.height,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .unwrap();
    bytes
}

/// Lay out until `url` has decoded (the loader answers on a thread).
fn wait_for(doc: &mut RinchDocument, url: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while doc.tree.image_cache.get(url).is_none() {
        assert!(std::time::Instant::now() < deadline, "{url} never decoded");
        assert!(!doc.tree.image_cache.is_failed(url), "{url} failed to load");
        std::thread::sleep(std::time::Duration::from_millis(2));
        doc.resolve_layout(W as f32, H as f32);
    }
}

/// A `mask-image` url from an app's image scheme is loaded by that loader,
/// as a `background-image` url is, and its decode pushes the masked node
/// paint-dirty so an incremental paint redraws it (#2099). Before the
/// decode the box is masked away; after it, the left half shows.
#[test]
fn a_mask_image_from_an_app_scheme_loads_and_repaints_its_node() {
    let png = half_png();
    rinch_core::image::register_image_scheme("t2099", move |_: &str| {
        rinch_core::image::ImageLoadResult::Loaded(png.clone())
    });
    let mut doc = RinchDocument::new();
    doc.load_css("html, body { margin: 0; background: #fff; }");
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "position: absolute; left: 0; top: 0; width: 40px; height: 40px; \
         background: red; mask-image: url(t2099:half); mask-size: 100% 100%",
    );
    doc.append_child(body, div);
    let before = paint(&mut doc);
    expect(&before, &[((5, 20), WH), ((35, 20), WH)], 0);
    doc.tree.paint_dirty_nodes.clear();
    wait_for(&mut doc, "t2099:half");
    assert!(
        doc.tree.paint_dirty_nodes.contains(&div.0),
        "the mask image's decode must repaint its node"
    );
    let after = paint(&mut doc);
    rinch_core::image::unregister_image_scheme("t2099");
    expect(&after, &[((5, 20), R), ((35, 20), WH)], 0);
}

/// A `data:` mask image (the icon-tint case written inline) loads. A mask
/// url is loaded exactly as a `background-image` url is (the cascade notes it
/// in `pending_background_urls`), and since #2100 a `data:` url there is
/// decoded in place rather than handed to the document's loader.
#[test]
fn a_data_url_mask_image_loads() {
    const HALF: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAQAAAAECAYAAACp8Z5+AAAAGUlEQVR4nGNgYGD4D8UMDAwMDEwMaICwAAB8lAIFSJaEeAAAAABJRU5ErkJggg==";
    let mut doc = RinchDocument::new();
    doc.load_css("html, body { margin: 0; background: #fff; }");
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        &format!(
            "position: absolute; left: 0; top: 0; width: 40px; height: 40px; \
             background: red; mask-image: url({HALF}); mask-size: 100% 100%"
        ),
    );
    doc.append_child(body, div);
    let _ = paint(&mut doc);
    wait_for(&mut doc, HALF);
    let p = paint(&mut doc);
    expect(&p, &[((5, 20), R), ((35, 20), WH)], 0);
}

/// A gradient mask at its initial geometry is drawn as #1798 drew it, and a
/// gradient mask sized and placed by `mask-size` / `mask-position` now
/// takes them (Chrome 156: a `linear-gradient(#000, #000)` 10x10 at
/// `10px 10px`, `no-repeat`, shows only that square).
#[test]
fn a_gradient_mask_takes_its_size_and_position() {
    let mut doc = new_doc();
    add(
        &mut doc,
        "div",
        "left: 0; top: 0; width: 40px; height: 40px; background: red; \
         mask: linear-gradient(#000, #000) 10px 10px / 10px 10px no-repeat",
    );
    let p = paint(&mut doc);
    expect(
        &p,
        &[
            ((5, 5), WH),
            ((15, 15), R),
            ((10, 10), R),
            ((19, 19), R),
            ((20, 15), WH),
            ((30, 30), WH),
        ],
        0,
    );
}

/// Each mask layer takes its own entries of the `mask-*` lists, and the
/// layers add (Chrome 156: two 10px squares, at 0 0 and 20px 20px).
#[test]
fn each_mask_layer_takes_its_own_geometry() {
    let mut doc = new_doc();
    add(
        &mut doc,
        "div",
        "left: 0; top: 0; width: 40px; height: 40px; background: red; \
         mask: linear-gradient(#000, #000) 0 0 / 10px 10px no-repeat, \
               linear-gradient(#000, #000) 20px 20px / 10px 10px no-repeat",
    );
    let p = paint(&mut doc);
    expect(
        &p,
        &[
            ((5, 5), R),
            ((9, 9), R),
            ((15, 5), WH),
            ((15, 15), WH),
            ((20, 20), R),
            ((29, 29), R),
            ((30, 30), WH),
            ((35, 35), WH),
        ],
        0,
    );
}

/// `mask-origin`'s initial value is `border-box` (a background's is
/// `padding-box`): an unpositioned mask tile starts at the border's outer
/// corner and masks the border too. `content-box` starts it inside the
/// border. Chrome 156 pixels, a 10px blue border around a 20px red box.
#[test]
fn a_url_mask_is_positioned_from_its_mask_origin() {
    let mut doc = new_doc();
    // rinch lays every box out as `border-box` (Taffy's default; see
    // `docs/internals/platform.md`), so the fixture declares it: a 40px
    // border box either way.
    let common = "top: 0; box-sizing: border-box; width: 40px; height: 40px; \
                  border: 10px solid blue; background: red; \
                  mask-image: url(h2099.png); mask-repeat: no-repeat";
    add(&mut doc, "div", &format!("left: 50px; {common}"));
    add(
        &mut doc,
        "div",
        &format!("left: 100px; {common}; mask-origin: content-box"),
    );
    let p = paint(&mut doc);
    let b = [0, 0, 255];
    let half_b = [127, 127, 255];
    expect(&p, &row(50, 0, &[half_b, b, b, b, WH]), 1);
    expect(&p, &row(50, 1, &[b, WH, WH, b, WH]), 0);
    expect(&p, &[((60, 10), WH), ((101, 1), WH), ((105, 5), WH)], 0);
    let half_r = [255, 127, 127];
    expect(&p, &row(109, 10, &[WH, half_r, R, R, R, WH]), 1);
    expect(&p, &row(109, 11, &[WH, R, WH, WH, R, WH]), 0);
}

/// A mask image whose loader answers *failed* (not merely one never asked):
/// once the failure is drained the box stays masked away (css-masking-1
/// §7.1, as Chrome hides it), and later layouts and paints do not ask the
/// loader again.
#[test]
fn a_url_mask_whose_load_fails_hides_the_box_and_is_not_retried() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let mut doc = RinchDocument::new();
    doc.tree.image_loader = Some(Arc::new(move |_: &str| {
        counted.fetch_add(1, Ordering::SeqCst);
        rinch_core::image::ImageLoadResult::Failed("refused by the test loader".into())
    }));
    doc.load_css("html, body { margin: 0; background: #fff; }");
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "position: absolute; left: 0; top: 0; width: 40px; height: 40px; \
         background: red; mask-image: url(failing-2099.png); mask-size: 100% 100%",
    );
    doc.append_child(body, div);
    let _ = paint(&mut doc);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !doc.tree.image_cache.is_failed("failing-2099.png") {
        assert!(
            std::time::Instant::now() < deadline,
            "the failure never drained"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
        doc.resolve_layout(W as f32, H as f32);
    }
    // Positive control: the loader was asked, exactly once.
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let p = paint(&mut doc);
    expect(&p, &[((5, 20), WH), ((35, 20), WH), ((20, 1), WH)], 0);
    // More frames, a restyle of the box included: no second request.
    doc.set_attribute(
        div,
        "style",
        "position: absolute; left: 0; top: 0; width: 40px; height: 40px; \
         background: blue; mask-image: url(failing-2099.png); mask-size: 100% 100%",
    );
    for _ in 0..5 {
        let p = paint(&mut doc);
        expect(&p, &[((5, 20), WH), ((35, 20), WH)], 0);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "a failed mask image is not requested again"
    );
}
