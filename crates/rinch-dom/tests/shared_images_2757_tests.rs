//! #2757: one process, several windows, the same picture in each. The source
//! is loaded and decoded once for the process, every document holds that one
//! decode (an `Arc`), and its mipmap level is made once.
//!
//! Before #2757 each document's `ImageCache` loaded and decoded the source on
//! its own (deskrinch's wallpaper: 2 x 31.64 MiB on two outputs), and each
//! made its own mipmap level of its own copy (2 x 7.91 MiB).

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_core::image::{
    ImageLoadResult, ImageLoader, register_image_scheme, register_rgba, reload_image,
};
use rinch_dom::RinchDocument;
use rinch_dom::image_cache::DecodedImage;
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// `reload_image` reaches every live document in the process; the tests that
/// reload run one at a time.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn alone() -> std::sync::MutexGuard<'static, ()> {
    ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner())
}

/// A `w` x `h` PNG, its pixels varied so no two rows or columns agree.
fn png(w: u32, h: u32, seed: u8) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        image::Rgba([(x as u8).wrapping_add(seed), y as u8, (x ^ y) as u8, 255])
    });
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// An app's blob store as a scheme loader: answers what it holds, counts how
/// often it is asked (each answer is decoded, so this is the decode count).
#[derive(Default)]
struct Blobs {
    bytes: Mutex<Vec<u8>>,
    asked: AtomicUsize,
}

impl Blobs {
    fn scheme(scheme: &str, bytes: Vec<u8>) -> Arc<Blobs> {
        let blobs = Arc::new(Blobs {
            bytes: Mutex::new(bytes),
            asked: AtomicUsize::new(0),
        });
        let inner = blobs.clone();
        register_image_scheme(scheme, move |_src: &str| {
            inner.asked.fetch_add(1, Ordering::SeqCst);
            ImageLoadResult::Loaded(inner.bytes.lock().unwrap().clone())
        });
        blobs
    }
    fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }
}

fn doc_with_img(src: &str, css: &str) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(&format!(
        "html, body {{ margin: 0; background: #fff; }} img {{ display: block; {css} }}"
    ));
    let body = doc.body();
    let img = doc.create_element("img");
    doc.append_child(body, img);
    doc.set_attribute(img, "src", src);
    doc.resolve_layout(800.0, 600.0);
    (doc, img)
}

/// Lay out until `done` holds (loads run on background threads).
fn settle(doc: &mut RinchDocument, mut done: impl FnMut(&RinchDocument) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        doc.resolve_layout(800.0, 600.0);
        if done(doc) {
            return true;
        }
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn picture<'a>(doc: &'a RinchDocument, src: &str) -> &'a DecodedImage {
    doc.tree.image_cache.get(src).expect("decoded")
}

/// Whether both documents hold the very same decoded picture (one
/// allocation, one pixel buffer), not two equal ones.
fn same_picture(a: &RinchDocument, b: &RinchDocument, src: &str) -> bool {
    let (pa, pb) = (picture(a, src), picture(b, src));
    std::ptr::eq(pa, pb) && pa.data.as_ptr() == pb.data.as_ptr()
}

fn paint(doc: &mut RinchDocument) {
    doc.resolve_layout(200.0, 200.0);
    let mut painter = TinySkiaPainter::new(200, 200);
    painter.fill_white();
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (200.0, 200.0),
        &mut doc.font_cx,
        &mut cx,
    );
}

#[test]
fn two_documents_decode_a_source_once_and_hold_one_copy() {
    let src = "share2757-once:wallpaper";
    let blobs = Blobs::scheme("share2757-once", png(30, 20, 1));
    let (mut a, _) = doc_with_img(src, "");
    assert!(settle(&mut a, |d| d.tree.image_cache.get(src).is_some()));
    let (mut b, img_b) = doc_with_img(src, "");
    assert!(settle(&mut b, |d| d.tree.image_cache.get(src).is_some()));
    assert_eq!(blobs.asked(), 1, "loaded and decoded once for both");
    assert!(same_picture(&a, &b, src), "one decoded copy, shared");
    let n = b.tree.nodes.get(img_b.0).unwrap();
    assert_eq!((n.layout.width, n.layout.height), (30.0, 20.0));
}

/// In-flight dedup: both documents ask while the load is still running; one
/// load answers both.
#[test]
fn two_documents_asking_at_once_share_one_load() {
    let src = "share2757-flight:wallpaper";
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let asked = Arc::new(AtomicUsize::new(0));
    let bytes = png(30, 20, 2);
    {
        let (gate, asked) = (gate.clone(), asked.clone());
        register_image_scheme("share2757-flight", move |_src: &str| {
            asked.fetch_add(1, Ordering::SeqCst);
            let (open, cv) = &*gate;
            let mut open = open.lock().unwrap();
            while !*open {
                open = cv.wait(open).unwrap();
            }
            ImageLoadResult::Loaded(bytes.clone())
        });
    }
    let (mut a, _) = doc_with_img(src, "");
    let (mut b, _) = doc_with_img(src, "");
    // Both have asked; the load cannot have answered.
    let deadline = Instant::now() + Duration::from_secs(10);
    while asked.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    std::thread::sleep(Duration::from_millis(50));
    assert!(a.tree.image_cache.get(src).is_none() && b.tree.image_cache.get(src).is_none());
    {
        let (open, cv) = &*gate;
        *open.lock().unwrap() = true;
        cv.notify_all();
    }
    assert!(settle(&mut a, |d| d.tree.image_cache.get(src).is_some()));
    assert!(settle(&mut b, |d| d.tree.image_cache.get(src).is_some()));
    assert_eq!(asked.load(Ordering::SeqCst), 1, "one load for both");
    assert!(same_picture(&a, &b, src));
}

/// The mipmap level of a picture drawn small is made once for both documents
/// (the minify cache keys on the picture's identity, which they now share).
#[test]
fn two_documents_share_one_mip_level() {
    let src = "share2757-mip:wallpaper";
    let blobs = Blobs::scheme("share2757-mip", png(400, 400, 3));
    // Drawn at 100px: 1/4, mipmap level 2 (100x100).
    let (mut a, _) = doc_with_img(src, "width: 100px");
    let (mut b, _) = doc_with_img(src, "width: 100px");
    assert!(settle(&mut a, |d| d.tree.image_cache.get(src).is_some()));
    assert!(settle(&mut b, |d| d.tree.image_cache.get(src).is_some()));
    rinch_dom::image_minify::begin_frame();
    let made = rinch_dom::image_minify::made();
    paint(&mut a);
    paint(&mut b);
    assert_eq!(
        rinch_dom::image_minify::made() - made,
        1,
        "one level for both"
    );
    let t = peniko::kurbo::Affine::scale(0.25);
    let la = rinch_dom::image_minify::minified(picture(&a, src), t).unwrap();
    let lb = rinch_dom::image_minify::minified(picture(&b, src), t).unwrap();
    assert!(Arc::ptr_eq(&la, &lb), "the same level");
    assert_eq!((la.width, la.height), (100, 100));
    assert!(blobs.asked() <= 2);
}

/// `reload_image` reaches both documents, and their reloads share one load.
#[test]
fn a_reload_reaches_both_documents_with_one_load() {
    let _alone = alone();
    let src = "share2757-reload:wallpaper";
    let blobs = Blobs::scheme("share2757-reload", png(3, 2, 4));
    let (mut a, _) = doc_with_img(src, "");
    assert!(settle(&mut a, |d| d.tree.image_cache.get(src).is_some()));
    let (mut b, _) = doc_with_img(src, "");
    assert!(settle(&mut b, |d| d.tree.image_cache.get(src).is_some()));
    assert_eq!(blobs.asked(), 1);
    *blobs.bytes.lock().unwrap() = png(1, 1, 5);
    reload_image(src);
    assert!(settle(&mut a, |d| d
        .tree
        .image_cache
        .get(src)
        .is_some_and(|i| i.width == 1)));
    assert!(settle(&mut b, |d| d
        .tree
        .image_cache
        .get(src)
        .is_some_and(|i| i.width == 1)));
    assert_eq!(blobs.asked(), 2, "one more load, for both");
    assert!(same_picture(&a, &b, src));
}

/// A `register_rgba` picture replaced reaches both documents, which share it,
/// over the app's own buffer.
#[test]
fn a_register_rgba_update_reaches_both_documents() {
    let _alone = alone();
    let src = register_rgba("share2757-rgba", 2, 2, vec![9u8; 16]).unwrap();
    let (mut a, _) = doc_with_img(&src, "");
    let (mut b, _) = doc_with_img(&src, "");
    assert!(settle(&mut a, |d| d.tree.image_cache.get(&src).is_some()));
    assert!(settle(&mut b, |d| d.tree.image_cache.get(&src).is_some()));
    assert!(same_picture(&a, &b, &src));
    let three: Arc<[u8]> = vec![7u8; 36].into();
    register_rgba("share2757-rgba", 3, 3, three.clone()).unwrap();
    assert!(settle(&mut a, |d| d
        .tree
        .image_cache
        .get(&src)
        .is_some_and(|i| i.width == 3)));
    assert!(settle(&mut b, |d| d
        .tree
        .image_cache
        .get(&src)
        .is_some_and(|i| i.width == 3)));
    assert!(same_picture(&a, &b, &src));
    assert_eq!(picture(&a, &src).data.as_ptr(), three.as_ptr(), "no copy");
}

/// The shared picture lives while some document holds it, and is freed when
/// the last one drops; a document made after that loads it again.
#[test]
fn the_picture_is_freed_when_the_last_document_drops() {
    let src = "share2757-free:wallpaper";
    let blobs = Blobs::scheme("share2757-free", png(30, 20, 6));
    let (mut a, _) = doc_with_img(src, "");
    assert!(settle(&mut a, |d| d.tree.image_cache.get(src).is_some()));
    let (mut b, _) = doc_with_img(src, "");
    assert!(settle(&mut b, |d| d.tree.image_cache.get(src).is_some()));
    let held = Arc::downgrade(a.tree.image_cache.get_shared(src).unwrap());
    assert_eq!(held.strong_count(), 2, "one handle per document");
    drop(a);
    assert!(held.upgrade().is_some(), "b still shows it");
    drop(b);
    assert!(held.upgrade().is_none(), "freed with the last document");
    let (mut c, _) = doc_with_img(src, "");
    assert!(settle(&mut c, |d| d.tree.image_cache.get(src).is_some()));
    assert_eq!(blobs.asked(), 2, "nobody held it: loaded again");
}

/// A loader a document holds as its own is never asked once for two
/// documents: it might answer each differently.
#[test]
fn documents_with_their_own_loaders_do_not_share() {
    struct Own(Arc<AtomicUsize>, Vec<u8>);
    impl ImageLoader for Own {
        fn load(&self, _src: &str) -> ImageLoadResult {
            self.0.fetch_add(1, Ordering::SeqCst);
            ImageLoadResult::Loaded(self.1.clone())
        }
    }
    let src = "share2757-own.png";
    let asked = Arc::new(AtomicUsize::new(0));
    let make = |seed: u32| {
        let mut doc = RinchDocument::new();
        doc.tree.image_loader = Some(Arc::new(Own(asked.clone(), png(4 + seed, 4, 0))));
        let body = doc.body();
        let img = doc.create_element("img");
        doc.append_child(body, img);
        doc.set_attribute(img, "src", src);
        doc
    };
    let (mut a, mut b) = (make(0), make(1));
    assert!(settle(&mut a, |d| d.tree.image_cache.get(src).is_some()));
    assert!(settle(&mut b, |d| d.tree.image_cache.get(src).is_some()));
    assert_eq!(asked.load(Ordering::SeqCst), 2);
    assert_eq!((picture(&a, src).width, picture(&b, src).width), (4, 5));
}

/// One document's own `RinchDocument::reload_image` loads fresh for it (the
/// shared picture is forgotten); the other keeps what it shows until it is
/// reloaded itself, and a document made afterwards shares the new picture.
#[test]
fn one_documents_own_reload_loads_fresh() {
    let _alone = alone();
    let src = "share2757-own-reload:wallpaper";
    let blobs = Blobs::scheme("share2757-own-reload", png(3, 2, 7));
    let (mut a, _) = doc_with_img(src, "");
    assert!(settle(&mut a, |d| d.tree.image_cache.get(src).is_some()));
    let (mut b, _) = doc_with_img(src, "");
    assert!(settle(&mut b, |d| d.tree.image_cache.get(src).is_some()));
    *blobs.bytes.lock().unwrap() = png(1, 1, 8);
    a.reload_image(src);
    assert!(settle(&mut a, |d| d
        .tree
        .image_cache
        .get(src)
        .is_some_and(|i| i.width == 1)));
    assert_eq!(blobs.asked(), 2);
    assert_eq!(picture(&b, src).width, 3, "b was not reloaded");
    let (mut c, _) = doc_with_img(src, "");
    assert!(settle(&mut c, |d| d.tree.image_cache.get(src).is_some()));
    assert!(same_picture(&a, &c, src));
    assert_eq!(blobs.asked(), 2);
}
