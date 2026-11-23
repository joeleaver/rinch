//! #2857: a registered `rinch-image:` source resolves in the frame that names
//! it. Its pixels are decoded and in memory already (`register_rgba`), so an
//! element whose picture moves from one registered name to another draws the
//! new picture in the very frame that applies the change, with no load thread
//! and no drain a frame later; and an unknown name fails in that frame. An
//! app's own loader for the scheme keeps the asynchronous path.
//!
//! Every test here takes [`ALONE`]: one registers a loader for the scheme,
//! which is process-wide.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_core::image::{
    ImageLoadResult, register_image_scheme, register_rgba, rgba_src, unregister_image_scheme,
    unregister_rgba,
};
use rinch_dom::RinchDocument;
use rinch_dom::image_cache::FileImageLoader;
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

const W: u32 = 60;
const H: u32 = 40;

/// The tests of this file one at a time: one takes the `rinch-image` scheme
/// over with a loader of its own, which every document in the process would
/// otherwise see.
static ALONE: Mutex<()> = Mutex::new(());

fn alone() -> MutexGuard<'static, ()> {
    ALONE.lock().unwrap_or_else(|e| e.into_inner())
}

/// A 2x2 picture all `rgb`, opaque.
fn solid(rgb: [u8; 3]) -> Arc<[u8]> {
    [[rgb[0], rgb[1], rgb[2], 255]; 4].concat().into()
}

const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const WHITE: [u8; 3] = [255, 255, 255];

/// Three pictures under names of `test`'s own: the first is never the one
/// under test.
struct Names {
    blue: String,
    red: String,
    green: String,
}

impl Names {
    fn register(test: &str) -> Self {
        let names = Self {
            blue: format!("t2857-{test}-blue"),
            red: format!("t2857-{test}-red"),
            green: format!("t2857-{test}-green"),
        };
        register_rgba(&names.blue, 2, 2, solid(BLUE)).unwrap();
        register_rgba(&names.red, 2, 2, solid(RED)).unwrap();
        register_rgba(&names.green, 2, 2, solid(GREEN)).unwrap();
        names
    }
}

impl Drop for Names {
    fn drop(&mut self) {
        for name in [&self.blue, &self.red, &self.green] {
            unregister_rgba(name);
        }
    }
}

/// A 40x20 box at (10, 10): `<img src>` or a `div` with `src` as its
/// background.
const BOX: &str = "position: absolute; left: 10px; top: 10px; width: 40px; height: 20px; \
     display: block";

fn background(src: &str) -> String {
    format!("{BOX}; background-image: url({src}); background-size: 40px 20px")
}

fn document() -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css("html, body { margin: 0; background: #fff; }");
    doc.tree.image_loader = Some(Arc::new(FileImageLoader));
    doc
}

fn with_background(src: &str) -> (RinchDocument, NodeId) {
    let mut doc = document();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", &background(src));
    doc.append_child(body, div);
    (doc, div)
}

fn with_img(src: &str) -> (RinchDocument, NodeId) {
    let mut doc = document();
    let body = doc.body();
    let img = doc.create_element("img");
    doc.set_attribute(img, "src", src);
    doc.set_attribute(img, "style", BOX);
    doc.append_child(body, img);
    (doc, img)
}

/// One frame: a layout, then a paint. Returns the box's middle pixel.
fn frame(doc: &mut RinchDocument) -> [u8; 3] {
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
    let px = painter.pixels().as_chunks::<4>().0[(20 * W + 30) as usize];
    [px[0], px[1], px[2]]
}

/// Frames until the box's middle is `want` (a load on a background thread
/// lands a frame or more later); `false` after ten seconds.
fn frames_until(doc: &mut RinchDocument, want: [u8; 3]) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if frame(doc) == want {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    false
}

#[test]
fn a_background_moved_to_another_registered_name_draws_it_in_that_frame() {
    let _alone = alone();
    let names = Names::register("bg");
    let (mut doc, div) = with_background(&rgba_src(&names.red));
    assert!(frames_until(&mut doc, RED), "the picture is drawn");
    doc.set_attribute(div, "style", &background(&rgba_src(&names.green)));
    assert_eq!(
        frame(&mut doc),
        GREEN,
        "the frame that applies the new background draws its picture"
    );
}

#[test]
fn an_img_moved_to_another_registered_name_draws_it_in_that_frame() {
    let _alone = alone();
    let names = Names::register("img");
    let (mut doc, img) = with_img(&rgba_src(&names.red));
    assert!(frames_until(&mut doc, RED), "the picture is drawn");
    doc.set_attribute(img, "src", &rgba_src(&names.green));
    assert_eq!(
        frame(&mut doc),
        GREEN,
        "the frame that applies the new src draws its picture"
    );
}

#[test]
fn an_unknown_name_fails_in_the_frame_that_names_it() {
    let _alone = alone();
    let names = Names::register("unknown");
    let (mut doc, img) = with_img(&rgba_src(&names.red));
    assert!(frames_until(&mut doc, RED), "the picture is drawn");
    let nowhere = rgba_src("t2857-unknown-nowhere");
    doc.set_attribute(img, "src", &nowhere);
    assert_eq!(frame(&mut doc), WHITE, "an unknown name draws no picture");
    assert!(
        doc.tree.image_cache.shows_failure(&nowhere),
        "the failure is known in that frame, so the <img> has its placeholder"
    );
}

#[test]
fn an_apps_own_loader_for_the_scheme_still_loads_on_its_thread() {
    let _alone = alone();
    let names = Names::register("app");
    let (mut doc, div) = with_background(&rgba_src(&names.red));
    assert!(frames_until(&mut doc, RED), "the picture is drawn");
    register_image_scheme("rinch-image", |_: &str| ImageLoadResult::Decoded {
        width: 2,
        height: 2,
        rgba: solid(GREEN),
    });
    let src = rgba_src("t2857-app-anything");
    doc.set_attribute(div, "style", &background(&src));
    let first = frame(&mut doc);
    let landed = frames_until(&mut doc, GREEN);
    unregister_image_scheme("rinch-image");
    assert_eq!(
        first, WHITE,
        "the app's loader is asked on the load thread: its answer lands a frame later"
    );
    assert!(landed, "the app's loader's picture is drawn once it lands");
}

/// Two documents on one thread name one registered picture; one moves to
/// another name while the other keeps the first. In that frame each draws its
/// own, and the picture both showed was one decoded image, not two.
#[test]
fn two_documents_on_one_thread_each_draw_their_own_in_the_frame() {
    let _alone = alone();
    let names = Names::register("two");
    let (mut moving, div) = with_background(&rgba_src(&names.red));
    let (mut staying, _) = with_background(&rgba_src(&names.red));
    assert!(frames_until(&mut moving, RED), "the picture is drawn");
    assert!(frames_until(&mut staying, RED), "the picture is drawn");
    let red = rgba_src(&names.red);
    assert!(
        Arc::ptr_eq(
            moving.tree.image_cache.get_shared(&red).unwrap(),
            staying.tree.image_cache.get_shared(&red).unwrap()
        ),
        "both documents hold one decoded picture for the name (#2757)"
    );
    moving.set_attribute(div, "style", &background(&rgba_src(&names.green)));
    assert_eq!(
        frame(&mut moving),
        GREEN,
        "the one that moved draws the new picture"
    );
    assert_eq!(
        frame(&mut staying),
        RED,
        "the other keeps the picture it showed"
    );
}
