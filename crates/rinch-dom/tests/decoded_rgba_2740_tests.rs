//! #2740: decoded pixels as an image source.
//!
//! An app that holds a picture as pixels (a notification's raw image, a frame
//! it made, a file it decoded itself under its own bounds) hands them over
//! decoded, either from a loader (`ImageLoadResult::Decoded`) or by name
//! (`register_rgba`, shown by `<img src="rinch-image:name">`). rinch checks
//! them and never decodes them; they are then any picture. Before #2740 the
//! only way was to re-encode them to PNG for rinch to decode straight back.
//!
//! `gpu_decoded_rgba_2740` (in `rinch`) holds the GPU painter to the same
//! pixels.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_core::image::{
    DecodedImageError, ImageLoadResult, MAX_DECODED_IMAGE_BYTES, register_image_scheme,
    register_rgba, unregister_rgba,
};
use rinch_dom::RinchDocument;
use rinch_dom::image_cache::FileImageLoader;
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use std::sync::Arc;
use std::time::{Duration, Instant};

const W: u32 = 120;
const H: u32 = 60;

/// 4x2, straight alpha: red, green, blue, half-transparent blue (alpha 128,
/// so the painter's alpha handling shows over the white page) / black, white,
/// yellow, magenta.
fn picture() -> Arc<[u8]> {
    [
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [0, 0, 255, 128],
        [0, 0, 0, 255],
        [255, 255, 255, 255],
        [255, 255, 0, 255],
        [255, 0, 255, 255],
    ]
    .concat()
    .into()
}

/// The `<img>`: 40x20 at (10, 10), so each source pixel is a 10x10 block.
const IMG_STYLE: &str = "position: absolute; left: 10px; top: 10px; width: 40px; height: 20px; \
     image-rendering: pixelated; display: block";

fn doc_with_img(src: &str) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css("html, body { margin: 0; background: #fff; }");
    doc.tree.image_loader = Some(Arc::new(FileImageLoader));
    let body = doc.body();
    let img = doc.create_element("img");
    doc.set_attribute(img, "src", src);
    doc.set_attribute(img, "style", IMG_STYLE);
    doc.append_child(body, img);
    doc.resolve_layout(W as f32, H as f32);
    (doc, img)
}

/// Lay out until `done` holds (loads run on background threads). The
/// viewport alternates so each layout really runs.
fn settle(doc: &mut RinchDocument, mut done: impl FnMut(&RinchDocument) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut wide = false;
    loop {
        wide = !wide;
        doc.resolve_layout(W as f32 + if wide { 1.0 } else { 0.0 }, H as f32);
        if done(doc) {
            doc.resolve_layout(W as f32, H as f32);
            return true;
        }
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn paint(doc: &mut RinchDocument) -> Vec<[u8; 4]> {
    doc.resolve_layout(W as f32, H as f32);
    let mut painter = TinySkiaPainter::new(W, H);
    painter.fill_white();
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (W as f32, H as f32),
        &mut doc.font_cx,
        &mut cx,
    );
    painter.pixels().as_chunks::<4>().0.to_vec()
}

fn at(px: &[[u8; 4]], x: u32, y: u32) -> [u8; 3] {
    let p = px[(y * W + x) as usize];
    [p[0], p[1], p[2]]
}

/// The centre of each 10x10 block, and the colour it must be over white.
/// The half-transparent blue over white is (127, 127, 255) premultiplied
/// and composited: 255 * (1 - 128/255) = 127.
const EXPECTED: &[((u32, u32), [u8; 3])] = &[
    ((15, 15), [255, 0, 0]),
    ((25, 15), [0, 255, 0]),
    ((35, 15), [0, 0, 255]),
    ((45, 15), [127, 127, 255]),
    ((15, 25), [0, 0, 0]),
    ((25, 25), [255, 255, 255]),
    ((35, 25), [255, 255, 0]),
    ((45, 25), [255, 0, 255]),
    // Outside the box: the page.
    ((55, 15), [255, 255, 255]),
    ((15, 35), [255, 255, 255]),
];

fn assert_picture(px: &[[u8; 4]], what: &str) {
    for &((x, y), want) in EXPECTED {
        let got = at(px, x, y);
        let close = got
            .iter()
            .zip(want)
            .all(|(&g, w)| (g as i32 - w as i32).abs() <= 1);
        assert!(close, "{what}: ({x}, {y}) is {got:?}, want {want:?}");
    }
}

fn box_of(doc: &RinchDocument, node: NodeId) -> (f32, f32) {
    let n = doc.tree.nodes.get(node.0).expect("node in tree");
    (n.layout.width, n.layout.height)
}

#[test]
fn registered_pixels_paint_as_given_and_are_not_copied() {
    let pixels = picture();
    let src = register_rgba("t2740-paint", 4, 2, pixels.clone()).expect("valid pixels");
    assert_eq!(src, "rinch-image:t2740-paint");
    let (mut doc, _img) = doc_with_img(&src);
    assert!(settle(&mut doc, |d| d.tree.image_cache.get(&src).is_some()));
    let cached = doc.tree.image_cache.get(&src).unwrap();
    assert_eq!((cached.width, cached.height), (4, 2));
    assert_eq!(
        cached.data.as_ptr(),
        pixels.as_ptr(),
        "the cache holds the app's own buffer"
    );
    assert_picture(&paint(&mut doc), "software");
    unregister_rgba("t2740-paint");
}

#[test]
fn a_loader_may_answer_with_decoded_pixels() {
    register_image_scheme("t2740-own", |_: &str| ImageLoadResult::Decoded {
        width: 4,
        height: 2,
        rgba: picture(),
    });
    let (mut doc, _img) = doc_with_img("t2740-own:x");
    assert!(settle(&mut doc, |d| d
        .tree
        .image_cache
        .get("t2740-own:x")
        .is_some()));
    assert_picture(&paint(&mut doc), "a loader's pixels");
    // `object-fit` and the rest apply as to any picture: a natural-size box.
    let body = doc.body();
    let natural = doc.create_element("img");
    doc.set_attribute(natural, "src", "t2740-own:x");
    doc.append_child(body, natural);
    doc.resolve_layout(W as f32 + 2.0, H as f32);
    assert_eq!(box_of(&doc, natural), (4.0, 2.0));
}

#[test]
fn a_wrong_length_fails_the_load_cleanly() {
    register_image_scheme("t2740-short", |_: &str| ImageLoadResult::Decoded {
        width: 4,
        height: 2,
        rgba: Arc::from(vec![255u8; 31]),
    });
    let (mut doc, _img) = doc_with_img("t2740-short:x");
    assert!(
        settle(&mut doc, |d| d.tree.image_cache.is_failed("t2740-short:x")),
        "a buffer one byte short is a failed load, not a panic or a picture"
    );
    let px = paint(&mut doc);
    assert_ne!(at(&px, 15, 15), [255, 255, 0], "no picture drawn");
    // And registering one is refused at the call, with the reason.
    assert_eq!(
        register_rgba("t2740-short", 4, 2, vec![0u8; 31]),
        Err(DecodedImageError::WrongLength {
            expected: 32,
            actual: 31
        })
    );
}

#[test]
fn the_size_limit_of_encoded_files_applies_to_pixels() {
    // 8192 x 16385 x 4 bytes is one row past 512 MiB. It is refused for what
    // it claims: the buffer is never looked at.
    register_image_scheme("t2740-huge", |_: &str| ImageLoadResult::Decoded {
        width: 8192,
        height: 16385,
        rgba: Arc::from(vec![0u8; 4]),
    });
    let (mut doc, _img) = doc_with_img("t2740-huge:x");
    assert!(settle(&mut doc, |d| d
        .tree
        .image_cache
        .is_failed("t2740-huge:x")));
    assert_eq!(MAX_DECODED_IMAGE_BYTES, 512 * 1024 * 1024);
    assert_eq!(
        register_rgba("t2740-huge", 8192, 16385, vec![0u8; 4]),
        Err(DecodedImageError::TooLarge {
            width: 8192,
            height: 16385
        })
    );
}

#[test]
fn registering_a_name_again_repaints_every_element_showing_it() {
    let src = register_rgba("t2740-update", 4, 2, picture()).unwrap();
    let (mut doc, img) = doc_with_img(&src);
    // A second element, natural size, to see the box follow the new picture.
    let body = doc.body();
    let natural = doc.create_element("img");
    doc.set_attribute(natural, "src", &src);
    doc.append_child(body, natural);
    assert!(settle(&mut doc, |d| d.tree.image_cache.get(&src).is_some()));
    assert_picture(&paint(&mut doc), "before");
    assert_eq!(box_of(&doc, natural), (4.0, 2.0));

    // A 2x1 picture, green then half-transparent red, from another thread.
    let first = doc.tree.image_cache.get(&src).unwrap().id();
    std::thread::spawn(|| {
        register_rgba("t2740-update", 2, 1, vec![0u8, 255, 0, 255, 255, 0, 0, 64]).unwrap();
    })
    .join()
    .unwrap();
    assert!(settle(&mut doc, |d| d
        .tree
        .image_cache
        .get(&src)
        .is_some_and(|i| i.id() != first)));
    assert!(
        doc.tree.paint_dirty_nodes.contains(&img.0),
        "the sized element, whose box did not change, is repainted"
    );
    assert_eq!(box_of(&doc, natural), (2.0, 1.0), "re-laid out");
    let px = paint(&mut doc);
    assert_eq!(at(&px, 20, 20), [0, 255, 0]);
    // 255 * (1 - 64/255) + 255 * 64/255 on red: (255, 191, 191).
    let red = at(&px, 40, 20);
    assert!(
        red[0] == 255 && (190..=192).contains(&red[1]) && (190..=192).contains(&red[2]),
        "the new picture's half-transparent red: {red:?}"
    );
    unregister_rgba("t2740-update");
}

#[test]
fn unregistering_a_name_on_screen_draws_no_picture() {
    let src = register_rgba("t2740-gone", 4, 2, picture()).unwrap();
    let (mut doc, img) = doc_with_img(&src);
    // A background showing it too.
    let body = doc.body();
    let bg = doc.create_element("div");
    doc.set_attribute(
        bg,
        "style",
        &format!(
            "position: absolute; left: 60px; top: 10px; width: 40px; height: 20px; \
             background-image: url({src}); background-size: 40px 20px; \
             image-rendering: pixelated"
        ),
    );
    doc.append_child(body, bg);
    assert!(settle(&mut doc, |d| d.tree.image_cache.get(&src).is_some()));
    let px = paint(&mut doc);
    assert_picture(&px, "before");
    assert_eq!(at(&px, 65, 15), [255, 0, 0], "the background draws it too");

    doc.tree.paint_dirty_nodes.clear();
    assert!(unregister_rgba("t2740-gone"));
    assert!(
        settle(&mut doc, |d| d.tree.image_cache.shows_failure(&src)),
        "the picture is taken away, not kept as a reload's failure keeps it"
    );
    assert!(doc.tree.image_cache.get(&src).is_none());
    assert!(
        doc.tree.paint_dirty_nodes.contains(&bg.0),
        "the background is repainted"
    );
    assert!(doc.tree.paint_dirty_nodes.contains(&img.0));
    let px = paint(&mut doc);
    for (x, y) in [(15, 15), (25, 25), (35, 25), (65, 15), (95, 25)] {
        let p = at(&px, x, y);
        assert!(
            ![
                [255, 0, 0],
                [0, 255, 0],
                [0, 0, 0],
                [255, 255, 0],
                [255, 0, 255]
            ]
            .contains(&p),
            "({x}, {y}) still draws the picture: {p:?}"
        );
    }

    // Registered again, it comes back.
    register_rgba("t2740-gone", 4, 2, picture()).unwrap();
    assert!(settle(&mut doc, |d| d.tree.image_cache.get(&src).is_some()));
    assert_picture(&paint(&mut doc), "registered again");
}

#[test]
fn a_name_registered_after_an_element_asked_for_it_is_shown() {
    let src = rinch_core::image::rgba_src("t2740-late");
    let (mut doc, _img) = doc_with_img(&src);
    assert!(settle(&mut doc, |d| d.tree.image_cache.is_failed(&src)));
    register_rgba("t2740-late", 4, 2, picture()).unwrap();
    assert!(settle(&mut doc, |d| d.tree.image_cache.get(&src).is_some()));
    assert_picture(&paint(&mut doc), "late");
    unregister_rgba("t2740-late");
}
